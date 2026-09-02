//! Backup, manifest, and transactional placement for the client directory.
//!
//! This is the reversibility half of the write layer; [`crate::client_write`]
//! is the permission half. Together they are what make managing files in a
//! game directory defensible: Kalpa records exactly what it placed, keeps a
//! copy of whatever it displaced, and can put the directory back.
//!
//! # Why placement is transactional
//!
//! Installing ReShade is not one file. It is a proxy DLL, a config, and a
//! shader tree — and a partial install is worse than no install, because a
//! proxy DLL with no shaders still loads into the game and can still stop it
//! launching. So placements are applied as a unit: any failure rolls back
//! every file already placed in that batch and restores every displaced file.
//!
//! # Why hashes matter
//!
//! Every placed file is recorded with the SHA-256 of the bytes Kalpa wrote.
//! Uninstall compares before deleting: if the file has changed, the user (or
//! ReShade itself, or another tool) modified it, and Kalpa leaves it alone and
//! says so rather than silently discarding someone's work.

use crate::client_write::{ManagedFile, ManagedKind, ManagedManifest};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// File name of the managed-file manifest inside the app data directory.
const MANIFEST_FILE: &str = "client-managed.json";
/// Directory name holding the timestamped backup folders.
const BACKUP_DIR: &str = "client-backups";
/// How many *unreferenced* backup folders to keep before pruning the oldest.
///
/// Referenced folders (ones the manifest still points at) are never pruned:
/// deleting those would turn an exact uninstall into a lossy one.
const MAX_UNREFERENCED_BACKUPS: usize = 20;

/// Monotonic suffix so two backups taken in the same second cannot collide.
static BACKUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// One file to place into the client directory.
#[derive(Debug, Clone)]
pub struct Placement {
    /// Destination, relative to the client directory. Forward-slashed.
    pub relative_path: String,
    pub kind: ManagedKind,
    /// Source file, typically a download temp file. Moved, not copied, when
    /// on the same volume.
    pub source: PathBuf,
}

// ── App-data resolution ──────────────────────────────────────────────────

fn app_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    app.path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app data dir: {e}"))
}

/// Where the managed-file manifest lives, inside the app data directory.
pub fn manifest_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app_data_dir(app)?;
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create app data dir: {e}"))?;
    Ok(dir.join(MANIFEST_FILE))
}

/// Root of the timestamped backup folders, inside the app data directory.
pub fn backup_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app_data_dir(app)?.join(BACKUP_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create backup directory: {e}"))?;
    Ok(dir)
}

// ── Manifest ─────────────────────────────────────────────────────────────

/// Load the manifest, returning an empty one when absent or unreadable.
///
/// A corrupt manifest must not brick the feature: it degrades to "Kalpa does
/// not believe it placed anything here", which is the safe direction — nothing
/// gets deleted on that basis.
pub fn load_manifest(app: &tauri::AppHandle) -> ManagedManifest {
    match manifest_path(app) {
        Ok(path) => load_manifest_at(&path),
        Err(_) => ManagedManifest::default(),
    }
}

/// Persist the manifest atomically via [`crate::atomic_file::atomic_write`].
pub fn save_manifest(app: &tauri::AppHandle, manifest: &ManagedManifest) -> Result<(), String> {
    let path = manifest_path(app)?;
    save_manifest_at(&path, manifest)
}

/// Inner form of [`load_manifest`], testable without an `AppHandle`.
fn load_manifest_at(path: &Path) -> ManagedManifest {
    let Ok(bytes) = fs::read(path) else {
        return ManagedManifest::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        eprintln!(
            "Warning: client manifest at {} is unreadable ({error}); treating it as empty.",
            path.display()
        );
        ManagedManifest::default()
    })
}

/// Inner form of [`save_manifest`], testable without an `AppHandle`.
fn save_manifest_at(path: &Path, manifest: &ManagedManifest) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|e| format!("Failed to serialize the client manifest: {e}"))?;
    crate::atomic_file::atomic_write(path, &bytes)
        .map_err(|e| format!("Failed to write the client manifest: {e}"))
}

/// Manifest key for one client directory.
///
/// Canonical where possible so two spellings of the same install share a
/// bucket; the configured form is the fallback when the directory has gone
/// away (uninstall bookkeeping still has to be findable).
fn install_key(client_root: &Path) -> String {
    dunce::canonicalize(client_root)
        .unwrap_or_else(|_| client_root.to_path_buf())
        .to_string_lossy()
        .to_string()
}

