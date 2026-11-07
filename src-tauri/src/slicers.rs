//! Slicer detection, selection and launch.
//!
//! CONTRACT FILE. The public items below (types, field names, serde shapes,
//! function signatures, `#[tauri::command]` names) are shared by the print
//! command, the frontend (src/types.ts) and the slicer implementation. Change
//! only the bodies and private items.
//!
//! Persisted in the tauri-plugin-store file `settings.json` under the key
//! `slicers`: `{ "manual": ["C:\\path\\to\\slicer.exe", ...], "selected": "path" | null }`.
//! Detected slicers are not persisted; they are rescanned at startup and on
//! `rescan_slicers`.

#![allow(dead_code, unused_variables)]

use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::{AppHandle, State};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Slicer {
    /// Display name, e.g. "Bambu Studio". Manual entries use the file stem.
    pub name: String,
    /// Absolute path of the executable; also the slicer's identity.
    pub path: String,
    /// Added through "Other...", as opposed to auto-detected.
    pub manual: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SlicerState {
    /// Detected slicers first (scan order), then manual entries (oldest
    /// first), de-duplicated by path.
    pub slicers: Vec<Slicer>,
    /// Path of the selected slicer, or null when none is available.
    pub selected: Option<String>,
}

/// Managed state, registered in lib.rs with `app.manage(Slicers::new())`.
pub struct Slicers {}

impl Slicers {
    pub fn new() -> Self {
        todo!()
    }
}

impl Default for Slicers {
    fn default() -> Self {
        Self::new()
    }
}

/// Opens `file` in the selected slicer. Errors with a user-facing message when
/// no slicer is selected or its executable is gone.
pub fn launch(app: &AppHandle, slicers: &Slicers, file: &Path) -> Result<(), String> {
    todo!()
}

/// Current state. The first call of a process scans; later calls reuse the
/// scan. Every call drops entries whose path is no longer a file.
#[tauri::command]
pub async fn list_slicers(
    app: AppHandle,
    slicers: State<'_, Slicers>,
) -> Result<SlicerState, String> {
    todo!()
}

/// Forces a rescan of the install locations.
#[tauri::command]
pub async fn rescan_slicers(
    app: AppHandle,
    slicers: State<'_, Slicers>,
) -> Result<SlicerState, String> {
    todo!()
}

/// "Other...": opens a native file picker for an executable, adds the pick as a
/// manual entry and selects it. Cancelling returns the unchanged state.
#[tauri::command]
pub async fn pick_manual_slicer(
    app: AppHandle,
    slicers: State<'_, Slicers>,
) -> Result<SlicerState, String> {
    todo!()
}

/// Selects one of the listed slicers by path.
#[tauri::command]
pub async fn select_slicer(
    app: AppHandle,
    slicers: State<'_, Slicers>,
    path: String,
) -> Result<SlicerState, String> {
    todo!()
}
