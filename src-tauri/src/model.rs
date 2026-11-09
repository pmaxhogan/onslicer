//! Data shapes shared by the disk caches, the Tauri commands and the frontend
//! (src/types.ts), plus the pure helpers that decide paths and staleness.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use reqwest::Url;
use serde::{Deserialize, Serialize};

/// How hard a fetch should try to bypass the disk cache.
#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Refresh {
    /// Use the cache when it is usable.
    #[default]
    None,
    /// Refetch, but keep whatever provably did not change (documents:
    /// incremental listing; elements: keep part lists of unchanged studios).
    Normal,
    /// Refetch everything and drop dependent caches.
    Full,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Doc {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub modified_at: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub thumb_href: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Element {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub element_type: String,
    #[serde(default)]
    pub data_type: Option<String>,
    #[serde(default)]
    pub microversion_id: Option<String>,
    #[serde(default)]
    pub printable: bool,
    /// The 300x170 thumbnail, when Onshape has one for this tab.
    #[serde(default)]
    pub thumb_href: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub part_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub body_type: String,
    #[serde(default)]
    pub is_mesh: bool,
    /// The 300x170 thumbnail, when Onshape has one for this part.
    #[serde(default)]
    pub thumb_href: Option<String>,
}

/// cache/docs.json
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DocsCache {
    pub fetched_at: String,
    pub items: Vec<Doc>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DocsResult {
    #[serde(flatten)]
    pub data: DocsCache,
    pub from_cache: bool,
    pub new_calls: u32,
    /// Docs that were new or changed; only set on a fetch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed: Option<usize>,
}

/// cache/elements/<docId>.json. Holds every element, including the 3MF blobs
/// the UI hides.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ElementsCache {
    pub fetched_at: String,
    /// The document's modifiedAt when this list was fetched.
    #[serde(default)]
    pub doc_modified_at: Option<String>,
    /// Fetched with thumbnail hrefs. A list cached before thumbnails existed
    /// lacks them and is refetched once (see `elements_stale`).
    #[serde(default)]
    pub with_thumbs: bool,
    pub items: Vec<Element>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ElementsResult {
    #[serde(flatten)]
    pub data: ElementsCache,
    pub from_cache: bool,
    /// Set when the doc changed but the refetch failed, so a cached list is shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_error: Option<String>,
}

/// cache/parts/<docId>-<elementId>.json
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PartsCache {
    pub fetched_at: String,
    /// Fetched with thumbnail hrefs; a list without them counts as a cache miss.
    #[serde(default)]
    pub with_thumbs: bool,
    pub items: Vec<Part>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PartsResult {
    #[serde(flatten)]
    pub data: PartsCache,
    pub from_cache: bool,
}

/// What to export: a whole part studio or blob (`part` null) or one part.
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExportTarget {
    pub doc: Doc,
    pub el: Element,
    #[serde(default)]
    pub part: Option<Part>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    /// Absolute path of the written file.
    pub file: String,
    pub bytes: usize,
}

/// cache/exported.json: per export target (see `export_key`), the tab's
/// microversion when it was last saved or printed.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExportRecords {
    #[serde(default)]
    pub items: BTreeMap<String, ExportRecord>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportRecord {
    pub doc_id: String,
    pub element_id: String,
    /// The tab's `tab_signature` at the export. None when it could not be
    /// determined; such a record is never stale.
    #[serde(default)]
    pub element_microversion: Option<String>,
    /// Same-document tabs an assembly references; empty for other tabs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<String>,
    pub exported_at: String,
}

/// invoke("check_exports"): the export targets whose tab changed in Onshape
/// since their last save, plus the document list fetched to find out (None
/// when nothing has been exported, which costs no API call).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExportCheck {
    pub stale: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<DocsResult>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CallStats {
    pub session: u64,
    pub total: u64,
    pub since: Option<String>,
}

// ---------- pure helpers ----------

/// Onshape ids (documents, workspaces, elements) are short alphanumeric
/// strings. They end up in cache file names, so anything else is rejected
/// rather than allowed to walk the file system.
pub fn check_id(what: &str, id: &str) -> Result<(), String> {
    if !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(())
    } else {
        Err(format!("invalid {what} id"))
    }
}

/// A file name segment Windows accepts: anything outside `[A-Za-z0-9_.- ]`
/// collapses to `_`, whitespace runs to one space, at most 80 characters, and
/// no trailing `.` or space (Windows rejects those).
pub fn sanitize(name: &str) -> String {
    let name = if name.is_empty() { "model" } else { name };
    let mut out = String::with_capacity(name.len());
    let mut in_bad = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | ' ') {
            if !(c == ' ' && out.ends_with(' ')) {
                out.push(c);
            }
            in_bad = false;
        } else if !in_bad {
            out.push('_');
            in_bad = true;
        }
    }
    let capped: String = out.trim().chars().take(80).collect();
    let trimmed = capped.trim_end_matches(['.', ' ']);
    if trimmed.is_empty() {
        "model".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 3MF blob tabs are frozen snapshots of an export - they silently go stale
/// when the source part studio changes, so they are hidden entirely.
pub fn is_stale_3mf_blob(el: &Element) -> bool {
    el.element_type == "BLOB"
        && (el.data_type.as_deref() == Some("application/3mf") || ends_with_ci(&el.name, ".3mf"))
}

/// An element list as shown: stale 3MF blobs dropped, and `printable`
/// recomputed, since a list cached by an older version may predate a newly
/// printable element type (assemblies).
pub fn visible(mut data: ElementsCache) -> ElementsCache {
    data.items.retain(|e| !is_stale_3mf_blob(e));
    for e in &mut data.items {
        e.printable = is_printable(&e.element_type, e.data_type.as_deref(), &e.name);
    }
    data
}

/// File extension of a blob whose data type marks it printable.
fn printable_blob_ext(data_type: Option<&str>) -> Option<&'static str> {
    match data_type {
        Some("model/stl" | "application/sla") => Some(".stl"),
        _ => None,
    }
}

pub fn is_printable(element_type: &str, data_type: Option<&str>, name: &str) -> bool {
    element_type == "PARTSTUDIO"
        || element_type == "ASSEMBLY"
        || (element_type == "BLOB"
            && (printable_blob_ext(data_type).is_some() || ends_with_ci(name, ".stl")))
}

/// Extension to save a blob under: from its data type, else its name, else .stl.
pub fn blob_ext(data_type: Option<&str>, name: &str) -> &'static str {
    if let Some(ext) = printable_blob_ext(data_type) {
        ext
    } else if ends_with_ci(name, ".3mf") {
        ".3mf"
    } else {
        ".stl"
    }
}

