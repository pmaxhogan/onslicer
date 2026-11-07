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
//!
//! Rules (see the unit tests at the bottom):
//! - Detection walks a fixed table of known install locations under a few
//!   roots (%ProgramFiles%, %ProgramFiles(x86)%, %LOCALAPPDATA%\Programs on
//!   Windows; /Applications and ~/Applications on macOS). Bambu Studio is
//!   first. Each table entry yields at most one slicer; versioned install
//!   folders (Cura, Creality Print, ...) resolve to the newest version.
//! - The list is detected slicers (scan order) then manual entries (oldest
//!   first), de-duplicated by path (case-insensitive on Windows).
//! - Manual entries are kept until their path is no longer a file.
//! - A selection that is still a listed file is never changed by listing or
//!   rescanning. Without one, the first listed slicer is selected.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
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
pub struct Slicers {
    /// Result of the last scan; `None` until the first command of the process
    /// needs it. The lock also serializes the store read-modify-write cycles.
    detected: Mutex<Option<Vec<Slicer>>>,
}

impl Slicers {
    pub fn new() -> Self {
        Self {
            detected: Mutex::new(None),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Vec<Slicer>>> {
        // A panic while holding the lock leaves only a cached scan behind,
        // which is always safe to reuse.
        self.detected
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
    let exe = {
        let mut detected = slicers.lock();
        let detected = detected.get_or_insert_with(scan);
        let store = Persistence::open(app)?;
        let persisted = store.load(detected);
        match persisted.selected.clone() {
            // An explicit selection is never silently swapped for another
            // slicer at launch time; the user is told to pick again instead.
            Some(selected) => selected,
            None => {
                let (state, next) = resolve(detected, &persisted, is_file);
                store.save_if_changed(&next)?;
                state.selected.ok_or_else(|| {
                    "No slicer found. Install one (for example Bambu Studio), \
                     or choose its program with \"Other...\" in Settings."
                        .to_string()
                })?
            }
        }
    };
    if !is_file(Path::new(&exe)) {
        return Err(format!(
            "The selected slicer is no longer at {exe}. Choose another one in Settings."
        ));
    }
    std::process::Command::new(&exe)
        .arg(file)
        .spawn()
        .map_err(|e| format!("Could not start the slicer at {exe}: {e}"))?;
    Ok(())
}

/// Current state. The first call of a process scans; later calls reuse the
/// scan. Every call drops entries whose path is no longer a file.
#[tauri::command]
pub async fn list_slicers(
    app: AppHandle,
    slicers: State<'_, Slicers>,
) -> Result<SlicerState, String> {
    let mut detected = slicers.lock();
    let detected = detected.get_or_insert_with(scan);
    current_state(&app, detected)
}

/// Forces a rescan of the install locations.
#[tauri::command]
pub async fn rescan_slicers(
    app: AppHandle,
    slicers: State<'_, Slicers>,
) -> Result<SlicerState, String> {
    let mut detected = slicers.lock();
    let detected = detected.insert(scan());
    current_state(&app, detected)
}

/// "Other...": opens a native file picker for an executable, adds the pick as a
/// manual entry and selects it. Cancelling returns the unchanged state.
#[tauri::command]
pub async fn pick_manual_slicer(
    app: AppHandle,
    slicers: State<'_, Slicers>,
) -> Result<SlicerState, String> {
    // The dialog is opened from Rust, so it needs the plugin registered in
    // lib.rs but no `dialog:` capability (capabilities only gate IPC calls
    // made by the webview). No lock is held while the dialog is open.
    let picked = pick_executable(&app)?;

    let mut detected = slicers.lock();
    let detected = detected.get_or_insert_with(scan);
    let Some(picked) = picked else {
        return current_state(&app, detected);
    };
    let picked = picked.to_string_lossy().into_owned();
    if !is_file(Path::new(&picked)) {
        return Err(format!("{picked} is not a file."));
    }

    let store = Persistence::open(&app)?;
    let mut persisted = store.load(detected);
    add_manual(&mut persisted, detected, &picked);
    let (state, next) = resolve(detected, &persisted, is_file);
    store.save_if_changed(&next)?;
    Ok(state)
}

/// Selects one of the listed slicers by path.
#[tauri::command]
pub async fn select_slicer(
    app: AppHandle,
    slicers: State<'_, Slicers>,
    path: String,
) -> Result<SlicerState, String> {
    let mut detected = slicers.lock();
    let detected = detected.get_or_insert_with(scan);
    let store = Persistence::open(&app)?;
    let persisted = store.load(detected);
    let (state, mut next) = resolve(detected, &persisted, is_file);
    let entry = state
        .slicers
        .iter()
        .find(|s| same_path(&s.path, &path))
        .ok_or_else(|| {
            format!("{path} is not in the slicer list. Rescan or add it with \"Other...\".")
        })?;
    next.selected = Some(entry.path.clone());
    let (state, next) = resolve(detected, &next, is_file);
    store.save_if_changed(&next)?;
    Ok(state)
}

// ---------------------------------------------------------------------------
// Tauri / store glue
// ---------------------------------------------------------------------------

const STORE_FILE: &str = "settings.json";
const STORE_KEY: &str = "slicers";
/// Pre-multi-slicer setting holding the Bambu Studio path; read once when the
/// `slicers` key does not exist yet.
const LEGACY_KEY: &str = "bambuPath";

/// The `slicers` value in settings.json.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
struct Persisted {
    #[serde(default)]
    manual: Vec<String>,
    #[serde(default)]
    selected: Option<String>,
}

struct Persistence {
    store: std::sync::Arc<tauri_plugin_store::Store<tauri::Wry>>,
    /// What the store held when it was opened (`None`: key missing or
    /// unreadable), to skip writes that would not change anything.
    raw: Option<Persisted>,
}

impl Persistence {
    fn open(app: &AppHandle) -> Result<Self, String> {
        use tauri_plugin_store::StoreExt;
        // Returns the instance the frontend already loaded for the same file,
        // so both sides share one in-memory copy.
        let store = app
            .store(STORE_FILE)
            .map_err(|e| format!("Could not open {STORE_FILE}: {e}"))?;
        let raw = store
            .get(STORE_KEY)
            .and_then(|v| serde_json::from_value::<Persisted>(v).ok());
        Ok(Self { store, raw })
    }

    /// The persisted value, or the legacy `bambuPath` migrated into one.
    fn load(&self, detected: &[Slicer]) -> Persisted {
        if let Some(persisted) = &self.raw {
            return persisted.clone();
        }
        let legacy = self
            .store
            .get(LEGACY_KEY)
            .and_then(|v| v.as_str().map(str::to_owned));
        migrate_legacy(legacy.as_deref(), detected, is_file)
    }

    fn save_if_changed(&self, next: &Persisted) -> Result<(), String> {
        if self.raw.as_ref() == Some(next) {
            return Ok(());
        }
        let value = serde_json::to_value(next).map_err(|e| e.to_string())?;
        self.store.set(STORE_KEY, value);
        self.store
            .save()
            .map_err(|e| format!("Could not save {STORE_FILE}: {e}"))
    }
}

fn current_state(app: &AppHandle, detected: &[Slicer]) -> Result<SlicerState, String> {
    let store = Persistence::open(app)?;
    let persisted = store.load(detected);
    let (state, next) = resolve(detected, &persisted, is_file);
    store.save_if_changed(&next)?;
    Ok(state)
}

/// Native "open file" dialog. `Ok(None)` when cancelled.
fn pick_executable(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    use tauri::Manager;
    use tauri_plugin_dialog::DialogExt;

    let mut dialog = app.dialog().file().set_title("Choose your slicer");
    if cfg!(windows) {
        dialog = dialog
            .add_filter("Programs", &["exe"])
            .add_filter("All files", &["*"]);
    }
    #[cfg(desktop)]
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    // Blocking is fine: async commands run on the async runtime, not the main
    // thread, and the plugin documents this exact use.
    let Some(picked) = dialog.blocking_pick_file() else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| format!("Unsupported file selection: {e}"))?;
    Ok(Some(app_bundle_executable(path)))
}

/// macOS file pickers return `.app` bundles, which are directories; launch the
/// binary inside instead. Elsewhere the path is returned unchanged.
fn app_bundle_executable(path: PathBuf) -> PathBuf {
    if cfg!(target_os = "macos") && path.is_dir() {
        if let Some(stem) = path.file_stem() {
            let inner = path.join("Contents").join("MacOS").join(stem);
            if inner.is_file() {
                return inner;
            }
        }
    }
    path
}

// ---------------------------------------------------------------------------
// Pure state rules
// ---------------------------------------------------------------------------

fn is_file(path: &Path) -> bool {
    path.is_file()
}

/// Identity of a path for de-duplication: Windows paths are case-insensitive
/// and accept both separators.
fn path_key(path: &str) -> String {
    if cfg!(windows) {
        path.replace('/', "\\").to_lowercase()
    } else {
        path.to_owned()
    }
}

fn same_path(a: &str, b: &str) -> bool {
    path_key(a) == path_key(b)
}

fn manual_slicer(path: &str) -> Slicer {
    let name = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned());
    Slicer {
        name,
        path: path.to_owned(),
        manual: true,
    }
}

/// Applies the list and selection rules. Returns the state for the frontend
/// and the value to persist (manual entries pruned, selection settled).
fn resolve(
    detected: &[Slicer],
    persisted: &Persisted,
    is_file: impl Fn(&Path) -> bool,
) -> (SlicerState, Persisted) {
    let mut slicers: Vec<Slicer> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for slicer in detected {
        let key = path_key(&slicer.path);
        if !seen.contains(&key) && is_file(Path::new(&slicer.path)) {
            seen.push(key);
            slicers.push(Slicer {
                manual: false,
                ..slicer.clone()
            });
        }
    }

    let mut manual: Vec<String> = Vec::new();
    for path in &persisted.manual {
        if manual.iter().any(|m| same_path(m, path)) || !is_file(Path::new(path)) {
            continue;
        }
        manual.push(path.clone());
        let key = path_key(path);
        if !seen.contains(&key) {
            seen.push(key);
            slicers.push(manual_slicer(path));
        }
    }

    let kept = persisted
        .selected
        .as_deref()
        .and_then(|sel| slicers.iter().find(|s| same_path(&s.path, sel)));
    let selected = kept.or(slicers.first()).map(|s| s.path.clone());

    let next = Persisted {
        manual,
        selected: selected.clone(),
    };
    (SlicerState { slicers, selected }, next)
}

/// Records a path picked through "Other..." and selects it. A path that is
/// already detected or already a manual entry is not added again.
fn add_manual(persisted: &mut Persisted, detected: &[Slicer], path: &str) {
    let known = detected.iter().any(|s| same_path(&s.path, path))
        || persisted.manual.iter().any(|m| same_path(m, path));
    if !known {
        persisted.manual.push(path.to_owned());
    }
    persisted.selected = Some(path.to_owned());
}

/// First run after the multi-slicer update: carry over the old Bambu Studio
/// path setting as the selection (and as a manual entry if it is not one of
/// the detected slicers).
fn migrate_legacy(
    legacy: Option<&str>,
    detected: &[Slicer],
    is_file: impl Fn(&Path) -> bool,
) -> Persisted {
    let mut persisted = Persisted::default();
    if let Some(path) = legacy.map(str::trim).filter(|p| !p.is_empty()) {
        if is_file(Path::new(path)) {
            add_manual(&mut persisted, detected, path);
        }
    }
    persisted
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// A known slicer and where its installer puts the executable.
struct Candidate {
    name: &'static str,
    /// Executable paths relative to each root, '/'-separated, tried in order;
    /// the first one found wins. A segment ending in '*' matches directory
    /// names starting with the rest (ASCII case-insensitive) and resolves to
    /// the highest version number that contains the remainder of the path.
    paths: &'static [&'static str],
}

/// Windows install locations, in scan order (Bambu Studio first: it was the
/// only supported slicer before, so it stays the default when present).
/// Every candidate is tried under %ProgramFiles%, %ProgramFiles(x86)% and
/// %LOCALAPPDATA%\Programs. Exe names and folders come from the vendors'
/// installer scripts / CMake targets, winget manifests or uninstall listings;
/// guesses are marked as such (a wrong guess only means "not detected").
const WINDOWS_CANDIDATES: &[Candidate] = &[
    Candidate {
        // NSIS, $PROGRAMFILES64\Bambu Studio (winget Bambulab.Bambustudio).
        name: "Bambu Studio",
        paths: &["Bambu Studio/bambu-studio.exe"],
    },
    Candidate {
        // CPack NSIS, OrcaSlicer/OrcaSlicer CMakeLists.txt.
        name: "OrcaSlicer",
        paths: &["OrcaSlicer/orca-slicer.exe"],
    },
    Candidate {
        // Inno Setup, Prusa3D\PrusaSlicer (older per-user installs used
        // %LOCALAPPDATA%\Programs\Prusa3D\PrusaSlicer).
        name: "PrusaSlicer",
        paths: &["Prusa3D/PrusaSlicer/prusa-slicer.exe"],
    },
    Candidate {
        // Mostly a portable zip; the MSI (winInstaller.wxs.in) installs to
        // %ProgramFiles(x86)%\SuperSlicer.
        name: "SuperSlicer",
        paths: &["SuperSlicer/superslicer.exe"],
    },
    Candidate {
        // One folder per version: "UltiMaker Cura 5.8.1" (5.2.2+),
        // "Ultimaker Cura 5.0.0" (prefix matched case-insensitively; the exe
        // is Ultimaker-Cura.exe there, same name to Windows), and
        // "Ultimaker Cura 4.13.1\Cura.exe" for 4.x.
        name: "UltiMaker Cura",
        paths: &[
            "UltiMaker Cura */UltiMaker-Cura.exe",
            "Ultimaker Cura */Cura.exe",
        ],
    },
    Candidate {
        // 6.x/7.x: Creality\Creality Print 7.3 (CrealityOfficial/CrealityPrint
        // package.cmake); 5.x: Creality Print 5.1 without the parent folder.
        name: "Creality Print",
        paths: &[
            "Creality/Creality Print */CrealityPrint.exe",
            "Creality Print */CrealityPrint.exe",
        ],
    },
    Candidate {
        // CPack NSIS, $PROGRAMFILES64\ElegooSlicer.
        name: "ElegooSlicer",
        paths: &["ElegooSlicer/elegoo-slicer.exe"],
    },
    Candidate {
        // Inno Setup (uninstall listings).
        name: "Anycubic Slicer Next",
        paths: &["AnycubicSlicerNext/AnycubicSlicerNext.exe"],
    },
    Candidate {
        // The older Anycubic Slicer; folder spelling varies by version.
        name: "Anycubic Slicer",
        paths: &[
            "AnycubicSlicer/Anycubic-Slicer.exe",
            "Anycubic Slicer/Anycubic-Slicer.exe",
        ],
    },
    Candidate {
        // Exe from QIDITECH/QIDIStudio CMake; folder from uninstall listings.
        name: "QIDI Studio",
        paths: &["QIDIStudio/qidi-studio.exe"],
    },
    Candidate {
        // Exe from QIDITECH/QIDISlicer CMake; folder from uninstall listings.
        name: "QIDISlicer",
        paths: &["QIDISlicer/qidi-slicer.exe"],
    },
    Candidate {
        // installer.nsi: InstallDir $PROGRAMFILES64\Snapmaker_Orca.
        name: "Snapmaker Orca",
        paths: &["Snapmaker_Orca/snapmaker-orca.exe"],
    },
    Candidate {
        // FlashForge's Orca fork (Flash Studio, formerly Orca-Flashforge).
        // Unverified: the repo names the exe both "flash-studio.exe" and
        // "flash studio.exe", and the shipped installer may use another folder.
        name: "Flash Studio",
        paths: &[
            "Orca-Flashforge/flash-studio.exe",
            "Orca-Flashforge/flash studio.exe",
            "Orca-Flashforge/orca-flashforge.exe",
        ],
    },
    Candidate {
        // FlashForge\FlashPrint 5 (uninstall listings).
        name: "FlashPrint",
        paths: &["FlashForge/FlashPrint */FlashPrint.exe"],
    },
    Candidate {
        // Simplify3D-5.0.1 / Simplify3D-4.1.2 (uninstall listings, forum).
        name: "Simplify3D",
        paths: &["Simplify3D-*/Simplify3D.exe", "Simplify3D/Simplify3D.exe"],
    },
    Candidate {
        // 5.x: Raise3D\ideaMaker-App (winget); 4.x and older: Raise3D\ideaMaker.
        name: "ideaMaker",
        paths: &[
            "Raise3D/ideaMaker-App/ideaMaker.exe",
            "Raise3D/ideaMaker/ideaMaker.exe",
        ],
    },
];

/// macOS app bundles (binary inside), in scan order. Kept small; anything
/// else can be added with "Other...".
const MACOS_CANDIDATES: &[Candidate] = &[
    Candidate {
        name: "Bambu Studio",
        paths: &["BambuStudio.app/Contents/MacOS/BambuStudio"],
    },
    Candidate {
        name: "OrcaSlicer",
        paths: &["OrcaSlicer.app/Contents/MacOS/OrcaSlicer"],
    },
    Candidate {
        name: "PrusaSlicer",
        paths: &["PrusaSlicer.app/Contents/MacOS/PrusaSlicer"],
    },
    Candidate {
        // Ultimaker/Cura packaging/MacOS/build_macos.py.
        name: "UltiMaker Cura",
        paths: &["UltiMaker-Cura.app/Contents/MacOS/UltiMaker-Cura"],
    },
];

fn platform_candidates() -> &'static [Candidate] {
    if cfg!(windows) {
        WINDOWS_CANDIDATES
    } else if cfg!(target_os = "macos") {
        MACOS_CANDIDATES
    } else {
        &[]
    }
}

