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
//! launching. So placements are applied as a unit: any failure attempts to roll
//! back every file already placed in that batch and restore every displaced
//! file.
//!
//! # What rollback actually guarantees
//!
//! Rollback is an attempt, not a promise. It is filesystem work, and the same
//! conditions that failed the placement (a locked DLL, a full disk, antivirus,
//! Controlled Folder Access) can fail the undo. So the guarantees are stated in
//! two tiers:
//!
//! * **Rollback succeeds** — the batch is fully undone. No file it placed
//!   survives, every displaced file is byte-identical to before, directories
//!   the batch created are gone, and *nothing* is recorded in the manifest.
//! * **Rollback is incomplete** — the client directory is left in a **mixed
//!   state**, and Kalpa says so. The files it could not restore keep Kalpa's
//!   bytes on disk, and the user's displaced originals stay in the backup
//!   folder. Those files are then recorded in the manifest exactly as if the
//!   placement had succeeded. That is not bookkeeping vanity: an entry is the
//!   only thing that marks a backup folder as *referenced*, and
//!   [`prune_unreferenced_backups`] deletes unreferenced folders. An
//!   unrecorded backup is a backup on a countdown to permanent deletion, and
//!   the file at stake can be the user's own `dxgi.dll` or `nvngx_dlss.dll`.
//!   The returned error names the affected files and says where the originals
//!   are; `revert_placements` is the way back.
//!
//! # Why placement takes a token
//!
//! [`apply_placements`] and [`revert_placements`] name their target with a
//! [`ApprovedRoot`](crate::client_write::ApprovedRoot), not a `&Path`. That is
//! not decoration: this module is the only code in Kalpa that writes into a
//! game install, and a `&Path` parameter would let any caller point it
//! anywhere simply by not asking permission first. The token can only come
//! from `client_write::begin_write`, so every gate in that module is on the
//! path to here by construction.
//!
//! Both functions also call
//! [`reassert_idle`](crate::client_write::ApprovedRoot::reassert_idle) as their
//! first act inside `MANIFEST_LOCK`, before a single byte moves. The token
//! proves the client was idle when it was minted; a download can easily put
//! minutes between that and the write.
//!
//! Restore is done by copying the backup *over* the placed file rather than
//! deleting and then copying. A half-failed overwrite leaves wrong bytes; a
//! failed copy after a successful delete leaves no file at all. Wrong is
//! recoverable and visible, missing is a silent behaviour change.
//!
//! # Why hashes matter
//!
//! Every placed file is recorded with the SHA-256 of the bytes Kalpa wrote.
//! Uninstall compares before deleting: if the file has changed, the user (or
//! ReShade itself, or another tool) modified it, and Kalpa leaves it alone and
//! says so rather than silently discarding someone's work.

use crate::client_write::{ApprovedRoot, ManagedFile, ManagedKind, ManagedManifest};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Serializes every manifest read-modify-write sequence in this module.
///
/// `load_manifest_at` -> mutate -> `save_manifest_at` is not safe to run
/// concurrently: `atomic_write` only guarantees the *write* half is atomic,
/// not the read-then-write sequence around it. Two concurrent callers (a
/// double-clicked button, an install racing a preset switch) can both load
/// the same manifest, both mutate their own copy, and the second save
/// silently discards the first caller's entries. For a placed file that is
/// not cosmetic: an unrecorded entry is a ghost — uninstall can never find it
/// to remove it, and its backup folder is unreferenced, so
/// [`prune_unreferenced_backups`] eventually deletes the user's displaced
/// original out from under them.
///
/// This is module-level rather than Tauri-managed state (the pattern used
/// elsewhere in this codebase, see `MetadataLock` in `lib.rs`) because this
/// module may only be edited in isolation from `lib.rs` and `commands.rs`
/// while this fix lands; a `static Mutex` gives the same process-wide
/// exclusion without threading a new managed value through the app builder
/// or any command signature.
///
/// **Known limit**: this is a single process's lock. It does nothing to
/// protect a client directory against two separate Kalpa processes (two app
/// windows, or a stray second instance) racing the same manifest file. That
/// is a pre-existing gap this change does not attempt to close.
static MANIFEST_LOCK: Mutex<()> = Mutex::new(());