/// Whether a cached element list must be refetched for a document whose
/// current modifiedAt is `doc_modified_at`. A list without a recorded
/// docModifiedAt, or fetched without thumbnails, is stale.
pub fn elements_stale(old: &ElementsCache, doc_modified_at: Option<&str>) -> bool {
    if !old.with_thumbs {
        return true;
    }
    match doc_modified_at {
        None => false,
        Some(now) => old.doc_modified_at.as_deref() != Some(now),
    }
}

/// Thumbnail cache key `<prefix>-<stamp>`. The stamp changes whenever the
/// image does: the href's `t` parameter (documents and tabs, bumped by Onshape
/// on change), else the content hash in a `/thumbnails/<hash>/s/<size>` path
/// (parts), else `0`. `prefix` must already be file-name safe.
pub fn thumb_key(prefix: &str, thumb_href: Option<&str>) -> String {
    let url = thumb_href.and_then(|href| Url::parse(href).ok());
    let t = url.as_ref().and_then(|url| {
        url.query_pairs()
            .find(|(k, _)| k == "t")
            .map(|(_, v)| v.into_owned())
    });
    let hash = || {
        let mut segs = url.as_ref()?.path_segments()?;
        segs.find(|s| *s == "thumbnails")?;
        let h = segs.next()?;
        (h.len() >= 16 && h.bytes().all(|b| b.is_ascii_hexdigit())).then(|| h.to_owned())
    };
    let stamp: String = t
        .filter(|t| !t.is_empty())
        .or_else(hash)
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if stamp.is_empty() {
        format!("{prefix}-0")
    } else {
        format!("{prefix}-{stamp}")
    }
}

