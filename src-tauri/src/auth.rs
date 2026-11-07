//! Onshape authentication: API keys, email + password login, or a pasted
//! browser session.
//!
//! CONTRACT FILE. The public items below (types, field names, serde shapes,
//! function signatures, `#[tauri::command]` names) are shared by the Onshape
//! client (onshape.rs), the frontend (src/types.ts) and the auth
//! implementation. Change only the bodies and private items.
//!
//! Storage:
//! - non-secret settings live in the tauri-plugin-store file `settings.json`
//!   under the key `auth`: `{ "mode": "apiKey" | "password" | "session",
//!   "username": "..." }`
//! - secrets live in the OS credential store (keyring crate, Windows
//!   Credential Manager), service `com.pmaxhogan.onslicer`, accounts
//!   `api-access-key`, `api-secret-key`, `password`, `session`.

#![allow(dead_code, unused_variables)]

use base64::Engine as _;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

/// Keyring service name shared by every secret.
pub const KEYRING_SERVICE: &str = "com.pmaxhogan.onslicer";

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

/// Managed state, registered in lib.rs with `app.manage(Auth::new(client))`.
/// `client` is the shared reqwest client (redirects disabled).
pub struct Auth {
    // TEMP: replaced by the full implementation at merge
    client: reqwest::Client,
}

// TEMP: replaced by the full implementation at merge. API-key mode only.
const ACCESS_KEY: &str = "api-access-key";
const SECRET_KEY: &str = "api-secret-key";

// TEMP: replaced by the full implementation at merge
fn entry(account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, account).map_err(|e| e.to_string())
}

// TEMP: replaced by the full implementation at merge
fn read_secret(account: &str) -> Result<Option<String>, String> {
    match entry(account)?.get_password() {
        Ok(v) => Ok(Some(v).filter(|v| !v.is_empty())),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("credential store: {e}")),
    }
}

// TEMP: replaced by the full implementation at merge
fn write_secret(account: &str, value: Option<String>) -> Result<(), String> {
    let Some(value) = value.map(|v| v.trim().to_string()) else {
        return Ok(());
    };
    let entry = entry(account)?;
    let res = if value.is_empty() {
        match entry.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            other => other,
        }
    } else {
        entry.set_password(&value)
    };
    res.map_err(|e| format!("credential store: {e}"))
}

// TEMP: replaced by the full implementation at merge
fn current_settings() -> Result<AuthSettings, String> {
    Ok(AuthSettings {
        mode: AuthMode::ApiKey,
        has_api_keys: read_secret(ACCESS_KEY)?.is_some() && read_secret(SECRET_KEY)?.is_some(),
        username: String::new(),
        has_password: false,
        has_session: false,
    })
}

impl Auth {
    pub fn new(client: reqwest::Client) -> Self {
        // TEMP: replaced by the full implementation at merge
        Self { client }
    }

    /// Headers to attach to a request for `url`. Only called for
    /// `*.onshape.com` hosts, including the sibling host the STL export
    /// redirects to. Errors with a user-facing message when the selected mode
    /// has no usable credentials, e.g. "Onshape API keys not set - open Settings".
    pub async fn headers_for(
        &self,
        app: &AppHandle,
        url: &reqwest::Url,
    ) -> Result<HeaderMap, String> {
        // TEMP: replaced by the full implementation at merge
        let (Some(access), Some(secret)) = (read_secret(ACCESS_KEY)?, read_secret(SECRET_KEY)?)
        else {
            return Err("Onshape API keys not set - open Settings".into());
        };
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{access}:{secret}"));
        let mut value =
            HeaderValue::from_str(&format!("Basic {token}")).map_err(|e| e.to_string())?;
        value.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, value);
        Ok(headers)
    }

    /// Called once when an onshape.com response came back 401 or 403.
    /// `Ok(true)`: credentials were refreshed (for example a fresh password
    /// login), so the caller retries the request once. `Ok(false)`: nothing to
    /// refresh, so the caller surfaces the original error.
    pub async fn handle_auth_failure(&self, app: &AppHandle) -> Result<bool, String> {
        // TEMP: replaced by the full implementation at merge
        Ok(false)
    }
}

#[tauri::command]
pub async fn get_auth_settings(
    app: AppHandle,
    auth: State<'_, Auth>,
) -> Result<AuthSettings, String> {
    // TEMP: replaced by the full implementation at merge
    current_settings()
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
    // TEMP: replaced by the full implementation at merge
    if input.mode != AuthMode::ApiKey {
        return Err("Only API keys are supported in this build".into());
    }
    write_secret(ACCESS_KEY, input.access_key)?;
    write_secret(SECRET_KEY, input.secret_key)?;
    current_settings()
}