/// Take the process-wide manifest lock, recovering from poisoning instead of
/// propagating it.
///
/// The data this lock protects is a file on disk, re-read fresh from disk on
/// every acquisition — there is no in-memory invariant that a panicking
/// holder could have left half-updated. A poisoned lock here just means some
/// earlier critical section panicked; refusing every future manifest
/// read-modify-write because of that would brick placement/revert entirely,
/// which is a worse outcome than proceeding with a guard over a value nobody
/// actually reads (`()`).
fn lock_manifest() -> std::sync::MutexGuard<'static, ()> {
    MANIFEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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
    kind: ManagedKind,
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
///
/// Returns the indices into `placed` of the records it could **not** undo. An
/// empty result is the clean-rollback guarantee; anything else means the client
/// directory is in a mixed state and the caller must record those entries (see
/// [`record_incomplete_rollback`]) rather than silently dropping them.
///
/// Takes the restore-copy operation so a failing restore is reachable from
/// tests; callers in production pass `fs::copy`.
///
/// Injected for the same reason [`move_into_place_with`] injects `rename`: the
/// interesting branch is the one that only happens when the filesystem refuses,
/// and inside a tempdir a copy always succeeds. Rollback failure is precisely
/// the path where the user's original can be lost, so it has to be testable.
fn roll_back_with(
    backup_root: &Path,
    placed: &[PlacedRecord],
    created_dirs: Vec<PathBuf>,
    restore: impl Fn(&Path, &Path) -> std::io::Result<u64>,
) -> Vec<usize> {
    let mut failed: Vec<usize> = Vec::new();

    for (index, record) in placed.iter().enumerate().rev() {
        let Some(id) = &record.displaced_backup else {
            // Nothing was displaced, so deleting Kalpa's file *is* the restore.
            if let Err(error) = fs::remove_file(&record.resolved) {
                if record.resolved.exists() {
                    eprintln!(
                        "Warning: could not remove placed file {}: {error}",
                        record.relative_path
                    );
                    failed.push(index);
                }
            }
            continue;
        };

        let backup = match backup_file_path(backup_root, id, &record.relative_path) {
            Ok(path) if path.is_file() => path,
            _ => {
                // A displaced original with no readable backup: deleting the
                // placed file here would leave nothing at all where the user's
                // file used to be. Keep the bytes and report it.
                eprintln!(
                    "Warning: the backup of displaced file {} is missing; leaving Kalpa's copy in place.",
                    record.relative_path
                );
                failed.push(index);
                continue;
            }
        };

        if let Some(parent) = record.resolved.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // Overwrite, never delete-then-copy. A copy that dies halfway leaves
        // wrong bytes; a copy that dies after a successful delete leaves no
        // file. For a proxy DLL, missing is a silent behaviour change and wrong
        // at least still loads or visibly fails.
        if let Err(error) = restore(&backup, &record.resolved) {
            eprintln!(
                "Warning: could not restore displaced file {}: {error}",
                record.relative_path
            );
            failed.push(index);
        }
    }

    failed.sort_unstable();
    // `remove_dir` refuses a non-empty directory, so anything still holding a
    // file Kalpa could not undo survives on its own.
    remove_created_dirs(created_dirs);
    failed
}

