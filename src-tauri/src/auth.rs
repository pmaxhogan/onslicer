//! Onshape authentication: API keys, email + password login, or a pasted
//! browser session.
//!
//! CONTRACT FILE. The public items below (types, field names, serde shapes,
//! function signatures, `#[tauri::command]` names) are shared by the Onshape
//! client (onshape.rs), the frontend (src/contract-types.ts) and this
//! implementation. Change only the bodies and private items.
//!
//! Storage:
//! - non-secret settings live in the tauri-plugin-store file `settings.json`
//!   under the key `auth`: `{ "mode": "apiKey" | "password" | "session",
//!   "username": "..." }`
//! - secrets live in the OS credential store (keyring crate, Windows
//!   Credential Manager), service `com.pmaxhogan.onslicer`, accounts
//!   `api-access-key`, `api-secret-key`, `password`, `session`. Values longer
//!   than one credential blob allows are split across `<account>.<n>` entries
//!   (see `write_secret`).
//!
//! Password mode signs in the way the Onshape web client does
//! (`POST /api/users/session` with `{ email, password }`) and keeps the
//! returned cookies as the session. Sessions expire often, so a 401/403 makes
//! the client call `handle_auth_failure`, which signs in again and asks for one
//! retry. Session mode sends a cookie string pasted from the browser as is.

use base64::Engine;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, COOKIE};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use tauri::async_runtime::Mutex as AsyncMutex;
use tauri::{AppHandle, State};
use tauri_plugin_store::StoreExt;

/// Keyring service name shared by every secret.
pub const KEYRING_SERVICE: &str = "com.pmaxhogan.onslicer";

const STORE_FILE: &str = "settings.json";
const STORE_KEY: &str = "auth";
const ACCESS_KEY: &str = "api-access-key";
const SECRET_KEY: &str = "api-secret-key";
const PASSWORD: &str = "password";
const SESSION: &str = "session";
const LOGIN_URL: &str = "https://cad.onshape.com/api/users/session";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum AuthMode {
    #[default]
    ApiKey,
    Password,
    Session,
}

/// What the settings dialog shows. Secrets never go back to the UI, only
/// whether one is stored.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AuthSettings {
    pub mode: AuthMode,
    pub has_api_keys: bool,
    pub username: String,
    pub has_password: bool,
    pub has_session: bool,
}

/// A save from the settings dialog. For each field `None` keeps the stored
/// value and `Some("")` deletes it.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AuthInput {
    pub mode: AuthMode,
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub session: Option<String>,
}

/// The non-secret part, as persisted in the store.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
struct StoredAuth {
    mode: AuthMode,
    username: String,
}

/// Managed state, registered in lib.rs with `app.manage(Auth::new(client))`.
/// `client` is the shared reqwest client (redirects disabled).
pub struct Auth {
    client: reqwest::Client,
    /// Keyring reads are cached; `None` inside means "known to be absent".
    secrets: std::sync::Mutex<HashMap<&'static str, Option<String>>>,
    /// Serializes password logins so parallel requests that all hit an
    /// expired session trigger one sign-in, not one each.
    login: AsyncMutex<()>,
}