/// Directories the candidate paths are relative to, in search order.
#[derive(Debug, Default)]
struct Roots(Vec<PathBuf>);

impl Roots {
    fn from_env() -> Self {
        let mut dirs: Vec<PathBuf> = Vec::new();
        if cfg!(windows) {
            // ProgramW6432 is the 64-bit Program Files even in a 32-bit process.
            for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
                if let Some(dir) = std::env::var_os(var) {
                    dirs.push(PathBuf::from(dir));
                }
            }
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                dirs.push(PathBuf::from(local).join("Programs"));
            }
        } else if cfg!(target_os = "macos") {
            dirs.push(PathBuf::from("/Applications"));
            if let Some(home) = std::env::var_os("HOME") {
                dirs.push(PathBuf::from(home).join("Applications"));
            }
        }
        let mut unique: Vec<PathBuf> = Vec::new();
        for dir in dirs {
            let key = path_key(&dir.to_string_lossy());
            if !unique.iter().any(|u| path_key(&u.to_string_lossy()) == key) {
                unique.push(dir);
            }
        }
        Self(unique)
    }
}

fn scan() -> Vec<Slicer> {
    detect(platform_candidates(), &Roots::from_env())
}

/// One slicer per candidate that is installed, in candidate order. For each
/// candidate, roots are searched in order, then its paths in order.
fn detect(candidates: &[Candidate], roots: &Roots) -> Vec<Slicer> {
    let mut found: Vec<Slicer> = Vec::new();
    for candidate in candidates {
        let hit = roots.0.iter().find_map(|root| {
            candidate.paths.iter().find_map(|rel| {
                let segments: Vec<&str> = rel.split('/').collect();
                resolve_pattern(root, &segments)
            })
        });
        if let Some(path) = hit {
            let path = path.to_string_lossy().into_owned();
            if !found.iter().any(|s| same_path(&s.path, &path)) {
                found.push(Slicer {
                    name: candidate.name.to_owned(),
                    path,
                    manual: false,
                });
            }
        }
    }
    found
}

