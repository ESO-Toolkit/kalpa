//! Crash-atomic persistence for the tauri-plugin-store `settings.json` file.
//!
//! tauri-plugin-store's own `Store::save` does a truncate-in-place
//! `std::fs::write(path, bytes)` (see the crate's `store.rs`). An interrupted
//! write — power loss, process kill, disk-full, AV — can leave `settings.json`
//! truncated or half-written, destroying every persisted user setting. The
//! plugin's `serialize` hook only controls the *bytes*; the non-atomic write
//! itself is not interceptable, and the plugin additionally flushes every store
//! with that same non-atomic write on `RunEvent::Exit`.
//!
//! This module owns settings persistence instead. The plugin store stays the
//! single in-memory cache — it is shared by the Rust first-opener and the JS
//! write paths because plugin-store caches one instance per path — but we stop
//! calling the plugin's `save()`:
//!   * the Rust migration and the JS `setSetting`/`setSettings` paths persist
//!     via [`flush`] (exposed as the `flush_settings` command), and
//!   * on `RunEvent::ExitRequested` the app calls [`detach_on_exit`], which drops
//!     the store from the plugin registry so the plugin's exit handler can't
//!     truncate-write it after us (it does not flush — every write already did).
//!
//! [`flush`] writes with the standard write-temp + fsync + atomic-rename pattern:
//! it stages the bytes into a unique temp file, fsyncs them, then renames that
//! over the primary. The rename is atomic on a single NTFS volume, so the on-disk
//! `settings.json` is only ever the previous complete file or the next complete
//! file — never a partial one.
//!
//! The primary is therefore the *sole* committed truth: a successful write lives
//! in `settings.json`, and a staging leftover only exists when a write's rename
//! never completed — i.e. an uncommitted write. [`recover`] runs before the store
//! opens and simply discards those uncommitted leftovers and quarantines a
//! primary that fails to parse (external corruption or a pre-fix partial write),
//! so the plugin starts from clean, complete state.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value as JsonValue;
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

/// The settings store path, relative to the plugin's AppData base directory.
const STORE_FILE: &str = "settings.json";

/// Number of times to attempt the final rename. On Windows an AV scanner or the
/// search indexer can briefly hold the destination open, failing the rename
/// transiently; a few short retries absorb that without giving up on the save.
const RENAME_ATTEMPTS: usize = 5;
const RENAME_BACKOFF: Duration = Duration::from_millis(40);

/// Infix for per-write staging files: `settings.json.tmp-<pid>-<n>`. Staging
/// names are unique so a crashed write's leftover is never the name a later write
/// reuses (no clobber), and recovery can recognise and clear them all.
const STAGING_INFIX: &str = ".tmp-";
/// Where a corrupt primary is set aside for inspection.
const QUARANTINE_SUFFIX: &str = ".corrupt";

/// Serialises [`atomic_write`] within this process so two writers (e.g. a
/// frontend flush racing the token migration) take consistent turns.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Per-process counter making each staging file name unique.
static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Set when the store opened with an empty cache while the primary on disk may
/// still hold real settings (it couldn't be loaded — e.g. a transient lock, since
/// plugin-store swallows load errors). While tainted, [`flush`] refuses to write
/// so the empty cache can't overwrite the real file; it clears once a reload of
/// the primary succeeds.
static SETTINGS_TAINTED: AtomicBool = AtomicBool::new(false);

fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut s: OsString = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// A unique staging path for one write, e.g. `settings.json.tmp-12345-7`.
fn unique_staging(main: &Path) -> PathBuf {
    let n = STAGING_COUNTER.fetch_add(1, Ordering::Relaxed);
    suffixed(main, &format!("{STAGING_INFIX}{}-{n}", std::process::id()))
}

/// Every staging leftover for `main` currently on disk (siblings whose file name
/// starts with `<main file name>.tmp-`).
fn staging_files(main: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (main.parent(), main.file_name()) else {
        return Vec::new();
    };
    let mut prefix_os = name.to_owned();
    prefix_os.push(STAGING_INFIX);
    let prefix = prefix_os.to_string_lossy();

    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(prefix.as_ref())
            {
                out.push(entry.path());
            }
        }
    }
    out
}

/// Write `bytes` to `path` (truncating) and fsync them to disk before returning,
/// so the file's contents are durable once this succeeds.
fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

