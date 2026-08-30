# Fable Consultation — P0-A3 follow-up: the acquired-authority proof

The master prompt requires a final Fable review of the P0 lane before
`fix/audit-p0-a3-sidecar-handshake` (PR #389) merges. That review was completed
in `p0-final-fable.md` and its blocker was adopted as `D-P0-A3-FINAL`.

**One commit landed after that review**: `af1f16eb fix(native): require acquired
authority handoff proof`. This file reviews only that delta, so the required
final review covers the tree that will actually merge. Do not re-litigate A1,
A2, or the parts of A3 already reviewed.

## Finding and acceptance criteria

P0-A3 — Native Sidecar Ready Handshake. Criteria unchanged:

- Parent exits only after the child proves its UI/runtime is ready.
- Existing stale-marker recovery still works.
- Failed or timed-out child startup keeps or restores the WebView path.
- No fixed `sleep(300ms)` determines correctness.
- Duplicate sidecars are rejected without resetting user settings incorrectly.
- Deep-link startup does not leave two independent writers active.
- Shutdown and retry behavior are observable in logs.

## What the delta changes

Before `af1f16eb`, the WebView parent released UI authority and exited once the
child published `native-boot.ready`. `ready` proves the child's Slint event loop
is live; it does **not** prove the child ever took the OS authority lock the
parent just dropped. A child that reached ready and then died, hung, or failed
to claim the lock left nobody holding authority while the parent exited.

The delta adds a second, ordered proof: `native-boot.acquired`.

## Repository constraints

- Windows is the shipping platform for the sidecar.
- `native_boot.rs` is `#[path]`-included by the Slint crate, so both binaries
  share source but get **separate process-global statics**, and both run against
  the same state dir.
- The parent exits after a successful handoff; every failure must leave the user
  with a working, visible WebView window.
- `fs4` advisory locks are released only by the kernel on process death.
- No new native dependency without maintainer approval.

## Current code

`src-tauri/src/native_boot.rs` — the new proof (child side is `signal_acquired`,
reached through `AuthorityGuard::signal_acquired`):

```rust
pub(crate) fn signal_acquired(state_dir: &Path, launch_id: &str) -> Result<bool, String> {
    let pending_matches = read_record(&pending_path(state_dir))
        .map(|record| record.launch_id == launch_id)
        .unwrap_or(false);
    let active_matches = read_record(&state_dir.join(ACTIVE_FILE))
        .map(|record| record.launch_id == launch_id)
        .unwrap_or(false);
    if !pending_matches || !active_matches {
        return Ok(false);
    }
    write_record(
        &acquired_path(state_dir),
        &BootRecord { launch_id: launch_id.to_string(), parent_pid: std::process::id() },
    )?;
    Ok(true)
}

pub(crate) fn wait_for_acquired(
    state_dir: &Path,
    launch_id: &str,
    timeout: Duration,
    mut child_state: impl FnMut() -> Result<ChildState, String>,
) -> Result<WaitOutcome, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if acquired_matches(state_dir, launch_id) {
            return Ok(if child_state()? == ChildState::Running {
                WaitOutcome::Ready
            } else {
                WaitOutcome::ChildExited
            });
        }
        if child_state()? == ChildState::Exited { return Ok(WaitOutcome::ChildExited); }
        if Instant::now() >= deadline { return Ok(WaitOutcome::TimedOut); }
        std::thread::sleep(POLL_INTERVAL);
    }
}

pub(crate) fn terminate_and_reap_child(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Result<(), String> {
    if child.try_wait().map_err(|e| format!("Could not inspect child process: {e}"))?.is_some() {
        return Ok(());
    }
    let kill_error = child.kill().err();
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL_INTERVAL),
            Ok(None) => return Err(kill_error.map_or_else(
                || "Timed out reaping terminated child process.".to_string(),
                |e| format!("Could not terminate child process: {e}"))),
            Err(e) => return Err(format!("Could not reap child process: {e}")),
        }
    }
}
```

`AuthorityGuard::Drop` clears the active record *before* unlocking, so a
departing owner cannot delete a successor's freshly published record:

```rust
impl Drop for AuthorityGuard {
    fn drop(&mut self) {
        clear_record_if_owned(&self.state_dir.join(ACTIVE_FILE), &self.launch_id);
        let _ = FileExt::unlock(&self.file);
    }
}
```

`src-tauri/src/commands.rs` — the parent's ordered handoff, after
`wait_for_ready` returns `Ready` (the `cancel.is_some()` arm is the runtime
toggle handoff; the startup path has no cancel token and does not release):

```rust
if first_outcome == WaitOutcome::Ready && cancel.is_some() {
    if let Err(error) = commit_native_handoff_authority_release() {
        let _ = terminate_and_reap_child(&mut child, Duration::from_secs(1));
        clear_owned(&state_dir, &launch_id);
        return Err(error);
    }
    eprintln!("[native-shell] ready launch_id={launch_id}; waiting for authority proof");
    let acquired = wait_for_acquired(&state_dir, &launch_id, READY_TIMEOUT, || {
        if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            cancelled = true;
            let _ = terminate_and_reap_child(&mut child, Duration::from_secs(1));
            return Ok(ChildState::Exited);
        }
        match child.try_wait() {
            Ok(Some(_)) => Ok(ChildState::Exited),
            Ok(None) => Ok(ChildState::Running),
            Err(e) => Err(format!("Failed to inspect native authority handoff {launch_id}: {e}")),
        }
    });
    if !matches!(acquired, Ok(WaitOutcome::Ready)) {
        let reap = terminate_and_reap_child(&mut child, Duration::from_secs(1));
        let reclaim = reclaim_webview_authority(&state_dir);   // take the lock back
        clear_owned(&state_dir, &launch_id);
        /* ... build failure message, log reap error ... */
        reclaim.map_err(|e| format!("{failure} WebView authority recovery failed: {e}"))?;
        return Err(failure);
    }
    clear_owned(&state_dir, &launch_id);
    eprintln!("[native-shell] authority acquired launch_id={launch_id}");
    return Ok(());
}
```

The child takes the released lock through `claim_after_ready_release` (poll on
`try_claim_authority`, bounded, never asks the parent to shut down), then calls
`guard.signal_acquired()`.

`commit_native_handoff_authority_release` and
`cancel_native_handoff_for_activation` serialize on one process-global authority
mutex, so an incoming activation either cancels before release or is seen by the
release path.

## Candidate designs the executor considered

1. **Adopted.** Release authority, then require a matching `acquired` record
   before the parent exits; on any non-`Ready` outcome, terminate and reap the
   child, reclaim the WebView authority lock, and fail back to the WebView.
2. Keep `ready` as the only proof and let the child's failure to take the lock
   be discovered by the next launch's stale-marker recovery.
3. Do not release the lock at all — have the parent pass the locked handle to
   the child, or have the child wait on the lock and the parent exit blind.

## Failure modes to evaluate

1. **The release/reclaim window.** Between `commit_native_handoff_authority_release`
   and a failed `reclaim_webview_authority`, nobody holds authority. Can a third
   process (Start-menu relaunch, deep link, single-instance activation) claim it
   and leave two writers, or leave the user with no window?
2. **Reclaim failure.** `reclaim_webview_authority` can itself fail or time out;
   the parent returns `Err` and stays alive but unauthoritative. Is remaining
   alive-without-authority safe, or should it be fatal?
3. **Kill-then-take race.** The parent terminates the child and reclaims. The
   child may already hold the lock; `terminate_and_reap_child` is bounded at 1s
   and the reclaim path calls `request_active_shutdown`. Can these deadlock or
   interleave into a lost active record?
4. **`signal_acquired` preconditions.** It requires *both* `pending` and `active`
   to match the launch ID. `clear_owned` on the parent deletes `pending` after
   success. Is there an ordering where the parent clears `pending` while the
   child is mid-`signal_acquired`, so a legitimate acquisition reports `Ok(false)`?
5. **Cancellation inside the acquired wait.** `cancelled = true` kills the child
   after the parent has already released authority. Reclaim then runs. Is the
   incoming activation guaranteed to land in an authoritative WebView?
6. **Startup path asymmetry.** The startup launch has `cancel == None` and never
   enters the acquired wait; it accepts `Ready` only when
   `live_native_authority_exists` is true. Is that check equivalent proof, given
   it is a lock probe rather than a launch-ID-bound record?
7. **Crash between release and acquire.** Power loss with `ready` published,
   `acquired` absent, both processes gone. What does the next boot see?

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