/// Persist manifest entries for the placements a rollback could not undo, and
/// build the error explaining the mixed state.
///
/// **Persisting is the point.** [`prune_unreferenced_backups`] only ever
/// deletes backup folders that no manifest entry points at. A rollback that
/// leaves Kalpa's file on disk and the user's original in an *unreferenced*
/// backup folder has therefore started a countdown: after
/// [`MAX_UNREFERENCED_BACKUPS`] more backups that original is deleted, and a
/// later install of the same path backs up Kalpa's own DLL as though it were
/// "the user's original". Writing the entry is what stops both. The returned
/// error still carries the original failure as its primary cause; the rollback
/// status is appended, not substituted.
///
/// Assumes the [`MANIFEST_LOCK`] is already held by the caller — it performs
/// its own load-mutate-save sequence on the manifest and must not race a
/// concurrent placement or revert. `std::sync::Mutex` is not reentrant, so
/// this function must never acquire the lock itself; every call site is
/// inside a section that already holds it.
fn record_incomplete_rollback_locked(
    manifest_path: &Path,
    client_root: &Path,
    placed: &[PlacedRecord],
    failed: &[usize],
    cause: String,
) -> String {
    let records: Vec<&PlacedRecord> = failed
        .iter()
        .filter_map(|&index| placed.get(index))
        .collect();

    let entries: Vec<ManagedFile> = records
        .iter()
        .map(|record| ManagedFile {
            relative_path: record.relative_path.clone(),
            kind: record.kind,
            // Hash whatever is actually on disk now, so a later revert can
            // recognise it. If even hashing fails the entry is still written
            // with an empty hash: it can never match, so revert refuses to
            // delete the file — but the backup stays referenced, which is what
            // this entry exists for.
            sha256: hash_file(&record.resolved).unwrap_or_default(),
            placed_at: rfc3339_now(),
            displaced_backup: record.displaced_backup.clone(),
        })
        .collect();

    let names: Vec<String> = records
        .iter()
        .map(|record| record.relative_path.clone())
        .collect();
    let names = names.join(", ");

    let mut manifest = load_manifest_at(manifest_path);
    let bucket = manifest
        .installs
        .entry(install_key(client_root))
        .or_default();
    for entry in &entries {
        bucket.retain(|existing| existing.relative_path != entry.relative_path);
        bucket.push(entry.clone());
    }
    bucket.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    let save = save_manifest_at(manifest_path, &manifest);

    let mut message = format!(
        "{cause}\n\nRollback was incomplete, so the client folder is in a mixed state. \
         Affected file(s): {names}. Kalpa's copy of each is still on disk and the original \
         it displaced is preserved in Kalpa's backup folder, recorded in the manifest so it \
         will not be pruned. Use Revert on the affected file(s) to put the originals back."
    );
    if let Err(error) = save {
        message.push_str(&format!(
            "\n\nWarning: the manifest could not be updated ({error}). The displaced \
             original(s) are still in Kalpa's backup folder, but are not referenced by the \
             manifest and could eventually be pruned — copy them out before continuing."
        ));
    }
    message
}

/// Apply a batch of placements as a unit, backing up anything displaced.
///
/// On success the manifest is updated and the new entries returned.
///
/// On failure the call attempts to remove every file it placed and restore
/// every file it displaced. If that rollback fully succeeds, the original error
/// is returned and nothing is recorded. If it does not, the files it could not
/// undo are recorded in the manifest — keeping their backups referenced and
/// safe from pruning — and the returned error carries the original failure plus
/// a plain statement that the folder is in a mixed state, which files are
/// affected, and that the originals are preserved in the backup folder.
pub fn apply_placements(
    app: &tauri::AppHandle,
    root: &ApprovedRoot,
    placements: Vec<Placement>,
) -> Result<Vec<ManagedFile>, String> {
    let manifest = manifest_path(app)?;
    let backups = backup_root(app)?;
    apply_placements_in(&manifest, &backups, root, placements)
}

/// Inner form of [`apply_placements`], testable without an `AppHandle`.
fn apply_placements_in(
    manifest_path: &Path,
    backup_root: &Path,
    root: &ApprovedRoot,
    placements: Vec<Placement>,
) -> Result<Vec<ManagedFile>, String> {
    apply_placements_in_with(
        manifest_path,
        backup_root,
        root,
        placements,
        |from: &Path, to: &Path| fs::copy(from, to),
    )
}