// ── Timestamps and hashing ───────────────────────────────────────────────

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn rfc3339_now() -> String {
    crate::metadata::format_timestamp(now_secs())
}

/// A fresh backup folder id.
///
/// `format_timestamp` has one-second resolution, so a per-process counter is
/// appended: two placements inside the same second must not share a folder, or
/// the second would silently overwrite the first backup.
fn new_backup_id() -> String {
    let stamp = crate::metadata::format_timestamp(now_secs()).replace(':', "-");
    let seq = BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{stamp}-{seq:06}-{nanos:09}")
}

fn hash_file(path: &Path) -> Result<String, String> {
    crate::file_hashes::hash_file(path)
}

// ── Backups ──────────────────────────────────────────────────────────────

/// Copy an existing file out of the client directory into a fresh timestamped
/// backup folder, returning that folder's id.
///
/// Returns `Ok(None)` when there was nothing to displace.
pub fn backup_existing(
    app: &tauri::AppHandle,
    client_root: &Path,
    relative_path: &str,
) -> Result<Option<String>, String> {
    let root = backup_root(app)?;
    backup_existing_in(&root, client_root, relative_path)
}

/// Inner form of [`backup_existing`], testable without an `AppHandle`.
fn backup_existing_in(
    backup_root: &Path,
    client_root: &Path,
    relative_path: &str,
) -> Result<Option<String>, String> {
    let source = crate::client_write::safe_relative_join(client_root, relative_path)?;
    if !source.is_file() {
        return Ok(None);
    }
    let id = new_backup_id();
    let destination = backup_file_path(backup_root, &id, relative_path)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create backup subdirectory: {e}"))?;
    }
    fs::copy(&source, &destination)
        .map_err(|e| format!("Failed to back up {relative_path}: {e}"))?;
    Ok(Some(id))
}

/// Where one displaced file lives inside its backup folder.
///
/// The relative shape is preserved so a backup folder reads like a thin slice
/// of the game directory, and so restore is a pure path substitution.
fn backup_file_path(backup_root: &Path, id: &str, relative_path: &str) -> Result<PathBuf, String> {
    let folder = backup_root.join(id);
    crate::client_write::safe_relative_join(&folder, relative_path)
}

/// Delete backup folders no manifest entry points at, oldest first, once there
/// are more than [`MAX_UNREFERENCED_BACKUPS`] of them.
///
/// Ids sort lexicographically in time order by construction, so a plain sort is
/// an age sort. Referenced folders are never candidates.
fn prune_unreferenced_backups(backup_root: &Path, manifest: &ManagedManifest) {
    let referenced: std::collections::BTreeSet<&str> = manifest
        .installs
        .values()
        .flatten()
        .filter_map(|file| file.displaced_backup.as_deref())
        .collect();

    let mut candidates: Vec<String> = fs::read_dir(backup_root)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| !referenced.contains(name.as_str()))
        .collect();

    if candidates.len() <= MAX_UNREFERENCED_BACKUPS {
        return;
    }
    candidates.sort();
    let excess = candidates.len() - MAX_UNREFERENCED_BACKUPS;
    for name in candidates.into_iter().take(excess) {
        let _ = fs::remove_dir_all(backup_root.join(name));
    }
}

// ── Placement ────────────────────────────────────────────────────────────

/// Bookkeeping for one file this batch already wrote, so it can be undone.
struct PlacedRecord {
    relative_path: String,
    resolved: PathBuf,
    displaced_backup: Option<String>,
}

/// Move `source` onto `destination`, falling back to copy across volumes.
///
/// On Windows a download temp file on `C:` and a game install on another drive
/// is the ordinary case, not an edge case: `fs::rename` returns
/// `ERROR_NOT_SAME_DEVICE` there and the copy path is the one that actually
/// runs.
fn move_into_place(source: &Path, destination: &Path) -> Result<(), String> {
    move_into_place_with(source, destination, |from, to| fs::rename(from, to))
}

/// Inner form of [`move_into_place`] taking the rename operation, so the
/// cross-volume fallback is reachable from tests.
///
/// This matters more than it looks. `fs::rename` cannot move a file between
/// volumes, and a game installed on one drive with temp files landing on
/// another is an ordinary Windows configuration rather than an edge case — for
/// those users the fallback is the *only* path that ever runs. Inside a single
/// tempdir a real rename always succeeds, so without injection the branch
/// selection could never be exercised.
fn move_into_place_with(
    source: &Path,
    destination: &Path,
    rename: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), String> {
    match rename(source, destination) {
        Ok(()) => Ok(()),
        Err(_) => copy_then_remove(source, destination),
    }
}

