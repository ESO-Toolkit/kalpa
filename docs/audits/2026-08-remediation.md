# August 2026 Audit Remediation Tracker

This file is the durable execution record for `2026-08-remediation-master-prompt.md`.

| ID | Status | Branch | PR | Merged SHA | Fable decision | Sol verdict | Tests | Notes |
|---|---|---|---|---|---|---|---|---|
| W1 | pr-open | `fix/audit-w1-worker-consistency` | [#369](https://github.com/ESO-Toolkit/kalpa/pull/369) (draft) | - | D-W1-1, D-W1-2, D-W1-3 | REVISE; follow-up REVISE; all verified findings addressed | Worker check; 184 tests; Wrangler dry-run; name guard | Fable twice-reject redesign implemented; awaiting refreshed CI/maintainer sign-off. |
| W2 | todo | - | - | - | pending | - | - | Requires maintainer approval before merge if reconciliation can delete D1 rows. |
| W3 | todo | - | - | - | - | - | - | Worker low-severity hardening. |
| P0-A1 | pr-open | `fix/audit-p0-a1-atomic-writer` | [#380](https://github.com/ESO-Toolkit/kalpa/pull/380) (draft) | - | D-P0-1 | REVISE; all verified findings addressed; follow-up APPROVE | Root 490; Rust 814 passed/17 ignored; Slint 760 passed/15 ignored; native build; Tauri build; sandbox 3/3 | Shared crash-safe atomic writer; stacked on W1. |
| P0-A2 | pr-open | `fix/audit-p0-a2-cross-process-locking` | [#388](https://github.com/ESO-Toolkit/kalpa/pull/388) (draft) | - | D-P0-A2 | REVISE; verified finding fixed; follow-up APPROVE | Root 490; Rust 828 passed/18 ignored; Slint 777 passed/16 ignored; native build; Tauri build; sandbox 3/3 | Shared bounded cross-process RMW locking; stacked on P0-A1. |
| P0-A3 | todo | - | - | - | pending | - | - | Native sidecar ready handshake. |
| R4 | todo | - | - | - | pending | - | - | Preserve separately tracked sibling ownership. |
| R5 | todo | - | - | - | pending | - | - | Folder-qualified conflict protection. |
| R6 | pr-open | `fix/audit-r6-crash-safe-installer` | [#399](https://github.com/ESO-Toolkit/kalpa/pull/399) (draft) | - | D-R6-1 | REVISE; all verified findings addressed; follow-up APPROVE | Root 490; Rust 840 passed/18 ignored; strict clippy both crates; native build; transaction 10; cross-instance copy 3; focused editor/restore tests | Crash-safe merged staging, folder swaps, hash promotion, and deterministic recovery; stacked on P0-A2. Slint test link and destructive sandbox have environment blockers recorded below. |
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

## Session Log

### 2026-08-27 — W1 twice-reject escalation

- PR #369 was returned to draft after review identified four additional correctness failures: stale KV versions can be frozen by ID-only shadow merging; partial vote cleanup can corrupt a still-live pack; a failed KV detail deletion is not retryable after the DO tombstone commits; and a failed KV detail write leaves a committed create that retries as a duplicate.
- Per the twice-reject rule, W1 is blocked and implementation is paused while Fable is reconsulted with both prior Sol reviews and the new review evidence.
- Fable selected D-W1-3: journaled and resumable lifecycle transitions, tombstone-first delete, DO-authoritative detail reads, version-aware shadow reconciliation, and alarm-based effect repair.
- Fresh Sol review returned `REVISE` with four findings: orphan detail adoption during account purge; stale same-author operations crossing slug reuse; vote/install counters committing before a fallible KV mirror; and a backup write racing account deletion. The single prescribed follow-up also returned `REVISE` after verifying those cases.
- Addressed every follow-up finding with author-scoped orphan hydration, created-at lifecycle compare-and-swap for update/delete, durable dirty-mirror markers repaired by alarm, and DO-serialized backup/account deletion guarded by a deleted-author latch. Added exact failure/retry regressions.
- Final local evidence: Worker TypeScript check passes; all 184 tests pass; Wrangler dry-run passes; `wrangler.toml` remains `kalpa-pack-hub`. No deploy, merge, schema change, or Worker rename was performed.
- Active branch: `fix/audit-r6-crash-safe-installer`, stacked on `fix/audit-p0-a2-cross-process-locking`.
- Design: completed the required fresh read-only Fable consultation and recorded D-R6-1. The selected design is a copied-residual merged stage, journaled whole-folder tombstone swap, post-swap hash promotion, and deterministic lock-protected restart recovery. The maintainer choices were to refuse links and use copies rather than hard links.
- Implementation: added the shared installer transaction module to Tauri and Slint; moved ZIP extraction and keep-mine baseline construction into staged images; migrated install, update, batch update, remove, scan, profile activation, restore, and native equivalents to recover or hold the transaction lock; and excluded reserved transaction state from addon scans.
- Verification: `npm ci`, `npm test` (37 files, 490 tests), `npm run check`, formatting for both Rust crates, strict all-target/all-feature clippy for both crates, the native Slint release-sidecar build, and the full main Rust suite (840 passed, 18 ignored) pass. The focused transaction suite passes 10 tests, covering rollback after one of multiple folder swaps, abandoned staged/swapped markers, missing expected live folders and hash baselines, invalid journal paths, locked files, transient rename retries, successful cleanup, cancellation preservation, traversal/reserved names, residual files, and keep-mine behavior. Three focused cross-instance copy tests and focused editor/Protected Edits restore tests also pass; strict main-crate clippy and all 10 transaction tests were rerun after the final batch enable/disable guard.
- Environment limitations: two single-job full Slint test attempts reached the final link and were terminated by Windows/LLVM memory exhaustion; the same crate passes strict clippy and its optimized native sidecar build completes. Destructive sandbox verification is blocked because PID 52768 is a user-owned Kalpa process in another T3 worktree and the runner cannot isolate app state; it was not terminated.
- Review: adversarial Sol review returned REVISE for unguarded live AddOns mutations in the manual editor, Protected Edits restore, cross-instance copy, and batch enable/disable paths. All verified sites were migrated to the shared transaction guard; the final focused follow-up returned APPROVE with no findings, missing tests, or wire-contract changes.
- Handoff: opened stacked draft PR [#399](https://github.com/ESO-Toolkit/kalpa/pull/399) against `fix/audit-p0-a2-cross-process-locking`. No merge or deployment was performed; the next action is review and merge in stack order after P0-A2.
- Blockers: final Slint test linking and destructive sandbox execution are externally blocked as described above; no implementation blocker.

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

- W2: maintainer approval is required before merging any reconciliation path that can delete rows from shared D1.
- W1: decide whether moving vote-record authority from KV/in-memory memo into DO storage belongs in W1 or W3. The current W1 code prevents resurrection but retains the pre-existing eviction/double-toggle limitation for later hardening.
- W1: owner sign-off is required before the later manual `kv` → `do` authority flip and must accept backup restore as the post-flip rollback path.
- P0-A2: D-P0-1 establishes the lock invariants; a fresh Fable consultation must choose the concrete cross-platform dependency/API and user-visible timeout behavior before implementation.
- R4/R5: ownership/conflict behavior that changes install outcomes requires explicit design review before implementation.
- H2: `kalpa-elder-scrolls-themes/` is currently untracked; provenance must be inspected before tracking or ignoring it.
