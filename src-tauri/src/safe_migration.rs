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
    /// How many files could not be opened (locked by the game, an AV scan, or a
    /// cloud-sync client) and are therefore ABSENT from the archive. `file_count`
    /// only counts what was included, so without this a rollback silently misses
    /// files while reporting full success.
    #[serde(default)]
    pub skipped_count: u32,
    /// Up to [`MAX_RECORDED_SKIPS`] of the skipped paths. Bounded because a
    /// tree-wide read failure would otherwise bloat the snapshot store, which is
    /// rewritten in full on every snapshot.
    #[serde(default)]
    pub skipped_files: Vec<String>,
}

/// Cap on the number of skipped paths recorded per snapshot manifest.
const MAX_RECORDED_SKIPS: usize = 100;

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
    pub plan_digest: String,
    pub will_track: Vec<DryRunAddon>,
    pub already_tracked: Vec<DryRunAddon>,
    pub missing_on_disk: Vec<DryRunAddon>,
    pub unmanaged_on_disk: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MigrationTrackAction {
    folder_name: String,
    esoui_id: u32,
    installed_version: String,
    download_url: String,
}

#[derive(Debug, Clone)]
struct MigrationPlan {
    dry_run: DryRunResult,
    actions: Vec<MigrationTrackAction>,
    addon_count: u32,
}

fn migration_download_url(esoui_id: u32) -> String {
    format!("https://www.esoui.com/downloads/landing.php?fileid={esoui_id}")
}

