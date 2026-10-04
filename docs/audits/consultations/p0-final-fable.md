# Fable Consultation — Final P0 Review before P0-A3 merges

This is the master prompt's required final review of the whole P0 lane before
`fix/audit-p0-a3-sidecar-handshake` (PR #389) merges. A1 and A2 are already
implemented and Sol-approved; A3 is complete and gated. You are reviewing the
assembled lane for remaining correctness gaps, not choosing a new design.

## The lane

- **P0-A1** (#380) — `src-tauri/src/atomic_file.rs`. One shared crash-safe
  publisher: staging file via `create_new`, `flush` + `sync_all`, drop handle,
  rename with retries. Promise is "old-or-new, never torn".
- **P0-A2** (#388) — `src-tauri/src/transaction_lock.rs`. Bounded cross-process
  read-modify-write locking on `fs4` advisory locks, canonical target identity,
  ordered multi-lock acquisition to avoid deadlock.
- **P0-A3** (#389) — `src-tauri/src/native_boot.rs` plus handoff logic in
  `commands.rs` and `lib.rs`. Replaces a fixed `sleep(300ms)` with a ready
  handshake and an OS-lock-backed UI authority transfer between the Tauri
  WebView shell and the shipped Slint sidecar.

## Acceptance criteria for A3

- Parent exits only after the child proves its UI/runtime is ready.
- Existing stale-marker recovery still works.
- Failed or timed-out child startup keeps or restores the WebView path.
- No fixed `sleep(300ms)` determines correctness.
- Duplicate sidecars are rejected without resetting user settings incorrectly.
- Deep-link startup does not leave two independent writers active.
- Shutdown and retry behavior are observable in logs.

## Repository constraints

- Windows is the shipping platform for the sidecar. There is no portable
  directory fsync there; `sync_parent_best_effort` is a no-op on non-unix.
- `native_boot.rs`, `atomic_file.rs` and `transaction_lock.rs` are `#[path]`-
  included by the Slint crate, so both binaries share the source but get
  **separate process-global statics**. Both can run against the same state dir.
- No new native dependency without maintainer approval.
- The sidecar is launched by the parent, which then exits; a failed handshake
  must always leave the user with a working WebView window.

## Excerpt 1 — the ready protocol (`native_boot.rs`)

```rust
pub(crate) fn prepare(state_dir: &Path, launch_id: &str) -> Result<(), String> {
    fs::create_dir_all(state_dir)?;
    let _ = fs::remove_file(ready_path(state_dir));
    write_record(&pending_path(state_dir),
        &BootRecord { launch_id: launch_id.to_string(), parent_pid: std::process::id() })
}

pub(crate) fn signal_ready(state_dir: &Path, launch_id: &str) -> Result<bool, String> {
    let Some(pending) = read_record(&pending_path(state_dir)) else { return Ok(false) };
    if pending.launch_id != launch_id { return Ok(false) }
    write_record(&ready_path(state_dir), &pending)?;
    Ok(true)
}

pub(crate) fn wait_for_ready(
    state_dir: &Path, launch_id: &str, timeout: Duration,
    mut child_state: impl FnMut() -> Result<ChildState, String>,
) -> Result<WaitOutcome, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if ready_matches(state_dir, launch_id) {
            return Ok(if child_state()? == ChildState::Running {
                WaitOutcome::Ready
            } else {
                WaitOutcome::ChildExited
            });
        }
        if child_state()? == ChildState::Exited { return Ok(WaitOutcome::ChildExited) }
        if Instant::now() >= deadline { return Ok(WaitOutcome::TimedOut) }
        std::thread::sleep(POLL_INTERVAL);
    }
}

// Every boot record is published through the A1 writer:
fn write_record(path: &Path, record: &BootRecord) -> Result<(), String> {
    let bytes = serde_json::to_vec(record)?;
    crate::atomic_file::atomic_write(path, &bytes)
        .map_err(|error| format!("Could not publish {}: {error}", path.display()))
}
```

## Excerpt 2 — duplicate-sidecar acceptance (`native_boot.rs`, added in A3)

```rust
/// Positive proof that a native active record is backed by a live process
/// holding the OS authority lock. A crash can leave `native-shell.active`
/// behind, so the record alone is never sufficient for duplicate acceptance.
pub(crate) fn live_native_authority_exists(state_dir: &Path) -> bool {
    if !native_authority_is_active(state_dir) { return false }
    let Ok(file) = OpenOptions::new().create(true).read(true).write(true)
        .truncate(false).open(state_dir.join(AUTHORITY_FILE)) else { return false };
    match FileExt::try_lock(&file) {
        Err(TryLockError::WouldBlock) => true,          // someone holds it -> live
        Ok(()) => { let _ = FileExt::unlock(&file); false }
        Err(TryLockError::Error(_)) => false,
    }
}
```

Consumed in `commands.rs::launch_native_shell_process`:

```rust
let existing_native_ready = crate::native_boot::ready_matches(&state_dir, &launch_id)
    && crate::native_boot::live_native_authority_exists(&state_dir);
crate::native_boot::clear_owned(&state_dir, &launch_id);
match outcome? {
    WaitOutcome::Ready => { /* commit */ }
    WaitOutcome::ChildExited if cancelled => Err("cancelled to preserve an activation"),
    WaitOutcome::ChildExited if existing_native_ready => {
        eprintln!("[native-shell] duplicate child acknowledged live native owner");
        Ok(())
    }
    WaitOutcome::ChildExited => Err("Native performance UI exited before reporting ready."),
    WaitOutcome::TimedOut => Err("timed out"),
}
```

## Excerpt 3 — the exit commit (`commands.rs`, added in A3)

```rust
/// Atomically choose between committing the ready child or preserving an
/// activation that raced the end of the wait. The callback and this function
/// serialize on the authority mutex, closing the post-ready/pre-exit gap.
fn commit_native_handoff_authority_release() -> Result<(), String> {
    let mut authority = webview_authority().lock()
        .map_err(|_| "WebView authority state is unavailable.".to_string())?;
    if NATIVE_HANDOFF_CANCELLED.swap(false, Ordering::SeqCst) {
        NATIVE_HANDOFF_ACTIVE.store(false, Ordering::SeqCst);
        return Err("Native handoff was cancelled to preserve an incoming activation.".into());
    }
    authority.take();
    Ok(())
}

pub(crate) fn finish_native_handoff_exit() -> bool {
    if !NATIVE_HANDOFF_ACTIVE.swap(false, Ordering::SeqCst) { return false }
    NATIVE_HANDOFF_CANCELLED.swap(false, Ordering::SeqCst)
}
```

`lib.rs` calls `finish_native_handoff_exit()` from `RunEvent::ExitRequested` and
prevents the exit when it returns true. The single-instance handler buffers a
deep link when `WEBVIEW_LAUNCH_ID_ENV` is set (i.e. this process is a
reverse-handoff child) and replays it from the page-load-finished callback.

## Excerpt 4 — the A1 rename budget

```rust
const RENAME_ATTEMPTS: usize = 5;
const RENAME_BACKOFF: Duration = Duration::from_millis(40);

fn is_transient_rename_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::PermissionDenied
        || matches!(error.raw_os_error(), Some(5 | 32 | 33))   // ACCESS_DENIED, SHARING, LOCK
}

fn rename_with_retries(from: &Path, to: &Path) -> io::Result<()> {
    for attempt in 0..RENAME_ATTEMPTS {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if attempt + 1 < RENAME_ATTEMPTS && is_transient_rename_error(&e) => {
                std::thread::sleep(RENAME_BACKOFF);
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}
```

## Observed evidence to weigh

During this session's full Slint test suite,
`transaction_lock::tests::two_process_read_modify_write_has_no_lost_updates`
(A2's test) failed once. The helper panicked on
`atomic_file::atomic_write(...).unwrap()` — the **rename**, not the lock (that
test already uses a 10s lock timeout). It passes 5/5 in isolation and 3/3 on
subsequent full-suite runs. Total rename budget is ~200ms.

This matters to A3 specifically because `native_boot::write_record` publishes
`native-boot.pending` and `native-boot.ready` through that same writer.

## Failure modes to evaluate

1. `signal_ready` loses its rename to a transient Windows sharing violation.
   The child *is* ready but never publishes proof; the parent times out. Is
   fail-safe (WebView retained) sufficient, or does the handshake need its own
   retry/confirmation independent of the ~200ms budget?
2. Is ~200ms the right budget at all, given `settings_store` uses the same on
   the real settings write path, and A1 promises "old-or-new, never torn"?
3. `live_native_authority_exists` opens the authority file with `create(true)`
   and `try_lock`. Can the probe itself perturb the lock, race a legitimate
   claimant, or return a false negative that turns a real duplicate into a
   spurious startup failure? Note it is called *after* the wait returned.
4. `prepare` removes the ready file then writes pending, non-atomically as a
   pair. What does a crash between those two steps leave, and can a subsequent
   launch with a recycled or colliding `launch_id` misread it?
5. `NATIVE_HANDOFF_ACTIVE` / `NATIVE_HANDOFF_CANCELLED` are process-global
   atomics read in `ExitRequested` and in the single-instance callback, while
   `commit_native_handoff_authority_release` holds the authority mutex. Is
   there an ordering in which the exit is prevented but authority was already
   released, leaving no visible window and no authoritative writer?
6. Both binaries `#[path]`-include these modules and get separate statics. Does
   any A3 invariant depend on a static that is silently per-process?
7. `wait_for_ready` checks `ready_matches` before `child_state`. A child that
   published ready and then crashed is reported `ChildExited`. Combined with the
   new `existing_native_ready` arm, can a crashed child be accepted as a live
   duplicate owner?

## Required output

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