/// A part id as a file-name segment. Part ids are short alphanumeric strings in
/// practice; anything else is replaced by a hex hash of the id.
pub fn part_file_id(part_id: &str) -> String {
    if check_id("part", part_id).is_ok() {
        return part_id.to_owned();
    }
    // FNV-1a: stable across runs and builds, unlike std's DefaultHasher.
    let hash = part_id.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("x{hash:016x}")
}

/// Stable export path per (doc, element, part), relative to the app data dir
/// and '/'-separated, named after the thing itself so the slicer shows a
/// meaningful object name. Because the path never changes, a save can
/// overwrite it and the slicer's "Reload from disk" picks up new geometry:
///   exports/<doc>/<tab>.stl            whole part studio or blob
///   exports/<doc>/<tab>/<part>.stl     single part
/// Returns (dir, file).
pub fn export_rel(
    doc_name: &str,
    el_name: &str,
    part_name: Option<&str>,
    ext: &str,
) -> (String, String) {
    let dir = match part_name {
        Some(_) => format!("exports/{}/{}", sanitize(doc_name), sanitize(el_name)),
        None => format!("exports/{}", sanitize(doc_name)),
    };
    let mut base = sanitize(part_name.unwrap_or(el_name));
    if ends_with_ci(&base, ".stl") || ends_with_ci(&base, ".3mf") {
        // sanitize() output is ASCII, so the last 4 bytes are the extension.
        base.truncate(base.len() - 4);
    }
    let file = format!("{dir}/{base}{ext}");
    (dir, file)
}

