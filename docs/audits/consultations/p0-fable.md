# Full P0 design consultation (P0-A1/A2/A3)

You are the read-only architecture advisor for Kalpa, a public-beta Tauri v2 desktop app with a shipped Slint sidecar. Produce the full P0 design before implementation begins. The executor will implement only P0-A1 in this branch, but the A1 API must leave a sound path for A2 and A3.

## Findings and acceptance criteria

### P0-A1 — shared crash-safe atomic writer

`metadata::save_json_with_backup` and several other writers use fixed staging names. The Tauri process and Slint sidecar share modules and can target the same files, so writers can truncate, rename, or clean up one another's staging file.

- Both Rust crates use one shared atomic-write implementation.
- Temporary names are unique per process and operation.
- Replacement data is flushed before rename.
- Existing metadata primary/backup recovery semantics are preserved.
- Failure cleanup removes only the current operation's staging file.
- No fixed shared `json.tmp` filename remains.
- The helper explicitly does not solve read-modify-write races; P0-A2 does.
- Audit/migrate metadata, settings, safe migration, SavedVariables I/O, edit backups, and Slint duplicates/shared paths.
- Tests cover concurrent thread writes, path uniqueness, owned cleanup on failure, recoverable primary/backup, and both crates compiling the helper.

### P0-A2 — cross-process read-modify-write locking

Atomic replacement prevents torn files but not lost updates. An OS-level lock must span read → mutate → write for `kalpa.json`, `settings.json`, `kalpa-profiles.json`, and confirmed sidecar mirrors.

- Recommend a safe cross-platform locking mechanism and dependency strategy for both crates.
- Define canonical lock identity, ordering for multi-store operations, timeout/cancellation/user-visible behavior, crash release, and stale lock-file behavior.
- Never infer safety from a PID and delete a lock file.
- Preserve useful in-process mutexes.
- Tests must use two processes, kill the owner, prove no lost update, prove ordering avoids deadlock, and prove both crates use one protocol.

### P0-A3 — native-sidecar readiness handshake

The launcher currently relies partly on a fixed delay and the existing `native-boot.pending` recovery marker. Parent exit must depend on a positive child-ready signal, while duplicate/deep-link launches must not create two independent writers.

- Deliberately extend or replace (not ignore) `native-boot.pending`.
- Define ready identity/authentication, timeout, child-exit-before-ready, stale marker recovery, duplicate sidecars, deep links, shutdown/retry logs, and WebView fallback.
- No fixed sleep determines correctness.
- Final P0 design must explain how A3 prevents concurrent writers while A2 remains defense in depth.

## Current code excerpts

### Excerpt 1 — tested settings atomic writer (condensed from `settings_store.rs`)

```rust
static WRITE_LOCK: Mutex<()> = Mutex::new(());
static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_staging(main: &Path) -> PathBuf {
    let n = STAGING_COUNTER.fetch_add(1, Ordering::Relaxed);
    suffixed(main, &format!(".tmp-{}-{n}", std::process::id()))
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    let mut f = fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

fn rename_with_retries(from: &Path, to: &Path) -> io::Result<()> {
    for attempt in 0..5 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 4 => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let staging = unique_staging(path);
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = write_synced(&staging, bytes) {
        let _ = fs::remove_file(&staging); return Err(e);
    }
    if let Err(e) = rename_with_retries(&staging, path) {
        let _ = fs::remove_file(&staging); return Err(e);
    }
    Ok(())
}
```

The tests already prove whole-file replacement, unique staging does not clobber an existing leftover, cleanup on rename failure, and parseable output. Recovery discards `<primary>.tmp-*` as uncommitted and quarantines corrupt primaries.

### Excerpt 2 — metadata backup/recovery uses a fixed `.json.tmp`

