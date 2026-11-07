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

use reqwest::header::HeaderMap;
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
pub struct Auth {}

impl Auth {
    pub fn new(client: reqwest::Client) -> Self {
        todo!()
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
        todo!()
    }

    /// Called once when an onshape.com response came back 401 or 403.
    /// `Ok(true)`: credentials were refreshed (for example a fresh password
    /// login), so the caller retries the request once. `Ok(false)`: nothing to
    /// refresh, so the caller surfaces the original error.
    pub async fn handle_auth_failure(&self, app: &AppHandle) -> Result<bool, String> {
        todo!()
    }
}

#[tauri::command]
pub async fn get_auth_settings(
    app: AppHandle,
    auth: State<'_, Auth>,
) -> Result<AuthSettings, String> {
    todo!()
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
    todo!()
}