impl Auth {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            secrets: std::sync::Mutex::new(HashMap::new()),
            login: AsyncMutex::new(()),
        }
    }

    /// Headers to attach to a request for `url`. Only called for
    /// `*.onshape.com` hosts, including the sibling host the STL export
    /// redirects to. Errors with a user-facing message when the selected mode
    /// has no usable credentials, e.g. "Onshape API keys not set - open Settings".
    pub async fn headers_for(
        &self,
        app: &AppHandle,
        _url: &reqwest::Url,
    ) -> Result<HeaderMap, String> {
        match load_stored(app)?.mode {
            AuthMode::ApiKey => {
                let (Some(access), Some(secret)) =
                    (self.secret(ACCESS_KEY)?, self.secret(SECRET_KEY)?)
                else {
                    return Err("Onshape API keys not set - open Settings".into());
                };
                let mut headers = HeaderMap::new();
                headers.insert(AUTHORIZATION, header_value(&basic_auth(&access, &secret))?);
                Ok(headers)
            }
            AuthMode::Password => {
                let session = match self.secret(SESSION)? {
                    Some(s) => s,
                    None => self.login(app, None).await?,
                };
                cookie_headers(&session)
            }
            AuthMode::Session => match self.secret(SESSION)? {
                Some(s) => cookie_headers(&s),
                None => Err("Onshape session not set - open Settings".into()),
            },
        }
    }

    /// Onshape rotates the `on` session cookie on each response. GETs still
    /// take an older value but POSTs answer 401 to it, so a cookie session
    /// keeps the rotated values. Only in memory: the stored session still
    /// works for GETs, and the first response of a run rotates it again.
    /// Call only with successful responses (an error may set a fresh,
    /// signed-out session). A no-op with API keys, where no session is
    /// loaded.
    pub(crate) fn note_response_cookies(&self, headers: &HeaderMap) {
        let updates = session_cookies(headers);
        if updates.is_empty() {
            return;
        }
        let Ok(mut cache) = self.secrets.lock() else {
            return;
        };
        if let Some(Some(current)) = cache.get(SESSION) {
            let merged = merge_cookies(current, &updates);
            if merged != *current {
                cache.insert(SESSION, Some(merged));
            }
        }
    }

    /// Called once when an onshape.com response came back 401 or 403.
    /// `Ok(true)`: credentials were refreshed (for example a fresh password
    /// login), so the caller retries the request once. `Ok(false)`: nothing to
    /// refresh, so the caller surfaces the original error.
    pub async fn handle_auth_failure(&self, app: &AppHandle) -> Result<bool, String> {
        match load_stored(app)?.mode {
            AuthMode::ApiKey => Ok(false),
            AuthMode::Password => {
                let failed = self.secret(SESSION)?;
                self.login(app, failed.as_deref()).await?;
                Ok(true)
            }
            AuthMode::Session => {
                Err("Onshape session expired or invalid - paste a fresh one in Settings".into())
            }
        }
    }

    /// Signs in with the stored email + password and stores the session.
    /// `failed` is the session that just got rejected: if another request
    /// already replaced it while this one waited for the lock, reuse that.
    async fn login(&self, app: &AppHandle, failed: Option<&str>) -> Result<String, String> {
        let _guard = self.login.lock().await;
        if let Some(current) = self.secret(SESSION)? {
            if failed != Some(current.as_str()) {
                return Ok(current);
            }
        }
        let email = load_stored(app)?.username;
        let password = self.secret(PASSWORD)?;
        let Some(password) = password.filter(|_| !email.is_empty()) else {
            return Err("Onshape email or password not set - open Settings".into());
        };
        let res = self
            .client
            .post(LOGIN_URL)
            .header("Accept", "application/json;charset=UTF-8; qs=0.09")
            .json(&json!({ "email": email, "password": password }))
            .send()
            .await
            .map_err(|e| format!("Onshape sign-in failed: {e}"))?;
        let status = res.status();
        let cookies = session_cookies(res.headers());
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(format!(
                "Onshape sign-in failed ({status}): {}",
                body.chars().take(200).collect::<String>()
            ));
        }
        if cookies.is_empty() {
            return Err("Onshape sign-in returned no session cookies".into());
        }
        self.set_secret(SESSION, Some(&cookies))?;
        Ok(cookies)
    }

    fn secret(&self, account: &'static str) -> Result<Option<String>, String> {
        let mut cache = self.secrets.lock().map_err(|e| e.to_string())?;
        if let Some(v) = cache.get(account) {
            return Ok(v.clone());
        }
        let v = read_secret(account)?;
        cache.insert(account, v.clone());
        Ok(v)
    }

    fn set_secret(&self, account: &'static str, value: Option<&str>) -> Result<(), String> {
        let mut cache = self.secrets.lock().map_err(|e| e.to_string())?;
        match value {
            Some(v) => write_secret(account, v)?,
            None => delete_secret(account)?,
        }
        cache.insert(account, value.map(str::to_owned));
        Ok(())
    }

    fn settings(&self, app: &AppHandle) -> Result<AuthSettings, String> {
        let stored = load_stored(app)?;
        Ok(AuthSettings {
            mode: stored.mode,
            has_api_keys: self.secret(ACCESS_KEY)?.is_some() && self.secret(SECRET_KEY)?.is_some(),
            username: stored.username,
            has_password: self.secret(PASSWORD)?.is_some(),
            has_session: self.secret(SESSION)?.is_some(),
        })
    }
}

#[tauri::command]
pub async fn get_auth_settings(
    app: AppHandle,
    auth: State<'_, Auth>,
) -> Result<AuthSettings, String> {
    auth.settings(&app)
}

