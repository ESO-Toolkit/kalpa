use crate::metadata;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;

// ─── Snapshot types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotManifest {
    pub id: String,
    pub label: String,
    pub created_at: String,
    pub source_paths: Vec<String>,
    pub file_count: u32,
    pub total_size: u64,
    pub archive_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotStore {
    pub version: u32,
    pub snapshots: Vec<SnapshotManifest>,
}

impl Default for SnapshotStore {
    fn default() -> Self {
        Self {
            version: 1,
            snapshots: Vec::new(),
        }
    }
}

// ─── Transaction log types ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpLogEntry {
    pub operation: String,
    pub started_at: String,
    pub finished_at: String,
    pub status: String,
    pub snapshot_id: Option<String>,
    pub files_created: Vec<String>,
    pub files_modified: Vec<String>,
    pub details: String,
}

// ─── Dry-run types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DryRunAddon {
    pub folder_name: String,
    pub esoui_id: u32,
    pub minion_version: String,
    pub status: String, // "will_track", "already_tracked", "missing_on_disk"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DryRunResult {
    pub will_track: Vec<DryRunAddon>,
    pub already_tracked: Vec<DryRunAddon>,
    pub missing_on_disk: Vec<DryRunAddon>,
    pub unmanaged_on_disk: Vec<String>,
}

// ─── Integrity check types ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrityResult {
    pub addons_folder_ok: bool,
    pub saved_variables_ok: bool,
    pub addon_count: u32,
    pub issues: Vec<String>,
}

// ─── Precondition check types ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreconditionResult {
    pub eso_running: bool,
    pub minion_running: bool,
    pub minion_found: bool,
    pub addons_path_valid: bool,
    pub saved_variables_exists: bool,
    pub warnings: Vec<String>,
}

// ─── Snapshot directory helpers ─────────────────────────────────────────────

/// Root directory for Kalpa snapshots: `{live}/KalpaBackups/`
fn snapshots_root(addons_dir: &Path) -> PathBuf {
    let parent = addons_dir.parent().unwrap_or(addons_dir);
    parent.join("KalpaBackups")
}

fn snapshot_store_path(addons_dir: &Path) -> PathBuf {
    snapshots_root(addons_dir).join("snapshots.json")
}

fn load_snapshot_store(addons_dir: &Path) -> SnapshotStore {
    metadata::load_json_with_backup(&snapshot_store_path(addons_dir))
}

fn save_snapshot_store(addons_dir: &Path, store: &SnapshotStore) -> Result<(), String> {
    let root = snapshots_root(addons_dir);
    fs::create_dir_all(&root).map_err(|e| format!("Failed to create KalpaBackups: {}", e))?;
    metadata::save_json_with_backup(&snapshot_store_path(addons_dir), store)
}

/// Generate a timestamp-based snapshot ID with millisecond precision to avoid collisions.
fn snapshot_id(label_hint: &str) -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let ts = metadata::format_timestamp(dur.as_secs());
    let millis = dur.as_millis() % 1000;
    let safe_label: String = label_hint
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    format!("{}-{:03}-{}", ts.replace(':', "-"), millis, safe_label)
}

// ─── Transaction log helpers ────────────────────────────────────────────────

fn ops_log_path(addons_dir: &Path) -> PathBuf {
    snapshots_root(addons_dir).join("kalpa-ops.jsonl")
}

/// Maximum number of entries to keep in the ops log. When exceeded, the log is
/// trimmed to the most recent entries.
const OPS_LOG_MAX_ENTRIES: usize = 500;