/// Resolves a '/'-split candidate path under `base` to an existing file.
fn resolve_pattern(base: &Path, segments: &[&str]) -> Option<PathBuf> {
    let Some((first, rest)) = segments.split_first() else {
        return is_file(base).then(|| base.to_path_buf());
    };
    let Some(prefix) = first.strip_suffix('*') else {
        return resolve_pattern(&base.join(first), rest);
    };
    let mut matches: Vec<(Vec<u64>, String)> = std::fs::read_dir(base)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter_map(|name| {
            let head = name.get(..prefix.len())?;
            head.eq_ignore_ascii_case(prefix)
                .then(|| (version_key(&name[prefix.len()..]), name))
        })
        .collect();
    // Newest first. On equal versions the shorter name wins, so a release
    // ("Creality Print 7.3") beats a suffixed build ("Creality Print 7.3 Beta");
    // then by name so the result is deterministic.
    matches.sort_by(|(va, na), (vb, nb)| {
        vb.cmp(va)
            .then_with(|| na.len().cmp(&nb.len()))
            .then_with(|| na.cmp(nb))
    });
    matches
        .into_iter()
        .find_map(|(_, name)| resolve_pattern(&base.join(name), rest))
}

/// Numeric components of a version-ish string: "5.10.0" -> [5, 10, 0].
fn version_key(s: &str) -> Vec<u64> {
    s.split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse().unwrap_or(u64::MAX))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A unique directory under the system temp dir, removed on drop.
    struct TempTree(PathBuf);

    impl TempTree {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "onslicer-slicers-test-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn root(&self, name: &str) -> PathBuf {
            let root = self.0.join(name);
            std::fs::create_dir_all(&root).unwrap();
            root
        }

        /// Creates `root/rel` (a '/'-separated path) as an empty file.
        fn file(&self, root: &Path, rel: &str) -> String {
            let path = rel.split('/').fold(root.to_path_buf(), |p, s| p.join(s));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"").unwrap();
            path.to_string_lossy().into_owned()
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn detected(name: &str, path: &str) -> Slicer {
        Slicer {
            name: name.into(),
            path: path.into(),
            manual: false,
        }
    }

    fn files(paths: &[&str]) -> impl Fn(&Path) -> bool {
        let set: HashSet<String> = paths.iter().map(|p| path_key(p)).collect();
        move |p: &Path| set.contains(&path_key(&p.to_string_lossy()))
    }

    fn names(slicers: &[Slicer]) -> Vec<&str> {
        slicers.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn bambu_studio_is_scanned_first() {
        assert_eq!(WINDOWS_CANDIDATES[0].name, "Bambu Studio");
        assert_eq!(
            WINDOWS_CANDIDATES[0].paths[0],
            "Bambu Studio/bambu-studio.exe"
        );
    }

    #[test]
    fn detection_follows_table_order_not_disk_order() {
        let tree = TempTree::new();
        let pf = tree.root("pf");
        let orca = tree.file(&pf, "OrcaSlicer/orca-slicer.exe");
        let prusa = tree.file(&pf, "Prusa3D/PrusaSlicer/prusa-slicer.exe");
        let bambu = tree.file(&pf, "Bambu Studio/bambu-studio.exe");
        // Unrelated program folders are ignored.
        tree.file(&pf, "Notepad++/notepad++.exe");

        let found = detect(WINDOWS_CANDIDATES, &Roots(vec![pf]));
        assert_eq!(names(&found), ["Bambu Studio", "OrcaSlicer", "PrusaSlicer"]);
        assert_eq!(found[0].path, bambu);
        assert_eq!(found[1].path, orca);
        assert_eq!(found[2].path, prusa);
        assert!(found.iter().all(|s| !s.manual));
    }

    #[test]
    fn detection_searches_every_root_and_prefers_earlier_roots() {
        let tree = TempTree::new();
        let pf = tree.root("pf");
        let x86 = tree.root("x86");
        let local = tree.root("local-programs");
        let bambu_pf = tree.file(&pf, "Bambu Studio/bambu-studio.exe");
        tree.file(&x86, "Bambu Studio/bambu-studio.exe");
        let cura = tree.file(&x86, "UltiMaker Cura 5.8.1/UltiMaker-Cura.exe");
        let orca = tree.file(&local, "OrcaSlicer/orca-slicer.exe");

        let found = detect(WINDOWS_CANDIDATES, &Roots(vec![pf, x86, local]));
        assert_eq!(
            found,
            vec![
                detected("Bambu Studio", &bambu_pf),
                detected("OrcaSlicer", &orca),
                detected("UltiMaker Cura", &cura),
            ]
        );
    }

    #[test]
    fn detection_with_nothing_installed_is_empty() {
        let tree = TempTree::new();
        let missing = tree.0.join("does-not-exist");
        assert!(detect(WINDOWS_CANDIDATES, &Roots(vec![tree.root("pf"), missing])).is_empty());
    }

    #[test]
    fn folder_without_executable_is_not_detected() {
        let tree = TempTree::new();
        let pf = tree.root("pf");
        std::fs::create_dir_all(pf.join("Bambu Studio")).unwrap();
        tree.file(&pf, "Bambu Studio/Uninstall.exe");
        assert!(detect(WINDOWS_CANDIDATES, &Roots(vec![pf])).is_empty());
    }

    #[test]
    fn versioned_folders_pick_the_newest_numerically() {
        let tree = TempTree::new();
        let pf = tree.root("pf");
        tree.file(&pf, "UltiMaker Cura 5.9.1/UltiMaker-Cura.exe");
        let newest = tree.file(&pf, "UltiMaker Cura 5.10.0/UltiMaker-Cura.exe");
        tree.file(&pf, "UltiMaker Cura 5.2.0/UltiMaker-Cura.exe");
        // A newer folder left behind by an uninstall, without the exe.
        std::fs::create_dir_all(pf.join("UltiMaker Cura 5.11.0")).unwrap();

        let found = detect(WINDOWS_CANDIDATES, &Roots(vec![pf]));
        assert_eq!(found, vec![detected("UltiMaker Cura", &newest)]);
    }

    #[test]
    fn versioned_folders_match_prefix_case_insensitively() {
        let tree = TempTree::new();
        let pf = tree.root("pf");
        // Cura 5.0-5.2 used "Ultimaker" capitalization.
        let cura = tree.file(&pf, "Ultimaker Cura 5.0.0/UltiMaker-Cura.exe");
        let found = detect(WINDOWS_CANDIDATES, &Roots(vec![pf]));
        assert_eq!(found, vec![detected("UltiMaker Cura", &cura)]);
    }

    #[test]
    fn later_paths_are_fallbacks() {
        let tree = TempTree::new();
        let pf = tree.root("pf");
        // Cura 4.x: different exe name.
        let cura4 = tree.file(&pf, "Ultimaker Cura 4.13.1/Cura.exe");
        let found = detect(WINDOWS_CANDIDATES, &Roots(vec![pf.clone()]));
        assert_eq!(found, vec![detected("UltiMaker Cura", &cura4)]);

        // With a 5.x install present the first path wins.
        let cura5 = tree.file(&pf, "UltiMaker Cura 5.8.1/UltiMaker-Cura.exe");
        let found = detect(WINDOWS_CANDIDATES, &Roots(vec![pf]));
        assert_eq!(found, vec![detected("UltiMaker Cura", &cura5)]);
    }

    #[test]
    fn nested_versioned_folder() {
        let tree = TempTree::new();
        let pf = tree.root("pf");
        tree.file(&pf, "Creality/Creality Print 5.1/CrealityPrint.exe");
        let newest = tree.file(&pf, "Creality/Creality Print 6.0/CrealityPrint.exe");
        let found = detect(WINDOWS_CANDIDATES, &Roots(vec![pf]));
        assert_eq!(found, vec![detected("Creality Print", &newest)]);

        // A beta of the same version does not beat the release.
        let tree = TempTree::new();
        let pf = tree.root("pf");
        let release = tree.file(&pf, "Creality/Creality Print 7.3/CrealityPrint.exe");
        tree.file(&pf, "Creality/Creality Print 7.3 Beta/CrealityPrint.exe");
        let found = detect(WINDOWS_CANDIDATES, &Roots(vec![pf]));
        assert_eq!(found, vec![detected("Creality Print", &release)]);

        // 5.x had no "Creality" parent folder.
        let tree = TempTree::new();
        let pf = tree.root("pf");
        let v5 = tree.file(&pf, "Creality Print 5.1/CrealityPrint.exe");
        let found = detect(WINDOWS_CANDIDATES, &Roots(vec![pf]));
        assert_eq!(found, vec![detected("Creality Print", &v5)]);
    }

    #[test]
    fn candidate_tables_are_well_formed() {
        for table in [WINDOWS_CANDIDATES, MACOS_CANDIDATES] {
            let mut names: Vec<&str> = table.iter().map(|c| c.name).collect();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), table.len(), "duplicate candidate name");
            for candidate in table {
                assert!(!candidate.paths.is_empty(), "{}", candidate.name);
                for path in candidate.paths {
                    let segments: Vec<&str> = path.split('/').collect();
                    assert!(segments.iter().all(|s| !s.is_empty()), "{path}");
                    assert!(segments.iter().all(|s| !s.contains('\\')), "{path}");
                    let (file, dirs) = segments.split_last().unwrap();
                    assert!(!file.contains('*'), "{path}: wildcard file name");
                    for dir in dirs {
                        assert!(
                            !dir.contains('*') || (dir.ends_with('*') && dir.len() > 1),
                            "{path}: '*' only as a non-empty prefix match"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn version_key_parses_numbers() {
        assert_eq!(version_key(" 5.10.0"), vec![5, 10, 0]);
        assert_eq!(version_key("-4.1.2"), vec![4, 1, 2]);
        assert_eq!(version_key(""), Vec::<u64>::new());
        assert!(version_key(" 5.10.0") > version_key(" 5.9.1"));
        assert!(version_key(" 6.0") > version_key(" 5.1"));
    }

    #[test]
    fn list_is_detected_then_manual_oldest_first() {
        let det = vec![
            detected("Bambu Studio", "C:/pf/bambu.exe"),
            detected("OrcaSlicer", "C:/pf/orca.exe"),
        ];
        let persisted = Persisted {
            manual: vec!["D:/tools/zeta.exe".into(), "D:/tools/alpha.exe".into()],
            selected: None,
        };
        let (state, _) = resolve(
            &det,
            &persisted,
            files(&[
                "C:/pf/bambu.exe",
                "C:/pf/orca.exe",
                "D:/tools/zeta.exe",
                "D:/tools/alpha.exe",
            ]),
        );
        assert_eq!(
            names(&state.slicers),
            ["Bambu Studio", "OrcaSlicer", "zeta", "alpha"]
        );
        assert_eq!(
            state.slicers.iter().map(|s| s.manual).collect::<Vec<_>>(),
            [false, false, true, true]
        );
    }

    #[test]
    fn nothing_selected_picks_first_detected() {
        let det = vec![
            detected("Bambu Studio", "C:/pf/bambu.exe"),
            detected("OrcaSlicer", "C:/pf/orca.exe"),
        ];
        let persisted = Persisted {
            manual: vec!["D:/m.exe".into()],
            selected: None,
        };
        let (state, next) = resolve(
            &det,
            &persisted,
            files(&["C:/pf/bambu.exe", "C:/pf/orca.exe", "D:/m.exe"]),
        );
        assert_eq!(state.selected.as_deref(), Some("C:/pf/bambu.exe"));
        assert_eq!(next.selected.as_deref(), Some("C:/pf/bambu.exe"));
    }

    #[test]
    fn nothing_selected_and_nothing_detected_picks_first_manual() {
        let persisted = Persisted {
            manual: vec!["D:/m1.exe".into(), "D:/m2.exe".into()],
            selected: None,
        };
        let (state, _) = resolve(&[], &persisted, files(&["D:/m1.exe", "D:/m2.exe"]));
        assert_eq!(state.selected.as_deref(), Some("D:/m1.exe"));
    }

    #[test]
    fn nothing_available_selects_nothing() {
        let persisted = Persisted {
            manual: vec!["D:/gone.exe".into()],
            selected: Some("D:/gone.exe".into()),
        };
        let (state, next) = resolve(&[], &persisted, files(&[]));
        assert!(state.slicers.is_empty());
        assert_eq!(state.selected, None);
        assert_eq!(next, Persisted::default());
    }

    #[test]
    fn valid_selection_is_kept_even_if_not_first() {
        let det = vec![
            detected("Bambu Studio", "C:/pf/bambu.exe"),
            detected("OrcaSlicer", "C:/pf/orca.exe"),
        ];
        let persisted = Persisted {
            manual: vec!["D:/m.exe".into()],
            selected: Some("D:/m.exe".into()),
        };
        let exists = files(&["C:/pf/bambu.exe", "C:/pf/orca.exe", "D:/m.exe"]);
        let (state, next) = resolve(&det, &persisted, &exists);
        assert_eq!(state.selected.as_deref(), Some("D:/m.exe"));
        assert_eq!(next, persisted);

        let persisted = Persisted {
            selected: Some("C:/pf/orca.exe".into()),
            ..persisted
        };
        let (state, _) = resolve(&det, &persisted, &exists);
        assert_eq!(state.selected.as_deref(), Some("C:/pf/orca.exe"));
    }

    #[test]
    fn uninstalled_selection_falls_back_to_first_available() {
        let det = vec![
            detected("Bambu Studio", "C:/pf/bambu.exe"),
            detected("OrcaSlicer", "C:/pf/orca.exe"),
        ];
        let persisted = Persisted {
            manual: vec![],
            selected: Some("C:/pf/orca.exe".into()),
        };
        // Orca was uninstalled since the last scan.
        let (state, next) = resolve(&det, &persisted, files(&["C:/pf/bambu.exe"]));
        assert_eq!(names(&state.slicers), ["Bambu Studio"]);
        assert_eq!(state.selected.as_deref(), Some("C:/pf/bambu.exe"));
        assert_eq!(next.selected.as_deref(), Some("C:/pf/bambu.exe"));
    }

    #[test]
    fn selection_that_is_a_file_but_not_listed_is_replaced() {
        let det = vec![detected("Bambu Studio", "C:/pf/bambu.exe")];
        let persisted = Persisted {
            manual: vec![],
            selected: Some("D:/stray.exe".into()),
        };
        let (state, _) = resolve(
            &det,
            &persisted,
            files(&["C:/pf/bambu.exe", "D:/stray.exe"]),
        );
        assert_eq!(state.selected.as_deref(), Some("C:/pf/bambu.exe"));
    }

    #[test]
    fn manual_entries_are_pruned_when_their_file_is_gone() {
        let tree = TempTree::new();
        let dir = tree.root("tools");
        let keep = tree.file(&dir, "keep/keep.exe");
        let gone = tree.file(&dir, "gone/gone.exe");
        std::fs::remove_file(&gone).unwrap();

        let persisted = Persisted {
            manual: vec![gone.clone(), keep.clone()],
            selected: Some(gone),
        };
        let (state, next) = resolve(&[], &persisted, is_file);
        assert_eq!(names(&state.slicers), ["keep"]);
        assert_eq!(next.manual, vec![keep.clone()]);
        assert_eq!(next.selected, Some(keep));
    }

    #[test]
    fn rescan_keeps_valid_manual_entries_and_selection() {
        let det_before = vec![detected("Bambu Studio", "C:/pf/bambu.exe")];
        let exists = files(&["C:/pf/bambu.exe", "C:/pf/orca.exe", "D:/m.exe"]);
        let persisted = Persisted {
            manual: vec!["D:/m.exe".into()],
            selected: Some("C:/pf/bambu.exe".into()),
        };
        let (_, after_list) = resolve(&det_before, &persisted, &exists);

        // Rescan finds a newly installed slicer, listed before manual entries.
        let det_after = vec![
            detected("Bambu Studio", "C:/pf/bambu.exe"),
            detected("OrcaSlicer", "C:/pf/orca.exe"),
        ];
        let (state, next) = resolve(&det_after, &after_list, &exists);
        assert_eq!(names(&state.slicers), ["Bambu Studio", "OrcaSlicer", "m"]);
        assert_eq!(state.selected.as_deref(), Some("C:/pf/bambu.exe"));
        assert_eq!(next, persisted);

        // A rescan that detects nothing still keeps the manual entry.
        let (state, next) = resolve(&[], &after_list, &exists);
        assert_eq!(names(&state.slicers), ["m"]);
        assert_eq!(next.manual, vec!["D:/m.exe".to_string()]);
    }

    #[test]
    fn rescan_after_install_selects_when_nothing_was_selected() {
        let persisted = Persisted::default();
        let (state, after_empty_scan) = resolve(&[], &persisted, files(&[]));
        assert_eq!(state.selected, None);

        let det = vec![detected("Bambu Studio", "C:/pf/bambu.exe")];
        let (state, _) = resolve(&det, &after_empty_scan, files(&["C:/pf/bambu.exe"]));
        assert_eq!(state.selected.as_deref(), Some("C:/pf/bambu.exe"));
    }

    #[test]
    fn manual_duplicate_of_detected_is_listed_once_as_detected() {
        let det = vec![detected("Bambu Studio", "C:/pf/bambu.exe")];
        let persisted = Persisted {
            manual: vec![
                "C:/pf/bambu.exe".into(),
                "D:/m.exe".into(),
                "D:/m.exe".into(),
            ],
            selected: None,
        };
        let (state, next) = resolve(&det, &persisted, files(&["C:/pf/bambu.exe", "D:/m.exe"]));
        assert_eq!(names(&state.slicers), ["Bambu Studio", "m"]);
        assert!(!state.slicers[0].manual);
        assert_eq!(
            next.manual,
            vec!["C:/pf/bambu.exe".to_string(), "D:/m.exe".into()]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_dedup_case_and_separator_insensitively() {
        let det = vec![detected(
            "Bambu Studio",
            r"C:\Program Files\Bambu Studio\bambu-studio.exe",
        )];
        let persisted = Persisted {
            manual: vec![
                "c:/program files/bambu studio/BAMBU-STUDIO.EXE".into(),
                r"D:\Tools\Slicer.exe".into(),
                r"d:\tools\slicer.exe".into(),
            ],
            selected: Some(r"d:\TOOLS\slicer.exe".into()),
        };
        let exists = files(&[
            r"C:\Program Files\Bambu Studio\bambu-studio.exe",
            r"D:\Tools\Slicer.exe",
        ]);
        let (state, next) = resolve(&det, &persisted, exists);
        assert_eq!(names(&state.slicers), ["Bambu Studio", "Slicer"]);
        // The selection is reported with the listed spelling.
        assert_eq!(state.selected.as_deref(), Some(r"D:\Tools\Slicer.exe"));
        assert_eq!(next.manual.len(), 2);

        let mut p = Persisted::default();
        add_manual(
            &mut p,
            &det,
            r"C:\PROGRAM FILES\Bambu Studio\bambu-studio.exe",
        );
        assert!(p.manual.is_empty());
    }

    #[test]
    fn picking_a_new_path_adds_and_selects_it() {
        let det = vec![detected("Bambu Studio", "C:/pf/bambu.exe")];
        let mut persisted = Persisted {
            manual: vec!["D:/old.exe".into()],
            selected: Some("C:/pf/bambu.exe".into()),
        };
        add_manual(&mut persisted, &det, "D:/new.exe");
        let (state, next) = resolve(
            &det,
            &persisted,
            files(&["C:/pf/bambu.exe", "D:/old.exe", "D:/new.exe"]),
        );
        assert_eq!(names(&state.slicers), ["Bambu Studio", "old", "new"]);
        assert!(state.slicers[2].manual);
        assert_eq!(state.selected.as_deref(), Some("D:/new.exe"));
        assert_eq!(
            next.manual,
            vec!["D:/old.exe".to_string(), "D:/new.exe".into()]
        );
    }

    #[test]
    fn picking_a_detected_or_known_path_does_not_duplicate_it() {
        let det = vec![detected("Bambu Studio", "C:/pf/bambu.exe")];
        let mut persisted = Persisted {
            manual: vec!["D:/m.exe".into()],
            selected: Some("D:/m.exe".into()),
        };
        add_manual(&mut persisted, &det, "C:/pf/bambu.exe");
        assert_eq!(persisted.manual, vec!["D:/m.exe".to_string()]);
        assert_eq!(persisted.selected.as_deref(), Some("C:/pf/bambu.exe"));

        add_manual(&mut persisted, &det, "D:/m.exe");
        assert_eq!(persisted.manual, vec!["D:/m.exe".to_string()]);
        assert_eq!(persisted.selected.as_deref(), Some("D:/m.exe"));

        let (state, _) = resolve(&det, &persisted, files(&["C:/pf/bambu.exe", "D:/m.exe"]));
        assert_eq!(names(&state.slicers), ["Bambu Studio", "m"]);
    }

    #[test]
    fn legacy_bambu_path_is_migrated() {
        let det = vec![detected("Bambu Studio", "C:/pf/bambu.exe")];
        let exists = files(&["C:/pf/bambu.exe", "E:/portable/bambu-studio.exe"]);

        // Old default path: already detected, just selected.
        let p = migrate_legacy(Some("C:/pf/bambu.exe"), &det, &exists);
        assert_eq!(p.manual, Vec::<String>::new());
        assert_eq!(p.selected.as_deref(), Some("C:/pf/bambu.exe"));

        // Custom path: becomes a manual entry and stays selected.
        let p = migrate_legacy(Some(" E:/portable/bambu-studio.exe "), &det, &exists);
        assert_eq!(p.manual, vec!["E:/portable/bambu-studio.exe".to_string()]);
        let (state, _) = resolve(&det, &p, &exists);
        assert_eq!(
            state.selected.as_deref(),
            Some("E:/portable/bambu-studio.exe")
        );

        // Missing or stale legacy values migrate to nothing.
        assert_eq!(migrate_legacy(None, &det, &exists), Persisted::default());
        assert_eq!(
            migrate_legacy(Some("Z:/gone.exe"), &det, &exists),
            Persisted::default()
        );
    }

    #[test]
    fn persisted_shape_matches_the_contract() {
        let p = Persisted {
            manual: vec!["C:/a.exe".into()],
            selected: None,
        };
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            serde_json::json!({ "manual": ["C:/a.exe"], "selected": null })
        );
        let partial: Persisted = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(partial, Persisted::default());

        let state = SlicerState {
            slicers: vec![manual_slicer("C:/a.exe")],
            selected: Some("C:/a.exe".into()),
        };
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            serde_json::json!({
                "slicers": [{ "name": "a", "path": "C:/a.exe", "manual": true }],
                "selected": "C:/a.exe"
            })
        );
    }

    /// Runs detection against this machine's real install locations:
    /// `cargo test --locked -- --ignored --nocapture detect_on_this_machine`.
    #[test]
    #[ignore = "depends on what is installed on this machine"]
    fn detect_on_this_machine() {
        let roots = Roots::from_env();
        println!("roots: {:?}", roots.0);
        let found = scan();
        for slicer in &found {
            println!("detected: {} -> {}", slicer.name, slicer.path);
        }
        if found.is_empty() {
            println!("detected: nothing");
        }
    }
}