/// Rename `from` → `to`, retrying briefly. On Windows an AV scanner or the search
/// indexer can hold the destination open and fail the rename transiently; a few
/// short retries absorb that without giving up.
fn rename_with_retries(from: &Path, to: &Path) -> io::Result<()> {
    let mut last_err = None;
    for attempt in 0..RENAME_ATTEMPTS {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                if attempt + 1 < RENAME_ATTEMPTS {
                    std::thread::sleep(RENAME_BACKOFF);
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| io::Error::other("rename failed")))
}

/// Crash-atomic file write: stage the full contents into a unique `<path>.tmp-*`,
/// fsync it, then rename it over `path`. fsync-before-rename makes the staged
/// bytes durable before the rename publishes them, and the rename is atomic on a
/// single NTFS volume — so a crash at any point leaves either the old complete
/// file or the new complete file, never a partial one. `path` is never truncated
/// in place.
///
/// On ANY error (write, fsync, or rename) the staging file is removed before
/// returning, so a failed write never leaves a readable file behind that a later
/// recovery could mistake for a committed save. `path` itself is only touched by
/// the final rename, so a failure leaves the previous good file intact.
///
/// Durability nuance: we do not force write-through on the rename's metadata, so
/// a power cut within the OS's metadata-flush window may leave the *previous*
/// complete file (the just-confirmed save can roll back). That preserves the
/// no-corruption guarantee but not last-write durability; a stronger guarantee
/// would need a platform-specific write-through replace (e.g. `MoveFileExW` with
/// `MOVEFILE_WRITE_THROUGH`).
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let staging = unique_staging(path);

    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    if let Err(e) = write_synced(&staging, bytes) {
        let _ = fs::remove_file(&staging);
        return Err(e);
    }
    if let Err(e) = rename_with_retries(&staging, path) {
        let _ = fs::remove_file(&staging);
        return Err(e);
    }
    Ok(())
}

/// State of the primary settings file, distinguishing genuine corruption from a
/// merely-unreadable file. Conflating the two would let recovery quarantine a
/// good file that is only transiently locked (e.g. by an AV scan).
enum PrimaryState {
    /// Readable and parses as a settings object — the shape the plugin loads.
    Valid,
    /// Readable but does not parse: genuinely corrupt data.
    Corrupt,
    /// Could not be read at all: missing, or a transient I/O / lock error.
    Unreadable,
}

fn classify(path: &Path) -> PrimaryState {
    match fs::read(path) {
        Ok(bytes) => {
            if serde_json::from_slice::<BTreeMap<String, JsonValue>>(&bytes).is_ok() {
                PrimaryState::Valid
            } else {
                PrimaryState::Corrupt
            }
        }
        Err(_) => PrimaryState::Unreadable,
    }
}

/// Move a corrupt primary aside so plugin-store starts clean instead of silently
/// merging nothing, and a human can inspect what was lost.
fn quarantine(main: &Path) {
    let dest = suffixed(main, QUARANTINE_SUFFIX);
    let _ = fs::remove_file(&dest); // overwrite any prior quarantine
    match fs::rename(main, &dest) {
        Ok(()) => eprintln!("[settings_store] recovery: quarantined corrupt {main:?} -> {dest:?}"),
        Err(e) => eprintln!("[settings_store] recovery: failed to quarantine {main:?}: {e}"),
    }
}

/// Repair on-disk settings state BEFORE the plugin opens (and merge-loads) the
/// file. atomic_write only ever publishes a complete primary, so:
///   * staging leftovers (`<main>.tmp-*`) are uncommitted writes whose rename
///     never completed — discard them. They are never promoted: resurrecting a
///     write the caller was never told succeeded would be wrong, and a leftover
///     can even be a write whose fsync failed.
///   * a primary that is readable but doesn't parse is external corruption or a
///     pre-fix partial write — quarantine it so the plugin starts clean rather
///     than silently loading partial JSON.
///   * a primary that exists but is *unreadable* is left untouched: it may be a
///     good file behind a transient lock, and moving it aside would destroy it.
///     [`ensure_loaded`] then guards against the plugin opening it as empty.
fn recover_path(main: &Path) {
    for s in staging_files(main) {
        let _ = fs::remove_file(&s);
    }
    match classify(main) {
        PrimaryState::Corrupt => quarantine(main),
        PrimaryState::Unreadable if main.exists() => eprintln!(
            "[settings_store] recovery: {main:?} is present but unreadable; leaving it in place"
        ),
        PrimaryState::Valid | PrimaryState::Unreadable => {}
    }
}

/// Resolve the absolute settings.json path via the plugin's own resolver, so it
/// always matches the file the plugin store reads and writes.
fn settings_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    tauri_plugin_store::resolve_store_path(app, STORE_FILE).ok()
}