/// Stores the input, then returns the new settings. In password mode with a
/// new username or password it logs in right away so a bad password fails here,
/// in the dialog, instead of on the next API call.
#[tauri::command]
pub async fn save_auth_settings(
    app: AppHandle,
    auth: State<'_, Auth>,
    input: AuthInput,
) -> Result<AuthSettings, String> {
    let mut stored = load_stored(&app)?;
    let mode_changed = stored.mode != input.mode;
    stored.mode = input.mode;
    let mut credentials_changed = false;
    if let Some(u) = &input.username {
        credentials_changed |= u.trim() != stored.username;
        stored.username = u.trim().to_owned();
    }
    save_stored(&app, &stored)?;

    for (account, value) in [
        (ACCESS_KEY, &input.access_key),
        (SECRET_KEY, &input.secret_key),
        (PASSWORD, &input.password),
    ] {
        if let Some(v) = value {
            let v = v.trim();
            auth.set_secret(account, (!v.is_empty()).then_some(v))?;
            credentials_changed |= account == PASSWORD;
        }
    }
    if let Some(s) = &input.session {
        let s = normalize_cookie(s);
        auth.set_secret(SESSION, (!s.is_empty()).then_some(s.as_str()))?;
    }

    // A session from one mode is not valid for the other: a pasted browser
    // session must not survive a switch to password mode or the reverse.
    if input.mode == AuthMode::Password && (credentials_changed || mode_changed) {
        auth.set_secret(SESSION, None)?;
        auth.login(&app, None).await?;
    } else if mode_changed && input.mode != AuthMode::Session {
        auth.set_secret(SESSION, None)?;
    }
    auth.settings(&app)
}

// ---------- store ----------

fn load_stored(app: &AppHandle) -> Result<StoredAuth, String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    Ok(store
        .get(STORE_KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

fn save_stored(app: &AppHandle, stored: &StoredAuth) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.set(
        STORE_KEY,
        serde_json::to_value(stored).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())
}

// ---------- keyring ----------

/// Windows caps one credential blob at 2560 bytes (and may store the text as
/// UTF-16), so a long browser cookie string is split into chunks: `<account>` holds
/// the chunk count, `<account>.0` .. `<account>.<n-1>` the text.
const CHUNK_CHARS: usize = 1000;

fn entry(account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, account).map_err(|e| e.to_string())
}

fn get(account: &str) -> Result<Option<String>, String> {
    match entry(account)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("reading {account} from the credential store: {e}")),
    }
}

fn delete(account: &str) -> Result<(), String> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("deleting {account} from the credential store: {e}")),
    }
}

fn read_secret(account: &str) -> Result<Option<String>, String> {
    let Some(count) = get(account)? else {
        return Ok(None);
    };
    let count: usize = count.parse().map_err(|_| {
        format!("credential store entry {account} is corrupt - save it again in Settings")
    })?;
    let mut value = String::new();
    for i in 0..count {
        match get(&format!("{account}.{i}"))? {
            Some(chunk) => value.push_str(&chunk),
            None => return Ok(None),
        }
    }
    Ok(Some(value))
}

fn write_secret(account: &str, value: &str) -> Result<(), String> {
    delete_secret(account)?;
    let chunks = chunk(value, CHUNK_CHARS);
    for (i, c) in chunks.iter().enumerate() {
        entry(&format!("{account}.{i}"))?
            .set_password(c)
            .map_err(|e| format!("saving {account} to the credential store: {e}"))?;
    }
    entry(account)?
        .set_password(&chunks.len().to_string())
        .map_err(|e| format!("saving {account} to the credential store: {e}"))
}

fn delete_secret(account: &str) -> Result<(), String> {
    if let Some(count) = get(account)?.and_then(|c| c.parse::<usize>().ok()) {
        for i in 0..count {
            delete(&format!("{account}.{i}"))?;
        }
    }
    delete(account)
}

fn chunk(value: &str, size: usize) -> Vec<String> {
    let chars: Vec<char> = value.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    chars.chunks(size).map(|c| c.iter().collect()).collect()
}

// ---------- headers ----------

fn header_value(v: &str) -> Result<HeaderValue, String> {
    HeaderValue::from_str(v)
        .map_err(|_| "credentials contain characters not allowed in a header".into())
}

fn basic_auth(access: &str, secret: &str) -> String {
    let token = base64::engine::general_purpose::STANDARD.encode(format!("{access}:{secret}"));
    format!("Basic {token}")
}

/// Cookie header plus, when the session carries an XSRF-TOKEN cookie, the
/// matching X-XSRF-TOKEN header the web client sends with it.
fn cookie_headers(session: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(COOKIE, header_value(session)?);
    if let Some(token) = cookie_value(session, "XSRF-TOKEN") {
        headers.insert("X-XSRF-TOKEN", header_value(token)?);
    }
    Ok(headers)
}