```rust
pub fn load_json_with_backup<T: DeserializeOwned + Default>(path: &Path) -> T {
    if let Ok(content) = fs::read_to_string(path) {
        if let Ok(data) = serde_json::from_str(&content) { return data; }
    }
    let tmp = path.with_extension("json.tmp");
    if let Ok(content) = fs::read_to_string(&tmp) {
        if let Ok(data) = serde_json::from_str::<T>(&content) {
            let _ = fs::remove_file(path);
            let _ = fs::rename(&tmp, path);
            return data;
        }
    }
    let bak = path.with_extension("json.bak");
    if let Ok(content) = fs::read_to_string(&bak) {
        if let Ok(data) = serde_json::from_str::<T>(&content) { return data; }
    }
    T::default()
}

pub fn save_json_with_backup<T: Serialize>(path: &Path, data: &T) -> Result<(), String> {
    let json = serde_json::to_string_pretty(data)?;
    let tmp = path.with_extension("json.tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(json.as_bytes())?;
    file.sync_all()?;
    let bak = path.with_extension("json.bak");
    let _ = fs::copy(path, &bak);
    fs::rename(&tmp, path).map_err(|e| { let _ = fs::remove_file(&tmp); e })
}
```

The intended semantics are: primary first; legacy `.json.tmp` recovery second; `.json.bak` previous-valid fallback third. A new design must not treat arbitrary unique staging leftovers as committed data. Compatibility with an existing legacy `.json.tmp` recovery artifact is desirable.

### Excerpt 3 — other shared fixed-name writers

```rust
// saved_variables/io.rs
let tmp_path = sv_dir.join(format!("{file_name}.tmp"));
let mut f = fs::File::create(&tmp_path)?;
f.write_all(content)?;
f.sync_all()?;
fs::rename(&tmp_path, &file_path).map_err(|e| {
    let _ = fs::remove_file(&tmp_path); e
})?;

// edit_backups.rs restore
let tmp_path = dest.parent().unwrap().join(format!("{name}.kalpa-restore-tmp"));
fs::write(&tmp_path, &bytes)?;
fs::rename(&tmp_path, &dest).map_err(|e| {
    let _ = fs::remove_file(&tmp_path); e
})?;

// safe_migration.rs copy destination
let tmp_dest = dest.with_extension("kalpa-tmp");
fs::copy(&src, &tmp_dest)?;
fs::rename(&tmp_dest, &dest).map_err(|e| {
    let _ = fs::remove_file(&tmp_dest); e
})?;
```

`safe_migration` also streams a potentially large ZIP to `<id>.zip.tmp`, calls `sync_all`, hashes it, then renames to the final archive. A shared solution should support streamed writes without buffering whole archives in memory.

### Excerpt 4 — Slint has duplicate byte/string writers

```rust
fn write_file_atomically(path: &Path, content: &[u8]) -> Result<(), String> {
    let temp = path.with_file_name(format!("{name}.kalpa-tmp"));
    fs::write(&temp, content)?;
    fs::rename(&temp, path).map_err(|e| {
        let _ = fs::remove_file(&temp); e
    })
}

fn write_string_atomic(path: &Path, contents: &str) -> Result<(), String> {
    fs::create_dir_all(path.parent().unwrap())?;
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let staging = path.with_file_name(format!("{name}.tmp-{}-{nanos}", process::id()));
    let mut file = fs::File::create(&staging)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    fs::rename(&staging, path).map_err(|e| {
        let _ = fs::remove_file(&staging); e
    })
}
```

Slint path-includes shared `metadata.rs`, `edit_backups.rs`, and `safe_migration.rs`, but has its own crate root and its own settings/hash writer call sites.

### Excerpt 5 — A3 launcher context

The repository already has a `native-boot.pending` marker for startup recovery, duplicate-sidecar detection, deep-link paths that can open the WebView while Slint is active, and a launcher path that waits a fixed ~300 ms before treating startup as successful. A3 must inspect exact symbols before implementation; this consultation should define protocol invariants rather than invent code from line-number hints.

## Repository constraints