/// Repair on-disk settings state left by a crash. Call once, before the plugin
/// store is first opened, so the repaired file is what the plugin's load() reads.
pub fn recover<R: Runtime>(app: &AppHandle<R>) {
    if let Some(path) = settings_path(app) {
        recover_path(&path);
    }
}

/// Whether an empty in-memory cache must NOT be trusted to overwrite the primary
/// — i.e. the file on disk may still hold real settings the load missed. True when
/// the primary parses as a non-empty object, or exists but can't be read at all
/// (it might be good data behind a transient lock). A missing, empty, or corrupt
/// primary has nothing worth protecting.
fn primary_may_hold_settings(path: &Path) -> bool {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<BTreeMap<String, JsonValue>>(&bytes)
            .map(|m| !m.is_empty())
            .unwrap_or(false),
        Err(_) => path.exists(),
    }
}

/// Read and parse the primary into a settings map, or `None` if it can't be read
/// or doesn't parse as a JSON object.
fn read_settings_map(path: &Path) -> Option<BTreeMap<String, JsonValue>> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Guard against tauri-plugin-store silently swallowing a load error: its
/// `StoreBuilder::build_inner` ignores `load()` failures, so a settings file that
/// was transiently unreadable when the store opened yields an *empty* in-memory
/// cache. If the file may hold real settings, retry the load; if that still can't
/// load it, taint the store so [`flush`] refuses to overwrite the file until it
/// loads. Call once, right after the store is first opened. No-op for a genuinely
/// empty/fresh store.
pub fn ensure_loaded<R: Runtime>(app: &AppHandle<R>) {
    let Some(store) = app.get_store(STORE_FILE) else {
        return;
    };
    if !store.is_empty() {
        return; // cache already has data — load succeeded
    }
    let Some(path) = settings_path(app) else {
        return;
    };
    if !primary_may_hold_settings(&path) {
        return; // missing/empty/corrupt primary — the empty cache is fine to write
    }
    // Retry the swallowed load so the cache reflects disk before anything flushes.
    if store.reload().is_ok() && !store.is_empty() {
        return;
    }
    // Still couldn't load the file's settings into the cache. Refuse to overwrite
    // it until it loads, so a later flush can't replace real settings with empty.
    SETTINGS_TAINTED.store(true, Ordering::SeqCst);
    eprintln!(
        "[settings_store] settings file present but could not be loaded; refusing to overwrite it until it loads"
    );
}

/// Atomically persist the plugin store's current in-memory cache to disk,
/// replacing the plugin's non-atomic `Store::save`. Errors (rather than silently
/// succeeding) if the store isn't open — which only happens after shutdown
/// detach — so a caller can tell its write was not persisted.
pub fn flush<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let Some(store) = app.get_store(STORE_FILE) else {
        // The store is only absent after `detach_on_exit` removes it during
        // shutdown. Report a real error rather than a false success, so a write
        // that lost its race with shutdown surfaces as failed instead of being
        // silently reported as persisted.
        return Err("settings store is not open".to_string());
    };

    let path = settings_path(app).ok_or_else(|| "could not resolve settings path".to_string())?;

    if SETTINGS_TAINTED.load(Ordering::SeqCst) {
        // The store opened empty over a file that may hold real settings, so the
        // caller's write was applied to an untrusted (empty) cache. Merge WITHOUT
        // touching the live cache or the taint flag, so that if the write fails the
        // frontend's rollback snapshot (taken from the empty cache) stays valid:
        // build `disk ∪ pending` in a LOCAL map (pending wins conflicts) and write
        // that. Only AFTER the bytes are durably written do we bring the live cache
        // up to date and clear the taint — then return success, so no rollback runs.
        let Some(mut merged) = read_settings_map(&path) else {
            // Still can't read the file; refuse rather than overwrite it. The
            // frontend rolls its write back; a later attempt retries the reload.
            return Err("settings could not be loaded yet; not overwriting them".to_string());
        };
        for (key, value) in store.entries() {
            merged.insert(key, value);
        }
        let bytes = serde_json::to_vec_pretty(&merged).map_err(|e| e.to_string())?;
        atomic_write(&path, &bytes).map_err(|e| e.to_string())?;
        let _ = store.reload(); // disk now equals `merged`; sync the live cache
        SETTINGS_TAINTED.store(false, Ordering::SeqCst);
        return Ok(());
    }

    // Snapshot the cache into a sorted map: deterministic key order keeps the
    // on-disk file stable across saves (smaller diffs) and is still exactly what
    // the plugin deserialises back. 2-space pretty output matches the plugin's
    // default serializer, so the file format is unchanged.
    let cache: BTreeMap<String, JsonValue> = store.entries().into_iter().collect();
    let bytes = serde_json::to_vec_pretty(&cache).map_err(|e| e.to_string())?;

    atomic_write(&path, &bytes).map_err(|e| e.to_string())
}