/// `current` with each `name=value` of `updates` replacing the cookie of
/// that name, or appended when there is none.
fn merge_cookies(current: &str, updates: &str) -> String {
    let mut pairs: Vec<(String, String)> = current
        .split(';')
        .filter_map(|p| p.trim().split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    for (k, v) in updates.split(';').filter_map(|p| p.trim().split_once('=')) {
        match pairs.iter_mut().find(|(name, _)| name == k) {
            Some(pair) => pair.1 = v.to_string(),
            None => pairs.push((k.to_string(), v.to_string())),
        }
    }
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn cookie_value<'a>(cookies: &'a str, name: &str) -> Option<&'a str> {
    cookies
        .split(';')
        .filter_map(|p| p.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

/// `name=value` pairs of every Set-Cookie in a login response, joined into a
/// Cookie header. Cookies the server deletes (empty value) are skipped.
fn session_cookies(headers: &HeaderMap) -> String {
    headers
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .map(str::trim)
        .filter(|p| {
            p.split_once('=')
                .is_some_and(|(k, v)| !k.is_empty() && !v.is_empty())
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Accepts what people paste from DevTools: a bare cookie string, or the whole
/// `Cookie: ...` header line, possibly spread over several lines.
fn normalize_cookie(raw: &str) -> String {
    let joined = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = joined.trim();
    let s = match s.split_once(':') {
        Some((k, rest)) if k.eq_ignore_ascii_case("cookie") => rest.trim(),
        _ => s,
    };
    s.trim_end_matches(';').trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_auth_encodes_key_pair() {
        assert_eq!(basic_auth("a", "b"), "Basic YTpi");
    }

    #[test]
    fn normalize_cookie_strips_header_name_and_whitespace() {
        assert_eq!(normalize_cookie("Cookie: a=1; b=2;\n"), "a=1; b=2");
        assert_eq!(normalize_cookie("  a=1;\n  b=2 "), "a=1; b=2");
        assert_eq!(normalize_cookie("cookie:a=1"), "a=1");
        assert_eq!(normalize_cookie(""), "");
    }

    #[test]
    fn cookie_value_finds_named_cookie() {
        let c = "on_session=abc; XSRF-TOKEN=tok=en; x=1";
        assert_eq!(cookie_value(c, "XSRF-TOKEN"), Some("tok=en"));
        assert_eq!(cookie_value(c, "missing"), None);
    }

    #[test]
    fn cookie_headers_adds_xsrf_when_present() {
        let h = cookie_headers("a=1; XSRF-TOKEN=t").unwrap();
        assert_eq!(h.get(COOKIE).unwrap(), "a=1; XSRF-TOKEN=t");
        assert_eq!(h.get("X-XSRF-TOKEN").unwrap(), "t");
        assert!(cookie_headers("a=1").unwrap().get("X-XSRF-TOKEN").is_none());
    }

    #[test]
    fn merge_cookies_replaces_by_name_and_appends_new_ones() {
        assert_eq!(
            merge_cookies("on=old; XSRF-TOKEN=t; _u=u", "on=new; extra=1"),
            "on=new; XSRF-TOKEN=t; _u=u; extra=1"
        );
        assert_eq!(merge_cookies("a=1", ""), "a=1");
        assert_eq!(merge_cookies("a=x=y", "a=b=c"), "a=b=c");
    }

    #[test]
    fn session_cookies_keeps_name_value_pairs_only() {
        let mut h = HeaderMap::new();
        for v in [
            "on_session=abc; Path=/; HttpOnly; Secure",
            "XSRF-TOKEN=t; Path=/",
            "deleted=; Max-Age=0",
        ] {
            h.append(
                reqwest::header::SET_COOKIE,
                HeaderValue::from_str(v).unwrap(),
            );
        }
        assert_eq!(session_cookies(&h), "on_session=abc; XSRF-TOKEN=t");
    }

    #[test]
    fn chunk_splits_on_char_boundaries() {
        assert_eq!(chunk("", 3), vec![""]);
        assert_eq!(chunk("abcdefg", 3), vec!["abc", "def", "g"]);
        assert_eq!(chunk(&"x".repeat(2500), 1000).len(), 3);
    }

    #[test]
    fn stored_auth_defaults_to_api_key_mode() {
        let s: StoredAuth = serde_json::from_value(json!({})).unwrap();
        assert_eq!(s.mode, AuthMode::ApiKey);
        let s: StoredAuth =
            serde_json::from_value(json!({ "mode": "password", "username": "a@b" })).unwrap();
        assert_eq!(s.mode, AuthMode::Password);
        assert_eq!(s.username, "a@b");
    }
}