- Windows is primary, but macOS/Linux builds must compile and file replacement must remain correct.
- Use safe Rust and avoid new dependencies unless justified. Dependencies, if chosen, must be added via Cargo tooling to both affected crates.
- The same source file can be path-included by the Tauri and Slint crates. Avoid Tauri-specific imports in a shared filesystem helper.
- P0-A1 must remain narrowly atomic-publication only. It cannot claim to serialize cross-process RMW.
- Do not delete or promote another operation's staging file. Cleanup ownership must be structural, not guessed from age/PID.
- Preserve persisted JSON and backup wire formats. No schema changes.
- Existing in-process mutexes may remain where useful, but a crate-local mutex is not cross-process protection.
- Windows AV/Controlled Folder Access can cause transient create/rename failures; bounded retry is allowed, indefinite blocking is not.
- Directory metadata fsync is not uniformly available/meaningful on Windows. State the exact guarantee honestly.
- Tests must avoid touching real ESO/app-data state.

## Candidate designs drafted by the executor

### Candidate A — shared `atomic_file.rs` with owned staging guard and streamed transaction

Path-include one dependency-free module in both crate roots. It exposes `atomic_write(path, bytes)` plus `AtomicFile::new(path)`, `writer()`, and `commit()` for streamed ZIP/copy use. Creation uses `create_new(true)` on a sibling name containing PID + process-local atomic counter, retrying only name collisions. The guard deletes only its exact owned staging path on drop/error. `commit` flushes and `sync_all`s, closes the handle, then uses bounded rename retry. Optional callback/hook runs after staging sync but before rename so metadata can copy the prior primary to its stable `.json.bak`; helper never manages semantic backups itself. Unique staging leftovers are always uncommitted; legacy `.json.tmp` is read-only compatibility recovery and is never a future write target.

A2 later wraps whole RMW transactions in a separate shared OS lock API; A1 remains usable inside that lock. A3 uses authenticated parent-child launch IDs and a ready channel/marker tied to the pending record, with the parent retaining WebView until positive ready.

### Candidate B — byte-only shared helper plus bespoke streamed writers

Centralize only small byte/string writes. Keep ZIP and `fs::copy` staging bespoke, but give each site a common `unique_staging_path` utility and cleanup pattern. This is smaller but may violate “one shared atomic-write implementation,” duplicates durability/rename logic, and makes bug-class drift likely.

### Candidate C — dependency-backed temp files and locks now

Use `tempfile::NamedTempFile::persist` for A1 and introduce `fs4` locks in the same foundational module in anticipation of A2. This delegates unique cleanup but adds dependencies to both crates and risks coupling A1 publication with an incompletely reviewed locking protocol. Windows replace semantics of `persist` must be verified rather than assumed.

## Failure modes to evaluate

1. Two processes create staging files in the same clock tick or have counters at the same value.
2. One writer fails to rename while another writer commits; cleanup must not unlink the other's staging or primary.
3. Crash/power loss before sync, after sync but before backup, after backup but before rename, and immediately after rename.
4. The previous primary is corrupt but `.json.bak` is valid; a failed new save must not destroy the last recoverable copy.
5. Legacy fixed `.json.tmp` exists during upgrade while new unique staging leftovers also exist.
6. Windows AV holds staging or destination transiently; bounded retry must not silently degrade to remove-then-rename.
7. Destination is a directory/permission denied/disk full; primary must remain intact and only owned staging is cleaned.
8. Stream writer errors or is dropped without commit.
9. A1 atomic replacement is mistakenly used as evidence that RMW is race-free.
10. A2 lock identity aliases (relative path, case, symlink), opposite acquisition order, process death, timeout, or stale lock-file bytes.
11. A3 ready signal comes from a stale/different child, child becomes ready then exits, duplicate process starts, deep link races startup, or marker survives power loss.

## Required output

Return only this structure:

```text
DECISION:
1. Chosen design and numbered implementation steps

REJECTED:
1. Alternative and the concrete failure that rejects it

CRASH_RECOVERY:
1. Behavior after process kill, power loss, stale marker, timeout, or partial write

TESTS:
1. Tests distinguishing a correct design from a plausible but incorrect one

RISKS:
1. Remaining risks and required human decisions
```