fn update_plan_digest_str(hasher: &mut Sha256, field: &str, value: &str) {
    hasher.update(field.as_bytes());
    hasher.update([0]);
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

fn update_plan_digest_u32(hasher: &mut Sha256, field: &str, value: u32) {
    hasher.update(field.as_bytes());
    hasher.update([0]);
    hasher.update(4u64.to_le_bytes());
    hasher.update(value.to_le_bytes());
}

fn migration_plan_digest(actions: &[MigrationTrackAction]) -> String {
    let mut sorted: Vec<&MigrationTrackAction> = actions.iter().collect();
    sorted.sort_unstable_by(|left, right| {
        left.folder_name
            .cmp(&right.folder_name)
            .then(left.esoui_id.cmp(&right.esoui_id))
            .then(left.installed_version.cmp(&right.installed_version))
            .then(left.download_url.cmp(&right.download_url))
    });

    let mut hasher = Sha256::new();
    update_plan_digest_str(&mut hasher, "schema", "kalpa.minion-migration.plan.v1");
    hasher.update(b"actions");
    hasher.update([0]);
    hasher.update((sorted.len() as u64).to_le_bytes());
    for action in sorted {
        hasher.update(b"action");
        hasher.update([0]);
        update_plan_digest_str(&mut hasher, "folderName", &action.folder_name);
        update_plan_digest_u32(&mut hasher, "esouiId", action.esoui_id);
        update_plan_digest_str(&mut hasher, "installedVersion", &action.installed_version);
        update_plan_digest_str(&mut hasher, "downloadUrl", &action.download_url);
    }
    crate::file_hashes::to_hex(&hasher.finalize())
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
    fs::create_dir_all(&root).map_err(|e| format!("Failed to create KalpaBackups: {e}"))?;
    metadata::save_json_with_backup(&snapshot_store_path(addons_dir), store)
}

/// Guards every load-modify-save sequence against `snapshots.json` so that
/// concurrent snapshot creators/deleters cannot race and silently drop each
/// other's manifest entry. Hold this only around the store mutation itself,
/// never across zip building/hashing or an `.await`.
static SNAPSHOT_STORE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    fs::create_dir_all(&root).map_err(|e| format!("Failed to create KalpaBackups: {e}"))?;
    let path = ops_log_path(addons_dir);

    // Rotate: if the log has grown too large, trim to the most recent entries
    if let Ok(existing) = fs::read_to_string(&path) {
        let lines: Vec<&str> = existing.lines().collect();
        if lines.len() >= OPS_LOG_MAX_ENTRIES {
            // Keep the most recent half to avoid trimming on every append
            let keep_from = lines.len() - OPS_LOG_MAX_ENTRIES / 2;
            let trimmed = lines[keep_from..].join("\n");
            let _ = fs::write(&path, format!("{trimmed}\n"));
        }
    }

    let line =
        serde_json::to_string(entry).map_err(|e| format!("Failed to serialize log entry: {e}"))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("Failed to open ops log: {e}"))?;
    writeln!(file, "{line}").map_err(|e| format!("Failed to write ops log: {e}"))?;
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
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    // Release builds are GUI-subsystem, so a console child spawned without
    // CREATE_NO_WINDOW pops a visible console window — here up to four times in a
    // row when the migration wizard checks its preconditions.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let name = process.name();
    Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {name}"), "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
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
    fs::create_dir_all(&root).map_err(|e| format!("Failed to create KalpaBackups: {e}"))?;

    let id = snapshot_id(label);
    let archive_path = root.join(format!("{id}.zip"));
    let file = crate::atomic_file::AtomicFile::create(&archive_path)
        .map_err(|e| format!("Failed to create snapshot archive: {e}"))?;

    // AtomicFile owns its unique staging path and cleans it if ZIP creation is
    // abandoned, without touching another process's in-flight snapshot.
    let build_zip = || -> Result<_, String> {
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(6));

        let parent = addons_dir.parent().unwrap_or(addons_dir);
        let mut file_count: u32 = 0;
        let mut total_size: u64 = 0;
        let mut source_paths: Vec<String> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();

        // AddOns folder
        if include_addons {
            source_paths.push("AddOns".to_string());
            let result = add_dir_to_zip(&mut zip, addons_dir, "AddOns", &options, &mut skipped)?;
            file_count += result.0;
            total_size += result.1;
        }

        // SavedVariables folder
        if include_saved_vars {
            let sv_dir = parent.join("SavedVariables");
            if sv_dir.is_dir() {
                source_paths.push("SavedVariables".to_string());
                let result =
                    add_dir_to_zip(&mut zip, &sv_dir, "SavedVariables", &options, &mut skipped)?;
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
                    let mut reader = fs::File::open(&settings_file)
                        .map_err(|e| format!("Failed to read {filename}: {e}"))?;
                    zip.start_file(*filename, options)
                        .map_err(|e| format!("Failed to add {filename} to archive: {e}"))?;
                    let written = std::io::copy(&mut reader, &mut zip)
                        .map_err(|e| format!("Failed to write {filename} to archive: {e}"))?;
                    file_count += 1;
                    total_size += written;
                }
            }
        }

        // Take the owned staging file back from the ZIP writer. commit_with below
        // flushes and fsyncs it before hashing and publishing.
        let out = zip
            .finish()
            .map_err(|e| format!("Failed to finalize snapshot archive: {e}"))?;

        Ok((source_paths, file_count, total_size, skipped, out))
    };

    let (source_paths, file_count, total_size, skipped, staged) = build_zip()?;
    let mut sha256 = None;
    staged
        .commit_with(|staging| {
            sha256 = Some(sha256_file(staging).map_err(std::io::Error::other)?);
            Ok(())
        })
        .map_err(|e| format!("Failed to finalize snapshot: {e}"))?;
    let sha256 = sha256.expect("snapshot hash callback runs before publication");

    // Record in snapshot store
    let skipped_count = skipped.len() as u32;
    let mut skipped_files = skipped;
    skipped_files.truncate(MAX_RECORDED_SKIPS);
    let manifest = SnapshotManifest {
        id,
        label: label.to_string(),
        created_at: now_timestamp(),
        source_paths,
        file_count,
        total_size,
        archive_sha256: sha256,
        skipped_count,
        skipped_files,
    };
    {
        let _guard = SNAPSHOT_STORE_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut store = load_snapshot_store(addons_dir);
        store.snapshots.push(manifest.clone());
        save_snapshot_store(addons_dir, &store)?;
    }

    Ok(manifest)
}

