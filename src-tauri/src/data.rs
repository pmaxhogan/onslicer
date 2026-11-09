//! Documents, elements, parts, thumbnails and exports: Onshape fetches behind
//! on-disk caches in the app data dir.
//!
//!   cache/docs.json                       document list
//!   cache/elements/<docId>.json           element (tab) list per document
//!   cache/parts/<docId>-<elementId>.json  part list per part studio
//!   cache/thumbs/<docId>-<stamp>.png      document thumbnails
//!   cache/thumbs/elements/<docId>-<elementId>-<stamp>.png        tab thumbnails
//!   cache/thumbs/parts/<docId>-<elementId>-<partId>-<stamp>.png  part thumbnails
//!   cache/exported.json                   tab microversion at each last export
//!   exports/...                           exported models, see export_rel

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use reqwest::Response;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::async_runtime::Mutex as AsyncMutex;
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

/// The 300x170 size (the gallery card's aspect ratio), else the first one.
fn pick_thumb(thumb: Option<RawThumbnail>) -> Option<String> {
    let sizes = thumb.and_then(|t| t.sizes).unwrap_or_default();
    sizes
        .iter()
        .find(|s| s.size.as_deref() == Some("300x170"))
        .or(sizes.first())
        .and_then(|s| s.href.clone())
}