/// Inner form of [`apply_placements_in`] taking the restore-copy used during
/// rollback, so tests can drive the mixed-state path.
///
/// Takes [`MANIFEST_LOCK`] for the whole batch, not just the final save.
/// This is the load-modify-save sequence the module doc warns about:
/// `atomic_write` only makes the write itself atomic, and without the lock
/// two concurrent batches (a double-clicked button, an install racing a
/// preset switch) can each load the same manifest, mutate their own copy,
/// and have the second save silently discard the first batch's entries — a
/// lost entry that later leaves an orphaned backup for `prune_unreferenced_backups`
/// to delete.
///
/// The lock is held across the placement loop itself (file copies, backups,
/// hashing), not only around the manifest load/save at the end, for two
/// reasons: first, a mid-batch failure calls
/// [`record_incomplete_rollback_locked`], which is itself a manifest
/// load-mutate-save and must not race a concurrent batch; second, the
/// manifest entries this function writes must describe exactly the files
/// this batch placed on disk, so the disk work and the manifest write have
/// to be one critical section to stay consistent with each other. That does
/// mean a second `apply_placements` blocks on filesystem I/O rather than
/// just a manifest write — accepted here because correctness (no lost
/// entries, no manifest describing files that were never placed) matters
/// more than one caller waiting slightly longer.
fn apply_placements_in_with(
    manifest_path: &Path,
    backup_root: &Path,
    root: &ApprovedRoot,
    placements: Vec<Placement>,
    restore: impl Fn(&Path, &Path) -> std::io::Result<u64>,
) -> Result<Vec<ManagedFile>, String> {
    let _guard = lock_manifest();
    apply_placements_in_with_locked(manifest_path, backup_root, root, placements, restore)
}

/// Body of [`apply_placements_in_with`]; assumes [`MANIFEST_LOCK`] is already
/// held. Must never acquire the lock itself and must never call another
/// function that does — `std::sync::Mutex` is not reentrant.
fn apply_placements_in_with_locked(
    manifest_path: &Path,
    backup_root: &Path,
    root: &ApprovedRoot,
    placements: Vec<Placement>,
    restore: impl Fn(&Path, &Path) -> std::io::Result<u64>,
) -> Result<Vec<ManagedFile>, String> {
    // Gate 4, re-asserted. `begin_write` proved the client was idle when the
    // token was minted, which may have been a multi-minute download ago. This
    // is the last point before any byte is written, and it is inside
    // MANIFEST_LOCK, so no concurrent batch can slip a write in between the
    // check and the placement.
    root.reassert_idle()?;
    let client_root = root.path();

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
                let failed = roll_back_with(backup_root, &placed, created_dirs, &restore);
                if failed.is_empty() {
                    return Err(error);
                }
                return Err(record_incomplete_rollback_locked(
                    manifest_path,
                    client_root,
                    &placed,
                    &failed,
                    error,
                ));
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
        let failed = roll_back_with(backup_root, &placed, created_dirs, &restore);
        if failed.is_empty() {
            return Err(error);
        }
        return Err(record_incomplete_rollback_locked(
            manifest_path,
            client_root,
            &placed,
            &failed,
            error,
        ));
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
        kind: placement.kind,
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
    root: &ApprovedRoot,
    relative_paths: &[String],
) -> Result<Vec<String>, String> {
    let manifest = manifest_path(app)?;
    let backups = backup_root(app)?;
    revert_placements_in(&manifest, &backups, root, relative_paths)
}

/// Inner form of [`revert_placements`], testable without an `AppHandle`.
///
/// Takes [`MANIFEST_LOCK`] across the whole load-mutate-save sequence (and
/// the file removal/restore work that must stay consistent with what gets
/// written to the manifest), for the same reason
/// [`apply_placements_in_with`] does: without it, a revert racing an apply
/// (or another revert) on the same manifest can read a bucket that is about
/// to be overwritten and lose the other call's changes when it saves.
fn revert_placements_in(
    manifest_path: &Path,
    backup_root: &Path,
    root: &ApprovedRoot,
    relative_paths: &[String],
) -> Result<Vec<String>, String> {
    let _guard = lock_manifest();
    revert_placements_in_locked(manifest_path, backup_root, root, relative_paths)
}