/// Recursively add a directory to a ZIP archive.
///
/// Files that cannot be opened are skipped and their archive-relative paths are
/// pushed onto `skipped`; the caller records them so a snapshot whose contents
/// are incomplete never presents itself as a complete rollback point.
fn add_dir_to_zip<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    dir: &Path,
    prefix: &str,
    options: &SimpleFileOptions,
    skipped: &mut Vec<String>,
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
            let zip_path = format!("{current_prefix}/{name}");

            if path.is_dir() {
                // Skip symlinks
                if path.read_link().is_ok() {
                    continue;
                }
                stack.push((path, zip_path));
            } else if path.is_file() {
                // Skip symlinks/reparse points (matches the directory branch above)
                if path.read_link().is_ok() {
                    continue;
                }
                // Stream the file into the archive instead of loading it fully
                // into memory — SavedVariables can be hundreds of MB per file.
                let mut reader = match fs::File::open(&path) {
                    Ok(f) => f,
                    Err(_) => {
                        // Unreadable (locked by another process, permission denied).
                        // Record it — an unrecorded omission makes a partial
                        // snapshot indistinguishable from a complete one.
                        skipped.push(zip_path);
                        continue;
                    }
                };
                zip.start_file(&zip_path, *options)
                    .map_err(|e| format!("Failed to add '{zip_path}' to archive: {e}"))?;
                let written = std::io::copy(&mut reader, zip)
                    .map_err(|e| format!("Failed to write '{zip_path}' to archive: {e}"))?;
                file_count += 1;
                total_size += written;
            }
        }
    }

    Ok((file_count, total_size))
}

/// Op-log suffix naming the files a snapshot could not include. Empty when the
/// snapshot is complete, so the common case reads exactly as before.
fn skipped_detail(manifest: &SnapshotManifest) -> String {
    if manifest.skipped_count == 0 {
        return String::new();
    }
    format!(
        ", {} unreadable file(s) SKIPPED and absent from the archive: {}",
        manifest.skipped_count,
        manifest.skipped_files.join(", ")
    )
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|e| format!("Failed to open file for hashing: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("Failed to read file for hashing: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(crate::file_hashes::to_hex(&hasher.finalize()))
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
                "Snapshot {} files, {} bytes, SHA-256: {}{}",
                manifest.file_count,
                manifest.total_size,
                manifest.archive_sha256,
                skipped_detail(&manifest)
            ),
        },
    );

    Ok(manifest)
}

fn minion_addons_for_migration<'a>(
    minion_addons: &'a [crate::commands::MinionAddon],
    addons_dir: &Path,
) -> Vec<&'a crate::commands::MinionAddon> {
    let has_scoped_entries = minion_addons
        .iter()
        .any(|addon| addon.addons_path.is_some());
    minion_addons
        .iter()
        .filter(|addon| {
            crate::commands::minion_addon_is_for_root(addon, addons_dir, has_scoped_entries)
        })
        .collect()
}

fn load_minion_addons_for_migration() -> Result<Vec<crate::commands::MinionAddon>, String> {
    let xml_path = crate::commands::find_minion_xml().ok_or("Minion installation not found.")?;
    let content =
        fs::read_to_string(&xml_path).map_err(|e| format!("Failed to read Minion data: {e}"))?;
    Ok(crate::commands::parse_minion_addons(&content))
}

