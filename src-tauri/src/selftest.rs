//! Headless end-to-end check against the real Onshape account. Runs instead of
//! the UI when ONSLICER_SELFTEST=1: prints one JSON line to stdout, writes
//! selftest-result.json in the app data dir, exits 0 on success and 1 on
//! failure. Never launches a slicer.

use std::io::Write;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::data;
use crate::model::{Element, ExportTarget, Refresh};
use crate::onshape::Onshape;
use crate::settings;

pub const ENV_VAR: &str = "ONSLICER_SELFTEST";

pub fn requested() -> bool {
    std::env::var(ENV_VAR).as_deref() == Ok("1")
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Step {
    name: &'static str,
    ok: bool,
    detail: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Report {
    started_at: String,
    steps: Vec<Step>,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    api_calls: u64,
    finished_at: String,
}

/// Runs the selftest, reports it and exits the process.
pub async fn run_and_exit(app: AppHandle) {
    let mut report = Report {
        started_at: settings::now_iso(),
        steps: Vec::new(),
        ok: false,
        error: None,
        api_calls: 0,
        finished_at: String::new(),
    };
    match steps(&app, &mut report.steps).await {
        Ok(()) => report.ok = report.steps.iter().all(|s| s.ok),
        Err(e) => report.error = Some(e),
    }
    report.api_calls = app.state::<Onshape>().session_calls();
    report.finished_at = settings::now_iso();

    let line = serde_json::to_string(&report)
        .unwrap_or_else(|e| format!("{{\"ok\":false,\"error\":\"{e}\"}}"));
    let mut stdout = std::io::stdout();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
    let mut code = if report.ok { 0 } else { 1 };
    if let Err(e) = write_result(&app, &report) {
        eprintln!("selftest: {e}");
        code = 1;
    }
    // The call counter autosaves on a debounce that exiting now would cut short.
    if report.api_calls > 0 {
        if let Err(e) = settings::flush(&app) {
            eprintln!("selftest: {e}");
        }
    }
    std::process::exit(code);
}

fn write_result(app: &AppHandle, report: &Report) -> Result<(), String> {
    let path = data::data_path(app, "selftest-result.json")?;
    let json = serde_json::to_vec_pretty(report).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, json).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

async fn steps(app: &AppHandle, out: &mut Vec<Step>) -> Result<(), String> {
    let mut step = |name: &'static str, ok: bool, detail: String| {
        out.push(Step { name, ok, detail });
    };

    let docs = data::get_docs(app, Refresh::Normal).await?;
    let n = docs.data.items.len();
    step(
        "docs",
        n > 0,
        format!("{n} docs, fromCache={}", docs.from_cache),
    );
    let doc = docs
        .data
        .items
        .iter()
        .find(|d| d.name == "button holders")
        .or(docs.data.items.first())
        .cloned()
        .ok_or("no documents")?;

    let thumb = data::get_thumb(app, &doc).await?;
    let png = thumb.as_deref().is_some_and(|b| b.starts_with(b"\x89PNG"));
    step(
        "thumb",
        png,
        match &thumb {
            Some(b) => format!("{} bytes, png={png}", b.len()),
            None => "no thumbnail".into(),
        },
    );

    let els = data::get_elements(app, &doc, Refresh::Normal).await?;
    step(
        "elements",
        !els.data.items.is_empty(),
        els.data
            .items
            .iter()
            .map(|e| format!("{}:{}", e.name, e.element_type))
            .collect::<Vec<_>>()
            .join(", "),
    );
    step(
        "no-3mf-blobs",
        !els.data
            .items
            .iter()
            .any(|e| e.name.to_ascii_lowercase().ends_with(".3mf")),
        "stale 3mf blobs hidden".into(),
    );

    let ps: Element = els
        .data
        .items
        .iter()
        .find(|e| e.element_type == "PARTSTUDIO")
        .cloned()
        .ok_or("no part studio in the test document")?;
    let parts = data::get_parts(app, &doc, &ps.id, true).await?;
    step(
        "parts",
        !parts.data.items.is_empty(),
        parts
            .data
            .items
            .iter()
            .map(|p| format!("{}({})", p.name, p.body_type))
            .collect::<Vec<_>>()
            .join(", "),
    );
    let is_png = |b: &Option<Vec<u8>>| b.as_deref().is_some_and(|b| b.starts_with(b"\x89PNG"));
    let tab_thumb = data::get_item_thumb(app, &doc, &ps, None).await?;
    step(
        "tab-thumb",
        is_png(&tab_thumb),
        format!("{} bytes", tab_thumb.as_ref().map_or(0, Vec::len)),
    );
    let part_thumb = match parts.data.items.first() {
        Some(p) => data::get_item_thumb(app, &doc, &ps, Some(p)).await?,
        None => None,
    };
    step(
        "part-thumb",
        is_png(&part_thumb),
        format!("{} bytes", part_thumb.as_ref().map_or(0, Vec::len)),
    );
    let solid = parts
        .data
        .items
        .iter()
        .find(|p| p.body_type == "solid")
        .or(parts.data.items.first())
        .cloned();
    let res = data::save(
        app,
        &ExportTarget {
            doc: doc.clone(),
            el: ps.clone(),
            part: solid,
        },
    )
    .await?;
    step(
        "export",
        res.bytes > 100,
        format!("{} bytes -> {}", res.bytes, res.file),
    );

    // Cache invalidation: a plain elements refresh keeps the parts cache of an
    // unchanged part studio (same microversionId); a full refresh drops it.
    let parts_cache = data::parts_path(app, &doc.id, &ps.id)?;
    let existed_before = parts_cache.is_file();
    let refreshed = data::get_elements(app, &doc, Refresh::Normal).await?;
    let kept_after_refresh = parts_cache.is_file();
    let has_mv = refreshed
        .data
        .items
        .iter()
        .find(|e| e.id == ps.id)
        .is_some_and(|e| e.microversion_id.is_some());
    data::get_elements(app, &doc, Refresh::Full).await?;
    let kept_after_full = parts_cache.is_file();
    step(
        "cache-invalidation",
        existed_before && has_mv && kept_after_refresh && !kept_after_full,
        format!(
            "parts cache before={existed_before} mv={has_mv} after-refresh={kept_after_refresh} after-full={kept_after_full}"
        ),
    );

    // Blob export (different endpoint + extension inference). 3MF blobs are
    // hidden from the UI, so reach one via the unfiltered cached element list.
    let raw = data::raw_elements(app, &doc.id)
        .await?
        .ok_or("elements cache missing")?;
    let blob = raw.items.into_iter().find(|e| {
        let name = e.name.to_ascii_lowercase();
        e.element_type == "BLOB" && (name.ends_with(".stl") || name.ends_with(".3mf"))
    });
    match blob {
        Some(el) => {
            let res = data::save(
                app,
                &ExportTarget {
                    doc: doc.clone(),
                    el,
                    part: None,
                },
            )
            .await?;
            step(
                "blob-export",
                res.bytes > 100 && (res.file.ends_with(".stl") || res.file.ends_with(".3mf")),
                format!("{} bytes -> {}", res.bytes, res.file),
            );
        }
        None => step(
            "blob-export",
            true,
            "no blob element available to test (skipped)".into(),
        ),
    }
    Ok(())
}
