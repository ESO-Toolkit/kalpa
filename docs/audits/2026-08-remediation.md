# August 2026 Audit Remediation Tracker

This file is the durable execution record for `2026-08-remediation-master-prompt.md`.

| ID | Status | Branch | PR | Merged SHA | Fable decision | Sol verdict | Tests | Notes |
|---|---|---|---|---|---|---|---|---|
| W1 | pr-open | `fix/audit-w1-worker-consistency` | [#369](https://github.com/ESO-Toolkit/kalpa/pull/369) (draft) | - | D-W1-1, D-W1-2, D-W1-3 | REVISE; follow-up REVISE; all verified findings addressed | Worker check; 184 tests; Wrangler dry-run; name guard | Fable twice-reject redesign implemented; awaiting refreshed CI/maintainer sign-off. |
| W2 | todo | - | - | - | pending | - | - | Requires maintainer approval before merge if reconciliation can delete D1 rows. |
| W3 | todo | - | - | - | - | - | - | Worker low-severity hardening. |
| P0-A1 | pr-open | `fix/audit-p0-a1-atomic-writer` | [#380](https://github.com/ESO-Toolkit/kalpa/pull/380) (draft) | - | D-P0-1 | REVISE; all verified findings addressed; follow-up APPROVE | Root 490; Rust 814 passed/17 ignored; Slint 760 passed/15 ignored; native build; Tauri build; sandbox 3/3 | Shared crash-safe atomic writer; stacked on W1. |
| P0-A2 | pr-open | `fix/audit-p0-a2-cross-process-locking` | [#388](https://github.com/ESO-Toolkit/kalpa/pull/388) (draft) | - | D-P0-A2 | REVISE; verified finding fixed; follow-up APPROVE | Root 490; Rust 828 passed/18 ignored; Slint 777 passed/16 ignored; native build; Tauri build; sandbox 3/3 | Shared bounded cross-process RMW locking; stacked on P0-A1. |
| P0-A3 | pr-open | `fix/audit-p0-a3-sidecar-handshake` | [#389](https://github.com/ESO-Toolkit/kalpa/pull/389) (draft) | - | D-P0-1; D-P0-A3-FINAL; D-P0-A3-CLASSIFY; D-P0-A3-ACTIVATION | REVISE; verified findings fixed, two partially refuted with evidence | Root 490; Rust 854 passed/18 ignored; Slint 800 passed/16 ignored; clippy `-D warnings` + fmt clean both crates; native sidecar build; 4/4 real-binary Windows handshake scenarios | Two-phase ready/acquired authority handoff with bounded child termination; stacked on P0-A2. `test:e2e:sandbox` still outstanding — blocked by another worktree's live Kalpa process. |
| R4 | todo | - | - | - | pending | - | - | Preserve separately tracked sibling ownership. |
| R5 | todo | - | - | - | pending | - | - | Folder-qualified conflict protection. |
| R6 | todo | - | - | - | pending | - | - | Crash-safe installer transaction. |
| R7 | todo | - | - | - | - | - | - | Bound native build evidence to uploaded bytes. |
| R8 | todo | - | - | - | - | - | - | Manifest-less Protected Edits disclosure. |
| R9 | todo | - | - | - | - | - | - | Record the downloaded artifact version. |
| F1 | todo | - | - | - | - | - | - | Import-source sequencing. |
| F2 | todo | - | - | - | - | - | Uploader log-directory sequencing. |
| F3 | todo | - | - | - | - | - | Imported log must use fresh list data. |
| F4 | todo | - | - | - | - | - | Logout invalidates private loads. |
| F5 | todo | - | - | - | - | - | Controlled-state parent veto. |
| F6 | todo | - | - | - | - | - | - | Optimistic-state sequencing. |
| H1 | todo | - | - | - | - | - | - | Generate release copy from matching CHANGELOG section. |
| H2 | todo | - | - | - | - | - | - | Decide theme-image provenance and tracking policy. |
| H3 | todo | - | - | - | - | - | - | Triage Worker package-version synchronization. |
| H4 | todo | - | - | - | - | - | - | Update `claude.md` structure tree. |
| H5 | todo | - | - | - | - | - | - | Propose branch pruning; do not delete without approval. |
| H6 | todo | - | - | - | - | - | - | Revisit ignored quick-xml advisories when dependencies permit. |

## Decisions

### D-P0-A2 — Shared `fs4` transaction locks with canonical target identity

- Chosen after the required fresh Fable consultation: one path-included `transaction_lock.rs` shared by Tauri and Slint, using `fs4` 1.1.0 exclusive OS locks on persistent sibling `.<name>.kalpa.lock` files. Cargo added `fs4` with its `sync` feature to both crates and `dunce` to Slint; generated lockfiles were not hand-edited. The dependencies compile on the supported Windows/Linux targets and introduced no new Cargo advisories.
- Identity and ordering: make targets absolute, canonicalize the nearest existing ancestor, append unresolved components, normalize Windows verbatim prefixes/case for comparison, then sort and deduplicate multi-lock requests. Multi-lock acquisition uses one total deadline and drops partially acquired guards on failure.
- Ownership and behavior: lock-file bytes have no authority and lock files are never removed as stale. The open file handle owns the OS lock, so RAII and process death release it. Interactive acquisition polls every 25 ms for at most 2 seconds and returns structured, user-visible timeout, cancellation, or I/O errors.
- Lock scope: retain useful in-process mutexes and consistently acquire local mutexes before OS locks. Gather/network/archive/scan/profile-apply/UI-callback work stays outside; only the short reload → merge → P0-A1 atomic publish transaction is locked.
- Settings compatibility: Tauri sends and merges exact changed entries. Slint compares each full UI callback snapshot with its last local baseline, reloads the current disk object under the shared lock, and applies only locally changed fields. Existing aliases are canonicalized only when their logical control changes. JSON schema, persisted keys, Tauri command shapes, and frontend wire contracts remain unchanged.
- Scope boundary and rollback: locks coordinate Kalpa's Tauri and Slint writers on local supported filesystems; arbitrary external writers and remote/network filesystem semantics are not claimed. P0-A3 launch readiness remains separate. Reverting the P0-A2 commits removes coordination without requiring a data migration; persistent empty lock files are harmless.

### D-P0-1 — One shared atomic publisher, separate transaction locks and launch protocol

- Chosen for P0-A1: a dependency-free `atomic_file.rs` shared by the Tauri and Slint crates. Each operation creates a same-directory staging file with `create_new(true)` and a PID, monotonic counter, and random nonce; writes are flushed and `sync_all` completes before a bounded atomic replacement attempt. The operation owns exactly one staging path and cleanup never scans for or removes another writer's file.
- The helper supports byte/string writes and streamed `Read + Write + Seek` use through `AtomicFile`, so large snapshot ZIPs do not need to be buffered. A process-local final-publish mutex mitigates simultaneous Windows replacements but is explicitly not a read-modify-write lock.
- Durability contract: after success, readers see a complete old or new file. Unix additionally attempts a parent-directory sync after rename; Windows has no portable directory-sync guarantee, so the file contents are durable before publication but rename metadata persistence across sudden power loss is not claimed.
- Compatibility: metadata keeps the primary → legacy `.json.tmp` → `.json.bak` recovery order. New unique staging files are uncommitted implementation details and are never promoted or globally cleaned. Existing primary/backup formats and JSON/wire shapes remain unchanged.
- P0-A2 design: put a separate shared OS-backed transaction lock around the full read → mutate → write sequence, with canonical identities and ordering, bounded waits, crash release, and two-process/owner-kill tests. Never infer lock ownership from a PID or delete a lock file as stale. Existing useful in-process mutexes remain.
- P0-A3 design: replace fixed-delay correctness with a parent-issued launch ID bound to the pending boot record and a positive child-ready signal. The parent keeps WebView authority until the matching child reports ready; timeout, early exit, stale markers, duplicate/deep-link launches, retry, and shutdown must fail back observably. A3 reduces concurrent writers; A2 remains defense in depth.
- Rejected: buffering all output into one bytes-only helper, because snapshot archives can be large; and sharing only a staging-name utility while retaining bespoke publication logic, because that violates the one-implementation acceptance criterion and permits durability/cleanup drift.

### D-W1-1 — Durable Object storage is mutation authority

- Chosen: store canonical packs as per-ID records in the existing SQLite-backed `PackIndexDO`; rewrite the unchanged KV index/detail read model from canonical state after each mutation.
- Create uniqueness and author caps run inside the DO gate. Updates merge editable content into the canonical record and preserve DO-owned identity/counters. Missing records are never reconstructed from caller-provided KV bodies.
- Rejected: continuing to treat KV as authoritative. KV can return a stale index even when DO RPCs are serialized, so tombstones alone or merely removing seed healing cannot prevent all lost updates.
- Rejected: one serialized DO index value. Per-ID records avoid a single-value growth ceiling and make existence/uniqueness point lookups.
- Compatibility: successful response fields and the KV `PackIndex`/`Pack` shapes are unchanged; duplicate create uses the existing `{ error: string }` error shape with HTTP 409.
- Rollout caveat: the first post-deploy bootstrap must import the existing KV index. A quiet/controlled rollout or an explicit bootstrap mechanism must be chosen before merge because the freshest KV read can still lag a just-completed pre-deploy mutation.

### D-W1-2 — Continuously merged shadow with a parity-gated authority flip

- Chosen: default the durable `meta:authority` flag to `kv`, re-read and additively merge the KV index on every serialized mutation, and keep per-id tombstones so stale KV cannot restore deletions. DO records win once present. During shadow mode, list and backup reads use the merged DO view, while mutations update only affected KV detail keys; they do not overwrite the full KV index from a potentially incomplete shadow. After the parity-gated flip, the DO resumes full KV-index mirroring with the unchanged wire shape.
- The first deployment is the shadow/backfill phase. After at least one post-deploy daily backup, an operator must observe two clean parity reads more than 60 seconds apart before explicitly switching authority to `do`.
- Parity uses D1 ids, `backup:latest`, and enumerated `pack:<id>` detail keys as independent witnesses. Missing untombstoned ids block the flip. The admin-only adoption endpoint can recover a specifically adjudicated id from its independently propagated KV detail record; it never adopts D1 rows automatically because a failed historical D1 delete can be a zombie.
- Rejected: one-shot or timed bootstrap gates. KV provides no bounded propagation time, and an auto-deployed maintenance gate would create operator-bounded mutation downtime without proving completeness.
- Rejected: all dated backups as live witnesses. Their intentional 90-day retention includes deleted packs and would make parity impossible; only a post-deploy `backup:latest` represents the current corpus.
- Rollback: shadow-mode mutations deliberately do not rewrite the full KV index, so rolling back to old code can omit post-deploy creates from listings even though their detail and D1 rows survive. Any rollback after this deployment therefore requires a verified backup/DO export restore; changing the flag alone is not a reconciliation strategy.

### D-W1-3 — Journal lifecycle transitions and serve detail from DO authority

- Chosen after the twice-reject Fable reconsultation: commit canonical lifecycle state together with a deterministic operation journal and per-pack pending marker, then resume idempotent KV vote/detail, D1, and full-index effects on retry and by Durable Object alarm.
- Delete commits the tombstone before destructive cleanup, so partial vote cleanup cannot coexist with a live pack. Same-owner retry resumes pending cleanup; slug reuse is refused until the old lifecycle finishes.
- Create retry by the same actor resumes the stored canonical create instead of returning duplicate. Success is acknowledged only after the KV detail step; an incomplete mirror returns a retryable failure while public detail reads use the DO and remain consistent with canonical lifecycle state.
- Unowned shadow records reconcile against newer KV detail by `updated_at`; an ownership latch prevents stale KV from overwriting DO mutations. Version disagreement is exposed as `stale_shadow` and blocks the authority flip.
- Rejected: external-effects-first compensation, because the compensating store can fail and a crash can leave an unowned orphan; pending markers without operation identity or alarms, because retries cannot be safely attributed and cleanup may remain stuck indefinitely.

### D-P0-A3-FINAL — Duplicate detection keys on the OS lock, never on a published marker

- Context: the master prompt requires a final Fable review of the P0 lane before P0-A3 merges. The consultation is `docs/audits/consultations/p0-final-fable.md`. The verdict was that A3 is sound on its main paths, with one blocker.
- **Blocker (verified against the code before adoption, per the advisor protocol).** Duplicate-launch acceptance required the duplicate *child* to publish `native-boot.ready`. The child (`prototypes/slint-kalpa/src/main.rs`, `confirm_native_boot_ready`) only logs a `signal_ready` failure and exits anyway. The parent then saw `ChildExited` with `existing_native_ready == false`, returned `Err`, and `try_launch_native_performance_mode_on_startup` ran `revert_performance_mode_to_webview` plus the `NATIVE_BOOT_FAILED` note. `lib.rs` then booted the WebView, which calls `claim_webview_authority` and therefore `request_active_shutdown`. Net effect: a transient file-publication failure flipped the user's native-performance-mode setting off **and** shut down the sidecar they were actively using. That violates the acceptance criterion "duplicate sidecars are rejected without resetting user settings incorrectly".
- Chosen: treat the held OS authority lock as the sole positive proof of a live owner.
  1. `try_launch_native_performance_mode_on_startup` now exits a duplicate activation via `live_native_authority_exists` *before spawning any child*, placed ahead of the settings gate rather than after it, as Fable suggested. The reverse handoff is the only way a live native owner can coexist with native mode disabled, and `prototypes/slint-kalpa/src/main.rs` sets `KALPA_FORCE_WEBVIEW=1` on that child, which short-circuits the function before either gate. Ordering the checks the other way would therefore only weaken duplicate rejection without ever rescuing a WebView boot. No settings write, no marker churn, no child round-trip.
  2. The parent's `ChildExited` acceptance arm drops the `ready_matches &&` conjunct. The lock alone proves liveness; the marker only ever proved the child once reached readiness.
- Rejected: keeping the `ready` conjunct as belt-and-braces. It cannot be a *safety* addition, because the only thing it can do is turn a live-owner duplicate into a spurious failure that reverts settings and kills the sidecar.
- Safety of the drop is pinned by `a_crashed_child_that_published_ready_is_not_a_live_owner`: a real child process claims authority, publishes `ready`, then `process::exit`s without unwinding. Afterwards `ready_matches` is still true while `live_native_authority_exists` is false, because only the kernel released the lock. `a_live_owner_is_detected_across_processes` covers the positive case cross-process.
- Also adopted (Fable item 2b): `native_boot::write_record` now retries the *whole* `atomic_write` up to 3 times at 100ms spacing and logs `raw_os_error`. Retrying the rename in place cannot clear a scanner holding the staging file; only a fresh `create_new` staging path escapes it. This keeps the handshake independent of the shared publisher's rename budget.
- Deferred (Fable item 2a): widening `atomic_file::rename_with_retries` to geometric backoff and adding `ERROR_USER_MAPPED_FILE` (1224) to `is_transient_rename_error`. That is P0-A1's file and #380 is already Sol-approved, so it is tracked in Open Questions rather than changed from the A3 branch.

### D-P0-A3-CLASSIFY — Owner kind travels in the launch ID, minted in exactly one place

- Context: the master prompt's final Fable P0 review (`p0-final-fable.md`) was completed at `c4efb8b5`, but one code commit landed after it (`af1f16eb`, the acquired-authority proof). A second, delta-scoped consultation was run so the required review covers the tree that actually merges: `docs/audits/consultations/p0-a3-acquired-proof-fable.md`. It confirmed the acquired proof itself is sound and returned one blocker.
- **Blocker (verified against the code and against `git show c4efb8b5` before adoption).** `af1f16eb` changed `complete_webview_handoff` from `claim_webview_authority` — which mints `webview-<id>` via `claim_webview_after_shutdown` — to `claim_after_ready_release(&state_dir, &launch_id)` using the raw ID minted by `return_to_webview_shell`. That was necessary for `signal_acquired`'s `active.launch_id == launch_id` equality check, but `native-shell.active` is also the *only* record a successor can read without holding the lock, and `native_authority_is_active` classifies an owner by "the ID does not start with `webview-`". After any reverse handoff (native UI to WebView for the uploader, Pack Hub or an app update), the WebView therefore held the lock under a record that read as a **native** owner.
- Concrete failure: with that record in place, toggling native mode back on made (a) the sidecar take `main.rs`'s `else if active_kind.0` branch, publish `ready` and exit, believing a native shell was already up, and (b) the parent compute `existing_native_ready = live_native_authority_exists` as true against its own lock. If the parent's poll observed `ChildExited` first — likely, since the child exits microseconds after writing `ready` — it took the "duplicate child acknowledged live native owner" arm, returned `Ok(())` and called `app.exit(0)`. Both processes gone, settings still native: **no window at all**. Whether the poll landed on `Ready` or `ChildExited` was a timing coin-flip, so the safe arm was not reliable.
- Chosen: make owner kind a property with exactly one mint site rather than a string convention repeated at call sites.
  1. `native_boot` gains `WEBVIEW_LAUNCH_PREFIX`, `webview_launch_id()` and `is_webview_launch_id()`; `native_authority_is_active` / `webview_authority_is_active` / `claim_webview_after_shutdown` now go through them instead of three separate `"webview-"` literals.
  2. `return_to_webview_shell` mints through `webview_launch_id()`. The mint plus `prepare` is extracted into `prepare_webview_handoff` purely so the call site is unit-testable.
  3. `complete_webview_handoff` refuses a launch ID that is not WebView-shaped. It fails closed: the WebView exits, and the still-live Slint parent reclaims on that exit, which is strictly better than publishing a misclassified active record.
- Rejected: fixing only the classifier (e.g. treating "not native-shaped" as WebView). The two roles are symmetric and both mint IDs; any rule inferred from the string alone breaks the moment a third role appears. Rejected: a `debug_assert!` on the ID shape, as Fable suggested — the failure is silent in exactly the release builds users run.
- Also adopted (Fable item 4): the two startup stale-marker cleanup sites now remove `native-boot.acquired` alongside `pending`/`ready`. Harmless today because `prepare` deletes it and every reader is launch-ID-bound, but recovery should not leave behind a marker it does not know about.

### D-P0-A3-ACTIVATION — Never reveal or emit without UI authority

- Context: the required adversarial review (Sol, `codex exec --sandbox read-only`) returned `VERDICT: REVISE` with three P0 findings, all one bug class — a process that stays alive and visible after releasing authority when the subsequent reclaim fails. Fable independently raised the same class as its RISKS item 2, calling it silent but not a merge blocker.
- Verified: the strongest instance is real and violates an explicit acceptance criterion. `lib.rs`'s single-instance activation callback logged a failed `cancel_native_handoff_for_activation` and then revealed the window and emitted the deep link anyway — while a live sidecar could still hold authority. The reverse-handoff branch immediately above it already had the correct rule ("never reveal/emit before then") and the ordinary path did not follow it.
- Chosen: add `commands::holds_webview_authority()` and gate reveal/focus/deep-link emit on it. A live WebView holds authority at all times except during a handoff — startup fails closed if `claim_webview_authority` cannot — so the gate rejects nothing legitimate. Pinned by `released_authority_is_reported_as_not_held_until_reclaimed`.
- Partially refuted, with evidence: Sol's findings 2 and 3 (reclaim failure in the WebView-to-native and native-to-WebView directions) do **not** produce two writers. In both, the counterpart child has already been terminated *and reaped* by `terminate_and_reap_child` before the reclaim is attempted, so the state is one writer without a lock, not two writers. Reclaim can only fail if something still holds the lock for the full `READY_TIMEOUT`. The activation gate above now also prevents that lock-less process from acting on an activation. Whether losing authority should additionally be made *terminal* (exit the process) is a user-visible behaviour change, so it is recorded in Open Questions rather than decided here — matching Fable, which called it a human decision.

## Session Log

### 2026-08-27 — W1 twice-reject escalation

- PR #369 was returned to draft after review identified four additional correctness failures: stale KV versions can be frozen by ID-only shadow merging; partial vote cleanup can corrupt a still-live pack; a failed KV detail deletion is not retryable after the DO tombstone commits; and a failed KV detail write leaves a committed create that retries as a duplicate.
- Per the twice-reject rule, W1 is blocked and implementation is paused while Fable is reconsulted with both prior Sol reviews and the new review evidence.
- Fable selected D-W1-3: journaled and resumable lifecycle transitions, tombstone-first delete, DO-authoritative detail reads, version-aware shadow reconciliation, and alarm-based effect repair.
- Fresh Sol review returned `REVISE` with four findings: orphan detail adoption during account purge; stale same-author operations crossing slug reuse; vote/install counters committing before a fallible KV mirror; and a backup write racing account deletion. The single prescribed follow-up also returned `REVISE` after verifying those cases.
- Addressed every follow-up finding with author-scoped orphan hydration, created-at lifecycle compare-and-swap for update/delete, durable dirty-mirror markers repaired by alarm, and DO-serialized backup/account deletion guarded by a deleted-author latch. Added exact failure/retry regressions.
- Final local evidence: Worker TypeScript check passes; all 184 tests pass; Wrangler dry-run passes; `wrangler.toml` remains `kalpa-pack-hub`. No deploy, merge, schema change, or Worker rename was performed.

### 2026-08-28 — Claude (P0-A3 verification, final Fable review, adversarial review)

- Resumed the pushed branch at `af1f16eb`; `origin` was in sync and the base `fix/audit-p0-a2-cross-process-locking` was unchanged (0 behind, 8 ahead), so no coordination mismatch with the concurrent session handling the PR stack.
- Re-ran every gate on the current tree before changing anything: `npm run check`, root vitest 490/490, the Windows sidecar placeholder script, main Rust 852 passed/18 ignored, Slint 798 passed/16 ignored, strict clippy and `fmt --check` on both crates, `npm run build:native-slint`. Both previously flaky P0-A2 concurrency tests passed this run.
- **Manual Windows validation with the real built sidecar** (`src-tauri/binaries/kalpa-slint-x86_64-pc-windows-msvc.exe`), driven against an isolated `KALPA_NATIVE_STATE_DIR` so the concurrently running Kalpa instance in another worktree was untouched. Five scenarios, all pass: normal handshake (`ready` at 223ms, `acquired` 30ms later, both bound to the launch ID, `native-shell.active` matching); duplicate sidecar against a live owner (duplicate logs `ready without authority` and exits, owner keeps authority and its active record); stale/mismatched `native-boot.pending` (`rejected stale`, no `ready`, no `acquired`); no pending record at all, held past the parent's 10s `READY_TIMEOUT` with no false readiness ever published; and — after the D-P0-A3-CLASSIFY fix and a sidecar rebuild — a WebView-shaped owner holding authority, where an incoming sidecar logs `WebView retains UI authority; rejecting sidecar` instead of taking the native-owner branch. That last scenario is the real-binary discriminator for the blocker. The first harness run produced a false negative because PowerShell 5.1 `Set-Content -Encoding utf8` writes a BOM, which `read_record` correctly refuses — a harness bug, not a code defect.
- **Final Fable P0 review**: the existing record (`p0-final-fable.md`) predated `af1f16eb`, so a delta-scoped follow-up was run rather than duplicating it. Result: the acquired proof is sound; one verified blocker adopted as D-P0-A3-CLASSIFY.
- **Adversarial review** (Sol): `VERDICT: REVISE`, three P0 findings of one class. The verified instance is fixed under D-P0-A3-ACTIVATION; the other two are partially refuted with code evidence recorded there, and the residual question is in Open Questions.
- Also corrected a tracker/code mismatch in D-P0-A3-FINAL: the duplicate-owner check sits *ahead of* the settings gate, not after it. `KALPA_FORCE_WEBVIEW=1` on the reverse-handoff child short-circuits both gates, so the ordering is safe as written.
- **Outstanding**: `npm run test:e2e:sandbox` could not run — a Kalpa debug binary from another worktree (`t3code-a452d50a`, PID 52768) was live for this whole session, and the runner asserts no existing Kalpa process and would collide on the shared real app-data dir. Killing another session's app was out of scope. It also only isolates the AddOns folder and empties the real manifest-cache DB. Run it before release, per the master prompt.
- The `src-tauri/Cargo.toml` CRLF phantom (empty numstat) remains excluded, as in prior sessions.

### 2026-08-28 — Codex (P0-A3 completion)

- Closed the final handoff races in both directions: readiness no longer causes the predecessor to surrender authority until the successor publishes a matching acquired-authority proof, and cancellation remains sticky through authority reclaim so incoming activations are preserved.
- Replaced timeout and cancellation cleanup with bounded terminate-and-reap behavior even when `kill` races process exit. Added real-child regressions for ready-then-exit, acquired-proof ordering, pre-ready cancellation, and terminate/exit races.
- Verification: `npm run check` passes; main Rust has 852 passed/18 ignored; Slint has 798 passed/16 ignored; strict clippy passes for all targets in both crates; formatting and `git diff --check` pass. The unrelated `src-tauri/Cargo.toml` CRLF phantom remains excluded.
- Sol review: three review rounds identified early authority release, lost cancellation, and unbounded child reaping. All verified findings were fixed; the required GPT-5.5/xhigh read-only follow-up returned exact `VERDICT: APPROVE`.

### 2026-08-27 — P0-A3 resumed after interrupted session

- Recovered the interrupted P0-A3 session. Three commits were already on `fix/audit-p0-a3-sidecar-handshake` (unpushed); a fourth change was left uncommitted mid-`cargo fmt`. The uncommitted work type-checked cleanly on recovery and was completed rather than discarded.
- Committed as `fix(native): close handoff activation races`: authority release now commits under the authority mutex so a late activation cannot race the exit; duplicate-sidecar acceptance now requires positive proof of a live owner (matching ready marker **and** a genuinely held OS authority lock) so a stale `native-shell.active` record cannot mask a real startup failure; a deep link delivered to a reverse-handoff child is buffered until the page-ready callback owns authority.
- Replaced `incoming_activation_cancels_only_an_active_native_handoff`, whose name and body had diverged when `cancel_native_handoff_for_activation` gained an `AppHandle` parameter. The successor covers `commit_native_handoff_authority_release` and all three `finish_native_handoff_exit` cases. Both handoff atomics are process-global, so the cases are deliberately kept in one test function rather than split, which would let the parallel harness race the flags.
- Gates: Rust 842 passed/18 ignored; Slint 790 passed/16 ignored; `cargo clippy --all-targets -- -D warnings` clean on both crates; `cargo fmt --check` clean on both; `npm run build:native-slint` succeeds. The unrelated `src-tauri/Cargo.toml` CRLF phantom (empty numstat) remains excluded, as in prior sessions.
- **Outstanding before merge**: the master prompt's required final Fable P0 review, and `npm run test:e2e:sandbox`. The sandbox gate was deliberately not run unattended — it only isolates the AddOns folder and would empty the developer's real manifest-cache database.

### Cross-lane finding — two P0-A2 concurrency tests are flaky under full-suite load

- `transaction_lock::tests::two_process_read_modify_write_has_no_lost_updates` (added by P0-A2) failed once during a full Slint-suite run: the helper panicked on `atomic_file::atomic_write(...).unwrap()` at `transaction_lock.rs:350`.
- It is not a P0-A3 regression — P0-A3 does not touch `transaction_lock.rs` or `atomic_file.rs`, and the same test passed in the main crate on the same tree.
- Reproduction profile: 5/5 pass in isolation, 3/3 subsequent full-suite runs pass, 1 failure observed in the first full-suite run. Intermittent and load-dependent.
- The failure is in the *rename*, not the lock: the lock timeout was already raised to 10s for this test, and the panic is on the atomic publish. `atomic_file::rename_with_retries` allows `RENAME_ATTEMPTS = 5` at `RENAME_BACKOFF = 40ms`, a total budget of ~200ms — plausibly exhausted when an antivirus scanner or the search indexer holds the freshly created staging file under heavy parallel load.
- Not yet confirmed as a production defect, but the same ~200ms budget is used by `settings_store::rename_with_retries` on the real settings write path, so it is worth triaging against P0-A1 rather than only de-flaking the test. Recorded here so it is neither lost nor misattributed to P0-A3.
- **Update after the P0-A3 fix round.** A second, different test in the same family also failed once under full-suite load: `tests::concurrent_native_settings_writers_preserve_both_keys` (Slint `main.rs:19968`), where a `persist_addons_path_to_settings_path` call returned `Err` from one of two concurrent writer threads. It passes 8/8 in isolation. Its exact mechanism was **not** captured, so it is not claimed to be the same rename failure; it could equally be an A2 lock timeout.
- What is established: two independent A2 concurrency tests fail intermittently, only under full-suite parallel load, and one of the two was definitively inside `atomic_write`'s rename. `persist_addons_path_to_settings_path` is the real settings write path rather than a test-only harness, so this is worth root-causing rather than de-flaking.
- Recommended next step: Fable item 2a against P0-A1, plus logging `raw_os_error` on rename failure so the next occurrence names the code. `native_boot::write_record` already does.

### 2026-08-26 — Codex (P0-A2)

- Active branch: `fix/audit-p0-a2-cross-process-locking`, stacked on `fix/audit-p0-a1-atomic-writer`.
- Design: completed the fresh read-only Fable consultation and recorded D-P0-A2. Fable selected shared `fs4` try-locks with canonical sibling lock identities, deterministic multi-lock ordering, one bounded deadline, persistent non-authoritative lock files, and local-mutex → OS-lock ordering.
- Failing-before evidence: the real two-process counter regression encoded the prior read/sleep/write protocol, where both writers could observe the same value and one complete atomic publication replaced the other. Additional test-first cases covered a blocked contender timing out, owner termination releasing the lock, opposite lock request order, partial-acquisition cancellation, aliases, and fresh missing parents.
- Implementation: added the shared lock helper to both crates; wrapped metadata, settings, and profile RMW transactions; preserved the P0-A1 atomic writer; kept profile apply and other long work outside locks; and added bounded actionable error reporting. Tauri settings now merge exact frontend entries into freshly reloaded disk state and refresh the plugin cache. Slint narrow helpers merge under lock.
- Sol review: initial `REVISE` found that Slint's unrelated settings callback still wrote a stale full snapshot, serializing rather than preventing a cross-shell lost update. The writer now tracks a local baseline and applies only changed fields to the latest locked disk object. Exact newer-Tauri-field and two-snapshot independent-change races were added. The single required follow-up returned `APPROVE`, no findings, no missing tests, wire contract OK.
- Verification: root checks pass (`npm run check`; 37 files/490 tests; `npm audit --omit=dev` reports 0 vulnerabilities). Main Rust strict clippy, format, and tests pass (828 passed, 18 ignored); Slint strict clippy, format, and tests pass (777 passed, 16 ignored). The native Slint release build, Tauri debug build, and destructive sandbox pass (3/3). One first full Slint run saw an older 20-write stress test hit the intentional 2-second timeout under parallel contention; its isolated rerun and the complete rerun both passed. Cargo audit reports only the repository's existing transitive advisory baseline, with none attributable to `fs4` or `dunce`.
- Safety: Cargo test/build outputs used `B:\codex-build\kalpa-p0a2-target`; the unmodified native sidecar script used its required repository-local artifact path. The first sandbox exposed a fresh-install missing-parent lock-path bug, which was fixed and regression-tested. The final sandbox moved aside and restored both Roaming and Local app state, each confirmed restored.
- Handoff: pushed the branch and opened stacked draft PR [#388](https://github.com/ESO-Toolkit/kalpa/pull/388). No merge was performed.
- Blockers: none.
- Exact next action: review P0-A2 while P0-A1 lands, then mark the stacked PR ready once its base is mergeable.

### 2026-08-26 — Codex (P0-A1)

- Active branch: `fix/audit-p0-a1-atomic-writer`, stacked on `fix/audit-w1-worker-consistency`.
- Design: completed the required full P0 consultation with `claude --model fable` before implementation. Fable selected D-P0-1: one shared streaming atomic publisher for A1, a distinct OS transaction-lock layer for A2, and a launch-ID/positive-ready protocol for A3.
- Failing-before evidence: the concurrent metadata-save regression against the fixed `test.json.tmp` stage failed on Windows with finalization OS error 2. Clean-target testing then exposed simultaneous Windows replacement `PermissionDenied`, addressed with a process-local final-publish mutex and bounded transient retry without remove-then-rename.
- Implementation: added the shared `atomic_file.rs` to both Rust crates and migrated metadata, settings, safe migration ZIP/restore streams, SavedVariables, edit backups, profile mirror, Pack Hub import/export, addon editor, native settings fallback, and the Slint duplicate writer paths. Legacy `.json.tmp` recovery remains read-only compatibility behavior; new unique stages are never promoted or swept. P0-A2 and P0-A3 were not implemented.
- Tests: added uniqueness, owner-only cleanup, failed-publish cleanup, concurrent complete/parseable JSON, primary/backup recovery, concurrent pack export, import/editor completeness, and native fallback staging regressions. Root checks pass (`npm run check`; 37 files/490 tests). Main Rust strict clippy, format, and tests pass (814 passed, 17 ignored); Slint strict clippy, format, and tests pass (760 passed, 15 ignored). Native Slint and Tauri debug builds pass with Cargo outputs on `B:\codex-build\kalpa-p0a1-target`. The destructive sandbox passes 3/3.
- Sol review: initial `REVISE` identified four remaining fixed replacement writers in `commands.rs` (Pack Hub SavedVariables import, addon editor, native settings fallback, and pack export). All four, plus the profile mirror found in the same sweep, were migrated and covered by regressions. Required follow-up verdict: `APPROVE`; no findings, no missing tests, wire contract OK, bug-class sweep clean.
- Safety: an ENOSPC event invalidated earlier partial evidence; source diffs were inspected for truncation, `safe_migration.rs` was restored from HEAD, disk capacity was recovered, and every required gate was rerun from scratch. Existing Roaming/Local app state was moved aside and restored around the destructive sandbox. Fresh sandbox outputs are preserved at `C:\Users\brayd\AppData\Roaming\com.kalpa.desktop.codex-p0a1-sandbox-output-20260826` and `C:\Users\brayd\AppData\Local\com.kalpa.desktop.codex-p0a1-sandbox-output-20260826`.
- Handoff: pushed the branch and opened stacked draft PR [#380](https://github.com/ESO-Toolkit/kalpa/pull/380). No merge was performed.
- Blockers: none.
- Exact next action: review and merge the stacked P0-A1 draft after W1, then begin P0-A2 from D-P0-1 with a fresh Fable consultation before choosing the lock dependency/API.

### 2026-08-26 — Codex

- Active branch: `fix/audit-w1-worker-consistency`
- Completed: read repository guidance, master prompt, and audit memory; fetched and fast-forward checked `main`; created the persistent tracker; started Kalpa successfully in Tauri dev mode; completed the W1 Fable review; captured five failing-before DO tests; implemented DO-authoritative mutations; added route-level duplicate, stale-update, and delete/vote race coverage; Worker typecheck and 159 tests pass.
- Active work: W1 PR maintainer review after addressing every verified finding from the initial and follow-up Sol reviews.
- Completed follow-up: Fable selected the continuously merged shadow design in D-W1-2. Implemented lifecycle guards for vote/install, canonical update/delete authorization, atomic restore preservation, deleted-pack vote cleanup (including `backup:latest`), repeated KV backfill with tombstones, parity-gated authority control, and explicit detail-witness adoption. Worker typecheck, 168 tests, and Wrangler dry-run pass; `wrangler.toml` remains `kalpa-pack-hub`.
- Sol follow-up: `REVISE`. Verified an incomplete-shadow full-index clobber, scheduled-backup reintroduction of another user's vote on a deleted pack, and delete/recreate vote-cleanup interleaving. Addressed by suppressing full-index writes during shadow mode, serving merged list/backup reads from the DO, filtering backup votes by deleted pack id, and moving vote cleanup inside the serialized delete lifecycle. Added the three requested regressions; Worker typecheck and 170 tests pass.
- Handoff: pushed `fix/audit-w1-worker-consistency` and opened PR [#369](https://github.com/ESO-Toolkit/kalpa/pull/369). All three GitHub CI jobs pass. No merge or deployment was performed; unrelated local `Cargo.toml` and theme-directory changes remain excluded.
- Blockers: no implementation or CI blockers remain. Maintainer approval is still required because merge auto-deploys the Worker shadow phase. The authority flip remains a separate operator step after soak/parity checks.
- Exact next action: maintainer reviews and merges PR #369 after accepting the shadow-mode rollback caveat, then monitors the production parity/authority-flip runbook.

### Sol review 1 — REVISE

Verified findings:

1. First post-deploy bootstrap can permanently omit a live pack when KV returns a stale index.
2. A stale vote/install request can cross delete-and-recreate and mutate the new pack lifecycle.
3. Update/delete authorization still trusts stale KV ownership rather than canonical DO ownership.
4. Restore finalization preserves concurrent packs from KV instead of atomically from DO authority.
5. Account deletion leaves other users' vote records attached to removed pack IDs, so a reused ID can inherit votes.

Wire contract verdict: OK. Bug-class sweep found the restore and account-deletion sites above.

## Open Questions

- P0-A1: adopt Fable item 2a, widening the rename budget from ~200ms (`RENAME_ATTEMPTS = 5` at a flat 40ms) to a bounded geometric backoff of roughly 2.5s, and adding `ERROR_USER_MAPPED_FILE` (1224) to `is_transient_rename_error`. Two P0-A2 concurrency tests flake under load and one was definitively in the rename. The same budget serves the real `settings.json` write path in both binaries, so the trade is bounded extra latency against a hard failure. Needs a maintainer decision on acceptable worst-case settings-save latency.

- P0-A3: should losing UI authority be *terminal*? After a failed reclaim (either handoff direction) the process stays alive and visible without holding the authority lock. It is not two writers — the counterpart child is terminated and reaped first — and the activation path now refuses to reveal or emit in that state (D-P0-A3-ACTIVATION). The remaining options are to leave it as-is, retry the reclaim on the next activation/toggle, or exit with a user-visible message. Exiting costs the user their session for a rare condition, so it needs a maintainer decision. Raised independently by Fable (RISKS 2) and Sol (findings 2 and 3).

- W2: maintainer approval is required before merging any reconciliation path that can delete rows from shared D1.
- W1: decide whether moving vote-record authority from KV/in-memory memo into DO storage belongs in W1 or W3. The current W1 code prevents resurrection but retains the pre-existing eviction/double-toggle limitation for later hardening.
- W1: owner sign-off is required before the later manual `kv` → `do` authority flip and must accept backup restore as the post-flip rollback path.
- P0-A2: D-P0-1 establishes the lock invariants; a fresh Fable consultation must choose the concrete cross-platform dependency/API and user-visible timeout behavior before implementation.
- R4/R5: ownership/conflict behavior that changes install outcomes requires explicit design review before implementation.
- H2: `kalpa-elder-scrolls-themes/` is currently untracked; provenance must be inspected before tracking or ignoring it.
