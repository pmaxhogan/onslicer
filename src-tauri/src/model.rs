//! Data shapes shared by the disk caches, the Tauri commands and the frontend
//! (src/types.ts), plus the pure helpers that decide paths and staleness.

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

/// File extension of a blob whose data type marks it printable.
fn printable_blob_ext(data_type: Option<&str>) -> Option<&'static str> {
    match data_type {
        Some("model/stl" | "application/sla") => Some(".stl"),
        _ => None,
    }
}

pub fn is_printable(element_type: &str, data_type: Option<&str>, name: &str) -> bool {
    element_type == "PARTSTUDIO"
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
/// docModifiedAt is stale.
pub fn elements_stale(old: &ElementsCache, doc_modified_at: Option<&str>) -> bool {
    match doc_modified_at {
        None => false,
        Some(now) => old.doc_modified_at.as_deref() != Some(now),
    }
}

/// Thumbnail cache key `<docId>-<stamp>`, where the stamp is the thumbnail
/// href's `t` parameter (Onshape bumps it when the thumbnail changes), or `0`.
pub fn thumb_key(doc_id: &str, thumb_href: Option<&str>) -> String {
    let stamp = thumb_href
        .and_then(|href| Url::parse(href).ok())
        .and_then(|url| {
            url.query_pairs()
                .find(|(k, _)| k == "t")
                .map(|(_, v)| v.into_owned())
        })
        .filter(|t| !t.is_empty());
    match stamp {
        Some(t) => {
            let clean: String = t
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            format!("{doc_id}-{clean}")
        }
        None => format!("{doc_id}-0"),
    }
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
        }
    }

    fn cache(doc_modified_at: Option<&str>) -> ElementsCache {
        ElementsCache {
            fetched_at: "2026-01-15T00:00:00Z".into(),
            doc_modified_at: doc_modified_at.map(Into::into),
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
    fn printable_and_blob_ext() {
        assert!(is_printable("PARTSTUDIO", None, "x"));
        assert!(is_printable("BLOB", Some("application/sla"), "x"));
        assert!(is_printable("BLOB", None, "Bracket.STL"));
        assert!(!is_printable("BLOB", None, "notes.txt"));
        assert!(!is_printable("ASSEMBLY", None, "x.stl"));
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