fn build_migration_plan_from_addons(
    minion_addons: &[crate::commands::MinionAddon],
    addons_dir: &Path,
    store: &metadata::MetadataStore,
) -> MigrationPlan {
    let minion_addons = minion_addons_for_migration(minion_addons, addons_dir);

    let mut will_track: Vec<DryRunAddon> = Vec::new();
    let mut already_tracked: Vec<DryRunAddon> = Vec::new();
    let mut missing_on_disk: Vec<DryRunAddon> = Vec::new();
    let mut actions: Vec<MigrationTrackAction> = Vec::new();

    // Track which disk folders are referenced by Minion.
    let mut minion_folders: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut effectively_tracked: std::collections::HashSet<String> =
        store.addons.keys().cloned().collect();

    for addon in &minion_addons {
        for folder in &addon.folders {
            minion_folders.insert(folder.clone());

            let entry = DryRunAddon {
                folder_name: folder.clone(),
                esoui_id: addon.uid,
                minion_version: addon.version.clone(),
                status: String::new(),
            };

            if effectively_tracked.contains(folder) {
                already_tracked.push(DryRunAddon {
                    status: "already_tracked".to_string(),
                    ..entry
                });
            } else if addons_dir.join(folder).is_dir() {
                let action = MigrationTrackAction {
                    folder_name: folder.clone(),
                    esoui_id: addon.uid,
                    installed_version: addon.version.clone(),
                    download_url: migration_download_url(addon.uid),
                };
                actions.push(action);
                effectively_tracked.insert(folder.clone());
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

    // Find addons on disk that Minion doesn't know about.
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
            // Skip Kalpa internal folders.
            if name.starts_with("kalpa") {
                continue;
            }
            if !minion_folders.contains(&name) && !store.addons.contains_key(&name) {
                let manifest_path = addons_dir.join(&name).join(format!("{name}.txt"));
                let addon_manifest = addons_dir.join(&name).join(format!("{name}.addon"));
                if manifest_path.exists() || addon_manifest.exists() {
                    unmanaged_on_disk.push(name);
                }
            }
        }
    }
    unmanaged_on_disk.sort();

    let plan_digest = migration_plan_digest(&actions);
    MigrationPlan {
        dry_run: DryRunResult {
            plan_digest,
            will_track,
            already_tracked,
            missing_on_disk,
            unmanaged_on_disk,
        },
        actions,
        addon_count: minion_addons.len() as u32,
    }
}

/// Phase 2: Dry-run migration — compare Minion data with disk state.
pub fn dry_run_migration(addons_dir: &Path) -> Result<DryRunResult, String> {
    let minion_addons = load_minion_addons_for_migration()?;
    let store = metadata::load_metadata(addons_dir);
    Ok(build_migration_plan_from_addons(&minion_addons, addons_dir, &store).dry_run)
}

/// Phase 3: Execute the metadata-only migration.
/// Only writes kalpa.json — does NOT move/delete any addon folders or SavedVariables.
pub fn execute_migration(
    addons_dir: &Path,
    expected_plan_digest: Option<&str>,
) -> Result<MigrationExecuteOutcome, String> {
    let minion_addons = load_minion_addons_for_migration()?;
    execute_migration_from_addons(addons_dir, &minion_addons, expected_plan_digest)
}

fn execute_migration_from_addons(
    addons_dir: &Path,
    minion_addons: &[crate::commands::MinionAddon],
    expected_plan_digest: Option<&str>,
) -> Result<MigrationExecuteOutcome, String> {
    let start = now_timestamp();

    let mut store = metadata::load_metadata(addons_dir);
    let plan = build_migration_plan_from_addons(minion_addons, addons_dir, &store);

    if let Some(expected_digest) = expected_plan_digest {
        if expected_digest != plan.dry_run.plan_digest {
            return Ok(MigrationExecuteOutcome::PlanChanged {
                expected_digest: expected_digest.to_string(),
                actual_digest: plan.dry_run.plan_digest.clone(),
                fresh_plan: plan.dry_run,
            });
        }
    }

    for action in &plan.actions {
        metadata::record_install(
            &mut store,
            &action.folder_name,
            action.esoui_id,
            &action.installed_version,
            &action.download_url,
        );
    }

    // Atomic write of kalpa.json only
    metadata::save_metadata(addons_dir, &store)?;

    let result = MigrationResult {
        imported: plan.actions.len() as u32,
        already_tracked: plan.dry_run.already_tracked.len() as u32,
        skipped_missing: plan.dry_run.missing_on_disk.len() as u32,
        addon_count: plan.addon_count,
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

    Ok(MigrationExecuteOutcome::Applied { result })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationResult {
    pub imported: u32,
    pub already_tracked: u32,
    pub skipped_missing: u32,
    pub addon_count: u32,
}

// `rename_all` camelCases only the variant tags; `rename_all_fields` is what
// camelCases the struct-variant FIELDS (`fresh_plan` → `freshPlan`). Without it
// the wizard reads undefined and the fresh plan never renders — same trap as
// uploader::watcher::LiveEvent.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MigrationExecuteOutcome {
    Applied {
        result: MigrationResult,
    },
    PlanChanged {
        expected_digest: String,
        actual_digest: String,
        fresh_plan: DryRunResult,
    },
}

/// Create a pre-operation snapshot (for bulk operations like update-all, pack install).
pub fn create_pre_operation_snapshot(
    addons_dir: &Path,
    operation_label: &str,
) -> Result<SnapshotManifest, String> {
    let start = now_timestamp();
    let label = format!("Pre-{operation_label}");

    // Only snapshot SavedVariables and settings for routine operations (much faster)
    let manifest = create_zip_snapshot(&label, addons_dir, false, true, true)?;

    let _ = append_op_log(
        addons_dir,
        &OpLogEntry {
            operation: format!("pre_{operation_label}_snapshot"),
            started_at: start,
            finished_at: now_timestamp(),
            status: "success".to_string(),
            snapshot_id: Some(manifest.id.clone()),
            files_created: vec![format!("{}.zip", manifest.id)],
            files_modified: vec![],
            details: format!(
                "Pre-operation snapshot: {} files, {} bytes, SHA-256: {}{}",
                manifest.file_count,
                manifest.total_size,
                manifest.archive_sha256,
                skipped_detail(&manifest)
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
            issues.push(format!("Tracked addon '{folder_name}' folder is missing."));
            continue;
        }
        // Check manifest exists
        let txt = folder.join(format!("{folder_name}.txt"));
        let addon_ext = folder.join(format!("{folder_name}.addon"));
        if !txt.exists() && !addon_ext.exists() {
            issues.push(format!(
                "Tracked addon '{folder_name}' has no manifest file."
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
                                        "SavedVariables file '{name}' is empty (possibly truncated)."
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

/// Extract a snapshot ZIP archive entry-by-entry onto the live directory.
/// Pure extraction with no snapshotting side effects — shared by the normal
/// restore path and by the automatic rollback path, so rollback can never
/// recursively create another Pre-restore snapshot.
fn extract_archive_entries(archive_path: &Path, parent: &Path) -> Result<u32, String> {
    let file = fs::File::open(archive_path)
        .map_err(|e| format!("Failed to open snapshot archive: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Failed to read snapshot archive: {e}"))?;

    let mut restored: u32 = 0;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read archive entry: {e}"))?;

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
            let mut out = crate::atomic_file::AtomicFile::create(&dest)
                .map_err(|e| format!("Failed to create restore file: {e}"))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("Failed to write restore file: {e}"))?;
            out.commit()
                .map_err(|e| format!("Failed to finalize restored file: {e}"))?;
            restored += 1;
        }
    }

    Ok(restored)
}

/// Restore a snapshot by ID — extracts the ZIP back to the ESO live directory.
/// Automatically creates a pre-restore snapshot of SavedVariables and settings
/// so the user can undo the restore if it doesn't produce the desired result.
/// If extraction fails partway through, this automatically attempts to roll
/// back to that pre-restore snapshot so the tree is never left half-restored
/// without at least an attempt to recover, and the error always names the
/// pre-restore snapshot so the user can restore it manually if rollback fails.
pub fn restore_snapshot(addons_dir: &Path, snapshot_id: &str) -> Result<u32, String> {
    let start = now_timestamp();

    // Look up the target snapshot's manifest first so the automatic pre-restore
    // safety snapshot (below) can mirror whether it includes the AddOns folder.
    let store = load_snapshot_store(addons_dir);
    let manifest = store
        .snapshots
        .iter()
        .find(|s| s.id == snapshot_id)
        .ok_or("Snapshot not found.")?;

    // Create an automatic pre-restore snapshot (SavedVariables + settings always,
    // plus AddOns whenever the target snapshot includes AddOns) so the user has a
    // rollback point if the restore goes wrong partway through. Mirroring the
    // target's AddOns inclusion matters: if an AddOns-inclusive restore fails
    // partway, the rollback below re-extracts this Pre-restore snapshot — and a
    // Pre-restore snapshot with no AddOns entries would leave AddOns half-restored
    // while the error still reports a full automatic recovery.
    let include_addons = manifest.source_paths.iter().any(|p| p == "AddOns");
    let pre_restore_manifest =
        create_zip_snapshot("Pre-restore", addons_dir, include_addons, true, true).map_err(
            |e| {
                format!(
                    "Failed to create a safety snapshot before restoring. \
                 Your current data would be unrecoverable if the restore fails. \
                 Please free disk space or create a manual snapshot first. Error: {e}"
                )
            },
        )?;

    let root = snapshots_root(addons_dir);
    let archive_path = root.join(format!("{snapshot_id}.zip"));
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

    let restored = match extract_archive_entries(&archive_path, parent) {
        Ok(restored) => restored,
        Err(e) => {
            // Best-effort rollback: restore the just-created Pre-restore snapshot's
            // own archive directly via the shared extraction helper (not through
            // restore_snapshot itself), so this can never re-enter the rollback
            // logic or create yet another Pre-restore snapshot.
            let pre_restore_archive = root.join(format!("{}.zip", pre_restore_manifest.id));
            let rollback_result = extract_archive_entries(&pre_restore_archive, parent);
            let rollback_ok = rollback_result.is_ok();

            let _ = append_op_log(
                addons_dir,
                &OpLogEntry {
                    operation: "restore_snapshot".to_string(),
                    started_at: start.clone(),
                    finished_at: now_timestamp(),
                    status: "failed".to_string(),
                    snapshot_id: Some(snapshot_id.to_string()),
                    files_created: vec![],
                    files_modified: vec![],
                    details: format!(
                        "Restore from snapshot {snapshot_id} failed: {e}. Automatic rollback to \
                         Pre-restore snapshot {}: {}{}",
                        pre_restore_manifest.id,
                        match &rollback_result {
                            Ok(n) => format!("succeeded ({n} files restored)"),
                            Err(rollback_err) => format!("failed ({rollback_err})"),
                        },
                        skipped_detail(&pre_restore_manifest)
                    ),
                },
            );

            return if rollback_ok {
                // A Pre-restore snapshot that skipped files cannot roll everything
                // back, so never report unqualified success for one.
                if pre_restore_manifest.skipped_count > 0 {
                    return Err(format!(
                        "Restore failed ({e}). Rollback ran from snapshot {} (\"{}\"), but that \
                         safety snapshot could not read {} file(s), so they were NOT rolled back: \
                         {}.",
                        pre_restore_manifest.id,
                        pre_restore_manifest.label,
                        pre_restore_manifest.skipped_count,
                        pre_restore_manifest.skipped_files.join(", ")
                    ));
                }
                Err(format!(
                    "Restore failed ({e}). Your pre-restore state was automatically restored \
                     from snapshot {} (\"{}\").",
                    pre_restore_manifest.id, pre_restore_manifest.label
                ))
            } else {
                let rollback_err = rollback_result.err().unwrap_or_default();
                Err(format!(
                    "Restore failed ({e}); automatic rollback also failed ({rollback_err}). \
                     Restore manually from snapshot {} (\"{}\").",
                    pre_restore_manifest.id, pre_restore_manifest.label
                ))
            };
        }
    };

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
            details: format!(
                "Restored {restored} files from snapshot {snapshot_id}. Pre-restore snapshot {}{}",
                pre_restore_manifest.id,
                skipped_detail(&pre_restore_manifest)
            ),
        },
    );

    Ok(restored)
}

/// Delete a snapshot by ID.
pub fn delete_snapshot(addons_dir: &Path, snapshot_id: &str) -> Result<(), String> {
    let root = snapshots_root(addons_dir);
    let archive_path = root.join(format!("{snapshot_id}.zip"));

    // Update and persist the manifest FIRST, and only delete the archive file
    // after that succeeds. Deleting the archive before the store write could
    // leave a dangling manifest entry (pointing at a now-missing archive) if
    // the store save then failed.
    {
        let _guard = SNAPSHOT_STORE_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut store = load_snapshot_store(addons_dir);
        store.snapshots.retain(|s| s.id != snapshot_id);
        save_snapshot_store(addons_dir, &store)?;
    }

    if archive_path.is_file() {
        fs::remove_file(&archive_path)
            .map_err(|e| format!("Failed to delete snapshot archive: {e}"))?;
    }

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
        .map_err(|e| format!("Failed to create Minion backup directory: {e}"))?;

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

    fn test_minion_addon(
        uid: u32,
        version: &str,
        folders: &[&str],
        addons_path: &Path,
    ) -> crate::commands::MinionAddon {
        crate::commands::MinionAddon {
            uid,
            version: version.to_string(),
            folders: folders.iter().map(|folder| (*folder).to_string()).collect(),
            addons_path: Some(addons_path.to_path_buf()),
        }
    }

    #[test]
    fn minion_migration_uses_only_the_selected_addons_root() {
        let tmp = tempfile::tempdir().unwrap();
        let live_addons_dir = tmp.path().join("live").join("AddOns");
        let pts_addons_dir = tmp.path().join("pts").join("AddOns");
        fs::create_dir_all(&live_addons_dir).unwrap();
        fs::create_dir_all(&pts_addons_dir).unwrap();

        let minion_addons = vec![
            crate::commands::MinionAddon {
                uid: 123,
                version: "1.0".to_string(),
                folders: vec!["MyAddon".to_string()],
                addons_path: Some(pts_addons_dir),
            },
            crate::commands::MinionAddon {
                uid: 123,
                version: "2.0".to_string(),
                folders: vec!["MyAddon".to_string()],
                addons_path: Some(live_addons_dir.clone()),
            },
        ];

        let selected = minion_addons_for_migration(&minion_addons, &live_addons_dir);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].version, "2.0");
    }

    #[test]
    fn migration_execute_outcome_serializes_camel_case_fields() {
        let outcome = MigrationExecuteOutcome::PlanChanged {
            expected_digest: "aa".to_string(),
            actual_digest: "bb".to_string(),
            fresh_plan: DryRunResult {
                plan_digest: "bb".to_string(),
                will_track: vec![],
                already_tracked: vec![],
                missing_on_disk: vec![],
                unmanaged_on_disk: vec![],
            },
        };

        let value = serde_json::to_value(&outcome).unwrap();
        assert_eq!(value["status"], "planChanged");
        // The TS contract (MigrationExecuteOutcome in src/types.ts) reads these
        // exact keys; snake_case here renders the fresh plan as undefined.
        assert!(value.get("expectedDigest").is_some());
        assert!(value.get("actualDigest").is_some());
        assert!(value["freshPlan"].get("planDigest").is_some());
    }

    #[test]
    fn migration_plan_digest_is_deterministic_and_order_independent_over_actions() {
        let actions = vec![
            MigrationTrackAction {
                folder_name: "AddonB".to_string(),
                esoui_id: 20,
                installed_version: "2.0".to_string(),
                download_url: migration_download_url(20),
            },
            MigrationTrackAction {
                folder_name: "AddonA".to_string(),
                esoui_id: 10,
                installed_version: "1.0".to_string(),
                download_url: migration_download_url(10),
            },
        ];
        let reversed = vec![actions[1].clone(), actions[0].clone()];

        let digest = migration_plan_digest(&actions);

        assert_eq!(digest, migration_plan_digest(&actions));
        assert_eq!(digest, migration_plan_digest(&reversed));
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));

        let mut changed = actions.clone();
        changed[0].installed_version = "2.1".to_string();
        assert_ne!(digest, migration_plan_digest(&changed));
    }

    #[test]
    fn execute_migration_with_stale_digest_refuses_and_writes_nothing_to_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(addons_dir.join("AddonA")).unwrap();
        fs::create_dir_all(addons_dir.join("AddonB")).unwrap();

        let reviewed_addons = vec![test_minion_addon(1, "1.0", &["AddonA"], &addons_dir)];
        let fresh_addons = vec![
            test_minion_addon(1, "1.0", &["AddonA"], &addons_dir),
            test_minion_addon(2, "2.0", &["AddonB"], &addons_dir),
        ];
        let reviewed_plan = build_migration_plan_from_addons(
            &reviewed_addons,
            &addons_dir,
            &metadata::MetadataStore::default(),
        );

        let outcome = execute_migration_from_addons(
            &addons_dir,
            &fresh_addons,
            Some(&reviewed_plan.dry_run.plan_digest),
        )
        .unwrap();

        match outcome {
            MigrationExecuteOutcome::PlanChanged {
                expected_digest,
                actual_digest,
                fresh_plan,
            } => {
                assert_eq!(expected_digest, reviewed_plan.dry_run.plan_digest);
                assert_ne!(actual_digest, expected_digest);
                assert_eq!(fresh_plan.will_track.len(), 2);
                assert_eq!(fresh_plan.plan_digest, actual_digest);
            }
            MigrationExecuteOutcome::Applied { .. } => panic!("stale digest should be refused"),
        }
        assert!(!addons_dir.join("kalpa.json").exists());
        assert!(metadata::load_metadata(&addons_dir).addons.is_empty());
        assert!(read_ops_log(&addons_dir).is_empty());
    }

    #[test]
    fn execute_migration_with_matching_digest_applies() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(addons_dir.join("AddonA")).unwrap();

        let minion_addons = vec![test_minion_addon(
            1,
            "1.0",
            &["AddonA", "MissingAddon"],
            &addons_dir,
        )];
        let reviewed_plan = build_migration_plan_from_addons(
            &minion_addons,
            &addons_dir,
            &metadata::MetadataStore::default(),
        );

        let outcome = execute_migration_from_addons(
            &addons_dir,
            &minion_addons,
            Some(&reviewed_plan.dry_run.plan_digest),
        )
        .unwrap();

        let MigrationExecuteOutcome::Applied { result } = outcome else {
            panic!("matching digest should apply");
        };
        assert_eq!(result.imported, 1);
        assert_eq!(result.already_tracked, 0);
        assert_eq!(result.skipped_missing, 1);
        assert_eq!(result.addon_count, 1);

        let store = metadata::load_metadata(&addons_dir);
        let imported = store.addons.get("AddonA").unwrap();
        assert_eq!(imported.esoui_id, 1);
        assert_eq!(imported.installed_version, "1.0");
        assert_eq!(imported.download_url, migration_download_url(1));
        assert!(!store.addons.contains_key("MissingAddon"));
    }

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
            skipped_count: 0,
            skipped_files: Vec::new(),
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
