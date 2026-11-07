//! Documents, elements, parts, thumbnails and exports: Onshape fetches behind
//! on-disk caches in the app data dir.
//!
//!   cache/docs.json                       document list
//!   cache/elements/<docId>.json           element (tab) list per document
//!   cache/parts/<docId>-<elementId>.json  part list per part studio
//!   cache/thumbs/<docId>-<stamp>.png      document thumbnails
//!   exports/...                           exported models, see export_rel

use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use reqwest::Response;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::model::*;
use crate::onshape::{api_url, error_chain, Onshape, JSON_ACCEPT};
use crate::settings::now_iso;

const MAX_DOCS: usize = 200;

// ---------- paths and file helpers ----------

pub fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("no app data dir: {e}"))
}

/// `rel` ('/'-separated) under the app data dir.
pub fn data_path(app: &AppHandle, rel: &str) -> Result<PathBuf, String> {
    let mut path = data_dir(app)?;
    path.extend(rel.split('/'));
    Ok(path)
}

/// A missing or unreadable cache file is a cache miss.
pub async fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = tokio::fs::read(path).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

async fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    write_file(path, &bytes).await
}

async fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    tokio::fs::write(path, bytes)
        .await
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Deletes the files in `dir` whose name matches. A missing `dir` has nothing
/// to delete; any other error propagates.
pub async fn remove_matching(dir: &Path, matches: impl Fn(&str) -> bool) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("cannot clean {}: {e}", dir.display());
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(fail(e)),
    };
    while let Some(entry) = entries.next_entry().await.map_err(fail)? {
        if !entry.file_type().await.map_err(fail)?.is_file() {
            continue;
        }
        let name = entry.file_name();
        if name.to_str().is_some_and(&matches) {
            match tokio::fs::remove_file(entry.path()).await {
                Err(e) if e.kind() != ErrorKind::NotFound => return Err(fail(e)),
                _ => {}
            }
        }
    }
    Ok(())
}

// ---------- Onshape responses ----------

fn onshape(app: &AppHandle) -> tauri::State<'_, Onshape> {
    app.state::<Onshape>()
}

async fn fetch(app: &AppHandle, path: &str, accept: &str) -> Result<Response, String> {
    onshape(app).get(app, api_url(path)?, accept).await
}

/// "<what> <status>: <first 200 chars of the body>"
async fn http_error(what: &str, res: Response) -> String {
    let status = res.status().as_u16();
    let body = res.text().await.unwrap_or_default();
    format!("{what} {status}: {}", snippet(&body, 200))
}

async fn body_bytes(res: Response) -> Result<Vec<u8>, String> {
    res.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("download failed: {}", error_chain(&e)))
}

async fn body_json<T: DeserializeOwned>(what: &str, res: Response) -> Result<T, String> {
    let bytes = body_bytes(res).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{what}: unexpected response: {e}"))
}

fn workspace(doc: &Doc) -> Result<&str, String> {
    let ws = doc
        .workspace_id
        .as_deref()
        .ok_or_else(|| format!("document {} has no default workspace", doc.name))?;
    check_id("workspace", ws)?;
    Ok(ws)
}

// ---------- documents ----------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDocsPage {
    items: Option<Vec<RawDoc>>,
    next: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDoc {
    id: String,
    name: Option<String>,
    modified_at: Option<String>,
    default_workspace: Option<RawId>,
    thumbnail: Option<RawThumbnail>,
}

#[derive(Deserialize)]
struct RawId {
    id: Option<String>,
}

#[derive(Deserialize)]
struct RawThumbnail {
    sizes: Option<Vec<RawThumbSize>>,
}

#[derive(Deserialize)]
struct RawThumbSize {
    size: Option<String>,
    href: Option<String>,
}

fn doc_item(d: RawDoc) -> Doc {
    let sizes = d.thumbnail.and_then(|t| t.sizes).unwrap_or_default();
    let thumb_href = sizes
        .iter()
        .find(|s| s.size.as_deref() == Some("300x170"))
        .or(sizes.first())
        .and_then(|s| s.href.clone());
    Doc {
        id: d.id,
        name: d.name.unwrap_or_default(),
        modified_at: d.modified_at,
        workspace_id: d.default_workspace.and_then(|w| w.id),
        thumb_href,
    }
}

