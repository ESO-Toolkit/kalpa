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
//! [`flush`] serialises the cache and writes it with the standard
//! write-temp + fsync + atomic-rename pattern, so the on-disk file is only ever
//! the previous complete file or the next complete file — never a partial one.
//! Each write stages to a *unique* temp name, so a crashed write's leftover is
//! never the name a later write reuses — a recovery candidate can never be
//! clobbered. [`recover`] runs before the store is first opened and repairs
//! on-disk state left by a crash; settings it cannot write back to the primary
//! are returned for [`seed_recovered`] to put into the live cache so they are not
//! lost.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

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
/// reuses — recovery candidates therefore cannot be clobbered.
const STAGING_INFIX: &str = ".tmp-";
/// Where an unrecoverable corrupt primary is set aside for inspection.
const QUARANTINE_SUFFIX: &str = ".corrupt";

/// Serialises [`atomic_write`] within this process so two writers (e.g. a
/// frontend flush racing the token migration) take consistent turns.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Per-process counter making each staging file name unique.
static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut s: OsString = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// A unique staging path for one write, e.g. `settings.json.tmp-12345-7`. Unique
/// per write so a crashed write's leftover never collides with — and so can never
/// be clobbered by — a later write.
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

/// File modification time, or the epoch if it can't be read — used only to pick
/// the freshest recovery candidate, so a missing timestamp just sorts oldest.
fn mtime(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
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
/// in place, and the unique staging name means this write can never clobber a
/// recovery candidate left by a previous crash.
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

    write_synced(&staging, bytes)?;
    if let Err(e) = rename_with_retries(&staging, path) {
        // The rename never landed. Drop our staging file so it isn't leaked, and
        // surface the error: `path` was never touched, so the previous good file
        // survives.
        let _ = fs::remove_file(&staging);
        return Err(e);
    }
    Ok(())
}

/// Read a settings file and return its bytes only if it parses as a JSON object
/// — the same shape the plugin deserialises into (`HashMap<String, Value>`). A
/// truncated or half-written file returns `None`, which is what recovery keys off.
fn read_if_valid(path: &Path) -> Option<Vec<u8>> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice::<BTreeMap<String, JsonValue>>(&bytes).ok()?;
    Some(bytes)
}

/// Move a corrupt primary aside so plugin-store starts clean instead of silently
/// merging nothing, and a human can inspect what was lost.
fn quarantine(main: &Path) {
    if !main.exists() {
        return;
    }
    let dest = suffixed(main, QUARANTINE_SUFFIX);
    let _ = fs::remove_file(&dest); // overwrite any prior quarantine
    match fs::rename(main, &dest) {
        Ok(()) => eprintln!("[settings_store] recovery: quarantined corrupt {main:?} -> {dest:?}"),
        Err(e) => eprintln!("[settings_store] recovery: failed to quarantine {main:?}: {e}"),
    }
}

