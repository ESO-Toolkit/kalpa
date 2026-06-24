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

/// Serialises the file-write half of [`atomic_write`] within this process, so a
/// late frontend flush and the shutdown flush can never both be mid-write
/// against the shared temp path.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// `settings.json` → `settings.json.tmp`. Appends a suffix rather than replacing
/// the extension so the temp name can never collide with the real file's stem.
fn tmp_path(main: &Path) -> PathBuf {
    suffixed(main, ".tmp")
}

fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut s: OsString = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Crash-atomic file write: stage the full contents into `<path>.tmp`, fsync it,
/// then rename it over `path`. fsync-before-rename guarantees the temp file's
/// bytes are durable before it becomes the canonical file; the rename is atomic
/// on a single NTFS volume, so a crash at any point leaves either the old
/// complete file or the new complete file. `path` is never truncated in place.
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = tmp_path(path);

    // Hold the lock across the whole stage-and-rename so two concurrent writers
    // can't clobber each other's shared temp file.
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // Stage + fsync the complete contents before the rename makes them canonical.
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }

    let mut last_err = None;
    for attempt in 0..RENAME_ATTEMPTS {
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                if attempt + 1 < RENAME_ATTEMPTS {
                    std::thread::sleep(RENAME_BACKOFF);
                }
            }
        }
    }

    // The rename never landed. Drop the temp so it isn't leaked, and surface the
    // error: `path` itself was never touched, so the previous good file survives.
    let _ = fs::remove_file(&tmp);
    Err(last_err.unwrap_or_else(|| io::Error::other("rename failed")))
}

/// Read a settings file and return its bytes only if it parses as a JSON object
/// — the same shape the plugin deserialises into (`HashMap<String, Value>`). A
/// truncated or half-written file returns `None`, which is what recovery keys off.
fn read_if_valid(path: &Path) -> Option<Vec<u8>> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice::<BTreeMap<String, JsonValue>>(&bytes).ok()?;
    Some(bytes)
}

/// Repair on-disk settings state left by an interrupted write, BEFORE the plugin
/// opens (and merge-loads) the file. Handles the only states [`atomic_write`]
/// can leave on a crash, plus a pre-existing corrupt primary:
///   * primary present & valid → drop any stale `.tmp`; done.
///   * primary missing/corrupt + `.tmp` valid → promote the `.tmp` (a save whose
///     rename didn't land; the staged file is a complete, fsynced snapshot).
///   * primary corrupt + no valid `.tmp` → quarantine the corrupt file to
///     `.corrupt` so the plugin starts clean instead of silently merging nothing
///     (and a human can inspect what was lost).
fn recover_path(main: &Path) {
    let tmp = tmp_path(main);

    if read_if_valid(main).is_some() {
        if tmp.exists() {
            let _ = fs::remove_file(&tmp);
        }
        return;
    }

    // Primary is missing or corrupt. Promote a complete staged write if we have
    // one. Route the restore through `atomic_write` rather than a bare rename so
    // that (a) a transient lock is retried, and (b) the staging file is consumed
    // on success — otherwise a later write would reuse the same `.tmp` name and
    // truncate this recoverable copy. The validated bytes are held in memory, so
    // the data is safe even as the staging file is rewritten.
    if let Some(bytes) = read_if_valid(&tmp) {
        match atomic_write(main, &bytes) {
            Ok(()) => eprintln!("[settings_store] recovery: restored {main:?} from staged write"),
            Err(e) => eprintln!(
                "[settings_store] recovery: failed to restore {main:?} from staged write: {e}"
            ),
        }
        return;
    }

    // No usable staged copy. Quarantine a corrupt primary so it doesn't get
    // silently swallowed by the plugin's load, then start fresh.
    if main.exists() {
        let quarantine = suffixed(main, ".corrupt");
        let _ = fs::remove_file(&quarantine); // overwrite any prior quarantine
        match fs::rename(main, &quarantine) {
            Ok(()) => eprintln!(
                "[settings_store] recovery: quarantined corrupt {main:?} -> {quarantine:?}; starting fresh"
            ),
            Err(e) => eprintln!("[settings_store] recovery: failed to quarantine {main:?}: {e}"),
        }
    }
    // Drop a present-but-invalid temp so it can't be mistaken for a good one later.
    if tmp.exists() {
        let _ = fs::remove_file(&tmp);
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

/// Atomically persist the plugin store's current in-memory cache to disk,
/// replacing the plugin's non-atomic `Store::save`. A no-op (returns `Ok`) if
/// the store has not been opened yet — there is nothing to persist.
pub fn flush<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let Some(store) = app.get_store(STORE_FILE) else {
        return Ok(());
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

/// Final atomic flush at shutdown, then drop the store from the plugin's
/// registry so tauri-plugin-store's own `RunEvent::Exit` handler can't perform a
/// non-atomic truncate-write of `settings.json` after us. Call from the app's
/// `RunEvent::ExitRequested` handler (which fires before `Exit`).
pub fn flush_and_detach<R: Runtime>(app: &AppHandle<R>) {
    if let Err(e) = flush(app) {
        eprintln!("[settings_store] shutdown flush failed: {e}");
    }
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