/// `Refresh::Normal` walks the modifiedAt-desc listing only until it reaches
/// a doc that is unchanged in the cache - everything after that is unchanged
/// too - so a typical refresh is one call. It cannot see deletions;
/// `Refresh::Full` re-lists everything (1 call per 20 docs).
pub async fn get_docs(app: &AppHandle, refresh: Refresh) -> Result<DocsResult, String> {
    let path = data_path(app, "cache/docs.json")?;
    let cached: Option<DocsCache> = read_json(&path).await;
    if refresh == Refresh::None {
        if let Some(data) = cached {
            return Ok(DocsResult {
                data,
                from_cache: true,
                new_calls: 0,
                changed: None,
            });
        }
    }

    let cached = cached.filter(|_| refresh != Refresh::Full);
    let known: HashMap<&str, &str> = cached
        .iter()
        .flat_map(|c| &c.items)
        .filter_map(|d| Some((d.id.as_str(), d.modified_at.as_deref()?)))
        .collect();
    let mut fresh = Vec::new();
    let mut reached_unchanged = false;
    let mut calls = 0;
    let mut next = Some(
        "/api/v6/documents?filter=0&limit=20&sortColumn=modifiedAt&sortOrder=desc".to_string(),
    );
    while let Some(page_url) = next.take() {
        if fresh.len() >= MAX_DOCS || reached_unchanged {
            break;
        }
        let res = fetch(app, &page_url, JSON_ACCEPT).await?;
        calls += 1;
        if !res.status().is_success() {
            return Err(http_error("documents", res).await);
        }
        let page: RawDocsPage = body_json("documents", res).await?;
        for d in page.items.unwrap_or_default() {
            let unchanged = d
                .modified_at
                .as_deref()
                .is_some_and(|m| known.get(d.id.as_str()) == Some(&m));
            if unchanged {
                reached_unchanged = true;
                break;
            }
            fresh.push(doc_item(d));
        }
        next = page.next.filter(|n| !n.is_empty());
    }

    let changed = fresh.len();
    let mut items = fresh;
    if let Some(cached) = &cached {
        let fresh_ids: HashSet<String> = items.iter().map(|d| d.id.clone()).collect();
        items.extend(
            cached
                .items
                .iter()
                .filter(|d| !fresh_ids.contains(&d.id))
                .cloned(),
        );
        items.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    }
    items.truncate(MAX_DOCS);
    let data = DocsCache {
        fetched_at: now_iso(),
        items,
    };
    write_json(&path, &data).await?;
    Ok(DocsResult {
        data,
        from_cache: false,
        new_calls: calls,
        changed: Some(changed),
    })
}

// ---------- elements ----------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawElement {
    id: String,
    name: Option<String>,
    element_type: Option<String>,
    data_type: Option<String>,
    microversion_id: Option<String>,
}

fn visible(mut data: ElementsCache) -> ElementsCache {
    data.items.retain(|e| !is_stale_3mf_blob(e));
    data
}

async fn fetch_elements(app: &AppHandle, doc: &Doc) -> Result<Vec<Element>, String> {
    let ws = workspace(doc)?;
    let res = fetch(
        app,
        &format!("/api/v6/documents/d/{}/w/{ws}/elements", doc.id),
        JSON_ACCEPT,
    )
    .await?;
    if !res.status().is_success() {
        return Err(http_error("elements", res).await);
    }
    let raw: Vec<RawElement> = body_json("elements", res).await?;
    Ok(raw
        .into_iter()
        .map(|e| {
            let name = e.name.unwrap_or_default();
            let element_type = e.element_type.unwrap_or_default();
            let data_type = e.data_type.filter(|t| !t.is_empty());
            Element {
                printable: is_printable(&element_type, data_type.as_deref(), &name),
                id: e.id,
                name,
                element_type,
                data_type,
                microversion_id: e.microversion_id,
            }
        })
        .collect())
}