/// Repair on-disk settings state left by an interrupted write, BEFORE the plugin
/// opens (and merge-loads) the file. Returns the recovered settings map ONLY when
/// it held a valid copy that it could not write back to the primary — so the
/// caller can seed it into the live store ([`seed_recovered`]).
///
///   * primary valid → it is the committed authority; drop uncommitted staging
///     leftovers; return None.
///   * primary missing/corrupt → promote the freshest complete staging leftover
///     (a crash-staged write). On success, clean up the rest; return None.
///   * promotion fails (primary locked) → leave the staging leftovers untouched
///     (their unique names keep them safe for the next launch to retry),
///     quarantine the corrupt primary, and return the recovered map so the caller
///     seeds the live cache.
///   * nothing recoverable → quarantine a corrupt primary; return None.
fn recover_path(main: &Path) -> Option<BTreeMap<String, JsonValue>> {
    let staging = staging_files(main);

    // The primary, when valid, is the committed authority. atomic_write only ever
    // publishes a complete file, so a staging leftover here is an uncommitted save
    // (its rename never completed) — discard them all.
    if read_if_valid(main).is_some() {
        for s in &staging {
            let _ = fs::remove_file(s);
        }
        return None;
    }

    // Primary missing or corrupt — only possible from external corruption or a
    // pre-fix partial write, since atomic_write never leaves an invalid primary.
    // Promote the freshest complete staged copy if there is one. Staging files
    // have unique names, so none has been clobbered by a later write.
    let Some(src) = staging
        .iter()
        .filter(|s| read_if_valid(s).is_some())
        .max_by_key(|s| mtime(s))
    else {
        // Nothing usable. Quarantine a corrupt primary and drop junk staging files.
        quarantine(main);
        for s in &staging {
            let _ = fs::remove_file(s);
        }
        return None;
    };

    // Hold the recovered bytes so we can seed the cache if the on-disk promotion
    // can't complete.
    let bytes = read_if_valid(src)?;

    if rename_with_retries(src, main).is_ok() && read_if_valid(main).is_some() {
        for s in &staging {
            if s != src {
                let _ = fs::remove_file(s);
            }
        }
        eprintln!("[settings_store] recovery: restored {main:?} from {src:?}");
        return None;
    }

    // Could not write the primary (e.g. it is locked). Leave the staging files
    // in place — their unique names keep them safe from later writes, so a future
    // launch retries — and quarantine the corrupt primary. Return the recovered
    // settings so the caller seeds them into the live cache, making this session
    // show them and persist them on the next successful write.
    quarantine(main);
    eprintln!(
        "[settings_store] recovery: could not rewrite {main:?}; seeding cache, will retry next launch"
    );
    serde_json::from_slice::<BTreeMap<String, JsonValue>>(&bytes).ok()
}

/// Resolve the absolute settings.json path via the plugin's own resolver, so it
/// always matches the file the plugin store reads and writes.
fn settings_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    tauri_plugin_store::resolve_store_path(app, STORE_FILE).ok()
}

/// Repair on-disk settings state left by a crash. Call once, before the plugin
/// store is first opened, so the repaired file is what the plugin's load() reads.
/// Returns settings that could not be written back to the primary; pass them to
/// [`seed_recovered`] after the store opens so they become the live state.
#[must_use]
pub fn recover<R: Runtime>(app: &AppHandle<R>) -> Option<BTreeMap<String, JsonValue>> {
    recover_path(&settings_path(app)?)
}