/// Drop the store from the plugin's registry at shutdown so tauri-plugin-store's
/// own `RunEvent::Exit` handler can't perform a non-atomic truncate-write of
/// `settings.json` after us (its exit save runs before our run-callback for the
/// same event, so we must detach during the earlier `ExitRequested`).
///
/// We deliberately do NOT flush here: every `setSetting`/`setSettings` already
/// persists atomically the moment it completes, so the on-disk file is current
/// as of the last *finished* write. Flushing the live cache at exit could instead
/// snapshot a multi-key batch mid-flight (set but not yet flushed) and persist it
/// half-applied — the very inconsistency the batched flush exists to prevent.
///
/// Detaching during `ExitRequested` is safe because Kalpa never prevents exit:
/// window close hides to tray, and this callback runs after every plugin's
/// `ExitRequested` handler, so by the time it runs exit is committed. If a future
/// change starts calling `ExitRequestApi::prevent`, this detach must move with it
/// (a closed store would strand the live frontend).
pub fn detach_on_exit<R: Runtime>(app: &AppHandle<R>) {
    if let Some(store) = app.get_store(STORE_FILE) {
        store.close_resource();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    /// Unique temp dir per test, no external crates and no `Date`/random.
    fn temp_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "kalpa-settings-test-{}-{}-{tag}",
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A staging-leftover path with a stable, recognisable name (matches the
    /// `<main>.tmp-*` pattern that recovery scans for).
    fn staged(main: &Path, tag: &str) -> PathBuf {
        suffixed(main, &format!("{STAGING_INFIX}{tag}"))
    }

    #[test]
    fn atomic_write_creates_complete_file_and_cleans_up_staging() {
        let dir = temp_dir("create");
        let main = dir.join("settings.json");

        atomic_write(&main, b"{\"a\":1}").unwrap();

        assert_eq!(fs::read(&main).unwrap(), b"{\"a\":1}");
        // No staging file may linger after a successful save.
        assert!(staging_files(&main).is_empty(), "staging should be removed");
    }

    #[test]
    fn atomic_write_replaces_existing_file_wholesale() {
        let dir = temp_dir("replace");
        let main = dir.join("settings.json");
        fs::write(&main, b"{\"old\":true}").unwrap();

        atomic_write(&main, b"{\"new\":true}").unwrap();

        assert_eq!(fs::read(&main).unwrap(), b"{\"new\":true}");
    }

    #[test]
    fn atomic_write_removes_staging_and_keeps_primary_on_failure() {
        let dir = temp_dir("write-fail");
        // Make the target a directory so the final rename fails deterministically
        // on every platform, exercising the failure cleanup path.
        let main = dir.join("settings.json");
        fs::create_dir(&main).unwrap();

        assert!(
            atomic_write(&main, b"{\"a\":1}").is_err(),
            "writing over a directory must fail"
        );
        assert!(
            staging_files(&main).is_empty(),
            "staging is removed when the write fails"
        );
    }

    #[test]
    fn atomic_write_does_not_clobber_an_existing_staging_leftover() {
        let dir = temp_dir("no-clobber");
        let main = dir.join("settings.json");
        // A leftover from an earlier crash.
        let leftover = staged(&main, "old");
        fs::write(&leftover, b"{\"leftover\":1}").unwrap();

        // A normal write uses a unique staging name, so the leftover is untouched.
        atomic_write(&main, b"{\"new\":1}").unwrap();

        assert_eq!(fs::read(&main).unwrap(), b"{\"new\":1}");
        assert_eq!(fs::read(&leftover).unwrap(), b"{\"leftover\":1}");
    }

    #[test]
    fn recovery_keeps_valid_primary_and_drops_staging() {
        let dir = temp_dir("valid-primary");
        let main = dir.join("settings.json");
        fs::write(&main, b"{\"keep\":1}").unwrap();
        fs::write(staged(&main, "x"), b"{\"stale\":1}").unwrap();

        recover_path(&main);

        assert_eq!(fs::read(&main).unwrap(), b"{\"keep\":1}");
        assert!(staging_files(&main).is_empty(), "stale staging removed");
    }

    #[test]
    fn recovery_does_not_promote_staging_over_a_corrupt_primary() {
        // A staging leftover is an uncommitted write — even with valid JSON it must
        // NOT be promoted (it may be a write whose fsync failed and which already
        // reported failure to the caller).
        let dir = temp_dir("no-promote");
        let main = dir.join("settings.json");
        fs::write(&main, b"{\"corrupt").unwrap();
        fs::write(staged(&main, "x"), b"{\"uncommitted\":1}").unwrap();

        recover_path(&main);

        assert!(!main.exists(), "corrupt primary is quarantined");
        assert_eq!(
            fs::read(suffixed(&main, ".corrupt")).unwrap(),
            b"{\"corrupt"
        );
        assert!(
            staging_files(&main).is_empty(),
            "uncommitted staging is discarded, never promoted"
        );
    }

    #[test]
    fn recovery_quarantines_corrupt_primary_with_no_staging() {
        let dir = temp_dir("quarantine");
        let main = dir.join("settings.json");
        fs::write(&main, b"{\"truncated").unwrap();

        recover_path(&main);

        assert!(!main.exists(), "corrupt primary should be moved aside");
        assert_eq!(
            fs::read(suffixed(&main, ".corrupt")).unwrap(),
            b"{\"truncated"
        );
    }

    #[test]
    fn recovery_drops_staging_when_primary_missing() {
        let dir = temp_dir("missing-primary");
        let main = dir.join("settings.json");
        fs::write(staged(&main, "x"), b"{\"uncommitted\":1}").unwrap();

        recover_path(&main);

        // No committed primary to recover; the uncommitted staging is just cleared.
        assert!(!main.exists());
        assert!(staging_files(&main).is_empty());
    }

    #[test]
    fn recovery_is_noop_on_fresh_install() {
        let dir = temp_dir("fresh");
        let main = dir.join("settings.json");

        recover_path(&main); // must not panic or create anything

        assert!(!main.exists());
        assert!(staging_files(&main).is_empty());
        assert!(!suffixed(&main, ".corrupt").exists());
    }

    #[test]
    fn empty_file_is_classified_corrupt() {
        let dir = temp_dir("empty");
        let main = dir.join("settings.json");
        fs::write(&main, b"").unwrap();

        // Readable but unparseable → corrupt (gets quarantined), distinct from a
        // file that cannot be read at all.
        assert!(matches!(classify(&main), PrimaryState::Corrupt));
    }

    #[test]
    fn recovery_leaves_an_unreadable_primary_in_place() {
        let dir = temp_dir("unreadable");
        let main = dir.join("settings.json");
        // A directory at the primary path makes fs::read fail on every platform,
        // standing in for a file behind a transient lock.
        fs::create_dir(&main).unwrap();
        fs::write(staged(&main, "x"), b"{\"uncommitted\":1}").unwrap();

        recover_path(&main);

        // The unreadable primary is NOT quarantined — it may be a good file behind
        // a transient lock, and moving it aside would destroy it. Only uncommitted
        // staging is cleared.
        assert!(matches!(classify(&main), PrimaryState::Unreadable));
        assert!(main.exists(), "unreadable primary left in place");
        assert!(!suffixed(&main, ".corrupt").exists(), "not quarantined");
        assert!(staging_files(&main).is_empty());
    }

    #[test]
    fn primary_may_hold_settings_protects_only_recoverable_files() {
        let dir = temp_dir("may-hold");

        let missing = dir.join("missing.json");
        assert!(
            !primary_may_hold_settings(&missing),
            "missing → nothing to protect"
        );

        let empty_obj = dir.join("empty.json");
        fs::write(&empty_obj, b"{}").unwrap();
        assert!(
            !primary_may_hold_settings(&empty_obj),
            "empty {{}} → nothing to protect"
        );

        let with_data = dir.join("data.json");
        fs::write(&with_data, b"{\"a\":1}").unwrap();
        assert!(
            primary_may_hold_settings(&with_data),
            "real settings → protect"
        );

        let corrupt = dir.join("corrupt.json");
        fs::write(&corrupt, b"{\"half").unwrap();
        assert!(
            !primary_may_hold_settings(&corrupt),
            "corrupt → quarantined separately, nothing loadable to protect"
        );

        // A directory reads as an I/O error but exists — stands in for a file
        // behind a transient lock, which may hold real settings.
        let unreadable = dir.join("locked.json");
        fs::create_dir(&unreadable).unwrap();
        assert!(
            primary_may_hold_settings(&unreadable),
            "present but unreadable → may hold settings, must protect"
        );
    }
}