/// `Refresh::Normal` refetches the element list (1 call) and drops only the
/// part lists of elements whose microversionId changed (or is unknown);
/// `Refresh::Full` drops every part list of this doc, as an escape hatch.
/// Without an explicit refresh, a cached list is still refetched when the doc
/// list says the doc changed since that list was cached, so after a top-level
/// refresh, opening an edited doc shows new tabs without a doc-level refresh.
pub async fn get_elements(
    app: &AppHandle,
    doc: &Doc,
    refresh: Refresh,
) -> Result<ElementsResult, String> {
    check_id("document", &doc.id)?;
    let path = data_path(app, &format!("cache/elements/{}.json", doc.id))?;
    let old: Option<ElementsCache> = read_json(&path).await;
    let from_old = |old: ElementsCache, stale_error: Option<String>| ElementsResult {
        data: visible(old),
        from_cache: true,
        stale_error,
    };
    if refresh == Refresh::None {
        if let Some(old) = &old {
            if !elements_stale(old, doc.modified_at.as_deref()) {
                return Ok(from_old(old.clone(), None));
            }
        }
    }
    let items = match fetch_elements(app, doc).await {
        Ok(items) => items,
        // An automatic (staleness-driven) refetch must never hide a usable
        // cache, e.g. offline or with the API allocation exhausted.
        Err(e) => match old {
            Some(old) if refresh == Refresh::None => return Ok(from_old(old, Some(e))),
            _ => return Err(e),
        },
    };
    let data = ElementsCache {
        fetched_at: now_iso(),
        doc_modified_at: doc.modified_at.clone(),
        items,
    };

    let mv = |items: &[Element]| -> HashMap<String, Option<String>> {
        items
            .iter()
            .map(|e| (e.id.clone(), e.microversion_id.clone()))
            .collect()
    };
    let old_mv = old.as_ref().map(|o| mv(&o.items)).unwrap_or_default();
    let new_mv = mv(&data.items);
    let prefix = format!("{}-", doc.id);
    remove_matching(&data_path(app, "cache/parts")?, |name| {
        let Some(rest) = name.strip_prefix(&prefix) else {
            return false;
        };
        if refresh == Refresh::Full {
            return true;
        }
        let eid = rest.strip_suffix(".json").unwrap_or(rest);
        match new_mv.get(eid).cloned().flatten() {
            None => true,
            Some(now) => old_mv.get(eid).cloned().flatten().as_ref() != Some(&now),
        }
    })
    .await?;
    write_json(&path, &data).await?;
    Ok(ElementsResult {
        data: visible(data),
        from_cache: false,
        stale_error: None,
    })
}

/// The cached element list including hidden 3MF blobs, if any.
pub async fn raw_elements(app: &AppHandle, doc_id: &str) -> Result<Option<ElementsCache>, String> {
    check_id("document", doc_id)?;
    Ok(read_json(&data_path(app, &format!("cache/elements/{doc_id}.json"))?).await)
}

// ---------- parts ----------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPart {
    part_id: String,
    name: Option<String>,
    body_type: Option<String>,
    is_mesh: Option<bool>,
}

pub fn parts_path(app: &AppHandle, doc_id: &str, element_id: &str) -> Result<PathBuf, String> {
    check_id("document", doc_id)?;
    check_id("element", element_id)?;
    data_path(app, &format!("cache/parts/{doc_id}-{element_id}.json"))
}

/// Disk-cached part list or None. Never hits the API, so the dialog filter can
/// match part names without a network round-trip.
pub async fn peek_parts(
    app: &AppHandle,
    doc_id: &str,
    element_id: &str,
) -> Result<Option<PartsCache>, String> {
    Ok(read_json(&parts_path(app, doc_id, element_id)?).await)
}

