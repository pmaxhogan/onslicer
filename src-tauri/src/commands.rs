//! Tauri commands for the document browser. JSON shapes: src/types.ts.

use std::path::Path;

use tauri::ipc::Response;
use tauri::{AppHandle, State};

use crate::data;
use crate::model::*;
use crate::onshape::Onshape;
use crate::slicers::{self, Slicers};

#[tauri::command]
pub async fn get_docs(app: AppHandle, refresh: Refresh) -> Result<DocsResult, String> {
    data::get_docs(&app, refresh).await
}

#[tauri::command]
pub async fn get_elements(
    app: AppHandle,
    doc: Doc,
    refresh: Refresh,
) -> Result<ElementsResult, String> {
    data::get_elements(&app, &doc, refresh).await
}

#[tauri::command]
pub async fn peek_parts(
    app: AppHandle,
    doc_id: String,
    element_id: String,
) -> Result<Option<PartsCache>, String> {
    data::peek_parts(&app, &doc_id, &element_id).await
}

#[tauri::command]
pub async fn get_parts(
    app: AppHandle,
    doc: Doc,
    element_id: String,
    refresh: bool,
) -> Result<PartsResult, String> {
    data::get_parts(&app, &doc, &element_id, refresh).await
}

/// Raw PNG bytes (an ArrayBuffer in the UI); empty when the doc has no
/// thumbnail.
#[tauri::command]
pub async fn get_thumb(app: AppHandle, doc: Doc) -> Result<Response, String> {
    let bytes = data::get_thumb(&app, &doc).await?;
    Ok(Response::new(bytes.unwrap_or_default()))
}

/// Raw PNG bytes of a tab's thumbnail (`part` null) or one part's; empty when
/// it has none.
#[tauri::command]
pub async fn get_item_thumb(
    app: AppHandle,
    doc: Doc,
    el: Element,
    part: Option<Part>,
) -> Result<Response, String> {
    let bytes = data::get_item_thumb(&app, &doc, &el, part.as_ref()).await?;
    Ok(Response::new(bytes.unwrap_or_default()))
}

/// Re-export in place without opening anything.
#[tauri::command]
pub async fn save_export(app: AppHandle, target: ExportTarget) -> Result<SaveResult, String> {
    data::save(&app, &target).await
}

/// Export, then open the file in the selected slicer.
#[tauri::command]
pub async fn print_export(
    app: AppHandle,
    slicers: State<'_, Slicers>,
    target: ExportTarget,
) -> Result<SaveResult, String> {
    let res = data::save(&app, &target).await?;
    slicers::launch(&app, &slicers, Path::new(&res.file))?;
    Ok(res)
}

/// Export targets whose tab changed in Onshape since their last save.
#[tauri::command]
pub async fn check_exports(app: AppHandle) -> Result<ExportCheck, String> {
    data::check_exports(&app).await
}

#[tauri::command]
pub fn get_call_stats(app: AppHandle, onshape: State<'_, Onshape>) -> CallStats {
    onshape.call_stats(&app)
}
