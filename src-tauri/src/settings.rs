//! Non-secret settings in the tauri-plugin-store file `settings.json` (in the
//! app data dir). Only individual keys are read and written; the file also
//! holds the `auth` and `slicers` sections owned by those modules.

use std::sync::Arc;
use std::time::SystemTime;

use tauri::{AppHandle, Wry};
use tauri_plugin_store::{Store, StoreExt};

pub const SETTINGS_FILE: &str = "settings.json";
pub const API_CALLS_TOTAL: &str = "apiCallsTotal";
pub const API_CALLS_SINCE: &str = "apiCallsSince";

pub fn store(app: &AppHandle) -> Result<Arc<Store<Wry>>, String> {
    app.store(SETTINGS_FILE)
        .map_err(|e| format!("cannot open {SETTINGS_FILE}: {e}"))
}

/// Writes pending changes now instead of on the autosave debounce.
pub fn flush(app: &AppHandle) -> Result<(), String> {
    store(app)?
        .save()
        .map_err(|e| format!("cannot save {SETTINGS_FILE}: {e}"))
}

/// Current UTC time as ISO 8601 with milliseconds, like JS toISOString().
pub fn now_iso() -> String {
    humantime::format_rfc3339_millis(SystemTime::now()).to_string()
}