/// Cross-volume half of [`move_into_place`].
///
/// The source removal is best-effort: the bytes are already published, and
/// failing the placement because a temp file lingered would be worse than the
/// leak.
fn copy_then_remove(source: &Path, destination: &Path) -> Result<(), String> {
    fs::copy(source, destination).map_err(|e| {
        format!(
            "Failed to place {}: {e}",
            destination
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        )
    })?;
    let _ = fs::remove_file(source);
    Ok(())
}

/// Create the parents of `target`, recording every directory this call brought
/// into existence so a rollback can take them back out again.
///
/// Only directories under `client_root` are recorded; Kalpa never removes a
/// directory it did not create.
fn create_parents_tracked(
    client_root: &Path,
    target: &Path,
    created: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let Some(parent) = target.parent() else {
        return Ok(());
    };
    let mut missing = Vec::new();
    let mut probe = parent.to_path_buf();
    while !probe.exists() {
        if !probe.starts_with(client_root) || probe == client_root {
            break;
        }
        missing.push(probe.clone());
        match probe.parent() {
            Some(next) if next != probe => probe = next.to_path_buf(),
            _ => break,
        }
    }
    fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {e}"))?;
    created.extend(missing);
    Ok(())
}

/// Remove directories created by this batch, deepest first, stopping at any
/// that is not empty.
fn remove_created_dirs(mut created: Vec<PathBuf>) {
    created.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for dir in created {
        // `remove_dir` refuses a non-empty directory, which is exactly the
        // guard we want: never take out a folder holding someone else's files.
        let _ = fs::remove_dir(dir);
    }
}

/// Undo every file this batch placed and put back everything it displaced.
fn roll_back(backup_root: &Path, placed: &[PlacedRecord], created_dirs: Vec<PathBuf>) {
    for record in placed.iter().rev() {
        let _ = fs::remove_file(&record.resolved);
        if let Some(id) = &record.displaced_backup {
            if let Ok(backup) = backup_file_path(backup_root, id, &record.relative_path) {
                if backup.is_file() {
                    if let Some(parent) = record.resolved.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    if let Err(error) = fs::copy(&backup, &record.resolved) {
                        eprintln!(
                            "Warning: could not restore displaced file {}: {error}",
                            record.relative_path
                        );
                    }
                }
            }
        }
    }
    remove_created_dirs(created_dirs);
}

/// Apply a batch of placements as a unit, backing up anything displaced.
///
/// On any failure every file placed by this call is removed and every
/// displaced file restored, then the original error is returned. On success
/// the manifest is updated and the new entries returned.
pub fn apply_placements(
    app: &tauri::AppHandle,
    client_root: &Path,
    placements: Vec<Placement>,
) -> Result<Vec<ManagedFile>, String> {
    let manifest = manifest_path(app)?;
    let backups = backup_root(app)?;
    apply_placements_in(&manifest, &backups, client_root, placements)
}

/// Inner form of [`apply_placements`], testable without an `AppHandle`.
fn apply_placements_in(
    manifest_path: &Path,
    backup_root: &Path,
    client_root: &Path,
    placements: Vec<Placement>,
) -> Result<Vec<ManagedFile>, String> {
    let mut placed: Vec<PlacedRecord> = Vec::new();
    let mut created_dirs: Vec<PathBuf> = Vec::new();
    let mut entries: Vec<ManagedFile> = Vec::new();

    for placement in &placements {
        match place_one(
            backup_root,
            client_root,
            placement,
            &mut placed,
            &mut created_dirs,
        ) {
            Ok(entry) => entries.push(entry),
            Err(error) => {
                roll_back(backup_root, &placed, created_dirs);
                return Err(error);
            }
        }
    }

    let mut manifest = load_manifest_at(manifest_path);
    let key = install_key(client_root);
    let bucket = manifest.installs.entry(key).or_default();
    for entry in &entries {
        bucket.retain(|existing| existing.relative_path != entry.relative_path);
        bucket.push(entry.clone());
    }
    bucket.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    if let Err(error) = save_manifest_at(manifest_path, &manifest) {
        // A placement Kalpa cannot record is a placement Kalpa cannot undo,
        // which is precisely the state this module exists to prevent.
        roll_back(backup_root, &placed, created_dirs);
        return Err(error);
    }

    prune_unreferenced_backups(backup_root, &manifest);
    Ok(entries)
}