pub fn append_op_log(addons_dir: &Path, entry: &OpLogEntry) -> Result<(), String> {
    let root = snapshots_root(addons_dir);
    fs::create_dir_all(&root).map_err(|e| format!("Failed to create KalpaBackups: {}", e))?;
    let path = ops_log_path(addons_dir);

    // Rotate: if the log has grown too large, trim to the most recent entries
    if let Ok(existing) = fs::read_to_string(&path) {
        let lines: Vec<&str> = existing.lines().collect();
        if lines.len() >= OPS_LOG_MAX_ENTRIES {
            // Keep the most recent half to avoid trimming on every append
            let keep_from = lines.len() - OPS_LOG_MAX_ENTRIES / 2;
            let trimmed = lines[keep_from..].join("\n");
            let _ = fs::write(&path, format!("{}\n", trimmed));
        }
    }

    let line = serde_json::to_string(entry)
        .map_err(|e| format!("Failed to serialize log entry: {}", e))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("Failed to open ops log: {}", e))?;
    writeln!(file, "{}", line).map_err(|e| format!("Failed to write ops log: {}", e))?;
    Ok(())
}

fn now_timestamp() -> String {
    metadata::format_timestamp(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
}

// ─── Process detection ──────────────────────────────────────────────────────

/// Known process names that we check for. Using an enum prevents command
/// injection if someone were to call `is_process_running` with untrusted input,
/// since only these known-safe values are ever interpolated into shell commands.
#[derive(Debug, Clone, Copy)]
enum KnownProcess {
    Eso64,
    Eso,
    Minion,
    MinionUnix,
}

impl KnownProcess {
    fn name(self) -> &'static str {
        match self {
            KnownProcess::Eso64 => "eso64.exe",
            KnownProcess::Eso => "eso.exe",
            KnownProcess::Minion => "Minion.exe",
            KnownProcess::MinionUnix => "minion",
        }
    }
}

#[cfg(target_os = "windows")]
fn is_process_running(process: KnownProcess) -> bool {
    use std::process::Command;
    let name = process.name();
    Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {}", name), "/NH"])
        .output()
        .map(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.contains(name)
        })
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn is_process_running(process: KnownProcess) -> bool {
    use std::process::Command;
    let name = process.name();
    Command::new("pgrep")
        .arg("-i")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ─── Core snapshot implementation ───────────────────────────────────────────

/// Create a ZIP snapshot of the specified directories and files.
/// Returns the snapshot manifest.
fn create_zip_snapshot(
    label: &str,
    addons_dir: &Path,
    include_addons: bool,
    include_saved_vars: bool,
    include_settings: bool,
) -> Result<SnapshotManifest, String> {
    let root = snapshots_root(addons_dir);
    fs::create_dir_all(&root).map_err(|e| format!("Failed to create KalpaBackups: {}", e))?;

    let id = snapshot_id(label);
    let archive_path = root.join(format!("{}.zip", id));
    let tmp_path = root.join(format!("{}.zip.tmp", id));

    let file = fs::File::create(&tmp_path)
        .map_err(|e| format!("Failed to create snapshot archive: {}", e))?;

    // Use a closure so we can clean up tmp_path on any failure
    let build_zip = || -> Result<(Vec<String>, u32, u64), String> {
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(6));

        let parent = addons_dir.parent().unwrap_or(addons_dir);
        let mut file_count: u32 = 0;
        let mut total_size: u64 = 0;
        let mut source_paths: Vec<String> = Vec::new();

        // AddOns folder
        if include_addons {
            source_paths.push("AddOns".to_string());
            let result = add_dir_to_zip(&mut zip, addons_dir, "AddOns", &options)?;
            file_count += result.0;
            total_size += result.1;
        }

        // SavedVariables folder
        if include_saved_vars {
            let sv_dir = parent.join("SavedVariables");
            if sv_dir.is_dir() {
                source_paths.push("SavedVariables".to_string());
                let result = add_dir_to_zip(&mut zip, &sv_dir, "SavedVariables", &options)?;
                file_count += result.0;
                total_size += result.1;
            }
        }

        // Settings files
        if include_settings {
            for filename in &["UserSettings.txt", "AddOnSettings.txt"] {
                let settings_file = parent.join(filename);
                if settings_file.is_file() {
                    source_paths.push(filename.to_string());
                    let data = fs::read(&settings_file)
                        .map_err(|e| format!("Failed to read {}: {}", filename, e))?;
                    zip.start_file(*filename, options)
                        .map_err(|e| format!("Failed to add {} to archive: {}", filename, e))?;
                    zip.write_all(&data)
                        .map_err(|e| format!("Failed to write {} to archive: {}", filename, e))?;
                    file_count += 1;
                    total_size += data.len() as u64;
                }
            }
        }

        zip.finish()
            .map_err(|e| format!("Failed to finalize snapshot archive: {}", e))?;

        Ok((source_paths, file_count, total_size))
    };

    let (source_paths, file_count, total_size) = match build_zip() {
        Ok(result) => result,
        Err(e) => {
            // Clean up the .tmp file on failure
            let _ = fs::remove_file(&tmp_path);
            return Err(e);
        }
    };

    // Compute SHA-256 of the archive
    let sha256 = match sha256_file(&tmp_path) {
        Ok(h) => h,
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            return Err(e);
        }
    };

    // Atomic rename
    fs::rename(&tmp_path, &archive_path)
        .map_err(|e| format!("Failed to finalize snapshot: {}", e))?;

    // Record in snapshot store
    let manifest = SnapshotManifest {
        id,
        label: label.to_string(),
        created_at: now_timestamp(),
        source_paths,
        file_count,
        total_size,
        archive_sha256: sha256,
    };
    let mut store = load_snapshot_store(addons_dir);
    store.snapshots.push(manifest.clone());
    save_snapshot_store(addons_dir, &store)?;

    Ok(manifest)
}

