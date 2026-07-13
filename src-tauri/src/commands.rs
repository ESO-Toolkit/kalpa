use crate::auth::{self, AuthState, AuthTokens, AuthUser};
use crate::esoui::{self, EsouiAddonDetail, EsouiAddonInfo, EsouiCategory, EsouiSearchResult};
use crate::file_hashes;
use crate::installer;
use crate::manifest::{self, AddonManifest};
use crate::manifest_cache;
use crate::metadata;
use crate::uploader::native::session::{SessionProvider, StoredSessionProvider};
use crate::AllowedAddonsPath;
use crate::MetadataLock;
use crate::{PendingDeepLink, PendingDeepLinkPayload};
use rayon::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{Emitter, Manager};
use tempfile::NamedTempFile;

/// Validate that `addons_path` matches the approved path stored in managed state.
/// Prevents a compromised webview from targeting arbitrary filesystem locations.
fn validate_addons_path(addons_path: &str) -> Result<(PathBuf, PathBuf), String> {
    let path = PathBuf::from(addons_path);
    if !path.is_dir() {
        return Err(format!("AddOns folder not found: {addons_path}"));
    }

    let canonical = path
        .canonicalize()
        .map_err(|e| format!("Invalid path: {e}"))?;

    if canonical.file_name().and_then(|n| n.to_str()) != Some("AddOns") {
        return Err("Selected directory must be the ESO AddOns folder.".to_string());
    }

    Ok((path, canonical))
}

fn require_allowed_path(
    state: &tauri::State<'_, AllowedAddonsPath>,
    addons_path: &str,
) -> Result<PathBuf, String> {
    let (_, canonical) = validate_addons_path(addons_path)?;
    let guard = state.0.lock().map_err(|_| "Internal error.".to_string())?;
    let Some(allowed_path) = &*guard else {
        return Err("Addons path has not been initialized.".to_string());
    };
    if canonical != allowed_path.canonical {
        return Err("Addons path does not match the configured path.".to_string());
    }
    Ok(allowed_path.configured.clone())
}

/// Called by the frontend to register the approved addons directory.
/// Stores both the configured and canonicalized paths so commands can validate
/// symlink/junction targets without losing the configured ESO live directory.
#[tauri::command]
pub fn set_addons_path(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<(), String> {
    let (configured, canonical) = validate_addons_path(&addons_path)?;
    let mut guard = state.0.lock().map_err(|_| "Internal error.".to_string())?;
    *guard = Some(crate::ApprovedAddonsPath {
        configured,
        canonical,
    });
    Ok(())
}

#[tauri::command]
pub fn consume_initial_deep_link(
    state: tauri::State<'_, PendingDeepLink>,
) -> Result<PendingDeepLinkPayload, String> {
    let mut guard = state.0.lock().map_err(|_| "Internal error.".to_string())?;
    Ok(std::mem::take(&mut *guard))
}

/// Validate a user-supplied name (backup name, etc.) to prevent path traversal
/// and reject Windows reserved device names.
fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Name cannot be empty.".to_string());
    }

    // Reject path traversal and separators.
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err("Name contains invalid characters.".to_string());
    }

    // Reject characters forbidden in Windows file/folder names.
    let forbidden: &[char] = &['<', '>', ':', '"', '|', '?', '*'];
    if name.contains(forbidden) {
        return Err(
            "Name contains a forbidden character (< > : \" | ? * are not allowed).".to_string(),
        );
    }

    // Reject trailing dots and spaces (silently stripped by Windows, causing mismatches).
    if name.ends_with('.') || name.ends_with(' ') {
        return Err("Name must not end with a dot or space.".to_string());
    }

    // Reject Windows reserved device names (case-insensitive).
    // These include CON, PRN, AUX, NUL, COM1-COM9, LPT1-LPT9.
    // Check the stem (name without extension) since "CON.txt" is also reserved.
    let stem = name.split('.').next().unwrap_or(name);
    let upper = stem.to_uppercase();
    let is_reserved = matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    if is_reserved {
        return Err(format!(
            "\"{stem}\" is a Windows reserved name and cannot be used."
        ));
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    pub installed_folders: Vec<String>,
    pub installed_deps: Vec<String>,
    pub failed_deps: Vec<String>,
    pub skipped_deps: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub folder_name: String,
    pub esoui_id: u32,
    pub current_version: String,
    pub remote_version: String,
    pub download_url: String,
    pub has_update: bool,
    /// Remote ESOUI last-updated timestamp (epoch ms) observed during this
    /// check. The frontend merges it into live addon state so the "Recently
    /// Updated" sort is accurate immediately, without waiting for a re-scan.
    pub remote_last_update: u64,
}

/// Determine the "primary" folder from a list of installed folders.
///
/// Prefer a folder whose name appears in the ESOUI title, otherwise
/// fall back to the first folder in the list.
fn determine_primary_folder(installed_folders: &[String], esoui_title: &str) -> String {
    installed_folders
        .iter()
        .find(|f| esoui_title.contains(f.as_str()))
        .or(installed_folders.first())
        .cloned()
        .unwrap_or_default()
}

/// Record metadata for a set of installed folders. The primary folder gets
/// the esoui_id and version from ESOUI; secondary folders get id 0 and
/// their local manifest version.
#[allow(clippy::too_many_arguments)]
fn record_installed_folders(
    store: &mut metadata::MetadataStore,
    addons_dir: &Path,
    installed_folders: &[String],
    esoui_id: u32,
    esoui_version: &str,
    esoui_title: &str,
    download_url: &str,
    esoui_last_update: u64,
) {
    let primary = determine_primary_folder(installed_folders, esoui_title);
    for folder in installed_folders {
        let is_primary = *folder == primary;
        let version = if is_primary && !esoui_version.is_empty() {
            esoui_version.to_string()
        } else {
            read_local_version(addons_dir, folder)
        };
        metadata::record_install_ext(
            store,
            folder,
            if is_primary { esoui_id } else { 0 },
            &version,
            download_url,
            if is_primary { esoui_last_update } else { 0 },
        );
    }
}

struct ResolvedDeps {
    installed_deps: Vec<String>,
    failed_deps: Vec<String>,
    skipped_deps: Vec<String>,
}

fn resolve_transitive_deps(
    addons_dir: &Path,
    installed_folders: &[String],
    store: &mut metadata::MetadataStore,
) -> ResolvedDeps {
    let mut all_installed = build_installed_set(addons_dir);

    let mut installed_deps: Vec<String> = Vec::new();
    let mut failed_deps: Vec<String> = Vec::new();
    let mut skipped_deps: Vec<String> = Vec::new();

    // Seed with the folders we just installed; loop resolves the full chain.
    let mut folders_to_scan: Vec<String> = installed_folders.to_vec();
    let mut seen: HashSet<String> = HashSet::new();

    while !folders_to_scan.is_empty() {
        let mut missing_deps: Vec<String> = Vec::new();
        for folder in &folders_to_scan {
            let addon = find_manifest(addons_dir, folder)
                .and_then(|p| manifest::parse_manifest(folder, &p));
            if let Some(addon) = addon {
                for dep in &addon.depends_on {
                    let key = normalize_addon_name(&dep.name);
                    if !all_installed.contains(&key) && seen.insert(key) {
                        missing_deps.push(dep.name.clone());
                    }
                }
            }
        }

        if missing_deps.is_empty() {
            break;
        }

        let mut newly_installed_folders: Vec<String> = Vec::new();
        for (i, dep_name) in missing_deps.iter().enumerate() {
            // Throttle between ESOUI requests to avoid hammering the server
            if i > 0 {
                std::thread::sleep(Duration::from_millis(200));
            }
            match try_install_dep(dep_name, addons_dir, store) {
                Ok(dep_folders) => {
                    for f in &dep_folders {
                        // Only mark an extracted folder as installed if it is
                        // actually a loadable addon (has a matching manifest);
                        // a stray non-addon folder in the zip must not satisfy
                        // a dependency. Subfolders are gated the same way.
                        if find_manifest(addons_dir, f).is_some() {
                            all_installed.insert(normalize_addon_name(f));
                        }
                        newly_installed_folders.push(f.clone());
                        collect_subfolder_names(&addons_dir.join(f), &mut all_installed);
                    }
                    installed_deps.push(dep_name.clone());
                }
                Err("not_found") => skipped_deps.push(dep_name.clone()),
                Err(_) => failed_deps.push(dep_name.clone()),
            }
        }

        folders_to_scan = newly_installed_folders;
    }

    ResolvedDeps {
        installed_deps,
        failed_deps,
        skipped_deps,
    }
}

/// Try to auto-install a single missing dependency from ESOUI.
/// Returns Ok(folders) on success, or Err(reason) on failure.
fn try_install_dep(
    dep_name: &str,
    addons_dir: &Path,
    store: &mut metadata::MetadataStore,
) -> Result<Vec<String>, &'static str> {
    let dep_id = if let Some(meta) = store.addons.get(dep_name) {
        meta.esoui_id
    } else {
        match esoui::search_addon_by_name(dep_name) {
            Ok(Some(id)) => id,
            Ok(None) => return Err("not_found"),
            Err(_) => return Err("search_failed"),
        }
    };
    let dep_info = esoui::fetch_addon_info(dep_id).map_err(|_| "fetch_failed")?;
    let dep_tmp =
        esoui::download_addon(&dep_info.download_url, None).map_err(|_| "download_failed")?;
    let dep_folders =
        installer::extract_addon_zip(dep_tmp.path(), addons_dir).map_err(|_| "extract_failed")?;

    file_hashes::record_hashes_for_folders(addons_dir, &dep_folders, dep_id, &dep_info.version)
        .map_err(|_| "hash_record_failed")?;

    for f in &dep_folders {
        let dep_version = read_local_version(addons_dir, f);
        metadata::record_install(store, f, dep_id, &dep_version, &dep_info.download_url);
    }
    Ok(dep_folders)
}

/// Normalize an addon folder or dependency name for matching.
///
/// ESO resolves addon names case-INSENSITIVELY, so Kalpa must too: a folder
/// stored on disk as `LUIMedia` still satisfies a `LuiMedia` dependency in-game.
/// Used as the key for both the installed-name set and the version map.
///
/// This deliberately does NOT delete codepoints (e.g. zero-width chars): ESO
/// case-insensitivity does not imply that `Lib\u{200B}Foo` resolves to `LibFoo`,
/// so stripping here would let a non-loadable folder satisfy a dependency.
/// Stray invisible characters in a manifest's `DependsOn` token are cleaned at
/// parse time instead (see `manifest::parse_dependencies`).
pub(crate) fn normalize_addon_name(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Extract just the `AddOnVersion` number from a manifest file without
/// parsing the full manifest.  Returns `None` if the file can't be read
/// or doesn't contain an `AddOnVersion` line.
///
/// Matching mirrors `manifest::parse_manifest` exactly (lossy UTF-8 decode,
/// leading BOM stripped, `## ` directive prefix, `key: value` split) so the
/// version read here can never disagree with the top-level manifest parse — a
/// mismatch (e.g. one invalid byte making this stricter) would manufacture a
/// false "outdated" flag or hide a real one.
fn read_addon_version(manifest_path: &Path) -> Option<u32> {
    let bytes = fs::read(manifest_path).ok()?;
    let raw = String::from_utf8_lossy(&bytes);
    let content: &str = raw.strip_prefix('\u{FEFF}').unwrap_or(&raw);
    for line in content.lines() {
        let Some(line) = line.trim().strip_prefix("## ") else {
            continue;
        };
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim() == "AddOnVersion" {
            return value.trim().parse().ok();
        }
    }
    None
}

/// Record a discovered library version into the version map, keeping the MAX
/// seen for that (normalized) name. A previously-unknown version (`None`, e.g.
/// a top-level copy that declared no `AddOnVersion`) is replaced by a concrete
/// bundled version rather than masking it — otherwise a real outdated copy
/// could go unflagged.
fn merge_max_version(version_map: &mut HashMap<String, Option<u32>>, key: String, ver: u32) {
    let slot = version_map.entry(key).or_insert(None);
    *slot = Some((*slot).map_or(ver, |cur| cur.max(ver)));
}

/// Record a folder as installed (and capture its `AddOnVersion`) IFF it carries
/// a matching manifest (`<name>.txt`/`.addon`) — that is ESO's own rule for
/// loading an addon/library. A bare folder (non-addon dir, or a partial/blocked
/// extraction that lost its manifest) must NOT satisfy a dependency.
fn record_addon(
    dir: &Path,
    name: &str,
    names: &mut HashSet<String>,
    versions: &mut HashMap<String, Option<u32>>,
) {
    let Some(manifest) = find_manifest_in(dir, name) else {
        return;
    };
    let key = normalize_addon_name(name);
    match read_addon_version(&manifest) {
        Some(ver) => merge_max_version(versions, key.clone(), ver),
        // Record the name even with no declared version; a bundled copy with a
        // concrete version may still fill it in via merge_max_version.
        None => {
            versions.entry(key.clone()).or_insert(None);
        }
    }
    names.insert(key);
}

/// Walk the 2 sub-levels inside `folder_path`, recording every manifest-bearing
/// folder. Shared by the full installed index and the install-time resolver so
/// they agree on what counts as installed.
fn collect_subfolders_into(
    folder_path: &Path,
    names: &mut HashSet<String>,
    versions: &mut HashMap<String, Option<u32>>,
) {
    let Ok(sub_entries) = fs::read_dir(folder_path) else {
        return;
    };
    for sub in sub_entries.flatten() {
        let sub_path = sub.path();
        if !sub_path.is_dir() {
            continue;
        }
        if let Some(sub_name) = sub_path.file_name().and_then(|n| n.to_str()) {
            record_addon(&sub_path, sub_name, names, versions);
        }
        if let Ok(sub2_entries) = fs::read_dir(&sub_path) {
            for sub2 in sub2_entries.flatten() {
                let sub2_path = sub2.path();
                if sub2_path.is_dir() {
                    if let Some(sub2_name) = sub2_path.file_name().and_then(|n| n.to_str()) {
                        record_addon(&sub2_path, sub2_name, names, versions);
                    }
                }
            }
        }
    }
}

/// Names-only subfolder walk for callers that don't need versions (the
/// install-time transitive resolver).
fn collect_subfolder_names(folder_path: &Path, out: &mut HashSet<String>) {
    let mut versions = HashMap::new();
    collect_subfolders_into(folder_path, out, &mut versions);
}

/// Scan the AddOns dir once, returning BOTH the set of installed (loadable)
/// addon names and a map of normalized name → max `AddOnVersion`.
///
/// ESO scans top-level addon folders plus 2 levels of subfolders (3 levels total
/// from the AddOns root), matching ESO's own resolution depth on PC. Disabled
/// folders (ending in `.disabled`) are excluded. Both outputs come from the SAME
/// manifest-gated traversal so the "missing" and "outdated" checks can never
/// disagree about what exists or what version it is. Names/keys are normalized
/// (see [`normalize_addon_name`]); callers MUST normalize the name they look up.
pub(crate) fn build_installed_index(
    addons_dir: &Path,
) -> (HashSet<String>, HashMap<String, Option<u32>>) {
    let mut names = HashSet::new();
    let mut versions: HashMap<String, Option<u32>> = HashMap::new();
    let Ok(entries) = fs::read_dir(addons_dir) else {
        return (names, versions);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.ends_with(".disabled") {
            continue;
        }
        // The top-level folder counts only if it is itself an addon, but we
        // still descend into it — ESO discovers nested addons/libraries under
        // plain wrapper folders such as `Libs/`.
        record_addon(&path, name, &mut names, &mut versions);
        collect_subfolders_into(&path, &mut names, &mut versions);
    }
    (names, versions)
}

/// Like [`build_installed_index`] but seeds the TOP-LEVEL addon names/versions
/// from an already-parsed manifest list instead of re-reading (and re-parsing
/// the `AddOnVersion` from) every top-level manifest on disk a second time.
///
/// The startup scan has just parsed every top-level manifest; the parsed list is
/// exactly the set of top-level folders that carry a matching manifest — the same
/// gate [`record_addon`] applies — and each entry carries the same
/// `addon_version` [`read_addon_version`] would recover. Bundled libraries under
/// wrapper folders are NOT in the parsed list, so subfolders are still walked
/// (over every non-disabled top-level directory, matching the descent in
/// [`build_installed_index`]).
fn build_installed_index_from_parsed(
    parsed: &[AddonManifest],
    top_dirs: &[(String, PathBuf, bool)],
) -> (HashSet<String>, HashMap<String, Option<u32>>) {
    let mut names = HashSet::new();
    let mut versions: HashMap<String, Option<u32>> = HashMap::new();

    // Top-level: mirror `record_addon` using the already-parsed manifest data.
    // Disabled folders are excluded, exactly as `build_installed_index` skips
    // `.disabled` directories.
    for addon in parsed {
        if addon.disabled {
            continue;
        }
        let key = normalize_addon_name(&addon.folder_name);
        match addon.addon_version {
            Some(ver) => merge_max_version(&mut versions, key.clone(), ver),
            None => {
                versions.entry(key.clone()).or_insert(None);
            }
        }
        names.insert(key);
    }

    // Subfolders (2 levels): ESO discovers nested addons/libraries under plain
    // wrapper folders such as `Libs/`, so descend into every non-disabled
    // top-level directory just as `build_installed_index` does.
    for (_name, path, disabled) in top_dirs {
        if *disabled {
            continue;
        }
        collect_subfolders_into(path, &mut names, &mut versions);
    }

    (names, versions)
}

/// Names-only view of [`build_installed_index`], for callers that don't need
/// versions (the install-time transitive resolver).
pub(crate) fn build_installed_set(addons_dir: &Path) -> HashSet<String> {
    build_installed_index(addons_dir).0
}

/// Read the local manifest version for a folder, or empty string if not found.
fn read_local_version(addons_dir: &Path, folder: &str) -> String {
    find_manifest(addons_dir, folder)
        .and_then(|p| manifest::parse_manifest(folder, &p))
        .map(|m| m.version)
        .unwrap_or_default()
}

/// Look for a manifest file inside `dir` with the given `base_name`.
fn find_manifest_in(dir: &std::path::Path, base_name: &str) -> Option<PathBuf> {
    let txt = dir.join(format!("{base_name}.txt"));
    if txt.exists() {
        return Some(txt);
    }
    let addon = dir.join(format!("{base_name}.addon"));
    if addon.exists() {
        return Some(addon);
    }
    None
}

pub(crate) fn find_manifest(addons_dir: &std::path::Path, folder_name: &str) -> Option<PathBuf> {
    find_manifest_in(&addons_dir.join(folder_name), folder_name)
}

// ── Enhanced addon folder detection ─────────────────────────────────────

/// Candidate document root directories where ESO might store data.
pub(crate) fn documents_candidates() -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();

    if let Some(doc) = dirs::document_dir() {
        bases.push(doc);
    }

    #[cfg(target_os = "windows")]
    {
        // Raw USERPROFILE\Documents — can differ from Known Folder if user
        // moved Documents to OneDrive or another location.
        if let Ok(userprofile) = std::env::var("USERPROFILE") {
            let alt = PathBuf::from(&userprofile).join("Documents");
            if alt.is_dir() && !bases.iter().any(|b| b == &alt) {
                bases.push(alt);
            }
        }

        // Public Documents edge case (rare but documented)
        let public = PathBuf::from(r"C:\Users\Public\Documents");
        if public.is_dir() && !bases.iter().any(|b| b == &public) {
            bases.push(public);
        }
    }

    bases
}

/// All valid AddOns directories found across document roots and ESO environments.
fn candidate_addons_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let envs = ["live", "liveeu", "pts"];

    for base in documents_candidates() {
        let eso_root = base.join("Elder Scrolls Online");
        if !eso_root.is_dir() {
            continue;
        }
        for env in &envs {
            let addons = eso_root.join(env).join("AddOns");
            if addons.is_dir() {
                out.push(addons);
            }
        }
    }

    out
}

/// Score an AddOns directory to determine which is the "best" candidate.
fn score_addons_dir(addons: &Path) -> i32 {
    let mut score = 0;
    let parent = addons.parent().unwrap_or(addons);

    // ESO settings file means the game has actually been run in this env
    if parent.join("AddOnSettings.txt").is_file() {
        score += 3;
    }
    // SavedVariables dir means addon data exists here
    if parent.join("SavedVariables").is_dir() {
        score += 2;
    }

    // Count addon manifests (cap at 20 to avoid huge scans)
    if let Ok(entries) = std::fs::read_dir(addons) {
        let mut count = 0;
        for entry in entries.flatten() {
            if count >= 20 {
                break;
            }
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let folder_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if find_manifest(addons, &folder_name).is_some() {
                score += 1;
                count += 1;
            }
        }
    }

    score
}

/// Check if a path is inside an OneDrive-synced directory.
pub(crate) fn is_onedrive_path(p: &Path) -> bool {
    p.ancestors().any(|a| {
        a.file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.contains("OneDrive"))
            .unwrap_or(false)
    })
}

/// Collect user-facing warnings based on the selected primary path.
fn collect_warnings(primary: &Path, candidates: &[PathBuf]) -> Vec<String> {
    let mut warnings = Vec::new();

    if is_onedrive_path(primary) {
        warnings.push(
            "Your selected ESO AddOns folder is inside OneDrive. \
             Cloud sync can sometimes cause missing or outdated addons. \
             If you see issues, consider disabling sync for this folder."
                .to_string(),
        );
    }

    if candidates.len() > 1 {
        warnings.push(
            "Multiple ESO AddOns folders were detected. \
             This can happen with NA/EU/PTS clients or when Documents is synced to the cloud."
                .to_string(),
        );
    }

    warnings
}

/// Return a sort priority for the server environment folder (lower = preferred).
fn env_priority(addons: &Path) -> u8 {
    let env = addons
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");
    match env {
        "live" => 0,
        "liveeu" => 1,
        "pts" => 2,
        _ => 3,
    }
}