/// Seed settings that [`recover`] could not write back to disk into the live
/// store, then persist them. Call right after the store is first opened. Making
/// the recovered settings the live state means they are not lost — an early write
/// persists them rather than an empty store, and the flush writes them through as
/// soon as the primary is writable again.
pub fn seed_recovered<R: Runtime>(
    app: &AppHandle<R>,
    recovered: Option<BTreeMap<String, JsonValue>>,
) {
    let Some(map) = recovered else {
        return;
    };
    let Some(store) = app.get_store(STORE_FILE) else {
        eprintln!("[settings_store] recovery: store not open; cannot seed recovered settings");
        return;
    };
    for (key, value) in map {
        store.set(key, value);
    }
    if let Err(e) = flush(app) {
        eprintln!("[settings_store] recovery: seeded cache but could not persist yet: {e}");
    }
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
    fn atomic_write_does_not_clobber_an_existing_staging_leftover() {
        let dir = temp_dir("no-clobber");
        let main = dir.join("settings.json");
        // A recovery candidate from an earlier crash.
        let leftover = staged(&main, "old");
        fs::write(&leftover, b"{\"recoverable\":1}").unwrap();

        // A normal write uses a unique staging name, so the leftover is untouched.
        atomic_write(&main, b"{\"new\":1}").unwrap();

        assert_eq!(fs::read(&main).unwrap(), b"{\"new\":1}");
        assert_eq!(read_if_valid(&leftover).unwrap(), b"{\"recoverable\":1}");
    }

    #[test]
    fn recovery_keeps_valid_primary_and_drops_staging() {
        let dir = temp_dir("valid-primary");
        let main = dir.join("settings.json");
        fs::write(&main, b"{\"keep\":1}").unwrap();
        // A leftover staging file from some earlier crash.
        fs::write(staged(&main, "x"), b"{\"stale\":1}").unwrap();

        assert!(
            recover_path(&main).is_none(),
            "valid primary needs no seeding"
        );

        assert_eq!(fs::read(&main).unwrap(), b"{\"keep\":1}");
        assert!(staging_files(&main).is_empty(), "stale staging removed");
    }

    #[test]
    fn recovery_promotes_staged_write_when_primary_is_corrupt() {
        let dir = temp_dir("promote-corrupt");
        let main = dir.join("settings.json");
        // A corrupt/partial primary alongside a complete staged write.
        fs::write(&main, b"{\"half").unwrap();
        fs::write(staged(&main, "x"), b"{\"recovered\":42}").unwrap();

        assert!(
            recover_path(&main).is_none(),
            "successful promotion needs no seeding"
        );

        assert_eq!(fs::read(&main).unwrap(), b"{\"recovered\":42}");
        assert!(staging_files(&main).is_empty());
    }

    #[test]
    fn recovery_promotes_staged_write_when_primary_is_missing() {
        let dir = temp_dir("promote-missing");
        let main = dir.join("settings.json");
        fs::write(staged(&main, "x"), b"{\"recovered\":true}").unwrap();

        recover_path(&main);

        assert_eq!(fs::read(&main).unwrap(), b"{\"recovered\":true}");
        assert!(staging_files(&main).is_empty());
    }

    #[test]
    fn recovery_promotes_a_valid_staging_over_an_invalid_one() {
        let dir = temp_dir("valid-over-invalid");
        let main = dir.join("settings.json");
        fs::write(&main, b"{\"corrupt").unwrap();
        fs::write(staged(&main, "a"), b"{\"half").unwrap(); // invalid
        fs::write(staged(&main, "b"), b"{\"good\":1}").unwrap(); // valid

        recover_path(&main);

        assert_eq!(fs::read(&main).unwrap(), b"{\"good\":1}");
        assert!(staging_files(&main).is_empty(), "all staging cleaned up");
    }

    #[test]
    fn recovery_seeds_cache_and_keeps_staging_when_promotion_fails() {
        let dir = temp_dir("promote-fail");
        let main = dir.join("settings.json");
        // Make `main` a directory so renaming a file over it fails deterministically
        // on every platform, forcing the promotion-failure path.
        fs::create_dir(&main).unwrap();
        let leftover = staged(&main, "x");
        fs::write(&leftover, b"{\"saved\":1}").unwrap();

        let recovered = recover_path(&main);

        // Authoritative: the recovered settings are returned to seed the live cache.
        assert_eq!(
            recovered.unwrap().get("saved"),
            Some(&serde_json::json!(1)),
            "recovered settings are returned for cache seeding"
        );
        // The staging leftover is kept (unique name → safe from later writes), so a
        // future launch can still recover it from disk.
        assert_eq!(read_if_valid(&leftover).unwrap(), b"{\"saved\":1}");
    }

    #[test]
    fn recovery_quarantines_corrupt_primary_with_no_staged_write() {
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
    fn recovery_drops_invalid_staging_when_primary_missing() {
        let dir = temp_dir("invalid-staging");
        let main = dir.join("settings.json");
        // Neither file is usable: a half-written staging file, no primary.
        fs::write(staged(&main, "x"), b"{\"half").unwrap();

        recover_path(&main);

        assert!(!main.exists());
        assert!(
            staging_files(&main).is_empty(),
            "invalid staging should be removed"
        );
    }

    #[test]
    fn recovery_is_noop_on_fresh_install() {
        let dir = temp_dir("fresh");
        let main = dir.join("settings.json");

        assert!(recover_path(&main).is_none()); // must not panic or create anything

        assert!(!main.exists());
        assert!(staging_files(&main).is_empty());
    }

    #[test]
    fn empty_file_is_treated_as_invalid() {
        let dir = temp_dir("empty");
        let main = dir.join("settings.json");
        fs::write(&main, b"").unwrap();

        assert!(read_if_valid(&main).is_none());
    }
}
