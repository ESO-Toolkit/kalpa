# Fable Consultation — P0-A3 ship review: making authority loss fatal

Third and final P0-A3 consultation. The first (`p0-final-fable.md`) reviewed the
assembled P0 lane; the second (`p0-a3-acquired-proof-fable.md`) reviewed the
acquired-authority delta and returned the launch-ID classification blocker,
which is fixed and adopted as `D-P0-A3-CLASSIFY`.

This one reviews the **last change before merge**, and asks whether P0-A3 is
done. Do not re-litigate settled ground.

## Finding and acceptance criteria

P0-A3 — Native Sidecar Ready Handshake. Criteria unchanged:

- Parent exits only after the child proves its UI/runtime is ready.
- Existing stale-marker recovery still works.
- Failed or timed-out child startup keeps or restores the WebView path.
- No fixed `sleep(300ms)` determines correctness.
- Duplicate sidecars are rejected without resetting user settings incorrectly.
- Deep-link startup does not leave two independent writers active.
- Shutdown and retry behavior are observable in logs.

## What prompted this change

The adversarial reviewer (Sol) returned `REVISE` with three P0 findings, all one
class: after releasing UI authority, a failed reclaim leaves the process alive
and visible while holding no lock. Your own prior review raised the same thing
as RISKS item 2 and called it "not fatal — a child whose loop is hung is not
writing — but it is a silent state", a human decision.

On re-examination I concluded it **is** two writers, not one lock-less writer,
and therefore not merely silent:

1. The process stays visible and keeps writing settings/metadata.
2. It holds no lock, so `live_native_authority_exists` is false for everyone.
3. The next launch of `kalpa.exe` therefore sees no live owner, no pending
   marker, `performanceMode` still native — and **spawns a sidecar**.
4. That sidecar finds the authority lock free, claims it, and starts writing.

Two independent writers on the same state, reachable without any crash.

On the Slint side there is a second mechanism you flagged: the shutdown timer
reads the launch ID out of `native_authority()`, so an unauthoritative shell is
also permanently deaf to `request_active_shutdown` — it will never yield.

## The change

Both directions now do what the forward-handoff child already did (every
authority failure in the page-ready callback ends in `quit_event_loop`).

`src-tauri/src/commands.rs`:

```rust
static WEBVIEW_AUTHORITY_LOST: AtomicBool = AtomicBool::new(false);

fn reclaim_webview_authority(state_dir: &Path) -> Result<(), String> {
    let mut authority = webview_authority().lock().map_err(/* ... */)?;
    if authority.is_some() { return Ok(()); }
    let guard = match crate::native_boot::claim_webview_after_shutdown(
        state_dir, crate::native_boot::READY_TIMEOUT,
    ) {
        Ok(guard) => guard,
        Err(error) => {
            WEBVIEW_AUTHORITY_LOST.store(true, Ordering::SeqCst);
            eprintln!("[native-shell] fatal: released UI authority and could not reclaim it: {error}");
            return Err(error);
        }
    };
    *authority = Some(guard);
    Ok(())
}
```

and in the `launch_native_performance_mode` command:

```rust
if let Err(error) = launch_result {
    abort_native_handoff();
    if webview_authority_was_lost() {
        eprintln!("[native-shell] exiting: cannot continue without UI authority ({error})");
        app.exit(1);
    }
    return Err(error);
}
```

`prototypes/slint-kalpa/src/main.rs`, in `return_to_webview_shell`:

```rust
let guard = match reclaimed {
    Ok(guard) => guard,
    Err(error) => {
        eprintln!("[native-shell] fatal: released UI authority and could not reclaim it: {error}");
        let _ = slint::quit_event_loop();
        return Err(format!("{failure} Native authority recovery failed: {error}"));
    }
};
```

Also in this branch since your last review:

- `webview_launch_id()` / `is_webview_launch_id()` centralise the owner-kind
  prefix that was three separate string literals; `return_to_webview_shell`
  mints through it and `complete_webview_handoff` refuses a non-WebView-shaped
  ID (`D-P0-A3-CLASSIFY`).
- The single-instance activation path gates reveal/focus/deep-link emit on
  `holds_webview_authority()` (`D-P0-A3-ACTIVATION`).
- Stale-marker cleanup also removes `native-boot.acquired` (your item 4).
- New real-child test `post_acquire_cancellation_reaps_the_child_and_frees_the_lock`
  (your TESTS item 4): child holds the lock, cancellation arrives, and the test
  asserts the reap is what frees the lock for the reclaim.

## One recommendation of yours I did NOT adopt — please confirm or correct

Your item 3 suggested the startup path could require
`acquired_matches || existing_native_ready` instead of the lock probe alone.

I believe that **weakens** the guard rather than tightening it. `A || B` accepts
strictly more than `B`. `native-boot.acquired` is a *file* that survives the
publisher's death, so a child that published `acquired` and then crashed would
newly be accepted as a live owner. That is precisely the case
`a_crashed_child_that_published_ready_is_not_a_live_owner` exists to reject: it
spawns a real child that claims authority, publishes, then `process::exit`s, and
asserts `ready_matches` is still true while `live_native_authority_exists` is
false — because only the kernel released the lock.

If I have misread your intent (e.g. you meant `acquired_matches && ...`), say so.

## Failure modes to evaluate

1. **Is `app.exit(1)` reachable on a path where the reclaim was never needed?**
   `WEBVIEW_AUTHORITY_LOST` is process-global and never cleared. Can a
   *successful* later handoff be poisoned by a flag set by an earlier failure?
2. **Slint `quit_event_loop` from `return_to_webview_shell`.** This runs on a UI
   callback and the function still returns `Err` to its caller, which will try
   to render an error. Is quitting while a caller is about to touch the UI safe,
   or should the error path be silent once quit is requested?
3. **Is exiting the right call at all**, versus retrying the reclaim on a timer?
   My reasoning: `claim_webview_after_shutdown` already polls for the full
   `READY_TIMEOUT` while re-requesting shutdown, so failing means another
   process held the lock for 10 continuous seconds — it *is* the UI now. Is
   there a case where the lock is held that long by something that is not a
   legitimate successor?
4. **The user-visible cost.** Exiting drops the user's session. Given the child
   is terminated *and reaped* before the reclaim is attempted, how reachable is
   this in practice? Is there a cheaper intermediate (e.g. reveal a modal and
   refuse writes) that is actually safer, or is that just a lock-less writer
   with extra steps?
5. **Deep-link ordering.** With the activation gate in place, an activation that
   arrives while authority is released is dropped rather than queued. Is
   silently dropping it acceptable, or must it be buffered and replayed after
   the reclaim, the way the reverse-handoff child buffers its startup link?

## Also relevant: a concurrent agent's edit is in this worktree

Another agent added, uncommitted and outside my commits, a
`try_claim_authority_with_grace(state_dir, launch_id, 100ms)` used by the
sidecar's `acquire_native_shell_lock`. Its stated rationale is that the parent's
`live_native_authority_exists` probe momentarily *takes* the lock, so a child
racing that probe can misread it as a duplicate owner and exit.

That rationale looks correct to me — the probe does `try_lock` then `unlock`.
Please state whether a 100ms grace on the child's claim is the right shape, or
whether the probe itself should avoid taking the lock (and if so, how, given a
non-blocking acquire is the only portable liveness test `fs4` offers).

I am not committing that change; I only need to know whether it is sound, and
whether it interacts with anything above.

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

Add one final line, exactly one of:

```text
SHIP: YES - P0-A3 meets its acceptance criteria
SHIP: NO - <the single blocking reason>
```