fn ends_with_ci(s: &str, suffix: &str) -> bool {
    s.len() >= suffix.len()
        && s.is_char_boundary(s.len() - suffix.len())
        && s[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// `<docId>/<elementId>/<partId or empty>`; src/ui.ts `exportKey` builds the
/// same string for the Save buttons.
pub fn export_key(doc_id: &str, element_id: &str, part_id: Option<&str>) -> String {
    format!("{doc_id}/{element_id}/{}", part_id.unwrap_or(""))
}

/// What an export record compares: the tab's microversion, followed for an
/// assembly by `|<tab>=<microversion>` for each same-document tab it
/// references (sorted), since editing a referenced part studio need not move
/// the assembly's own microversion. None when any of them is unknown.
pub fn tab_signature(
    doc_id: &str,
    element_id: &str,
    deps: &[String],
    current: &HashMap<(String, String), Option<String>>,
) -> Option<String> {
    let mv = |el: &str| {
        current
            .get(&(doc_id.to_string(), el.to_string()))
            .cloned()
            .flatten()
    };
    let mut sig = mv(element_id)?;
    let mut deps: Vec<&String> = deps.iter().collect();
    deps.sort();
    deps.dedup();
    for dep in deps {
        sig.push_str(&format!("|{dep}={}", mv(dep)?));
    }
    Some(sig)
}

/// Keys of the records whose tab (or, for an assembly, a tab it references)
/// now has a different microversion. A tab that is gone, or whose current or
/// recorded microversion is unknown, is not stale: there is nothing reliable
/// to compare.
pub fn stale_keys(
    records: &ExportRecords,
    current: &HashMap<(String, String), Option<String>>,
) -> Vec<String> {
    records
        .items
        .iter()
        .filter(|(_, r)| {
            let now = tab_signature(&r.doc_id, &r.element_id, &r.deps, current);
            matches!((now, r.element_microversion.as_deref()), (Some(a), Some(b)) if a != b)
        })
        .map(|(k, _)| k.clone())
        .collect()
}

/// The parts of an assembly definition (`GET /assemblies/...`) that matter
/// for an export: is there anything in it, and which tabs of its own
/// document it pulls parts or sub-assemblies from.
#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AssemblyDef {
    pub root_assembly: SubAssembly,
    pub sub_assemblies: Vec<SubAssembly>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SubAssembly {
    pub instances: Vec<AsmInstance>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AsmInstance {
    pub document_id: String,
    pub element_id: String,
}

impl AssemblyDef {
    /// Tabs of `doc_id` other than `self_id` that the assembly references,
    /// sorted and deduplicated. References into other documents point at
    /// versions, which never change.
    pub fn same_doc_deps(&self, doc_id: &str, self_id: &str) -> Vec<String> {
        let set: BTreeSet<String> = std::iter::once(&self.root_assembly)
            .chain(&self.sub_assemblies)
            .flat_map(|a| &a.instances)
            .filter(|i| {
                i.document_id == doc_id && !i.element_id.is_empty() && i.element_id != self_id
            })
            .map(|i| i.element_id.clone())
            .collect();
        set.into_iter().collect()
    }
}

/// Rescales a binary STL to millimeters, in place. Onshape's assembly STL
/// translation ignores the requested unit and writes meters, saying so in the
/// header ("... Units = meters"); slicers read STL as millimeters. A header
/// without a unit is left alone.
pub fn stl_to_mm(bytes: &mut [u8]) -> Result<(), String> {
    let n = match bytes.get(80..84) {
        Some(b) => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize,
        None => return Err("export is not a binary STL".into()),
    };
    if bytes.len() != 84 + 50 * n {
        return Err("export is not a single binary STL".into());
    }
    let header = String::from_utf8_lossy(&bytes[..80]).to_ascii_lowercase();
    let Some(unit) = header.split("units =").nth(1) else {
        return Ok(());
    };
    let unit = unit.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    let unit = unit
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or("");
    let scale: f32 = match unit {
        "meter" | "meters" | "m" => 1000.0,
        "centimeter" | "centimeters" | "cm" => 10.0,
        "millimeter" | "millimeters" | "mm" => return Ok(()),
        "inch" | "inches" | "in" => 25.4,
        "foot" | "feet" | "ft" => 304.8,
        other => return Err(format!("export has an unknown unit ({other})")),
    };
    for t in 0..n {
        // Each triangle: normal (3 floats, unchanged), 3 vertices, attribute.
        for k in 0..9 {
            let o = 84 + 50 * t + 12 + 4 * k;
            let v = f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
            bytes[o..o + 4].copy_from_slice(&(v * scale).to_le_bytes());
        }
    }
    let mut h = [0u8; 80];
    let text = b"STL generated by ONSHAPE, rescaled by onslicer. Units = millimeters";
    h[..text.len()].copy_from_slice(text);
    bytes[..80].copy_from_slice(&h);
    Ok(())
}

/// `GET /translations/{id}`, and the response to starting one.
#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Translation {
    pub id: String,
    /// ACTIVE, DONE or FAILED.
    pub request_state: String,
    pub failure_reason: Option<String>,
    pub result_external_data_ids: Option<Vec<String>>,
}

/// First `n` characters of a response body, for error messages.
pub fn snippet(body: &str, n: usize) -> String {
    body.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(element_type: &str, data_type: Option<&str>, name: &str) -> Element {
        Element {
            id: "e1".into(),
            name: name.into(),
            element_type: element_type.into(),
            data_type: data_type.map(Into::into),
            microversion_id: None,
            printable: false,
            thumb_href: None,
        }
    }

    fn cache(doc_modified_at: Option<&str>) -> ElementsCache {
        ElementsCache {
            fetched_at: "2026-01-15T00:00:00Z".into(),
            doc_modified_at: doc_modified_at.map(Into::into),
            with_thumbs: true,
            items: vec![],
        }
    }

    #[test]
    fn sanitize_replaces_characters_windows_rejects() {
        assert_eq!(sanitize("a/b\\c:d*e?f\"g<h>i|j"), "a_b_c_d_e_f_g_h_i_j");
    }

    #[test]
    fn sanitize_collapses_runs_of_invalid_characters() {
        assert_eq!(sanitize("a//b"), "a_b");
        assert_eq!(sanitize("tab\there"), "tab_here");
    }

    #[test]
    fn sanitize_collapses_whitespace_and_trims_trailing_dots_and_spaces() {
        assert_eq!(sanitize("  my   part. . "), "my part");
    }

    #[test]
    fn sanitize_falls_back_to_model() {
        assert_eq!(sanitize(""), "model");
        assert_eq!(sanitize("..."), "model");
    }

    #[test]
    fn sanitize_caps_the_length_at_80_characters() {
        assert_eq!(sanitize(&"x".repeat(200)).len(), 80);
    }

    #[test]
    fn stale_3mf_blobs_are_flagged_by_data_type_or_name() {
        assert!(is_stale_3mf_blob(&el("BLOB", Some("application/3mf"), "")));
        assert!(is_stale_3mf_blob(&el("BLOB", None, "Plate.3MF")));
    }

    #[test]
    fn stl_blobs_and_part_studios_are_not_stale_3mf() {
        assert!(!is_stale_3mf_blob(&el("BLOB", Some("model/stl"), "a.stl")));
        assert!(!is_stale_3mf_blob(&el("PARTSTUDIO", None, "x.3mf")));
    }

    #[test]
    fn visible_recomputes_printable_and_hides_3mf_blobs() {
        let mut c = cache(None);
        c.items = vec![
            el("ASSEMBLY", None, "Asm"),
            el("BLOB", Some("application/3mf"), "Plate.3mf"),
            el("VARIABLESTUDIO", None, "vars"),
        ];
        let v = visible(c);
        let got: Vec<_> = v
            .items
            .iter()
            .map(|e| (e.name.as_str(), e.printable))
            .collect();
        assert_eq!(got, vec![("Asm", true), ("vars", false)]);
    }

    #[test]
    fn printable_and_blob_ext() {
        assert!(is_printable("PARTSTUDIO", None, "x"));
        assert!(is_printable("BLOB", Some("application/sla"), "x"));
        assert!(is_printable("BLOB", None, "Bracket.STL"));
        assert!(!is_printable("BLOB", None, "notes.txt"));
        assert!(is_printable("ASSEMBLY", None, "x"));
        assert!(!is_printable("DRAWING", None, "x.stl"));
        assert_eq!(blob_ext(Some("model/stl"), "x.3mf"), ".stl");
        assert_eq!(blob_ext(None, "Plate.3MF"), ".3mf");
        assert_eq!(blob_ext(None, "thing"), ".stl");
    }

    #[test]
    fn elements_never_stale_without_doc_modified_at() {
        assert!(!elements_stale(&cache(Some("2026-01-01T00:00:00Z")), None));
    }

    #[test]
    fn elements_stale_compares_the_recorded_doc_modified_at() {
        let old = cache(Some("2026-01-01T00:00:00Z"));
        assert!(!elements_stale(&old, Some("2026-01-01T00:00:00Z")));
        assert!(elements_stale(&old, Some("2026-02-01T00:00:00Z")));
    }

    #[test]
    fn elements_without_recorded_doc_modified_at_are_stale() {
        assert!(elements_stale(&cache(None), Some("2026-01-01T00:00:00Z")));
    }

    #[test]
    fn thumb_key_is_stamped_with_the_t_parameter() {
        assert_eq!(
            thumb_key("d1", Some("https://cad.onshape.com/thumb?t=12:34-ab")),
            "d1-1234ab"
        );
    }

    #[test]
    fn thumb_key_uses_the_content_hash_of_a_part_thumbnail() {
        assert_eq!(
            thumb_key(
                "d1-e1-JJD",
                Some("https://cad.onshape.com/api/thumbnails/dcbf8fe354db4fbf7da6a914ee1e69de4870c8ae/s/300x170")
            ),
            "d1-e1-JJD-dcbf8fe354db4fbf7da6a914ee1e69de4870c8ae"
        );
        // A short or non-hex segment is not a content hash.
        assert_eq!(
            thumb_key(
                "p",
                Some("https://cad.onshape.com/api/thumbnails/d/abc/s/300x170")
            ),
            "p-0"
        );
    }

    #[test]
    fn part_file_id_keeps_plain_ids_and_hashes_the_rest() {
        assert_eq!(part_file_id("JJD"), "JJD");
        let odd = part_file_id("J/../D");
        assert!(odd.starts_with('x') && odd.len() == 17);
        assert!(check_id("part", &odd).is_ok());
        assert_eq!(odd, part_file_id("J/../D"));
    }

    #[test]
    fn elements_fetched_without_thumbnails_are_stale() {
        let mut old = cache(Some("2026-01-01T00:00:00Z"));
        old.with_thumbs = false;
        assert!(elements_stale(&old, Some("2026-01-01T00:00:00Z")));
        assert!(elements_stale(&old, None));
    }

    #[test]
    fn thumb_key_uses_0_without_t_or_with_an_invalid_href() {
        assert_eq!(
            thumb_key("d1", Some("https://cad.onshape.com/thumb")),
            "d1-0"
        );
        assert_eq!(thumb_key("d1", Some("not a url")), "d1-0");
        assert_eq!(thumb_key("d1", None), "d1-0");
    }

    #[test]
    fn export_rel_names_a_whole_tab_after_the_tab_without_a_doubled_extension() {
        assert_eq!(
            export_rel("My Doc", "Bracket.stl", None, ".stl"),
            (
                "exports/My Doc".to_string(),
                "exports/My Doc/Bracket.stl".to_string()
            )
        );
    }

    #[test]
    fn export_rel_nests_a_single_part_under_the_tab() {
        assert_eq!(
            export_rel("My Doc", "Bracket.stl", Some("Arm: left"), ".stl"),
            (
                "exports/My Doc/Bracket.stl".to_string(),
                "exports/My Doc/Bracket.stl/Arm_ left.stl".to_string()
            )
        );
    }

    fn record(el: &str, mv: Option<&str>) -> ExportRecord {
        ExportRecord {
            doc_id: "d".into(),
            element_id: el.into(),
            element_microversion: mv.map(Into::into),
            deps: vec![],
            exported_at: "t".into(),
        }
    }

    fn mvs(pairs: &[(&str, Option<&str>)]) -> HashMap<(String, String), Option<String>> {
        pairs
            .iter()
            .map(|(e, mv)| (("d".to_string(), e.to_string()), mv.map(String::from)))
            .collect()
    }

    #[test]
    fn tab_signature_is_the_microversion_plus_sorted_deps() {
        let cur = mvs(&[
            ("a", Some("1")),
            ("s1", Some("x")),
            ("s2", Some("y")),
            ("u", None),
        ]);
        assert_eq!(tab_signature("d", "a", &[], &cur).as_deref(), Some("1"));
        let deps = vec!["s2".to_string(), "s1".to_string(), "s2".to_string()];
        assert_eq!(
            tab_signature("d", "a", &deps, &cur).as_deref(),
            Some("1|s1=x|s2=y")
        );
        assert_eq!(tab_signature("d", "a", &["u".to_string()], &cur), None);
        assert_eq!(tab_signature("d", "a", &["gone".to_string()], &cur), None);
        assert_eq!(tab_signature("d", "u", &[], &cur), None);
        assert_eq!(tab_signature("other", "a", &[], &cur), None);
    }

    #[test]
    fn an_assembly_goes_stale_when_a_referenced_tab_changes() {
        let mut records = ExportRecords::default();
        let mut asm = record("asm", Some("1|ps=x"));
        asm.deps = vec!["ps".into()];
        records.items.insert("d/asm/".into(), asm);
        let same = mvs(&[("asm", Some("1")), ("ps", Some("x"))]);
        assert!(stale_keys(&records, &same).is_empty());
        let moved = mvs(&[("asm", Some("1")), ("ps", Some("x2"))]);
        assert_eq!(stale_keys(&records, &moved), vec!["d/asm/"]);
        let gone = mvs(&[("asm", Some("1"))]);
        assert!(stale_keys(&records, &gone).is_empty());
    }

    #[test]
    fn records_without_deps_keep_their_old_shape() {
        let r: ExportRecord = serde_json::from_str(
            r#"{"docId":"d","elementId":"e","elementMicroversion":"m","exportedAt":"t"}"#,
        )
        .unwrap();
        assert!(r.deps.is_empty());
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("deps").is_none());
    }

    #[test]
    fn assembly_def_lists_same_doc_tabs_once() {
        let def: AssemblyDef = serde_json::from_str(
            r#"{
              "rootAssembly": {"instances": [
                {"documentId": "d", "elementId": "ps1", "type": "Part"},
                {"documentId": "d", "elementId": "ps1", "type": "Part"},
                {"documentId": "other", "elementId": "ext", "type": "Part"},
                {"documentId": "d", "elementId": "sub", "type": "Assembly"}
              ]},
              "subAssemblies": [{"instances": [
                {"documentId": "d", "elementId": "ps2"},
                {"documentId": "d", "elementId": "asm"}
              ]}]
            }"#,
        )
        .unwrap();
        assert_eq!(def.same_doc_deps("d", "asm"), vec!["ps1", "ps2", "sub"]);
        let empty: AssemblyDef = serde_json::from_str("{}").unwrap();
        assert!(empty.root_assembly.instances.is_empty());
    }

    fn stl(header: &str, verts: &[f32]) -> Vec<u8> {
        let mut b = vec![0u8; 80];
        b[..header.len()].copy_from_slice(header.as_bytes());
        let n = verts.len() / 9;
        b.extend_from_slice(&(n as u32).to_le_bytes());
        for tri in verts.chunks(9) {
            b.extend_from_slice(&[0u8; 12]);
            for v in tri {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&[0u8; 2]);
        }
        b
    }

    fn verts(b: &[u8]) -> Vec<f32> {
        (0..9)
            .map(|k| f32::from_le_bytes(b[96 + 4 * k..100 + 4 * k].try_into().unwrap()))
            .collect()
    }

    #[test]
    fn stl_to_mm_scales_meters_and_rewrites_the_header() {
        let v = [0.0, 0.0, 0.0, 0.065, 0.0, 0.0, 0.0, 0.11, 0.001];
        let mut b = stl(
            "root\0 STL generated by ONSHAPE BY PTC INC, 1.223. Units = meters",
            &v,
        );
        stl_to_mm(&mut b).unwrap();
        let got = verts(&b);
        for (g, want) in got
            .iter()
            .zip([0.0, 0.0, 0.0, 65.0, 0.0, 0.0, 0.0, 110.0, 1.0])
        {
            assert!((g - want).abs() < 1e-3, "{got:?}");
        }
        assert!(String::from_utf8_lossy(&b[..80]).contains("Units = millimeters"));
        // Already millimeters now: a second pass changes nothing.
        let again = b.clone();
        stl_to_mm(&mut b).unwrap();
        assert_eq!(b, again);
    }

    #[test]
    fn stl_to_mm_handles_other_units_and_bad_input() {
        let mut b = stl("Units = inches", &[1.0; 9]);
        stl_to_mm(&mut b).unwrap();
        assert!((verts(&b)[0] - 25.4).abs() < 1e-4);
        let mut b = stl("no unit here", &[2.0; 9]);
        stl_to_mm(&mut b).unwrap();
        assert_eq!(verts(&b)[0], 2.0);
        assert!(stl_to_mm(&mut stl("Units = parsecs", &[1.0; 9])).is_err());
        assert!(stl_to_mm(&mut b"PK\x03\x04 zip".to_vec()).is_err());
        let mut short = stl("Units = meters", &[1.0; 9]);
        short.pop();
        assert!(stl_to_mm(&mut short).is_err());
    }

    #[test]
    fn translation_parses_the_api_shape() {
        let t: Translation = serde_json::from_str(
            r#"{"id":"t1","requestState":"FAILED","failureReason":"no geometry","resultExternalDataIds":null}"#,
        )
        .unwrap();
        assert_eq!(t.request_state, "FAILED");
        assert_eq!(t.failure_reason.as_deref(), Some("no geometry"));
        assert!(t.result_external_data_ids.is_none());
        let t: Translation = serde_json::from_str(
            r#"{"id":"t1","requestState":"DONE","resultExternalDataIds":["f"]}"#,
        )
        .unwrap();
        assert_eq!(t.result_external_data_ids, Some(vec!["f".to_string()]));
    }

    #[test]
    fn export_key_leaves_the_part_empty_for_a_whole_tab() {
        assert_eq!(export_key("d", "e", None), "d/e/");
        assert_eq!(export_key("d", "e", Some("JHD")), "d/e/JHD");
    }

    #[test]
    fn stale_keys_flag_only_tabs_whose_microversion_moved() {
        let mut records = ExportRecords::default();
        records
            .items
            .insert("d/e1/".into(), record("e1", Some("a")));
        records
            .items
            .insert("d/e1/P".into(), record("e1", Some("a")));
        records
            .items
            .insert("d/e2/".into(), record("e2", Some("b")));
        records.items.insert("d/e3/".into(), record("e3", None));
        records
            .items
            .insert("d/gone/".into(), record("gone", Some("c")));
        records
            .items
            .insert("d/e4/".into(), record("e4", Some("x")));
        let current: HashMap<_, _> = [
            ("e1", Some("a2")),
            ("e2", Some("b")),
            ("e3", Some("z")),
            ("e4", None),
        ]
        .into_iter()
        .map(|(e, mv)| (("d".to_string(), e.to_string()), mv.map(String::from)))
        .collect();
        assert_eq!(stale_keys(&records, &current), vec!["d/e1/", "d/e1/P"]);
    }

    #[test]
    fn check_id_rejects_path_tricks() {
        assert!(check_id("document", "0123abcdef").is_ok());
        assert!(check_id("document", "").is_err());
        assert!(check_id("document", "../x").is_err());
        assert!(check_id("document", "a/b").is_err());
    }

    #[test]
    fn serde_shapes_match_the_frontend() {
        let r: Refresh = serde_json::from_str("\"full\"").unwrap();
        assert_eq!(r, Refresh::Full);
        let d: Doc = serde_json::from_str(r#"{"id":"d","name":"n","thumbHref":null}"#).unwrap();
        assert_eq!(d.modified_at, None);
        let res = DocsResult {
            data: DocsCache {
                fetched_at: "t".into(),
                items: vec![],
            },
            from_cache: true,
            new_calls: 0,
            changed: None,
        };
        assert_eq!(
            serde_json::to_string(&res).unwrap(),
            r#"{"fetchedAt":"t","items":[],"fromCache":true,"newCalls":0}"#
        );
    }
}