/// Recursively add a directory to a ZIP archive.
fn add_dir_to_zip(
    zip: &mut zip::ZipWriter<fs::File>,
    dir: &Path,
    prefix: &str,
    options: &SimpleFileOptions,
) -> Result<(u32, u64), String> {
    let mut file_count: u32 = 0;
    let mut total_size: u64 = 0;

    let mut stack: Vec<(PathBuf, String)> = vec![(dir.to_path_buf(), prefix.to_string())];

    while let Some((current_dir, current_prefix)) = stack.pop() {
        let entries = fs::read_dir(&current_dir)
            .map_err(|e| format!("Failed to read {}: {}", current_dir.display(), e))?;

        for entry in entries.flatten() {
            let path = entry.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let zip_path = format!("{}/{}", current_prefix, name);

            if path.is_dir() {
                // Skip symlinks
                if path.read_link().is_ok() {
                    continue;
                }
                stack.push((path, zip_path));
            } else if path.is_file() {
                let data = match fs::read(&path) {
                    Ok(d) => d,
                    Err(_) => continue, // Skip unreadable files (e.g. locked by another process)
                };
                zip.start_file(&zip_path, *options)
                    .map_err(|e| format!("Failed to add '{}' to archive: {}", zip_path, e))?;
                zip.write_all(&data)
                    .map_err(|e| format!("Failed to write '{}' to archive: {}", zip_path, e))?;
                file_count += 1;
                total_size += data.len() as u64;
            }
        }
    }

    Ok((file_count, total_size))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|e| format!("Failed to open file for hashing: {}", e))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("Failed to read file for hashing: {}", e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Phase 0: Check preconditions before migration.
pub fn check_preconditions(addons_dir: &Path) -> PreconditionResult {
    let parent = addons_dir.parent().unwrap_or(addons_dir);
    let eso_running =
        is_process_running(KnownProcess::Eso64) || is_process_running(KnownProcess::Eso);
    let minion_running =
        is_process_running(KnownProcess::Minion) || is_process_running(KnownProcess::MinionUnix);
    let minion_found = crate::commands::find_minion_xml().is_some();
    let addons_path_valid = addons_dir.is_dir();
    let saved_variables_exists = parent.join("SavedVariables").is_dir();

    let mut warnings = Vec::new();
    if eso_running {
        warnings
            .push("ESO appears to be running. Please close the game before migrating.".to_string());
    }
    if minion_running {
        warnings.push(
            "Minion appears to be running. Consider closing it before proceeding.".to_string(),
        );
    }
    if !addons_path_valid {
        warnings.push("AddOns folder not found.".to_string());
    }

    PreconditionResult {
        eso_running,
        minion_running,
        minion_found,
        addons_path_valid,
        saved_variables_exists,
        warnings,
    }
}

/// Phase 1: Create a full pre-migration snapshot.
pub fn create_pre_migration_snapshot(
    addons_dir: &Path,
    include_addons: bool,
) -> Result<SnapshotManifest, String> {
    let start = now_timestamp();
    let label = "Pre-migration";

    let manifest = create_zip_snapshot(label, addons_dir, include_addons, true, true)?;

    // Log the operation
    let _ = append_op_log(
        addons_dir,
        &OpLogEntry {
            operation: "pre_migration_snapshot".to_string(),
            started_at: start,
            finished_at: now_timestamp(),
            status: "success".to_string(),
            snapshot_id: Some(manifest.id.clone()),
            files_created: vec![format!("{}.zip", manifest.id)],
            files_modified: vec![],
            details: format!(
                "Snapshot {} files, {} bytes, SHA-256: {}",
                manifest.file_count, manifest.total_size, manifest.archive_sha256
            ),
        },
    );

    Ok(manifest)
}

/// Phase 2: Dry-run migration — compare Minion data with disk state.
pub fn dry_run_migration(addons_dir: &Path) -> Result<DryRunResult, String> {
    let xml_path = crate::commands::find_minion_xml().ok_or("Minion installation not found.")?;
    let content =
        fs::read_to_string(&xml_path).map_err(|e| format!("Failed to read Minion data: {}", e))?;
    let minion_addons = crate::commands::parse_minion_addons(&content);

    let store = metadata::load_metadata(addons_dir);

    let mut will_track: Vec<DryRunAddon> = Vec::new();
    let mut already_tracked: Vec<DryRunAddon> = Vec::new();
    let mut missing_on_disk: Vec<DryRunAddon> = Vec::new();

    // Track which disk folders are referenced by Minion
    let mut minion_folders: std::collections::HashSet<String> = std::collections::HashSet::new();

    for addon in &minion_addons {
        for folder in &addon.folders {
            minion_folders.insert(folder.clone());

            let entry = DryRunAddon {
                folder_name: folder.clone(),
                esoui_id: addon.uid,
                minion_version: addon.version.clone(),
                status: String::new(),
            };

            if store.addons.contains_key(folder) {
                already_tracked.push(DryRunAddon {
                    status: "already_tracked".to_string(),
                    ..entry
                });
            } else if addons_dir.join(folder).is_dir() {
                will_track.push(DryRunAddon {
                    status: "will_track".to_string(),
                    ..entry
                });
            } else {
                missing_on_disk.push(DryRunAddon {
                    status: "missing_on_disk".to_string(),
                    ..entry
                });
            }
        }
    }

    // Find addons on disk that Minion doesn't know about
    let mut unmanaged_on_disk: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(addons_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            // Skip Kalpa internal folders
            if name.starts_with("kalpa") {
                continue;
            }
            if !minion_folders.contains(&name) && !store.addons.contains_key(&name) {
                // Has a manifest? Then it's a real addon
                let manifest_path = addons_dir.join(&name).join(format!("{}.txt", name));
                let addon_manifest = addons_dir.join(&name).join(format!("{}.addon", name));
                if manifest_path.exists() || addon_manifest.exists() {
                    unmanaged_on_disk.push(name);
                }
            }
        }
    }
    unmanaged_on_disk.sort();

    Ok(DryRunResult {
        will_track,
        already_tracked,
        missing_on_disk,
        unmanaged_on_disk,
    })
}