fn doc_item(d: RawDoc) -> Doc {
    Doc {
        id: d.id,
        name: d.name.unwrap_or_default(),
        modified_at: d.modified_at,
        workspace_id: d.default_workspace.and_then(|w| w.id),
        thumb_href: pick_thumb(d.thumbnail),
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
    thumbnail_info: Option<RawThumbnail>,
}

async fn fetch_elements(app: &AppHandle, doc: &Doc) -> Result<Vec<Element>, String> {
    let ws = workspace(doc)?;
    let res = fetch(
        app,
        &format!(
            "/api/v6/documents/d/{}/w/{ws}/elements?withThumbnails=true",
            doc.id
        ),
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
                thumb_href: pick_thumb(e.thumbnail_info),
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
        with_thumbs: true,
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
    thumbnail_info: Option<RawThumbnail>,
}

pub fn parts_path(app: &AppHandle, doc_id: &str, element_id: &str) -> Result<PathBuf, String> {
    check_id("document", doc_id)?;
    check_id("element", element_id)?;
    data_path(app, &format!("cache/parts/{doc_id}-{element_id}.json"))
}

/// A cached part list, unless it predates thumbnails (then it is refetched).
async fn read_parts(path: &Path) -> Option<PartsCache> {
    read_json::<PartsCache>(path)
        .await
        .filter(|p| p.with_thumbs)
}

/// Disk-cached part list or None. Never hits the API, so the dialog filter can
/// match part names without a network round-trip.
pub async fn peek_parts(
    app: &AppHandle,
    doc_id: &str,
    element_id: &str,
) -> Result<Option<PartsCache>, String> {
    Ok(read_parts(&parts_path(app, doc_id, element_id)?).await)
}

pub async fn get_parts(
    app: &AppHandle,
    doc: &Doc,
    element_id: &str,
    refresh: bool,
) -> Result<PartsResult, String> {
    let path = parts_path(app, &doc.id, element_id)?;
    if !refresh {
        if let Some(data) = read_parts(&path).await {
            return Ok(PartsResult {
                data,
                from_cache: true,
            });
        }
    }
    let ws = workspace(doc)?;
    let res = fetch(
        app,
        &format!(
            "/api/v6/parts/d/{}/w/{ws}/e/{element_id}?withThumbnails=true",
            doc.id
        ),
        JSON_ACCEPT,
    )
    .await?;
    if !res.status().is_success() {
        return Err(http_error("parts", res).await);
    }
    let raw: Vec<RawPart> = body_json("parts", res).await?;
    let data = PartsCache {
        fetched_at: now_iso(),
        with_thumbs: true,
        items: raw
            .into_iter()
            .map(|p| Part {
                part_id: p.part_id,
                name: p.name.unwrap_or_default(),
                body_type: p.body_type.unwrap_or_default(),
                is_mesh: p.is_mesh.unwrap_or(false),
                thumb_href: pick_thumb(p.thumbnail_info),
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
    cached_thumb(app, "cache/thumbs", &doc.id, doc.thumb_href.as_deref()).await
}

/// PNG bytes of a tab's thumbnail (`part` None) or of one part's, or None when
/// it has none. Cached like document thumbnails, in their own directories so a
/// document thumbnail's cleanup never touches them.
pub async fn get_item_thumb(
    app: &AppHandle,
    doc: &Doc,
    el: &Element,
    part: Option<&Part>,
) -> Result<Option<Vec<u8>>, String> {
    check_id("document", &doc.id)?;
    check_id("element", &el.id)?;
    match part {
        None => {
            let prefix = format!("{}-{}", doc.id, el.id);
            cached_thumb(
                app,
                "cache/thumbs/elements",
                &prefix,
                el.thumb_href.as_deref(),
            )
            .await
        }
        Some(p) => {
            let prefix = format!("{}-{}-{}", doc.id, el.id, part_file_id(&p.part_id));
            cached_thumb(app, "cache/thumbs/parts", &prefix, p.thumb_href.as_deref()).await
        }
    }
}

/// `<dir>/<prefix>-<stamp>.png`, downloaded from `href` on a miss. A download
/// replaces the older stamps of the same prefix; the trailing '-' keeps that
/// cleanup from matching another item whose prefix extends this one.
async fn cached_thumb(
    app: &AppHandle,
    dir: &str,
    prefix: &str,
    href: Option<&str>,
) -> Result<Option<Vec<u8>>, String> {
    let Some(href) = href else {
        return Ok(None);
    };
    let key = thumb_key(prefix, Some(href));
    let path = data_path(app, &format!("{dir}/{key}.png"))?;
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
    let old = format!("{prefix}-");
    remove_matching(&data_path(app, dir)?, |name| name.starts_with(&old)).await?;
    write_file(&path, &bytes).await?;
    Ok(Some(bytes))
}

// ---------- export ----------

/// Exports from Onshape and writes to the stable path. Launches nothing.
/// Records the tab's `tab_signature` for `check_exports`, read from a fresh
/// element list (1 call) taken *before* the export: an edit made during the
/// export then shows as a change instead of being hidden.
pub async fn save(app: &AppHandle, target: &ExportTarget) -> Result<SaveResult, String> {
    let ExportTarget { doc, el, part } = target;
    check_id("document", &doc.id)?;
    check_id("element", &el.id)?;
    let ws = workspace(doc)?;
    let mut current: HashMap<(String, String), Option<String>> =
        match get_elements(app, doc, Refresh::Normal).await {
            Ok(r) => r
                .data
                .items
                .into_iter()
                .map(|e| ((doc.id.clone(), e.id), e.microversion_id))
                .collect(),
            Err(_) => HashMap::new(),
        };
    current
        .entry((doc.id.clone(), el.id.clone()))
        .or_insert_with(|| el.microversion_id.clone());
    let mut deps = vec![];
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
        "ASSEMBLY" => {
            let def: AssemblyDef = fetch_json(
                app,
                &format!("/api/v6/assemblies/d/{}/w/{ws}/e/{}", doc.id, el.id),
                "assembly",
            )
            .await?;
            if def.root_assembly.instances.is_empty() {
                return Err("this assembly is empty".into());
            }
            deps = def.same_doc_deps(&doc.id, &el.id);
            (export_assembly(app, doc, ws, el).await?, ".stl")
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
    let key = export_key(&doc.id, &el.id, part.as_ref().map(|p| p.part_id.as_str()));
    let record = ExportRecord {
        doc_id: doc.id.clone(),
        element_id: el.id.clone(),
        element_microversion: tab_signature(&doc.id, &el.id, &deps, &current),
        deps,
        exported_at: now_iso(),
    };
    update_records(app, |r| {
        r.items.insert(key, record);
    })
    .await?;
    Ok(SaveResult {
        file: path.to_string_lossy().into_owned(),
        bytes: bytes.len(),
    })
}

async fn fetch_json<T: DeserializeOwned>(
    app: &AppHandle,
    path: &str,
    what: &str,
) -> Result<T, String> {
    let res = fetch(app, path, JSON_ACCEPT).await?;
    if !res.status().is_success() {
        return Err(http_error(what, res).await);
    }
    res.json()
        .await
        .map_err(|e| format!("bad {what} response: {}", error_chain(&e)))
}

/// How long an assembly export may take before it is given up on.
const TRANSLATION_TIMEOUT: Duration = Duration::from_secs(300);

/// Onshape has no direct STL download for assemblies: start a translation,
/// poll it until done, then download the result (one STL of every part in
/// its assembled position). 3 calls plus one per poll; the polls back off
/// from 1 to 5 seconds.
async fn export_assembly(
    app: &AppHandle,
    doc: &Doc,
    ws: &str,
    el: &Element,
) -> Result<Vec<u8>, String> {
    let url = api_url(&format!(
        "/api/v6/assemblies/d/{}/w/{ws}/e/{}/translations",
        doc.id, el.id
    ))?;
    let body = json!({
        "formatName": "STL",
        "storeInDocument": false,
        "grouping": true,
        "stlMode": "BINARY",
        "destinationName": el.name,
    });
    let res = onshape(app).post_json(app, url, &body).await?;
    if !res.status().is_success() {
        return Err(http_error("assembly export", res).await);
    }
    let mut t: Translation = res
        .json()
        .await
        .map_err(|e| format!("bad assembly export response: {}", error_chain(&e)))?;
    check_id("translation", &t.id)?;
    let started = Instant::now();
    let mut delay = Duration::from_secs(1);
    while t.request_state == "ACTIVE" {
        if started.elapsed() > TRANSLATION_TIMEOUT {
            return Err("assembly export timed out in Onshape".into());
        }
        tokio::time::sleep(delay).await;
        delay = (delay + Duration::from_secs(1)).min(Duration::from_secs(5));
        let id = t.id.clone();
        t = fetch_json(
            app,
            &format!("/api/v6/translations/{id}"),
            "assembly export",
        )
        .await?;
        t.id = id;
    }
    if t.request_state != "DONE" {
        let why = t.failure_reason.unwrap_or_else(|| t.request_state.clone());
        return Err(format!("assembly export failed in Onshape: {why}"));
    }
    let fid = t
        .result_external_data_ids
        .and_then(|ids| ids.into_iter().next())
        .ok_or("assembly export finished without a file")?;
    check_id("export file", &fid)?;
    let res = fetch(
        app,
        &format!("/api/v6/documents/d/{}/externaldata/{fid}", doc.id),
        "*/*",
    )
    .await?;
    if !res.status().is_success() {
        return Err(http_error("assembly download", res).await);
    }
    let mut bytes = body_bytes(res).await?;
    stl_to_mm(&mut bytes)?;
    Ok(bytes)
}

// ---------- change checks ----------

/// Serializes read-modify-write of cache/exported.json between parallel saves.
static RECORDS_LOCK: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));

fn records_path(app: &AppHandle) -> Result<PathBuf, String> {
    data_path(app, "cache/exported.json")
}

async fn update_records(app: &AppHandle, f: impl FnOnce(&mut ExportRecords)) -> Result<(), String> {
    let _guard = RECORDS_LOCK.lock().await;
    let path = records_path(app)?;
    let mut records: ExportRecords = read_json(&path).await.unwrap_or_default();
    f(&mut records);
    write_json(&path, &records).await
}

/// Which exported targets changed in Onshape since their last save. Nothing
/// exported: no API call. Otherwise the incremental document list (usually 1
/// call), then each exported-from document's element list only when that
/// document changed (`get_elements` refetches on a new modifiedAt).
pub async fn check_exports(app: &AppHandle) -> Result<ExportCheck, String> {
    let records: ExportRecords = read_json(&records_path(app)?).await.unwrap_or_default();
    if records.items.is_empty() {
        return Ok(ExportCheck {
            stale: vec![],
            docs: None,
        });
    }
    let docs = get_docs(app, Refresh::Normal).await?;
    let doc_ids: BTreeSet<&str> = records.items.values().map(|r| r.doc_id.as_str()).collect();
    let mut current = HashMap::new();
    for id in doc_ids {
        let Some(doc) = docs.data.items.iter().find(|d| d.id == id) else {
            continue;
        };
        // A failed refetch leaves the tabs of this doc unknown, not stale.
        let Ok(els) = get_elements(app, doc, Refresh::None).await else {
            continue;
        };
        if els.stale_error.is_some() {
            continue;
        }
        for e in els.data.items {
            current.insert((doc.id.clone(), e.id), e.microversion_id);
        }
    }
    Ok(ExportCheck {
        stale: stale_keys(&records, &current),
        docs: Some(docs),
    })
}