/// Determine the server environment label from an AddOns path.
fn detect_server_env(addons: &Path) -> String {
    addons
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|env| match env {
            "live" => "NA/EU Live".to_string(),
            "liveeu" => "EU Live".to_string(),
            "pts" => "PTS".to_string(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

/// Count addon folders that have a valid manifest file.
pub(crate) fn count_addon_manifests(addons: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(addons) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| {
            let path = e.path();
            if !path.is_dir() {
                return false;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => return false,
            };
            find_manifest(addons, &name).is_some()
        })
        .count()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedCandidate {
    pub path: String,
    pub server_env: String,
    pub addon_count: usize,
    pub is_onedrive: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonsDetectionResult {
    pub primary: Option<String>,
    pub candidates: Vec<DetectedCandidate>,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub fn detect_addons_folders() -> AddonsDetectionResult {
    let candidates = candidate_addons_dirs();

    if candidates.is_empty() {
        return AddonsDetectionResult {
            primary: None,
            candidates: vec![],
            warnings: vec![],
        };
    }

    // Score and sort: highest score first; on tie, prefer live > liveeu > pts > unknown
    let mut scored: Vec<(PathBuf, i32)> = candidates
        .iter()
        .map(|p| (p.clone(), score_addons_dir(p)))
        .collect();
    scored.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| env_priority(&a.0).cmp(&env_priority(&b.0)))
    });

    let primary_path = &scored[0].0;
    let primary = Some(primary_path.to_string_lossy().to_string());
    let warnings = collect_warnings(primary_path, &candidates);

    let detected: Vec<DetectedCandidate> = scored
        .iter()
        .map(|(p, _)| DetectedCandidate {
            path: p.to_string_lossy().to_string(),
            server_env: detect_server_env(p),
            addon_count: count_addon_manifests(p),
            is_onedrive: is_onedrive_path(p),
        })
        .collect();

    AddonsDetectionResult {
        primary,
        candidates: detected,
        warnings,
    }
}

/// Legacy detection command — thin wrapper for backwards compatibility.
#[tauri::command]
pub fn detect_addons_folder() -> Result<String, String> {
    let result = detect_addons_folders();
    result
        .primary
        .ok_or_else(|| "Could not find ESO AddOns folder. Please set it manually.".to_string())
}

/// Detect all ESO game instances (region × launcher) on this machine.
///
/// Returns a list of [`GameInstance`] structs sorted by activity score (most-active first).  Each
/// instance carries the AddOns path, region, detected launcher type, and
/// helpful metadata (addon count, OneDrive flag, SavedVariables presence).
///
/// Use this command in place of `detect_addons_folders` for the setup wizard
/// and the instance-switcher in settings.
#[tauri::command]
pub fn detect_game_instances() -> Vec<crate::game_instances::GameInstance> {
    crate::game_instances::detect_all_game_instances()
}

#[tauri::command]
pub async fn scan_installed_addons(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    addons_path: String,
) -> Result<Vec<AddonManifest>, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let cache_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app data dir: {e}"))?;
    let lock = meta_lock.0.clone();
    tokio::task::spawn_blocking(move || {
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;
        scan_installed_addons_blocking(&addons_dir, &cache_dir)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

fn scan_installed_addons_blocking(
    addons_dir: &Path,
    cache_dir: &Path,
) -> Result<Vec<AddonManifest>, String> {
    let entries =
        fs::read_dir(addons_dir).map_err(|e| format!("Failed to read AddOns folder: {e}"))?;

    // Collect top-level directories first so we can process them in parallel.
    // Each entry is (base_name, path, is_disabled).
    //
    // When both `Foo/` and `Foo.disabled/` exist on disk the enabled copy wins
    // and the disabled duplicate is silently ignored.  This prevents two
    // manifests with the same `folder_name` from colliding in the UI, cache,
    // and downstream commands (remove / enable / update).
    let mut seen: HashMap<String, (PathBuf, bool)> = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let is_disabled = name.ends_with(".disabled");
        let base_name = if is_disabled {
            name.strip_suffix(".disabled").unwrap_or(&name).to_string()
        } else {
            name
        };
        // If we already saw an enabled copy, skip the disabled duplicate
        match seen.get(&base_name) {
            Some((_, false)) if is_disabled => continue,
            _ => {}
        }
        seen.insert(base_name, (path, is_disabled));
    }
    let top_dirs: Vec<(String, PathBuf, bool)> = seen
        .into_iter()
        .map(|(name, (path, disabled))| (name, path, disabled))
        .collect();

    // Open SQLite manifest cache in the app data dir (not the AddOns folder,
    // which ESO scans recursively and could cause odd behavior with a .db file).
    let folder_names: Vec<&str> = top_dirs.iter().map(|(name, _, _)| name.as_str()).collect();
    let mut cache_conn = manifest_cache::open_and_prune(cache_dir, &folder_names);

    // Two-pass strategy:
    // 1. Check the cache for each folder (sequential — SQLite is single-threaded)
    // 2. Parse uncached manifests in parallel with rayon, then store results back
    let mut addons: Vec<AddonManifest> = Vec::with_capacity(top_dirs.len());
    let mut uncached: Vec<(String, PathBuf, bool)> = Vec::new();

    for (base_name, path, is_disabled) in &top_dirs {
        let manifest_path = match find_manifest_in(path, base_name) {
            Some(p) => p,
            None => continue,
        };
        if let Some(ref conn) = cache_conn {
            if let Some(mut cached) =
                manifest_cache::parse_manifest_cached(conn, base_name, &manifest_path)
            {
                cached.disabled = *is_disabled;
                addons.push(cached);
                continue;
            }
        }
        uncached.push((base_name.clone(), manifest_path, *is_disabled));
    }

    // Parse uncached manifests in parallel
    let newly_parsed: Vec<(PathBuf, AddonManifest, bool)> = uncached
        .par_iter()
        .filter_map(|(folder_name, manifest_path, is_disabled)| {
            let mut m = manifest::parse_manifest(folder_name, manifest_path)?;
            m.disabled = *is_disabled;
            Some((manifest_path.clone(), m, *is_disabled))
        })
        .collect();

    // Store newly parsed manifests in a single transaction for performance.
    // Without this, each INSERT is its own implicit transaction with a WAL flush.
    if let Some(ref mut conn) = cache_conn {
        if let Ok(tx) = conn.transaction() {
            for (manifest_path, m, _) in &newly_parsed {
                manifest_cache::store_parsed(&tx, &m.folder_name, manifest_path, m);
            }
            let _ = tx.commit();
        }
    }
    addons.extend(newly_parsed.into_iter().map(|(_, m, _)| m));

    // Derive the installed-name set and version map from the manifests just
    // parsed above (top level) plus a subfolder-only walk for bundled libraries,
    // instead of re-reading every top-level manifest a second time. "missing" and
    // "outdated" still agree on what exists because the same manifest gate and
    // `AddOnVersion` value feed both.
    let (installed, version_map) = build_installed_index_from_parsed(&addons, &top_dirs);

    // Load metadata and clean up stale entries:
    // - Remove entries for addon folders that no longer exist on disk
    // - Deduplicate entries with the same esoui_id (keep the one that exists)
    let mut store = metadata::load_metadata(addons_dir);
    // The directory scan above already enumerated every top-level folder (enabled
    // or .disabled) into `top_dirs`, so a metadata key matching one of those base
    // names is known-present without touching the filesystem again. Keys that miss
    // the set still get the original stat checks — a metadata key can differ from
    // the on-disk name in case (Windows is case-insensitive) or carry an unusual
    // suffix, and those must keep pruning exactly as before.
    let on_disk: HashSet<&str> = top_dirs.iter().map(|(name, _, _)| name.as_str()).collect();
    let stale: Vec<String> = store
        .addons
        .keys()
        .filter(|name| {
            !on_disk.contains(name.as_str())
                && !addons_dir.join(name).is_dir()
                && !addons_dir.join(format!("{name}.disabled")).is_dir()
        })
        .cloned()
        .collect();
    if !stale.is_empty() {
        for name in &stale {
            metadata::remove_entry(&mut store, name);
        }
        if let Err(e) = metadata::save_metadata(addons_dir, &store) {
            eprintln!("Warning: failed to prune stale metadata: {e}");
        }
    }

    // Modified-file counts live in one JSON manifest per addon under
    // .kalpa-hashes; each read is independent disk I/O, so collect them in
    // parallel (mirroring the manifest parsing above) instead of paying one
    // sequential read per addon inside the enrichment loop.
    let modified_counts: HashMap<String, u32> = addons
        .par_iter()
        .filter_map(|addon| {
            file_hashes::load_modified_file_count(addons_dir, &addon.folder_name)
                .map(|count| (addon.folder_name.clone(), count))
        })
        .collect();

    // Check for missing/outdated dependencies and enrich with ESOUI ID.
    // All lookups normalize the dependency name so resolution is
    // case-insensitive, matching ESO (and the normalized `installed`/`version_map`).
    for addon in &mut addons {
        addon.missing_dependencies = addon
            .depends_on
            .iter()
            .filter(|dep| !installed.contains(&normalize_addon_name(&dep.name)))
            .map(|dep| dep.name.clone())
            .collect();

        addon.outdated_dependencies = addon
            .depends_on
            .iter()
            .filter(|dep| {
                let Some(min) = dep.min_version else {
                    return false;
                };
                let key = normalize_addon_name(&dep.name);
                if !installed.contains(&key) {
                    return false;
                }
                match version_map.get(&key) {
                    Some(Some(installed_ver)) => *installed_ver < min,
                    _ => false,
                }
            })
            .map(|dep| dep.name.clone())
            .collect();

        // Optional dependencies that are not installed (subfolder-aware,
        // case-insensitive) so the UI can show present/absent for them too.
        addon.missing_optional_dependencies = addon
            .optional_depends_on
            .iter()
            .filter(|dep| !installed.contains(&normalize_addon_name(&dep.name)))
            .map(|dep| dep.name.clone())
            .collect();

        if let Some(meta) = store.addons.get(&addon.folder_name) {
            addon.esoui_id = Some(meta.esoui_id);
            addon.tags = meta.tags.clone();
            addon.esoui_last_update = meta.esoui_last_update;
            addon.installed_at = meta.installed_at.clone();
        }

        if let Some(count) = modified_counts.get(&addon.folder_name) {
            addon.modified_file_count = *count;
        }
    }

    // Cached keys: comparison-time lowercasing allocates a String per compare
    // (O(n log n) allocations); the key itself is unchanged.
    addons.sort_by_cached_key(|a| a.title.to_lowercase());

    Ok(addons)
}

#[tauri::command]
pub async fn set_addon_tags(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    addons_path: String,
    folder_name: String,
    tags: Vec<String>,
) -> Result<(), String> {
    validate_name(&folder_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let lock = meta_lock.0.clone();
    tokio::task::spawn_blocking(move || {
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;
        let mut store = metadata::load_metadata(&addons_dir);
        match store.addons.get_mut(&folder_name) {
            Some(meta) => meta.tags = tags,
            None => {
                // Create a minimal entry for untracked addons so tags can be saved
                store.addons.insert(
                    folder_name.clone(),
                    metadata::AddonMetadata {
                        esoui_id: 0,
                        installed_version: String::new(),
                        download_url: String::new(),
                        installed_at: String::new(),
                        tags,
                        esoui_last_update: 0,
                    },
                );
            }
        }
        metadata::save_metadata(&addons_dir, &store)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn resolve_esoui_addon(input: String) -> Result<EsouiAddonInfo, String> {
    tokio::task::spawn_blocking(move || {
        let id = esoui::parse_esoui_input(&input)?;
        esoui::fetch_addon_info(id)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn fetch_esoui_detail(esoui_id: u32) -> Result<EsouiAddonDetail, String> {
    tokio::task::spawn_blocking(move || esoui::fetch_addon_detail(esoui_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn search_esoui_addons(query: String) -> Result<Vec<EsouiSearchResult>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    tokio::task::spawn_blocking(move || esoui::search_esoui(&query))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn install_addon(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    addons_path: String,
    download_url: String,
    esoui_id: u32,
    esoui_title: String,
    esoui_version: String,
) -> Result<InstallResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;

    if !download_url.starts_with("https://cdn.esoui.com/")
        && !download_url.starts_with("https://www.esoui.com/")
    {
        return Err("Invalid download URL: only ESOUI download links are allowed.".to_string());
    }

    let lock = meta_lock.0.clone();
    tokio::task::spawn_blocking(move || {
        // Download outside the lock — network I/O doesn't touch kalpa.json
        let tmp_file = esoui::download_addon(&download_url, None)?;

        // Acquire lock only for extract + metadata update
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;
        install_addon_blocking(
            &addons_dir,
            tmp_file,
            &download_url,
            esoui_id,
            &esoui_title,
            &esoui_version,
        )
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

fn install_addon_blocking(
    addons_dir: &Path,
    tmp_file: NamedTempFile,
    download_url: &str,
    esoui_id: u32,
    esoui_title: &str,
    esoui_version: &str,
) -> Result<InstallResult, String> {
    let installed_folders = installer::extract_addon_zip(tmp_file.path(), addons_dir)?;

    file_hashes::record_hashes_for_folders(
        addons_dir,
        &installed_folders,
        esoui_id,
        esoui_version,
    )?;

    let mut store = metadata::load_metadata(addons_dir);

    // Only the primary folder gets the esoui_id so that check_for_updates
    // compares versions correctly. Secondary folders get esoui_id 0.
    record_installed_folders(
        &mut store,
        addons_dir,
        &installed_folders,
        esoui_id,
        esoui_version,
        esoui_title,
        download_url,
        0, // esoui_last_update will be populated during next update check
    );

    let resolved = resolve_transitive_deps(addons_dir, &installed_folders, &mut store);

    metadata::save_metadata(addons_dir, &store)?;

    Ok(InstallResult {
        installed_folders,
        installed_deps: resolved.installed_deps,
        failed_deps: resolved.failed_deps,
        skipped_deps: resolved.skipped_deps,
    })
}

#[tauri::command]
pub async fn remove_addon(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    addons_path: String,
    folder_name: String,
) -> Result<(), String> {
    validate_name(&folder_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let lock = meta_lock.0.clone();
    tokio::task::spawn_blocking(move || {
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;

        // Remove both the enabled and disabled copies if they exist.
        // If only one exists, remove that one. Handles the edge case where
        // an external tool or reinstall left both Foo/ and Foo.disabled/.
        let enabled_exists = addons_dir.join(&folder_name).is_dir();
        let disabled_name = format!("{folder_name}.disabled");
        let disabled_exists = addons_dir.join(&disabled_name).is_dir();

        if enabled_exists {
            installer::remove_addon(&addons_dir, &folder_name)?;
        }
        if disabled_exists {
            installer::remove_addon(&addons_dir, &disabled_name)?;
        }
        if !enabled_exists && !disabled_exists {
            return Err(format!("Addon folder not found: {folder_name}"));
        }

        // Clean up metadata
        let mut store = metadata::load_metadata(&addons_dir);
        metadata::remove_entry(&mut store, &folder_name);
        metadata::save_metadata(&addons_dir, &store)?;

        Ok(())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub fn disable_addon(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    folder_name: String,
) -> Result<(), String> {
    validate_name(&folder_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let src = addons_dir.join(&folder_name);
    if !src.is_dir() {
        return Err(format!("Addon folder not found: {folder_name}"));
    }
    let dst = addons_dir.join(format!("{folder_name}.disabled"));
    if dst.exists() {
        return Err(format!("{folder_name} is already disabled."));
    }
    fs::rename(&src, &dst).map_err(|e| format!("Failed to disable {folder_name}: {e}"))
}

#[tauri::command]
pub fn enable_addon(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    folder_name: String,
) -> Result<(), String> {
    validate_name(&folder_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let src = addons_dir.join(format!("{folder_name}.disabled"));
    if !src.is_dir() {
        return Err(format!("Disabled addon folder not found: {folder_name}"));
    }
    let dst = addons_dir.join(&folder_name);
    if dst.exists() {
        return Err(format!("A folder named {folder_name} already exists."));
    }
    fs::rename(&src, &dst).map_err(|e| format!("Failed to enable {folder_name}: {e}"))
}

#[tauri::command]
pub async fn install_dependency(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    addons_path: String,
    dep_name: String,
) -> Result<InstallResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let lock = meta_lock.0.clone();
    tokio::task::spawn_blocking(move || {
        // Network I/O outside the lock: search ESOUI, fetch info, download ZIP
        let dep_id = {
            let store = metadata::load_metadata(&addons_dir);
            if let Some(meta) = store.addons.get(&*dep_name) {
                meta.esoui_id
            } else {
                match esoui::search_addon_by_name(&dep_name) {
                    Ok(Some(id)) => id,
                    Ok(None) => return Err(format!("Failed to install {dep_name}: not_found")),
                    Err(_) => return Err(format!("Failed to install {dep_name}: search_failed")),
                }
            }
        };
        let dep_info = esoui::fetch_addon_info(dep_id)
            .map_err(|_| format!("Failed to install {dep_name}: fetch_failed"))?;
        let dep_tmp = esoui::download_addon(&dep_info.download_url, None)
            .map_err(|_| format!("Failed to install {dep_name}: download_failed"))?;

        // Acquire lock only for extract + metadata update
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;
        install_dependency_blocking(&addons_dir, &dep_name, dep_id, dep_info, dep_tmp)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

fn install_dependency_blocking(
    addons_dir: &Path,
    dep_name: &str,
    dep_id: u32,
    dep_info: EsouiAddonInfo,
    dep_tmp: NamedTempFile,
) -> Result<InstallResult, String> {
    // Surface the real extraction error (installer already explains the common
    // Controlled Folder Access / permission case with fix steps) rather than a
    // generic "extract_failed" the user can't act on.
    let dep_folders = installer::extract_addon_zip(dep_tmp.path(), addons_dir)
        .map_err(|e| format!("Failed to install {dep_name}: {e}"))?;

    file_hashes::record_hashes_for_folders(addons_dir, &dep_folders, dep_id, &dep_info.version)
        .map_err(|e| format!("Failed to install {dep_name}: {e}"))?;

    let mut store = metadata::load_metadata(addons_dir);
    for f in &dep_folders {
        let dep_version = read_local_version(addons_dir, f);
        metadata::record_install(&mut store, f, dep_id, &dep_version, &dep_info.download_url);
    }

    let resolved = resolve_transitive_deps(addons_dir, &dep_folders, &mut store);
    metadata::save_metadata(addons_dir, &store)?;
    Ok(InstallResult {
        installed_folders: dep_folders,
        installed_deps: resolved.installed_deps,
        failed_deps: resolved.failed_deps,
        skipped_deps: resolved.skipped_deps,
    })
}

#[tauri::command]
pub async fn check_for_updates(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    addons_path: String,
) -> Result<Vec<UpdateCheckResult>, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let lock = meta_lock.0.clone();
    tokio::task::spawn_blocking(move || {
        // Phase 0: fetch the full ESOUI filelist outside the lock — big HTTP call
        let api_lookup = esoui::fetch_filelist_lookup()?;

        // Phase 1: acquire lock for metadata comparison and save
        let pending = {
            let _guard = lock
                .lock()
                .map_err(|_| "Internal metadata lock error".to_string())?;
            check_for_updates_metadata(&addons_dir, &api_lookup)?
        };
        // Lock is released here

        // Phase 2: assemble final results (pure data, no lock needed).
        //
        // download_url is filled from the stored fallback_url. We deliberately
        // do NOT make a filedetails request per pending update here: the field
        // is never read by the frontend, and every update path
        // (update_addon / update_batch_with_decisions) re-resolves the real
        // download URL via fetch_addon_info itself. Pre-fetching it added one
        // uncached HTTPS request per pending update to every startup/refresh —
        // most painful on patch day when everything has an update.
        let results: Vec<UpdateCheckResult> = pending
            .into_iter()
            .map(|p| UpdateCheckResult {
                folder_name: p.folder_name,
                esoui_id: p.esoui_id,
                current_version: p.current_version,
                remote_version: p.remote_version,
                download_url: p.fallback_url,
                has_update: p.has_update,
                remote_last_update: p.remote_last_update,
            })
            .collect();

        Ok(results)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

struct UpdatePending {
    folder_name: String,
    esoui_id: u32,
    current_version: String,
    remote_version: String,
    fallback_url: String,
    has_update: bool,
    remote_last_update: u64,
}

/// Phase 1 of check_for_updates: compare local metadata against the ESOUI API
/// lookup table. Must be called under the metadata lock.
fn check_for_updates_metadata(
    addons_dir: &Path,
    api_lookup: &HashMap<String, Arc<esoui::ApiAddonLookup>>,
) -> Result<Vec<UpdatePending>, String> {
    let mut store = metadata::load_metadata(addons_dir);
    let mut metadata_changed = false;

    let folder_names: Vec<String> = store.addons.keys().cloned().collect();

    let mut pending: Vec<UpdatePending> = Vec::new();

    for folder_name in &folder_names {
        if !addons_dir.join(folder_name).is_dir() {
            continue;
        }

        let meta = match store.addons.get(folder_name) {
            Some(m) => m.clone(),
            None => continue,
        };

        if meta.esoui_id == 0 {
            continue;
        }

        let api_entry = match api_lookup.get(folder_name) {
            Some(entry) => entry,
            None => continue,
        };

        // Normalize versions: strip leading "v"/"V" and trim whitespace
        let local_ver = meta
            .installed_version
            .trim()
            .strip_prefix('v')
            .or_else(|| meta.installed_version.trim().strip_prefix('V'))
            .unwrap_or(meta.installed_version.trim());
        let remote_ver = api_entry
            .version
            .trim()
            .strip_prefix('v')
            .or_else(|| api_entry.version.trim().strip_prefix('V'))
            .unwrap_or(api_entry.version.trim());

        let has_update = !remote_ver.is_empty() && !local_ver.is_empty() && remote_ver != local_ver;

        if let Some(entry) = store.addons.get_mut(folder_name) {
            if !has_update && meta.installed_version != api_entry.version {
                entry.installed_version = api_entry.version.clone();
                metadata_changed = true;
            }
            if entry.esoui_last_update != api_entry.last_update {
                entry.esoui_last_update = api_entry.last_update;
                metadata_changed = true;
            }
        }

        pending.push(UpdatePending {
            folder_name: folder_name.clone(),
            esoui_id: meta.esoui_id,
            current_version: meta.installed_version.clone(),
            remote_version: api_entry.version.clone(),
            fallback_url: meta.download_url.clone(),
            has_update,
            remote_last_update: api_entry.last_update,
        });
    }

    if metadata_changed {
        if let Err(e) = metadata::save_metadata(addons_dir, &store) {
            eprintln!("Warning: failed to save metadata after update check: {e}");
        }
    }

    Ok(pending)
}

// ── Update cancellation + progress ──────────────────────────────────────

/// Per-file progress for a single addon update, emitted on the `update-progress`
/// event. The frontend correlates events by `operation_id` and renders the phase
/// plus "Extracting N of M".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgressEvent {
    pub operation_id: String,
    pub folder_name: String,
    /// Currently always "extracting" — the slow, file-count-bound phase.
    pub phase: String,
    pub file_index: usize,
    pub file_total: usize,
}

/// RAII registration of a cancellation flag in [`crate::UpdateCancels`]. Inserted
/// on construction, removed on drop (every exit path: success, error, or panic),
/// so a stale flag can never cancel a later, unrelated operation. An empty
/// `operation_id` (caller opted out of cancellation) registers nothing.
struct CancelGuard {
    registry: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    operation_id: String,
    flag: Arc<AtomicBool>,
}

impl CancelGuard {
    fn register(
        registry: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
        operation_id: String,
    ) -> Self {
        let flag = Arc::new(AtomicBool::new(false));
        if !operation_id.is_empty() {
            if let Ok(mut map) = registry.lock() {
                map.insert(operation_id.clone(), flag.clone());
            }
        }
        Self {
            registry,
            operation_id,
            flag,
        }
    }

    fn flag(&self) -> &AtomicBool {
        &self.flag
    }
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if !self.operation_id.is_empty() {
            if let Ok(mut map) = self.registry.lock() {
                map.remove(&self.operation_id);
            }
        }
    }
}

/// How many files must pass since the last emitted progress event before the
/// next one is sent, so a thousands-of-files addon doesn't flood the event bus.
const PROGRESS_EMIT_STRIDE: usize = 64;

/// Throttle decision for the extraction-progress emitter, factored out as a pure
/// function so the first/every-stride/last contract is directly testable.
///
/// `prev` is the `done` value at the last emitted event, or `usize::MAX` if none
/// has been emitted yet. Emits when: this is the first event, OR we've reached
/// completion (`done >= total`), OR at least `PROGRESS_EMIT_STRIDE` files have
/// passed since the last emit. The first-event branch is independent of the
/// stride, so the frontend reliably learns the operation id (enabling Stop) on
/// the very first file even for a small archive.
fn should_emit_progress(prev: usize, done: usize, total: usize) -> bool {
    prev == usize::MAX || done >= total || done.saturating_sub(prev) >= PROGRESS_EMIT_STRIDE
}

/// Build a throttled extraction-progress callback that emits `update-progress`
/// events: on the first file, every `PROGRESS_EMIT_STRIDE` files, and at
/// completion, so a thousands-of-files addon doesn't flood the event bus.
fn make_progress_emitter(
    app: tauri::AppHandle,
    operation_id: String,
    folder_name: String,
) -> impl Fn(usize, usize) {
    let last = AtomicUsize::new(usize::MAX);
    move |done: usize, total: usize| {
        let prev = last.load(Ordering::Relaxed);
        if !should_emit_progress(prev, done, total) {
            return;
        }
        last.store(done, Ordering::Relaxed);
        let _ = app.emit(
            "update-progress",
            UpdateProgressEvent {
                operation_id: operation_id.clone(),
                folder_name: folder_name.clone(),
                phase: "extracting".to_string(),
                file_index: done,
                file_total: total,
            },
        );
    }
}

/// Signal an in-flight update (identified by `operation_id`) to stop. Returns
/// `true` if a matching in-flight operation was found and flagged, `false` if it
/// had already finished or never existed. The extraction loop polls the flag
/// between files and aborts cleanly, rolling back any partially-written folder.
#[tauri::command]
pub async fn cancel_update(
    cancels: tauri::State<'_, crate::UpdateCancels>,
    operation_id: String,
) -> Result<bool, String> {
    let map = cancels
        .0
        .lock()
        .map_err(|_| "Internal cancel registry lock error".to_string())?;
    match map.get(&operation_id) {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            Ok(true)
        }
        None => Ok(false),
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn update_addon(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    cancels: tauri::State<'_, crate::UpdateCancels>,
    app: tauri::AppHandle,
    addons_path: String,
    esoui_id: u32,
    api_version: Option<String>,
    operation_id: Option<String>,
) -> Result<InstallResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let lock = meta_lock.0.clone();
    let registry = cancels.0.clone();
    let operation_id = operation_id.unwrap_or_default();
    let cancel = CancelGuard::register(registry, operation_id.clone());
    tokio::task::spawn_blocking(move || {
        // Network I/O outside the lock: fetch info + download ZIP
        let info = esoui::fetch_addon_info(esoui_id)?;
        let tmp_file = esoui::download_addon(&info.download_url, None)?;

        // Acquire lock only for extract + metadata update
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;

        // Build a progress emitter for the file-bound extraction. The
        // cancellation flag was registered before this blocking task started, so
        // a Stop request made while downloading or waiting on the metadata lock
        // is still observed before extraction writes files.
        let progress = make_progress_emitter(app, operation_id, info.title.clone());
        let hooks = installer::ExtractHooks {
            cancel: Some(cancel.flag()),
            progress: Some(&progress),
        };
        update_addon_blocking(
            &addons_dir,
            esoui_id,
            api_version.as_deref(),
            info,
            tmp_file,
            hooks,
        )
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

fn update_addon_blocking(
    addons_dir: &Path,
    esoui_id: u32,
    api_version: Option<&str>,
    info: EsouiAddonInfo,
    tmp_file: NamedTempFile,
    hooks: installer::ExtractHooks,
) -> Result<InstallResult, String> {
    // Extract the downloaded ZIP
    let installed_folders = installer::extract_addon_zip_with(tmp_file.path(), addons_dir, hooks)?;

    // Store the API version (from filelist.json) when available, since
    // check_for_updates compares against the API version. Using the
    // HTML-scraped version here caused perpetual "update available" when
    // the two sources returned slightly different version strings.
    let version = api_version.unwrap_or(&info.version);

    file_hashes::record_hashes_for_folders(addons_dir, &installed_folders, esoui_id, version)?;

    // Clean up any old metadata entries for the same esoui_id
    // that aren't in the newly extracted folders (handles addon renames).
    let mut store = metadata::load_metadata(addons_dir);
    let old_folders: Vec<String> = store
        .addons
        .iter()
        .filter(|(_, m)| m.esoui_id == esoui_id)
        .map(|(name, _)| name.clone())
        .collect();
    for old in &old_folders {
        if !installed_folders.contains(old) {
            metadata::remove_entry(&mut store, old);
        }
    }
    record_installed_folders(
        &mut store,
        addons_dir,
        &installed_folders,
        esoui_id,
        version,
        &info.title,
        &info.download_url,
        0, // preserved from existing metadata
    );

    let resolved = resolve_transitive_deps(addons_dir, &installed_folders, &mut store);

    metadata::save_metadata(addons_dir, &store)?;

    Ok(InstallResult {
        installed_folders,
        installed_deps: resolved.installed_deps,
        failed_deps: resolved.failed_deps,
        skipped_deps: resolved.skipped_deps,
    })
}

// ── Batch update (parallel downloads, sequential extraction) ────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchUpdateEntry {
    pub esoui_id: u32,
    pub folder_name: String,
    pub api_version: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchUpdateResult {
    pub completed: Vec<String>,
    pub failed: Vec<String>,
    pub errors: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchUpdateProgress {
    pub folder_name: String,
    /// "downloading" | "extracting" | "completed" | "failed"
    pub phase: String,
    pub index: usize,
    pub total: usize,
}

#[tauri::command]
pub async fn batch_update_addons(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    app: tauri::AppHandle,
    addons_path: String,
    updates: Vec<BatchUpdateEntry>,
) -> Result<BatchUpdateResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let lock = meta_lock.0.clone();
    tokio::task::spawn_blocking(move || {
        // Phase 1 (parallel): download ZIPs outside the lock — no metadata access
        let download_results = batch_download_addons(&updates, &app)?;

        // Phase 2: acquire lock only for extract + metadata update
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;
        batch_extract_and_record(&addons_dir, download_results, updates.len(), &app)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

struct BatchDownloaded {
    tmp: NamedTempFile,
    info: EsouiAddonInfo,
    esoui_id: u32,
    api_version: String,
    index: usize,
}

type BatchDownloadResults = Vec<(String, Result<BatchDownloaded, String>)>;

/// Phase 1 of batch_update_addons: parallel downloads without holding the
/// metadata lock. Network I/O doesn't touch kalpa.json.
fn batch_download_addons(
    updates: &[BatchUpdateEntry],
    app: &tauri::AppHandle,
) -> Result<BatchDownloadResults, String> {
    let total = updates.len();

    // Emit "downloading" for all addons at the start
    for (i, entry) in updates.iter().enumerate() {
        let _ = app.emit(
            "batch-update-progress",
            BatchUpdateProgress {
                folder_name: entry.folder_name.clone(),
                phase: "downloading".to_string(),
                index: i,
                total,
            },
        );
    }

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .map_err(|e| format!("Thread pool error: {e}"))?;

    let app_clone = app.clone();
    let download_results: Vec<(String, Result<BatchDownloaded, String>)> = pool.install(|| {
        updates
            .par_iter()
            .enumerate()
            .map(|(i, entry)| {
                let result = fetch_and_download_with_retry(entry.esoui_id).map(|(tmp, info)| {
                    // Emit "extracting" as soon as download finishes
                    let _ = app_clone.emit(
                        "batch-update-progress",
                        BatchUpdateProgress {
                            folder_name: entry.folder_name.clone(),
                            phase: "extracting".to_string(),
                            index: i,
                            total,
                        },
                    );
                    BatchDownloaded {
                        tmp,
                        info,
                        esoui_id: entry.esoui_id,
                        api_version: entry.api_version.clone(),
                        index: i,
                    }
                });
                if result.is_err() {
                    let _ = app_clone.emit(
                        "batch-update-progress",
                        BatchUpdateProgress {
                            folder_name: entry.folder_name.clone(),
                            phase: "failed".to_string(),
                            index: i,
                            total,
                        },
                    );
                }
                (entry.folder_name.clone(), result)
            })
            .collect()
    });

    Ok(download_results)
}

/// Phase 2 of batch_update_addons: extract ZIPs and record metadata.
/// Must be called under the metadata lock.
fn batch_extract_and_record(
    addons_dir: &Path,
    download_results: BatchDownloadResults,
    total: usize,
    app: &tauri::AppHandle,
) -> Result<BatchUpdateResult, String> {
    let mut store = metadata::load_metadata(addons_dir);
    let mut completed: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut errors: HashMap<String, String> = HashMap::new();

    for (folder_name, result) in download_results {
        match result {
            Err(e) => {
                errors.insert(folder_name.clone(), e);
                failed.push(folder_name);
                // Already emitted "failed" during download phase
            }
            Ok(dl) => match installer::extract_addon_zip(dl.tmp.path(), addons_dir) {
                Err(e) => {
                    let _ = app.emit(
                        "batch-update-progress",
                        BatchUpdateProgress {
                            folder_name: folder_name.clone(),
                            phase: "failed".to_string(),
                            index: dl.index,
                            total,
                        },
                    );
                    errors.insert(folder_name.clone(), e);
                    failed.push(folder_name);
                }
                Ok(installed_folders) => {
                    let version = &dl.api_version;

                    if let Err(e) = file_hashes::record_hashes_for_folders(
                        addons_dir,
                        &installed_folders,
                        dl.esoui_id,
                        version,
                    ) {
                        // Files are on disk, but the hash baseline didn't persist.
                        // Don't record this addon in metadata — leaving metadata
                        // pointing at a folder with no baseline would let the next
                        // update silently clobber user edits. Surface it as failed.
                        let _ = app.emit(
                            "batch-update-progress",
                            BatchUpdateProgress {
                                folder_name: folder_name.clone(),
                                phase: "failed".to_string(),
                                index: dl.index,
                                total,
                            },
                        );
                        errors.insert(folder_name.clone(), e);
                        failed.push(folder_name);
                        continue;
                    }

                    // Clean up old metadata entries for this esoui_id
                    // that aren't in the newly extracted folders (handles renames)
                    let old_folders: Vec<String> = store
                        .addons
                        .iter()
                        .filter(|(_, m)| m.esoui_id == dl.esoui_id)
                        .map(|(name, _)| name.clone())
                        .collect();
                    for old in &old_folders {
                        if !installed_folders.contains(old) {
                            metadata::remove_entry(&mut store, old);
                        }
                    }
                    record_installed_folders(
                        &mut store,
                        addons_dir,
                        &installed_folders,
                        dl.esoui_id,
                        version,
                        &dl.info.title,
                        &dl.info.download_url,
                        0,
                    );
                    let _ = app.emit(
                        "batch-update-progress",
                        BatchUpdateProgress {
                            folder_name: folder_name.clone(),
                            phase: "completed".to_string(),
                            index: dl.index,
                            total,
                        },
                    );
                    completed.push(folder_name);
                }
            },
        }
    }

    metadata::save_metadata(addons_dir, &store)?;

    Ok(BatchUpdateResult {
        completed,
        failed,
        errors,
    })
}

// ── Protected Edits: Conflict scanning & file browser ───────���─────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileConflict {
    pub relative_path: String,
    pub user_hash: String,
    pub upstream_hash: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictReport {
    pub session_id: String,
    pub folder_name: String,
    pub update_version: String,
    pub safe_files: Vec<String>,
    pub auto_kept_files: Vec<String>,
    pub conflicts: Vec<FileConflict>,
}

fn generate_session_id(folder_name: &str) -> String {
    use sha2::{Digest, Sha256};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let input = format!("{folder_name}-{nanos}");
    let hash = Sha256::digest(input.as_bytes());
    file_hashes::to_hex(&hash[..16])
}

/// Build a conflict report for one addon folder against a downloaded ZIP, and
/// return the ZIP hash map alongside it so an immediately-following extraction
/// can reuse it as the new hash baseline instead of re-hashing the folder.
///
/// The on-disk hash pass is skipped entirely when no stored manifest exists:
/// with `stored == None`, every file resolves to `user_modified == false`
/// regardless of its disk hash (see the match below), so the disk hashes cannot
/// affect the report — and roughly two-thirds of installed addons have no
/// `.kalpa-hashes` manifest yet, making this the common case.
fn build_conflict_report(
    addons_dir: &Path,
    folder_name: &str,
    zip_path: &Path,
    update_version: &str,
    session_id: &str,
) -> Result<(ConflictReport, HashMap<String, String>), String> {
    let stored = file_hashes::load_hash_manifest(addons_dir, folder_name);
    let addon_path = addons_dir.join(folder_name);

    // Only hash the folder from disk when there is a baseline to compare against.
    // Without one, `user_modified` is always false, so the disk pass is dead work.
    let disk_hashes = if stored.is_some() && addon_path.is_dir() {
        file_hashes::compute_addon_hashes(&addon_path)?
    } else {
        HashMap::new()
    };

    let zip_hashes = file_hashes::hash_zip_entries(zip_path, folder_name)?;

    let stored_files = stored.as_ref().map(|m| &m.files);

    let mut safe_files = Vec::new();
    let mut auto_kept_files = Vec::new();
    let mut conflicts = Vec::new();

    for (rel_path, zip_hash) in &zip_hashes {
        let stored_hash = stored_files.and_then(|f| f.get(rel_path));
        let disk_hash = disk_hashes.get(rel_path);

        let user_modified = match (stored_hash, disk_hash) {
            (Some(stored), Some(disk)) => !file_hashes::signatures_match(stored, disk),
            (Some(_), None) => true, // file deleted
            (None, _) => false,      // no stored hash = no baseline = treat as unmodified
        };

        let upstream_changed = match stored_hash {
            // ZIP-vs-baseline comparisons must be exact. The mixed
            // legacy-SHA/size-signature bridge is only safe for live disk
            // checks; using it here would hide upstream binary size changes.
            Some(stored) => stored != zip_hash,
            None => true, // new file in upstream or no baseline
        };

        match (user_modified, upstream_changed) {
            (false, _) => safe_files.push(rel_path.clone()),
            (true, false) => auto_kept_files.push(rel_path.clone()),
            (true, true) => {
                conflicts.push(FileConflict {
                    relative_path: rel_path.clone(),
                    user_hash: disk_hash.cloned().unwrap_or_default(),
                    upstream_hash: zip_hash.clone(),
                });
            }
        }
    }

    // New files in ZIP (not in stored manifest) are always safe
    // They're already in safe_files from the loop above (no baseline = unmodified)

    safe_files.sort();
    auto_kept_files.sort();
    conflicts.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    let report = ConflictReport {
        session_id: session_id.to_string(),
        folder_name: folder_name.to_string(),
        update_version: update_version.to_string(),
        safe_files,
        auto_kept_files,
        conflicts,
    };

    Ok((report, zip_hashes))
}

#[tauri::command]
pub async fn scan_update_conflicts(
    state: tauri::State<'_, AllowedAddonsPath>,
    pending: tauri::State<'_, crate::PendingUpdates>,
    addons_path: String,
    folder_name: String,
    esoui_id: u32,
) -> Result<ConflictReport, String> {
    validate_name(&folder_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let pending_clone = pending.0.clone();

    tokio::task::spawn_blocking(move || {
        let info = esoui::fetch_addon_info(esoui_id)?;
        let tmp_file = esoui::download_addon(&info.download_url, None)?;

        let (_, kept_path) = tmp_file
            .keep()
            .map_err(|e| format!("Failed to persist temp ZIP: {e}"))?;

        let session_id = generate_session_id(&folder_name);

        // Keep the ZIP hash map: the apply step (update_addon_with_decisions)
        // reuses it as the new baseline instead of re-decompressing and
        // re-hashing the whole archive a second time — the big saving on
        // many-file addons.
        let (report, zip_hashes) = build_conflict_report(
            &addons_dir,
            &folder_name,
            &kept_path,
            &info.version,
            &session_id,
        )?;

        if let Ok(mut map) = pending_clone.lock() {
            map.insert(
                session_id.clone(),
                crate::PendingUpdate {
                    zip_path: kept_path,
                    folder_name: folder_name.clone(),
                    esoui_id,
                    update_version: info.version,
                    zip_hashes: Arc::new(zip_hashes),
                },
            );
        }

        Ok(report)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchConflictEntry {
    pub esoui_id: u32,
    pub folder_name: String,
    pub api_version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchConflictAddon {
    pub session_id: String,
    pub folder_name: String,
    pub update_version: String,
    pub conflicts: Vec<FileConflict>,
    pub auto_kept_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoConflictAddon {
    pub session_id: String,
    pub folder_name: String,
    pub update_version: String,
    pub auto_kept_files: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchConflictResult {
    pub no_conflict_addons: Vec<NoConflictAddon>,
    pub conflicting_addons: Vec<BatchConflictAddon>,
    pub failed: Vec<String>,
    pub errors: HashMap<String, String>,
}

#[tauri::command]
pub async fn scan_batch_conflicts(
    state: tauri::State<'_, AllowedAddonsPath>,
    pending: tauri::State<'_, crate::PendingUpdates>,
    app: tauri::AppHandle,
    addons_path: String,
    updates: Vec<BatchConflictEntry>,
) -> Result<BatchConflictResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let pending_clone = pending.0.clone();

    tokio::task::spawn_blocking(move || {
        let total = updates.len();

        for (i, entry) in updates.iter().enumerate() {
            let _ = app.emit(
                "batch-update-progress",
                BatchUpdateProgress {
                    folder_name: entry.folder_name.clone(),
                    phase: "downloading".to_string(),
                    index: i,
                    total,
                },
            );
        }

        // Phase 1: parallel download
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .map_err(|e| format!("Thread pool error: {e}"))?;

        struct Downloaded {
            kept_path: PathBuf,
            esoui_id: u32,
            api_version: String,
        }

        let app_clone = app.clone();
        let download_results: Vec<(String, usize, Result<Downloaded, String>)> =
            pool.install(|| {
                updates
                    .par_iter()
                    .enumerate()
                    .map(|(i, entry)| {
                        let result = fetch_and_download_with_retry(entry.esoui_id).and_then(
                            |(tmp, _info)| {
                                let _ = app_clone.emit(
                                    "batch-update-progress",
                                    BatchUpdateProgress {
                                        folder_name: entry.folder_name.clone(),
                                        phase: "scanning".to_string(),
                                        index: i,
                                        total,
                                    },
                                );
                                let (_, kept_path) = tmp
                                    .keep()
                                    .map_err(|e| format!("Failed to persist temp ZIP: {e}"))?;
                                Ok(Downloaded {
                                    kept_path,
                                    esoui_id: entry.esoui_id,
                                    api_version: entry.api_version.clone(),
                                })
                            },
                        );
                        if result.is_err() {
                            let _ = app_clone.emit(
                                "batch-update-progress",
                                BatchUpdateProgress {
                                    folder_name: entry.folder_name.clone(),
                                    phase: "failed".to_string(),
                                    index: i,
                                    total,
                                },
                            );
                        }
                        (entry.folder_name.clone(), i, result)
                    })
                    .collect()
            });

        // Phase 2: conflict analysis
        let mut no_conflict_addons = Vec::new();
        let mut conflicting_addons = Vec::new();
        let mut failed = Vec::new();
        let mut errors: HashMap<String, String> = HashMap::new();

        for (folder_name, _index, result) in download_results {
            match result {
                Err(e) => {
                    errors.insert(folder_name.clone(), e);
                    failed.push(folder_name);
                }
                Ok(dl) => {
                    let session_id = generate_session_id(&folder_name);
                    let version = &dl.api_version;

                    match build_conflict_report(
                        &addons_dir,
                        &folder_name,
                        &dl.kept_path,
                        version,
                        &session_id,
                    ) {
                        Err(e) => {
                            let _ = std::fs::remove_file(&dl.kept_path);
                            errors.insert(folder_name.clone(), e);
                            failed.push(folder_name);
                        }
                        // Keep the ZIP map on the pending update so the deferred
                        // interactive `update_addon_with_decisions` reuses it as
                        // the baseline instead of re-hashing the archive.
                        Ok((report, zip_hashes)) => {
                            if let Ok(mut map) = pending_clone.lock() {
                                map.insert(
                                    session_id.clone(),
                                    crate::PendingUpdate {
                                        zip_path: dl.kept_path,
                                        folder_name: folder_name.clone(),
                                        esoui_id: dl.esoui_id,
                                        update_version: version.to_string(),
                                        zip_hashes: Arc::new(zip_hashes),
                                    },
                                );
                            }

                            if report.conflicts.is_empty() {
                                no_conflict_addons.push(NoConflictAddon {
                                    session_id: report.session_id,
                                    folder_name: report.folder_name,
                                    update_version: report.update_version,
                                    auto_kept_files: report.auto_kept_files,
                                });
                            } else {
                                conflicting_addons.push(BatchConflictAddon {
                                    session_id: report.session_id,
                                    folder_name: report.folder_name,
                                    update_version: report.update_version,
                                    conflicts: report.conflicts,
                                    auto_kept_files: report.auto_kept_files,
                                });
                            }
                        }
                    }
                }
            }
        }

        Ok(BatchConflictResult {
            no_conflict_addons,
            conflicting_addons,
            failed,
            errors,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// ── Streaming batch update (download→extract overlap, single metadata write) ──

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingBatchResult {
    /// Addons that were extracted and recorded in this call.
    pub completed: Vec<String>,
    /// Addons that failed to download or extract.
    pub failed: Vec<String>,
    /// Per-addon raw error strings (carry the CFA substring for the UI to group).
    pub errors: HashMap<String, String>,
    /// Conflicting addons left for the interactive "ask" flow (zips kept pending).
    pub conflicts: Vec<BatchConflictAddon>,
    /// Transitive deps auto-installed once for the whole batch.
    pub installed_deps: Vec<String>,
    pub failed_deps: Vec<String>,
    pub skipped_deps: Vec<String>,
}

/// One downloaded addon handed from a download worker to the extractor. The
/// folder name and index travel alongside on the channel, so they aren't
/// duplicated here.
struct StreamedDownload {
    zip: NamedTempFile,
    info: EsouiAddonInfo,
    esoui_id: u32,
    api_version: String,
}

/// Update all addons in a single IPC call with a streaming pipeline:
/// downloads run in parallel and each completed download is extracted as soon as
/// it arrives, while the rest are still downloading. The metadata store is
/// loaded once, mutated in memory across every addon, dependency-resolved once
/// over the union of installed folders, and saved once — eliminating the N×
/// load/save/dep-resolve churn of the per-addon path.
///
/// `conflict_policy` controls how user-modified files are handled:
/// - `"keep_mine"` / `"take_update"`: conflicts are auto-resolved inline.
/// - `"ask"` (or anything else): conflicting addons are NOT extracted; their
///   zips are kept in `PendingUpdates` and returned for the interactive modal,
///   exactly like `scan_batch_conflicts` + `update_addon_with_decisions`.
#[tauri::command]
pub async fn update_batch_with_decisions(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    pending: tauri::State<'_, crate::PendingUpdates>,
    app: tauri::AppHandle,
    addons_path: String,
    updates: Vec<BatchUpdateEntry>,
    conflict_policy: String,
) -> Result<StreamingBatchResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let lock = meta_lock.0.clone();
    let pending = pending.0.clone();

    tokio::task::spawn_blocking(move || {
        let t_start = std::time::Instant::now();
        let total = updates.len();

        // Emit "downloading" for every addon up front so the UI shows progress
        // immediately, before the first byte lands.
        for (i, entry) in updates.iter().enumerate() {
            let _ = app.emit(
                "batch-update-progress",
                BatchUpdateProgress {
                    folder_name: entry.folder_name.clone(),
                    phase: "downloading".to_string(),
                    index: i,
                    total,
                },
            );
        }

        let auto_resolve = conflict_policy == "keep_mine" || conflict_policy == "take_update";

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(download_thread_count(total))
            .build()
            .map_err(|e| format!("Thread pool error: {e}"))?;

        // Channel: download workers (producers) → extractor (single consumer).
        // The extractor owns the metadata lock, so writes never race. The folder
        // name rides alongside so the extractor can report errors even when the
        // download itself failed (no StreamedDownload to read it from).
        let (tx, rx) =
            std::sync::mpsc::channel::<(usize, String, Result<StreamedDownload, String>)>();

        // Hold the metadata lock for the whole extract phase. Acquire it before
        // spawning downloads so a concurrent per-addon op can't interleave.
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;

        // Run the parallel downloads on a dedicated OS thread, NOT inside the
        // rayon pool's scope on this thread. The consumer below blocks on
        // `rx.recv()`, which a rayon worker cannot reclaim — if the consumer ran
        // on a pool worker it would occupy a slot, and for a single-addon batch
        // (`num_threads(1)`) it would deadlock outright (the lone worker blocked
        // on the channel, the download task never scheduled). Driving downloads
        // from a separate thread keeps all N pool threads free for downloading
        // and the consumer free to drain the channel.
        let app_dl = app.clone();
        let producer = std::thread::spawn(move || {
            pool.install(move || {
                updates
                    .par_iter()
                    .enumerate()
                    .for_each_with(tx, |tx, (i, entry)| {
                        let result =
                            fetch_and_download_with_retry(entry.esoui_id).map(|(zip, info)| {
                                StreamedDownload {
                                    zip,
                                    info,
                                    esoui_id: entry.esoui_id,
                                    api_version: entry.api_version.clone(),
                                }
                            });
                        let phase = if result.is_ok() {
                            "extracting"
                        } else {
                            "failed"
                        };
                        let _ = app_dl.emit(
                            "batch-update-progress",
                            BatchUpdateProgress {
                                folder_name: entry.folder_name.clone(),
                                phase: phase.to_string(),
                                index: i,
                                total,
                            },
                        );
                        let _ = tx.send((i, entry.folder_name.clone(), result));
                    });
                // tx dropped here → the consumer's rx loop ends once drained.
            });
        });

        // Consumer: extract each download as it arrives, on THIS thread, holding
        // the metadata lock. `rx.iter()` blocks until a download is ready and
        // ends when the producer drops its sender.
        let extract_outcome = extract_streamed_downloads(
            &addons_dir,
            &app,
            total,
            auto_resolve,
            &conflict_policy,
            &pending,
            rx,
        );

        // The producer has finished sending by the time the channel drained;
        // join so the thread isn't detached. A producer panic is intentionally
        // swallowed — fetch_and_download_with_retry returns Result rather than
        // panicking, so a panic here would be a bug, and the consumer has
        // already produced a complete result from whatever it received.
        let _ = producer.join();

        let elapsed = t_start.elapsed();
        eprintln!(
            "[batch-update] {} addons: {} completed, {} failed, {} conflicts in {:.2}s",
            total,
            extract_outcome.completed.len(),
            extract_outcome.failed.len(),
            extract_outcome.conflicts.len(),
            elapsed.as_secs_f64(),
        );

        Ok(extract_outcome)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Pick a download parallelism that scales with the batch but stays polite to
/// ESOUI. Small batches don't need 4 threads; large ones benefit from a few
/// more. Capped at 6 to avoid hammering the server.
fn download_thread_count(addon_count: usize) -> usize {
    addon_count.clamp(1, 6)
}

/// Consumer side of the streaming pipeline. Holds the metadata store in memory,
/// extracts each download as it arrives, resolves all transitive deps once at
/// the end, and saves the store once.
fn extract_streamed_downloads(
    addons_dir: &Path,
    app: &tauri::AppHandle,
    total: usize,
    auto_resolve: bool,
    conflict_policy: &str,
    pending: &Arc<Mutex<HashMap<String, crate::PendingUpdate>>>,
    rx: std::sync::mpsc::Receiver<(usize, String, Result<StreamedDownload, String>)>,
) -> StreamingBatchResult {
    let mut store = metadata::load_metadata(addons_dir);
    let mut completed: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut errors: HashMap<String, String> = HashMap::new();
    let mut conflicts: Vec<BatchConflictAddon> = Vec::new();
    // Union of all folders extracted this batch — dep-resolved once at the end.
    let mut all_installed_folders: Vec<String> = Vec::new();

    let emit_phase = |folder: &str, phase: &str, index: usize| {
        let _ = app.emit(
            "batch-update-progress",
            BatchUpdateProgress {
                folder_name: folder.to_string(),
                phase: phase.to_string(),
                index,
                total,
            },
        );
    };

    for (index, folder_name, result) in rx.iter() {
        let dl = match result {
            Ok(dl) => dl,
            Err(e) => {
                // "failed" already emitted by the download worker.
                errors.insert(folder_name.clone(), e);
                failed.push(folder_name);
                continue;
            }
        };

        let session_id = generate_session_id(&folder_name);
        // Keep the ZIP hash map: this addon is extracted right below, so the map
        // doubles as the new hash baseline (no second decompression to re-hash).
        let (report, zip_hashes) = match build_conflict_report(
            addons_dir,
            &folder_name,
            dl.zip.path(),
            &dl.api_version,
            &session_id,
        ) {
            Ok(r) => r,
            Err(e) => {
                emit_phase(&folder_name, "failed", index);
                errors.insert(folder_name.clone(), e);
                failed.push(folder_name);
                continue;
            }
        };

        let has_conflicts = !report.conflicts.is_empty();

        // "ask" policy with real conflicts → defer to the interactive modal.
        // Persist the zip so the frontend can diff and resolve it later.
        if has_conflicts && !auto_resolve {
            let (_, kept_path) = match dl.zip.keep() {
                Ok(kept) => kept,
                Err(e) => {
                    emit_phase(&folder_name, "failed", index);
                    errors.insert(
                        folder_name.clone(),
                        format!("Failed to persist temp ZIP: {e}"),
                    );
                    failed.push(folder_name);
                    continue;
                }
            };
            if let Ok(mut map) = pending.lock() {
                map.insert(
                    session_id.clone(),
                    crate::PendingUpdate {
                        zip_path: kept_path,
                        folder_name: folder_name.clone(),
                        esoui_id: dl.esoui_id,
                        update_version: dl.api_version.clone(),
                        zip_hashes: Arc::new(zip_hashes),
                    },
                );
            }
            conflicts.push(BatchConflictAddon {
                session_id: report.session_id,
                folder_name: report.folder_name,
                update_version: report.update_version,
                conflicts: report.conflicts,
                auto_kept_files: report.auto_kept_files,
            });
            // No progress emit: the addon is "pending review", not done/failed.
            continue;
        }

        // Build the keep/skip set. Always honor auto-kept files (user edits we
        // never overwrite). Under "keep_mine" also skip the conflicting files;
        // under "take_update" let them be overwritten (the new bytes win).
        let mut kept_files: Vec<String> = report.auto_kept_files.clone();
        if has_conflicts && conflict_policy == "keep_mine" {
            kept_files.extend(report.conflicts.iter().map(|c| c.relative_path.clone()));
        }

        let skip_files: HashSet<String> = kept_files
            .iter()
            .map(|p| format!("{folder_name}/{p}"))
            .collect();

        let extract_result = if skip_files.is_empty() {
            installer::extract_addon_zip(dl.zip.path(), addons_dir)
        } else {
            installer::extract_addon_zip_selective(dl.zip.path(), addons_dir, &skip_files)
        };

        let installed_folders = match extract_result {
            Ok(folders) => folders,
            Err(e) => {
                emit_phase(&folder_name, "failed", index);
                errors.insert(folder_name.clone(), e);
                failed.push(folder_name);
                continue;
            }
        };

        // For kept files, store the upstream hash as the new baseline so the
        // user's edit stays detectable on the next update. The ZIP hashes were
        // already computed for the conflict report above — reuse them here
        // instead of decompressing the ZIP a second time.
        let hash_overrides: Option<HashMap<String, String>> = if kept_files.is_empty() {
            None
        } else {
            let overrides: HashMap<String, String> = kept_files
                .iter()
                .filter_map(|p| zip_hashes.get(p).map(|h| (p.clone(), h.clone())))
                .collect();
            (!overrides.is_empty()).then_some(overrides)
        };

        // Record the baseline straight from the ZIP hash map (plus a disk pass
        // over only the files the ZIP didn't provide), rather than re-hashing
        // the whole freshly extracted folder. If the baseline can't be recorded,
        // don't write metadata for this addon: a folder tracked in metadata but
        // missing its hash baseline would let the next update silently overwrite
        // user edits. Surface it as a failed addon instead.
        if let Err(e) = file_hashes::record_hashes_with_zip_baseline(
            addons_dir,
            dl.zip.path(),
            &installed_folders,
            &folder_name,
            &zip_hashes,
            dl.esoui_id,
            &dl.api_version,
            hash_overrides.as_ref(),
        ) {
            emit_phase(&folder_name, "failed", index);
            errors.insert(folder_name.clone(), e);
            failed.push(folder_name);
            continue;
        }

        // Drop stale metadata entries for this esoui_id whose folders were
        // renamed/removed by the new release.
        let old_folders: Vec<String> = store
            .addons
            .iter()
            .filter(|(_, m)| m.esoui_id == dl.esoui_id)
            .map(|(name, _)| name.clone())
            .collect();
        for old in &old_folders {
            if !installed_folders.contains(old) {
                metadata::remove_entry(&mut store, old);
            }
        }

        record_installed_folders(
            &mut store,
            addons_dir,
            &installed_folders,
            dl.esoui_id,
            &dl.api_version,
            &dl.info.title,
            &dl.info.download_url,
            0,
        );

        for f in &installed_folders {
            if !all_installed_folders.contains(f) {
                all_installed_folders.push(f.clone());
            }
        }

        emit_phase(&folder_name, "completed", index);
        completed.push(folder_name);
    }

    // Resolve every dependency once over the union of installed folders. The
    // resolver dedups via its own seen/all_installed sets, so a single pass over
    // the whole batch produces the same end state as N per-addon passes.
    let resolved = if all_installed_folders.is_empty() {
        ResolvedDeps {
            installed_deps: Vec::new(),
            failed_deps: Vec::new(),
            skipped_deps: Vec::new(),
        }
    } else {
        resolve_transitive_deps(addons_dir, &all_installed_folders, &mut store)
    };

    if let Err(e) = metadata::save_metadata(addons_dir, &store) {
        // The files were extracted, but kalpa.json didn't persist — so Kalpa
        // can't track these versions/hashes (next update would misbehave).
        // Don't report success silently: move every "completed" addon into
        // "failed" with the save error so the UI surfaces it. The frontend only
        // looks up errors for names in `failed`, so the reason must be keyed by
        // each affected folder.
        let reason = format!("Update applied but could not be saved to kalpa.json: {e}");
        eprintln!("[batch-update] metadata save failed: {e}");
        for folder in completed.drain(..) {
            errors.insert(folder.clone(), reason.clone());
            failed.push(folder);
        }
    }

    StreamingBatchResult {
        completed,
        failed,
        errors,
        conflicts,
        installed_deps: resolved.installed_deps,
        failed_deps: resolved.failed_deps,
        skipped_deps: resolved.skipped_deps,
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffData {
    pub user_content: String,
    pub upstream_content: String,
    pub is_binary: bool,
}

#[tauri::command]
pub async fn get_conflict_diff(
    state: tauri::State<'_, AllowedAddonsPath>,
    pending: tauri::State<'_, crate::PendingUpdates>,
    addons_path: String,
    session_id: String,
    relative_path: String,
) -> Result<DiffData, String> {
    validate_relative_path(&relative_path)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let pending_clone = pending.0.clone();

    tokio::task::spawn_blocking(move || {
        let (zip_path, folder_name) = {
            let map = pending_clone
                .lock()
                .map_err(|_| "Failed to access pending updates".to_string())?;
            let pu = map
                .get(&session_id)
                .ok_or_else(|| format!("Session {session_id} not found"))?;
            (pu.zip_path.clone(), pu.folder_name.clone())
        };

        // Read user's file from disk
        let user_file_path = addons_dir
            .join(&folder_name)
            .join(relative_path.replace('/', "\\"));
        let user_content = if user_file_path.exists() {
            let bytes =
                fs::read(&user_file_path).map_err(|e| format!("Failed to read user file: {e}"))?;
            if bytes.iter().take(512).any(|&b| b == 0) {
                return Ok(DiffData {
                    user_content: String::new(),
                    upstream_content: String::new(),
                    is_binary: true,
                });
            }
            String::from_utf8_lossy(&bytes).to_string()
        } else {
            String::new()
        };

        // Read upstream file from ZIP
        let file = fs::File::open(&zip_path).map_err(|e| format!("Failed to open ZIP: {e}"))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| format!("Failed to read ZIP: {e}"))?;
        let zip_entry_name = format!("{folder_name}/{relative_path}");
        let mut entry = archive
            .by_name(&zip_entry_name)
            .map_err(|e| format!("File not found in ZIP: {e}"))?;

        const MAX_DIFF_SIZE: u64 = 5 * 1024 * 1024; // 5 MB
        if entry.size() > MAX_DIFF_SIZE {
            return Err("File too large for diff view (exceeds 5 MB limit).".to_string());
        }

        let mut upstream_bytes = Vec::with_capacity(entry.size().min(MAX_DIFF_SIZE) as usize);
        std::io::Read::read_to_end(&mut entry, &mut upstream_bytes)
            .map_err(|e| format!("Failed to read ZIP entry: {e}"))?;

        if upstream_bytes.iter().take(512).any(|&b| b == 0) {
            return Ok(DiffData {
                user_content: String::new(),
                upstream_content: String::new(),
                is_binary: true,
            });
        }

        let upstream_content = String::from_utf8_lossy(&upstream_bytes).to_string();

        Ok(DiffData {
            user_content,
            upstream_content,
            is_binary: false,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDecision {
    pub relative_path: String,
    pub action: String, // "keep_mine" | "take_update"
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn update_addon_with_decisions(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    pending: tauri::State<'_, crate::PendingUpdates>,
    cancels: tauri::State<'_, crate::UpdateCancels>,
    app: tauri::AppHandle,
    addons_path: String,
    session_id: String,
    decisions: Vec<FileDecision>,
    operation_id: Option<String>,
) -> Result<InstallResult, String> {
    for d in &decisions {
        validate_relative_path(&d.relative_path)?;
    }
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let pending_clone = pending.0.clone();
    let lock = meta_lock.0.clone();
    let registry = cancels.0.clone();
    let operation_id = operation_id.unwrap_or_default();
    let cancel = CancelGuard::register(registry, operation_id.clone());

    tokio::task::spawn_blocking(move || {
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;
        let pu = {
            let map = pending_clone
                .lock()
                .map_err(|_| "Failed to access pending updates".to_string())?;
            map.get(&session_id)
                .ok_or_else(|| format!("Session {session_id} not found"))?
                .clone()
        };

        // Run the fallible work in a helper so that, once the session is claimed,
        // the pending entry and kept temp ZIP are cleaned up whether that work
        // succeeds OR fails. The temp ZIP was `.keep()`-ed (no auto-delete), so an
        // early `?` return inside the helper (backup, hashing, extraction,
        // recording, metadata save) would otherwise orphan it and leave a stale
        // pending entry that blocks re-resolving the conflict.
        //
        // The only exits before this point are the lock-poisoned and
        // session-not-found errors above, where there is nothing to clean up: no
        // `pu` was obtained and the session either never existed or wasn't ours to
        // remove. Validation failures before `spawn_blocking` likewise leave the
        // session intact on purpose, so the user can retry or cancel it.
        //
        // Build a progress emitter for the file-bound extraction. The
        // cancellation flag was registered before this blocking task started, so
        // a Stop request made while waiting on the metadata lock is still
        // observed before extraction writes files.
        let progress = make_progress_emitter(app, operation_id, pu.folder_name.clone());
        let hooks = installer::ExtractHooks {
            cancel: Some(cancel.flag()),
            progress: Some(&progress),
        };
        let outcome = update_with_decisions_inner(&addons_dir, &pu, &decisions, hooks);

        // Delete the kept temp ZIP first, then drop the pending entry. If the
        // delete genuinely fails (e.g. another process briefly holds the file),
        // keep the entry so `cancel_pending_update` can retry cleanup later. A
        // file that's already gone counts as removed.
        let removed = match fs::remove_file(&pu.zip_path) {
            Ok(()) => true,
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
        };
        if removed {
            if let Ok(mut map) = pending_clone.lock() {
                map.remove(&session_id);
            }
        }

        outcome
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Extract a pending conflict-resolved update and record its new hash baseline.
/// Separated from the command so the caller can clean up the pending session and
/// temp ZIP regardless of whether this succeeds or fails.
fn update_with_decisions_inner(
    addons_dir: &Path,
    pu: &crate::PendingUpdate,
    decisions: &[FileDecision],
    hooks: installer::ExtractHooks,
) -> Result<InstallResult, String> {
    let kept_files: Vec<String> = decisions
        .iter()
        .filter(|d| d.action == "keep_mine")
        .map(|d| d.relative_path.clone())
        .collect();

    // Collect files to skip during extraction (full ZIP path with folder prefix)
    let skip_files: HashSet<String> = kept_files
        .iter()
        .map(|p| format!("{}/{}", pu.folder_name, p))
        .collect();

    // Collect files to back up (user chose "take_update" on their edited files)
    let files_to_backup: Vec<String> = decisions
        .iter()
        .filter(|d| d.action == "take_update")
        .map(|d| d.relative_path.clone())
        .collect();

    // Get current version from hash manifest for backup metadata
    let from_version = file_hashes::load_hash_manifest(addons_dir, &pu.folder_name)
        .map(|m| m.installed_version)
        .unwrap_or_default();

    // Back up files before overwriting
    if !files_to_backup.is_empty() {
        crate::edit_backups::backup_user_files(
            addons_dir,
            &pu.folder_name,
            &files_to_backup,
            &from_version,
            &pu.update_version,
        )?;
    }

    // The ZIP's hash/signature map. Reuse the one computed during conflict
    // detection (stored on the pending update) so we don't re-decompress and
    // re-hash the whole archive again here — the dominant cost on many-file
    // addons. Fall back to hashing it if a pending entry predates that field.
    // This map becomes the new baseline after extraction (reused by
    // record_hashes_with_zip_baseline) and supplies the upstream hashes for kept
    // "keep_mine" files so the user's edit stays detectable on the next update.
    let zip_hashes = if pu.zip_hashes.is_empty() {
        Arc::new(file_hashes::hash_zip_entries(
            &pu.zip_path,
            &pu.folder_name,
        )?)
    } else {
        Arc::clone(&pu.zip_hashes)
    };
    let hash_overrides: Option<HashMap<String, String>> = if kept_files.is_empty() {
        None
    } else {
        let overrides: HashMap<String, String> = kept_files
            .iter()
            .filter_map(|p| zip_hashes.get(p).map(|h| (p.clone(), h.clone())))
            .collect();
        (!overrides.is_empty()).then_some(overrides)
    };

    // Extract with selective skipping (cancellable, progress-reporting)
    let installed_folders = if skip_files.is_empty() {
        installer::extract_addon_zip_with(&pu.zip_path, addons_dir, hooks)?
    } else {
        installer::extract_addon_zip_selective_with(&pu.zip_path, addons_dir, &skip_files, hooks)?
    };

    // Record the baseline from the ZIP hash map (plus a disk pass over only
    // the files the ZIP didn't provide), rather than re-hashing the folder.
    // Fail the update if it can't persist: saving metadata without a hash
    // baseline would let the next update silently overwrite the user's edits.
    file_hashes::record_hashes_with_zip_baseline(
        addons_dir,
        &pu.zip_path,
        &installed_folders,
        &pu.folder_name,
        &zip_hashes,
        pu.esoui_id,
        &pu.update_version,
        hash_overrides.as_ref(),
    )?;

    // Update metadata
    let mut store = metadata::load_metadata(addons_dir);
    let old_folders: Vec<String> = store
        .addons
        .iter()
        .filter(|(_, m)| m.esoui_id == pu.esoui_id)
        .map(|(name, _)| name.clone())
        .collect();
    for old in &old_folders {
        if !installed_folders.contains(old) {
            metadata::remove_entry(&mut store, old);
        }
    }
    record_installed_folders(
        &mut store,
        addons_dir,
        &installed_folders,
        pu.esoui_id,
        &pu.update_version,
        &pu.folder_name,
        "",
        0,
    );

    let resolved = resolve_transitive_deps(addons_dir, &installed_folders, &mut store);

    metadata::save_metadata(addons_dir, &store)?;

    Ok(InstallResult {
        installed_folders,
        installed_deps: resolved.installed_deps,
        failed_deps: resolved.failed_deps,
        skipped_deps: resolved.skipped_deps,
    })
}

// ── File browser commands ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonFileEntry {
    pub relative_path: String,
    pub is_directory: bool,
    pub size_bytes: u64,
    pub status: String, // "stock" | "modified" | "unknown"
    pub extension: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonFileTree {
    pub folder_name: String,
    pub files: Vec<AddonFileEntry>,
    pub modified_count: u32,
}

#[tauri::command]
pub async fn list_addon_files(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    folder_name: String,
) -> Result<AddonFileTree, String> {
    validate_name(&folder_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;

    tokio::task::spawn_blocking(move || {
        let addon_path = addons_dir.join(&folder_name);
        if !addon_path.is_dir() {
            return Err(format!("Addon folder not found: {folder_name}"));
        }

        let manifest = file_hashes::load_hash_manifest(&addons_dir, &folder_name);
        let modified_set: HashSet<String> = manifest
            .as_ref()
            .map(|m| m.modified_files.iter().cloned().collect())
            .unwrap_or_default();
        let has_manifest = manifest.is_some();

        let mut files = Vec::new();

        const MAX_WALK_DEPTH: u32 = 32;

        fn walk_files(
            base: &Path,
            current: &Path,
            files: &mut Vec<AddonFileEntry>,
            modified_set: &HashSet<String>,
            has_manifest: bool,
            depth: u32,
        ) -> Result<(), String> {
            if depth > MAX_WALK_DEPTH {
                return Err("Directory tree too deep (> 32 levels).".to_string());
            }

            let entries =
                fs::read_dir(current).map_err(|e| format!("Failed to read directory: {e}"))?;

            for entry in entries {
                let entry = entry.map_err(|e| format!("Dir entry error: {e}"))?;
                let path = entry.path();

                // Skip symlinks/junctions to avoid loops
                if let Ok(meta) = path.symlink_metadata() {
                    if meta.file_type().is_symlink() {
                        continue;
                    }
                }

                let relative = path
                    .strip_prefix(base)
                    .map_err(|e| format!("Path prefix error: {e}"))?;
                let rel_str = relative
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .collect::<Vec<_>>()
                    .join("/");

                let is_dir = path.is_dir();
                let size = if is_dir {
                    0
                } else {
                    path.metadata().map(|m| m.len()).unwrap_or(0)
                };

                let extension = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();

                let status = if is_dir {
                    "stock".to_string()
                } else if !has_manifest {
                    "unknown".to_string()
                } else if modified_set.contains(&rel_str) {
                    "modified".to_string()
                } else {
                    "stock".to_string()
                };

                files.push(AddonFileEntry {
                    relative_path: rel_str,
                    is_directory: is_dir,
                    size_bytes: size,
                    status,
                    extension,
                });

                if is_dir {
                    walk_files(base, &path, files, modified_set, has_manifest, depth + 1)?;
                }
            }
            Ok(())
        }

        walk_files(
            &addon_path,
            &addon_path,
            &mut files,
            &modified_set,
            has_manifest,
            0,
        )?;

        files.sort_by(|a, b| {
            b.is_directory
                .cmp(&a.is_directory)
                .then(a.relative_path.cmp(&b.relative_path))
        });

        let modified_count = files.iter().filter(|f| f.status == "modified").count() as u32;

        Ok(AddonFileTree {
            folder_name,
            files,
            modified_count,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

fn validate_relative_path(relative_path: &str) -> Result<(), String> {
    if relative_path.contains("..") {
        return Err("Invalid path: contains '..'".to_string());
    }
    if Path::new(relative_path).is_absolute() {
        return Err("Absolute paths are not allowed.".to_string());
    }
    if relative_path.starts_with('/') || relative_path.starts_with('\\') {
        return Err("Path must be relative.".to_string());
    }
    // Reject Windows-forbidden characters (same set as `validate_name`) in any
    // path segment. Without this, ':' slips through and enables NTFS
    // alternate-data-stream syntax ("file.lua:stream") or a drive-relative path
    // ("C:foo"), neither of which is caught by the checks above.
    let forbidden: &[char] = &['<', '>', ':', '"', '|', '?', '*'];
    for segment in relative_path.split(['/', '\\']) {
        if segment.contains(forbidden) {
            return Err(
                "Path contains a forbidden character (< > : \" | ? * are not allowed).".to_string(),
            );
        }
    }
    Ok(())
}

fn resolve_addon_file_path(
    addons_dir: &Path,
    folder_name: &str,
    relative_path: &str,
) -> Result<PathBuf, String> {
    let file_path = addons_dir
        .join(folder_name)
        .join(relative_path.replace('/', "\\"));

    let canonical_addons = addons_dir
        .canonicalize()
        .map_err(|e| format!("Failed to resolve addons path: {e}"))?;
    let canonical_file = file_path
        .canonicalize()
        .map_err(|e| format!("Failed to resolve file path: {e}"))?;

    if !canonical_file.starts_with(&canonical_addons) {
        return Err("File path is outside the AddOns directory.".to_string());
    }

    Ok(file_path)
}

#[tauri::command]
pub async fn read_addon_file(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    folder_name: String,
    relative_path: String,
) -> Result<String, String> {
    validate_name(&folder_name)?;
    validate_relative_path(&relative_path)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;

    tokio::task::spawn_blocking(move || {
        let file_path = resolve_addon_file_path(&addons_dir, &folder_name, &relative_path)?;

        const MAX_EDITOR_SIZE: u64 = 5 * 1024 * 1024;
        let meta = fs::metadata(&file_path).map_err(|e| format!("Failed to read file: {e}"))?;
        if meta.len() > MAX_EDITOR_SIZE {
            return Err(format!(
                "File is too large to edit ({:.1} MB). Maximum is 5 MB.",
                meta.len() as f64 / (1024.0 * 1024.0)
            ));
        }

        let bytes = fs::read(&file_path).map_err(|e| format!("Failed to read file: {e}"))?;

        if bytes.iter().take(512).any(|&b| b == 0) {
            return Err("Cannot read binary file.".to_string());
        }

        String::from_utf8(bytes).map_err(|_| "File contains invalid UTF-8.".to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn write_addon_file(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    folder_name: String,
    relative_path: String,
    content: String,
) -> Result<(), String> {
    validate_name(&folder_name)?;
    validate_relative_path(&relative_path)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;

    tokio::task::spawn_blocking(move || {
        let file_path = resolve_addon_file_path(&addons_dir, &folder_name, &relative_path)?;

        fs::write(&file_path, &content).map_err(|e| format!("Failed to write file: {e}"))?;

        // Re-hash only the file we just wrote — compute_addon_hashes would walk
        // the entire addon folder (0.7-1.5 s per Ctrl+S on large addons) just to
        // read back one entry. Compare it against the stored baseline to keep the
        // modified_files cache current.
        if let Some(mut manifest) = file_hashes::load_hash_manifest(&addons_dir, &folder_name) {
            let key = relative_path.replace('\\', "/");
            let sig = file_hashes::file_signature(&key, &file_path)?;
            let is_modified = manifest
                .files
                .get(&key)
                .map(|stored| !file_hashes::signatures_match(stored, &sig))
                .unwrap_or(true);
            if is_modified && !manifest.modified_files.contains(&key) {
                manifest.modified_files.push(key);
                manifest.modified_files.sort();
            } else if !is_modified {
                manifest.modified_files.retain(|f| f != &key);
            }
            file_hashes::save_hash_manifest(&addons_dir, &manifest)?;
        }

        Ok(())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn rescan_addon_hashes(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    folder_name: String,
) -> Result<Vec<String>, String> {
    validate_name(&folder_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;

    tokio::task::spawn_blocking(move || {
        file_hashes::detect_modifications(&addons_dir, &folder_name)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn cancel_pending_update(
    pending: tauri::State<'_, crate::PendingUpdates>,
    session_id: String,
) -> Result<(), String> {
    let pending_clone = pending.0.clone();
    tokio::task::spawn_blocking(move || {
        if let Ok(mut map) = pending_clone.lock() {
            if let Some(pu) = map.remove(&session_id) {
                let _ = fs::remove_file(&pu.zip_path);
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn list_edit_backups(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    folder_name: String,
) -> Result<Vec<crate::edit_backups::BackupManifest>, String> {
    validate_name(&folder_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;

    Ok(crate::edit_backups::list_backups(&addons_dir, &folder_name))
}

#[tauri::command]
pub async fn restore_edit_backup(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    folder_name: String,
    backed_up_at: String,
    relative_path: String,
) -> Result<(), String> {
    validate_name(&folder_name)?;
    validate_relative_path(&relative_path)?;
    if backed_up_at.contains("..") || backed_up_at.contains('/') || backed_up_at.contains('\\') {
        return Err("Invalid backup timestamp.".to_string());
    }
    let addons_dir = require_allowed_path(&state, &addons_path)?;

    tokio::task::spawn_blocking(move || {
        crate::edit_backups::restore_backup_file(
            &addons_dir,
            &folder_name,
            &backed_up_at,
            &relative_path,
        )
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportEntry {
    pub esoui_id: u32,
    pub folder_name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportData {
    pub version: u32,
    pub addons: Vec<ExportEntry>,
}

#[tauri::command]
pub fn export_addon_list(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<String, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let store = metadata::load_metadata(&addons_dir);

    let mut entries: Vec<ExportEntry> = store
        .addons
        .iter()
        .filter(|(folder, _)| addons_dir.join(folder).is_dir())
        .map(|(folder, meta)| ExportEntry {
            esoui_id: meta.esoui_id,
            folder_name: folder.clone(),
            version: meta.installed_version.clone(),
        })
        .collect();

    entries.sort_by(|a, b| a.folder_name.cmp(&b.folder_name));

    // Deduplicate by esoui_id (multiple folders can share an ID),
    // but keep all untracked entries (esoui_id == 0)
    let mut seen_ids: HashSet<u32> = HashSet::new();
    entries.retain(|e| e.esoui_id == 0 || seen_ids.insert(e.esoui_id));

    let export = ExportData {
        version: 1,
        addons: entries,
    };

    serde_json::to_string_pretty(&export).map_err(|e| format!("Failed to export: {e}"))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub installed: Vec<String>,
    pub failed: Vec<String>,
    pub skipped: Vec<String>,
    /// Per-folder error reasons for entries in `failed`. Omitted from JSON when empty.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub errors: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoLinkResult {
    pub linked: Vec<String>,
    pub not_found: Vec<String>,
}

/// Try to auto-link untracked addons to their ESOUI IDs by searching ESOUI.
#[tauri::command]
pub async fn auto_link_addons(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    addons_path: String,
) -> Result<AutoLinkResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let lock = meta_lock.0.clone();
    tokio::task::spawn_blocking(move || {
        // Fetch filelist outside the lock (network I/O)
        let api_lookup = esoui::fetch_filelist_lookup()?;
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;
        auto_link_addons_blocking(&addons_dir, &api_lookup)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

fn auto_link_addons_blocking(
    addons_dir: &Path,
    api_lookup: &HashMap<String, Arc<esoui::ApiAddonLookup>>,
) -> Result<AutoLinkResult, String> {
    let mut store = metadata::load_metadata(addons_dir);

    let entries =
        fs::read_dir(addons_dir).map_err(|e| format!("Failed to read AddOns folder: {e}"))?;

    let mut linked: Vec<String> = Vec::new();
    let mut not_found: Vec<String> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let folder_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };

        // Must have a manifest to be a real addon
        if find_manifest(addons_dir, &folder_name).is_none() {
            continue;
        }

        // Look up this folder name in the API data
        if let Some(api_entry) = api_lookup.get(&folder_name) {
            let already_tracked = store.addons.get(&folder_name);

            // Skip bundled secondary folders: if esouiId is 0 and another
            // addon in the store installed this folder (shares download_url),
            // don't auto-link it to its own ESOUI entry — that would cause
            // version mismatches since the bundled version differs from the
            // standalone version.
            let is_bundled_secondary = already_tracked.is_some_and(|m| {
                m.esoui_id == 0
                    && store
                        .addons
                        .values()
                        .any(|other| other.esoui_id != 0 && other.download_url == m.download_url)
            });
            if is_bundled_secondary {
                continue;
            }

            let needs_update = match already_tracked {
                Some(meta) => {
                    // Update existing entries: fill in missing esoui_id or last_update
                    (meta.esoui_id == 0 && api_entry.esoui_id > 0)
                        || meta.esoui_last_update == 0
                        || meta.esoui_last_update != api_entry.last_update
                }
                None => true,
            };
            if needs_update {
                // Reconcile ESOUI metadata in place. This is NOT a download, so
                // `installed_at` (the local "last downloaded" time) and tags must
                // be preserved — only API-derived fields change. A brand-new,
                // auto-linked entry was present on disk but never downloaded by
                // Kalpa, so its `installed_at` is left empty (unknown).
                let entry = store.addons.entry(folder_name.clone()).or_insert_with(|| {
                    metadata::AddonMetadata {
                        esoui_id: 0,
                        installed_version: read_local_version(addons_dir, &folder_name),
                        download_url: api_entry.file_info_uri.clone(),
                        installed_at: String::new(),
                        tags: Vec::new(),
                        esoui_last_update: 0,
                    }
                });
                metadata::reconcile_addon(
                    entry,
                    api_entry.esoui_id,
                    api_entry.last_update,
                    &api_entry.file_info_uri,
                );
                linked.push(folder_name);
            }
        } else if !store.addons.contains_key(&folder_name) {
            not_found.push(folder_name);
        }
    }

    metadata::save_metadata(addons_dir, &store)?;

    Ok(AutoLinkResult { linked, not_found })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchRemoveResult {
    pub removed: Vec<String>,
    pub failed: Vec<String>,
    pub errors: HashMap<String, String>,
}

/// Batch remove multiple addons.
#[tauri::command]
pub async fn batch_remove_addons(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    addons_path: String,
    folder_names: Vec<String>,
) -> Result<BatchRemoveResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    for name in &folder_names {
        validate_name(name)?;
    }
    let lock = meta_lock.0.clone();
    tokio::task::spawn_blocking(move || {
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;

        let mut store = metadata::load_metadata(&addons_dir);
        let mut removed: Vec<String> = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        let mut errors: HashMap<String, String> = HashMap::new();

        for name in &folder_names {
            match installer::remove_addon(&addons_dir, name) {
                Ok(()) => {
                    metadata::remove_entry(&mut store, name);
                    removed.push(name.clone());
                }
                Err(e) => {
                    errors.insert(name.clone(), e);
                    failed.push(name.clone());
                }
            }
        }

        metadata::save_metadata(&addons_dir, &store)?;
        Ok(BatchRemoveResult {
            removed,
            failed,
            errors,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchTagEntry {
    pub folder_name: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchTagResult {
    pub updated: Vec<String>,
    pub failed: Vec<String>,
    pub errors: HashMap<String, String>,
}

/// Set tags on multiple addons in one pass: a single metadata load, one loop,
/// one save. Mirrors batch_remove_addons so the whole batch takes the metadata
/// lock once instead of N times (one set_addon_tags call each).
#[tauri::command]
pub async fn batch_set_tags(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    addons_path: String,
    entries: Vec<BatchTagEntry>,
) -> Result<BatchTagResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    for entry in &entries {
        validate_name(&entry.folder_name)?;
    }
    let lock = meta_lock.0.clone();
    tokio::task::spawn_blocking(move || {
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;

        let mut store = metadata::load_metadata(&addons_dir);
        let mut updated: Vec<String> = Vec::new();

        for entry in entries {
            match store.addons.get_mut(&entry.folder_name) {
                Some(meta) => meta.tags = entry.tags,
                None => {
                    // Create a minimal entry for untracked addons so tags persist,
                    // matching set_addon_tags.
                    store.addons.insert(
                        entry.folder_name.clone(),
                        metadata::AddonMetadata {
                            esoui_id: 0,
                            installed_version: String::new(),
                            download_url: String::new(),
                            installed_at: String::new(),
                            tags: entry.tags,
                            esoui_last_update: 0,
                        },
                    );
                }
            }
            updated.push(entry.folder_name);
        }

        metadata::save_metadata(&addons_dir, &store)?;
        Ok(BatchTagResult {
            updated,
            failed: Vec::new(),
            errors: HashMap::new(),
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchEnableEntry {
    pub folder_name: String,
    /// Target state: true = enable (rename .disabled -> base), false = disable.
    pub enable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchEnableResult {
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
    pub failed: Vec<String>,
    pub errors: HashMap<String, String>,
}

/// Enable or disable multiple addons in one pass. Enable/disable is a folder
/// rename (no metadata), but the whole batch takes the metadata lock once to
/// serialize with other addon operations, mirroring batch_remove_addons.
#[tauri::command]
pub async fn batch_set_enabled(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    addons_path: String,
    entries: Vec<BatchEnableEntry>,
) -> Result<BatchEnableResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    for entry in &entries {
        validate_name(&entry.folder_name)?;
    }
    let lock = meta_lock.0.clone();
    tokio::task::spawn_blocking(move || {
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;

        let mut enabled: Vec<String> = Vec::new();
        let mut disabled: Vec<String> = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        let mut errors: HashMap<String, String> = HashMap::new();

        for entry in &entries {
            let folder_name = &entry.folder_name;
            let result = if entry.enable {
                // Rename Foo.disabled -> Foo (matches enable_addon).
                let src = addons_dir.join(format!("{folder_name}.disabled"));
                let dst = addons_dir.join(folder_name);
                if !src.is_dir() {
                    Err(format!("Disabled addon folder not found: {folder_name}"))
                } else if dst.exists() {
                    Err(format!("A folder named {folder_name} already exists."))
                } else {
                    fs::rename(&src, &dst)
                        .map_err(|e| format!("Failed to enable {folder_name}: {e}"))
                }
            } else {
                // Rename Foo -> Foo.disabled (matches disable_addon).
                let src = addons_dir.join(folder_name);
                let dst = addons_dir.join(format!("{folder_name}.disabled"));
                if !src.is_dir() {
                    Err(format!("Addon folder not found: {folder_name}"))
                } else if dst.exists() {
                    Err(format!("{folder_name} is already disabled."))
                } else {
                    fs::rename(&src, &dst)
                        .map_err(|e| format!("Failed to disable {folder_name}: {e}"))
                }
            };

            match result {
                Ok(()) => {
                    if entry.enable {
                        enabled.push(folder_name.clone());
                    } else {
                        disabled.push(folder_name.clone());
                    }
                }
                Err(e) => {
                    errors.insert(folder_name.clone(), e);
                    failed.push(folder_name.clone());
                }
            }
        }

        Ok(BatchEnableResult {
            enabled,
            disabled,
            failed,
            errors,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn import_addon_list(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    addons_path: String,
    json_data: String,
) -> Result<ImportResult, String> {
    let export: ExportData =
        serde_json::from_str(&json_data).map_err(|e| format!("Invalid export file: {e}"))?;

    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let lock = meta_lock.0.clone();

    tokio::task::spawn_blocking(move || {
        // Split into already-installed (skip) and to-install before any lock
        let (to_skip, to_install): (Vec<_>, Vec<_>) = export
            .addons
            .iter()
            .partition(|e| addons_dir.join(&e.folder_name).is_dir());

        let skipped: Vec<String> = to_skip.iter().map(|e| e.folder_name.clone()).collect();

        // Phase 1 (parallel): download ZIPs outside the lock — no metadata access
        let download_results = import_download_addons(&to_install)?;

        // Phase 2: acquire lock only for extract + metadata update
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;
        import_extract_and_record(&addons_dir, download_results, skipped)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Return true if the error string indicates an HTTP 429 rate-limit response.
/// Matches both `fetch_addon_info` ("Too many requests...") and `download_addon`
/// ("Download failed (HTTP 429)...") error formats.
fn is_rate_limited(err: &str) -> bool {
    err.contains("Too many requests") || err.contains("HTTP 429")
}

/// Global counter used to spread jitter across parallel retry workers.
/// Each thread gets a different offset so retries don't cluster even when
/// `SystemTime` resolves to the same nanosecond.
static RETRY_SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Fetch addon info and download its ZIP, retrying up to 3 times on HTTP 429.
/// Backoff: base delay doubles each attempt (1 s, 2 s) with up to 500 ms of
/// jitter so parallel workers don't retry in lockstep.
fn fetch_and_download_with_retry(esoui_id: u32) -> Result<(NamedTempFile, EsouiAddonInfo), String> {
    let seq = RETRY_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let mut last_err = String::new();
    for attempt in 0u32..3 {
        if attempt > 0 {
            let base_ms = 1000u64 * (1 << (attempt - 1));
            // Spread jitter using an atomic counter + clock nanos so threads
            // that started at the same instant still desynchronize.
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let jitter_ms = ((nanos.wrapping_add(seq.wrapping_mul(7919))) % 500) as u64;
            std::thread::sleep(std::time::Duration::from_millis(base_ms + jitter_ms));
        }
        let info = match esoui::fetch_addon_info(esoui_id) {
            Ok(i) => i,
            Err(e) => {
                last_err = e.clone();
                if is_rate_limited(&e) {
                    continue;
                }
                return Err(e);
            }
        };
        match esoui::download_addon(&info.download_url, None) {
            Ok(tmp) => return Ok((tmp, info)),
            Err(e) => {
                last_err = e.clone();
                if is_rate_limited(&e) {
                    continue;
                }
                return Err(e);
            }
        }
    }
    Err(last_err)
}

struct ImportDownloaded {
    tmp: NamedTempFile,
    info: EsouiAddonInfo,
    esoui_id: u32,
}

type ImportDownloadResults = Vec<(String, Result<ImportDownloaded, String>)>;

/// Phase 1 of import_addon_list: parallel downloads without the metadata lock.
fn import_download_addons(to_install: &[&ExportEntry]) -> Result<ImportDownloadResults, String> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .map_err(|e| format!("Thread pool error: {e}"))?;

    let download_results: Vec<(String, Result<ImportDownloaded, String>)> = pool.install(|| {
        to_install
            .par_iter()
            .map(|entry| {
                let result = fetch_and_download_with_retry(entry.esoui_id).map(|(tmp, info)| {
                    ImportDownloaded {
                        tmp,
                        info,
                        esoui_id: entry.esoui_id,
                    }
                });
                (entry.folder_name.clone(), result)
            })
            .collect()
    });

    Ok(download_results)
}

/// Phase 2 of import_addon_list: extract ZIPs and record metadata.
/// Must be called under the metadata lock.
fn import_extract_and_record(
    addons_dir: &Path,
    download_results: ImportDownloadResults,
    skipped: Vec<String>,
) -> Result<ImportResult, String> {
    let mut store = metadata::load_metadata(addons_dir);
    let mut installed: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut errors: HashMap<String, String> = HashMap::new();

    for (folder_name, result) in download_results {
        match result {
            Err(e) => {
                errors.insert(folder_name.clone(), e);
                failed.push(folder_name);
            }
            Ok(dl) => match installer::extract_addon_zip(dl.tmp.path(), addons_dir) {
                Err(e) => {
                    errors.insert(folder_name.clone(), e);
                    failed.push(folder_name);
                }
                Ok(folders) => {
                    // Record the modification baseline before tracking the install,
                    // so the next update can detect user edits instead of silently
                    // overwriting them. Treat a baseline failure as a failed import
                    // rather than leaving metadata pointing at a folder with no
                    // baseline (mirrors install_addon_blocking / try_install_dep).
                    if let Err(e) = file_hashes::record_hashes_for_folders(
                        addons_dir,
                        &folders,
                        dl.esoui_id,
                        &dl.info.version,
                    ) {
                        errors.insert(folder_name.clone(), e);
                        failed.push(folder_name);
                        continue;
                    }

                    record_installed_folders(
                        &mut store,
                        addons_dir,
                        &folders,
                        dl.esoui_id,
                        &dl.info.version,
                        &dl.info.title,
                        &dl.info.download_url,
                        0,
                    );
                    installed.push(folder_name);
                }
            },
        }
    }

    if let Err(e) = metadata::save_metadata(addons_dir, &store) {
        // Files extracted but kalpa.json didn't persist — surface partial state
        // instead of discarding the result with a blanket error.
        let reason = format!("Imported but could not be saved to kalpa.json: {e}");
        eprintln!("[import] metadata save failed: {e}");
        for folder_name in installed.drain(..) {
            errors.insert(folder_name.clone(), reason.clone());
            failed.push(folder_name);
        }
    }

    Ok(ImportResult {
        installed,
        failed,
        skipped,
        errors,
    })
}

// ─── Batch Pack Install ──────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackInstallEntry {
    pub esoui_id: u32,
    /// Display label for progress events (addon title); falls back to the id.
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackInstallProgress {
    pub esoui_id: u32,
    pub label: String,
    /// "downloading" | "extracting" | "completed" | "failed"
    pub phase: String,
    pub index: usize,
    pub total: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackInstallResult {
    /// esoui_ids that installed successfully.
    pub installed: Vec<u32>,
    /// esoui_ids that failed to download or extract.
    pub failed: Vec<u32>,
    /// Folders written by the installed pack addons (excludes transitive deps).
    pub installed_folders: Vec<String>,
    /// Transitive dependencies auto-installed across the whole pack.
    pub installed_deps: Vec<String>,
    pub failed_deps: Vec<String>,
    pub skipped_deps: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub errors: HashMap<u32, String>,
}

struct PackDownloaded {
    tmp: NamedTempFile,
    info: EsouiAddonInfo,
    esoui_id: u32,
    label: String,
}

/// Install a pack's addons in one pass: parallel download (4-way with 429
/// backoff) outside the lock, then a single locked extract+record phase with
/// ONE transitive-dependency resolution over the union of installed folders
/// and ONE metadata save. Mirrors import_addon_list, plus per-addon streaming
/// progress on the "pack-install-progress" event so the three pack UIs can
/// reflect download/extract/completed/failed per addon.
#[tauri::command]
pub async fn batch_install_pack_addons(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    app: tauri::AppHandle,
    addons_path: String,
    entries: Vec<PackInstallEntry>,
) -> Result<PackInstallResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let lock = meta_lock.0.clone();

    tokio::task::spawn_blocking(move || {
        let total = entries.len();
        let label_for = |e: &PackInstallEntry| {
            if e.label.is_empty() {
                e.esoui_id.to_string()
            } else {
                e.label.clone()
            }
        };

        // Phase 1 (parallel, no lock): download every addon's ZIP, emitting
        // progress per addon. Network I/O does not touch kalpa.json.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .map_err(|e| format!("Thread pool error: {e}"))?;

        let app_dl = app.clone();
        let download_results: Vec<(usize, Result<PackDownloaded, String>)> = pool.install(|| {
            entries
                .par_iter()
                .enumerate()
                .map(|(i, entry)| {
                    let label = label_for(entry);
                    let _ = app_dl.emit(
                        "pack-install-progress",
                        PackInstallProgress {
                            esoui_id: entry.esoui_id,
                            label: label.clone(),
                            phase: "downloading".to_string(),
                            index: i,
                            total,
                        },
                    );
                    let result =
                        fetch_and_download_with_retry(entry.esoui_id).map(|(tmp, info)| {
                            PackDownloaded {
                                tmp,
                                info,
                                esoui_id: entry.esoui_id,
                                label: label.clone(),
                            }
                        });
                    if result.is_err() {
                        let _ = app_dl.emit(
                            "pack-install-progress",
                            PackInstallProgress {
                                esoui_id: entry.esoui_id,
                                label,
                                phase: "failed".to_string(),
                                index: i,
                                total,
                            },
                        );
                    }
                    (i, result)
                })
                .collect()
        });

        // Phase 2 (locked): extract + record under a single lock, collect the
        // union of installed folders, resolve transitive deps once, save once.
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;

        let mut store = metadata::load_metadata(&addons_dir);
        let mut installed: Vec<u32> = Vec::new();
        let mut failed: Vec<u32> = Vec::new();
        let mut errors: HashMap<u32, String> = HashMap::new();
        let mut union_folders: Vec<String> = Vec::new();

        for (index, result) in download_results {
            // esoui_id/label are carried in the Ok payload; on Err we still
            // need them, so recover from the original entries by index.
            match result {
                Err(e) => {
                    let esoui_id = entries[index].esoui_id;
                    errors.insert(esoui_id, e);
                    failed.push(esoui_id);
                }
                Ok(dl) => {
                    let _ = app.emit(
                        "pack-install-progress",
                        PackInstallProgress {
                            esoui_id: dl.esoui_id,
                            label: dl.label.clone(),
                            phase: "extracting".to_string(),
                            index,
                            total,
                        },
                    );
                    match installer::extract_addon_zip(dl.tmp.path(), &addons_dir) {
                        Err(e) => {
                            errors.insert(dl.esoui_id, e);
                            failed.push(dl.esoui_id);
                            let _ = app.emit(
                                "pack-install-progress",
                                PackInstallProgress {
                                    esoui_id: dl.esoui_id,
                                    label: dl.label.clone(),
                                    phase: "failed".to_string(),
                                    index,
                                    total,
                                },
                            );
                        }
                        Ok(folders) => {
                            // Record the modification baseline BEFORE tracking the
                            // install in metadata. Without it, the next update sees
                            // stored=None, treats every file as unmodified, and would
                            // silently overwrite the user's edits — the exact failure
                            // the hash system prevents. If the baseline can't persist,
                            // treat the addon as failed rather than leaving metadata
                            // pointing at a folder with no baseline (mirrors
                            // install_addon_blocking and try_install_dep).
                            if let Err(e) = file_hashes::record_hashes_for_folders(
                                &addons_dir,
                                &folders,
                                dl.esoui_id,
                                &dl.info.version,
                            ) {
                                errors.insert(dl.esoui_id, e);
                                failed.push(dl.esoui_id);
                                let _ = app.emit(
                                    "pack-install-progress",
                                    PackInstallProgress {
                                        esoui_id: dl.esoui_id,
                                        label: dl.label.clone(),
                                        phase: "failed".to_string(),
                                        index,
                                        total,
                                    },
                                );
                                continue;
                            }

                            record_installed_folders(
                                &mut store,
                                &addons_dir,
                                &folders,
                                dl.esoui_id,
                                &dl.info.version,
                                &dl.info.title,
                                &dl.info.download_url,
                                0,
                            );
                            union_folders.extend(folders.iter().cloned());
                            installed.push(dl.esoui_id);
                            let _ = app.emit(
                                "pack-install-progress",
                                PackInstallProgress {
                                    esoui_id: dl.esoui_id,
                                    label: dl.label.clone(),
                                    phase: "completed".to_string(),
                                    index,
                                    total,
                                },
                            );
                        }
                    }
                }
            }
        }

        // One transitive-dependency pass over everything the pack installed.
        let resolved = resolve_transitive_deps(&addons_dir, &union_folders, &mut store);

        if let Err(e) = metadata::save_metadata(&addons_dir, &store) {
            // The addons were extracted to disk, but kalpa.json didn't persist
            // (e.g. Windows Controlled Folder Access, read-only or full disk).
            // Don't discard the whole result and report a blanket failure: move
            // every installed addon into `failed` with the save error so the UI
            // surfaces partial state, and still return Ok so the frontend can
            // refresh. Mirrors update_batch_with_decisions' save handling.
            let reason = format!("Installed but could not be saved to kalpa.json: {e}");
            eprintln!("[pack-install] metadata save failed: {e}");
            for esoui_id in installed.drain(..) {
                errors.insert(esoui_id, reason.clone());
                failed.push(esoui_id);
            }
        }

        Ok(PackInstallResult {
            installed,
            failed,
            installed_folders: union_folders,
            installed_deps: resolved.installed_deps,
            failed_deps: resolved.failed_deps,
            skipped_deps: resolved.skipped_deps,
            errors,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// ─── Category Browsing ───────────────────────────────────────

#[tauri::command]
pub async fn get_esoui_categories() -> Result<Vec<EsouiCategory>, String> {
    tokio::task::spawn_blocking(esoui::fetch_categories)
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn browse_esoui_category(
    category_id: u32,
    page: u32,
    sort_by: String,
) -> Result<Vec<esoui::EsouiSearchResult>, String> {
    tokio::task::spawn_blocking(move || esoui::browse_category(category_id, page, &sort_by))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn browse_esoui_popular(
    page: u32,
    sort_by: String,
) -> Result<esoui::BrowsePopularPage, String> {
    tokio::task::spawn_blocking(move || esoui::browse_popular(page, &sort_by))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

// ─── API Version Compatibility ───────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiCompatInfo {
    pub game_api_version: u32,
    pub outdated_addons: Vec<String>,
    pub up_to_date_addons: Vec<String>,
}

#[tauri::command]
pub async fn check_api_compatibility(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<ApiCompatInfo, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        // Read the game's current API version from AddOnSettings.txt
        let settings_path = addons_dir
            .parent()
            .map(|p| p.join("AddOnSettings.txt"))
            .ok_or("Could not find AddOnSettings.txt.")?;

        let game_api_version = if settings_path.exists() {
            let content = fs::read_to_string(&settings_path)
                .map_err(|e| format!("Failed to read AddOnSettings.txt: {e}"))?;
            content
                .lines()
                .find(|line| line.starts_with("#Version"))
                .and_then(|line| line.strip_prefix("#Version").map(|s| s.trim()))
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0)
        } else {
            return Err(
                "AddOnSettings.txt not found. Make sure you've launched ESO at least once."
                    .to_string(),
            );
        };

        if game_api_version == 0 {
            return Err("Could not determine game API version.".to_string());
        }

        // Check each addon's APIVersion against the game's version
        let entries =
            fs::read_dir(&addons_dir).map_err(|e| format!("Failed to read AddOns folder: {e}"))?;

        let mut outdated_addons: Vec<String> = Vec::new();
        let mut up_to_date_addons: Vec<String> = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let folder_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            let manifest = find_manifest(&addons_dir, &folder_name)
                .and_then(|p| manifest::parse_manifest(&folder_name, &p));

            if let Some(m) = manifest {
                if m.api_version.is_empty() {
                    continue;
                }
                // Addon is compatible if any of its API versions matches the game's
                let compatible = m.api_version.contains(&game_api_version);
                if compatible {
                    up_to_date_addons.push(m.title);
                } else {
                    outdated_addons.push(m.title);
                }
            }
        }

        outdated_addons.sort();
        up_to_date_addons.sort();

        Ok(ApiCompatInfo {
            game_api_version,
            outdated_addons,
            up_to_date_addons,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// ─── SavedVariables Backup & Restore ─────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub name: String,
    pub created_at: String,
    pub created_at_epoch: u64,
    pub file_count: u32,
    pub total_size: u64,
    pub kind: BackupKind,
    /// Distinct megaservers this backup's world-scoped subtrees span (read
    /// from the character-backup metadata sidecar). `> 1` means an
    /// Unknown-server backup silently bundled a same-named twin from another
    /// server. `None` for non-`Character` backups or if the sidecar can't be
    /// read (e.g. an older backup predating this field).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worlds_spanned: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BackupKind {
    Manual,
    AutoBeforeRestore,
    Character,
}

use crate::saved_variables::io as sv_io;

fn backups_dir(addons_dir: &std::path::Path) -> PathBuf {
    sv_io::backups_dir(addons_dir)
}

fn saved_variables_dir(addons_dir: &std::path::Path) -> PathBuf {
    sv_io::saved_variables_dir(addons_dir)
}

#[tauri::command]
pub async fn list_backups(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<Vec<BackupInfo>, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        let backups = backups_dir(&addons_dir);
        if !backups.is_dir() {
            return Ok(Vec::new());
        }

        // Self-heal any character backup orphaned by a crash mid-finalization before
        // listing, so a recovered backup is visible to the restore flow.
        recover_orphaned_backups(&backups);

        let mut results: Vec<BackupInfo> = Vec::new();
        let entries =
            fs::read_dir(&backups).map_err(|e| format!("Failed to read backups folder: {e}"))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            // Skip dot-prefixed transient dirs (e.g. a crash-orphaned char-backup
            // staging folder); real backups never start with '.'.
            if name.starts_with('.') {
                continue;
            }

            // Count files and total size (skip dot-prefixed metadata like the
            // character-backup marker so the count reflects real SavedVariables).
            let mut file_count: u32 = 0;
            let mut total_size: u64 = 0;
            if let Ok(files) = fs::read_dir(&path) {
                for f in files.flatten() {
                    let is_dotfile = f
                        .file_name()
                        .to_str()
                        .map(|n| n.starts_with('.'))
                        .unwrap_or(false);
                    if f.path().is_file() && !is_dotfile {
                        file_count += 1;
                        total_size += f.metadata().map(|m| m.len()).unwrap_or(0);
                    }
                }
            }

            // Extract timestamp from folder name or use modification time
            let created_at_epoch = fs::metadata(&path)
                .and_then(|m| m.modified())
                .map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                })
                .unwrap_or(0);
            let created_at = metadata::format_timestamp(created_at_epoch);

            // Only classify a `char-`-named dir as Character when it actually
            // carries the v2 marker file — an unmarked `char-*` dir (e.g. a
            // legacy/manual backup routed there by `resolve_char_backup_name`)
            // restores via the WholeFile path (see classify_backup_for_restore),
            // so it must display as Manual to match, not claim a Character
            // format it doesn't have.
            let kind = if name.starts_with("char-") && path.join(CHAR_BACKUP_MARKER).is_file() {
                BackupKind::Character
            } else if name.starts_with("auto-before-restore-") {
                BackupKind::AutoBeforeRestore
            } else {
                BackupKind::Manual
            };

            // For a Character backup, read `worlds_spanned` from its metadata
            // sidecar so the restore UI can warn about an Unknown-server backup
            // that silently spans multiple megaservers. Best-effort: a missing or
            // unreadable sidecar (e.g. an older backup) just yields `None`.
            let worlds_spanned = (kind == BackupKind::Character)
                .then(|| fs::read(path.join(CHAR_BACKUP_META)).ok())
                .flatten()
                .and_then(|bytes| serde_json::from_slice::<CharBackupMeta>(&bytes).ok())
                .map(|m| m.worlds_spanned);

            results.push(BackupInfo {
                name,
                created_at,
                created_at_epoch,
                file_count,
                total_size,
                kind,
                worlds_spanned,
            });
        }

        results.sort_by_key(|b| std::cmp::Reverse(b.created_at_epoch));
        Ok(results)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Reserved directory-name prefixes inside `kalpa-backups`. User-created manual
/// backups must not use them, or they could collide with — and the transactional
/// swap could overwrite — a character backup (`char-`), a safe-restore snapshot
/// (`auto-before-restore-`), an auto-cleanup snapshot (`auto-cleanup-`, created
/// by `delete_saved_variables_blocking` in `io.rs`), or a dot-prefixed scratch dir.
const RESERVED_BACKUP_PREFIXES: &[&str] = &["char-", "auto-before-restore-", "auto-cleanup-"];

/// Validate a user-supplied *manual* backup name: the general name rules plus a
/// rejection of the reserved internal prefixes. (Restore/delete keep using
/// `validate_name` so they can still address `char-`/`auto-` backups.)
fn validate_backup_name(name: &str) -> Result<(), String> {
    validate_name(name)?;
    if name.starts_with('.') || RESERVED_BACKUP_PREFIXES.iter().any(|p| name.starts_with(p)) {
        return Err(
            "Backup name uses a reserved prefix (e.g. \"char-\"). Please choose another name."
                .to_string(),
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn create_backup(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    backup_name: String,
) -> Result<BackupInfo, String> {
    validate_backup_name(&backup_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        // Serialize against every other backup-surface command (restore/delete/
        // character-backup) for the whole operation.
        let _mutation_guard = BACKUP_MUTATION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let sv_dir = saved_variables_dir(&addons_dir);
        if !sv_dir.is_dir() {
            return Err("SavedVariables folder not found.".to_string());
        }

        let backups = backups_dir(&addons_dir);
        fs::create_dir_all(&backups)
            .map_err(|e| format!("Failed to create backups folder: {e}"))?;

        let backup_path = backups.join(&backup_name);
        if backup_path.exists() {
            return Err(format!("Backup '{backup_name}' already exists."));
        }

        // Best-effort sweep of stale `.tmp-manual-*` staging dirs left behind by a
        // crash between a prior staging-dir creation and its final rename below.
        // Safe to remove unconditionally here: `BACKUP_MUTATION_LOCK` (acquired
        // above) serializes every `create_backup` call, so none can be mid-staging
        // concurrently, and manual staging is always disposable — a partial
        // manual backup is discarded on failure, never recovered.
        if let Ok(entries) = fs::read_dir(&backups) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir()
                    && path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(".tmp-manual-"))
                {
                    let _ = fs::remove_dir_all(&path);
                }
            }
        }

        // Stage into a dot-prefixed sibling dir and only rename it into the final
        // visible name once every file has copied successfully. This mirrors the
        // character-backup staging pattern: a crash mid-loop leaves only an
        // ignored `.tmp-manual-*` scratch dir (list_backups skips '.'-prefixed
        // entries) instead of a partial backup under the real, restorable name.
        let seq = BACKUP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let staging = backups.join(format!(".tmp-manual-{backup_name}-{seq}"));
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir_all(&staging).map_err(|e| format!("Failed to create backup: {e}"))?;

        // Copy all .lua files from SavedVariables
        let mut file_count: u32 = 0;
        let mut total_size: u64 = 0;
        let entries = fs::read_dir(&sv_dir).map_err(|e| {
            let _ = fs::remove_dir_all(&staging);
            format!("Failed to read SavedVariables: {e}")
        })?;

        let mut failed: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name() {
                    let dest = staging.join(name);
                    match fs::copy(&path, &dest) {
                        Ok(_) => {
                            file_count += 1;
                            total_size += fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
                        }
                        Err(e) => {
                            failed.push(format!("{}: {}", name.to_string_lossy(), e));
                        }
                    }
                }
            }
        }

        if !failed.is_empty() {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!(
                "Backup incomplete — {} file(s) failed to copy: {}",
                failed.len(),
                failed.join(", ")
            ));
        }

        // All files copied — publish atomically under the real name.
        if let Err(e) = fs::rename(&staging, &backup_path) {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("Failed to finalize backup: {e}"));
        }

        let created_at_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let created_at = metadata::format_timestamp(created_at_epoch);

        // Manual backups created here never carry the `char-` marked format —
        // reserved prefixes are rejected up front by `validate_backup_name`, and
        // this path never writes CHAR_BACKUP_MARKER — so classify strictly
        // between AutoBeforeRestore and Manual (see list_backups for the
        // marker-based Character classification).
        let kind = if backup_name.starts_with("auto-before-restore-") {
            BackupKind::AutoBeforeRestore
        } else {
            BackupKind::Manual
        };

        Ok(BackupInfo {
            name: backup_name,
            created_at,
            created_at_epoch,
            file_count,
            total_size,
            kind,
            // This path never creates a Character backup (see `kind` above), so
            // there is no sidecar to read a world span from.
            worlds_spanned: None,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeRestoreResult {
    pub restored_files: u32,
    pub safety_snapshot: Option<BackupInfo>,
}

/// Remove oldest snapshots matching `prefix`, keeping at most `keep` of the most recent.
/// Errors are logged but do not fail the caller — pruning is best-effort.
fn prune_auto_snapshots(backups_dir: &std::path::Path, prefix: &str, keep: usize) {
    let mut dirs: Vec<_> = match fs::read_dir(backups_dir) {
        Ok(rd) => rd
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(prefix) && e.path().is_dir())
            .collect(),
        Err(_) => return,
    };
    if dirs.len() <= keep {
        return;
    }
    // Sort by name ascending (names embed epoch timestamps, so lexicographic == chronological).
    dirs.sort_by_key(|e| e.file_name());
    let to_remove = dirs.len() - keep;
    for entry in dirs.into_iter().take(to_remove) {
        if let Err(e) = fs::remove_dir_all(entry.path()) {
            eprintln!(
                "Warning: failed to remove old auto-snapshot {:?}: {}",
                entry.path(),
                e
            );
        }
    }
}

/// Restore a per-character (v2) backup by MERGING each stored character subtree
/// into the matching live SavedVariables file, leaving other characters and all
/// account-wide data byte-identical. Returns `(restored_file_count, failures)`.
///
/// This is all-or-nothing: every backup file is read, extracted, and merged
/// into memory first (phase 1); only if every file merges cleanly are the
/// results written to disk (phase 2). If any file fails during phase 1, the
/// restore aborts before writing anything, so live SavedVariables can never
/// be left in a mixed old/new state (the caller has already taken a safety
/// snapshot before calling this).
fn restore_character_subtrees_merge(
    backup_path: &Path,
    sv_dir: &Path,
    meta: &CharBackupMeta,
) -> (u32, Vec<String>) {
    use crate::saved_variables::char_backup::{
        char_base, extract_character_blocks, merge_character_block,
    };

    let mut restored: u32 = 0;
    let mut failed: Vec<String> = Vec::new();

    let base = char_base(meta.character.as_bytes()).to_vec();
    // Re-extract from the (already-isolated) stored files using the SAME world
    // filter the backup used, so a known-server backup can never restore a
    // subtree under a different megaserver even if one somehow leaked into the
    // stored file. Unknown server -> no filter (account-keyed / any world).
    let world: Option<&str> =
        if crate::saved_variables::scrub::WELL_KNOWN_WORLDS.contains(&meta.server.as_str()) {
            Some(meta.server.as_str())
        } else {
            None
        };

    let entries = match fs::read_dir(backup_path) {
        Ok(e) => e,
        Err(e) => {
            failed.push(format!("backup: {e}"));
            return (restored, failed);
        }
    };

    // Phase 1: read + extract + merge every backup file into memory without
    // touching any live SavedVariables file. If any file fails, abort
    // immediately (before phase 2 writes anything) so a mid-list failure can
    // never leave the live data in a mixed old/new state.
    let mut buffered: Vec<(String, Vec<u8>)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        let name_str = name.to_string_lossy().to_string();
        // Skip the dot-prefixed marker/metadata and any non-.lua file.
        if name_str.starts_with('.') || path.extension().and_then(|e| e.to_str()) != Some("lua") {
            continue;
        }
        let stored = match fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                failed.push(format!("{name_str}: {e}"));
                return (0, failed);
            }
        };
        let blocks = extract_character_blocks(&stored, &base, world);
        if blocks.is_empty() {
            failed.push(format!("{name_str}: no character subtree found in backup"));
            return (0, failed);
        }
        let live_path = sv_dir.join(&name_str);
        let mut live = if live_path.is_file() {
            match fs::read(&live_path) {
                Ok(b) => b,
                Err(e) => {
                    failed.push(format!("{name_str}: {e}"));
                    return (0, failed);
                }
            }
        } else {
            Vec::new()
        };
        for block in &blocks {
            match merge_character_block(&live, block) {
                Ok(merged) => live = merged,
                Err(e) => {
                    failed.push(format!("{name_str}: {e}"));
                    return (0, failed);
                }
            }
        }
        buffered.push((name_str, live));
    }

    // Phase 2: every file merged cleanly in phase 1 (no early return above),
    // so commit all buffered writes.
    for (name_str, bytes) in buffered {
        match sv_io::write_raw_bytes(sv_dir, &name_str, &bytes) {
            Ok(_) => restored += 1,
            Err(e) => failed.push(format!("{name_str}: {e}")),
        }
    }

    (restored, failed)
}

/// Restore a backup, but first capture the user's current SavedVariables into a
/// timestamped "auto-before-restore-…" snapshot so the restore can be undone.
/// If the user has no current SavedVariables, the snapshot step is skipped.
///
/// A per-character (v2) backup — identified by its `.kalpa-char-backup.json`
/// metadata — is restored by MERGING each stored subtree back into the matching
/// live file (other characters / account-wide data untouched). Every other
/// backup (manual, auto-before-restore, and LEGACY whole-file character backups
/// that predate the per-character format) restores by copying whole files.
#[tauri::command]
pub async fn restore_backup_safe(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    backup_name: String,
) -> Result<SafeRestoreResult, String> {
    validate_name(&backup_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
    // Serialize against every other backup-surface command (create/delete/
    // character-backup) for the whole operation.
    let _mutation_guard = BACKUP_MUTATION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let sv_dir = saved_variables_dir(&addons_dir);
    let backup_path = backups_dir(&addons_dir).join(&backup_name);

    if !backup_path.is_dir() {
        return Err(format!("Backup '{backup_name}' not found."));
    }

    let mut safety_snapshot: Option<BackupInfo> = None;

    if sv_dir.is_dir() {
        let has_files = fs::read_dir(&sv_dir)
            .map(|mut it| it.any(|e| e.as_ref().map(|e| e.path().is_file()).unwrap_or(false)))
            .unwrap_or(false);
        if has_files {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let snapshot_name = format!("auto-before-restore-{now}");
            let snapshot_path = backups_dir(&addons_dir).join(&snapshot_name);
            fs::create_dir_all(&snapshot_path)
                .map_err(|e| format!("Failed to create safety snapshot folder: {e}"))?;

            let mut file_count: u32 = 0;
            let mut total_size: u64 = 0;
            let entries =
                fs::read_dir(&sv_dir).map_err(|e| format!("Failed to read SavedVariables: {e}"))?;
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(name) = path.file_name() {
                        let dest = snapshot_path.join(name);
                        fs::copy(&path, &dest).map_err(|e| {
                            format!(
                                "Failed to copy '{}' to safety snapshot: {}. Restore aborted to prevent data loss.",
                                path.display(),
                                e
                            )
                        })?;
                        file_count += 1;
                        total_size += fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
                    }
                }
            }

            safety_snapshot = Some(BackupInfo {
                name: snapshot_name,
                created_at: metadata::format_timestamp(now),
                created_at_epoch: now,
                file_count,
                total_size,
                kind: BackupKind::AutoBeforeRestore,
                worlds_spanned: None,
            });

            // Keep only the 3 most recent auto-before-restore snapshots to prevent
            // unbounded disk growth (SavedVariables can reach 1-2 GB on trade-addon-heavy accounts).
            prune_auto_snapshots(&backups_dir(&addons_dir), "auto-before-restore-", 3);
        }
    }

    fs::create_dir_all(&sv_dir)
        .map_err(|e| format!("Failed to create SavedVariables folder: {e}"))?;

    let mut restored: u32 = 0;
    let mut failed: Vec<String> = Vec::new();

    match classify_backup_for_restore(&backup_path) {
        CharRestoreMode::Refuse(reason) => return Err(reason),
        CharRestoreMode::Merge(meta) => {
            // Per-character backup: merge each stored subtree into its live file,
            // leaving other characters and account-wide data untouched.
            let (r, f) = restore_character_subtrees_merge(&backup_path, &sv_dir, &meta);
            restored = r;
            failed = f;
        }
        CharRestoreMode::WholeFile => {
            // Manual / auto-before-restore / legacy whole-file character backup:
            // read every source file into memory first (phase 1), then only
            // write them into the SavedVariables folder (phase 2) if every
            // read succeeded — a mid-list read failure must not leave the
            // live data half-restored.
            let entries =
                fs::read_dir(&backup_path).map_err(|e| format!("Failed to read backup: {e}"))?;
            let mut buffered: Vec<(String, Vec<u8>)> = Vec::new();
            for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name() {
                    // Skip dot-prefixed metadata (e.g. the character-backup marker);
                    // it isn't a SavedVariables file and must not land in the game dir.
                    if name.to_str().map(|n| n.starts_with('.')).unwrap_or(false) {
                        continue;
                    }
                    let name_str = name.to_string_lossy().to_string();
                    match fs::read(&path) {
                        Ok(bytes) => buffered.push((name_str, bytes)),
                        Err(e) => {
                            failed.push(format!("{name_str}: {e}"));
                        }
                    }
                }
            }
            }

            if failed.is_empty() {
                for (name_str, bytes) in buffered {
                    match sv_io::write_raw_bytes(&sv_dir, &name_str, &bytes) {
                        Ok(_) => restored += 1,
                        Err(e) => failed.push(format!("{name_str}: {e}")),
                    }
                }
            }
        }
    }

    if !failed.is_empty() {
        let snap_note = match &safety_snapshot {
            Some(snap) => format!(
                " Your previous settings were saved as a safety snapshot (\"{}\") and can be restored to undo.",
                snap.name
            ),
            None => String::new(),
        };
        return Err(format!(
            "Restore incomplete — {} file(s) failed to copy: {}{}",
            failed.len(),
            failed.join(", "),
            snap_note
        ));
    }

    Ok(SafeRestoreResult {
        restored_files: restored,
        safety_snapshot,
    })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Return the absolute path to the kalpa-backups folder so the UI can reveal it.
#[tauri::command]
pub fn get_backups_folder_path(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<String, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let path = backups_dir(&addons_dir);
    fs::create_dir_all(&path).map_err(|e| format!("Failed to create backups folder: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn delete_backup(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    backup_name: String,
) -> Result<(), String> {
    validate_name(&backup_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        // Serialize against every other backup-surface command (create/restore/
        // character-backup) for the whole operation. Acquired before
        // `BACKUP_FINALIZE_LOCK` (taken below by `recover_orphaned_backups` and
        // `purge_backup_scratch`) to keep lock ordering consistent.
        let _mutation_guard = BACKUP_MUTATION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let backups_root = backups_dir(&addons_dir);
        // Reconcile crash leftovers before deleting so a deletion can't race with a
        // pending recovery for the same backup name.
        recover_orphaned_backups(&backups_root);
        let backup_path = backups_root.join(&backup_name);

        if !backup_path.is_dir() {
            return Err(format!("Backup '{backup_name}' not found."));
        }

        fs::remove_dir_all(&backup_path).map_err(|e| format!("Failed to delete backup: {e}"))?;

        // Also purge any leftover staging/tombstone scratch dirs for this backup so a
        // crash-recovery pass can't later resurrect a deleted character backup from a
        // tombstone that outlived a successful swap (e.g. failed cleanup).
        purge_backup_scratch(&backups_root, &backup_name);
        Ok(())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Remove `.tmp-<name>-<seq>` / `.old-<name>-<seq>` scratch directories left for
/// the backup directory `name` (e.g. `char-Bob-backup`). Best-effort.
fn purge_backup_scratch(backups_root: &Path, name: &str) {
    let _guard = BACKUP_FINALIZE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entries = match fs::read_dir(backups_root) {
        Ok(e) => e,
        Err(_) => return,
    };
    let tmp_prefix = format!(".tmp-{name}-");
    let old_prefix = format!(".old-{name}-");
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(entry_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // The trailing segment is the numeric sequence id we appended.
        let is_scratch = |prefix: &str| {
            entry_name
                .strip_prefix(prefix)
                .is_some_and(|seq| !seq.is_empty() && seq.bytes().all(|b| b.is_ascii_digit()))
        };
        if is_scratch(&tmp_prefix) || is_scratch(&old_prefix) {
            let _ = fs::remove_dir_all(&path);
        }
    }
}

// ─── Addon Profiles ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonProfile {
    pub name: String,
    pub enabled_addons: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileStore {
    pub profiles: Vec<AddonProfile>,
    pub active_profile: Option<String>,
}

fn profiles_path(addons_dir: &std::path::Path) -> PathBuf {
    addons_dir.join("kalpa-profiles.json")
}

/// Stable FNV-1a 64-bit hash for deriving mirror file names from paths.
/// `DefaultHasher` is seeded per-process and would scatter one instance's
/// mirror across many files over time, so a fixed algorithm is required.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Where the app-data mirror of one instance's profile store lives:
/// `<data dir>/kalpa/profile-mirrors/<path hash>.json`. Keyed by the
/// canonicalized, lowercased AddOns path so live/EU/PTS each get their own
/// mirror and Windows path-casing differences can't split one instance in two.
fn default_profile_mirror_path(addons_dir: &std::path::Path) -> Option<PathBuf> {
    let canonical = addons_dir
        .canonicalize()
        .unwrap_or_else(|_| addons_dir.to_path_buf());
    let key = canonical.to_string_lossy().to_lowercase();
    let name = format!("{:016x}.json", fnv1a64(key.as_bytes()));
    Some(
        dirs::data_dir()?
            .join("kalpa")
            .join("profile-mirrors")
            .join(name),
    )
}

fn load_profiles_with_mirror(
    addons_dir: &std::path::Path,
    mirror: Option<&std::path::Path>,
) -> ProfileStore {
    let primary = profiles_path(addons_dir);
    // The store lives inside the AddOns folder (which is what scopes profiles
    // per instance), but that folder gets wiped by reinstalls and PTS cleanups.
    // When the primary AND its crash-recovery artifacts are all gone, fall back
    // to the app-data mirror so profiles survive the wipe.
    let primary_gone = !primary.exists()
        && !primary.with_extension("json.tmp").exists()
        && !primary.with_extension("json.bak").exists();
    if primary_gone {
        if let Some(mirror) = mirror {
            if let Ok(content) = fs::read_to_string(mirror) {
                if let Ok(store) = serde_json::from_str::<ProfileStore>(&content) {
                    return store;
                }
            }
        }
    }
    metadata::load_json_with_backup(&primary)
}

fn save_profiles_with_mirror(
    addons_dir: &std::path::Path,
    mirror: Option<&std::path::Path>,
    store: &ProfileStore,
) -> Result<(), String> {
    metadata::save_json_with_backup(&profiles_path(addons_dir), store)?;
    // Best-effort: the mirror is a recovery copy, so its write never fails the
    // save — a full data dir or blocked path just means no fallback this time.
    if let Some(mirror) = mirror {
        if let Some(parent) = mirror.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(store) {
            let _ = fs::write(mirror, json);
        }
    }
    Ok(())
}

fn load_profiles(addons_dir: &std::path::Path) -> ProfileStore {
    load_profiles_with_mirror(
        addons_dir,
        default_profile_mirror_path(addons_dir).as_deref(),
    )
}

fn save_profiles(addons_dir: &std::path::Path, store: &ProfileStore) -> Result<(), String> {
    save_profiles_with_mirror(
        addons_dir,
        default_profile_mirror_path(addons_dir).as_deref(),
        store,
    )
}

#[tauri::command]
pub fn list_profiles(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<(Vec<AddonProfile>, Option<String>), String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let store = load_profiles(&addons_dir);
    Ok((store.profiles, store.active_profile))
}

/// Snapshot the currently enabled addons: top-level folders that carry a
/// matching manifest (the same gate ESO applies when loading).
fn snapshot_enabled_addons(addons_dir: &std::path::Path) -> Vec<String> {
    let mut enabled: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(addons_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let folder_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            // Only include folders with manifests (actual addons)
            if find_manifest(addons_dir, &folder_name).is_some() {
                enabled.push(folder_name);
            }
        }
    }
    enabled.sort();
    enabled
}

fn now_timestamp() -> String {
    metadata::format_timestamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
}

#[tauri::command]
pub fn create_profile(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    profile_name: String,
) -> Result<AddonProfile, String> {
    validate_name(&profile_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let mut store = load_profiles(&addons_dir);

    if store.profiles.iter().any(|p| p.name == profile_name) {
        return Err(format!("Profile '{profile_name}' already exists."));
    }

    let profile = AddonProfile {
        name: profile_name,
        enabled_addons: snapshot_enabled_addons(&addons_dir),
        created_at: now_timestamp(),
    };

    store.profiles.push(profile.clone());
    save_profiles(&addons_dir, &store)?;

    Ok(profile)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivateProfileResult {
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
    pub failed: Vec<String>,
    pub missing: Vec<String>,
    /// Addons kept (or re-)enabled because an addon in the profile requires
    /// them via `## DependsOn`, even though they are not in the snapshot —
    /// typically libraries installed after the profile was created.
    pub kept_dependencies: Vec<String>,
}

/// The read-only rename plan for activating a profile — what WOULD change.
/// Powers the pre-activation preview in the UI; `apply_profile` recomputes it
/// at activation time so a stale preview can never rename the wrong folders.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilePlan {
    pub to_enable: Vec<String>,
    pub to_disable: Vec<String>,
    /// See [`ActivateProfileResult::kept_dependencies`].
    pub kept_dependencies: Vec<String>,
    pub missing: Vec<String>,
    /// Addons the plan wants disabled but cannot: both `Foo/` and
    /// `Foo.disabled/` exist, so the rename would collide.
    pub blocked: Vec<String>,
}

/// On-disk copies of one addon base name: `Foo/` and/or `Foo.disabled/`.
#[derive(Default)]
struct FolderCopies {
    enabled: Option<PathBuf>,
    disabled: Option<PathBuf>,
}

impl FolderCopies {
    /// The copy ESO would load after activation. When both exist the enabled
    /// copy wins — the same duplicate policy the scanner applies.
    fn winning_path(&self) -> Option<&PathBuf> {
        self.enabled.as_ref().or(self.disabled.as_ref())
    }
}

/// Compute the rename plan for activating a profile WITHOUT touching disk.
///
/// Extracted from the `activate_profile` command so the planning is
/// unit-testable without Tauri state, and exposed via `preview_profile` so
/// the UI can show what will change before anything is renamed. Two behaviors
/// go beyond the raw snapshot:
///
/// - **Dependency retention**: any installed addon that a profile addon
///   (transitively) requires via `## DependsOn` is kept enabled — or
///   re-enabled — even when absent from the snapshot. Profiles are created
///   before later updates can pull in new libraries, so activating an old
///   snapshot must not disable a library its own addons need at load time.
///   Dependencies already satisfied by a bundled copy inside an enabled
///   folder (ESO's nested-addon resolution) trigger no retention.
/// - **Duplicate tolerance**: when both `Foo/` and `Foo.disabled/` exist the
///   enabled copy wins (matching the scanner), so an "enable" is already
///   satisfied and plans no rename; a "disable" cannot be expressed by
///   rename and is reported as `blocked` instead of attempted.
pub(crate) fn plan_profile(addons_dir: &std::path::Path, profile: &AddonProfile) -> ProfilePlan {
    // Group every top-level directory by base name so both copies of a
    // duplicated addon are visible to one decision below.
    let mut copies: HashMap<String, FolderCopies> = HashMap::new();
    if let Ok(entries) = fs::read_dir(addons_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let folder_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            // Skip non-addon folders and our own files
            if folder_name.starts_with("kalpa-") {
                continue;
            }

            match folder_name.strip_suffix(".disabled") {
                Some(base) => copies.entry(base.to_string()).or_default().disabled = Some(path),
                None => copies.entry(folder_name).or_default().enabled = Some(path),
            }
        }
    }

    // Names each top-level folder makes loadable: its own manifest name plus
    // bundled libraries at ESO's nested-addon resolution depth (normalized,
    // see `normalize_addon_name`).
    let provides: HashMap<String, HashSet<String>> = copies
        .iter()
        .filter_map(|(base, c)| {
            let path = c.winning_path()?;
            let mut names = HashSet::new();
            if find_manifest_in(path, base).is_some() {
                names.insert(normalize_addon_name(base));
            }
            collect_subfolder_names(path, &mut names);
            Some((base.clone(), names))
        })
        .collect();

    // The target enabled set: the snapshot itself, expanded below with the
    // installed dependencies of its addons.
    let mut desired: HashSet<String> = profile
        .enabled_addons
        .iter()
        .filter(|name| copies.contains_key(name.as_str()))
        .cloned()
        .collect();

    // Everything loadable after activation without further action: names
    // provided by desired folders, plus by manifest-less wrapper folders
    // (e.g. `Libs/`) — those carry bundled libraries but are never renamed
    // by profiles, so their contents always stay available.
    let mut satisfied: HashSet<String> = HashSet::new();
    for (base, c) in &copies {
        let untouched_wrapper = c
            .enabled
            .as_ref()
            .is_some_and(|path| find_manifest_in(path, base).is_none());
        if desired.contains(base) || untouched_wrapper {
            if let Some(names) = provides.get(base) {
                satisfied.extend(names.iter().cloned());
            }
        }
    }

    let mut kept_dependencies: Vec<String> = Vec::new();
    let mut queue: Vec<String> = desired.iter().cloned().collect();
    while let Some(base) = queue.pop() {
        let Some(manifest_path) = copies
            .get(&base)
            .and_then(|c| c.winning_path())
            .and_then(|path| find_manifest_in(path, &base))
        else {
            continue;
        };
        let Some(manifest) = manifest::parse_manifest(&base, &manifest_path) else {
            continue;
        };
        for dep in &manifest.depends_on {
            let key = normalize_addon_name(&dep.name);
            if satisfied.contains(&key) {
                continue;
            }
            // Find an installed top-level folder that provides this
            // dependency. Prefer an exact folder-name match over a wrapper
            // that merely bundles it, then a currently-enabled copy, then
            // lexicographic order for determinism.
            let mut candidates: Vec<&String> = provides
                .iter()
                .filter(|(b, names)| !desired.contains(*b) && names.contains(&key))
                .map(|(b, _)| b)
                .collect();
            candidates.sort_by_key(|b| {
                (
                    normalize_addon_name(b) != key,
                    copies.get(*b).is_none_or(|c| c.enabled.is_none()),
                    (*b).clone(),
                )
            });
            let Some(provider) = candidates.first().map(|b| (*b).clone()) else {
                continue; // genuinely missing — the scanner will flag it
            };
            desired.insert(provider.clone());
            if let Some(names) = provides.get(&provider) {
                satisfied.extend(names.iter().cloned());
            }
            kept_dependencies.push(provider.clone());
            queue.push(provider);
        }
    }

    let mut to_enable: Vec<String> = Vec::new();
    let mut to_disable: Vec<String> = Vec::new();
    let mut blocked: Vec<String> = Vec::new();

    for (base, c) in &copies {
        let want_enabled = desired.contains(base);
        match (&c.enabled, &c.disabled) {
            (Some(enabled_path), Some(_)) => {
                // Duplicate folders. Enabled copy wins, so wanting the addon
                // enabled is already satisfied; wanting it disabled cannot be
                // done by rename without a collision.
                if !want_enabled && find_manifest_in(enabled_path, base).is_some() {
                    blocked.push(base.clone());
                }
            }
            (Some(path), None) => {
                // Only disable folders that carry a manifest (real addons) —
                // never wrapper directories like `Libs/`.
                if !want_enabled && find_manifest_in(path, base).is_some() {
                    to_disable.push(base.clone());
                }
            }
            (None, Some(_)) => {
                if want_enabled {
                    to_enable.push(base.clone());
                }
            }
            (None, None) => {}
        }
    }

    // Report addons referenced in the profile that no longer exist on disk
    let mut missing: Vec<String> = profile
        .enabled_addons
        .iter()
        .filter(|name| !copies.contains_key(name.as_str()))
        .cloned()
        .collect();

    // Deterministic output for the UI and tests regardless of dir order.
    to_enable.sort();
    to_disable.sort();
    blocked.sort();
    missing.sort();
    kept_dependencies.sort();

    ProfilePlan {
        to_enable,
        to_disable,
        kept_dependencies,
        missing,
        blocked,
    }
}

/// Execute a profile activation: recompute the plan against the CURRENT disk
/// state, then perform the renames it calls for.
pub(crate) fn apply_profile(
    addons_dir: &std::path::Path,
    profile: &AddonProfile,
) -> ActivateProfileResult {
    let plan = plan_profile(addons_dir, profile);

    let mut enabled: Vec<String> = Vec::new();
    let mut disabled: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    for base in &plan.to_enable {
        let src = addons_dir.join(format!("{base}.disabled"));
        let dst = addons_dir.join(base);
        match fs::rename(&src, &dst) {
            Ok(_) => enabled.push(base.clone()),
            Err(e) => failed.push(format!("{base} (enable: {e})")),
        }
    }
    for base in &plan.to_disable {
        let src = addons_dir.join(base);
        let dst = addons_dir.join(format!("{base}.disabled"));
        match fs::rename(&src, &dst) {
            Ok(_) => disabled.push(base.clone()),
            Err(e) => failed.push(format!("{base} (disable: {e})")),
        }
    }
    for base in &plan.blocked {
        failed.push(format!(
            "{base} (disable: both '{base}' and '{base}.disabled' exist — remove the stale copy, then re-activate)"
        ));
    }
    failed.sort();

    ActivateProfileResult {
        enabled,
        disabled,
        failed,
        missing: plan.missing,
        kept_dependencies: plan.kept_dependencies,
    }
}

#[tauri::command]
pub async fn activate_profile(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    profile_name: String,
) -> Result<ActivateProfileResult, String> {
    validate_name(&profile_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        let mut store = load_profiles(&addons_dir);

        let profile = store
            .profiles
            .iter()
            .find(|p| p.name == profile_name)
            .cloned()
            .ok_or_else(|| format!("Profile '{profile_name}' not found."))?;

        let result = apply_profile(&addons_dir, &profile);

        store.active_profile = Some(profile_name);
        save_profiles(&addons_dir, &store)?;

        Ok(result)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Dry-run of `activate_profile`: what would be enabled/disabled/kept, with
/// no renames performed. The UI shows this as a confirmation step.
#[tauri::command]
pub async fn preview_profile(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    profile_name: String,
) -> Result<ProfilePlan, String> {
    validate_name(&profile_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        let store = load_profiles(&addons_dir);
        let profile = store
            .profiles
            .iter()
            .find(|p| p.name == profile_name)
            .ok_or_else(|| format!("Profile '{profile_name}' not found."))?;
        Ok(plan_profile(&addons_dir, profile))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Re-snapshot the currently enabled addons into an existing profile,
/// refreshing its timestamp — "update profile from current state".
#[tauri::command]
pub fn update_profile(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    profile_name: String,
) -> Result<AddonProfile, String> {
    validate_name(&profile_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let mut store = load_profiles(&addons_dir);

    let profile = store
        .profiles
        .iter_mut()
        .find(|p| p.name == profile_name)
        .ok_or_else(|| format!("Profile '{profile_name}' not found."))?;

    profile.enabled_addons = snapshot_enabled_addons(&addons_dir);
    profile.created_at = now_timestamp();
    let updated = profile.clone();

    save_profiles(&addons_dir, &store)?;
    Ok(updated)
}

#[tauri::command]
pub fn rename_profile(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    old_name: String,
    new_name: String,
) -> Result<(), String> {
    validate_name(&old_name)?;
    validate_name(&new_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let mut store = load_profiles(&addons_dir);

    if old_name == new_name {
        return Ok(());
    }
    if store.profiles.iter().any(|p| p.name == new_name) {
        return Err(format!("Profile '{new_name}' already exists."));
    }
    let profile = store
        .profiles
        .iter_mut()
        .find(|p| p.name == old_name)
        .ok_or_else(|| format!("Profile '{old_name}' not found."))?;

    profile.name = new_name.clone();
    if store.active_profile.as_deref() == Some(&old_name) {
        store.active_profile = Some(new_name);
    }

    save_profiles(&addons_dir, &store)
}

#[tauri::command]
pub fn delete_profile(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    profile_name: String,
) -> Result<(), String> {
    validate_name(&profile_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let mut store = load_profiles(&addons_dir);

    store.profiles.retain(|p| p.name != profile_name);
    if store.active_profile.as_deref() == Some(&profile_name) {
        store.active_profile = None;
    }

    save_profiles(&addons_dir, &store)
}

// ─── Cross-instance addon copy ───────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyAddonsResult {
    pub copied: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
}

/// Recursively copy a directory. On any error the partially-written
/// destination is removed so a failed copy can't leave a half-addon that a
/// later scan would mistake for a real install.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    fn walk(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let from = entry.path();
            let to = dst.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                walk(&from, &to)?;
            } else {
                fs::copy(&from, &to)?;
            }
        }
        Ok(())
    }
    walk(src, dst).map_err(|e| {
        let _ = fs::remove_dir_all(dst);
        format!("{e}")
    })
}

/// Copy every enabled addon folder (manifest-gated) from `source` into
/// `target`, skipping addons the target already has (enabled OR disabled —
/// never overwrite another instance's state). Metadata entries (ESOUI id,
/// tags, version) ride along for the copied addons so the target instance
/// can check updates for them immediately.
pub(crate) fn copy_addons_between(
    source: &std::path::Path,
    target: &std::path::Path,
) -> CopyAddonsResult {
    let mut copied: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    for folder in snapshot_enabled_addons(source) {
        let dst = target.join(&folder);
        let dst_disabled = target.join(format!("{folder}.disabled"));
        if dst.exists() || dst_disabled.exists() {
            skipped.push(folder);
            continue;
        }
        match copy_dir_recursive(&source.join(&folder), &dst) {
            Ok(_) => copied.push(folder),
            Err(e) => failed.push(format!("{folder}: {e}")),
        }
    }

    if !copied.is_empty() {
        let src_store = metadata::load_metadata(source);
        let mut dst_store = metadata::load_metadata(target);
        let mut changed = false;
        for name in &copied {
            if let Some(entry) = src_store.addons.get(name) {
                dst_store.addons.insert(name.clone(), entry.clone());
                changed = true;
            }
        }
        if changed {
            if let Err(e) = metadata::save_metadata(target, &dst_store) {
                failed.push(format!("metadata: {e}"));
            }
        }
    }

    copied.sort();
    skipped.sort();
    failed.sort();
    CopyAddonsResult {
        copied,
        skipped,
        failed,
    }
}

/// Copy the active instance's enabled addons into another DETECTED game
/// instance (e.g. set up PTS from the live loadout). The target is validated
/// against the freshly detected instance list — an arbitrary directory is
/// rejected so this command can't be used to write outside ESO's folders.
#[tauri::command]
pub async fn copy_addons_to_instance(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    target_addons_path: String,
) -> Result<CopyAddonsResult, String> {
    let source = require_allowed_path(&state, &addons_path)?;
    let target = PathBuf::from(&target_addons_path);
    let target_canonical = target
        .canonicalize()
        .map_err(|e| format!("Target AddOns folder is not accessible: {e}"))?;

    let is_detected_instance = crate::game_instances::detect_all_game_instances()
        .iter()
        .any(|inst| {
            PathBuf::from(&inst.addons_path)
                .canonicalize()
                .is_ok_and(|p| p == target_canonical)
        });
    if !is_detected_instance {
        return Err("Target folder is not a detected ESO game instance.".to_string());
    }
    if source.canonicalize().is_ok_and(|p| p == target_canonical) {
        return Err("Source and target are the same instance.".to_string());
    }

    tokio::task::spawn_blocking(move || Ok(copy_addons_between(&source, &target_canonical)))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

// ─── Multi-Character SavedVariables ──────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterInfo {
    pub server: String,
    pub name: String,
    /// True when this character was recovered from a SavedVariables addon file
    /// rather than from `AddOnSettings.txt`. This happens when the character has
    /// no per-character addon-settings block — most commonly because the account
    /// uses ESO's default "Account-Wide Addon Settings" mode, which collapses all
    /// per-character blocks into a single `$AccountWide` block.
    pub recovered: bool,
}

/// Result of `list_characters`: the roster plus how many SavedVariables files
/// the scan could not fully read (a directory/file I/O error, or a
/// malformed/truncated file whose structure didn't balance), so the UI can warn
/// that a character might be missing rather than silently showing a short list.
/// There is no longer a file-size limit, so being large is never a skip reason.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterRoster {
    pub characters: Vec<CharacterInfo>,
    pub skipped_files: u32,
}

/// Server bucket assigned to characters recovered from SavedVariables whose
/// megaserver cannot be determined from any addon's data layout.
const UNKNOWN_SERVER: &str = "Unknown";

/// Normalize an ESO character name for cross-source matching and display:
/// strip a raw-API caret suffix (e.g. `Faewynd^Mx` -> `Faewynd`) and trim
/// surrounding whitespace. ESO names are case-sensitive and unique per server,
/// so casing is intentionally preserved.
fn normalize_character_name(name: &str) -> String {
    name.split('^').next().unwrap_or(name).trim().to_string()
}

/// Build the character roster as the union of two local sources:
///   1. `AddOnSettings.txt` `#<Server>-<Name>` headers — authoritative for the
///      megaserver, but absent entirely in account-wide addon-settings mode.
///   2. Characters extracted from SavedVariables `.lua` files as `(raw_key,
///      world)`, where `world` is a megaserver only when the character sat under
///      a world-scoped layout.
///
/// Dedup keeps a character present in both sources once, under its real server.
/// SavedVariables-only characters without a derivable megaserver are bucketed
/// under [`UNKNOWN_SERVER`]. Pure function so the union logic is unit testable
/// without Tauri state, a tokio runtime, or the filesystem.
fn build_character_list(
    addon_settings: Option<&str>,
    sv_chars: &[(String, Option<String>)],
) -> Vec<CharacterInfo> {
    let mut characters: Vec<CharacterInfo> = Vec::new();
    // (server, normalized name) already emitted from AddOnSettings.txt. Keyed on
    // the pair so the same name on two megaservers (NA + EU) is kept as two
    // distinct characters.
    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();
    // Normalized names already represented by ANY source. SavedVariables-only
    // characters dedup by name alone (they have no reliable server), so one
    // already listed from AddOnSettings is not re-listed under "Unknown".
    let mut known_names: HashSet<String> = HashSet::new();

    if let Some(content) = addon_settings {
        // Global metadata headers (and the account-wide sentinel `$AccountWide`,
        // handled separately) are not characters.
        let skip_prefixes = [
            "Version",
            "Acknowledged",
            "AddOnsEnabled",
            "LoadOutOfDateAddOns",
        ];
        for line in content.lines() {
            let Some(line) = line.strip_prefix('#') else {
                continue;
            };
            if line.starts_with('$') || skip_prefixes.iter().any(|p| line.starts_with(p)) {
                continue;
            }
            // Split on the FIRST '-': the server token ("NA Megaserver" etc.)
            // never contains '-', so the remainder is the full character name
            // even when the name itself contains '-' (e.g. "Jodynn-Jo").
            if let Some(pos) = line.find('-') {
                let server = line[..pos].trim().to_string();
                let name = line[pos + 1..].trim().to_string();
                if server.is_empty() || name.is_empty() {
                    continue;
                }
                let norm = normalize_character_name(&name);
                if seen_pairs.insert((server.clone(), norm.clone())) {
                    known_names.insert(norm);
                    characters.push(CharacterInfo {
                        server,
                        name,
                        recovered: false,
                    });
                }
            }
        }
    }

    // Normalize and classify SavedVariables-derived characters. Trust only a
    // canonical megaserver as a real server label; anything else is bucketed so
    // we never fabricate or mis-group a server.
    let mut recovered: Vec<(String, Option<String>)> = Vec::new();
    for (raw, world) in sv_chars {
        let name = normalize_character_name(raw);
        // Guard against markers / numeric character IDs leaking in as names.
        if name.is_empty() || name.starts_with('$') || name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let server = world
            .as_deref()
            .filter(|w| crate::saved_variables::scrub::WELL_KNOWN_WORLDS.contains(w))
            .map(str::to_string);
        recovered.push((name, server));
    }
    // Process known-megaserver characters first so that NA/EU twins of the same
    // name are both kept (distinct per server), while a later unknown-server
    // entry for an already-listed name is recognized as a duplicate and dropped.
    recovered.sort_by_key(|(_, server)| server.is_none());

    for (name, server) in recovered {
        match server {
            // Known megaserver: a distinct character per (server, name).
            Some(server) => {
                if seen_pairs.insert((server.clone(), name.clone())) {
                    known_names.insert(name.clone());
                    characters.push(CharacterInfo {
                        server,
                        name,
                        recovered: true,
                    });
                }
            }
            // Unknown server: dedup by name against every source, since we can't
            // tell it apart from a same-named character already listed.
            None => {
                if known_names.insert(name.clone()) {
                    characters.push(CharacterInfo {
                        server: UNKNOWN_SERVER.to_string(),
                        name,
                        recovered: true,
                    });
                }
            }
        }
    }

    characters
}

#[tauri::command]
pub async fn list_characters(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<CharacterRoster, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;

    tokio::task::spawn_blocking(move || -> Result<CharacterRoster, String> {
        // Source 1: AddOnSettings.txt (authoritative server, may be missing or,
        // in account-wide mode, hold no per-character headers at all). A missing
        // file is an expected fallback; any other read error (permissions, bad
        // encoding) is surfaced rather than silently degrading the roster.
        let addon_settings = match addons_dir.parent().map(|p| p.join("AddOnSettings.txt")) {
            Some(path) => match fs::read_to_string(&path) {
                Ok(content) => Some(content),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(format!("Failed to read AddOnSettings.txt: {e}")),
            },
            None => None,
        };

        // Source 2: a streaming, bounded-memory scan of the SavedVariables .lua
        // files (no size cap). Recovers characters that have no AddOnSettings.txt
        // block (the common account-wide case). `skipped` counts files the scan
        // couldn't fully read (I/O error or malformed structure).
        let (sv_chars, skipped) = collect_roster_characters(&addons_dir);

        Ok(CharacterRoster {
            characters: build_character_list(addon_settings.as_deref(), &sv_chars),
            skipped_files: skipped as u32,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Monotonic counter giving each character backup a unique staging/tombstone
/// directory so concurrent invocations (even with the same backup name) never
/// share scratch paths.
static BACKUP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Serializes the final swap so two backups targeting the same `char-<name>`
/// directory can't race on it (the per-call staging/tombstone handle the rest).
static BACKUP_FINALIZE_LOCK: Mutex<()> = Mutex::new(());

/// Serializes whole backup-surface commands (`create_backup`,
/// `restore_backup_safe`, `delete_backup`, `backup_character_settings`) against
/// each other, so e.g. a restore can't race a concurrent create/delete over the
/// same `kalpa-backups` folder. This is a *separate* lock from
/// `BACKUP_FINALIZE_LOCK` — that one is non-reentrant and already held by
/// `finalize_backup_replace`/`recover_orphaned_backups`, which are called from
/// inside some of these commands, so reusing it here would self-deadlock.
/// Lock ordering: callers always acquire `BACKUP_MUTATION_LOCK` first (for the
/// whole command) and only afterwards, while still holding it, acquire
/// `BACKUP_FINALIZE_LOCK` (via `recover_orphaned_backups`/
/// `finalize_backup_replace`/`purge_backup_scratch`) — never the reverse — so
/// the two locks can't deadlock each other.
static BACKUP_MUTATION_LOCK: Mutex<()> = Mutex::new(());

/// Extract `character_name`'s per-character subtree from each `.lua`
/// SavedVariables file and write a minimal, self-contained, restorable copy of
/// just that subtree into `staging` (same filename). `world` restricts
/// world-scoped subtrees to a single megaserver (`None` = take any, used for
/// `Unknown`-server recovered characters). Account-wide data and other
/// characters are excluded.
///
/// Returns `(matched, copied, last_err, worlds_spanned)` where `matched` is
/// files that yielded at least one subtree, `copied` is files whose minimal
/// copy was written, and `worlds_spanned` is the count of DISTINCT world-scoped
/// layers seen across every matched block (account-only data contributes 0).
/// This is only ever `> 1` for an Unknown-server backup (`world == None`),
/// which takes world-scoped subtrees from every megaserver since it has no
/// single world to filter to — a value `> 1` means a same-named twin on
/// another server was silently bundled into this backup.
/// Aborts with `Err` if any `.lua` file cannot be read, so an incomplete scan
/// never produces an incomplete backup that could replace a good one.
fn stage_character_subtrees(
    sv_dir: &Path,
    character_name: &str,
    world: Option<&str>,
    staging: &Path,
) -> Result<(u32, u32, Option<String>, u32), String> {
    use crate::saved_variables::char_backup::{
        build_backup_file, char_base, extract_character_blocks,
    };

    let base = char_base(character_name.as_bytes()).to_vec();

    let entries =
        fs::read_dir(sv_dir).map_err(|e| format!("Failed to read SavedVariables folder: {e}"))?;
    let mut matched: u32 = 0;
    let mut copied: u32 = 0;
    let mut last_err: Option<String> = None;
    // Distinct world-layer keys seen across every matched block. Only account-
    // keyed blocks (no world layer) leave this empty; a world-scoped block adds
    // its megaserver key.
    let mut worlds_seen: HashSet<Vec<u8>> = HashSet::new();

    for entry in entries {
        // Abort on a directory-enumeration error rather than silently omitting a
        // file (which could finalize an incomplete backup over a good one).
        let entry = entry.map_err(|e| format!("Failed to enumerate SavedVariables: {e}"))?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("lua") {
            continue;
        }
        let fname = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        // Read raw bytes (non-UTF8 safe) and abort on any read error.
        let bytes = fs::read(&path).map_err(|e| format!("Could not read {fname}: {e}"))?;
        let blocks = extract_character_blocks(&bytes, &base, world);
        if blocks.is_empty() {
            continue;
        }
        matched += 1;
        for block in &blocks {
            if let Some(w) = block.world_layer() {
                worlds_seen.insert(w.to_vec());
            }
        }
        match build_backup_file(&blocks) {
            Ok(content) => match fs::write(staging.join(&fname), &content) {
                Ok(_) => copied += 1,
                Err(e) => last_err = Some(e.to_string()),
            },
            // A subtree we couldn't safely represent/validate: leave copied <
            // matched so the caller fails closed rather than installing a
            // partial backup.
            Err(e) => last_err = Some(e),
        }
    }

    Ok((matched, copied, last_err, worlds_seen.len() as u32))
}

/// Atomically replace `final_dir` with `staging`, preserving the previous
/// contents of `final_dir` if the swap fails. All three paths must live on the
/// same volume. On success, `final_dir` holds the staged content and any prior
/// backup is gone; on failure, the prior `final_dir` (if any) is left intact and
/// `staging` is removed. Crucially, the existing backup is only deleted *after*
/// the new one is installed, so a finalization error never loses the last
/// known-good backup. The swap is serialized so concurrent backups of the same
/// name can't race on `final_dir`.
fn finalize_backup_replace(
    staging: &Path,
    final_dir: &Path,
    tombstone: &Path,
) -> std::io::Result<()> {
    let _guard = BACKUP_FINALIZE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let had_previous = final_dir.exists();
    if had_previous {
        // Move the existing backup aside (don't delete it yet).
        let _ = fs::remove_dir_all(tombstone);
        fs::rename(final_dir, tombstone)?;
    }
    match fs::rename(staging, final_dir) {
        Ok(()) => {
            if had_previous {
                let _ = fs::remove_dir_all(tombstone);
            }
            Ok(())
        }
        Err(e) => {
            // Roll back: restore the previous backup. Only discard staging once
            // the prior backup is safely back at `final_dir`. If the restore
            // itself fails, PRESERVE both the tombstone (the only good copy) and
            // staging, so `recover_orphaned_backups` sees the staging and treats
            // the tombstone as a recoverable crash state rather than a deletion.
            if had_previous {
                match fs::rename(tombstone, final_dir) {
                    Ok(()) => {
                        let _ = fs::remove_dir_all(staging);
                    }
                    Err(_) => return Err(e),
                }
            } else {
                let _ = fs::remove_dir_all(staging);
            }
            Err(e)
        }
    }
}

/// Recover character backups orphaned by a crash/power-loss during finalization.
///
/// `finalize_backup_replace` moves the prior backup to `.old-char-<name>-<seq>`,
/// then installs staging at `char-<name>`, then removes the tombstone. A tombstone
/// is only restored when it is a TRUE mid-finalize crash — proven by the matching
/// `.tmp-char-<name>-<seq>` staging dir still being present (the install rename
/// hadn't happened yet). Otherwise the tombstone is stale (the swap completed, or
/// the user later deleted the visible backup) and is discarded, so a deleted
/// backup never resurrects even if an earlier cleanup failed. Holds the finalize
/// lock so it never observes a live, mid-swap finalize.
fn recover_orphaned_backups(backups_root: &Path) {
    let _guard = BACKUP_FINALIZE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entries = match fs::read_dir(backups_root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(rest) = name.strip_prefix(".old-char-") else {
            continue;
        };
        // rest is "<backup_name>-<seq>"; the seq is the trailing numeric segment.
        let Some((base, seq)) = rest.rsplit_once('-') else {
            continue;
        };
        if base.is_empty() {
            continue;
        }
        let final_dir = backups_root.join(format!("char-{base}"));
        let staging = backups_root.join(format!(".tmp-char-{base}-{seq}"));
        if final_dir.exists() {
            // The replacement already landed; the tombstone is stale.
            let _ = fs::remove_dir_all(&path);
        } else if staging.exists() {
            // True crash between the two finalize renames (staging never got
            // installed) — restore the prior backup. Restore FIRST and only drop
            // the dead attempt's staging once that succeeds; if the restore fails,
            // leave both so the staging proof survives for the next recovery pass
            // rather than degrading into a "stale, delete" state.
            if fs::rename(&path, &final_dir).is_ok() {
                let _ = fs::remove_dir_all(&staging);
            }
        } else {
            // No final dir AND no staging: not a mid-finalize crash. The backup
            // was installed and then deleted — discard the tombstone rather than
            // resurrecting a backup the user removed.
            let _ = fs::remove_dir_all(&path);
        }
    }
}

/// Marker file written inside every character backup directory to label it as
/// one. Dot-prefixed so it is excluded from backup file counts and restores.
/// `char_backup_replaceable` keys off this exact FILENAME, so every character
/// backup — legacy whole-file OR new per-character — writes it. The marker's
/// CONTENT distinguishes the format (see [`CHAR_BACKUP_MARKER_V2_BODY`]).
const CHAR_BACKUP_MARKER: &str = ".kalpa-char-backup";

/// Marker file CONTENT written by the per-character (subtree) backup format. A
/// legacy whole-file character backup wrote `"kalpa character backup\n"` (no
/// version). The version lives in the marker — which is always present and is the
/// same file `char_backup_replaceable` already requires — so a per-character
/// backup is positively identifiable for restore even if its `.json` metadata
/// sidecar is later lost (in which case restore fails closed rather than
/// whole-file-copying minimal subtree files over live data).
const CHAR_BACKUP_MARKER_V2_BODY: &[u8] = b"kalpa character backup v2\n";

/// Prefix that identifies a per-character (v2+) marker body.
const CHAR_BACKUP_MARKER_V2_PREFIX: &[u8] = b"kalpa character backup v2";

/// Metadata sidecar written by the per-character (subtree) backup format. Carries
/// the data restore needs to re-extract and merge the stored subtrees. Restore
/// requires both the v2 marker AND a valid sidecar at the current version.
/// Dot-prefixed so it stays out of `list_backups` counts and file restores.
const CHAR_BACKUP_META: &str = ".kalpa-char-backup.json";

/// Format version for the per-character backup metadata. Bump if the on-disk
/// representation changes incompatibly.
const CHAR_BACKUP_VERSION: u32 = 2;

/// Sidecar metadata for a per-character (subtree) backup.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CharBackupMeta {
    /// Format version (currently [`CHAR_BACKUP_VERSION`]).
    version: u32,
    /// The character's normalized (caret-stripped) name — used to re-extract the
    /// stored subtrees on restore.
    character: String,
    /// The character's megaserver as shown in the UI (a known megaserver, or
    /// `Unknown`); informational/display only.
    server: String,
    /// Count of DISTINCT world-scoped layers this backup's subtrees span (see
    /// `stage_character_subtrees`). Always `<= 1` for a known-server backup
    /// (isolated by `world`); a value `> 1` only occurs for an Unknown-server
    /// backup and means a same-named twin on another megaserver was silently
    /// bundled in. Additive field: `#[serde(default)]` so older backups
    /// without it deserialize as `0` (no false warning).
    #[serde(default)]
    worlds_spanned: u32,
}

/// How `restore_backup_safe` should restore a backup directory.
enum CharRestoreMode {
    /// A per-character (v2) backup with valid metadata: MERGE its subtrees.
    Merge(CharBackupMeta),
    /// A manual / auto-before-restore / legacy whole-file backup: copy whole files.
    WholeFile,
    /// A per-character backup we can't restore safely (missing/unsupported
    /// metadata): refuse rather than risk corrupting live data.
    Refuse(String),
}

/// Classify a backup directory for restore. A directory is treated as a
/// per-character (v2) backup when its marker carries the v2 body OR a metadata
/// sidecar is present; such a directory is restored by MERGE only with a valid,
/// current-version sidecar — otherwise it is refused (never whole-file-copied, as
/// that would splat minimal subtree files over live SavedVariables). Everything
/// else (manual, auto-before-restore, legacy whole-file character backups) is
/// restored by whole-file copy.
///
/// File-access ERRORS are never collapsed with absence: if the marker or the
/// sidecar exists but can't be read, the directory is REFUSED rather than risk
/// taking the whole-file path on what might be a per-character backup.
fn classify_backup_for_restore(backup_path: &Path) -> CharRestoreMode {
    use std::io::ErrorKind::NotFound;

    let refuse = |msg: &str| CharRestoreMode::Refuse(msg.to_string());

    // Read the marker, distinguishing "confirmed absent" (NotFound) from a read
    // error (present but unreadable). Never collapse the two — a read error on a
    // possible per-character backup must fail closed.
    let marker = match fs::read(backup_path.join(CHAR_BACKUP_MARKER)) {
        Ok(c) => Some(c),
        Err(e) if e.kind() == NotFound => None,
        Err(_) => {
            return refuse(
                "This character backup's marker is present but unreadable, so it \
                 can't be restored safely.",
            )
        }
    };
    let v2_marker = marker
        .as_deref()
        .is_some_and(|c| c.starts_with(CHAR_BACKUP_MARKER_V2_PREFIX));

    // Read the metadata sidecar directly (no `exists()`/`try_exists()` probe,
    // which would collapse access errors into "absent"). Classify on the result:
    match fs::read(backup_path.join(CHAR_BACKUP_META)) {
        Ok(bytes) => match serde_json::from_slice::<CharBackupMeta>(&bytes) {
            Ok(m) if m.version == CHAR_BACKUP_VERSION => CharRestoreMode::Merge(m),
            Ok(m) => CharRestoreMode::Refuse(format!(
                "This character backup uses an unsupported format (version {}). \
                 Update Kalpa to restore it.",
                m.version
            )),
            Err(_) => refuse(
                "This character backup's metadata is corrupt, so it can't be \
                 restored safely.",
            ),
        },
        // Metadata CONFIRMED absent: a per-character (v2) marker without it is a
        // degraded backup we must refuse; otherwise there is no per-character
        // signal at all → manual / auto / legacy whole-file backup.
        Err(e) if e.kind() == NotFound => {
            if v2_marker {
                refuse(
                    "This character backup is missing its metadata, so it can't be \
                     restored safely.",
                )
            } else {
                CharRestoreMode::WholeFile
            }
        }
        // Metadata present but unreadable → fail closed.
        Err(_) => refuse(
            "This character backup's metadata is present but unreadable, so it \
             can't be restored safely.",
        ),
    }
}

/// Whether a character backup may be installed at `final_dir` by replacing
/// whatever is there: true only when the path is absent or is a marked character
/// backup. An existing UNMARKED directory is ambiguous (it could be a legacy
/// character backup OR an old manual backup that used the now-reserved `char-`
/// prefix), so it is never replaced — `resolve_char_backup_name` routes around it
/// to a fresh numbered name instead, preserving it.
fn char_backup_replaceable(final_dir: &Path) -> bool {
    !final_dir.exists() || final_dir.join(CHAR_BACKUP_MARKER).is_file()
}

/// Choose the directory name (without the `char-` prefix) for a new character
/// backup of `backup_name`: the requested name when its `char-*` dir is free or
/// an existing marked character backup (refresh in place), otherwise the first
/// numbered sibling (`<name>-2`, `-3`, …) that is. Returns `None` only if an
/// absurd number are taken. This never selects an unmarked directory, so a legacy
/// or manual `char-*` backup is preserved rather than silently overwritten, while
/// the backup still succeeds under a new name.
fn resolve_char_backup_name(backups_root: &Path, backup_name: &str) -> Option<String> {
    if char_backup_replaceable(&backups_root.join(format!("char-{backup_name}"))) {
        return Some(backup_name.to_string());
    }
    (2..=999).find_map(|n| {
        let candidate = format!("{backup_name}-{n}");
        char_backup_replaceable(&backups_root.join(format!("char-{candidate}")))
            .then_some(candidate)
    })
}

/// Result of [`backup_character_settings`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharBackupResult {
    /// Files copied into the backup (previously the whole return value).
    pub restored_files: u32,
    /// Distinct megaservers this backup's world-scoped subtrees span. `> 1`
    /// only for an Unknown-server character, and means a same-named twin on
    /// another server was silently bundled in — the caller should warn.
    pub worlds_spanned: u32,
}

#[tauri::command]
pub async fn backup_character_settings(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    character_name: String,
    server: String,
    backup_name: String,
) -> Result<CharBackupResult, String> {
    if character_name.trim().is_empty() {
        return Err("Character name cannot be empty.".to_string());
    }
    if character_name.len() < 3 {
        return Err("Character name must be at least 3 characters.".to_string());
    }
    validate_name(&backup_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        let sv_dir = saved_variables_dir(&addons_dir);
        if !sv_dir.is_dir() {
            return Err("SavedVariables folder not found.".to_string());
        }

        // Serialize against every other backup-surface command (create/restore/
        // delete) for the whole operation. Acquired before `BACKUP_FINALIZE_LOCK`
        // (taken below by `recover_orphaned_backups`/`finalize_backup_replace`) to
        // keep lock ordering consistent.
        let _mutation_guard = BACKUP_MUTATION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Restrict world-scoped subtrees to this character's megaserver so a same-name
        // NA/EU twin is backed up independently. A non-megaserver `server` (the
        // `Unknown` recovered bucket) means we can't isolate by world, so we take any
        // world-scoped occurrence (and account-keyed data, which carries no world).
        let world: Option<&str> =
            if crate::saved_variables::scrub::WELL_KNOWN_WORLDS.contains(&server.as_str()) {
                Some(server.as_str())
            } else {
                None
            };

        // Stage into a dot-prefixed temp dir on the same volume, then atomically
        // rename into place only after every matched file copies. This keeps a
        // failed/partial backup from leaving restorable state and replaces any prior
        // backup of the same name wholesale rather than mixing into it. A per-call
        // sequence number makes the staging/tombstone unique so concurrent backups
        // don't collide; the `.` prefix keeps them out of `list_backups`.
        let backups_root = backups_dir(&addons_dir);
        // Recover any crash-orphaned backup BEFORE touching scratch paths. The
        // sequence counter resets on restart, so a retried backup of the same name
        // could otherwise reuse a crash leftover's staging name and destroy the proof
        // that recovery relies on. Running recovery first restores/cleans the leftover
        // before this attempt can collide with it.
        recover_orphaned_backups(&backups_root);

        // Pick a target that is free or an existing marked character backup; route
        // around an unmarked `char-*` directory (legacy/manual) to a numbered name so
        // it is preserved, never silently overwritten.
        let effective_name =
            resolve_char_backup_name(&backups_root, &backup_name).ok_or_else(|| {
                "Too many existing backups with this name; please choose a different name."
                    .to_string()
            })?;
        let final_dir = backups_root.join(format!("char-{effective_name}"));
        let seq = BACKUP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let staging = backups_root.join(format!(".tmp-char-{effective_name}-{seq}"));
        let tombstone = backups_root.join(format!(".old-char-{effective_name}-{seq}"));
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir_all(&staging).map_err(|e| format!("Failed to create backup folder: {e}"))?;

        // Extract + stage just this character's per-character subtree(s) from each
        // file. Aborts if any file can't be read, so an incomplete scan never
        // silently produces (or installs) an incomplete backup.
        let (matched, copied, last_copy_err, worlds_spanned) =
            match stage_character_subtrees(&sv_dir, &character_name, world, &staging) {
                Ok(counts) => counts,
                Err(e) => {
                    let _ = fs::remove_dir_all(&staging);
                    return Err(format!(
                        "Could not read all SavedVariables files while backing up \
                         \"{character_name}\" ({e}). Backup aborted to avoid an incomplete \
                         copy; close ESO and try again."
                    ));
                }
            };

        if matched == 0 {
            // No file held this character's per-character data — discard staging and
            // don't report success. A character with only account-wide addon settings
            // (or, for a known server, data only under the OTHER megaserver) has no
            // per-character subtree to copy.
            let _ = fs::remove_dir_all(&staging);
            return Err(format!(
                "No per-character SavedVariables data found for \"{character_name}\". \
                 This character may only use account-wide addon settings."
            ));
        }

        if copied < matched {
            // A subtree matched but couldn't be written or safely represented —
            // discard the partial staging dir and surface it instead of leaving a
            // restorable incomplete backup.
            let _ = fs::remove_dir_all(&staging);
            let detail = last_copy_err.map(|e| format!(" ({e})")).unwrap_or_default();
            return Err(format!(
                "Backed up only {copied} of {matched} SavedVariables files for \
                 \"{character_name}\"; some files could not be saved{detail}."
            ));
        }

        // Stamp the staged dir as a per-character (v2) backup. The versioned marker
        // body positively identifies the format for restore (independent of the JSON
        // sidecar), while the filename is what `char_backup_replaceable` checks.
        if let Err(e) = fs::write(staging.join(CHAR_BACKUP_MARKER), CHAR_BACKUP_MARKER_V2_BODY) {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("Failed to write backup marker: {e}"));
        }

        // Write the per-character metadata sidecar. Its presence routes restore
        // through the subtree-MERGE path (vs. the legacy whole-file copy).
        let meta = CharBackupMeta {
            version: CHAR_BACKUP_VERSION,
            character: character_name.clone(),
            server: server.clone(),
            worlds_spanned,
        };
        match serde_json::to_vec_pretty(&meta) {
            Ok(json) => {
                if let Err(e) = fs::write(staging.join(CHAR_BACKUP_META), json) {
                    let _ = fs::remove_dir_all(&staging);
                    return Err(format!("Failed to write backup metadata: {e}"));
                }
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(format!("Failed to serialize backup metadata: {e}"));
            }
        }

        // All subtrees staged — install atomically, preserving any prior backup of
        // this name if finalization fails.
        finalize_backup_replace(&staging, &final_dir, &tombstone)
            .map_err(|e| format!("Failed to finalize backup: {e}"))?;

        Ok(CharBackupResult {
            restored_files: copied,
            worlds_spanned,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// ─── Minion Migration ────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MinionAddon {
    pub uid: u32,
    pub version: String,
    pub folders: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MinionMigrationResult {
    pub found: bool,
    pub addon_count: u32,
    pub imported: u32,
    pub already_tracked: u32,
}

pub fn find_minion_xml() -> Option<PathBuf> {
    if let Some(home) = dirs::home_dir() {
        let path = home.join(".minion").join("minion.xml");
        if path.exists() {
            return Some(path);
        }
    }
    None
}

pub fn parse_minion_addons(xml_content: &str) -> Vec<MinionAddon> {
    let mut addons: Vec<MinionAddon> = Vec::new();
    static RE_ADDON: OnceLock<Regex> = OnceLock::new();
    let re_addon = RE_ADDON.get_or_init(|| {
        Regex::new(r#"<addon[^>]*uid="(\d+)"[^>]*ui-version="([^"]*)"[^>]*>"#).unwrap()
    });
    static RE_DIR: OnceLock<Regex> = OnceLock::new();
    let re_dir = RE_DIR.get_or_init(|| Regex::new(r"<dir>([^<]+)</dir>").unwrap());

    // Simple state machine parser for Minion XML
    let mut current_uid: Option<u32> = None;
    let mut current_version = String::new();
    let mut current_dirs: Vec<String> = Vec::new();

    for line in xml_content.lines() {
        let line = line.trim();
        if let Some(caps) = re_addon.captures(line) {
            // Save previous addon if any
            if let Some(uid) = current_uid {
                if !current_dirs.is_empty() {
                    addons.push(MinionAddon {
                        uid,
                        version: current_version.clone(),
                        folders: current_dirs.clone(),
                    });
                }
            }
            current_uid = caps[1].parse::<u32>().ok();
            current_version = caps[2].to_string();
            current_dirs = Vec::new();
        } else if let Some(caps) = re_dir.captures(line) {
            current_dirs.push(caps[1].to_string());
        } else if line.contains("</addon>") {
            if let Some(uid) = current_uid {
                if !current_dirs.is_empty() {
                    addons.push(MinionAddon {
                        uid,
                        version: current_version.clone(),
                        folders: current_dirs.clone(),
                    });
                }
            }
            current_uid = None;
            current_dirs = Vec::new();
        }
    }

    addons
}

#[tauri::command]
pub fn detect_minion() -> Result<bool, String> {
    Ok(find_minion_xml().is_some())
}

/// Legacy migration command — delegates to the safe_migration implementation
/// to avoid duplicating the import logic.
#[tauri::command]
pub async fn migrate_from_minion(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<MinionMigrationResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        let result = safe_migration::execute_migration(&addons_dir)?;
        Ok(MinionMigrationResult {
            found: true,
            addon_count: result.addon_count,
            imported: result.imported,
            already_tracked: result.already_tracked,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// ─── Safe Migration Commands ────────────────────────────────────────

use crate::safe_migration;

#[tauri::command]
pub fn migration_check_preconditions(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<safe_migration::PreconditionResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    Ok(safe_migration::check_preconditions(&addons_dir))
}

#[tauri::command]
pub async fn migration_create_snapshot(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    include_addons: bool,
) -> Result<safe_migration::SnapshotManifest, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        safe_migration::create_pre_migration_snapshot(&addons_dir, include_addons)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn migration_dry_run(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<safe_migration::DryRunResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || safe_migration::dry_run_migration(&addons_dir))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn migration_execute(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<safe_migration::MigrationResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || safe_migration::execute_migration(&addons_dir))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub fn migration_check_integrity(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<safe_migration::IntegrityResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    Ok(safe_migration::check_integrity(&addons_dir))
}

#[tauri::command]
pub fn list_snapshots(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<Vec<safe_migration::SnapshotManifest>, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    Ok(safe_migration::list_snapshots(&addons_dir))
}

#[tauri::command]
pub async fn restore_snapshot(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    snapshot_id: String,
) -> Result<u32, String> {
    validate_name(&snapshot_id)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || safe_migration::restore_snapshot(&addons_dir, &snapshot_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub fn delete_snapshot(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    snapshot_id: String,
) -> Result<(), String> {
    validate_name(&snapshot_id)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    safe_migration::delete_snapshot(&addons_dir, &snapshot_id)
}

#[tauri::command]
pub async fn create_pre_operation_snapshot(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    operation_label: String,
) -> Result<safe_migration::SnapshotManifest, String> {
    validate_name(&operation_label)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        safe_migration::create_pre_operation_snapshot(&addons_dir, &operation_label)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub fn read_ops_log(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<Vec<safe_migration::OpLogEntry>, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    Ok(safe_migration::read_ops_log(&addons_dir))
}

#[tauri::command]
pub async fn backup_minion_config(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<u32, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || safe_migration::backup_minion_config(&addons_dir))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

// ── Pack Hub API (kalpa-pack-hub) ──────────────────────────────────────────

/// Base URL for the dedicated Pack Hub worker.
fn pack_hub_url() -> &'static str {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| {
        std::env::var("PACK_HUB_API_URL")
            .unwrap_or_else(|_| "https://kalpa-pack-hub.eso-toolkit.workers.dev".to_string())
    })
}

/// Validate a pack ID to prevent path traversal in URL interpolation.
fn validate_pack_id(id: &str) -> Result<(), String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap());
    if id.is_empty() || id.len() > 100 || !re.is_match(id) {
        return Err("Invalid pack ID.".to_string());
    }
    Ok(())
}

fn pack_hub_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .user_agent(format!("Kalpa/{}", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("failed to build pack hub HTTP client")
    })
}

// ── Pack structs (matching kalpa-pack-hub response) ───────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackAddonEntry {
    pub esoui_id: u32,
    pub name: String,
    #[serde(default = "default_true")]
    pub required: bool,
    pub note: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Full pack object returned by kalpa-pack-hub.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HubPack {
    pub id: String,
    #[serde(default)]
    pub author_id: String,
    pub author_name: String,
    pub is_anonymous: bool,
    pub title: String,
    pub description: String,
    pub pack_type: String,
    pub addons: serde_json::Value, // JSON string from D1 or parsed array
    pub vote_count: i64,
    #[serde(default)]
    pub install_count: i64,
    pub created_at: String,
    pub updated_at: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub user_voted: Option<bool>,
    #[serde(default)]
    pub status: Option<String>,
}

/// Frontend-friendly pack struct sent to the webview.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pack {
    pub id: String,
    pub author_id: String,
    pub title: String,
    pub description: String,
    pub pack_type: String,
    pub author_name: String,
    pub is_anonymous: bool,
    pub vote_count: i64,
    pub install_count: i64,
    pub user_voted: bool,
    pub tags: Vec<String>,
    pub addons: Vec<PackAddonEntry>,
    pub created_at: String,
    pub updated_at: String,
    pub status: String,
}

impl Pack {
    fn from_hub(hub: HubPack) -> Self {
        let addons: Vec<PackAddonEntry> = match &hub.addons {
            serde_json::Value::String(s) => serde_json::from_str(s).unwrap_or_else(|e| {
                eprintln!(
                    "Warning: failed to parse addons JSON string for pack {}: {}",
                    hub.id, e
                );
                Vec::new()
            }),
            serde_json::Value::Array(_) => serde_json::from_value(hub.addons.clone())
                .unwrap_or_else(|e| {
                    eprintln!(
                        "Warning: failed to parse addons array for pack {}: {}",
                        hub.id, e
                    );
                    Vec::new()
                }),
            _ => {
                eprintln!(
                    "Warning: unexpected addons type for pack {}: {}",
                    hub.id, hub.addons
                );
                Vec::new()
            }
        };
        Self {
            id: hub.id,
            author_id: hub.author_id,
            title: hub.title,
            description: hub.description,
            pack_type: hub.pack_type,
            author_name: if hub.is_anonymous {
                "Anonymous".to_string()
            } else {
                hub.author_name
            },
            is_anonymous: hub.is_anonymous,
            vote_count: hub.vote_count,
            install_count: hub.install_count,
            user_voted: hub.user_voted.unwrap_or(false),
            tags: hub.tags,
            addons,
            created_at: hub.created_at,
            updated_at: hub.updated_at,
            status: hub.status.unwrap_or_else(|| "published".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackListResponse {
    pub packs: Vec<HubPack>,
    pub page: i64,
    pub sort: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackSingleResponse {
    pub pack: HubPack,
}

/// Response sent to the frontend with packs and the current page number.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackPage {
    pub packs: Vec<Pack>,
    pub page: i64,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn list_packs(
    state: tauri::State<'_, AuthState>,
    pack_type: Option<String>,
    tag: Option<String>,
    query: Option<String>,
    sort: Option<String>,
    page: Option<i64>,
    author: Option<String>,
    status: Option<String>,
) -> Result<PackPage, String> {
    let access_token = get_current_token(&state);

    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/packs");

        let mut query_params: Vec<(&str, String)> = Vec::new();
        if let Some(t) = &pack_type {
            query_params.push(("type", t.clone()));
        }
        if let Some(t) = &tag {
            query_params.push(("tag", t.clone()));
        }
        if let Some(q) = &query {
            query_params.push(("q", q.clone()));
        }
        if let Some(s) = &sort {
            query_params.push(("sort", s.clone()));
        }
        if let Some(p) = &page {
            query_params.push(("page", p.to_string()));
        }
        if let Some(a) = &author {
            query_params.push(("author", a.clone()));
        }
        if let Some(st) = &status {
            query_params.push(("status", st.clone()));
        }

        let mut req = client.get(&url).query(&query_params);
        if let Some(token) = &access_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let response = req.send().map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                "Could not connect to Pack Hub. Check your internet connection.".to_string()
            } else {
                format!("Network error: {e}")
            }
        })?;

        if !response.status().is_success() {
            return Err(format!("Pack Hub returned HTTP {}", response.status()));
        }

        let body: PackListResponse = response
            .json()
            .map_err(|e| format!("Failed to parse packs response: {e}"))?;

        Ok(PackPage {
            packs: body.packs.into_iter().map(Pack::from_hub).collect(),
            page: body.page,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn get_pack(state: tauri::State<'_, AuthState>, id: String) -> Result<Pack, String> {
    validate_pack_id(&id)?;
    let access_token = get_current_token(&state);

    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/packs/{id}");

        let mut req = client.get(&url);
        if let Some(token) = &access_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let response = req.send().map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                "Could not connect to Pack Hub. Check your internet connection.".to_string()
            } else {
                format!("Network error: {e}")
            }
        })?;

        match response.status().as_u16() {
            200 => {}
            404 => return Err(format!("Pack \"{id}\" not found.")),
            status => return Err(format!("Pack Hub returned HTTP {status}")),
        }

        let body: PackSingleResponse = response
            .json()
            .map_err(|e| format!("Failed to parse pack response: {e}"))?;

        Ok(Pack::from_hub(body.pack))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Extract the current access token from auth state (if signed in).
fn get_current_token(state: &tauri::State<'_, AuthState>) -> Option<String> {
    state
        .tokens
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|t| t.access_token.clone()))
}

// ── Vote response from the hub API ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoteResponse {
    pub voted: bool,
    pub vote_count: i64,
}

#[tauri::command]
pub async fn vote_pack(
    state: tauri::State<'_, AuthState>,
    app: tauri::AppHandle,
    pack_id: String,
) -> Result<VoteResponse, String> {
    validate_pack_id(&pack_id)?;
    // Get current access token (refresh if needed)
    let access_token = {
        let tokens = {
            let guard = state
                .tokens
                .lock()
                .map_err(|e| format!("Auth lock poisoned: {e}"))?;
            guard.clone()
        };

        let Some(tokens) = tokens else {
            return Err("Sign in to vote on packs.".to_string());
        };

        match tokio::task::spawn_blocking({
            let tokens = tokens.clone();
            move || auth::ensure_valid_token(&tokens)
        })
        .await
        .map_err(|e| format!("Task failed: {e}"))?
        {
            Ok(Some(new_tokens)) => {
                let token = new_tokens.access_token.clone();
                // Persistence failure is logged in the helper and keeps the
                // refreshed token working in-memory; don't fail the refresh.
                let _ = save_auth_tokens(&app, &new_tokens);
                *state
                    .tokens
                    .lock()
                    .map_err(|e| format!("Auth lock poisoned: {e}"))? = Some(new_tokens);
                token
            }
            Ok(None) => tokens.access_token.clone(),
            Err(e) => {
                *state
                    .tokens
                    .lock()
                    .map_err(|e| format!("Auth lock poisoned: {e}"))? = None;
                return Err(e);
            }
        }
    };

    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/packs/{pack_id}/vote");

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    "Could not connect to Pack Hub. Check your internet connection.".to_string()
                } else {
                    format!("Network error: {e}")
                }
            })?;

        match response.status().as_u16() {
            200 => {}
            401 => return Err("Session expired. Please sign in again.".to_string()),
            404 => return Err("Pack not found.".to_string()),
            429 => return Err("Too many votes. Please wait a moment.".to_string()),
            status => {
                let body = response.text().unwrap_or_default();
                return Err(format!("Pack Hub returned HTTP {status} — {body}"));
            }
        }

        let body: VoteResponse = response
            .json()
            .map_err(|e| format!("Failed to parse vote response: {e}"))?;

        Ok(body)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// ── Track pack install ──────────────────────────────────────────────────

#[tauri::command]
pub async fn track_pack_install(pack_id: String) -> Result<(), String> {
    validate_pack_id(&pack_id)?;

    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/packs/{pack_id}/install");

        // Fire-and-forget: best-effort tracking, don't block the user
        drop(client.post(&url).send());

        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// ── Auth Helpers ─────────────────────────────────────────────────────────

/// Persist auth tokens, returning whether they were durably committed to the
/// credential store. A `false` means the tokens are **memory-only** for this
/// process (a Credential Manager failure) and the user may need to sign in again
/// next launch — it is logged here so the failure is never silently swallowed,
/// and the bool is returned so callers establishing auth (e.g. `auth_login`) can
/// surface it. Refresh paths intentionally keep working in-memory on a `false`
/// (the live token is still usable this session); they should not hard-fail the
/// in-flight operation just because persistence hiccuped.
#[must_use]
fn save_auth_tokens(_app: &tauri::AppHandle, tokens: &AuthTokens) -> bool {
    let persisted = crate::token_store::save_tokens(tokens);
    if !persisted {
        eprintln!(
            "[auth] WARNING: failed to persist auth tokens to the credential store; \
             the session is memory-only and will not survive a restart."
        );
    }
    persisted
}

fn clear_auth_tokens(_app: &tauri::AppHandle) {
    crate::token_store::clear_tokens();
}

fn clear_upload_session(upload_session: &Arc<StoredSessionProvider>) {
    upload_session.invalidate();
}

fn clear_auth_and_upload_sessions(
    app: &tauri::AppHandle,
    upload_session: &Arc<StoredSessionProvider>,
) {
    clear_auth_tokens(app);
    clear_upload_session(upload_session);
}

// ── Auth Commands ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn auth_login(
    state: tauri::State<'_, AuthState>,
    app: tauri::AppHandle,
    upload_session: tauri::State<'_, Arc<StoredSessionProvider>>,
) -> Result<AuthUser, String> {
    let tokens = tokio::task::spawn_blocking(auth::login)
        .await
        .map_err(|e| format!("Task failed: {e}"))??;

    // Save to store first so the login response can report durability. A failure
    // is logged in the helper and leaves the session memory-only (still usable
    // this process), so we do NOT fail the login — instead we surface
    // `sessionPersisted: false` to the UI so it can warn the user that they will
    // need to sign in again after a restart.
    let persisted = save_auth_tokens(&app, &tokens);

    let user = AuthUser {
        user_id: tokens.user_id.clone(),
        user_name: tokens.user_name.clone(),
        session_persisted: Some(persisted),
    };

    // Update in-memory state
    *state
        .tokens
        .lock()
        .map_err(|e| format!("Auth lock poisoned: {e}"))? = Some(tokens);

    // The native upload cookie is a separate website session and carries no
    // identity metadata here. Require a fresh upload login after an OAuth login
    // so direct uploads cannot silently reuse a prior account's cookie.
    clear_upload_session(&upload_session);

    Ok(user)
}

#[tauri::command]
pub async fn auth_logout(
    state: tauri::State<'_, AuthState>,
    app: tauri::AppHandle,
    upload_session: tauri::State<'_, Arc<StoredSessionProvider>>,
) -> Result<(), String> {
    // Clear in-memory state
    *state
        .tokens
        .lock()
        .map_err(|e| format!("Auth lock poisoned: {e}"))? = None;

    // Clear both credential families: OAuth tokens and the separate website
    // cookie used by direct ESO Logs uploads.
    clear_auth_and_upload_sessions(&app, &upload_session);

    Ok(())
}

#[tauri::command]
pub async fn auth_get_user(
    state: tauri::State<'_, AuthState>,
    app: tauri::AppHandle,
    upload_session: tauri::State<'_, Arc<StoredSessionProvider>>,
) -> Result<Option<AuthUser>, String> {
    let tokens = {
        let guard = state
            .tokens
            .lock()
            .map_err(|e| format!("Auth lock poisoned: {e}"))?;
        guard.clone()
    };

    let Some(tokens) = tokens else {
        return Ok(None);
    };

    // Check if token needs refresh
    match tokio::task::spawn_blocking({
        let tokens = tokens.clone();
        move || auth::ensure_valid_token(&tokens)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
    {
        Ok(Some(new_tokens)) => {
            // Persistence failure is logged in the helper; keep the refreshed
            // token working in-memory rather than failing the refresh. Report the
            // durability so a status check can also reflect a memory-only session.
            let persisted = save_auth_tokens(&app, &new_tokens);

            let user = AuthUser {
                user_id: new_tokens.user_id.clone(),
                user_name: new_tokens.user_name.clone(),
                session_persisted: Some(persisted),
            };

            *state
                .tokens
                .lock()
                .map_err(|e| format!("Auth lock poisoned: {e}"))? = Some(new_tokens);
            Ok(Some(user))
        }
        Ok(None) => {
            // Token still valid (no save happened) — durability unchanged/unknown.
            Ok(Some(AuthUser {
                user_id: tokens.user_id,
                user_name: tokens.user_name,
                session_persisted: None,
            }))
        }
        Err(_) => {
            // Refresh failed — clear session
            *state
                .tokens
                .lock()
                .map_err(|e| format!("Auth lock poisoned: {e}"))? = None;
            clear_auth_and_upload_sessions(&app, &upload_session);
            Ok(None)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePackPayload {
    pub title: String,
    pub description: String,
    pub pack_type: String,
    pub addons: Vec<PackAddonEntry>,
    pub tags: Vec<String>,
    pub is_anonymous: bool,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePackPayload {
    pub id: String,
    pub title: String,
    pub description: String,
    pub pack_type: String,
    pub addons: Vec<PackAddonEntry>,
    pub tags: Vec<String>,
    pub is_anonymous: bool,
    pub status: Option<String>,
}

#[tauri::command]
pub async fn create_pack(
    state: tauri::State<'_, AuthState>,
    app: tauri::AppHandle,
    payload: CreatePackPayload,
) -> Result<Pack, String> {
    // Get current access token (refresh if needed)
    let access_token = {
        let tokens = {
            let guard = state
                .tokens
                .lock()
                .map_err(|e| format!("Auth lock poisoned: {e}"))?;
            guard.clone()
        };

        let Some(tokens) = tokens else {
            return Err("Not signed in. Please sign in first.".to_string());
        };

        match tokio::task::spawn_blocking({
            let tokens = tokens.clone();
            move || auth::ensure_valid_token(&tokens)
        })
        .await
        .map_err(|e| format!("Task failed: {e}"))?
        {
            Ok(Some(new_tokens)) => {
                let token = new_tokens.access_token.clone();
                // Persistence failure is logged in the helper and keeps the
                // refreshed token working in-memory; don't fail the refresh.
                let _ = save_auth_tokens(&app, &new_tokens);
                *state
                    .tokens
                    .lock()
                    .map_err(|e| format!("Auth lock poisoned: {e}"))? = Some(new_tokens);
                token
            }
            Ok(None) => tokens.access_token.clone(),
            Err(e) => {
                *state
                    .tokens
                    .lock()
                    .map_err(|e| format!("Auth lock poisoned: {e}"))? = None;
                return Err(e);
            }
        }
    };

    // POST to Pack Hub API
    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/packs");

        let body = serde_json::json!({
            "title": payload.title,
            "description": payload.description,
            "pack_type": payload.pack_type,
            "addons": payload.addons,
            "tags": payload.tags,
            "is_anonymous": payload.is_anonymous,
            "status": payload.status.unwrap_or_else(|| "draft".to_string()),
        });

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    "Could not connect to Pack Hub. Check your internet connection.".to_string()
                } else {
                    format!("Network error: {e}")
                }
            })?;

        match response.status().as_u16() {
            200 | 201 => {}
            401 => return Err("Session expired. Please sign in again.".to_string()),
            429 => {
                return Err("Rate limit reached. Please wait before publishing again.".to_string())
            }
            status => {
                let body = response.text().unwrap_or_default();
                return Err(format!("Pack Hub returned HTTP {status} — {body}"));
            }
        }

        let body: PackSingleResponse = response
            .json()
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        Ok(Pack::from_hub(body.pack))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn update_pack(
    state: tauri::State<'_, AuthState>,
    app: tauri::AppHandle,
    payload: UpdatePackPayload,
) -> Result<Pack, String> {
    validate_pack_id(&payload.id)?;

    let access_token = {
        let tokens = {
            let guard = state
                .tokens
                .lock()
                .map_err(|e| format!("Auth lock poisoned: {e}"))?;
            guard.clone()
        };

        let Some(tokens) = tokens else {
            return Err("Not signed in. Please sign in first.".to_string());
        };

        match tokio::task::spawn_blocking({
            let tokens = tokens.clone();
            move || auth::ensure_valid_token(&tokens)
        })
        .await
        .map_err(|e| format!("Task failed: {e}"))?
        {
            Ok(Some(new_tokens)) => {
                let token = new_tokens.access_token.clone();
                // Persistence failure is logged in the helper and keeps the
                // refreshed token working in-memory; don't fail the refresh.
                let _ = save_auth_tokens(&app, &new_tokens);
                *state
                    .tokens
                    .lock()
                    .map_err(|e| format!("Auth lock poisoned: {e}"))? = Some(new_tokens);
                token
            }
            Ok(None) => tokens.access_token.clone(),
            Err(e) => {
                *state
                    .tokens
                    .lock()
                    .map_err(|e| format!("Auth lock poisoned: {e}"))? = None;
                return Err(e);
            }
        }
    };

    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{}/packs/{}", base, payload.id);

        let body = serde_json::json!({
            "title": payload.title,
            "description": payload.description,
            "pack_type": payload.pack_type,
            "addons": payload.addons,
            "tags": payload.tags,
            "is_anonymous": payload.is_anonymous,
            "status": payload.status,
        });

        let response = client
            .put(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    "Could not connect to Pack Hub. Check your internet connection.".to_string()
                } else {
                    format!("Network error: {e}")
                }
            })?;

        match response.status().as_u16() {
            200 => {}
            401 => return Err("Session expired. Please sign in again.".to_string()),
            403 => return Err("You can only edit packs you created.".to_string()),
            404 => return Err("Pack not found.".to_string()),
            status => {
                let body = response.text().unwrap_or_default();
                return Err(format!("Pack Hub returned HTTP {status} - {body}"));
            }
        }

        let body: PackSingleResponse = response
            .json()
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        Ok(Pack::from_hub(body.pack))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn delete_pack(
    state: tauri::State<'_, AuthState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    validate_pack_id(&id)?;

    let access_token = {
        let tokens = {
            let guard = state
                .tokens
                .lock()
                .map_err(|e| format!("Auth lock poisoned: {e}"))?;
            guard.clone()
        };

        let Some(tokens) = tokens else {
            return Err("Not signed in. Please sign in first.".to_string());
        };

        match tokio::task::spawn_blocking({
            let tokens = tokens.clone();
            move || auth::ensure_valid_token(&tokens)
        })
        .await
        .map_err(|e| format!("Task failed: {e}"))?
        {
            Ok(Some(new_tokens)) => {
                let token = new_tokens.access_token.clone();
                // Persistence failure is logged in the helper and keeps the
                // refreshed token working in-memory; don't fail the refresh.
                let _ = save_auth_tokens(&app, &new_tokens);
                *state
                    .tokens
                    .lock()
                    .map_err(|e| format!("Auth lock poisoned: {e}"))? = Some(new_tokens);
                token
            }
            Ok(None) => tokens.access_token.clone(),
            Err(e) => {
                *state
                    .tokens
                    .lock()
                    .map_err(|e| format!("Auth lock poisoned: {e}"))? = None;
                return Err(e);
            }
        }
    };

    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/packs/{id}");

        let response = client
            .delete(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    "Could not connect to Pack Hub. Check your internet connection.".to_string()
                } else {
                    format!("Network error: {e}")
                }
            })?;

        match response.status().as_u16() {
            200 => Ok(()),
            401 => Err("Session expired. Please sign in again.".to_string()),
            403 => Err("You can only delete packs you created.".to_string()),
            404 => Err("Pack not found.".to_string()),
            status => {
                let body = response.text().unwrap_or_default();
                Err(format!("Pack Hub returned HTTP {status} - {body}"))
            }
        }
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// ── Delete Account Data ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteAccountSummary {
    pub packs: u64,
    pub votes: u64,
    pub shares: u64,
}

#[tauri::command]
pub async fn delete_pack_hub_account(
    state: tauri::State<'_, AuthState>,
    app: tauri::AppHandle,
    upload_session: tauri::State<'_, Arc<StoredSessionProvider>>,
) -> Result<DeleteAccountSummary, String> {
    let access_token = {
        let tokens = {
            let guard = state
                .tokens
                .lock()
                .map_err(|e| format!("Auth lock poisoned: {e}"))?;
            guard.clone()
        };

        let Some(tokens) = tokens else {
            return Err("Not signed in. Please sign in first.".to_string());
        };

        match tokio::task::spawn_blocking({
            let tokens = tokens.clone();
            move || auth::ensure_valid_token(&tokens)
        })
        .await
        .map_err(|e| format!("Task failed: {e}"))?
        {
            Ok(Some(new_tokens)) => {
                let token = new_tokens.access_token.clone();
                // Persistence failure is logged in the helper and keeps the
                // refreshed token working in-memory; don't fail the refresh.
                let _ = save_auth_tokens(&app, &new_tokens);
                *state
                    .tokens
                    .lock()
                    .map_err(|e| format!("Auth lock poisoned: {e}"))? = Some(new_tokens);
                token
            }
            Ok(None) => tokens.access_token.clone(),
            Err(e) => {
                *state
                    .tokens
                    .lock()
                    .map_err(|e| format!("Auth lock poisoned: {e}"))? = None;
                return Err(e);
            }
        }
    };

    let result = tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/account");

        let response = client
            .delete(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    "Could not connect to Pack Hub. Check your internet connection.".to_string()
                } else {
                    format!("Network error: {e}")
                }
            })?;

        match response.status().as_u16() {
            200 => {
                #[derive(Deserialize)]
                struct Resp {
                    deleted: DeleteAccountSummary,
                }
                let body: Resp = response
                    .json()
                    .map_err(|e| format!("Invalid response: {e}"))?;

                Ok(body.deleted)
            }
            401 => Err("Session expired. Please sign in again.".to_string()),
            status => {
                let body = response.text().unwrap_or_default();
                Err(format!("Pack Hub returned HTTP {status} - {body}"))
            }
        }
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))??;

    // Sign the user out after successful deletion
    *state
        .tokens
        .lock()
        .map_err(|e| format!("Auth lock poisoned: {e}"))? = None;
    clear_auth_and_upload_sessions(&app, &upload_session);

    Ok(result)
}

// ── Private Sharing ─────────────────────────────────────────────────────────

/// Base URL for the share worker (separate from the pack hub).
fn share_worker_url() -> &'static str {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| {
        std::env::var("SHARE_WORKER_URL")
            .unwrap_or_else(|_| "https://kalpa-pack-hub.eso-toolkit.workers.dev".to_string())
    })
}

fn share_worker_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .user_agent(format!("Kalpa/{}", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("failed to build share worker HTTP client")
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharePackPayload {
    pub title: String,
    pub description: String,
    pub pack_type: String,
    pub tags: Vec<String>,
    pub addons: Vec<PackAddonEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareCodeResponse {
    pub code: String,
    pub expires_at: String,
    pub deep_link: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedPack {
    pub title: String,
    pub description: String,
    pub pack_type: String,
    pub tags: Vec<String>,
    pub addons: Vec<PackAddonEntry>,
    pub shared_by: String,
    pub shared_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShareResolveResponse {
    pack: ShareResolvedPack,
    shared_by: String,
    shared_at: String,
    expires_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShareResolvedPack {
    title: String,
    description: String,
    pack_type: String,
    tags: Vec<String>,
    addons: Vec<PackAddonEntry>,
}

/// Validate a share code (6 chars from the unambiguous alphabet).
fn validate_share_code(code: &str) -> Result<(), String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^[23456789ABCDEFGHJKMNPQRSTUVWXYZ]{6}$").unwrap());
    if !re.is_match(code) {
        return Err("Invalid share code format.".to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn create_share_code(
    state: tauri::State<'_, AuthState>,
    app: tauri::AppHandle,
    payload: SharePackPayload,
) -> Result<ShareCodeResponse, String> {
    let access_token = {
        let tokens = {
            let guard = state
                .tokens
                .lock()
                .map_err(|e| format!("Auth lock poisoned: {e}"))?;
            guard.clone()
        };

        let Some(tokens) = tokens else {
            return Err("Not signed in. Please sign in first.".to_string());
        };

        match tokio::task::spawn_blocking({
            let tokens = tokens.clone();
            move || auth::ensure_valid_token(&tokens)
        })
        .await
        .map_err(|e| format!("Task failed: {e}"))?
        {
            Ok(Some(new_tokens)) => {
                let token = new_tokens.access_token.clone();
                // Persistence failure is logged in the helper and keeps the
                // refreshed token working in-memory; don't fail the refresh.
                let _ = save_auth_tokens(&app, &new_tokens);
                *state
                    .tokens
                    .lock()
                    .map_err(|e| format!("Auth lock poisoned: {e}"))? = Some(new_tokens);
                token
            }
            Ok(None) => tokens.access_token.clone(),
            Err(e) => {
                *state
                    .tokens
                    .lock()
                    .map_err(|e| format!("Auth lock poisoned: {e}"))? = None;
                return Err(e);
            }
        }
    };

    tokio::task::spawn_blocking(move || {
        let client = share_worker_client();
        let url = format!("{}/shares", share_worker_url());

        let body = serde_json::json!({
            "title": payload.title,
            "description": payload.description,
            "packType": payload.pack_type,
            "tags": payload.tags,
            "addons": payload.addons,
        });

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    "Could not connect to share service. Check your internet connection."
                        .to_string()
                } else {
                    format!("Network error: {e}")
                }
            })?;

        match response.status().as_u16() {
            200 | 201 => {}
            401 => return Err("Session expired. Please sign in again.".to_string()),
            429 => {
                return Err(
                    "Maximum share codes reached. Wait for existing codes to expire.".to_string(),
                )
            }
            status => {
                let body = response.text().unwrap_or_default();
                return Err(format!("Share service returned HTTP {status} — {body}"));
            }
        }

        let result: ShareCodeResponse = response
            .json()
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        Ok(result)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn resolve_share_code(code: String) -> Result<SharedPack, String> {
    validate_share_code(&code)?;

    tokio::task::spawn_blocking(move || {
        let client = share_worker_client();
        let url = format!("{}/shares/{}", share_worker_url(), code);

        let response = client.get(&url).send().map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                "Could not connect to share service. Check your internet connection.".to_string()
            } else {
                format!("Network error: {e}")
            }
        })?;

        match response.status().as_u16() {
            200 => {}
            400 => return Err("Invalid share code format.".to_string()),
            404 => return Err("Share code not found or expired.".to_string()),
            status => {
                let body = response.text().unwrap_or_default();
                return Err(format!("Share service returned HTTP {status} — {body}"));
            }
        }

        let result: ShareResolveResponse = response
            .json()
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        Ok(SharedPack {
            title: result.pack.title,
            description: result.pack.description,
            pack_type: result.pack.pack_type,
            tags: result.pack.tags,
            addons: result.pack.addons,
            shared_by: result.shared_by,
            shared_at: result.shared_at,
            expires_at: result.expires_at,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// ── Pack Export / Import (.esopack files) ───────────────────────────────────

/// Scrubbed SavedVariables for one addon stored in an `.esopack` v2 file.
///
/// `encoding` is always `"lua-text"` for Phase 1. `lua` is the scrubbed Lua
/// source with identity placeholders in place of real names/IDs. `scrub_report`
/// is the full scrub report (drops + templated keys) for user review on import.
/// `detected_identities` captures the `ScrubContext` used during export so the
/// importer knows the placeholder → real-name mapping strategy.
///
/// `original_bytes` — serialized size before any scrubbing.
/// `scrubbed_bytes` — size after identity scrubbing (before per-character strip).
/// `final_bytes`    — actual size of `lua` after the per-character strip; this
///                    is the true exported footprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonSettings {
    pub encoding: String,
    pub lua: String,
    pub original_bytes: usize,
    pub scrubbed_bytes: usize,
    /// Byte length of the exported `lua` string — accurate post-strip size.
    /// Absent in files produced before this field was added; defaults to 0.
    #[serde(default)]
    pub final_bytes: usize,
    #[serde(default)]
    pub scrub_summary: crate::saved_variables::scrub::ScrubSummary,
    #[allow(dead_code)]
    #[serde(default, skip_serializing)]
    pub detected_identities: Option<crate::saved_variables::scrub::ScrubContext>,
    #[allow(dead_code)]
    #[serde(default, skip_serializing)]
    pub scrub_report: Option<crate::saved_variables::scrub::ScrubReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EsoPackFile {
    pub format: String,
    pub version: u32,
    pub pack: EsoPackData,
    pub shared_at: String,
    pub shared_by: String,
    /// v2 only: scrubbed SavedVariables keyed by addon folder name.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub settings: HashMap<String, AddonSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EsoPackData {
    pub title: String,
    pub description: String,
    pub pack_type: String,
    pub tags: Vec<String>,
    pub addons: Vec<PackAddonEntry>,
}

#[tauri::command]
pub fn export_pack_file(pack: EsoPackFile, path: String) -> Result<(), String> {
    let file_path = PathBuf::from(&path);

    if file_path.extension().and_then(|e| e.to_str()) != Some("esopack") {
        return Err("Export path must have .esopack extension.".to_string());
    }
    // Canonicalize the parent directory to prevent path traversal
    let parent = file_path
        .parent()
        .ok_or("Invalid file path: no parent directory.")?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|e| format!("Invalid directory: {e}"))?;
    let file_name = file_path
        .file_name()
        .ok_or("Invalid file path: no file name.")?;
    let file_path = canonical_parent.join(file_name);

    let json = serde_json::to_string_pretty(&pack)
        .map_err(|e| format!("Failed to serialize pack: {e}"))?;

    // Atomic write: write to .tmp then replace destination
    let tmp_path = file_path.with_extension("esopack.tmp");
    fs::write(&tmp_path, json).map_err(|e| format!("Failed to write file: {e}"))?;
    // On Windows, fs::rename fails if the destination exists. Remove it first.
    if file_path.exists() {
        fs::remove_file(&file_path).map_err(|e| {
            let _ = fs::remove_file(&tmp_path);
            format!("Failed to replace existing file: {e}")
        })?;
    }
    fs::rename(&tmp_path, &file_path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!("Failed to finalize write: {e}")
    })
}

#[tauri::command]
pub fn import_pack_file(path: String) -> Result<EsoPackFile, String> {
    let file_path = PathBuf::from(&path);

    if file_path.extension().and_then(|e| e.to_str()) != Some("esopack") {
        return Err("Only .esopack files can be imported.".to_string());
    }

    // Canonicalize to resolve any traversal components (also verifies existence)
    let file_path = file_path
        .canonicalize()
        .map_err(|_| "File not found.".to_string())?;

    let metadata = fs::metadata(&file_path).map_err(|e| format!("Failed to read file: {e}"))?;
    if metadata.len() > 10 * 1024 * 1024 {
        return Err("File is too large (max 10 MB).".to_string());
    }

    let contents =
        fs::read_to_string(&file_path).map_err(|e| format!("Failed to read file: {e}"))?;

    let pack: EsoPackFile =
        serde_json::from_str(&contents).map_err(|e| format!("Invalid .esopack file: {e}"))?;

    if pack.format != "esopack" {
        return Err("Not a valid .esopack file (wrong format field).".to_string());
    }

    if pack.version != 1 && pack.version != 2 {
        return Err(format!(
            "Unsupported .esopack version {}. Please update the app.",
            pack.version
        ));
    }

    Ok(pack)
}

/// Export the SavedVariables settings block for a list of addon folder names.
///
/// For each addon, reads the corresponding `.lua` file from the SavedVariables
/// directory, detects identities, scrubs the tree, and returns an `AddonSettings`
/// map keyed by addon folder name. Only `$AccountWide` subtrees are retained
/// (per-character data is not exported in Phase 1).
///
/// The caller merges this map into an `EsoPackFile` and writes it with
/// `export_pack_file`.
#[tauri::command]
pub async fn export_sv_settings(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    addon_folders: Vec<String>,
) -> Result<HashMap<String, AddonSettings>, String> {
    use crate::saved_variables::parser::parse_sv_file;
    use crate::saved_variables::scrub::{detect_identities_from_tree, scrub};
    use crate::saved_variables::serializer::serialize_to_lua;

    let addons_dir = require_allowed_path(&state, &addons_path)?;

    tokio::task::spawn_blocking(move || {
        let sv_dir = sv_io::saved_variables_dir(&addons_dir);
        let mut result: HashMap<String, AddonSettings> = HashMap::new();

        for folder in &addon_folders {
            let sv_file = sv_dir.join(format!("{folder}.lua"));
            if !sv_file.is_file() {
                continue;
            }

            let content = match fs::read_to_string(&sv_file) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "export_sv_settings: failed to read {}: {}",
                        sv_file.display(),
                        e
                    );
                    continue;
                }
            };

            let file_name = format!("{folder}.lua");
            let tree = match parse_sv_file(&content, &file_name) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("export_sv_settings: failed to parse {file_name}: {e}");
                    continue;
                }
            };

            let ctx = detect_identities_from_tree(&tree);
            // `scrub` consumes `tree` (mutating it in place); `ctx` was already
            // computed from it above, and it is not used afterwards.
            let (scrubbed, report) = scrub(tree, &ctx);

            let account_wide_only = strip_per_character_data(scrubbed);
            let lua = serialize_to_lua(&account_wide_only);
            let final_bytes = lua.len();

            result.insert(
                folder.clone(),
                AddonSettings {
                    encoding: "lua-text".to_string(),
                    lua,
                    original_bytes: report.original_bytes,
                    scrubbed_bytes: report.scrubbed_bytes,
                    final_bytes,
                    scrub_summary: (&report).into(),
                    detected_identities: None,
                    scrub_report: None,
                },
            );
        }

        Ok(result)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

fn strip_per_character_data(
    tree: crate::saved_variables::types::SvTreeNode,
) -> crate::saved_variables::types::SvTreeNode {
    crate::saved_variables::scrub::strip_per_character_data(tree)
}

/// Result of importing SavedVariables settings from a v2 `.esopack` file.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SvImportResult {
    /// Addons whose SV files were successfully written.
    pub applied: Vec<String>,
    /// Addons skipped because their SV file was not in the pack.
    pub skipped: Vec<String>,
    /// Addons where the import failed; contains error messages.
    pub errors: Vec<String>,
}

fn has_unresolved_identity_placeholders(lua: &str) -> bool {
    lua.contains("${ACCOUNT}")
        || lua.contains("${ACCOUNT:")
        || lua.contains("${ACCOUNT_NAME}")
        || lua.contains("${ACCOUNT_NAME:")
        || lua.contains("${CHAR:")
        || lua.contains("${CHAR_ID:")
}

/// Import SavedVariables settings from a v2 `.esopack` file.
///
/// For each addon in `addon_folders` that has a corresponding entry in the
/// pack's `settings` map, substitutes identity placeholders with the real
/// account/character identities from `ctx`, then writes the resulting Lua to
/// the SavedVariables directory. A `.bak` copy of the existing file is created
/// before each overwrite.
///
/// `ctx` must describe the *importer's* identities (not the exporter's). The
/// substitution maps:
///   `${ACCOUNT}` → `ctx.accounts[0]`
///   `${ACCOUNT:N}` → `ctx.accounts[N]`
///   `${CHAR:N}` → `ctx.characters[N]`
///   `${CHAR_ID:N}` → `ctx.character_ids[N]`
///   `${WORLD}` → first of `WELL_KNOWN_WORLDS` or `ctx.extra_worlds[0]`
///
/// Placeholder tokens that have no mapping in `ctx` are rejected — the
/// import is skipped and an error is returned for that addon.
#[tauri::command]
pub async fn import_sv_settings(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    settings: HashMap<String, AddonSettings>,
    ctx: crate::saved_variables::scrub::ScrubContext,
    addon_folders: Vec<String>,
) -> Result<SvImportResult, String> {
    use crate::saved_variables::parser::parse_sv_file;
    use crate::saved_variables::scrub::WELL_KNOWN_WORLDS;

    let addons_dir = require_allowed_path(&state, &addons_path)?;

    tokio::task::spawn_blocking(move || {
        let sv_dir = sv_io::saved_variables_dir(&addons_dir);
        fs::create_dir_all(&sv_dir)
            .map_err(|e| format!("Failed to create SavedVariables directory: {e}"))?;

        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        let mut errors = Vec::new();

        for folder in &addon_folders {
            if let Err(e) = validate_name(folder) {
                errors.push(format!("{folder}: invalid folder name: {e}"));
                continue;
            }

            let entry = match settings.get(folder.as_str()) {
                Some(e) => e,
                None => {
                    skipped.push(folder.clone());
                    continue;
                }
            };

            if entry.encoding != "lua-text" {
                errors.push(format!(
                    "{}: unsupported encoding '{}'",
                    folder, entry.encoding
                ));
                continue;
            }

            let substituted = crate::saved_variables::scrub::substitute_placeholders(
                &entry.lua,
                &ctx,
                WELL_KNOWN_WORLDS,
            );

            // Reject if identity placeholders could not be resolved
            if has_unresolved_identity_placeholders(&substituted) {
                errors.push(format!(
                    "{folder}: unresolved identity placeholders — launch ESO at least once to establish your identity"
                ));
                continue;
            }

            // Validate that the result is a well-formed SavedVariables file
            let file_name = format!("{folder}.lua");
            if let Err(e) = parse_sv_file(&substituted, &file_name) {
                errors.push(format!("{folder}: settings file failed validation: {e}"));
                continue;
            }

            let dest = sv_dir.join(format!("{folder}.lua"));

            // Create .bak before overwriting
            if dest.is_file() {
                let bak = dest.with_extension("lua.bak");
                if let Err(e) = fs::copy(&dest, &bak) {
                    errors.push(format!("{folder}: failed to create backup: {e}"));
                    continue;
                }
            }

            // Atomic write
            let tmp = sv_dir.join(format!("{folder}.lua.tmp"));
            if let Err(e) = fs::write(&tmp, &substituted) {
                errors.push(format!("{folder}: failed to write: {e}"));
                continue;
            }
            if dest.exists() {
                if let Err(e) = fs::remove_file(&dest) {
                    let _ = fs::remove_file(&tmp);
                    errors.push(format!(
                        "{folder}: failed to replace existing file: {e}"
                    ));
                    continue;
                }
            }
            if let Err(e) = fs::rename(&tmp, &dest) {
                let _ = fs::remove_file(&tmp);
                errors.push(format!("{folder}: failed to finalize write: {e}"));
                continue;
            }

            applied.push(folder.clone());
        }

        Ok(SvImportResult {
            applied,
            skipped,
            errors,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Walk the SavedVariables `.lua` files, parse each that parses successfully,
/// and invoke `visit` with its tree. Used by the scrub/identity-export path
/// (`collect_local_identities`), which needs the full parsed tree.
///
/// When `max_file_bytes` is `Some`, files larger than the cap are skipped so a
/// single pathological SavedVariables file cannot blow up memory. `None` means
/// no cap (the scrub/export path needs every identity for correctness). Returns
/// the number of `.lua` files skipped for ANY reason (size, unreadable, or
/// parse failure) so a caller relying on completeness can react.
///
/// NOTE: neither the Characters roster nor the identity-export path uses this
/// anymore — both stream the raw bytes with bounded memory
/// ([`collect_roster_characters`] and [`collect_local_identities`]). It is
/// retained for the size-cap unit test and any future tree-based caller.
#[cfg_attr(not(test), allow(dead_code))]
fn for_each_sv_tree(
    addons_dir: &Path,
    max_file_bytes: Option<u64>,
    mut visit: impl FnMut(&crate::saved_variables::types::SvTreeNode),
) -> usize {
    use crate::saved_variables::parser::parse_sv_file;

    let sv_dir = sv_io::saved_variables_dir(addons_dir);
    let entries = match fs::read_dir(&sv_dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    let mut skipped = 0usize;
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                // Couldn't enumerate this entry — count it so the roster can warn.
                skipped += 1;
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("lua") {
            continue;
        }
        if let Some(max) = max_file_bytes {
            if let Ok(meta) = fs::metadata(&path) {
                if meta.len() > max {
                    skipped += 1;
                    continue;
                }
            }
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.lua")
            .to_string();
        let tree = match parse_sv_file(&content, &file_name) {
            Ok(t) => t,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        visit(&tree);
    }

    skipped
}

/// Walk every SavedVariables `.lua` file and accumulate the merged
/// account/character identities found across all of them. Used by the scrub/
/// import path, which must see every identity, so it is intentionally uncapped.
///
/// Streams each file's raw bytes through
/// [`detect_identities_streaming`](crate::saved_variables::identity_stream), the
/// bounded-memory scanner that emits the SAME identities as the tree-based
/// `detect_identities_from_tree` (verified by a parity test) — instead of
/// parsing every `.lua` into a full `SvTreeNode` tree (~10x the source size),
/// which on a 1–2 GB SavedVariables file was a multi-GB transient. Memory is now
/// `O(nesting depth + one key)` per file regardless of file size, so the export
/// path no longer needs a size cap to stay safe.
///
/// Runs synchronously; callers wrap it in `spawn_blocking`.
fn collect_local_identities(addons_dir: &Path) -> crate::saved_variables::scrub::ScrubContext {
    use crate::saved_variables::identity_stream::detect_identities_streaming;
    use crate::saved_variables::scrub::ScrubContext;

    let sv_dir = sv_io::saved_variables_dir(addons_dir);
    let entries = match fs::read_dir(&sv_dir) {
        Ok(e) => e,
        Err(_) => return ScrubContext::default(),
    };

    let mut merged = ScrubContext::default();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("lua") {
            continue;
        }
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let ctx = match detect_identities_streaming(std::io::BufReader::new(file)) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for acc in ctx.accounts {
            if !merged.accounts.contains(&acc) {
                merged.accounts.push(acc);
            }
        }
        for ch in ctx.characters {
            if !merged.characters.contains(&ch) {
                merged.characters.push(ch);
            }
        }
        for id in ctx.character_ids {
            if !merged.character_ids.contains(&id) {
                merged.character_ids.push(id);
            }
        }
        for w in ctx.extra_worlds {
            if !merged.extra_worlds.contains(&w) {
                merged.extra_worlds.push(w);
            }
        }
    }
    merged
}

/// Scan SavedVariables for the Characters roster, returning `(raw_key, world)`
/// per character.
///
/// Uses the bounded-memory streaming extractor
/// ([`crate::saved_variables::roster_stream`]), which walks each `.lua` file's
/// raw bytes instead of parsing it into a full in-memory tree. This emits the
/// SAME `(name, world)` set as the stricter `detect_roster_characters_from_tree`
/// (verified by a parity test) — so addon config sections are never surfaced as
/// fake characters — while imposing NO file-size cap, so a character whose only
/// key lives in a huge SavedVariables file is no longer hidden.
///
/// Runs synchronously; callers wrap it in `spawn_blocking`. Returns the
/// `(raw_key, world)` characters plus the number of `.lua` files the scan could
/// not fully trust — a directory/file I/O error, or a malformed/truncated file
/// whose structure didn't balance — so the caller can warn that the roster may
/// be incomplete. (Unlike the old tree path, an oversized file is no longer a
/// skip reason, and a malformed file still contributes whatever it could
/// recover rather than being dropped wholesale.)
fn collect_roster_characters(addons_dir: &Path) -> (Vec<(String, Option<String>)>, usize) {
    use crate::saved_variables::roster_stream::extract_roster_characters_streaming;

    /// Aggregate cap on distinct character names merged across every `.lua` file,
    /// bounding `list_characters` memory against a directory of pathological files
    /// even though each individual scan is already capped. Wildly above any real
    /// account roster.
    const ROSTER_TOTAL_MAX: usize = 100_000;

    let sv_dir = sv_io::saved_variables_dir(addons_dir);
    let entries = match fs::read_dir(&sv_dir) {
        Ok(e) => e,
        // A missing SavedVariables folder is the expected "no characters yet"
        // case (0 skipped). Any OTHER error (permissions, etc.) means we couldn't
        // scan at all — report it as a skipped file so the UI still warns rather
        // than silently showing an empty roster.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), 0),
        Err(_) => return (Vec::new(), 1),
    };

    // raw key -> the distinct megaservers it was seen under. A name with one or
    // more known worlds yields one entry per world (so an NA and an EU character
    // sharing a name stay distinct); a name only ever seen world-less yields a
    // single unknown-world entry.
    let mut known_worlds: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    let mut all_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut skipped = 0usize;
    let mut aggregate_truncated = false;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                // Couldn't enumerate this entry — count it so the roster warns.
                skipped += 1;
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("lua") {
            continue;
        }
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let scan = match extract_roster_characters_streaming(std::io::BufReader::new(file)) {
            Ok(s) => s,
            Err(_) => {
                // Underlying read error mid-stream.
                skipped += 1;
                continue;
            }
        };
        if scan.malformed {
            // Best-effort: keep whatever characters were recovered, but flag the
            // file so the UI still warns the roster may be incomplete.
            skipped += 1;
        }
        for (name, world) in scan.characters {
            // Aggregate bound: a real account has at most a few dozen characters,
            // so this ceiling is never reached in practice, but it keeps the merged
            // roster from growing without limit across many pathological files
            // (each individually under the per-scan cap). New names beyond the cap
            // are dropped and the roster is flagged incomplete; worlds for names we
            // already kept still merge.
            if all_names.contains(&name) {
                if let Some(world) = world {
                    known_worlds.entry(name).or_default().insert(world);
                }
            } else if all_names.len() < ROSTER_TOTAL_MAX {
                all_names.insert(name.clone());
                if let Some(world) = world {
                    known_worlds.entry(name).or_default().insert(world);
                }
            } else {
                aggregate_truncated = true;
            }
        }
    }

    if aggregate_truncated {
        // Count the truncation once so the UI's "may be incomplete" warning fires.
        skipped += 1;
    }

    if skipped > 0 {
        // This scan is the only character source in account-wide mode, so an
        // unreadable/malformed file can hide a character. Surface it in the logs.
        eprintln!(
            "[characters] roster scan could not fully read {skipped} SavedVariables \
             file(s) (unreadable or malformed); some characters may be missing from the list"
        );
    }

    let mut out: Vec<(String, Option<String>)> = Vec::new();
    for name in all_names {
        match known_worlds.get(&name) {
            Some(worlds) if !worlds.is_empty() => {
                for world in worlds {
                    out.push((name.clone(), Some(world.clone())));
                }
            }
            _ => out.push((name, None)),
        }
    }
    (out, skipped)
}

/// Detect the account/character identities present in the local SavedVariables
/// directory. Reads any available `.lua` file that parses successfully and
/// accumulates identities across all of them. Returns the merged `ScrubContext`.
///
/// The frontend passes this context to `import_sv_settings` so that
/// placeholder tokens from a v2 `.esopack` file can be substituted with the
/// local player's real names.
#[tauri::command]
pub async fn detect_local_identities(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<crate::saved_variables::scrub::ScrubContext, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;

    tokio::task::spawn_blocking(move || -> Result<_, String> {
        Ok(collect_local_identities(&addons_dir))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// ─── SavedVariables Manager ─────────────────────────────────

#[tauri::command]
pub async fn get_saved_variables_path(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<String, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        let sv_dir = sv_io::saved_variables_dir(&addons_dir);
        if !sv_dir.is_dir() {
            return Err("SavedVariables folder not found.".to_string());
        }
        Ok(sv_dir.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn list_saved_variables(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<Vec<crate::saved_variables::SavedVariableFile>, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || sv_io::list_saved_variables_blocking(&addons_dir))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn read_saved_variable(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    file_name: String,
) -> Result<crate::saved_variables::SvReadResponse, String> {
    validate_name(&file_name)?;
    if !file_name.ends_with(".lua") {
        return Err("Only .lua files can be read.".to_string());
    }
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        sv_io::read_saved_variable_blocking(&addons_dir, &file_name)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn write_saved_variable(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    file_name: String,
    tree: crate::saved_variables::SvTreeNode,
    stamp: crate::saved_variables::SvFileStamp,
) -> Result<crate::saved_variables::SvFileStamp, String> {
    validate_name(&file_name)?;
    if !file_name.ends_with(".lua") {
        return Err("Only .lua files can be written.".to_string());
    }
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        sv_io::write_saved_variable_blocking(&addons_dir, &file_name, &tree, &stamp)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn preview_sv_save(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    file_name: String,
    tree: crate::saved_variables::SvTreeNode,
) -> Result<crate::saved_variables::SvDiffPreview, String> {
    validate_name(&file_name)?;
    if !file_name.ends_with(".lua") {
        return Err("Only .lua files can be previewed.".to_string());
    }
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        let changes = sv_io::preview_save(&addons_dir, &file_name, &tree)?;
        Ok(crate::saved_variables::SvDiffPreview { changes })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn restore_sv_backup(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    file_name: String,
) -> Result<crate::saved_variables::SvFileStamp, String> {
    validate_name(&file_name)?;
    if !file_name.ends_with(".lua") {
        return Err("Only .lua files can be restored.".to_string());
    }
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || sv_io::restore_backup_file(&addons_dir, &file_name))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

// ── Roster Pack Install (deep link: kalpa://install-pack/{id}) ────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RosterPack {
    pub id: String,
    pub title: String,
    pub addons: Vec<PackAddonEntry>,
}

#[tauri::command]
pub async fn fetch_roster_pack(pack_id: String) -> Result<RosterPack, String> {
    validate_pack_id(&pack_id)?;

    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/packs/{pack_id}");

        let response = client.get(&url).send().map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                "Could not connect to Pack Hub. Check your internet connection.".to_string()
            } else {
                format!("Network error: {e}")
            }
        })?;

        match response.status().as_u16() {
            200 => {}
            404 => return Err(format!("Pack \"{pack_id}\" not found.")),
            status => return Err(format!("Pack Hub returned HTTP {status}")),
        }

        let body: PackSingleResponse = response
            .json()
            .map_err(|e| format!("Failed to parse pack response: {e}"))?;

        let pack = Pack::from_hub(body.pack);
        Ok(RosterPack {
            id: pack.id,
            title: pack.title,
            addons: pack.addons,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn copy_sv_profile(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    file_name: String,
    from_key: String,
    to_key: String,
) -> Result<(), String> {
    validate_name(&file_name)?;
    if !file_name.ends_with(".lua") {
        return Err("Only .lua files can be modified.".to_string());
    }
    if from_key.is_empty() || to_key.is_empty() {
        return Err("Source and destination keys cannot be empty.".to_string());
    }
    if from_key.contains('"')
        || to_key.contains('"')
        || from_key.contains('\'')
        || to_key.contains('\'')
        || from_key.contains('\\')
        || to_key.contains('\\')
    {
        return Err("Character keys must not contain quotes or backslashes.".to_string());
    }
    if from_key.chars().any(|c| c.is_control()) || to_key.chars().any(|c| c.is_control()) {
        return Err("Character keys must not contain control characters.".to_string());
    }
    if from_key == to_key {
        return Err("Source and destination are the same character.".to_string());
    }
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        crate::saved_variables::profile::copy_sv_profile_blocking(
            &addons_dir,
            &file_name,
            &from_key,
            &to_key,
        )
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn is_eso_running() -> Result<bool, String> {
    tokio::task::spawn_blocking(|| {
        #[cfg(target_os = "windows")]
        {
            is_eso_running_windows()
        }

        #[cfg(not(target_os = "windows"))]
        {
            Ok(false)
        }
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Check for eso64.exe / eso.exe using the Windows Toolhelp32 snapshot API.
/// This avoids spawning a `tasklist` subprocess (which lists every process as CSV).
#[cfg(target_os = "windows")]
fn is_eso_running_windows() -> Result<bool, String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct PROCESSENTRY32W {
        dwSize: u32,
        cntUsage: u32,
        th32ProcessID: u32,
        th32DefaultHeapID: usize,
        th32ModuleID: u32,
        cntThreads: u32,
        th32ParentProcessID: u32,
        pcPriClassBase: i32,
        dwFlags: u32,
        szExeFile: [u16; 260],
    }

    const TH32CS_SNAPPROCESS: u32 = 0x00000002;
    const INVALID_HANDLE_VALUE: isize = -1;

    extern "system" {
        fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> isize;
        fn Process32FirstW(hSnapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32;
        fn Process32NextW(hSnapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32;
        fn CloseHandle(hObject: isize) -> i32;
    }

    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Err("Failed to create process snapshot".to_string());
        }

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        let mut found = false;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(260);
                let name = OsString::from_wide(&entry.szExeFile[..len])
                    .to_string_lossy()
                    .to_lowercase();
                if name == "eso64.exe" || name == "eso.exe" {
                    found = true;
                    break;
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }

        CloseHandle(snap);
        Ok(found)
    }
}

/// Delete selected SavedVariables files after creating an automatic backup.
#[tauri::command]
pub async fn delete_saved_variables(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    file_names: Vec<String>,
) -> Result<u32, String> {
    if file_names.is_empty() {
        return Err("No files selected for deletion.".to_string());
    }
    if file_names.len() > 200 {
        return Err("Too many files selected (max 200).".to_string());
    }
    for name in &file_names {
        validate_name(name)?;
        if !name.ends_with(".lua") {
            return Err(format!("Only .lua files can be deleted: {name}"));
        }
    }
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        let result = sv_io::delete_saved_variables_blocking(&addons_dir, &file_names);
        if result.is_ok() {
            // Keep only the 3 most recent auto-cleanup snapshots to prevent
            // unbounded disk growth from repeated deletes; best-effort.
            prune_auto_snapshots(&backups_dir(&addons_dir), "auto-cleanup-", 3);
        }
        result
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Scrub a SavedVariables file with the templating + heuristic pipeline and
/// return a report. Debug builds only — used to validate the scrubber against
/// real addons before the production export/import flow ships.
///
/// If `ctx` is the default (no accounts/characters supplied), the command
/// auto-detects identities from the parsed tree.
#[cfg(debug_assertions)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevScrubResult {
    pub file_name: String,
    pub original_bytes: usize,
    pub scrubbed_bytes: usize,
    pub detected_context: crate::saved_variables::scrub::ScrubContext,
    pub drops: Vec<crate::saved_variables::scrub::DropEntry>,
    pub templated_keys: Vec<crate::saved_variables::scrub::TemplateEntry>,
    pub scrubbed_lua: String,
}

#[cfg(debug_assertions)]
#[tauri::command]
pub async fn dev_scrub_saved_variable(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    file_name: String,
    ctx: Option<crate::saved_variables::scrub::ScrubContext>,
) -> Result<DevScrubResult, String> {
    validate_name(&file_name)?;
    if !file_name.ends_with(".lua") {
        return Err("Only .lua files can be scrubbed.".to_string());
    }
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || -> Result<DevScrubResult, String> {
        let response = sv_io::read_saved_variable_blocking(&addons_dir, &file_name)?;
        let effective_ctx = ctx.unwrap_or_else(|| {
            crate::saved_variables::scrub::detect_identities_from_tree(&response.tree)
        });
        // `scrub` consumes the tree; `effective_ctx` is already resolved above.
        let (scrubbed, report) =
            crate::saved_variables::scrub::scrub(response.tree, &effective_ctx);
        let scrubbed_lua = crate::saved_variables::serializer::serialize_to_lua(&scrubbed);
        Ok(DevScrubResult {
            file_name,
            original_bytes: report.original_bytes,
            scrubbed_bytes: report.scrubbed_bytes,
            detected_context: effective_ctx,
            drops: report.drops,
            templated_keys: report.templated_keys,
            scrubbed_lua,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub fn update_tray_tooltip(
    tray_state: tauri::State<'_, crate::TrayState>,
    update_count: u32,
) -> Result<(), String> {
    let guard = tray_state
        .0
        .lock()
        .map_err(|_| "Internal error".to_string())?;
    if let Some(tray) = &*guard {
        let tooltip = if update_count > 0 {
            format!(
                "Kalpa — {} update{} available",
                update_count,
                if update_count != 1 { "s" } else { "" }
            )
        } else {
            "Kalpa".to_string()
        };
        tray.set_tooltip(Some(&tooltip))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Atomically persist the settings store to disk. The frontend calls this
/// instead of the plugin-store `save()` so writes are crash-safe (write-temp +
/// fsync + atomic rename); see `settings_store`.
#[tauri::command]
pub async fn flush_settings(app: tauri::AppHandle) -> Result<(), String> {
    crate::settings_store::flush(&app)
}

/// Whether the settings store opened TAINTED (empty over an unreadable settings
/// file), so cached reads are untrusted defaults. Security-sensitive frontend reads
/// (the native-upload opt-out) consult this to fail CLOSED instead of trusting a
/// default that may mask a real persisted opt-out.
#[tauri::command]
pub fn settings_tainted() -> bool {
    crate::settings_store::is_tainted()
}

// ── Controlled Folder Access / write-access detection ──────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteAccessStatus {
    /// True when Kalpa cannot write into the AddOns folder.
    pub blocked: bool,
    /// True when the block looks like a permission denial (the common cause
    /// being Windows Controlled Folder Access). Lets the UI hedge the message.
    pub permission_denied: bool,
    /// Absolute path to this Kalpa executable, so the UI can show the user
    /// exactly which app to allow through Controlled Folder Access. Empty if
    /// it cannot be determined.
    pub exe_path: String,
}

/// Probe whether Kalpa (this process) can actually write into the AddOns
/// folder by creating and removing a tiny temp file. This is the correct
/// detector for Controlled Folder Access because CFA gates on the *writing
/// process* — checking Defender config would require admin and would not tell
/// us whether Kalpa specifically is exempted. The probe also naturally covers
/// read-only/ACL/antivirus blocks, and self-resolves once access is granted.
///
/// Fails open: if the directory is missing or any non-permission error occurs,
/// we report `blocked: false` so a detection hiccup never gates the app.
#[tauri::command]
pub async fn check_addons_write_access(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<WriteAccessStatus, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
        let exe_path = std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        if !addons_dir.is_dir() {
            return WriteAccessStatus {
                blocked: false,
                permission_denied: false,
                exe_path,
            };
        }

        // Probe inside an existing addon subfolder, not the AddOns root.
        // Controlled Folder Access has been observed to permit a write to the
        // root while still blocking writes into nested addon folders — which
        // is exactly where extraction writes. A root-only probe therefore
        // reports "writable" when real updates will fail. Pick the first
        // subdirectory; fall back to the root only if there are none.
        let probe_dir = fs::read_dir(&addons_dir)
            .ok()
            .and_then(|entries| {
                entries
                    .flatten()
                    .find(|e| e.path().is_dir())
                    .map(|e| e.path())
            })
            .unwrap_or_else(|| addons_dir.clone());

        // Best-effort write probe. Note a known limitation: some Controlled
        // Folder Access configurations permit a process to create (and even
        // overwrite) its *own* new file while still blocking modification of
        // pre-existing files — which is what extraction does. On such configs
        // this probe cannot replicate the block without overwriting the user's
        // real addon files, so it returns `blocked: false` and we rely on the
        // post-failure guidance instead. The probe only ever trips on
        // PermissionDenied, so it can false-negative (degrades to the failure
        // path) but never false-positive (never aborts a healthy update).
        let probe = probe_dir.join(".kalpa-write-probe");
        match fs::write(&probe, b"") {
            Ok(()) => {
                let _ = fs::remove_file(&probe);
                WriteAccessStatus {
                    blocked: false,
                    permission_denied: false,
                    exe_path,
                }
            }
            Err(e) => {
                let _ = fs::remove_file(&probe);
                // CFA can surface as PermissionDenied; treat that as the
                // actionable "blocked" case. Other errors (e.g. the parent
                // briefly unavailable) fail open to avoid false alarms.
                let permission_denied = e.kind() == std::io::ErrorKind::PermissionDenied;
                WriteAccessStatus {
                    blocked: permission_denied,
                    permission_denied,
                    exe_path,
                }
            }
        }
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))
}

/// Open the Windows Security "Ransomware protection" page, where the user can
/// allow Kalpa through Controlled Folder Access. Windows-only; the deep link
/// is a fixed constant (no interpolation).
#[tauri::command]
pub fn open_ransomware_protection_settings() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        // `cmd /c start "" "<uri>"` reliably hands the custom URI scheme to the
        // shell handler that opens Windows Security.
        Command::new("cmd")
            .args(["/C", "start", "", "windowsdefender://RansomwareProtection"])
            .spawn()
            .map_err(|e| format!("Failed to open Windows Security: {e}"))?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Windows Security is only available on Windows.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_emit_progress_first_stride_and_completion() {
        // First event (no prior emit) always fires, so the frontend learns the
        // operation id and can enable Stop even on a tiny archive.
        assert!(should_emit_progress(usize::MAX, 0, 5000));
        assert!(should_emit_progress(usize::MAX, 0, 0));

        // Within a stride of the last emit (prev=0): nothing until 64 files pass.
        assert!(!should_emit_progress(0, 1, 5000));
        assert!(!should_emit_progress(0, 63, 5000));
        assert!(should_emit_progress(0, 64, 5000));
        assert!(should_emit_progress(0, 200, 5000));

        // Completion (done >= total) always fires, even if fewer than a stride
        // of files have passed since the last emit — so the final "N of N" lands.
        assert!(should_emit_progress(5000, 5000, 5000));
        assert!(should_emit_progress(4990, 5000, 5000)); // only 10 since last emit
    }

    #[test]
    fn download_thread_count_scales_and_clamps() {
        // A single-addon batch must still get at least one thread. (The
        // streaming consumer runs on its own thread, so num_threads(1) is safe,
        // but 0 would panic when building the pool.)
        assert_eq!(download_thread_count(1), 1);
        assert_eq!(download_thread_count(3), 3);
        // Capped so a huge batch never hammers ESOUI with too many connections.
        assert_eq!(download_thread_count(50), 6);
        // Defensive: an empty batch never yields a zero-thread pool.
        assert_eq!(download_thread_count(0), 1);
    }

    #[test]
    fn prune_auto_snapshots_keeps_newest_removes_oldest() {
        // Names embed epoch timestamps and are sorted lexicographically, so an
        // inverted sort would delete the newest snapshots instead of the oldest.
        let tmp = tempfile::tempdir().unwrap();
        let backups_dir = tmp.path();
        let prefix = "auto-before-restore-";
        let epochs = [1_000_u64, 1_100, 1_200, 1_300, 1_400];
        for epoch in epochs {
            fs::create_dir_all(backups_dir.join(format!("{prefix}{epoch}"))).unwrap();
        }

        prune_auto_snapshots(backups_dir, prefix, 3);

        let remaining: std::collections::HashSet<String> = fs::read_dir(backups_dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();

        // Only the 3 highest-epoch snapshots survive.
        assert_eq!(remaining.len(), 3);
        assert!(remaining.contains(&format!("{prefix}1200")));
        assert!(remaining.contains(&format!("{prefix}1300")));
        assert!(remaining.contains(&format!("{prefix}1400")));
        assert!(!remaining.contains(&format!("{prefix}1000")));
        assert!(!remaining.contains(&format!("{prefix}1100")));
    }

    /// Build a roster slice of `(raw_key, world)` from terse pairs.
    fn sv(chars: &[(&str, Option<&str>)]) -> Vec<(String, Option<String>)> {
        chars
            .iter()
            .map(|(n, w)| (n.to_string(), w.map(str::to_string)))
            .collect()
    }

    #[test]
    fn build_character_list_recovers_account_wide_characters() {
        // Account-wide mode: AddOnSettings has only globals + the $AccountWide
        // sentinel, so every character must come from SavedVariables.
        let settings = "#Version 100027\n#$AccountWide\nSomeAddon 1\n";
        let roster = sv(&[("Mainchar", None), ("Alttank", None)]);
        let chars = build_character_list(Some(settings), &roster);
        assert_eq!(chars.len(), 2);
        assert!(chars
            .iter()
            .all(|c| c.server == UNKNOWN_SERVER && c.recovered));
        assert!(chars.iter().any(|c| c.name == "Mainchar"));
        assert!(chars.iter().any(|c| c.name == "Alttank"));
    }

    #[test]
    fn build_character_list_addonsettings_wins_over_sv_duplicate() {
        // Same character in both sources (plain + caret form) collapses to one,
        // keeping the authoritative AddOnSettings server.
        let settings = "#NA Megaserver-Faewynd\n";
        let roster = sv(&[("Faewynd", None), ("Faewynd^Mx", None)]);
        let chars = build_character_list(Some(settings), &roster);
        assert_eq!(chars.len(), 1);
        assert_eq!(chars[0].name, "Faewynd");
        assert_eq!(chars[0].server, "NA Megaserver");
        assert!(!chars[0].recovered);
    }

    #[test]
    fn build_character_list_strips_caret_suffix() {
        let chars = build_character_list(None, &sv(&[("Faewynd^Mx", None)]));
        assert_eq!(chars.len(), 1);
        assert_eq!(chars[0].name, "Faewynd");
        assert_eq!(chars[0].server, UNKNOWN_SERVER);
        assert!(chars[0].recovered);
    }

    #[test]
    fn build_character_list_skips_markers_and_numeric_ids() {
        let roster = sv(&[
            ("$AccountWide", None),
            ("123456789012345", None),
            ("Realchar", None),
        ]);
        let chars = build_character_list(None, &roster);
        assert_eq!(chars.len(), 1);
        assert_eq!(chars[0].name, "Realchar");
    }

    #[test]
    fn build_character_list_preserves_hyphenated_name() {
        // Regression guard: first-dash split must keep the whole name.
        let settings = "#NA Megaserver-Jodynn-Jo\n";
        let chars = build_character_list(Some(settings), &[]);
        assert_eq!(chars.len(), 1);
        assert_eq!(chars[0].server, "NA Megaserver");
        assert_eq!(chars[0].name, "Jodynn-Jo");
    }

    #[test]
    fn build_character_list_same_name_two_servers_kept() {
        // ESO names are unique per megaserver, so NA-Bob and EU-Bob are distinct.
        let settings = "#NA Megaserver-Bob\n#EU Megaserver-Bob\n";
        let chars = build_character_list(Some(settings), &[]);
        assert_eq!(chars.len(), 2);
        assert!(chars.iter().any(|c| c.server == "NA Megaserver"));
        assert!(chars.iter().any(|c| c.server == "EU Megaserver"));
    }

    #[test]
    fn build_character_list_uses_world_scoped_server() {
        // A SavedVariables-only character stored under a world layer gets its
        // real megaserver instead of the Unknown bucket.
        let chars = build_character_list(None, &sv(&[("Faewynd", Some("EU Megaserver"))]));
        assert_eq!(chars.len(), 1);
        assert_eq!(chars[0].server, "EU Megaserver");
        assert!(chars[0].recovered);
    }

    #[test]
    fn build_character_list_ignores_unknown_world_value() {
        // A non-megaserver world string must not be used as a server label.
        let chars = build_character_list(None, &sv(&[("Faewynd", Some("Some Guild"))]));
        assert_eq!(chars.len(), 1);
        assert_eq!(chars[0].server, UNKNOWN_SERVER);
    }

    #[test]
    fn build_character_list_keeps_recovered_na_eu_twins() {
        // Same name recovered under two megaservers stays two distinct characters.
        let roster = sv(&[
            ("Bob", Some("NA Megaserver")),
            ("Bob", Some("EU Megaserver")),
        ]);
        let chars = build_character_list(None, &roster);
        assert_eq!(chars.len(), 2);
        assert!(chars
            .iter()
            .any(|c| c.server == "NA Megaserver" && c.name == "Bob"));
        assert!(chars
            .iter()
            .any(|c| c.server == "EU Megaserver" && c.name == "Bob"));
    }

    #[test]
    fn build_character_list_unknown_dup_collapses_to_known_server() {
        // Known-megaserver + unknown-server entry of the same name collapse to
        // the known one, independent of input order.
        let roster = sv(&[("Bob", None), ("Bob", Some("NA Megaserver"))]);
        let chars = build_character_list(None, &roster);
        assert_eq!(chars.len(), 1);
        assert_eq!(chars[0].server, "NA Megaserver");
    }

    #[test]
    fn build_character_list_empty_when_no_sources() {
        let chars = build_character_list(None, &[]);
        assert!(chars.is_empty());
    }

    #[test]
    fn for_each_sv_tree_skips_oversized_without_parsing() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("live").join("AddOns");
        let sv = tmp.path().join("live").join("SavedVariables");
        fs::create_dir_all(&addons).unwrap();
        fs::create_dir_all(&sv).unwrap();
        fs::write(sv.join("small.lua"), "Var =\n{\n}\n").unwrap();
        // Oversized relative to the test cap; never read or parsed.
        fs::write(sv.join("big.lua"), vec![b'-'; 5000]).unwrap();

        let mut visited = 0usize;
        let skipped = for_each_sv_tree(&addons, Some(1000), |_| visited += 1);

        assert_eq!(skipped, 1, "big.lua should be skipped by the size cap");
        assert_eq!(visited, 1, "only small.lua should be parsed/visited");
    }

    #[test]
    fn purge_backup_scratch_removes_only_matching_scratch() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".old-char-Bob-backup-3")).unwrap();
        fs::create_dir_all(root.join(".tmp-char-Bob-backup-7")).unwrap();
        fs::create_dir_all(root.join(".old-char-Other-backup-1")).unwrap();
        fs::create_dir_all(root.join("char-Bob-backup")).unwrap();

        purge_backup_scratch(root, "char-Bob-backup");

        // This backup's tombstone/staging are gone...
        assert!(!root.join(".old-char-Bob-backup-3").exists());
        assert!(!root.join(".tmp-char-Bob-backup-7").exists());
        // ...but an unrelated backup's scratch and the real backup are untouched.
        assert!(root.join(".old-char-Other-backup-1").exists());
        assert!(root.join("char-Bob-backup").exists());
    }

    #[test]
    fn char_backup_replaceable_only_absent_or_marked() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        assert!(char_backup_replaceable(&root.join("char-none"))); // absent

        // Unmarked existing dir (legacy/manual) — NOT replaceable.
        let unmarked = root.join("char-manual");
        fs::create_dir_all(&unmarked).unwrap();
        assert!(!char_backup_replaceable(&unmarked));

        // Marked character backup — replaceable (refresh in place).
        let marked = root.join("char-real");
        fs::create_dir_all(&marked).unwrap();
        fs::write(marked.join(CHAR_BACKUP_MARKER), b"x").unwrap();
        assert!(char_backup_replaceable(&marked));
    }

    #[test]
    fn resolve_char_backup_name_routes_around_unmarked_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Free name is used as-is.
        assert_eq!(
            resolve_char_backup_name(root, "Faewynd-backup").as_deref(),
            Some("Faewynd-backup")
        );

        // An unmarked (legacy/manual) dir is preserved; a numbered sibling is used.
        fs::create_dir_all(root.join("char-Bob-backup")).unwrap();
        assert_eq!(
            resolve_char_backup_name(root, "Bob-backup").as_deref(),
            Some("Bob-backup-2")
        );
        assert!(root.join("char-Bob-backup").is_dir()); // original untouched

        // A marked backup at the requested name is refreshed in place.
        let marked = root.join("char-Alt-backup");
        fs::create_dir_all(&marked).unwrap();
        fs::write(marked.join(CHAR_BACKUP_MARKER), b"x").unwrap();
        assert_eq!(
            resolve_char_backup_name(root, "Alt-backup").as_deref(),
            Some("Alt-backup")
        );
    }

    /// Create `<dir>/<folder>/<folder>.txt` so the folder is a loadable addon.
    fn make_addon_folder(dir: &std::path::Path, folder: &str, manifest_body: &str) {
        let addon_dir = dir.join(folder);
        std::fs::create_dir_all(&addon_dir).unwrap();
        std::fs::write(addon_dir.join(format!("{folder}.txt")), manifest_body).unwrap();
    }

    #[test]
    fn conflict_report_does_not_treat_legacy_binary_hash_as_unchanged_upstream() {
        use std::io::Write;

        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let addon_dir = addons_dir.join("MediaAddon");
        fs::create_dir_all(&addon_dir).unwrap();

        let mut files = HashMap::new();
        files.insert("icons/a.dds".to_string(), "0".repeat(64));
        file_hashes::save_hash_manifest(
            &addons_dir,
            &file_hashes::HashManifest {
                addon_folder: "MediaAddon".to_string(),
                esoui_ids: vec![123],
                recorded_at: "2026-01-01T00-00-00Z".to_string(),
                installed_version: "1.0".to_string(),
                files,
                ..Default::default()
            },
        )
        .unwrap();

        let zip_path = tmp.path().join("update.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        archive
            .start_file("MediaAddon/icons/a.dds", options)
            .unwrap();
        archive.write_all(b"new-texture-bytes").unwrap();
        archive.finish().unwrap();

        let (report, _) =
            build_conflict_report(&addons_dir, "MediaAddon", &zip_path, "2.0", "session").unwrap();

        assert_eq!(report.auto_kept_files, Vec::<String>::new());
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].relative_path, "icons/a.dds");
    }

    #[test]
    fn normalize_addon_name_is_case_and_whitespace_insensitive() {
        assert_eq!(normalize_addon_name("LuiMedia"), "luimedia");
        assert_eq!(normalize_addon_name("LUIMEDIA"), "luimedia");
        assert_eq!(normalize_addon_name("  LuiMedia  "), "luimedia");
    }

    #[test]
    fn normalize_addon_name_does_not_strip_codepoints() {
        // A folder with an embedded zero-width char is a DIFFERENT addon to ESO,
        // so normalization must not collapse it onto the clean name.
        assert_ne!(
            normalize_addon_name("Lui\u{200B}Media"),
            normalize_addon_name("LuiMedia")
        );
    }

    // ── Addon profile activation ─────────────────────────────────────────

    /// Write a top-level addon folder with a manifest. `folder` may carry a
    /// `.disabled` suffix; the manifest name inside always uses the base name
    /// (matching what a rename-disable produces on real installs).
    fn write_addon(addons_dir: &Path, folder: &str, depends_on: &str) {
        let base = folder.strip_suffix(".disabled").unwrap_or(folder);
        let dir = addons_dir.join(folder);
        fs::create_dir_all(&dir).unwrap();
        let mut manifest = format!("## Title: {base}\n");
        if !depends_on.is_empty() {
            manifest.push_str(&format!("## DependsOn: {depends_on}\n"));
        }
        fs::write(dir.join(format!("{base}.txt")), manifest).unwrap();
    }

    fn profile_of(names: &[&str]) -> AddonProfile {
        AddonProfile {
            name: "test".to_string(),
            enabled_addons: names.iter().map(|s| s.to_string()).collect(),
            created_at: String::new(),
        }
    }

    #[test]
    fn apply_profile_enables_and_disables_by_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        write_addon(tmp.path(), "AddonA", "");
        write_addon(tmp.path(), "AddonB.disabled", "");

        let result = apply_profile(tmp.path(), &profile_of(&["AddonB"]));

        assert_eq!(result.enabled, vec!["AddonB"]);
        assert_eq!(result.disabled, vec!["AddonA"]);
        assert!(result.failed.is_empty());
        assert!(result.missing.is_empty());
        assert!(tmp.path().join("AddonA.disabled").is_dir());
        assert!(tmp.path().join("AddonB").is_dir());
    }

    #[test]
    fn apply_profile_keeps_required_dependency_enabled() {
        // LibX was installed (e.g. auto-resolved) after the profile snapshot,
        // so it is not in the snapshot — but AddonA requires it at load time.
        let tmp = tempfile::tempdir().unwrap();
        write_addon(tmp.path(), "AddonA", "LibX");
        write_addon(tmp.path(), "LibX", "");

        let result = apply_profile(tmp.path(), &profile_of(&["AddonA"]));

        assert!(result.disabled.is_empty(), "LibX must not be disabled");
        assert_eq!(result.kept_dependencies, vec!["LibX"]);
        assert!(tmp.path().join("LibX").is_dir());
    }

    #[test]
    fn apply_profile_reenables_disabled_dependency() {
        let tmp = tempfile::tempdir().unwrap();
        write_addon(tmp.path(), "AddonA", "LibX");
        write_addon(tmp.path(), "LibX.disabled", "");

        let result = apply_profile(tmp.path(), &profile_of(&["AddonA"]));

        assert_eq!(result.enabled, vec!["LibX"]);
        assert_eq!(result.kept_dependencies, vec!["LibX"]);
        assert!(tmp.path().join("LibX").is_dir());
        assert!(!tmp.path().join("LibX.disabled").is_dir());
    }

    #[test]
    fn apply_profile_keeps_transitive_dependencies() {
        let tmp = tempfile::tempdir().unwrap();
        write_addon(tmp.path(), "AddonA", "LibX");
        write_addon(tmp.path(), "LibX", "LibY");
        write_addon(tmp.path(), "LibY", "");

        let result = apply_profile(tmp.path(), &profile_of(&["AddonA"]));

        assert!(result.disabled.is_empty());
        assert_eq!(result.kept_dependencies, vec!["LibX", "LibY"]);
    }

    #[test]
    fn apply_profile_dependency_matching_is_case_insensitive() {
        // ESO resolves dependency names case-insensitively; retention must too.
        let tmp = tempfile::tempdir().unwrap();
        write_addon(tmp.path(), "AddonA", "libstub");
        write_addon(tmp.path(), "LibStub", "");

        let result = apply_profile(tmp.path(), &profile_of(&["AddonA"]));

        assert!(result.disabled.is_empty());
        assert_eq!(result.kept_dependencies, vec!["LibStub"]);
    }

    #[test]
    fn apply_profile_embedded_dependency_needs_no_retention() {
        // AddonA bundles its own copy of LibX; the standalone top-level LibX
        // is NOT required and must still be disabled per the snapshot.
        let tmp = tempfile::tempdir().unwrap();
        write_addon(tmp.path(), "AddonA", "LibX");
        let embedded = tmp.path().join("AddonA").join("LibX");
        fs::create_dir_all(&embedded).unwrap();
        fs::write(embedded.join("LibX.txt"), "## Title: LibX\n").unwrap();
        write_addon(tmp.path(), "LibX", "");

        let result = apply_profile(tmp.path(), &profile_of(&["AddonA"]));

        assert_eq!(result.disabled, vec!["LibX"]);
        assert!(result.kept_dependencies.is_empty());
    }

    #[test]
    fn apply_profile_duplicate_folders_enable_is_noop() {
        // Both Foo/ and Foo.disabled/ exist. The scanner's policy is "enabled
        // copy wins", so a profile wanting Foo enabled is already satisfied —
        // no rename, no spurious failure.
        let tmp = tempfile::tempdir().unwrap();
        write_addon(tmp.path(), "Foo", "");
        write_addon(tmp.path(), "Foo.disabled", "");

        let result = apply_profile(tmp.path(), &profile_of(&["Foo"]));

        assert!(result.enabled.is_empty());
        assert!(result.failed.is_empty());
        assert!(tmp.path().join("Foo").is_dir());
        assert!(tmp.path().join("Foo.disabled").is_dir());
    }

    #[test]
    fn apply_profile_duplicate_folders_disable_fails_actionably() {
        // Disabling Foo can't be expressed by rename while Foo.disabled
        // already exists; the failure must say so instead of surfacing a raw
        // OS collision error.
        let tmp = tempfile::tempdir().unwrap();
        write_addon(tmp.path(), "Foo", "");
        write_addon(tmp.path(), "Foo.disabled", "");
        write_addon(tmp.path(), "Bar", "");

        let result = apply_profile(tmp.path(), &profile_of(&["Bar"]));

        assert_eq!(result.failed.len(), 1);
        assert!(result.failed[0].contains("remove the stale copy"));
        // Both copies untouched — nothing was renamed or deleted.
        assert!(tmp.path().join("Foo").is_dir());
        assert!(tmp.path().join("Foo.disabled").is_dir());
    }

    #[test]
    fn apply_profile_reports_missing_and_skips_wrappers_and_kalpa_files() {
        let tmp = tempfile::tempdir().unwrap();
        write_addon(tmp.path(), "AddonA", "");
        // Manifest-less wrapper folder (like `Libs/`) must never be renamed.
        fs::create_dir_all(tmp.path().join("Libs")).unwrap();
        // Kalpa's own folders are ignored entirely.
        fs::create_dir_all(tmp.path().join("kalpa-backups")).unwrap();

        let result = apply_profile(tmp.path(), &profile_of(&["Uninstalled"]));

        assert_eq!(result.disabled, vec!["AddonA"]);
        assert_eq!(result.missing, vec!["Uninstalled"]);
        assert!(tmp.path().join("Libs").is_dir());
        assert!(tmp.path().join("kalpa-backups").is_dir());
    }

    #[test]
    fn plan_profile_is_read_only_and_reports_all_buckets() {
        // The preview must describe every pending change without renaming
        // anything — otherwise "preview" would silently BE the activation.
        let tmp = tempfile::tempdir().unwrap();
        write_addon(tmp.path(), "AddonA", "");
        write_addon(tmp.path(), "AddonB.disabled", "");
        write_addon(tmp.path(), "Dup", "");
        write_addon(tmp.path(), "Dup.disabled", "");

        let plan = plan_profile(tmp.path(), &profile_of(&["AddonB", "Gone"]));

        assert_eq!(plan.to_enable, vec!["AddonB"]);
        assert_eq!(plan.to_disable, vec!["AddonA"]);
        assert_eq!(plan.blocked, vec!["Dup"]);
        assert_eq!(plan.missing, vec!["Gone"]);
        // Disk untouched: still the original four folders.
        assert!(tmp.path().join("AddonA").is_dir());
        assert!(tmp.path().join("AddonB.disabled").is_dir());
        assert!(tmp.path().join("Dup").is_dir());
        assert!(tmp.path().join("Dup.disabled").is_dir());
    }

    #[test]
    fn profile_store_mirror_survives_addons_folder_wipe() {
        let addons = tempfile::tempdir().unwrap();
        let mirror_root = tempfile::tempdir().unwrap();
        let mirror = mirror_root.path().join("mirror.json");

        let store = ProfileStore {
            profiles: vec![profile_of(&["AddonA"])],
            active_profile: Some("test".to_string()),
        };
        save_profiles_with_mirror(addons.path(), Some(&mirror), &store).unwrap();

        // Simulate an AddOns-folder wipe (reinstall / PTS cleanup): the
        // primary store and its recovery artifacts are gone, the mirror isn't.
        fs::remove_file(profiles_path(addons.path())).unwrap();
        let bak = profiles_path(addons.path()).with_extension("json.bak");
        if bak.exists() {
            fs::remove_file(&bak).unwrap();
        }

        let loaded = load_profiles_with_mirror(addons.path(), Some(&mirror));
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.profiles[0].name, "test");
        assert_eq!(loaded.active_profile.as_deref(), Some("test"));
    }

    #[test]
    fn profile_store_primary_wins_over_mirror_when_present() {
        // The mirror is a fallback, not a sync source: a present primary must
        // never be shadowed by a stale mirror from an earlier save.
        let addons = tempfile::tempdir().unwrap();
        let mirror_root = tempfile::tempdir().unwrap();
        let mirror = mirror_root.path().join("mirror.json");

        let old = ProfileStore {
            profiles: vec![profile_of(&["Old"])],
            active_profile: None,
        };
        save_profiles_with_mirror(addons.path(), Some(&mirror), &old).unwrap();
        // Newer save without the mirror (e.g. data dir temporarily unwritable).
        let mut newer = ProfileStore {
            profiles: vec![profile_of(&["New"])],
            active_profile: None,
        };
        newer.profiles[0].name = "newer".to_string();
        metadata::save_json_with_backup(&profiles_path(addons.path()), &newer).unwrap();

        let loaded = load_profiles_with_mirror(addons.path(), Some(&mirror));
        assert_eq!(loaded.profiles[0].name, "newer");
    }

    #[test]
    fn copy_addons_between_copies_enabled_skips_existing_and_disabled() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        write_addon(src.path(), "AddonA", "");
        // Nested content must arrive too.
        let nested = src.path().join("AddonA").join("libs").join("inner");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("inner.lua"), "-- lua").unwrap();
        write_addon(src.path(), "AddonB", "");
        write_addon(src.path(), "Disabled.disabled", "");
        write_addon(dst.path(), "AddonB", "");

        let result = copy_addons_between(src.path(), dst.path());

        assert_eq!(result.copied, vec!["AddonA"]);
        assert_eq!(result.skipped, vec!["AddonB"]);
        assert!(result.failed.is_empty());
        assert!(dst.path().join("AddonA").join("AddonA.txt").is_file());
        assert!(dst
            .path()
            .join("AddonA")
            .join("libs")
            .join("inner")
            .join("inner.lua")
            .is_file());
        // Disabled addons are part of the source's OFF state — not copied.
        assert!(!dst.path().join("Disabled").exists());
        assert!(!dst.path().join("Disabled.disabled").exists());
    }

    #[test]
    fn copy_addons_between_never_overwrites_target_disabled_copy() {
        // Target has the addon disabled by choice; copying must not resurrect
        // or duplicate it.
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        write_addon(src.path(), "AddonA", "");
        write_addon(dst.path(), "AddonA.disabled", "");

        let result = copy_addons_between(src.path(), dst.path());

        assert_eq!(result.skipped, vec!["AddonA"]);
        assert!(!dst.path().join("AddonA").exists());
        assert!(dst.path().join("AddonA.disabled").is_dir());
    }

    #[test]
    fn copy_addons_between_carries_metadata_for_copied_addons() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        write_addon(src.path(), "AddonA", "");

        let mut store = metadata::MetadataStore::default();
        store.addons.insert(
            "AddonA".to_string(),
            metadata::AddonMetadata {
                esoui_id: 1234,
                installed_version: "1.0".to_string(),
                download_url: String::new(),
                installed_at: String::new(),
                tags: vec!["favorite".to_string()],
                esoui_last_update: 0,
            },
        );
        metadata::save_metadata(src.path(), &store).unwrap();

        let result = copy_addons_between(src.path(), dst.path());
        assert_eq!(result.copied, vec!["AddonA"]);

        let dst_store = metadata::load_metadata(dst.path());
        let entry = dst_store.addons.get("AddonA").expect("metadata copied");
        assert_eq!(entry.esoui_id, 1234);
        assert_eq!(entry.tags, vec!["favorite"]);
    }

    #[test]
    fn apply_profile_wrapper_bundled_library_satisfies_dependency() {
        // A dependency provided by a manifest-less wrapper folder's bundled
        // library is always loadable, so no standalone copy is retained.
        let tmp = tempfile::tempdir().unwrap();
        write_addon(tmp.path(), "AddonA", "LibX");
        let bundled = tmp.path().join("Libs").join("LibX");
        fs::create_dir_all(&bundled).unwrap();
        fs::write(bundled.join("LibX.txt"), "## Title: LibX\n").unwrap();
        write_addon(tmp.path(), "LibX", "");

        let result = apply_profile(tmp.path(), &profile_of(&["AddonA"]));

        assert_eq!(result.disabled, vec!["LibX"]);
        assert!(result.kept_dependencies.is_empty());
    }

    #[test]
    fn validate_backup_name_rejects_reserved_prefixes() {
        // Manual backups must not collide with internal namespaces.
        assert!(validate_backup_name("char-raid").is_err());
        assert!(validate_backup_name("auto-before-restore-2026").is_err());
        assert!(validate_backup_name("auto-cleanup-2026").is_err());
        assert!(validate_backup_name(".staging").is_err());
        // Ordinary names are fine.
        assert!(validate_backup_name("my raid backup").is_ok());
        assert!(validate_backup_name("character-naming").is_ok());
    }

    #[test]
    fn validate_relative_path_rejects_colon_tricks() {
        // NTFS alternate-data-stream syntax must be rejected.
        assert!(validate_relative_path("a.lua:stream").is_err());
        // Windows drive-relative paths (no leading separator, so not caught by
        // the absolute-path/leading-separator checks) must also be rejected.
        assert!(validate_relative_path("C:foo").is_err());
        // Ordinary relative paths remain valid.
        assert!(validate_relative_path("Sub/dir/file.lua").is_ok());
    }

    /// A world-scoped NA/EU twin file plus a non-.lua decoy.
    fn write_twin_sv(sv: &Path) {
        fs::create_dir_all(sv).unwrap();
        fs::write(
            sv.join("Addon.lua"),
            concat!(
                "Addon =\n{\n\t[\"Default\"] =\n\t{\n",
                "\t\t[\"NA Megaserver\"] =\n\t\t{\n\t\t\t[\"@me\"] =\n\t\t\t{\n",
                "\t\t\t\t[\"Bob\"] = { [\"loc\"] = \"NA\" },\n\t\t\t},\n\t\t},\n",
                "\t\t[\"EU Megaserver\"] =\n\t\t{\n\t\t\t[\"@me\"] =\n\t\t\t{\n",
                "\t\t\t\t[\"Bob\"] = { [\"loc\"] = \"EU\" },\n\t\t\t},\n\t\t},\n\t},\n}\n"
            ),
        )
        .unwrap();
        // Non-.lua file is ignored even though it mentions the name.
        fs::write(sv.join("notes.txt"), "[\"Bob\"]").unwrap();
    }

    #[test]
    fn stage_subtrees_isolates_world_scoped_twin() {
        let tmp = tempfile::tempdir().unwrap();
        let sv = tmp.path().join("SavedVariables");
        write_twin_sv(&sv);
        let staging = tmp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();

        let (matched, copied, err, worlds_spanned) =
            stage_character_subtrees(&sv, "Bob", Some("NA Megaserver"), &staging).unwrap();
        assert_eq!(matched, 1);
        assert_eq!(copied, 1);
        assert!(err.is_none());
        // Known-server backup: isolated to a single world, so span is 1.
        assert_eq!(worlds_spanned, 1);

        // The staged minimal file holds NA Bob but not EU Bob.
        let staged = fs::read(staging.join("Addon.lua")).unwrap();
        let s = String::from_utf8(staged).unwrap();
        assert!(s.contains("\"NA\""));
        assert!(!s.contains("\"EU\""));
        assert!(!staging.join("notes.txt").exists());
    }

    #[test]
    fn stage_subtrees_unknown_server_reports_worlds_spanned() {
        // An Unknown-server (world == None) backup takes world-scoped subtrees
        // from EVERY megaserver, so a same-named NA/EU twin both land in the
        // same backup. worlds_spanned must surface that as 2 so the caller can
        // warn, even though the staged file itself has no way to tell the
        // twins apart later.
        let tmp = tempfile::tempdir().unwrap();
        let sv = tmp.path().join("SavedVariables");
        write_twin_sv(&sv);
        let staging = tmp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();

        let (matched, copied, err, worlds_spanned) =
            stage_character_subtrees(&sv, "Bob", None, &staging).unwrap();
        assert_eq!(matched, 1);
        assert_eq!(copied, 1);
        assert!(err.is_none());
        assert_eq!(worlds_spanned, 2, "NA + EU twin both bundled in");
    }

    #[test]
    fn stage_subtrees_matched_zero_for_wrong_server() {
        // Backing up an NA character whose data only exists under EU yields no
        // subtree, so the backup correctly reports "no per-character data".
        let tmp = tempfile::tempdir().unwrap();
        let sv = tmp.path().join("SavedVariables");
        fs::create_dir_all(&sv).unwrap();
        fs::write(
            sv.join("Addon.lua"),
            concat!(
                "Addon =\n{\n\t[\"Default\"] =\n\t{\n",
                "\t\t[\"EU Megaserver\"] =\n\t\t{\n\t\t\t[\"@me\"] =\n\t\t\t{\n",
                "\t\t\t\t[\"Bob\"] = { [\"loc\"] = \"EU\" },\n\t\t\t},\n\t\t},\n\t},\n}\n"
            ),
        )
        .unwrap();
        let staging = tmp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();

        let (matched, _, _, _) =
            stage_character_subtrees(&sv, "Bob", Some("NA Megaserver"), &staging).unwrap();
        assert_eq!(matched, 0);
    }

    #[test]
    fn stage_subtrees_non_utf8_value_is_staged() {
        let tmp = tempfile::tempdir().unwrap();
        let sv = tmp.path().join("SavedVariables");
        fs::create_dir_all(&sv).unwrap();
        let mut bytes: Vec<u8> = concat!(
            "Addon =\n{\n\t[\"Default\"] =\n\t{\n\t\t[\"@me\"] =\n\t\t{\n",
            "\t\t\t[\"Bob\"] = { [\"icon\"] = \""
        )
        .as_bytes()
        .to_vec();
        bytes.extend_from_slice(&[0xff, 0xfe, 0x00]);
        bytes.extend_from_slice(b"\" },\n\t\t},\n\t},\n}\n");
        fs::write(sv.join("Addon.lua"), &bytes).unwrap();
        let staging = tmp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();

        let (matched, copied, err, _) =
            stage_character_subtrees(&sv, "Bob", None, &staging).unwrap();
        assert_eq!((matched, copied), (1, 1));
        assert!(err.is_none());
        let staged = fs::read(staging.join("Addon.lua")).unwrap();
        assert!(staged.contains(&0xff), "non-UTF8 byte preserved in backup");
    }

    #[test]
    fn classify_backup_for_restore_modes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let good_meta = serde_json::to_vec(&CharBackupMeta {
            version: CHAR_BACKUP_VERSION,
            character: "Bob".to_string(),
            server: "NA Megaserver".to_string(),
            worlds_spanned: 1,
        })
        .unwrap();

        // v2 marker + valid metadata -> Merge.
        let d = root.join("v2-ok");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(CHAR_BACKUP_MARKER), CHAR_BACKUP_MARKER_V2_BODY).unwrap();
        fs::write(d.join(CHAR_BACKUP_META), &good_meta).unwrap();
        assert!(matches!(
            classify_backup_for_restore(&d),
            CharRestoreMode::Merge(_)
        ));

        // v2 marker but the metadata sidecar was lost -> Refuse (NEVER whole-file,
        // which would splat minimal subtree files over live data).
        let d = root.join("v2-lost-meta");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(CHAR_BACKUP_MARKER), CHAR_BACKUP_MARKER_V2_BODY).unwrap();
        assert!(matches!(
            classify_backup_for_restore(&d),
            CharRestoreMode::Refuse(_)
        ));

        // Metadata claims a newer version than we support -> Refuse.
        let d = root.join("v2-future");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(CHAR_BACKUP_MARKER), CHAR_BACKUP_MARKER_V2_BODY).unwrap();
        fs::write(
            d.join(CHAR_BACKUP_META),
            br#"{"version":99,"character":"Bob","server":"NA Megaserver"}"#,
        )
        .unwrap();
        assert!(matches!(
            classify_backup_for_restore(&d),
            CharRestoreMode::Refuse(_)
        ));

        // v2 marker + corrupt (non-JSON) metadata -> Refuse.
        let d = root.join("v2-corrupt");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(CHAR_BACKUP_MARKER), CHAR_BACKUP_MARKER_V2_BODY).unwrap();
        fs::write(d.join(CHAR_BACKUP_META), b"not json at all").unwrap();
        assert!(matches!(
            classify_backup_for_restore(&d),
            CharRestoreMode::Refuse(_)
        ));

        // A sidecar present without a v2 marker is still treated as a
        // per-character candidate (fail-closed): valid sidecar -> Merge.
        let d = root.join("meta-only");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(CHAR_BACKUP_META), &good_meta).unwrap();
        assert!(matches!(
            classify_backup_for_restore(&d),
            CharRestoreMode::Merge(_)
        ));

        // Legacy whole-file character backup (old marker body, no sidecar) -> WholeFile.
        let d = root.join("legacy");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(CHAR_BACKUP_MARKER), b"kalpa character backup\n").unwrap();
        assert!(matches!(
            classify_backup_for_restore(&d),
            CharRestoreMode::WholeFile
        ));

        // Manual / auto backup (no marker at all) -> WholeFile.
        let d = root.join("manual");
        fs::create_dir_all(&d).unwrap();
        assert!(matches!(
            classify_backup_for_restore(&d),
            CharRestoreMode::WholeFile
        ));
    }

    #[test]
    fn char_backup_merge_restore_isolates_target() {
        use crate::saved_variables::char_backup::{build_backup_file, extract_character_blocks};

        let tmp = tempfile::tempdir().unwrap();
        let sv = tmp.path().join("SavedVariables");
        write_twin_sv(&sv);
        let live_file = sv.join("Addon.lua");
        let original = fs::read(&live_file).unwrap();

        // Build a v2 backup dir for NA Bob.
        let backup_dir = tmp.path().join("char-Bob-backup");
        fs::create_dir_all(&backup_dir).unwrap();
        let blocks = extract_character_blocks(&original, b"Bob", Some("NA Megaserver"));
        let content = build_backup_file(&blocks).unwrap();
        fs::write(backup_dir.join("Addon.lua"), &content).unwrap();
        fs::write(
            backup_dir.join(CHAR_BACKUP_MARKER),
            CHAR_BACKUP_MARKER_V2_BODY,
        )
        .unwrap();
        let meta = CharBackupMeta {
            version: CHAR_BACKUP_VERSION,
            character: "Bob".to_string(),
            server: "NA Megaserver".to_string(),
            worlds_spanned: 1,
        };
        fs::write(
            backup_dir.join(CHAR_BACKUP_META),
            serde_json::to_vec(&meta).unwrap(),
        )
        .unwrap();

        // Mutate the live file (NA and EU Bob).
        let mutated = String::from_utf8(original.clone())
            .unwrap()
            .replace("\"NA\"", "\"NA-changed\"")
            .replace("\"EU\"", "\"EU-changed\"");
        fs::write(&live_file, &mutated).unwrap();

        // Sanity: the v2 dir we built classifies as a Merge restore.
        assert!(matches!(
            classify_backup_for_restore(&backup_dir),
            CharRestoreMode::Merge(_)
        ));

        // Restore: merge only NA Bob's subtree back.
        let (restored, failed) = restore_character_subtrees_merge(&backup_dir, &sv, &meta);
        assert_eq!(restored, 1, "one file restored; failures: {failed:?}");
        assert!(failed.is_empty());

        let after = String::from_utf8(fs::read(&live_file).unwrap()).unwrap();
        assert!(after.contains("\"NA\""), "NA Bob reverted to backup");
        assert!(!after.contains("\"NA-changed\""));
        assert!(after.contains("\"EU-changed\""), "EU Bob left untouched");
    }

    #[test]
    fn recover_restores_tombstone_on_true_crash() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // True mid-finalize crash: prior backup at the tombstone, the new
        // attempt's staging still present, no char-<name> installed yet.
        let tombstone = root.join(".old-char-mychar-7");
        fs::create_dir_all(&tombstone).unwrap();
        fs::write(tombstone.join("data.lua"), b"good").unwrap();
        fs::create_dir_all(root.join(".tmp-char-mychar-7")).unwrap();

        recover_orphaned_backups(root);

        assert!(root.join("char-mychar").join("data.lua").is_file());
        assert!(!tombstone.exists());
        assert!(!root.join(".tmp-char-mychar-7").exists());
    }

    #[test]
    fn recover_does_not_resurrect_deleted_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Stale tombstone with NO matching staging and no final dir: the backup
        // was installed then deleted. It must NOT come back.
        let tombstone = root.join(".old-char-mychar-7");
        fs::create_dir_all(&tombstone).unwrap();
        fs::write(tombstone.join("data.lua"), b"old").unwrap();

        recover_orphaned_backups(root);

        assert!(!tombstone.exists());
        assert!(!root.join("char-mychar").exists());
    }

    #[test]
    fn recover_discards_stale_tombstone_when_final_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let tombstone = root.join(".old-char-mychar-7");
        fs::create_dir_all(&tombstone).unwrap();
        let final_dir = root.join("char-mychar");
        fs::create_dir_all(&final_dir).unwrap();
        fs::write(final_dir.join("new.lua"), b"new").unwrap();

        recover_orphaned_backups(root);

        assert!(!tombstone.exists());
        assert!(final_dir.join("new.lua").is_file());
    }

    #[test]
    fn finalize_backup_replace_installs_staging_on_success() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let final_dir = root.join("char-x");
        let tombstone = root.join(".old-char-x");
        let staging = root.join(".tmp-char-x");
        fs::create_dir_all(&final_dir).unwrap();
        fs::write(final_dir.join("old.lua"), b"old").unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("new.lua"), b"new").unwrap();

        finalize_backup_replace(&staging, &final_dir, &tombstone).unwrap();

        assert!(final_dir.join("new.lua").is_file());
        assert!(!final_dir.join("old.lua").exists());
        assert!(!tombstone.exists());
        assert!(!staging.exists());
    }

    #[test]
    fn finalize_backup_replace_preserves_previous_on_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let final_dir = root.join("char-x");
        let tombstone = root.join(".old-char-x");
        // Staging does NOT exist, so renaming it into place fails — exercising
        // the rollback path with a prior backup present.
        let staging = root.join(".tmp-char-x");
        fs::create_dir_all(&final_dir).unwrap();
        fs::write(final_dir.join("old.lua"), b"old").unwrap();

        let result = finalize_backup_replace(&staging, &final_dir, &tombstone);

        assert!(result.is_err());
        // The previous good backup is intact and the tombstone is cleaned up.
        assert!(final_dir.join("old.lua").is_file());
        assert_eq!(fs::read(final_dir.join("old.lua")).unwrap(), b"old");
        assert!(!tombstone.exists());
    }

    #[test]
    fn installed_set_matches_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        // Folders cased differently than the manifest's DependsOn token would be.
        make_addon_folder(dir.path(), "LUIMedia", "## Title: x\n");
        make_addon_folder(dir.path(), "LuiData", "## Title: x\n");
        let installed = build_installed_set(dir.path());
        assert!(installed.contains(&normalize_addon_name("LuiMedia")));
        assert!(installed.contains(&normalize_addon_name("luidata")));
        assert!(!installed.contains(&normalize_addon_name("Nonexistent")));
    }

    #[test]
    fn installed_set_finds_bundled_subfolders_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        // A library bundled one level deep inside another addon, oddly cased.
        let sub = dir.path().join("BigAddon").join("LIBStub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("LIBStub.txt"), "## Title: x\n").unwrap();
        let installed = build_installed_set(dir.path());
        assert!(installed.contains(&normalize_addon_name("LibStub")));
    }

    #[test]
    fn installed_set_ignores_folders_without_a_matching_manifest() {
        let dir = tempfile::tempdir().unwrap();
        // A folder named like a dependency but with no manifest (partial
        // extraction, or a plain non-addon dir) must NOT count as installed.
        std::fs::create_dir_all(dir.path().join("LibFoo")).unwrap();
        std::fs::create_dir_all(dir.path().join("RealAddon").join("libs")).unwrap();
        std::fs::write(
            dir.path().join("RealAddon").join("RealAddon.txt"),
            "## Title: x\n",
        )
        .unwrap();
        let installed = build_installed_set(dir.path());
        assert!(!installed.contains(&normalize_addon_name("LibFoo")));
        assert!(!installed.contains(&normalize_addon_name("libs")));
        assert!(installed.contains(&normalize_addon_name("RealAddon")));
    }

    #[test]
    fn installed_set_excludes_disabled_folders() {
        let dir = tempfile::tempdir().unwrap();
        // Even a fully-formed addon is excluded once disabled.
        make_addon_folder(dir.path(), "LuiMedia", "## Title: x\n");
        std::fs::rename(
            dir.path().join("LuiMedia"),
            dir.path().join("LuiMedia.disabled"),
        )
        .unwrap();
        let installed = build_installed_set(dir.path());
        assert!(!installed.contains(&normalize_addon_name("LuiMedia")));
    }

    #[test]
    fn merge_max_version_replaces_unknown_and_keeps_max() {
        let mut map: HashMap<String, Option<u32>> = HashMap::new();
        // Unknown (None) seed gets a concrete version instead of being masked.
        map.insert("libfoo".into(), None);
        merge_max_version(&mut map, "libfoo".into(), 5);
        assert_eq!(map.get("libfoo"), Some(&Some(5)));
        // Subsequent lower/higher versions keep the max.
        merge_max_version(&mut map, "libfoo".into(), 3);
        assert_eq!(map.get("libfoo"), Some(&Some(5)));
        merge_max_version(&mut map, "libfoo".into(), 9);
        assert_eq!(map.get("libfoo"), Some(&Some(9)));
    }

    #[test]
    fn installed_index_versions_cover_nested_bundled_and_wrappers() {
        let dir = tempfile::tempdir().unwrap();
        // Top-level addon with a version.
        make_addon_folder(dir.path(), "LibCombat", "## AddOnVersion: 10\n");
        // Bundled OLDER copy inside another addon must not lower the max.
        let bundled = dir.path().join("BigAddon").join("LibCombat");
        std::fs::create_dir_all(&bundled).unwrap();
        std::fs::write(bundled.join("LibCombat.txt"), "## AddOnVersion: 3\n").unwrap();
        std::fs::write(
            dir.path().join("BigAddon").join("BigAddon.txt"),
            "## Title: x\n",
        )
        .unwrap();
        // A library nested under a NON-addon top-level wrapper (`Libs/`): it must
        // appear in BOTH the installed set and the version map, so an outdated
        // check on it actually fires.
        let wrapped = dir.path().join("Libs").join("LibFoo");
        std::fs::create_dir_all(&wrapped).unwrap();
        std::fs::write(wrapped.join("LibFoo.txt"), "## AddOnVersion: 7\n").unwrap();

        let (names, versions) = build_installed_index(dir.path());
        assert!(names.contains(&normalize_addon_name("LibCombat")));
        assert!(names.contains(&normalize_addon_name("LibFoo")));
        assert_eq!(
            versions.get(&normalize_addon_name("LibCombat")),
            Some(&Some(10))
        );
        assert_eq!(
            versions.get(&normalize_addon_name("LibFoo")),
            Some(&Some(7))
        );
    }

    #[test]
    fn read_addon_version_handles_bom_and_crlf() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "\u{FEFF}## Title: A\r\n## AddOnVersion: 7221\r\n").unwrap();
        assert_eq!(read_addon_version(&path), Some(7221));
    }

    #[test]
    fn read_addon_version_tolerates_invalid_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        // An invalid UTF-8 byte elsewhere must not stop the version being read
        // (parse_manifest decodes lossily, so this must too).
        std::fs::write(&path, b"## Title: A\xFF\n## AddOnVersion: 7221\n").unwrap();
        assert_eq!(read_addon_version(&path), Some(7221));
    }

    #[test]
    fn validate_pack_id_accepts_valid_ids() {
        assert!(validate_pack_id("trial-essentials").is_ok());
        assert!(validate_pack_id("my_pack_123").is_ok());
        assert!(validate_pack_id("abc").is_ok());
        assert!(validate_pack_id("A-Z_0-9").is_ok());
    }

    #[test]
    fn validate_pack_id_rejects_path_traversal() {
        assert!(validate_pack_id("../admin").is_err());
        assert!(validate_pack_id("..%2Fadmin").is_err());
        assert!(validate_pack_id("foo/bar").is_err());
        assert!(validate_pack_id("foo\\bar").is_err());
    }

    #[test]
    fn validate_pack_id_rejects_empty() {
        assert!(validate_pack_id("").is_err());
    }

    #[test]
    fn validate_pack_id_rejects_special_chars() {
        assert!(validate_pack_id("id with spaces").is_err());
        assert!(validate_pack_id("id&param=1").is_err());
        assert!(validate_pack_id("<script>").is_err());
        assert!(validate_pack_id("id%20encoded").is_err());
    }

    #[test]
    fn validate_pack_id_rejects_over_100_chars() {
        let long_id = "a".repeat(101);
        assert!(validate_pack_id(&long_id).is_err());
        let max_id = "a".repeat(100);
        assert!(validate_pack_id(&max_id).is_ok());
    }

    #[test]
    fn validate_relative_path_accepts_valid() {
        assert!(validate_relative_path("init.lua").is_ok());
        assert!(validate_relative_path("Libs/LibAddonMenu/LAM.lua").is_ok());
    }

    #[test]
    fn validate_relative_path_rejects_traversal() {
        assert!(validate_relative_path("../secret.txt").is_err());
        assert!(validate_relative_path("foo/../../etc/passwd").is_err());
    }

    #[test]
    fn validate_relative_path_rejects_absolute() {
        assert!(validate_relative_path("C:\\Windows\\System32\\config").is_err());
        assert!(validate_relative_path("/etc/passwd").is_err());
        assert!(validate_relative_path("\\\\server\\share").is_err());
    }

    #[test]
    fn export_pack_file_rejects_non_esopack_extension() {
        let pack = EsoPackFile {
            format: "esopack".to_string(),
            version: 1,
            pack: EsoPackData {
                title: "Test".to_string(),
                description: String::new(),
                pack_type: "addon-pack".to_string(),
                tags: vec![],
                addons: vec![],
            },
            shared_at: String::new(),
            shared_by: String::new(),
            settings: HashMap::new(),
        };
        assert!(export_pack_file(pack.clone(), "C:\\test.json".to_string()).is_err());
        assert!(export_pack_file(pack, "C:\\test.exe".to_string()).is_err());
    }
}