/// Phase 3: Execute the metadata-only migration.
/// Only writes kalpa.json — does NOT move/delete any addon folders or SavedVariables.
pub fn execute_migration(addons_dir: &Path) -> Result<MigrationResult, String> {
    let start = now_timestamp();

    let xml_path = crate::commands::find_minion_xml().ok_or("Minion installation not found.")?;
    let content =
        fs::read_to_string(&xml_path).map_err(|e| format!("Failed to read Minion data: {}", e))?;
    let minion_addons = crate::commands::parse_minion_addons(&content);

    let mut store = metadata::load_metadata(addons_dir);
    let mut imported: u32 = 0;
    let mut already_tracked: u32 = 0;
    let mut skipped_missing: u32 = 0;

    for addon in &minion_addons {
        for folder in &addon.folders {
            if store.addons.contains_key(folder) {
                already_tracked += 1;
                continue;
            }
            if addons_dir.join(folder).is_dir() {
                metadata::record_install(
                    &mut store,
                    folder,
                    addon.uid,
                    &addon.version,
                    &format!(
                        "https://www.esoui.com/downloads/landing.php?fileid={}",
                        addon.uid
                    ),
                );
                imported += 1;
            } else {
                skipped_missing += 1;
            }
        }
    }

    // Atomic write of kalpa.json only
    metadata::save_metadata(addons_dir, &store)?;

    let result = MigrationResult {
        imported,
        already_tracked,
        skipped_missing,
        addon_count: minion_addons.len() as u32,
    };

    // Log the operation
    let _ = append_op_log(
        addons_dir,
        &OpLogEntry {
            operation: "minion_migration".to_string(),
            started_at: start,
            finished_at: now_timestamp(),
            status: "success".to_string(),
            snapshot_id: None,
            files_created: vec![],
            files_modified: vec!["kalpa.json".to_string()],
            details: format!(
                "Imported {} addons, {} already tracked, {} missing on disk",
                result.imported, result.already_tracked, result.skipped_missing
            ),
        },
    );

    Ok(result)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationResult {
    pub imported: u32,
    pub already_tracked: u32,
    pub skipped_missing: u32,
    pub addon_count: u32,
}

/// Create a pre-operation snapshot (for bulk operations like update-all, pack install).
pub fn create_pre_operation_snapshot(
    addons_dir: &Path,
    operation_label: &str,
) -> Result<SnapshotManifest, String> {
    let start = now_timestamp();
    let label = format!("Pre-{}", operation_label);

    // Only snapshot SavedVariables and settings for routine operations (much faster)
    let manifest = create_zip_snapshot(&label, addons_dir, false, true, true)?;

    let _ = append_op_log(
        addons_dir,
        &OpLogEntry {
            operation: format!("pre_{}_snapshot", operation_label),
            started_at: start,
            finished_at: now_timestamp(),
            status: "success".to_string(),
            snapshot_id: Some(manifest.id.clone()),
            files_created: vec![format!("{}.zip", manifest.id)],
            files_modified: vec![],
            details: format!(
                "Pre-operation snapshot: {} files, {} bytes, SHA-256: {}",
                manifest.file_count, manifest.total_size, manifest.archive_sha256
            ),
        },
    );

    Ok(manifest)
}

/// Run integrity checks on the ESO folders.
pub fn check_integrity(addons_dir: &Path) -> IntegrityResult {
    let parent = addons_dir.parent().unwrap_or(addons_dir);
    let mut issues: Vec<String> = Vec::new();

    let addons_folder_ok = addons_dir.is_dir();
    if !addons_folder_ok {
        issues.push("AddOns folder does not exist or is not accessible.".to_string());
    }

    let sv_dir = parent.join("SavedVariables");
    let saved_variables_ok = sv_dir.is_dir();
    if !saved_variables_ok {
        issues.push("SavedVariables folder not found.".to_string());
    }

    let mut addon_count: u32 = 0;

    // Check each tracked addon in kalpa.json
    let store = metadata::load_metadata(addons_dir);
    for folder_name in store.addons.keys() {
        let folder = addons_dir.join(folder_name);
        if !folder.is_dir() {
            issues.push(format!(
                "Tracked addon '{}' folder is missing.",
                folder_name
            ));
            continue;
        }
        // Check manifest exists
        let txt = folder.join(format!("{}.txt", folder_name));
        let addon_ext = folder.join(format!("{}.addon", folder_name));
        if !txt.exists() && !addon_ext.exists() {
            issues.push(format!(
                "Tracked addon '{}' has no manifest file.",
                folder_name
            ));
        }
        addon_count += 1;
    }

    // Check SavedVariables files aren't truncated (> 0 bytes for .lua files)
    if sv_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&sv_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if ext == "lua" {
                            if let Ok(meta) = fs::metadata(&path) {
                                if meta.len() == 0 {
                                    let name = path
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("unknown");
                                    issues.push(format!(
                                        "SavedVariables file '{}' is empty (possibly truncated).",
                                        name
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    IntegrityResult {
        addons_folder_ok,
        saved_variables_ok,
        addon_count,
        issues,
    }
}

/// List all snapshots.
pub fn list_snapshots(addons_dir: &Path) -> Vec<SnapshotManifest> {
    let store = load_snapshot_store(addons_dir);
    store.snapshots
}

/// Restore a snapshot by ID — extracts the ZIP back to the ESO live directory.
/// Automatically creates a pre-restore snapshot of SavedVariables and settings
/// so the user can undo the restore if it doesn't produce the desired result.
pub fn restore_snapshot(addons_dir: &Path, snapshot_id: &str) -> Result<u32, String> {
    let start = now_timestamp();

    // Create an automatic pre-restore snapshot (SavedVariables + settings only, fast)
    // so the user has a rollback point if the restore goes wrong partway through.
    let _ = create_zip_snapshot("Pre-restore", addons_dir, false, true, true);

    let store = load_snapshot_store(addons_dir);
    let manifest = store
        .snapshots
        .iter()
        .find(|s| s.id == snapshot_id)
        .ok_or("Snapshot not found.")?;

    let root = snapshots_root(addons_dir);
    let archive_path = root.join(format!("{}.zip", snapshot_id));
    if !archive_path.is_file() {
        return Err("Snapshot archive file not found.".to_string());
    }

    // Verify SHA-256
    let actual_sha = sha256_file(&archive_path)?;
    if actual_sha != manifest.archive_sha256 {
        return Err(format!(
            "Snapshot archive integrity check failed. Expected SHA-256: {}, got: {}",
            manifest.archive_sha256, actual_sha
        ));
    }

    let parent = addons_dir.parent().unwrap_or(addons_dir);
    let file = fs::File::open(&archive_path)
        .map_err(|e| format!("Failed to open snapshot archive: {}", e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Failed to read snapshot archive: {}", e))?;

    let mut restored: u32 = 0;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read archive entry: {}", e))?;

        let entry_path = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => continue, // Skip entries with path traversal
        };

        let dest = parent.join(&entry_path);

        // Defense-in-depth: ensure extracted path stays within the target directory
        if !dest.starts_with(parent) {
            continue;
        }

        if entry.is_dir() {
            let _ = fs::create_dir_all(&dest);
        } else {
            if let Some(parent_dir) = dest.parent() {
                let _ = fs::create_dir_all(parent_dir);
            }
            // Atomic write: write to .tmp then rename
            let mut tmp_name = dest.file_name().unwrap_or_default().to_os_string();
            tmp_name.push(".restore-tmp");
            let tmp_dest = dest.with_file_name(tmp_name);
            let mut out = fs::File::create(&tmp_dest)
                .map_err(|e| format!("Failed to create restore file: {}", e))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("Failed to write restore file: {}", e))?;
            fs::rename(&tmp_dest, &dest)
                .map_err(|e| format!("Failed to finalize restored file: {}", e))?;
            restored += 1;
        }
    }

    let _ = append_op_log(
        addons_dir,
        &OpLogEntry {
            operation: "restore_snapshot".to_string(),
            started_at: start,
            finished_at: now_timestamp(),
            status: "success".to_string(),
            snapshot_id: Some(snapshot_id.to_string()),
            files_created: vec![],
            files_modified: vec![format!("{} files restored", restored)],
            details: format!("Restored {} files from snapshot {}", restored, snapshot_id),
        },
    );

    Ok(restored)
}

/// Delete a snapshot by ID.
pub fn delete_snapshot(addons_dir: &Path, snapshot_id: &str) -> Result<(), String> {
    let root = snapshots_root(addons_dir);
    let archive_path = root.join(format!("{}.zip", snapshot_id));
    if archive_path.is_file() {
        fs::remove_file(&archive_path)
            .map_err(|e| format!("Failed to delete snapshot archive: {}", e))?;
    }

    let mut store = load_snapshot_store(addons_dir);
    store.snapshots.retain(|s| s.id != snapshot_id);
    save_snapshot_store(addons_dir, &store)?;

    Ok(())
}

/// Read the transaction log entries.
pub fn read_ops_log(addons_dir: &Path) -> Vec<OpLogEntry> {
    let path = ops_log_path(addons_dir);
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Copy Minion's config as a read-only backup (never modifies Minion state).
/// Only copies regular files in the top-level .minion/ directory (no recursion).
/// Skips symlinks and enforces a 50 MB total size cap to avoid copying unexpected data.
pub fn backup_minion_config(addons_dir: &Path) -> Result<u32, String> {
    const MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024; // 50 MB cap

    let home = dirs::home_dir().ok_or("Could not determine home directory.")?;
    let minion_dir = home.join(".minion");
    if !minion_dir.is_dir() {
        return Err("Minion config directory not found.".to_string());
    }

    let root = snapshots_root(addons_dir);
    let dest = root.join("minion-config-backup");
    fs::create_dir_all(&dest)
        .map_err(|e| format!("Failed to create Minion backup directory: {}", e))?;

    let mut copied: u32 = 0;
    let mut total_bytes: u64 = 0;
    if let Ok(entries) = fs::read_dir(&minion_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            // Skip symlinks (flat-only, no following links to unexpected locations)
            if path.read_link().is_ok() {
                continue;
            }
            // Only copy regular files at the top level (no directory recursion)
            if path.is_file() {
                if let Some(name) = path.file_name() {
                    let meta = match fs::metadata(&path) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    if total_bytes + meta.len() > MAX_TOTAL_BYTES {
                        break; // Stop copying if we'd exceed the size cap
                    }
                    let target = dest.join(name);
                    if fs::copy(&path, &target).is_ok() {
                        total_bytes += meta.len();
                        copied += 1;
                    }
                }
            }
        }
    }

    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_id_is_unique() {
        let id1 = snapshot_id("test");
        // They should at least be non-empty
        assert!(!id1.is_empty());
        assert!(id1.contains("test"));
    }

    #[test]
    fn sha256_of_known_content() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.txt");
        fs::write(&path, "hello world").unwrap();
        let hash = sha256_file(&path).unwrap();
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn snapshot_store_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        let mut store = SnapshotStore::default();
        store.snapshots.push(SnapshotManifest {
            id: "test-id".to_string(),
            label: "Test".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            source_paths: vec!["AddOns".to_string()],
            file_count: 5,
            total_size: 1024,
            archive_sha256: "abc123".to_string(),
        });

        save_snapshot_store(&addons_dir, &store).unwrap();
        let loaded = load_snapshot_store(&addons_dir);
        assert_eq!(loaded.snapshots.len(), 1);
        assert_eq!(loaded.snapshots[0].id, "test-id");
    }

    #[test]
    fn integrity_check_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        let result = check_integrity(&addons_dir);
        assert!(result.addons_folder_ok);
        assert!(!result.saved_variables_ok);
        assert!(result.issues.iter().any(|i| i.contains("SavedVariables")));
    }

    #[test]
    fn op_log_append_and_read() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        let entry = OpLogEntry {
            operation: "test_op".to_string(),
            started_at: "2024-01-01T00:00:00Z".to_string(),
            finished_at: "2024-01-01T00:00:01Z".to_string(),
            status: "success".to_string(),
            snapshot_id: None,
            files_created: vec![],
            files_modified: vec![],
            details: "Test operation".to_string(),
        };

        append_op_log(&addons_dir, &entry).unwrap();
        let entries = read_ops_log(&addons_dir);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].operation, "test_op");
    }
}