/// Body of [`revert_placements_in`]; assumes [`MANIFEST_LOCK`] is already
/// held. Must never acquire the lock itself.
fn revert_placements_in_locked(
    manifest_path: &Path,
    backup_root: &Path,
    root: &ApprovedRoot,
    relative_paths: &[String],
) -> Result<Vec<String>, String> {
    // Gate 4 again. Removing a proxy DLL the running client has loaded fails
    // on Windows anyway, but restoring a displaced original under a live
    // client is the case worth refusing: the game would be reading a file
    // mid-rewrite.
    root.reassert_idle()?;
    let client_root = root.path();

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

        /// A write token for this harness's client dir, reporting the client
        /// idle. Tests about gate 4 build their own with [`root_with`].
        fn root(&self) -> ApprovedRoot {
            ApprovedRoot::for_tests_idle(self.client.clone())
        }

        /// A write token whose running check answers `check`, so a test can
        /// make the client appear to start part-way through a batch.
        fn root_with(
            &self,
            check: impl Fn() -> Result<bool, String> + Send + Sync + 'static,
        ) -> ApprovedRoot {
            ApprovedRoot::for_tests(self.client.clone(), std::sync::Arc::new(check))
        }

        fn apply(&self, placements: Vec<Placement>) -> Result<Vec<ManagedFile>, String> {
            self.apply_as(&self.root(), placements)
        }

        fn apply_as(
            &self,
            root: &ApprovedRoot,
            placements: Vec<Placement>,
        ) -> Result<Vec<ManagedFile>, String> {
            apply_placements_in(&self.manifest, &self.backups, root, placements)
        }

        /// Apply with an injected rollback restore-copy, so the mixed-state
        /// path is reachable without a real filesystem failure.
        fn apply_with_restore(
            &self,
            placements: Vec<Placement>,
            restore: impl Fn(&Path, &Path) -> std::io::Result<u64>,
        ) -> Result<Vec<ManagedFile>, String> {
            apply_placements_in_with(
                &self.manifest,
                &self.backups,
                &self.root(),
                placements,
                restore,
            )
        }

        fn revert(&self, paths: &[String]) -> Result<Vec<String>, String> {
            self.revert_as(&self.root(), paths)
        }

        fn revert_as(&self, root: &ApprovedRoot, paths: &[String]) -> Result<Vec<String>, String> {
            revert_placements_in(&self.manifest, &self.backups, root, paths)
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

    /// Stands in for the real reasons a restore copy fails on a game directory:
    /// antivirus, Controlled Folder Access, a DLL held open by a running game.
    fn restore_always_fails(_: &Path, _: &Path) -> std::io::Result<u64> {
        Err(std::io::Error::other(
            "the process cannot access the file because it is being used by another process",
        ))
    }

    #[test]
    fn a_failed_rollback_keeps_the_users_original_referenced_and_unprunable() {
        let h = Harness::new();
        fs::write(h.client.join("dxgi.dll"), "the-users-original-dll").expect("seed");

        let error = h
            .apply_with_restore(
                vec![
                    h.placement("dxgi.dll", "kalpas-new-dll"),
                    // Rejected mid-batch, forcing the rollback.
                    h.placement("../evil.dll", "pwned"),
                ],
                restore_always_fails,
            )
            .expect_err("the batch must fail");

        let entries = h.entries();
        assert_eq!(
            entries.len(),
            1,
            "the un-rolled-back placement must be recorded: {entries:?}"
        );
        let id = entries[0]
            .displaced_backup
            .clone()
            .expect("the entry must still point at the backup holding the original");
        let backup = backup_file_path(&h.backups, &id, "dxgi.dll").expect("backup path");
        assert_eq!(
            fs::read_to_string(&backup).expect("read backup"),
            "the-users-original-dll"
        );

        // Now flood the backup root with unreferenced folders and prune. The
        // whole point of writing that manifest entry is that this cannot reach
        // the user's original.
        for index in 0..MAX_UNREFERENCED_BACKUPS + 5 {
            let dir = h.backups.join(format!("2020-01-01T00-00-00Z-{index:06}-0"));
            fs::create_dir_all(&dir).expect("mkdir");
            fs::write(dir.join("stale.bin"), "stale").expect("write");
        }
        prune_unreferenced_backups(&h.backups, &h.manifest());

        assert!(
            h.backups.join(&id).is_dir(),
            "the backup holding the user's original must survive pruning"
        );
        assert_eq!(
            fs::read_to_string(&backup).expect("read backup after prune"),
            "the-users-original-dll",
            "the user's original must still be recoverable byte-for-byte"
        );
        assert!(
            error.contains("dxgi.dll"),
            "error must name the file: {error}"
        );
    }

    #[test]
    fn a_failed_rollback_reports_a_mixed_state_without_losing_the_original_cause() {
        let h = Harness::new();
        fs::write(h.client.join("dxgi.dll"), "the-users-original-dll").expect("seed");

        let error = h
            .apply_with_restore(
                vec![
                    h.placement("dxgi.dll", "kalpas-new-dll"),
                    h.placement("../evil.dll", "pwned"),
                ],
                restore_always_fails,
            )
            .expect_err("the batch must fail");

        assert!(
            error.contains("'..'"),
            "the original failure must remain the primary cause: {error}"
        );
        assert!(
            error.contains("mixed state"),
            "the error must say the folder is in a mixed state: {error}"
        );
        assert!(
            error.contains("dxgi.dll"),
            "the error must name the affected file: {error}"
        );
        assert!(
            error.contains("backup folder"),
            "the error must say where the original is preserved: {error}"
        );
        assert_eq!(
            h.read("dxgi.dll"),
            "kalpas-new-dll",
            "a file that could not be restored keeps Kalpa's bytes, never nothing"
        );
    }

    #[test]
    fn a_successful_rollback_records_nothing_and_restores_byte_identically() {
        let h = Harness::new();
        fs::write(h.client.join("dxgi.dll"), "the-users-original-dll").expect("seed");

        let error = h
            .apply_with_restore(
                vec![
                    h.placement("dxgi.dll", "kalpas-new-dll"),
                    h.placement("reshade-shaders/Shaders/Bloom.fx", "// bloom"),
                    h.placement("../evil.dll", "pwned"),
                ],
                |from: &Path, to: &Path| fs::copy(from, to),
            )
            .expect_err("the batch must fail");

        assert!(error.contains("'..'"), "unexpected error: {error}");
        assert!(
            !error.contains("mixed state"),
            "a clean rollback must not claim a mixed state: {error}"
        );
        assert_eq!(
            h.read("dxgi.dll"),
            "the-users-original-dll",
            "restore-by-overwrite must put back byte-identical content"
        );
        assert!(
            !h.client.join("reshade-shaders/Shaders/Bloom.fx").exists(),
            "no file placed by the batch may survive a clean rollback"
        );
        assert!(
            !h.client.join("reshade-shaders").exists(),
            "directories the batch created must be removed too"
        );
        assert!(
            h.entries().is_empty(),
            "a cleanly rolled-back batch must record nothing"
        );
    }

    #[test]
    fn a_recorded_mixed_state_can_be_reverted_afterwards() {
        let h = Harness::new();
        fs::write(h.client.join("dxgi.dll"), "the-users-original-dll").expect("seed");

        h.apply_with_restore(
            vec![
                h.placement("dxgi.dll", "kalpas-new-dll"),
                h.placement("../evil.dll", "pwned"),
            ],
            restore_always_fails,
        )
        .expect_err("the batch must fail");

        let skipped = h.revert(&["dxgi.dll".to_string()]).expect("revert");

        assert!(skipped.is_empty(), "nothing had drifted: {skipped:?}");
        assert_eq!(
            h.read("dxgi.dll"),
            "the-users-original-dll",
            "the recorded entry is what makes the original recoverable"
        );
        assert!(h.entries().is_empty());
    }

    // ── MANIFEST_LOCK regression coverage ──────────────────────────────

    /// Without the lock, two threads each doing load-modify-save on the same
    /// manifest race: the second save silently discards whatever the first
    /// thread's copy recorded. This is the actual bug — a double-clicked
    /// button, or an install racing a preset switch — and a lost entry here
    /// is a ghost file: uninstall can never find it, and its backup folder
    /// becomes unreferenced and eventually pruned out from under the user's
    /// own displaced original.
    #[test]
    fn concurrent_applies_to_different_files_lose_no_entries() {
        for _ in 0..30 {
            let h = Harness::new();

            std::thread::scope(|scope| {
                let h_ref = &h;
                scope.spawn(|| {
                    h_ref
                        .apply(vec![h_ref.placement("dxgi.dll", "a")])
                        .expect("apply a should succeed");
                });
                scope.spawn(|| {
                    h_ref
                        .apply(vec![h_ref.placement("ReShade.ini", "b")])
                        .expect("apply b should succeed");
                });
            });

            let entries = h.entries();
            let paths: std::collections::BTreeSet<&str> =
                entries.iter().map(|e| e.relative_path.as_str()).collect();
            assert_eq!(
                entries.len(),
                2,
                "both concurrent placements must be recorded, not just one: {entries:?}"
            );
            assert!(paths.contains("dxgi.dll"), "lost entry: {entries:?}");
            assert!(paths.contains("ReShade.ini"), "lost entry: {entries:?}");
            assert!(h.client.join("dxgi.dll").is_file());
            assert!(h.client.join("ReShade.ini").is_file());
        }
    }

    /// An apply and a revert racing on the same manifest must not leave it
    /// pointing at a file that no longer exists, nor silently keep a file on
    /// disk with no manifest entry to make it reachable again.
    #[test]
    fn concurrent_apply_and_revert_stay_self_consistent() {
        for _ in 0..30 {
            let h = Harness::new();
            h.apply(vec![h.placement("existing.ini", "seed")])
                .expect("seed placement should succeed");
            assert!(h.client.join("existing.ini").is_file());

            std::thread::scope(|scope| {
                let h_ref = &h;
                scope.spawn(|| {
                    h_ref
                        .apply(vec![h_ref.placement("new.dll", "fresh")])
                        .expect("apply new should succeed");
                });
                scope.spawn(|| {
                    h_ref
                        .revert(&["existing.ini".to_string()])
                        .expect("revert existing should succeed");
                });
            });

            let entries = h.entries();
            let paths: std::collections::BTreeSet<&str> =
                entries.iter().map(|e| e.relative_path.as_str()).collect();

            assert!(
                paths.contains("new.dll"),
                "the concurrent apply's entry must survive the interleaved revert: {entries:?}"
            );
            assert!(
                !paths.contains("existing.ini"),
                "a reverted entry must not still be recorded: {entries:?}"
            );
            assert!(
                h.client.join("new.dll").is_file(),
                "an entry must not exist without its file on disk"
            );
            assert!(
                !h.client.join("existing.ini").exists(),
                "a removed file must not still be reachable through the manifest"
            );
        }
    }

    /// Gate 4 is re-asserted inside the placement, not just when the token is
    /// minted. `begin_write` can succeed and then the user launches the game
    /// while a 40 MB download finishes; these cover that window.
    mod client_started_mid_batch {
        use super::*;

        #[test]
        fn apply_refuses_and_places_nothing() {
            let h = Harness::new();
            let root = h.root_with(|| Ok(true));

            let error = h
                .apply_as(&root, vec![h.placement("dxgi.dll", "reshade")])
                .expect_err("apply must refuse while the client is active");

            assert!(
                error.contains("running"),
                "the refusal should say the client is running: {error}"
            );
            assert!(
                !h.client.join("dxgi.dll").exists(),
                "a refused batch must not place any file"
            );
            assert!(
                h.entries().is_empty(),
                "a refused batch must not record anything: {:?}",
                h.entries()
            );
        }

        #[test]
        fn revert_refuses_and_keeps_the_file_and_its_entry() {
            let h = Harness::new();
            h.apply(vec![h.placement("dxgi.dll", "reshade")])
                .expect("seed placement should succeed");

            let root = h.root_with(|| Ok(true));
            let error = h
                .revert_as(&root, &["dxgi.dll".to_string()])
                .expect_err("revert must refuse while the client is active");

            assert!(
                error.contains("running"),
                "the refusal should say the client is running: {error}"
            );
            assert!(
                h.client.join("dxgi.dll").is_file(),
                "a refused revert must not remove the file"
            );
            assert_eq!(
                h.entries().len(),
                1,
                "a refused revert must leave the manifest entry, or the file becomes a ghost"
            );
        }

        /// A check that cannot answer is not an idle client. Treating an
        /// errored process walk as "idle" would turn the one gate that
        /// protects a live install into a no-op on exactly the machines where
        /// process enumeration is restricted.
        #[test]
        fn an_unanswerable_check_refuses_rather_than_assuming_idle() {
            let h = Harness::new();
            let root = h.root_with(|| Err("process snapshot failed".to_string()));

            let error = h
                .apply_as(&root, vec![h.placement("dxgi.dll", "reshade")])
                .expect_err("apply must refuse when the check cannot answer");

            assert!(
                error.contains("process snapshot failed"),
                "the underlying failure should reach the user: {error}"
            );
            assert!(!h.client.join("dxgi.dll").exists());
            assert!(h.entries().is_empty());
        }
    }
}
