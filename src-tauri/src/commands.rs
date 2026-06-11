use crate::auth::{self, AuthState, AuthTokens, AuthUser};
use crate::esoui::{self, EsouiAddonDetail, EsouiAddonInfo, EsouiCategory, EsouiSearchResult};
use crate::file_hashes;
use crate::installer;
use crate::manifest::{self, AddonManifest};
use crate::manifest_cache;
use crate::metadata;
use crate::AllowedAddonsPath;
use crate::MetadataLock;
use crate::{PendingDeepLink, PendingDeepLinkPayload};
use rayon::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
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
                    if !all_installed.contains(&dep.name) && seen.insert(dep.name.clone()) {
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
                        all_installed.insert(f.clone());
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

/// Extract just the `AddOnVersion` number from a manifest file without
/// parsing the full manifest.  Returns `None` if the file can't be read
/// or doesn't contain an `AddOnVersion` line.
fn read_addon_version(manifest_path: &Path) -> Option<u32> {
    let content = fs::read_to_string(manifest_path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("## AddOnVersion:") {
            return rest.trim().parse().ok();
        }
    }
    None
}

/// Collect subfolder names (2 levels deep) inside a single addon folder,
/// mirroring how ESO discovers embedded libraries within an addon.
fn collect_subfolder_names(folder_path: &Path, out: &mut HashSet<String>) {
    let Ok(sub_entries) = fs::read_dir(folder_path) else {
        return;
    };
    for sub in sub_entries.flatten() {
        let sub_path = sub.path();
        if !sub_path.is_dir() {
            continue;
        }
        if let Some(sub_name) = sub_path.file_name().and_then(|n| n.to_str()) {
            out.insert(sub_name.to_string());
        }
        if let Ok(sub2_entries) = fs::read_dir(&sub_path) {
            for sub2 in sub2_entries.flatten() {
                if sub2.path().is_dir() {
                    if let Some(sub2_name) = sub2.path().file_name().and_then(|n| n.to_str()) {
                        out.insert(sub2_name.to_string());
                    }
                }
            }
        }
    }
}

/// Build the set of all "installed" names visible to ESO from the addons directory.
///
/// ESO scans top-level addon folders plus 2 levels of subfolders (3 levels total
/// from the AddOns root), matching ESO's own resolution depth on PC.
/// Disabled folders (ending in `.disabled`) are excluded.
pub(crate) fn build_installed_set(addons_dir: &Path) -> HashSet<String> {
    let Ok(entries) = fs::read_dir(addons_dir) else {
        return HashSet::new();
    };
    let mut installed = HashSet::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name.ends_with(".disabled") {
            continue;
        }
        installed.insert(name);
        collect_subfolder_names(&path, &mut installed);
    }
    installed
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

    let installed = build_installed_set(addons_dir);

    // Load metadata and clean up stale entries:
    // - Remove entries for addon folders that no longer exist on disk
    // - Deduplicate entries with the same esoui_id (keep the one that exists)
    let mut store = metadata::load_metadata(addons_dir);
    let stale: Vec<String> = store
        .addons
        .keys()
        .filter(|name| {
            !addons_dir.join(name).is_dir() && !addons_dir.join(format!("{name}.disabled")).is_dir()
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

    // Build a map of folder_name → addon_version for version constraint checking.
    let mut version_map: HashMap<String, Option<u32>> = addons
        .iter()
        .map(|a| (a.folder_name.clone(), a.addon_version))
        .collect();

    // Also scan bundled sub-libraries (2 levels deep, matching ESO's resolution
    // depth and collect_subfolder_names) and keep the MAX version per name.
    // This prevents false "outdated" flags when a newer copy is bundled inside
    // another addon.
    for addon in &addons {
        let addon_dir = addons_dir.join(&addon.folder_name);
        for depth_1 in fs::read_dir(&addon_dir).into_iter().flatten().flatten() {
            let d1 = depth_1.path();
            if !d1.is_dir() {
                continue;
            }
            if let Some(name) = d1.file_name().and_then(|n| n.to_str()) {
                if let Some(manifest) = find_manifest_in(&d1, name) {
                    if let Some(ver) = read_addon_version(&manifest) {
                        let entry = version_map.entry(name.to_string()).or_insert(Some(0));
                        if let Some(cur) = entry {
                            *cur = (*cur).max(ver);
                        }
                    }
                }
            }
            // Second level: scan sub-subdirectories
            for depth_2 in fs::read_dir(&d1).into_iter().flatten().flatten() {
                let d2 = depth_2.path();
                if !d2.is_dir() {
                    continue;
                }
                if let Some(name) = d2.file_name().and_then(|n| n.to_str()) {
                    if let Some(manifest) = find_manifest_in(&d2, name) {
                        if let Some(ver) = read_addon_version(&manifest) {
                            let entry = version_map.entry(name.to_string()).or_insert(Some(0));
                            if let Some(cur) = entry {
                                *cur = (*cur).max(ver);
                            }
                        }
                    }
                }
            }
        }
    }

    // Check for missing/outdated dependencies and enrich with ESOUI ID
    for addon in &mut addons {
        addon.missing_dependencies = addon
            .depends_on
            .iter()
            .filter(|dep| !installed.contains(&dep.name))
            .map(|dep| dep.name.clone())
            .collect();

        addon.outdated_dependencies = addon
            .depends_on
            .iter()
            .filter(|dep| {
                let Some(min) = dep.min_version else {
                    return false;
                };
                if !installed.contains(&dep.name) {
                    return false;
                }
                match version_map.get(&dep.name) {
                    Some(Some(installed_ver)) => *installed_ver < min,
                    _ => false,
                }
            })
            .map(|dep| dep.name.clone())
            .collect();

        if let Some(meta) = store.addons.get(&addon.folder_name) {
            addon.esoui_id = Some(meta.esoui_id);
            addon.tags = meta.tags.clone();
            addon.esoui_last_update = meta.esoui_last_update;
        }

        if let Some(hash_manifest) = file_hashes::load_hash_manifest(addons_dir, &addon.folder_name)
        {
            addon.modified_file_count = hash_manifest.modified_files.len() as u32;
        }
    }

    addons.sort_by_key(|a| a.title.to_lowercase());

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
    let dep_folders = installer::extract_addon_zip(dep_tmp.path(), addons_dir)
        .map_err(|_| format!("Failed to install {dep_name}: extract_failed"))?;

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
}

/// Phase 1 of check_for_updates: compare local metadata against the ESOUI API
/// lookup table. Must be called under the metadata lock.
fn check_for_updates_metadata(
    addons_dir: &Path,
    api_lookup: &HashMap<String, esoui::ApiAddonLookup>,
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
        });
    }

    if metadata_changed {
        if let Err(e) = metadata::save_metadata(addons_dir, &store) {
            eprintln!("Warning: failed to save metadata after update check: {e}");
        }
    }

    Ok(pending)
}