/// Place one file, appending its undo record before any fallible step that
/// happens after the bytes land.
fn place_one(
    backup_root: &Path,
    client_root: &Path,
    placement: &Placement,
    placed: &mut Vec<PlacedRecord>,
    created_dirs: &mut Vec<PathBuf>,
) -> Result<ManagedFile, String> {
    let resolved = crate::client_write::safe_relative_join(client_root, &placement.relative_path)?;
    create_parents_tracked(client_root, &resolved, created_dirs)?;
    // After the parents exist, not before: a symlinked subdirectory can
    // redirect a lexically-clean relative path, and only a post-creation
    // canonicalization sees it.
    crate::client_write::assert_contained(client_root, &resolved)?;

    let displaced_backup = backup_existing_in(backup_root, client_root, &placement.relative_path)?;

    if !placement.source.is_file() {
        return Err(format!(
            "Source file is missing for {}: {}",
            placement.relative_path,
            placement.source.display()
        ));
    }

    move_into_place(&placement.source, &resolved)?;
    // Recorded the instant the bytes exist, so every later failure undoes it.
    placed.push(PlacedRecord {
        relative_path: placement.relative_path.clone(),
        resolved: resolved.clone(),
        displaced_backup: displaced_backup.clone(),
    });

    let sha256 = hash_file(&resolved)?;
    Ok(ManagedFile {
        relative_path: placement.relative_path.clone(),
        kind: placement.kind,
        sha256,
        placed_at: rfc3339_now(),
        displaced_backup,
    })
}

// ── Revert ───────────────────────────────────────────────────────────────

/// Remove previously-placed files and restore what they displaced.
///
/// Files whose current hash differs from the manifest are left in place and
/// reported in the returned list of skipped paths, never deleted.
pub fn revert_placements(
    app: &tauri::AppHandle,
    client_root: &Path,
    relative_paths: &[String],
) -> Result<Vec<String>, String> {
    let manifest = manifest_path(app)?;
    let backups = backup_root(app)?;
    revert_placements_in(&manifest, &backups, client_root, relative_paths)
}

/// Inner form of [`revert_placements`], testable without an `AppHandle`.
fn revert_placements_in(
    manifest_path: &Path,
    backup_root: &Path,
    client_root: &Path,
    relative_paths: &[String],
) -> Result<Vec<String>, String> {
    let mut manifest = load_manifest_at(manifest_path);
    let key = install_key(client_root);
    let Some(bucket) = manifest.installs.get(&key).cloned() else {
        // Nothing recorded for this install: there is nothing Kalpa may delete.
        return Ok(relative_paths.to_vec());
    };

    let mut skipped: Vec<String> = Vec::new();
    let mut reverted: Vec<String> = Vec::new();
    let mut emptied_dirs: Vec<PathBuf> = Vec::new();

    for relative in relative_paths {
        let Some(entry) = bucket.iter().find(|file| &file.relative_path == relative) else {
            skipped.push(relative.clone());
            continue;
        };
        let resolved = match crate::client_write::safe_relative_join(client_root, relative) {
            Ok(path) => path,
            Err(_) => {
                skipped.push(relative.clone());
                continue;
            }
        };

        if resolved.is_file() {
            let current = hash_file(&resolved)?;
            if current != entry.sha256 {
                // Someone else's bytes. Leave them, and say so.
                skipped.push(relative.clone());
                continue;
            }
            fs::remove_file(&resolved).map_err(|e| format!("Failed to remove {relative}: {e}"))?;
        }

        if let Some(id) = &entry.displaced_backup {
            let backup = backup_file_path(backup_root, id, relative)?;
            if backup.is_file() {
                if let Some(parent) = resolved.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("Failed to create directory: {e}"))?;
                }
                fs::copy(&backup, &resolved)
                    .map_err(|e| format!("Failed to restore {relative}: {e}"))?;
            }
        } else if let Some(parent) = resolved.parent() {
            // Only a path that left nothing behind can free its directories.
            collect_ancestors(client_root, parent, &mut emptied_dirs);
        }

        reverted.push(relative.clone());
    }

    if !reverted.is_empty() {
        if let Some(bucket) = manifest.installs.get_mut(&key) {
            bucket.retain(|file| !reverted.contains(&file.relative_path));
            if bucket.is_empty() {
                manifest.installs.remove(&key);
            }
        }
        save_manifest_at(manifest_path, &manifest)?;
        remove_created_dirs(emptied_dirs);
    }

    Ok(skipped)
}