pub async fn get_parts(
    app: &AppHandle,
    doc: &Doc,
    element_id: &str,
    refresh: bool,
) -> Result<PartsResult, String> {
    let path = parts_path(app, &doc.id, element_id)?;
    if !refresh {
        if let Some(data) = read_json(&path).await {
            return Ok(PartsResult {
                data,
                from_cache: true,
            });
        }
    }
    let ws = workspace(doc)?;
    let res = fetch(
        app,
        &format!("/api/v6/parts/d/{}/w/{ws}/e/{element_id}", doc.id),
        JSON_ACCEPT,
    )
    .await?;
    if !res.status().is_success() {
        return Err(http_error("parts", res).await);
    }
    let raw: Vec<RawPart> = body_json("parts", res).await?;
    let data = PartsCache {
        fetched_at: now_iso(),
        items: raw
            .into_iter()
            .map(|p| Part {
                part_id: p.part_id,
                name: p.name.unwrap_or_default(),
                body_type: p.body_type.unwrap_or_default(),
                is_mesh: p.is_mesh.unwrap_or(false),
            })
            .collect(),
    };
    write_json(&path, &data).await?;
    Ok(PartsResult {
        data,
        from_cache: false,
    })
}

// ---------- thumbnails ----------

/// PNG bytes of the document's thumbnail, or None when it has none. Cached as
/// cache/thumbs/<thumb_key>.png, so a refresh only downloads thumbnails of new
/// or changed docs.
pub async fn get_thumb(app: &AppHandle, doc: &Doc) -> Result<Option<Vec<u8>>, String> {
    check_id("document", &doc.id)?;
    let Some(href) = doc.thumb_href.as_deref() else {
        return Ok(None);
    };
    let key = thumb_key(&doc.id, Some(href));
    let path = data_path(app, &format!("cache/thumbs/{key}.png"))?;
    match tokio::fs::read(&path).await {
        Ok(bytes) => return Ok(Some(bytes)),
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    }
    let res = fetch(app, href, "image/png").await?;
    if !res.status().is_success() {
        return Ok(None);
    }
    let bytes = body_bytes(res).await?;
    let prefix = format!("{}-", doc.id);
    remove_matching(&data_path(app, "cache/thumbs")?, |name| {
        name.starts_with(&prefix)
    })
    .await?;
    write_file(&path, &bytes).await?;
    Ok(Some(bytes))
}

// ---------- export ----------

/// Exports from Onshape and writes to the stable path. Launches nothing.
pub async fn save(app: &AppHandle, target: &ExportTarget) -> Result<SaveResult, String> {
    let ExportTarget { doc, el, part } = target;
    check_id("document", &doc.id)?;
    check_id("element", &el.id)?;
    let ws = workspace(doc)?;
    let (bytes, ext) = match el.element_type.as_str() {
        "PARTSTUDIO" => {
            let mut url = api_url(&format!(
                "/api/v6/partstudios/d/{}/w/{ws}/e/{}/stl?units=millimeter&mode=binary&grouping=true",
                doc.id, el.id
            ))?;
            if let Some(p) = part {
                url.query_pairs_mut().append_pair("partIds", &p.part_id);
            }
            let res = onshape(app).get(app, url, "*/*").await?;
            if !res.status().is_success() {
                let status = res.status().as_u16();
                let body = res.text().await.unwrap_or_default();
                return Err(format!(
                    "STL export failed ({status}): {}",
                    snippet(&body, 200)
                ));
            }
            (body_bytes(res).await?, ".stl")
        }
        "BLOB" => {
            let res = fetch(
                app,
                &format!("/api/v6/blobelements/d/{}/w/{ws}/e/{}", doc.id, el.id),
                "*/*",
            )
            .await?;
            if !res.status().is_success() {
                return Err(format!("blob download failed ({})", res.status().as_u16()));
            }
            (
                body_bytes(res).await?,
                blob_ext(el.data_type.as_deref(), &el.name),
            )
        }
        other => return Err(format!("unsupported element type {other}")),
    };

    let (_, rel) = export_rel(
        &doc.name,
        &el.name,
        part.as_ref().map(|p| p.name.as_str()),
        ext,
    );
    let path = data_path(app, &rel)?;
    write_file(&path, &bytes).await?;
    Ok(SaveResult {
        file: path.to_string_lossy().into_owned(),
        bytes: bytes.len(),
    })
}
