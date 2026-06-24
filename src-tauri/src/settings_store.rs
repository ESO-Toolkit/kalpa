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
//!   * on `RunEvent::ExitRequested` the app calls [`flush_and_detach`], which
//!     does a final atomic flush and then drops the store from the plugin
//!     registry so the plugin's exit handler can't truncate-write it after us.
//!
//! [`flush`] serialises the cache and writes it with the standard
//! write-temp + fsync + atomic-rename pattern, so the on-disk file is only ever
//! the previous complete file or the next complete file — never a partial one.
//! [`recover`] runs before the store is first opened and repairs on-disk state
//! left by a crash (a stray `.tmp`, or a corrupt primary file).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
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

/// Serialises [`atomic_write`] within this process, so two writers (e.g. a
/// frontend flush racing the token migration) can never both be mid-write
/// against the shared staging path.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Runtime staging suffix. A crash between staging and rename leaves a complete
/// `settings.json.tmp`; recovery treats it as a candidate (see [`recover_path`]).
const STAGING_SUFFIX: &str = ".tmp";
/// Where recovery parks a complete copy it could not promote, under a name the
/// runtime write path never reuses — so a later write cannot destroy it.
const PRESERVED_SUFFIX: &str = ".bak";
/// Distinct staging name for promotion, so promoting never touches the candidate
/// it is reading from.
const PROMOTE_SUFFIX: &str = ".recovering";
/// Where an unrecoverable corrupt primary is set aside for inspection.
const QUARANTINE_SUFFIX: &str = ".corrupt";

fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut s: OsString = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// `settings.json` → `settings.json.tmp` (the runtime staging path).
fn tmp_path(main: &Path) -> PathBuf {
    suffixed(main, STAGING_SUFFIX)
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

/// Crash-atomic file write: stage the full contents into `<path>.tmp`, fsync it,
/// then rename it over `path`. fsync-before-rename makes the staged bytes durable
/// before the rename publishes them, and the rename is atomic on a single NTFS
/// volume — so a crash at any point leaves either the old complete file or the
/// new complete file, never a partial one. `path` is never truncated in place.
///
/// Durability nuance: we do not force write-through on the rename's metadata, so
/// a power cut within the OS's metadata-flush window may leave the *previous*
/// complete file (the just-confirmed save can roll back). That preserves the
/// no-corruption guarantee but not last-write durability; a stronger guarantee
/// would need a platform-specific write-through replace (e.g. `MoveFileExW` with
/// `MOVEFILE_WRITE_THROUGH`).
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = tmp_path(path);

    // Hold the lock across the whole stage-and-rename so two concurrent writers
    // can't clobber each other's shared staging file.
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    write_synced(&tmp, bytes)?;
    if let Err(e) = rename_with_retries(&tmp, path) {
        // The rename never landed. Drop the staging file so it isn't leaked, and
        // surface the error: `path` was never touched, so the previous good file
        // survives.
        let _ = fs::remove_file(&tmp);
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

/// Durably replace `main` with `bytes` via a promotion-only staging file (never
/// the runtime `.tmp`, so a concurrent/later write can't interfere), retrying the
/// rename and re-validating the result. Returns `true` only when `main` now holds
/// the recovered content. On any failure `main` is left untouched.
fn promote(main: &Path, bytes: &[u8]) -> bool {
    let staging = suffixed(main, PROMOTE_SUFFIX);
    let ok = write_synced(&staging, bytes).is_ok()
        && rename_with_retries(&staging, main).is_ok()
        && read_if_valid(main).is_some();
    if !ok {
        let _ = fs::remove_file(&staging);
    }
    ok
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
/// opens (and merge-loads) the file:
///   * primary valid → it is the authority; drop stale `.tmp`/`.bak` and return.
///   * primary missing/corrupt → restore from the newest complete copy we have —
///     a crash-staged write (`.tmp`), else a preserved copy (`.bak`) parked by an
///     earlier failed promotion — promoting it via a separate staging file so the
///     source is never destroyed before `main` is durably replaced.
///   * promotion fails → preserve the recovered bytes under `.bak` (a name the
///     runtime write path never reuses) and quarantine the corrupt primary, so a
///     future launch can retry without the recoverable copy being clobbered.
///   * nothing recoverable → quarantine a corrupt primary and start fresh.
fn recover_path(main: &Path) {
    let tmp = tmp_path(main);
    let bak = suffixed(main, PRESERVED_SUFFIX);

    // The primary, when valid, is always the authority — drop any leftovers.
    if read_if_valid(main).is_some() {
        let _ = fs::remove_file(&tmp);
        let _ = fs::remove_file(&bak);
        return;
    }

    // Primary missing/corrupt — try the freshest complete copy first.
    if let Some(bytes) = read_if_valid(&tmp) {
        if promote(main, &bytes) {
            let _ = fs::remove_file(&tmp);
            let _ = fs::remove_file(&bak);
            eprintln!("[settings_store] recovery: restored {main:?} from staged write");
            return;
        }
        // Could not durably replace `main`. Move the staged copy aside to `.bak`
        // via an atomic, non-truncating rename (a name the runtime write path
        // never reuses), so a later `.tmp` write can't clobber it and a future
        // launch can retry. Crucially this consumes `.tmp` only if the move
        // succeeds; if even the move fails, `.tmp` is left as the surviving copy
        // rather than risking the loss of the only recovered bytes.
        match rename_with_retries(&tmp, &bak) {
            Ok(()) => eprintln!(
                "[settings_store] recovery: staged promotion failed; preserved copy at {bak:?}"
            ),
            Err(e) => eprintln!(
                "[settings_store] recovery: staged promotion failed and could not park copy ({e}); keeping {tmp:?}"
            ),
        }
        quarantine(main);
        return;
    }

    if let Some(bytes) = read_if_valid(&bak) {
        if promote(main, &bytes) {
            let _ = fs::remove_file(&tmp);
            let _ = fs::remove_file(&bak);
            eprintln!("[settings_store] recovery: restored {main:?} from preserved copy");
            return;
        }
        // Keep `.bak` intact for the next launch; just clean up and quarantine.
        let _ = fs::remove_file(&tmp);
        quarantine(main);
        eprintln!("[settings_store] recovery: preserved-copy promotion failed; keeping {bak:?}");
        return;
    }

    // Nothing recoverable. Quarantine a corrupt primary and drop a stale `.tmp`.
    quarantine(main);
    let _ = fs::remove_file(&tmp);
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
    use std::sync::atomic::{AtomicU32, Ordering};

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

    #[test]
    fn atomic_write_creates_complete_file_and_cleans_up_temp() {
        let dir = temp_dir("create");
        let main = dir.join("settings.json");

        atomic_write(&main, b"{\"a\":1}").unwrap();

        assert_eq!(fs::read(&main).unwrap(), b"{\"a\":1}");
        // The staging file must not linger after a successful save.
        assert!(!tmp_path(&main).exists(), "temp file should be removed");
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
    fn recovery_keeps_valid_primary_and_drops_stale_temp() {
        let dir = temp_dir("valid-primary");
        let main = dir.join("settings.json");
        fs::write(&main, b"{\"keep\":1}").unwrap();
        // A leftover temp from some earlier crash.
        fs::write(tmp_path(&main), b"{\"stale\":1}").unwrap();

        recover_path(&main);

        assert_eq!(fs::read(&main).unwrap(), b"{\"keep\":1}");
        assert!(!tmp_path(&main).exists(), "stale temp should be removed");
    }

    #[test]
    fn recovery_promotes_staged_write_when_primary_is_corrupt() {
        let dir = temp_dir("promote-corrupt");
        let main = dir.join("settings.json");
        // Simulate a save interrupted mid-rename: corrupt/partial primary, complete temp.
        fs::write(&main, b"{\"half").unwrap();
        fs::write(tmp_path(&main), b"{\"recovered\":42}").unwrap();

        recover_path(&main);

        assert_eq!(fs::read(&main).unwrap(), b"{\"recovered\":42}");
        assert!(!tmp_path(&main).exists());
    }

    #[test]
    fn recovery_promotes_staged_write_when_primary_is_missing() {
        let dir = temp_dir("promote-missing");
        let main = dir.join("settings.json");
        fs::write(tmp_path(&main), b"{\"recovered\":true}").unwrap();

        recover_path(&main);

        assert_eq!(fs::read(&main).unwrap(), b"{\"recovered\":true}");
        assert!(!tmp_path(&main).exists());
    }

    #[test]
    fn recovery_restores_from_preserved_bak_when_no_temp() {
        let dir = temp_dir("restore-bak");
        let main = dir.join("settings.json");
        fs::write(&main, b"{\"corrupt").unwrap();
        // No `.tmp`, but a `.bak` parked by an earlier failed promotion.
        fs::write(suffixed(&main, ".bak"), b"{\"preserved\":1}").unwrap();

        recover_path(&main);

        assert_eq!(fs::read(&main).unwrap(), b"{\"preserved\":1}");
        assert!(!suffixed(&main, ".bak").exists(), "preserved copy consumed");
    }

    #[test]
    fn recovery_prefers_temp_over_preserved_bak() {
        let dir = temp_dir("prefer-temp");
        let main = dir.join("settings.json");
        fs::write(&main, b"{\"corrupt").unwrap();
        fs::write(tmp_path(&main), b"{\"newest\":1}").unwrap();
        fs::write(suffixed(&main, ".bak"), b"{\"older\":1}").unwrap();

        recover_path(&main);

        // The crash-staged write is newer than a parked copy and wins.
        assert_eq!(fs::read(&main).unwrap(), b"{\"newest\":1}");
        assert!(!tmp_path(&main).exists());
        assert!(!suffixed(&main, ".bak").exists());
    }

    #[test]
    fn recovery_preserves_recovered_bytes_when_promotion_fails() {
        let dir = temp_dir("promote-fail");
        let main = dir.join("settings.json");
        // Make `main` a directory so renaming a file over it fails deterministically
        // on every platform, forcing the promotion failure path.
        fs::create_dir(&main).unwrap();
        fs::write(tmp_path(&main), b"{\"saved\":1}").unwrap();

        recover_path(&main);

        // The recovered bytes are parked under `.bak` (a name runtime writes never
        // reuse) so a future launch can retry — they are never lost.
        assert_eq!(
            read_if_valid(&suffixed(&main, ".bak")).unwrap(),
            b"{\"saved\":1}"
        );
        assert!(!tmp_path(&main).exists(), "stale staging dropped");
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
    fn recovery_drops_invalid_temp_when_primary_missing() {
        let dir = temp_dir("invalid-temp");
        let main = dir.join("settings.json");
        // Neither file is usable: a half-written temp, no primary.
        fs::write(tmp_path(&main), b"{\"half").unwrap();

        recover_path(&main);

        assert!(!main.exists());
        assert!(!tmp_path(&main).exists(), "invalid temp should be removed");
    }

    #[test]
    fn recovery_is_noop_on_fresh_install() {
        let dir = temp_dir("fresh");
        let main = dir.join("settings.json");

        recover_path(&main); // must not panic or create anything

        assert!(!main.exists());
        assert!(!tmp_path(&main).exists());
    }

    #[test]
    fn empty_file_is_treated_as_invalid() {
        let dir = temp_dir("empty");
        let main = dir.join("settings.json");
        fs::write(&main, b"").unwrap();

        assert!(read_if_valid(&main).is_none());
    }

    #[test]
    fn recovery_consumes_staging_file_so_a_later_write_cant_clobber_it() {
        // Reviewer scenario: corrupt primary + valid staged write. After recovery
        // the staged file must be gone, so the next atomic_write (which stages to
        // the same `.tmp` name) can't truncate the recoverable copy.
        let dir = temp_dir("consume-staging");
        let main = dir.join("settings.json");
        fs::write(&main, b"{\"corrupt").unwrap();
        fs::write(tmp_path(&main), b"{\"saved\":1}").unwrap();

        recover_path(&main);
        assert_eq!(read_if_valid(&main).unwrap(), b"{\"saved\":1}");
        assert!(!tmp_path(&main).exists(), "staging file must be consumed");

        // A subsequent write proceeds normally and does not resurrect stale data.
        atomic_write(&main, b"{\"saved\":2}").unwrap();
        assert_eq!(read_if_valid(&main).unwrap(), b"{\"saved\":2}");
    }
}