/// Every directory from `start` up to (but excluding) `client_root`.
fn collect_ancestors(client_root: &Path, start: &Path, out: &mut Vec<PathBuf>) {
    let mut probe = start.to_path_buf();
    while probe != client_root && probe.starts_with(client_root) {
        if !out.contains(&probe) {
            out.push(probe.clone());
        }
        match probe.parent() {
            Some(next) if next != probe => probe = next.to_path_buf(),
            _ => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Simulates the error Windows returns for a cross-volume `fs::rename`.
    fn not_same_device(_: &Path, _: &Path) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "the system cannot move the file to a different disk drive",
        ))
    }

    #[test]
    fn move_into_place_uses_rename_when_it_succeeds() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let source = tmp.path().join("source.bin");
        let destination = tmp.path().join("destination.bin");
        std::fs::write(&source, b"payload").expect("write source");

        move_into_place(&source, &destination).expect("should place");

        assert_eq!(std::fs::read(&destination).expect("read"), b"payload");
        assert!(!source.exists(), "rename should consume the source");
    }

    #[test]
    fn move_into_place_falls_back_across_volumes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let source = tmp.path().join("source.bin");
        let destination = tmp.path().join("destination.bin");
        std::fs::write(&source, b"payload").expect("write source");

        move_into_place_with(&source, &destination, not_same_device).expect("should fall back");

        assert_eq!(
            std::fs::read(&destination).expect("read"),
            b"payload",
            "the fallback must publish identical bytes"
        );
        assert!(
            !source.exists(),
            "the fallback should still consume the source"
        );
    }

    #[test]
    fn move_into_place_reports_an_error_when_the_fallback_also_fails() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let source = tmp.path().join("source.bin");
        // Parent does not exist, so the copy cannot succeed either.
        let destination = tmp.path().join("missing").join("destination.bin");
        std::fs::write(&source, b"payload").expect("write source");

        let err = move_into_place_with(&source, &destination, not_same_device)
            .expect_err("should surface the copy failure");
        assert!(
            err.contains("destination.bin"),
            "error should name the file: {err}"
        );
        assert!(
            source.exists(),
            "a failed placement must not consume the source"
        );
    }

    struct Harness {
        _temp: tempfile::TempDir,
        manifest: PathBuf,
        backups: PathBuf,
        client: PathBuf,
        sources: PathBuf,
    }

    impl Harness {
        fn new() -> Self {
            let temp = tempfile::tempdir().expect("tempdir");
            let manifest = temp.path().join("appdata").join("client-managed.json");
            let backups = temp.path().join("appdata").join("client-backups");
            let client = temp.path().join("client");
            let sources = temp.path().join("sources");
            fs::create_dir_all(manifest.parent().unwrap()).expect("mkdir appdata");
            fs::create_dir_all(&backups).expect("mkdir backups");
            fs::create_dir_all(&client).expect("mkdir client");
            fs::create_dir_all(&sources).expect("mkdir sources");
            Self {
                _temp: temp,
                manifest,
                backups,
                client,
                sources,
            }
        }

        /// A fresh source file with known bytes, named so two placements in one
        /// batch never share a source path.
        fn source(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.sources.join(name);
            fs::write(&path, contents).expect("write source");
            path
        }

        fn placement(&self, relative: &str, contents: &str) -> Placement {
            // Flatten the destination into a source fixture name. The colon MUST
            // be replaced along with the separators: a name beginning "C:" is
            // drive-relative on Windows, so joining it onto the temp dir silently
            // discards that dir and resolves against the current directory on C:
            // — which is how this helper used to write a fixture into the
            // repository itself.
            let name = relative.replace(['/', '\\', ':'], "_");
            Placement {
                relative_path: relative.to_string(),
                kind: ManagedKind::Shader,
                source: self.source(&name, contents),
            }
        }

        fn apply(&self, placements: Vec<Placement>) -> Result<Vec<ManagedFile>, String> {
            apply_placements_in(&self.manifest, &self.backups, &self.client, placements)
        }

        fn revert(&self, paths: &[String]) -> Result<Vec<String>, String> {
            revert_placements_in(&self.manifest, &self.backups, &self.client, paths)
        }

        fn read(&self, relative: &str) -> String {
            fs::read_to_string(self.client.join(relative)).expect("read placed file")
        }

        fn manifest(&self) -> ManagedManifest {
            load_manifest_at(&self.manifest)
        }

        fn entries(&self) -> Vec<ManagedFile> {
            let key = install_key(&self.client);
            self.manifest()
                .installs
                .get(&key)
                .cloned()
                .unwrap_or_default()
        }
    }

    fn sha256_of(text: &str) -> String {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("blob");
        fs::write(&path, text).expect("write");
        hash_file(&path).expect("hash")
    }

    #[test]
    fn places_every_file_and_records_every_hash() {
        let h = Harness::new();
        let placed = h
            .apply(vec![
                h.placement("dxgi.dll", "proxy-dll-bytes"),
                h.placement("ReShade.ini", "[GENERAL]"),
                h.placement("reshade-shaders/Shaders/Bloom.fx", "// bloom"),
            ])
            .expect("placement should succeed");

        assert_eq!(placed.len(), 3);
        assert_eq!(h.read("dxgi.dll"), "proxy-dll-bytes");
        assert_eq!(h.read("ReShade.ini"), "[GENERAL]");
        assert_eq!(h.read("reshade-shaders/Shaders/Bloom.fx"), "// bloom");

        assert_eq!(
            placed[0].sha256,
            sha256_of("proxy-dll-bytes"),
            "hash must be of the bytes actually written"
        );
        assert!(placed.iter().all(|entry| entry.sha256.len() == 64));
        assert!(placed.iter().all(|entry| entry.displaced_backup.is_none()));

        let recorded = h.entries();
        assert_eq!(recorded.len(), 3);
        assert!(recorded
            .iter()
            .any(|entry| entry.relative_path == "reshade-shaders/Shaders/Bloom.fx"));
    }

    #[test]
    fn backs_up_an_existing_file_before_overwriting_it() {
        let h = Harness::new();
        fs::write(h.client.join("dxgi.dll"), "the-users-original-dll").expect("seed");

        let placed = h
            .apply(vec![h.placement("dxgi.dll", "kalpas-new-dll")])
            .expect("placement should succeed");

        assert_eq!(h.read("dxgi.dll"), "kalpas-new-dll");
        let id = placed[0]
            .displaced_backup
            .clone()
            .expect("a displaced file must be recorded");
        let backup = backup_file_path(&h.backups, &id, "dxgi.dll").expect("backup path");
        assert_eq!(
            fs::read_to_string(&backup).expect("read backup"),
            "the-users-original-dll",
            "the backup must be byte-identical to what was displaced"
        );
    }

    #[test]
    fn backup_existing_returns_none_when_nothing_is_displaced() {
        let h = Harness::new();
        let id = backup_existing_in(&h.backups, &h.client, "dxgi.dll").expect("no error");
        assert!(id.is_none());
    }

    #[test]
    fn backup_ids_are_unique_within_one_second() {
        let h = Harness::new();
        fs::write(h.client.join("dxgi.dll"), "original").expect("seed");
        let first = backup_existing_in(&h.backups, &h.client, "dxgi.dll")
            .expect("backup")
            .expect("some");
        let second = backup_existing_in(&h.backups, &h.client, "dxgi.dll")
            .expect("backup")
            .expect("some");
        assert_ne!(first, second);
    }

    #[test]
    fn a_mid_batch_failure_rolls_back_completely() {
        let h = Harness::new();
        fs::write(h.client.join("dxgi.dll"), "the-users-original-dll").expect("seed");
        let before = fs::read(h.client.join("dxgi.dll")).expect("read seed");

        let error = h
            .apply(vec![
                h.placement("dxgi.dll", "kalpas-new-dll"),
                h.placement("reshade-shaders/Shaders/Bloom.fx", "// bloom"),
                // Rejected by safe_relative_join, mid-batch.
                h.placement("../evil.dll", "pwned"),
            ])
            .expect_err("the batch must fail");
        assert!(error.contains("'..'"), "unexpected error: {error}");

        assert_eq!(
            fs::read(h.client.join("dxgi.dll")).expect("read after rollback"),
            before,
            "the displaced original must be byte-identical to before the batch"
        );
        assert!(
            !h.client.join("reshade-shaders/Shaders/Bloom.fx").exists(),
            "no file placed by the batch may survive"
        );
        assert!(
            !h.client.join("reshade-shaders").exists(),
            "directories the batch created must be removed too"
        );
        assert!(
            !h.client.join("..").join("evil.dll").exists(),
            "traversal target must never be written"
        );
        assert!(
            h.entries().is_empty(),
            "a failed batch must record nothing in the manifest"
        );
    }

    #[test]
    fn rollback_leaves_directories_holding_foreign_files_alone() {
        let h = Harness::new();
        h.apply(vec![h.placement("reshade-shaders/A.fx", "a")])
            .expect("first batch");
        fs::write(h.client.join("reshade-shaders").join("theirs.fx"), "theirs")
            .expect("foreign file");

        let _ = h
            .apply(vec![
                h.placement("reshade-shaders/B.fx", "b"),
                h.placement("../evil.dll", "pwned"),
            ])
            .expect_err("the batch must fail");

        assert!(!h.client.join("reshade-shaders/B.fx").exists());
        assert_eq!(h.read("reshade-shaders/A.fx"), "a");
        assert_eq!(h.read("reshade-shaders/theirs.fx"), "theirs");
    }

    #[test]
    fn revert_removes_clean_files_and_restores_displaced_ones() {
        let h = Harness::new();
        fs::write(h.client.join("dxgi.dll"), "the-users-original-dll").expect("seed");
        h.apply(vec![
            h.placement("dxgi.dll", "kalpas-new-dll"),
            h.placement("reshade-shaders/Shaders/Bloom.fx", "// bloom"),
        ])
        .expect("placement should succeed");

        let skipped = h
            .revert(&[
                "dxgi.dll".to_string(),
                "reshade-shaders/Shaders/Bloom.fx".to_string(),
            ])
            .expect("revert should succeed");

        assert!(skipped.is_empty(), "nothing had drifted: {skipped:?}");
        assert_eq!(
            h.read("dxgi.dll"),
            "the-users-original-dll",
            "the displaced original must come back"
        );
        assert!(!h.client.join("reshade-shaders/Shaders/Bloom.fx").exists());
        assert!(
            !h.client.join("reshade-shaders").exists(),
            "directories emptied by the revert should be cleaned up"
        );
        assert!(h.entries().is_empty());
    }

    #[test]
    fn revert_skips_a_file_whose_content_changed() {
        let h = Harness::new();
        h.apply(vec![h.placement("ReShade.ini", "[GENERAL]")])
            .expect("placement should succeed");
        fs::write(h.client.join("ReShade.ini"), "[GENERAL]\nEdited=1").expect("user edit");

        let skipped = h
            .revert(&["ReShade.ini".to_string()])
            .expect("revert should succeed");

        assert_eq!(skipped, vec!["ReShade.ini".to_string()]);
        assert_eq!(
            h.read("ReShade.ini"),
            "[GENERAL]\nEdited=1",
            "the user's edit must survive untouched"
        );
        assert_eq!(
            h.entries().len(),
            1,
            "a skipped file stays in the manifest, still managed"
        );
    }

    #[test]
    fn revert_reports_paths_it_never_placed() {
        let h = Harness::new();
        h.apply(vec![h.placement("ReShade.ini", "[GENERAL]")])
            .expect("placement should succeed");
        fs::write(h.client.join("eso64.exe"), "the-game").expect("seed");

        let skipped = h
            .revert(&["eso64.exe".to_string()])
            .expect("revert should succeed");

        assert_eq!(skipped, vec!["eso64.exe".to_string()]);
        assert_eq!(
            fs::read_to_string(h.client.join("eso64.exe")).expect("read"),
            "the-game"
        );
    }

    #[test]
    fn revert_on_an_unknown_install_deletes_nothing() {
        let h = Harness::new();
        fs::write(h.client.join("dxgi.dll"), "not-ours").expect("seed");
        let skipped = h.revert(&["dxgi.dll".to_string()]).expect("revert");
        assert_eq!(skipped, vec!["dxgi.dll".to_string()]);
        assert!(h.client.join("dxgi.dll").exists());
    }

    #[test]
    fn traversal_attempts_are_rejected() {
        let h = Harness::new();
        for evil in [
            "../evil.dll",
            "reshade-shaders/../../evil.dll",
            "./../evil.dll",
        ] {
            let error = h
                .apply(vec![h.placement(evil, "pwned")])
                .expect_err("traversal must be refused");
            assert!(
                error.contains("'..'"),
                "unexpected error for {evil}: {error}"
            );
        }
        let absolute = if cfg!(windows) {
            "C:\\Windows\\System32\\evil.dll"
        } else {
            "/etc/evil.dll"
        };
        assert!(h.apply(vec![h.placement(absolute, "pwned")]).is_err());
    }

    #[test]
    fn a_corrupt_manifest_loads_as_empty() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("client-managed.json");
        fs::write(&path, "{ this is not json at all ]").expect("write");
        assert!(load_manifest_at(&path).installs.is_empty());

        let missing = temp.path().join("absent.json");
        assert!(load_manifest_at(&missing).installs.is_empty());
    }

    #[test]
    fn manifest_round_trips_through_an_atomic_save() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("client-managed.json");
        let mut installs = BTreeMap::new();
        installs.insert(
            "C:\\Games\\ESO\\game\\client".to_string(),
            vec![ManagedFile {
                relative_path: "dxgi.dll".to_string(),
                kind: ManagedKind::ReShadeCore,
                sha256: "a".repeat(64),
                placed_at: "2026-01-01T00:00:00Z".to_string(),
                displaced_backup: Some("2026-01-01T00-00-00Z-000000-000000000".to_string()),
            }],
        );
        let manifest = ManagedManifest { installs };
        save_manifest_at(&path, &manifest).expect("save");

        let loaded = load_manifest_at(&path);
        assert_eq!(
            loaded.installs.get("C:\\Games\\ESO\\game\\client"),
            manifest.installs.get("C:\\Games\\ESO\\game\\client")
        );
    }

    #[test]
    fn a_second_placement_of_the_same_path_replaces_its_manifest_entry() {
        let h = Harness::new();
        h.apply(vec![h.placement("ReShade.ini", "v1")])
            .expect("first");
        h.apply(vec![h.placement("ReShade.ini", "v2")])
            .expect("second");

        let entries = h.entries();
        assert_eq!(entries.len(), 1, "no duplicate rows: {entries:?}");
        assert_eq!(entries[0].sha256, sha256_of("v2"));
        assert!(
            entries[0].displaced_backup.is_some(),
            "the v1 file it overwrote must have been backed up"
        );
    }

    #[test]
    fn cross_volume_fallback_copies_and_removes_the_source() {
        // fs::rename succeeds inside one tempdir, so exercise the fallback the
        // Windows C:-temp/B:-game case actually takes by calling it directly.
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source.bin");
        let destination = temp.path().join("destination.bin");
        fs::write(&source, "payload").expect("write");

        copy_then_remove(&source, &destination).expect("fallback copy");

        assert_eq!(fs::read_to_string(&destination).expect("read"), "payload");
        assert!(!source.exists(), "the temp source must not be left behind");
    }

    #[test]
    fn a_missing_source_fails_the_batch_and_rolls_back() {
        let h = Harness::new();
        let mut ghost = h.placement("ghost.dll", "x");
        fs::remove_file(&ghost.source).expect("delete source");
        ghost.source = h.sources.join("ghost.dll");

        let error = h
            .apply(vec![h.placement("dxgi.dll", "new"), ghost])
            .expect_err("must fail");
        assert!(error.contains("Source file is missing"), "{error}");
        assert!(!h.client.join("dxgi.dll").exists());
        assert!(h.entries().is_empty());
    }

    #[test]
    fn unreferenced_backups_are_pruned_but_referenced_ones_are_kept() {
        let h = Harness::new();
        fs::write(h.client.join("dxgi.dll"), "original").expect("seed");
        let placed = h
            .apply(vec![h.placement("dxgi.dll", "new")])
            .expect("placement");
        let referenced = placed[0].displaced_backup.clone().expect("backup id");

        for index in 0..MAX_UNREFERENCED_BACKUPS + 5 {
            let dir = h.backups.join(format!("2020-01-01T00-00-00Z-{index:06}-0"));
            fs::create_dir_all(&dir).expect("mkdir");
            fs::write(dir.join("stale.bin"), "stale").expect("write");
        }
        prune_unreferenced_backups(&h.backups, &h.manifest());

        assert!(
            h.backups.join(&referenced).is_dir(),
            "a manifest-referenced backup must never be pruned"
        );
        let remaining = fs::read_dir(&h.backups)
            .expect("read_dir")
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .count();
        assert_eq!(remaining, MAX_UNREFERENCED_BACKUPS + 1);
    }
}
