# August 2026 Audit Remediation Tracker

This file is the durable execution record for `2026-08-remediation-master-prompt.md`.

| ID | Status | Branch | PR | Merged SHA | Fable decision | Sol verdict | Tests | Notes |
|---|---|---|---|---|---|---|---|---|
| W1 | pr-open | `fix/audit-w1-worker-consistency` | [#369](https://github.com/ESO-Toolkit/kalpa/pull/369) (draft) | - | D-W1-1, D-W1-2, D-W1-3 | REVISE; follow-up REVISE; all verified findings addressed | Worker check; 184 tests; Wrangler dry-run; name guard | Fable twice-reject redesign implemented; awaiting refreshed CI/maintainer sign-off. |
| W2 | pr-open | `fix/audit-w2-d1-reconciliation` | [#378](https://github.com/ESO-Toolkit/kalpa/pull/378) (draft) | - | D-W2-1, D-W2-2 | Fresh REVISE; follow-up APPROVE | Worker check; 208 tests; Wrangler dry-run | Technically ready and stacked on W1. Draft/merge hold remains: deletion-capable code and later `apply` each require separate maintainer approval. |
| W3 | pr-open | `fix/audit-w3-worker-hardening` | [#379](https://github.com/ESO-Toolkit/kalpa/pull/379) (draft) | - | D-W3-1 reaccepted after reconsultation | Fresh REVISE; follow-up APPROVE | Worker check; 233 tests; Wrangler dry-run | Technically ready and stacked on refreshed W2/D-W2-2; remains draft with no real deployment. |
| P0-A1 | pr-open | `fix/audit-p0-a1-atomic-writer` | [#380](https://github.com/ESO-Toolkit/kalpa/pull/380) (draft) | - | D-P0-1 | REVISE; all verified findings addressed; follow-up APPROVE | Root 490; Rust 814 passed/17 ignored; Slint 760 passed/15 ignored; native build; Tauri build; sandbox 3/3 | Shared crash-safe atomic writer; stacked on W1. |
| P0-A2 | todo | - | - | - | pending | - | - | Cross-process read-modify-write locking. |
| P0-A3 | todo | - | - | - | pending | - | - | Native sidecar ready handshake. |
| R4 | todo | - | - | - | pending | - | - | Preserve separately tracked sibling ownership. |
| R5 | todo | - | - | - | pending | - | - | Folder-qualified conflict protection. |
| R6 | todo | - | - | - | pending | - | - | Crash-safe installer transaction. |
| R7 | pr-open | `fix/audit-r7-build-evidence-bound` | [#372](https://github.com/ESO-Toolkit/kalpa/pull/372) (draft) | - | - | APPROVE | Focused 1; evidence 16; frontend 490; Rust 808 | D-R7-1: one-shot evidence uses the encoder's exact `scanned_len` byte bound; stacked on W1. |
| R8 | pr-open | `fix/audit-r8-protected-edits-disclosure` | [#381](https://github.com/ESO-Toolkit/kalpa/pull/381) (draft) | - | D-R8-1 | REVISE; all in-scope findings addressed | Frontend check; 497 tests; Rust 809/17 ignored; Slint 757/15 ignored; clippy/fmt; native release build; Luna PASS | Non-blocking disclosure for absent/invalid baselines across React and shipped Slint; stacked on W1. |
| R9 | pr-open | `fix/audit-r9-downloaded-version` | [#376](https://github.com/ESO-Toolkit/kalpa/pull/376) (draft, stacked) | - | not required | REVISE → REVISE; all verified findings addressed | 810 Rust; 490 frontend; clippy/fmt/check green | Persist checksum-bound filedetails version and invalidate stale update observations only after an applied update. |
| F1 | todo | - | - | - | - | - | - | Import-source sequencing. |
| F2 | pr-open | `fix/audit-f2-log-directory-sequencing` | [#373](https://github.com/ESO-Toolkit/kalpa/pull/373) (draft) | - | D-F2-1 | REVISE x2; all verified findings addressed | Frontend check; 496 tests | Stacked on W1; no wire or persisted-data changes. |
| F3 | pr-open | `fix/audit-f3-fresh-import-metadata` | [#375](https://github.com/ESO-Toolkit/kalpa/pull/375) (draft) | - | D-F3-1 | APPROVE | Frontend check; 498 tests | Stacked on F2; no wire or persisted-data changes. |
| F4 | pr-open | `fix/audit-f4-logout-invalidation` | [#374](https://github.com/ESO-Toolkit/kalpa/pull/374) (draft, stacked on F1) | - | D-F4-1 | APPROVE | Focused 1 test; pack sequencing 4 tests; frontend check; 494 tests | Successful logout invalidates every private-list page request before clearing signed-in state; no wire/persisted-data change. |
| F5 | pr-open | `fix/audit-f5-controlled-state-veto` | [#370](https://github.com/ESO-Toolkit/kalpa/pull/370) (draft, stacked on W1) | - | not required | APPROVE | Focused 10/10; frontend check; 493 tests | Controlled values remain authoritative when a parent vetoes a change; uncontrolled behavior is preserved. |
| F6 | pr-open | `fix/audit-f6-optimistic-sequencing` | [#377](https://github.com/ESO-Toolkit/kalpa/pull/377) (draft, stacked) | - | D-F6-1; Luna PASS | REVISE follow-up; no findings, requested tests addressed | Frontend check; 500 tests | Sequenced optimistic settings/library state and latest-only SavedVariables file/character refreshes implemented. |
| H1 | todo | - | - | - | - | - | - | Generate release copy from matching CHANGELOG section. |
| H2 | todo | - | - | - | - | - | - | Decide theme-image provenance and tracking policy. |
| H3 | pr-open | `fix/audit-h3-worker-version-policy` | [#386](https://github.com/ESO-Toolkit/kalpa/pull/386) (draft) | - | D-H3-1 | APPROVE | Worker policy/check; 218 tests; root check/490 tests/version gate; Wrangler dry-run | Stacked on W3; independent Worker uses sentinel `0.0.0`; no real deployment. |
| H4 | todo | - | - | - | - | - | - | Update `claude.md` structure tree. |
| H5 | todo | - | - | - | - | - | - | Propose branch pruning; do not delete without approval. |
| H6 | pr-open | `fix/audit-h6-quick-xml-advisory` | [#387](https://github.com/ESO-Toolkit/kalpa/pull/387) (draft, stacked on W1) | - | D-H6-1 | Initial REVISE resolved; follow-up no findings | Main audit clean; main/Slint clippy, test, fmt, native build green | Compatible upstream lock updates remove quick-xml advisories; CI ignores removed. |

## Decisions

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

 ### D-R8-1 — Disclose missing coverage without blocking updates

- Chosen: treat an absent, corrupt, or folder-mismatched `.kalpa-hashes` manifest as no trusted Protected Edits baseline and disclose that Kalpa cannot detect changed files before mutation. Updates remain enabled because no explicit blocking policy was chosen.
- React manual, selected, batch, context-menu, and launch-auto-update flows share explicit coverage state. Batch coverage is refreshed at action time and published only while its captured instance generation and AddOns path remain current. The context shortcut opens the detail review flow, whose conflict report supplies the freshest single-addon baseline status.
- Shipped Slint single and batch flows re-read manifests at action time, display the same specific risk, and never label unprotected updates “safe.”
- Rejected: silently creating a trusted baseline from a migrated addon's existing files. Those files may already contain user edits, so seeding requires separate design review.
- Rejected: blocking migrated addons from updating. That changes product policy beyond the audit's minimum acceptable fix.
 - Compatibility: wire fields are additive and serde-defaulted; persisted metadata and hash-manifest formats are unchanged.
### D-F6-1 — Persisted optimistic state tracks confirmed storage

- Chosen: centralize optimistic setting and installed-pack reference mutations in a hook that assigns monotonically increasing operation IDs, composes functional updates against the latest visible value, and records every successful serialized store write as the rollback target. Only the newest failed operation may change UI or surface an error; rollback restores confirmed storage rather than inverting the submitted value or reinstating a captured array.
- Settings hydration remains a confirmed rollback candidate while a user write is pending, but cannot replace the optimistic value and is ignored after any newer write confirms.
- SavedVariables list refreshes use a separate latest-request sequence. Only the active request may apply files, errors, or loading completion; unmount invalidates outstanding requests.
- Compatibility: no settings keys, persisted `installed_packs` shape, Tauri command arguments, or visible control structure changed.
- Rollback: revert the F6 commit; persisted data remains readable because this change only alters frontend sequencing.

### D-F2-1 — Gate log-directory work by operation and directory

- Chosen: mirror the active log directory synchronously and assign monotonically increasing IDs to list and detection requests. Apply list success, list error, selection reconciliation, loading completion, and detection errors only while both request ID and directory still match.
- Directory changes immediately invalidate older list/detection work and clear incompatible list, error, and selection state. Deferred imports capture and re-check their directory before refreshing or selecting.
- Rejected: checking only the directory. A same-directory refresh can still resolve out of order. Rejected: checking only an operation ID local to `loadLogs`; detection and import completion can independently reclaim stale directory or selection state.
- Compatibility: frontend-only state sequencing; no IPC, wire-format, persisted-data, or backend behavior changes.
### D-F1-1 — One sequence owns every import source

- Chosen: share-code resolution and `.esopack` import capture IDs from one component-scoped monotonic sequence; only the current ID may publish pack data, file settings, errors, or share loading cleanup. Opening the file picker, changing import methods, and Clear invalidate prior work.
- Rejected: independent per-source IDs, because a request from one source could still overwrite the other source. Rejected: guarding only successful results, because stale failures and cleanup could still replace the active operation's error/loading state.
- Compatibility: no wire-format, persisted-data, dependency, or visible UI changes. Rollback is a normal code revert with no data action.

### D-F4-1 — Successful logout invalidates the private-list lifecycle

- Chosen: after `auth_logout` succeeds, advance the existing `loadMyPacksSeqRef` before publishing signed-out state and clearing My Packs data/loading flags. The loader's existing success, error, and cleanup guards then reject every pre-logout page request.
- Rejected: clearing state without advancing the sequence, because a late authenticated result remains current and can repopulate it. Rejected: invalidating before `auth_logout` succeeds, because a failed logout leaves the user authenticated and should preserve the current private view.
- Compatibility: no wire-format, persisted-data, dependency, backend, or visible UI change. Rollback is a normal code revert with no data action.

### D-W2-1 — Bounded, ownership-gated D1 reconciliation

- Chosen: after the independent daily backup attempt, compare one validated DO authority snapshot against explicit columns from only `packs` and `pack_tags`. Build the entire deterministic repair plan before mutation; exact `dry-run` is the checked-in default, exact `apply` is the only mutation-capable mode, and missing/invalid values fail to dry-run.
- Durable DO tombstones plus current authoritative IDs are the ownership proof for deletion. An extra D1 row without that proof is reported as `unowned_extra` and never deleted. A valid empty authority can delete at most five tombstone-proven rows; larger empty divergence and all other count/ratio limit violations reject the whole plan.
- Inline D1 failures persist `d1-mirror:last_error`; reconciliation persists `d1-recon:last` or stage-specific `d1-recon:last_error` with planned/applied counts. Authority failure occurs before any D1 statement, and D1 read failure occurs before any mutation.
- Rejected: a KV repair queue, because KV cannot atomically append concurrent failures and the queue cannot discover historical or falsely successful stale writes. Rejected: a paginated cursor sweep, because no stable authority generation exists to prove completeness before zombie deletion.
- No D1 schema, Worker/Rust wire shape, or unrelated shared table changes. Rollback is a code revert; checked-in dry-run performs no reconciliation mutations. Enabling `apply` and merging deletion-capable code both require explicit maintainer approval.

### D-W2-2 — Authority-gated, bounded, single-flight scheduling

- Chosen after the twice-`REVISE` Fable reconsultation: retain and report deletion candidates while W1 remains in `kv` shadow authority; permit deletion planning only after the explicit parity-gated `do` authority flip.
- Bound shared D1 reads with ceiling-plus-one probes (`packs` 2,001; `pack_tags` 20,001) and reject the plan when either configured ceiling is exceeded.
- Serialize scheduled reconciliation with a token-owned, expiring lease in the existing PackIndexDO. Overlapping runs record `skipped-overlap`, and a stale token cannot release a newer lease.
- Preserve per-pack lifecycle gates for writes/deletes, deterministic planning, dry-run default, existing-table-only SQL, and the separate merge/apply approval sequence from D-W2-1.

### D-W3-1 — Bounded Worker edge state and disclosure

- Chosen: stream and byte-count every JSON request body; bound vote and auth identity memos with oldest-entry eviction; and keep auth-cache generation checks so stale in-flight lookups cannot repopulate reset state.
- Chosen: remove the manual Cache API list entry. Its invalidation could not coordinate across Worker isolates, and it is ineffective on the current `workers.dev` route. The unchanged list response uses a bounded 30-second `Cache-Control` TTL only.
- Chosen: serialize each install claim and counter update in one Durable Object storage transaction. Claims use an admin-keyed HMAC identity, a fixed 5,000-slot oldest-eviction ring, an alarm-backed one-hour retention limit, lifecycle cleanup, and retry mirroring. Live legacy `install-rate:<pack>:<ip>` keys remain honored through their existing TTL, avoiding rollout double counts.
- Chosen: public `/health` exposes only `status`, KV reachability, and timestamp. Corpus size and backup freshness remain operator data; the deploy workflow consumes only `status` and `kv`. This intentionally removes undocumented health fields but does not change Pack Hub/Rust response shapes or any D1 schema.
- Fable reaccepted D-W3-1 after two verified Sol revisions. It clarified that rejected/aborted streams fail as invalid JSON, decoding happens only after bounded byte assembly, and viewer-bearing responses explicitly emit `max-age=0` and vary on `Authorization` and `Origin`.
- The 5,000-slot claim ring is deliberately bounded idempotence: an identity may recount inside one hour only after 5,000 newer distinct claims evict it. Rotating `ADMIN_API_KEY` can likewise admit one extra count per identity until old claims expire; both are acceptable for a display counter and are not exact billing semantics.

### D-H3-1 — Worker package version is an independent sentinel

- Chosen: keep the private Pack Hub Worker at the non-release sentinel `0.0.0` in `package.json` and both npm lockfile fields. Its deployment workflow is triggered by Worker-path pushes to `main`; neither Wrangler, the runtime, health checks, nor the desktop wire client consumes this metadata. Desktop tags and `check:versions` separately own the six app version fields.
- Enforced: `npm run check:version-policy` rejects any value other than `0.0.0` in the three npm-owned Worker fields. It runs inside Worker `npm run check`, so both pull-request CI and the pre-deploy workflow execute it.
- Rejected: synchronizing the Worker to each desktop tag. That would imply shared release ownership which the independent deployment pipeline does not provide, and would manufacture version changes unrelated to Worker deployments.
- Compatibility: no Worker runtime, response shape, D1 schema, Wrangler configuration, desktop release field, or deployment trigger changed. Rollback is a normal code revert; it does not alter deployed state or persisted data.

### D-F3-1 — Select imports from the applied refresh result

- Chosen: return the exact `LogFileInfo[]` applied by the operation/directory-guarded `loadLogs` call, find the imported path in that result, and pass its metadata directly into selection. Stale, failed, or mismatched refreshes return no usable result and cannot select.
- Rejected: awaiting `setLogs` and then reading the selector's captured `logs` array. React state application does not update the closure of an already-running import callback, so classification remains render-timing dependent.
- Large imports use the refreshed `sizeBytes` and enter the existing deferred/full-preflight route without invoking `uploader_preflight`. Small imports still preflight normally.
- Compatibility: frontend-only orchestration; no IPC, wire-format, persisted-data, backend, dependency, or Worker changes.

## Session Log

### 2026-08-26 — Codex (F4)

- Active branch: `fix/audit-f4-logout-invalidation`; draft stacked PR [#374](https://github.com/ESO-Toolkit/kalpa/pull/374) targets `fix/audit-f1-import-sequencing`.
- Completed: reproduced a deferred authenticated My Packs request repopulating state after successful logout; implemented D-F4-1 by invalidating the shared private-list sequence before clearing signed-out state. The focused regression passes 1/1, related pack sequencing tests pass 4/4, `npm run check` passes, and `npm test` passes 39 files/494 tests.
- Review: required local read-only Sol review returned `APPROVE` with no findings, no missing tests, wire contract OK, and a clean authenticated-private-load bug-class sweep. No follow-up review was required.
- Blockers: no F4 implementation blockers. The PR remains draft and stacked until its F1 base is available in the integration history.
- Exact next action: monitor PR CI, then mark #374 ready after base-branch sequencing is confirmed; do not merge from this worktree.

### 2026-08-26 — Codex (F1)

- Active branch: `fix/audit-f1-import-sequencing`; draft stacked PR [#371](https://github.com/ESO-Toolkit/kalpa/pull/371) targets `fix/audit-w1-worker-consistency`.
- Completed: reproduced both cross-source races with deferred promises resolving in reverse order; implemented D-F1-1; added method-switch invalidation after the verified Sol finding; focused sequencing tests pass 3/3, `npm run check` passes, and `npm test` passes 40 files/593 tests.
- Review: initial Sol `REVISE` found the method-toggle invalidation gap; follow-up Sol `APPROVE` reported no findings, no missing tests, wire contract OK, and a clean bug-class sweep.
- Blockers: no F1 implementation blockers. The PR remains draft and stacked until its W1 base is available in the integration history.
- Exact next action: run PR CI, then mark #371 ready after the base-branch sequencing is confirmed; do not merge from this worktree.

### 2026-08-27 — W1 twice-reject escalation

- PR #369 was returned to draft after review identified four additional correctness failures: stale KV versions can be frozen by ID-only shadow merging; partial vote cleanup can corrupt a still-live pack; a failed KV detail deletion is not retryable after the DO tombstone commits; and a failed KV detail write leaves a committed create that retries as a duplicate.
- Per the twice-reject rule, W1 is blocked and implementation is paused while Fable is reconsulted with both prior Sol reviews and the new review evidence.
- Fable selected D-W1-3: journaled and resumable lifecycle transitions, tombstone-first delete, DO-authoritative detail reads, version-aware shadow reconciliation, and alarm-based effect repair.
- Fresh Sol review returned `REVISE` with four findings: orphan detail adoption during account purge; stale same-author operations crossing slug reuse; vote/install counters committing before a fallible KV mirror; and a backup write racing account deletion. The single prescribed follow-up also returned `REVISE` after verifying those cases.
- Addressed every follow-up finding with author-scoped orphan hydration, created-at lifecycle compare-and-swap for update/delete, durable dirty-mirror markers repaired by alarm, and DO-serialized backup/account deletion guarded by a deleted-author latch. Added exact failure/retry regressions.
- Final local evidence: Worker TypeScript check passes; all 184 tests pass; Wrangler dry-run passes; `wrangler.toml` remains `kalpa-pack-hub`. No deploy, merge, schema change, or Worker rename was performed.
### D-H6-1 — Adopt compatible upstream quick-xml fixes

- Chosen: update the main lockfile to `plist` 1.10.0 and
  `tauri-winrt-notification` 0.7.3, and the Slint lockfile to
  `wayland-scanner` 0.31.11. These compatible upstream releases converge both
  graphs on patched `quick-xml` 0.41.0 without changing direct dependency
  requirements or application code.
- Evidence before: `cargo audit` reported RUSTSEC-2026-0194 and
  RUSTSEC-2026-0195 against main's `quick-xml` 0.37.5/0.39.4 and Slint's
  0.39.4. Inverse trees traced the main edges through
  `tauri-winrt-notification`/`notify-rust` and `plist`/Tauri, and the Slint edge
  through `wayland-scanner`/Wayland UI crates.
- Evidence after: the main audit reports zero vulnerabilities; the Slint audit
  no longer reports either quick-xml advisory. Its three remaining advisories
  are unrelated pre-existing findings in `crossbeam-epoch`, `h2`, and
  `webbrowser` and are outside H6.
- Rejected: retaining CI ignores. The upstream constraints that justified the
  exception have moved, so suppression would now hide a fixable high-severity
  dependency issue.
- Compatibility and rollback: lockfile-only patch/minor transitive updates;
  wire and persisted-data formats are unchanged. Revert this branch's commit to
  restore the prior resolved graph and CI exceptions.

### 2026-08-26 — Codex (H6)

- Active branch: `fix/audit-h6-quick-xml-advisory`, stacked on W1.
- Finding: revisit the ignored RUSTSEC-2026-0194 and RUSTSEC-2026-0195
  quick-xml advisories when upstream dependency constraints permit.
- Toolchain: stable `rustc` 1.94.0, Cargo 1.94.0, cargo-audit 0.22.1.
- Objective before/after reproduction replaces a code regression test for this
  dependency-only finding: unignored audits and inverse trees identified three
  vulnerable quick-xml edges; Cargo-resolved upstream updates converge both
  lockfiles on quick-xml 0.41.0, clearing H6 from both audit reports.
- Scope: only Cargo-resolved lockfiles, the obsolete CI suppressions, and this
  tracker decision. No direct requirements, runtime code, wire formats, or
  persisted data changed.
- Gates: main clippy/fmt, strict all-targets clippy, 824 tests (807 passed, 17
  ignored), and fmt check pass. Slint clippy/fmt, strict all-targets clippy, 770
  tests (755 passed, 15 ignored), fmt check, and `npm run build:native-slint`
  pass. Main `cargo audit` reports zero vulnerabilities; Slint no longer reports
  either H6 advisory.
- Active work: draft stacked PR #387 is ready for maintainer review after W1.
- Exact next action: merge W1 first, then rebase or retarget #387 onto `main` and
  require its remote checks before marking it ready.
- Sol review: `REVISE`. Verified that `README.md` and `SECURITY.md` still
  described the removed quick-xml exceptions as current, and that Slint CI had
  no committed regression gate for its separate lockfile. Corrected both public
  documents and added a locked Cargo-metadata floor check for every resolved
  Slint quick-xml package without suppressing unrelated sidecar advisories.
- Follow-up action: focused and full gates were rerun and passed. The single
  required Sol follow-up returned no findings, confirming that the stale
  documentation, obsolete main-CI ignores, and missing Slint lockfile gate are
  resolved.
### D-R9-1 — The fetched descriptor is artifact provenance

- Chosen: persist the version from the same filedetails response that supplied the download URL and checksum. The frontend filelist version remains in the wire contract as the earlier observation that initiated the request, but it is not authoritative after the backend fetch.
- Successful applied updates invalidate the in-memory filelist observation after metadata is durable, so the next check cannot compare the newly installed artifact against the stale observation. Zero-success batches preserve the cache.
- Rejected: trusting the frontend-observed version. ESOUI can publish between check and download, which records the wrong version for the artifact actually fetched and creates a phantom update loop.
- Rejected: ordering or normalizing opaque version strings. Artifact identity is established by the fetched descriptor/checksum tuple, not a guessed version ordering.
- Compatibility and rollback: Tauri request/response fields and persisted metadata/hash schemas are unchanged. Rollback is a code revert; no migration or data rewrite is required.

## Session Log

### 2026-08-26 — Codex (R9)

- Active branch: `fix/audit-r9-downloaded-version`, stacked on `fix/audit-w1-worker-consistency`.
- Failing-before: the publish-race regression expected fetched `v2` but metadata selection returned stale observed `v1`. The follow-up zero-success regression initially failed to compile because conditional cache invalidation did not exist.
- Implemented: all single, legacy batch, streaming batch, and deferred-conflict update paths now carry the checksum-bound filedetails version through hashing and metadata. Successful applied updates discard the stale filelist observation only after metadata persists; failed and zero-success batches retain it.
- Verification: Rust clippy fix, fmt, clippy all targets with warnings denied, 810 tests (17 ignored), and fmt check pass. Frontend typecheck/lint/Prettier and 490 tests pass. No Slint sidecar gate was required because no shared sidecar module changed.
- Review: Fable and Luna were not required. Initial Sol review found the stale filelist phantom-loop path; follow-up Sol review found legacy zero-success cache invalidation. Both verified findings and requested regressions are addressed; the one-follow-up limit was observed.
- Handoff: pushed the branch and opened draft stacked PR [#376](https://github.com/ESO-Toolkit/kalpa/pull/376) against `fix/audit-w1-worker-consistency`. No merge was performed.
- Exact next action: wait for base PR #369, then review/merge #376 after its required checks are green.

### R9 Sol review — initial REVISE

- Verified: after the backend installed a newly published descriptor, a cached pre-download filelist could immediately offer the old observed version again.
- Resolution: invalidate the observation after successful metadata persistence and cover the applied-update cache transition.
- Wire contract: OK. Bug-class sweep found no other artifact-version provenance sites.

### R9 Sol follow-up — REVISE

- Verified low finding: the registered legacy batch command invalidated the cache when every item failed and `completed` was empty.
- Resolution: cache invalidation is conditional on at least one applied update; a zero-success regression proves the prior observation remains cached.
- Wire contract: OK. No further Sol review was run, per the single-follow-up rule.
### 2026-08-26 — Codex (R7)

- Active branch: `fix/audit-r7-build-evidence-bound`, stacked on `fix/audit-w1-worker-consistency`.
- Failing-before evidence: after preflight captured the vetted prefix length, appending a second player caused unbounded build-evidence extraction to return two players instead of the uploaded prefix's one.
- Implemented D-R7-1: one-shot native evidence now reads through `scanned_len` only, using a byte-limited reader; the separate live-stream evidence path remains unchanged because it has no one-shot preflight bound.
- Verification: append-after-scan/UTF-8 regression passes; build-evidence suite 16/16; root check passes; frontend tests 490/490; Rust clippy-fix, formatting, strict clippy, 808 tests (17 ignored), and fmt-check pass. Slint gates were not applicable because no shared or Slint module changed.
- Sol: `APPROVE`; no findings or missing tests, wire contract OK, bug-class sweep clean.
- Handoff: pushed commit `5dd53bd5` and opened draft stacked PR [#372](https://github.com/ESO-Toolkit/kalpa/pull/372). No merge or deployment was performed.
### 2026-08-26 — Codex (F5)

- Active branch: `fix/audit-f5-controlled-state-veto`, rebased onto `main` after W1 landed.
- Completed: reproduced the controlled-state divergence with failing-first hook and dialog tests; implemented the minimum fix so controlled mode renders only the supplied value and uncontrolled mode alone mutates internal state; added controlled-veto and uncontrolled dialog regressions.
- Evidence: before the fix, the hook rendered `requested-value` instead of the vetoing parent's `parent-value`, and the dialog context changed from `open` to `closed`. After the fix, the focused suite passes 10/10, `npm run check` passes, and `npm test` passes 593/593.
- Review: Sol `APPROVE` with no findings or missing tests, wire contract `OK`, and a clean bug-class sweep across dialog, checkbox, tooltip, and popover.
- Handoff: pushed the branch and opened draft PR [#370](https://github.com/ESO-Toolkit/kalpa/pull/370). No persisted-data, wire-format, dependency, or visual changes.
- Blockers: none known; CI and maintainer review remain.
- Exact next action: confirm the rebased diff contains only F5 and wait for green CI before marking it ready.

### 2026-08-27 — Codex W3 revalidation

- Merged updated W2 commit `4b2c18d0` forward with a normal merge. The authority-route conflict was resolved additively: W3's bounded streaming reader now protects W2's exact `authority` and `unowned_d1_ids` validation; D-W2-2 and W3 tracker history were both preserved.
- Marked W3 blocked after its two verified Sol `REVISE` rounds and reconsulted Fable with both reviews, the final diff/tests, D-W2-2 dependency, and separate H3 package-version boundary. Fable accepted Candidate A and reaccepted D-W3-1.
- Failing-before revalidation reproduced an escaped rejected body stream and missing explicit viewer cache metadata. Fixed by catching stream aborts as invalid JSON, decoding only the fully assembled bounded byte buffer, emitting explicit `public, max-age=0` for viewer-bearing list/detail responses, and varying all CORS responses on `Origin, Authorization`.
- Focused body/list/CORS tests pass 146/146; full Worker tests pass 230/230; Worker check, Wrangler dry-run, name guard, and `git diff --check` pass. Repository search found no in-repo consumer of removed health detail fields; the deploy workflow reads only `status` and `kv`. No real deployment, merge, authority flip, schema change, or H3 version decision occurred.
- Fresh Sol review: `REVISE`. It verified that list and detail routes chose their anonymous public TTL from failed `viewerId` resolution rather than the presence of an authentication attempt, so a transient ESO Logs failure could cache a redacted/no-vote fallback for a bearer token after recovery. Fixed by permitting anonymous TTLs only when the request has no `Authorization` header. Added both regressions plus the requested oversized W2 adjudication/no-authority-mutation case; full Worker tests now pass 233/233 and all Worker gates remain green.
- Sol follow-up after those corrections: `APPROVE`. Findings: none. Missing tests: none. Wire contract: OK. Bug-class propagation sweep: CLEAN. W3 is technically ready on its refreshed W2 base but remains draft; no deployment or merge was performed.
### 2026-08-26 — Codex H3

- Active branch: `fix/audit-h3-worker-version-policy`, stacked on W3.
- Evidence and decision: the Worker is private and independently auto-deployed on Worker-path changes; desktop releases are tag-triggered, and the root release/version gate intentionally owns only six desktop fields. Repository history synchronized the Worker through beta.1 but left it unchanged for seventeen later desktop releases. No runtime, release artifact, Wrangler configuration, or health check consumes the Worker package version. Chose D-H3-1 rather than forced desktop parity.
- Failing-before evidence: after adding the exact policy gate, `npm run check:version-policy` reported stale `0.1.0-beta.1` values in Worker `package.json`, the lockfile top level, and `packages[""]`.
- Implemented: used `npm version 0.0.0 --no-git-tag-version` to update all npm-owned fields and added the policy check to Worker `npm run check`, which is already invoked by both PR CI and `deploy-worker.yml` before deployment.
- Sol review: APPROVE, with no findings or missing tests; wire contract OK and the propagated package/version, lock, CI, deploy, release, changelog, and runtime sweep was clean. No follow-up was required.
- Tests: Worker check, production dependency audit, 218/218 tests, and Wrangler dry-run pass; root check, 490/490 tests, and all six desktop version fields pass. Worker name remains `kalpa-pack-hub`; no runtime code, schema, real deployment, release, or merge occurred.
- Handoff: pushed the branch and opened draft stacked PR [#386](https://github.com/ESO-Toolkit/kalpa/pull/386) targeting `fix/audit-w3-worker-hardening`.

### 2026-08-26 — Codex W3

- Active branch: `fix/audit-w3-worker-hardening`, stacked on W2.
- Failing-before evidence: focused regressions failed for UTF-8 byte limits/stream cancellation, bounded memo eviction, stale auth repopulation, atomic install claiming, health disclosure, and oversized admin restore input before the implementation existed.
- Implemented: incremental byte-bounded JSON reads across every Worker body route; bounded oldest eviction for vote/auth memos; auth-cache sequencing; transactional and time-bounded install idempotency; removal of isolate-unsafe manual list caching; awaited Worker background writes; and minimal public health output.
- Initial Sol review: REVISE. Verified cross-isolate cache invalidation, aliased list wire results, missed seed/adopt invalidation, non-atomic install state, durable IP-derived retention, and legacy limiter rollout gaps. Addressed by removing manual list storage, using one DO transaction plus retry healing, keyed HMAC identities with alarm/deletion cleanup, exact 5,001st-slot coverage, and honoring legacy limiter keys until expiry.
- Sol follow-up: REVISE. It reported public caching of personalized lists and non-atomic alarm scheduling from the snapshot it had read; both had already been corrected during the review by disabling authenticated list caching and moving alarm creation into the claim transaction. Its remaining verified legacy-limiter lifecycle finding was addressed by consulting canonical DO state, honoring a legacy key only when KV and DO lifecycles match, and adding tombstone/recreated-slug regressions. The review limit is exhausted, so no third review was requested.
- Tests: focused 107/107 and full 218/218 pass, including personalized-list cache headers, transactional alarm persistence, multi-page claim expiry, and both legacy lifecycle cases; Worker check and Wrangler dry-run pass. Directly faulting a DO output-gate KV write makes Miniflare mark the object broken, so the alarm-before-mirror ordering is objectively covered by the single storage transaction plus persisted-alarm assertion rather than an artificial caught-failure test. Worker name remains `kalpa-pack-hub`; no schema change, real deployment, merge, or authority flip was performed.
- Handoff: pushed the branch and opened draft stacked PR [#379](https://github.com/ESO-Toolkit/kalpa/pull/379) targeting `fix/audit-w2-d1-reconciliation`. No merge or deployment was performed.

### 2026-08-26 — Codex W2

- Active branch: `fix/audit-w2-d1-reconciliation`, stacked on W1.
- Failing-before evidence: the focused regression suite failed to import the intentionally absent `src/d1-reconcile.ts` module.
- Implemented: durable inline mirror breadcrumbs; canonical DO reconciliation state with tombstones; explicit-only mode handling; authority/D1 read fail-closed behavior; ownership-gated deterministic planning; mutation and ratio caps; idempotent upsert/tag/delete apply order; per-delete authority recheck; independent scheduled invocation; dry-run production default.
- Sol review: REVISE. Verified that unknown authority statuses could be interpreted as deletion and tag-only zombies were omitted. Fixed with full authority-shape/status validation and tag-ID planning. Added all requested boundary-plus-one cap tests. A further executor sweep moved the final liveness check plus D1 deletion into the DO lifecycle gate so slug reuse cannot cross a check-then-delete window.
- Sol follow-up: REVISE. Verified that planned upserts/tag replacements could cross delete-and-recreate outside the DO gate, and draft tag-only orphans were skipped. Addressed by routing reconciliation writes through a created-at lifecycle check inside the DO, moving ordinary create/update/vote D1 writes under the same serialized lifecycle, and planning draft tag-only deletion. Added exact-cap acceptance cases and stale-lifecycle regressions. The prompt permits one follow-up, so no third review was requested.
- Tests: focused missing-pack restore, owned/unowned pack and tag-only zombies, valid empty authority, malformed/failed authority, D1 read failure, mode, both sides of every safety cap, partial failure breadcrumb, draft removal, stale slug lifecycle, inline breadcrumb, and SQL-table whitelist coverage. Worker check, 197 tests, and Wrangler dry-run pass after reconstructing and rerunning from scratch following an ENOSPC interruption; Worker name remains `kalpa-pack-hub` and no real deploy ran.
- Handoff: pushed the branch and opened draft stacked PR [#378](https://github.com/ESO-Toolkit/kalpa/pull/378) targeting `fix/audit-w1-worker-consistency`. Deletion-capable reconciliation must not merge without explicit maintainer approval. Exact next action is maintainer review of the dry-run plan and ownership/limit evidence; enabling exact `apply` remains a separate later approval after soak.
- Compliance revalidation: W2 was marked blocked because two verified Sol `REVISE` rounds require architectural reassessment. Fable accepted corrected D-W2-1 and required D-W2-2: suppress all deletes until W1 reports DO authority, cap both shared D1 reads at ceiling plus one, and single-flight scheduled runs with a token-owned expiring DO lease. Three focused regressions failed before implementation and now pass; pack/tag ceiling and real-DO lease/authority tests were added. Worker check, 203 tests, and Wrangler dry-run pass; fresh Sol review is pending. The PR remains draft and both deletion-capable merge and later `apply` require explicit maintainer approval.
- Fresh Sol review after Fable: REVISE. Verified that unowned rows in the shared D1 `packs` table could permanently block W1's authority flip, malformed authority fields beyond status could reach D1, and restore's inline upsert retained stale `author_id`. Added an explicit `unowned_d1_ids` operator adjudication limited to D1 witnesses, reused full request validation plus persisted-field invariants for authority, and made restore conflict updates replace `author_id`. Focused endpoint/unit regressions pass; Worker check, 208 tests, and Wrangler dry-run pass. The single permitted Sol follow-up is pending.
- Sol follow-up after the three corrections: APPROVE. Findings: none. Missing tests: none. Wire contract: OK. Bug-class propagation sweep: CLEAN. W2 is technically ready on its W1 base, but PR #378 remains draft and unmerged pending explicit maintainer approval for deletion-capable code; exact `apply` remains a separate approval after a dry-run soak and plan inspection.

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

### 2026-08-26 — Codex (F2)

- Active branch: `fix/audit-f2-log-directory-sequencing`, stacked on `fix/audit-w1-worker-consistency`.
- Completed: captured the failing A → B reverse-resolution case; implemented D-F2-1; added stale success/rejection, late detection success/rejection, directory-change failure, and deferred-import regressions. Focused tests pass 6/6; `npm run check` passes; full frontend suite passes 38 files/496 tests.
- Sol: initial `REVISE` verified stale selection on detected directory changes, late detection reclaiming a manual folder, and deferred import selecting an old path. The required single follow-up `REVISE` verified stale detection-error toasts in initial and refresh catches. Every finding was reproduced in code and addressed; wire contract remained `OK`.
- Handoff: pushed the branch and opened draft PR [#373](https://github.com/ESO-Toolkit/kalpa/pull/373). No deployment, merge, dependency, IPC, or persisted-data change occurred.
- Blockers: none. PR remains draft and must follow its stacked W1 base.
- Exact next action: wait for stacked-base availability and green PR CI, then mark PR #373 ready for maintainer review.

### 2026-08-26 — Codex (F3)

- Active branch: `fix/audit-f3-fresh-import-metadata`, stacked on `fix/audit-f2-log-directory-sequencing`.
- Completed: captured two failing-before regressions proving prior-render metadata controlled import selection; implemented D-F3-1 so the imported path is selected from the exact guarded refresh result; kept large imports on the deferred/full-scan route and small imports on immediate preflight.
- Tests: focused uploader sequencing suite passes 8/8; `npm run check` passes; full frontend suite passes 38 files/498 tests.
- Sol: `APPROVE`; no findings or missing tests, wire contract `OK`, bug-class sweep `CLEAN`. The read-only reviewer could not start Vitest because its sandbox denied process spawn, while executor-run focused and full suites passed.
- Handoff: pushed the branch and opened draft PR [#375](https://github.com/ESO-Toolkit/kalpa/pull/375) against the F2 branch. No deployment, merge, dependency, IPC, or persisted-data change occurred.
- Blockers: none in F3. The PR remains stacked and must follow F2.
- Exact next action: after F2 lands, retarget PR #375 to `main`, require the resulting GitHub CI to pass, then mark it ready for maintainer review.

### Sol review 1 — REVISE

Verified findings:

1. First post-deploy bootstrap can permanently omit a live pack when KV returns a stale index.
2. A stale vote/install request can cross delete-and-recreate and mutate the new pack lifecycle.
3. Update/delete authorization still trusts stale KV ownership rather than canonical DO ownership.
4. Restore finalization preserves concurrent packs from KV instead of atomically from DO authority.
5. Account deletion leaves other users' vote records attached to removed pack IDs, so a reused ID can inherit votes.

Wire contract verdict: OK. Bug-class sweep found the restore and account-deletion sites above.

### 2026-08-26 — R8 Protected Edits disclosure

- Active branch: `fix/audit-r8-protected-edits-disclosure`, stacked on `fix/audit-w1-worker-consistency`.
- Decision: D-R8-1 selects honest, specific, non-blocking disclosure and explicitly excludes baseline seeding from migrated files.
- Implementation: added defaulted baseline-presence fields to installed-addon and conflict-report contracts; rejected corrupt/mismatched manifests; added persistent React and Slint warning hierarchy; ordered automatic updates after coverage scans; refreshed React batch and native single/batch coverage at action time; routed the context shortcut through fresh detail preflight; removed native “safe” claims.
- Sol initial: `REVISE`. Verified the context-menu legacy bypass, launch auto-update race, ignored fresh single-update report, and corrupt-manifest validity. All four were addressed.
- Sol follow-up: `REVISE`. Verified remaining first-run ordering, cached action-time coverage, and dishonest native copy. All in-scope disclosure findings were addressed. The requested broader disk-only baseline redesign and dormant native auto-update preference wiring were excluded from R8 rather than silently expanding scope.
- Luna initial: `FAIL`. Verified that Review Update was pointer-only and that a late coverage scan could publish the old instance after a path switch. Added Context Menu / Shift+F10 access, focused menu active-descendant behavior, focus restoration, and generation/path-guarded reverse-resolution handling.
- Luna follow-up: `PASS`; `TOKEN_VIOLATIONS`, `ACCESSIBILITY`, `STATE_FEEDBACK`, and `RESPONSIVE_BEHAVIOR` all reported none.
- Evidence: `npm run check`; 40 frontend files / 497 tests; main Rust 809 passed / 17 ignored; Slint 757 passed / 15 ignored; main and Slint clippy with warnings denied; both fmt checks; native Slint release sidecar build; `git diff --check`.
- Handoff: pushed the branch and opened draft stacked PR [#381](https://github.com/ESO-Toolkit/kalpa/pull/381). No merge was performed.

### 2026-08-26 — Codex F6

- Active branch: `fix/audit-f6-optimistic-sequencing` in isolated worktree `Kalpa-wt-f6`, stacked on `fix/audit-w1-worker-consistency`.
- Inventory: `Settings` mount hydration and direct optimistic persistence controls; `Packs` `installed_packs` hydration/install/removal; `SavedVariables.loadFiles` refresh application.
- Failing-before evidence: focused Vitest run failed both new suites because the sequencing hooks did not exist. The tests cover late hydration after user intent, reverse-settled quick toggles, confirmed rollback after a newer failure, functional array rollback, reverse-settled refresh results, and stale refresh errors.
- Passing-after evidence: focused 6/6 tests, full frontend 496/496 tests, and `npm run check` pass.
- Decision: D-F6-1. No wire-format, persisted-shape, backend, Rust, Worker, or dependency changes.
- Luna review: `PASS`. `TOKEN_VIOLATIONS: None`; `ACCESSIBILITY: None`; `STATE_FEEDBACK: None`; `RESPONSIVE_BEHAVIOR: None`.
- Sol review: `REVISE`. Verified that `list_characters` still ran outside the latest-request/unmount guard and could apply path-A character state after a path-B switch. Addressed with an independent latest/unmount request gate. Added the requested unmount, late-hydration failure rollback, and pre-settlement functional-array composition tests. Focused 9/9 tests, full frontend 499/499 tests, and `npm run check` pass.
- Sol follow-up: `REVISE` with no implementation findings, `WIRE_CONTRACT: OK`, and `BUG_CLASS_SWEEP: CLEAN`. It requested a rejected-after-unmount assertion and a late hydration value distinct from the constructor default. Both tests were strengthened; focused 10/10 tests, full frontend 500/500 tests, and `npm run check` pass. The one prescribed follow-up review is complete.
- Handoff: pushed the branch and opened draft PR [#377](https://github.com/ESO-Toolkit/kalpa/pull/377), stacked on `fix/audit-w1-worker-consistency`. The CI workflow listens only to PRs targeting `main`, so this stacked draft has no remote checks until it is retargeted after W1 merges; local frontend gates are green. Do not merge the stack out of order.

## Open Questions

- W2: maintainer approval is required before merging any reconciliation path that can delete rows from shared D1.
- W1: decide whether moving vote-record authority from KV/in-memory memo into DO storage belongs in W1 or W3. The current W1 code prevents resurrection but retains the pre-existing eviction/double-toggle limitation for later hardening.
- W1: owner sign-off is required before the later manual `kv` → `do` authority flip and must accept backup restore as the post-flip rollback path.
- P0-A2: D-P0-1 establishes the lock invariants; a fresh Fable consultation must choose the concrete cross-platform dependency/API and user-visible timeout behavior before implementation.
- R4/R5: ownership/conflict behavior that changes install outcomes requires explicit design review before implementation.
- H2: `kalpa-elder-scrolls-themes/` is currently untracked; provenance must be inspected before tracking or ignoring it.