#[tauri::command]
pub async fn update_addon(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    addons_path: String,
    esoui_id: u32,
    api_version: Option<String>,
) -> Result<InstallResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let lock = meta_lock.0.clone();
    tokio::task::spawn_blocking(move || {
        // Network I/O outside the lock: fetch info + download ZIP
        let info = esoui::fetch_addon_info(esoui_id)?;
        let tmp_file = esoui::download_addon(&info.download_url, None)?;

        // Acquire lock only for extract + metadata update
        let _guard = lock
            .lock()
            .map_err(|_| "Internal metadata lock error".to_string())?;
        update_addon_blocking(
            &addons_dir,
            esoui_id,
            api_version.as_deref(),
            info,
            tmp_file,
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
) -> Result<InstallResult, String> {
    // Extract the downloaded ZIP
    let installed_folders = installer::extract_addon_zip(tmp_file.path(), addons_dir)?;

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
    hash.iter().take(16).map(|b| format!("{b:02x}")).collect()
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
            (Some(stored), Some(disk)) => stored != disk,
            (Some(_), None) => true, // file deleted
            (None, _) => false,      // no stored hash = no baseline = treat as unmodified
        };

        let upstream_changed = match stored_hash {
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

        // The ZIP hash map isn't reused here: extraction happens later in a
        // separate `update_addon_with_decisions` invocation, after the user
        // resolves conflicts, by which point this map is long gone.
        let (report, _zip_hashes) = build_conflict_report(
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
                        // ZIP map unused: extraction is deferred to the
                        // interactive `update_addon_with_decisions` call.
                        Ok((report, _zip_hashes)) => {
                            if let Ok(mut map) = pending_clone.lock() {
                                map.insert(
                                    session_id.clone(),
                                    crate::PendingUpdate {
                                        zip_path: dl.kept_path,
                                        folder_name: folder_name.clone(),
                                        esoui_id: dl.esoui_id,
                                        update_version: version.to_string(),
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
pub async fn update_addon_with_decisions(
    state: tauri::State<'_, AllowedAddonsPath>,
    meta_lock: tauri::State<'_, MetadataLock>,
    pending: tauri::State<'_, crate::PendingUpdates>,
    addons_path: String,
    session_id: String,
    decisions: Vec<FileDecision>,
) -> Result<InstallResult, String> {
    for d in &decisions {
        validate_relative_path(&d.relative_path)?;
    }
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let pending_clone = pending.0.clone();
    let lock = meta_lock.0.clone();

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
        let outcome = update_with_decisions_inner(&addons_dir, &pu, &decisions);

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

    // Hash the ZIP once. This map becomes the new baseline after extraction
    // (reused by record_hashes_with_zip_baseline), and also supplies the
    // upstream hashes for kept "keep_mine" files so the user's edit stays
    // detectable on the next update cycle.
    let zip_hashes = file_hashes::hash_zip_entries(&pu.zip_path, &pu.folder_name)?;
    let hash_overrides: Option<HashMap<String, String>> = if kept_files.is_empty() {
        None
    } else {
        let overrides: HashMap<String, String> = kept_files
            .iter()
            .filter_map(|p| zip_hashes.get(p).map(|h| (p.clone(), h.clone())))
            .collect();
        (!overrides.is_empty()).then_some(overrides)
    };

    // Extract with selective skipping
    let installed_folders = if skip_files.is_empty() {
        installer::extract_addon_zip(&pu.zip_path, addons_dir)?
    } else {
        installer::extract_addon_zip_selective(&pu.zip_path, addons_dir, &skip_files)?
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

        // Re-hash the single file and update the manifest cache
        if let Some(mut manifest) = file_hashes::load_hash_manifest(&addons_dir, &folder_name) {
            let new_hash = file_hashes::compute_addon_hashes(&addons_dir.join(&folder_name))?;
            if let Some(hash) = new_hash.get(&relative_path.replace('\\', "/")) {
                let key = relative_path.replace('\\', "/");
                let is_modified = manifest.files.get(&key).map(|h| h != hash).unwrap_or(true);
                if is_modified && !manifest.modified_files.contains(&key) {
                    manifest.modified_files.push(key);
                    manifest.modified_files.sort();
                } else if !is_modified {
                    manifest.modified_files.retain(|f| f != &key);
                }
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
    api_lookup: &HashMap<String, esoui::ApiAddonLookup>,
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
                let version = already_tracked
                    .map(|m| m.installed_version.clone())
                    .unwrap_or_else(|| read_local_version(addons_dir, &folder_name));
                let download_url = already_tracked
                    .map(|m| m.download_url.clone())
                    .unwrap_or_else(|| api_entry.file_info_uri.clone());
                metadata::record_install_ext(
                    &mut store,
                    &folder_name,
                    api_entry.esoui_id,
                    &version,
                    &download_url,
                    api_entry.last_update,
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

    metadata::save_metadata(addons_dir, &store)?;

    Ok(ImportResult {
        installed,
        failed,
        skipped,
        errors,
    })
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
pub fn list_backups(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<Vec<BackupInfo>, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let backups = backups_dir(&addons_dir);
    if !backups.is_dir() {
        return Ok(Vec::new());
    }

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

        // Count files and total size
        let mut file_count: u32 = 0;
        let mut total_size: u64 = 0;
        if let Ok(files) = fs::read_dir(&path) {
            for f in files.flatten() {
                if f.path().is_file() {
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

        let kind = if name.starts_with("char-") {
            BackupKind::Character
        } else if name.starts_with("auto-before-restore-") {
            BackupKind::AutoBeforeRestore
        } else {
            BackupKind::Manual
        };

        results.push(BackupInfo {
            name,
            created_at,
            created_at_epoch,
            file_count,
            total_size,
            kind,
        });
    }

    results.sort_by_key(|b| std::cmp::Reverse(b.created_at_epoch));
    Ok(results)
}

#[tauri::command]
pub async fn create_backup(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    backup_name: String,
) -> Result<BackupInfo, String> {
    validate_name(&backup_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
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

        fs::create_dir_all(&backup_path).map_err(|e| format!("Failed to create backup: {e}"))?;

        // Copy all .lua files from SavedVariables
        let mut file_count: u32 = 0;
        let mut total_size: u64 = 0;
        let entries =
            fs::read_dir(&sv_dir).map_err(|e| format!("Failed to read SavedVariables: {e}"))?;

        let mut failed: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name() {
                    let dest = backup_path.join(name);
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
            return Err(format!(
                "Backup incomplete — {} file(s) failed to copy: {}",
                failed.len(),
                failed.join(", ")
            ));
        }

        let created_at_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let created_at = metadata::format_timestamp(created_at_epoch);

        let kind = if backup_name.starts_with("char-") {
            BackupKind::Character
        } else if backup_name.starts_with("auto-before-restore-") {
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

/// Remove oldest `auto-before-restore-*` snapshots, keeping at most `keep` of the most recent.
/// Errors are logged but do not fail the caller — pruning is best-effort.
fn prune_auto_snapshots(backups_dir: &std::path::Path, keep: usize) {
    let prefix = "auto-before-restore-";
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

/// Restore a backup, but first capture the user's current SavedVariables into a
/// timestamped "auto-before-restore-…" snapshot so the restore can be undone.
/// If the user has no current SavedVariables, the snapshot step is skipped.
#[tauri::command]
pub async fn restore_backup_safe(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    backup_name: String,
) -> Result<SafeRestoreResult, String> {
    validate_name(&backup_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    tokio::task::spawn_blocking(move || {
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
            });

            // Keep only the 3 most recent auto-before-restore snapshots to prevent
            // unbounded disk growth (SavedVariables can reach 1-2 GB on trade-addon-heavy accounts).
            prune_auto_snapshots(&backups_dir(&addons_dir), 3);
        }
    }

    fs::create_dir_all(&sv_dir)
        .map_err(|e| format!("Failed to create SavedVariables folder: {e}"))?;

    let mut restored: u32 = 0;
    let mut failed: Vec<String> = Vec::new();
    let entries = fs::read_dir(&backup_path).map_err(|e| format!("Failed to read backup: {e}"))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(name) = path.file_name() {
                let dest = sv_dir.join(name);
                match fs::copy(&path, &dest) {
                    Ok(_) => restored += 1,
                    Err(e) => {
                        failed.push(format!("{}: {}", name.to_string_lossy(), e));
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
pub fn delete_backup(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    backup_name: String,
) -> Result<(), String> {
    validate_name(&backup_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let backup_path = backups_dir(&addons_dir).join(&backup_name);

    if !backup_path.is_dir() {
        return Err(format!("Backup '{backup_name}' not found."));
    }

    fs::remove_dir_all(&backup_path).map_err(|e| format!("Failed to delete backup: {e}"))
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

fn load_profiles(addons_dir: &std::path::Path) -> ProfileStore {
    metadata::load_json_with_backup(&profiles_path(addons_dir))
}

fn save_profiles(addons_dir: &std::path::Path, store: &ProfileStore) -> Result<(), String> {
    metadata::save_json_with_backup(&profiles_path(addons_dir), store)
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

    // Snapshot currently enabled addons (those with manifests in the AddOns folder)
    let mut enabled: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(&addons_dir) {
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
            if find_manifest(&addons_dir, &folder_name).is_some() {
                enabled.push(folder_name);
            }
        }
    }
    enabled.sort();

    let profile = AddonProfile {
        name: profile_name,
        enabled_addons: enabled,
        created_at: metadata::format_timestamp(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        ),
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

        let enabled_set: HashSet<String> = profile.enabled_addons.iter().cloned().collect();

        let mut disabled: Vec<String> = Vec::new();
        let mut enabled: Vec<String> = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        let mut seen_on_disk: HashSet<String> = HashSet::new();

        if let Ok(entries) = fs::read_dir(&addons_dir) {
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

                let is_disabled = folder_name.ends_with(".disabled");
                let base_name = folder_name
                    .strip_suffix(".disabled")
                    .unwrap_or(&folder_name)
                    .to_string();

                seen_on_disk.insert(base_name.clone());

                if enabled_set.contains(&base_name) {
                    // Should be enabled
                    if is_disabled {
                        let new_path = addons_dir.join(&base_name);
                        match fs::rename(&path, &new_path) {
                            Ok(_) => enabled.push(base_name),
                            Err(e) => failed.push(format!("{base_name} (enable: {e})")),
                        }
                    }
                } else {
                    // Should be disabled
                    if !is_disabled && find_manifest(&addons_dir, &folder_name).is_some() {
                        let new_path = addons_dir.join(format!("{folder_name}.disabled"));
                        match fs::rename(&path, &new_path) {
                            Ok(_) => disabled.push(folder_name),
                            Err(e) => failed.push(format!("{folder_name} (disable: {e})")),
                        }
                    }
                }
            }
        }

        // Report addons referenced in the profile that no longer exist on disk
        let missing: Vec<String> = profile
            .enabled_addons
            .iter()
            .filter(|name| !seen_on_disk.contains(name.as_str()))
            .cloned()
            .collect();

        store.active_profile = Some(profile_name);
        save_profiles(&addons_dir, &store)?;

        Ok(ActivateProfileResult {
            enabled,
            disabled,
            failed,
            missing,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
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

// ─── Multi-Character SavedVariables ──────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterInfo {
    pub server: String,
    pub name: String,
}

#[tauri::command]
pub fn list_characters(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<Vec<CharacterInfo>, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let settings_path = addons_dir
        .parent()
        .map(|p| p.join("AddOnSettings.txt"))
        .ok_or("Could not find AddOnSettings.txt.")?;

    if !settings_path.exists() {
        return Err("AddOnSettings.txt not found.".to_string());
    }

    let content = fs::read_to_string(&settings_path)
        .map_err(|e| format!("Failed to read AddOnSettings.txt: {e}"))?;

    let mut characters: Vec<CharacterInfo> = Vec::new();
    let skip_prefixes = ["Version", "Acknowledged", "AddOnsEnabled"];

    for line in content.lines() {
        let Some(line) = line.strip_prefix('#') else {
            continue;
        };
        if skip_prefixes.iter().any(|p| line.starts_with(p)) {
            continue;
        }
        if let Some(pos) = line.find('-') {
            let server = line[..pos].trim().to_string();
            let name = line[pos + 1..].trim().to_string();
            if !server.is_empty() && !name.is_empty() {
                // Deduplicate
                if !characters
                    .iter()
                    .any(|c| c.server == server && c.name == name)
                {
                    characters.push(CharacterInfo { server, name });
                }
            }
        }
    }

    Ok(characters)
}

#[tauri::command]
pub fn backup_character_settings(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    character_name: String,
    backup_name: String,
) -> Result<u32, String> {
    if character_name.trim().is_empty() {
        return Err("Character name cannot be empty.".to_string());
    }
    if character_name.len() < 3 {
        return Err("Character name must be at least 3 characters.".to_string());
    }
    validate_name(&backup_name)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let sv_dir = saved_variables_dir(&addons_dir);
    if !sv_dir.is_dir() {
        return Err("SavedVariables folder not found.".to_string());
    }

    let backups = backups_dir(&addons_dir).join(format!("char-{backup_name}"));
    fs::create_dir_all(&backups).map_err(|e| format!("Failed to create backup folder: {e}"))?;

    // Copy all SavedVariables files that contain this character's data.
    // Search within bracket-quote delimiters to avoid false positives
    // (e.g. a character named "Lib" matching "LibStub" in addon code).
    // Only scan the first 10,000 lines — character names appear in the
    // first few hundred lines, so reading entire 100MB+ files is wasteful.
    let needle = format!("[\"{character_name}\"]");
    let max_lines: usize = 10_000;
    let mut count: u32 = 0;
    if let Ok(entries) = fs::read_dir(&sv_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let file = match fs::File::open(&path) {
                    Ok(f) => f,
                    Err(_) => continue,
                };
                let reader = BufReader::new(file);
                let mut found = false;
                for (i, line_result) in reader.lines().enumerate() {
                    if i >= max_lines {
                        break;
                    }
                    let line = match line_result {
                        Ok(l) => l,
                        Err(_) => break,
                    };
                    if line.contains(&needle) {
                        found = true;
                        break;
                    }
                }
                if found {
                    if let Some(name) = path.file_name() {
                        let dest = backups.join(name);
                        if fs::copy(&path, &dest).is_ok() {
                            count += 1;
                        }
                    }
                }
            }
        }
    }

    Ok(count)
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
pub fn migrate_from_minion(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<MinionMigrationResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    let result = safe_migration::execute_migration(&addons_dir)?;
    Ok(MinionMigrationResult {
        found: true,
        addon_count: result.addon_count,
        imported: result.imported,
        already_tracked: result.already_tracked,
    })
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
pub fn migration_dry_run(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<safe_migration::DryRunResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    safe_migration::dry_run_migration(&addons_dir)
}

#[tauri::command]
pub fn migration_execute(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<safe_migration::MigrationResult, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    safe_migration::execute_migration(&addons_dir)
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
pub fn restore_snapshot(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    snapshot_id: String,
) -> Result<u32, String> {
    validate_name(&snapshot_id)?;
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    safe_migration::restore_snapshot(&addons_dir, &snapshot_id)
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
pub fn backup_minion_config(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<u32, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;
    safe_migration::backup_minion_config(&addons_dir)
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
                save_auth_tokens(&app, &new_tokens);
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

fn save_auth_tokens(_app: &tauri::AppHandle, tokens: &AuthTokens) {
    crate::token_store::save_tokens(tokens);
}

fn clear_auth_tokens(_app: &tauri::AppHandle) {
    crate::token_store::clear_tokens();
}

// ── Auth Commands ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn auth_login(
    state: tauri::State<'_, AuthState>,
    app: tauri::AppHandle,
) -> Result<AuthUser, String> {
    let tokens = tokio::task::spawn_blocking(auth::login)
        .await
        .map_err(|e| format!("Task failed: {e}"))??;

    let user = AuthUser {
        user_id: tokens.user_id.clone(),
        user_name: tokens.user_name.clone(),
    };

    // Save to store
    save_auth_tokens(&app, &tokens);

    // Update in-memory state
    *state
        .tokens
        .lock()
        .map_err(|e| format!("Auth lock poisoned: {e}"))? = Some(tokens);

    Ok(user)
}

#[tauri::command]
pub async fn auth_logout(
    state: tauri::State<'_, AuthState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Clear in-memory state
    *state
        .tokens
        .lock()
        .map_err(|e| format!("Auth lock poisoned: {e}"))? = None;

    // Clear from store
    clear_auth_tokens(&app);

    Ok(())
}

#[tauri::command]
pub async fn auth_get_user(
    state: tauri::State<'_, AuthState>,
    app: tauri::AppHandle,
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
            // Tokens were refreshed — save them
            let user = AuthUser {
                user_id: new_tokens.user_id.clone(),
                user_name: new_tokens.user_name.clone(),
            };

            save_auth_tokens(&app, &new_tokens);

            *state
                .tokens
                .lock()
                .map_err(|e| format!("Auth lock poisoned: {e}"))? = Some(new_tokens);
            Ok(Some(user))
        }
        Ok(None) => {
            // Token still valid
            Ok(Some(AuthUser {
                user_id: tokens.user_id,
                user_name: tokens.user_name,
            }))
        }
        Err(_) => {
            // Refresh failed — clear session
            *state
                .tokens
                .lock()
                .map_err(|e| format!("Auth lock poisoned: {e}"))? = None;
            clear_auth_tokens(&app);
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
                save_auth_tokens(&app, &new_tokens);
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
                save_auth_tokens(&app, &new_tokens);
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
                save_auth_tokens(&app, &new_tokens);
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
                save_auth_tokens(&app, &new_tokens);
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
    clear_auth_tokens(&app);

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
                save_auth_tokens(&app, &new_tokens);
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
            let (scrubbed, report) = scrub(&tree, &ctx);

            let account_wide_only = strip_per_character_data(&scrubbed);
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
    tree: &crate::saved_variables::types::SvTreeNode,
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
    use crate::saved_variables::parser::parse_sv_file;
    use crate::saved_variables::scrub::{detect_identities_from_tree, ScrubContext};

    let addons_dir = require_allowed_path(&state, &addons_path)?;

    tokio::task::spawn_blocking(move || {
        let sv_dir = sv_io::saved_variables_dir(&addons_dir);
        let mut merged = ScrubContext::default();

        let entries = match fs::read_dir(&sv_dir) {
            Ok(e) => e,
            Err(_) => return Ok(merged),
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("lua") {
                continue;
            }
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown.lua")
                .to_string();
            let tree = match parse_sv_file(&content, &file_name) {
                Ok(t) => t,
                Err(_) => continue,
            };

            let ctx = detect_identities_from_tree(&tree);
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

        Ok(merged)
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
        sv_io::delete_saved_variables_blocking(&addons_dir, &file_names)
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
        let (scrubbed, report) =
            crate::saved_variables::scrub::scrub(&response.tree, &effective_ctx);
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
