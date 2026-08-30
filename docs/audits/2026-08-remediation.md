# August 2026 Audit Remediation Tracker

This file is the durable execution record for `2026-08-remediation-master-prompt.md`.

| ID | Status | Branch | PR | Merged SHA | Fable decision | Sol verdict | Tests | Notes |
|---|---|---|---|---|---|---|---|---|
| W1 | pr-open | `fix/audit-w1-worker-consistency` | [#369](https://github.com/ESO-Toolkit/kalpa/pull/369) (draft) | - | D-W1-1, D-W1-2, D-W1-3 | REVISE; follow-up REVISE; all verified findings addressed | Worker check; 184 tests; Wrangler dry-run; name guard | Fable twice-reject redesign implemented; awaiting refreshed CI/maintainer sign-off. |
| W2 | pr-open | `fix/audit-w2-d1-reconciliation` | [#378](https://github.com/ESO-Toolkit/kalpa/pull/378) (draft) | - | D-W2-1, D-W2-2 | Fresh REVISE; follow-up APPROVE | Worker check; 208 tests; Wrangler dry-run | Technically ready and stacked on W1. Draft/merge hold remains: deletion-capable code and later `apply` each require separate maintainer approval. |
| W3 | pr-open | `fix/audit-w3-worker-hardening` | [#379](https://github.com/ESO-Toolkit/kalpa/pull/379) (draft) | - | D-W3-1 reaccepted after reconsultation | Fresh REVISE; follow-up APPROVE | Worker check; 233 tests; Wrangler dry-run | Technically ready and stacked on refreshed W2/D-W2-2; remains draft with no real deployment. |
| P0-A1 | pr-open | `fix/audit-p0-a1-atomic-writer` | [#380](https://github.com/ESO-Toolkit/kalpa/pull/380) (draft) | - | D-P0-1 | REVISE; all verified findings addressed; follow-up APPROVE | Root 490; Rust 814 passed/17 ignored; Slint 760 passed/15 ignored; native build; Tauri build; sandbox 3/3 | Shared crash-safe atomic writer; stacked on W1. |
| P0-A2 | pr-open | `fix/audit-p0-a2-cross-process-locking` | [#388](https://github.com/ESO-Toolkit/kalpa/pull/388) (draft) | - | D-P0-A2 | REVISE; verified finding fixed; follow-up APPROVE | Root 490; Rust 828 passed/18 ignored; Slint 777 passed/16 ignored; native build; Tauri build; sandbox 3/3 | Shared bounded cross-process RMW locking; stacked on P0-A1. |
| P0-A3 | pr-open | `fix/audit-p0-a3-sidecar-handshake` | [#389](https://github.com/ESO-Toolkit/kalpa/pull/389) (ready) | - | D-P0-1; D-P0-A3-FINAL; D-P0-A3-CLASSIFY; D-P0-A3-ACTIVATION; D-P0-A3-FATAL; Fable `SHIP: YES` | REVISE; all three findings fixed (2 and 3 after reversing an earlier refutation) | Root 490; Rust 859 passed/18 ignored; Slint 805 passed/16 ignored; strict clippy + fmt clean both crates; native sidecar build; **sandbox e2e 3/3**; 5/5 real-binary Windows handshake scenarios | All gates green, no outstanding work. Pushed (force-with-lease, maintainer-authorised, after the other session's rebase). Stacked on P0-A2 — merge in dependency order. |
| R4 | review-approved | `fix/audit-r4-sibling-ownership` | [#392](https://github.com/ESO-Toolkit/kalpa/pull/392) (draft) | - | D-R4-1 | APPROVE | Rust 820 passed/17 ignored; Slint 766 passed/15 ignored; clippy `-D warnings` both crates; fmt --check both; `npm run check`; native build | Implements D-R4-1. Removal now preserves successful disk deletion while reporting post-delete cleanup warnings, and Slint retains only folders that actually failed removal. **R5 sequences after this.** |
| R5 | pr-open | `fix/audit-r5-folder-qualified-conflicts` | [#393](https://github.com/ESO-Toolkit/kalpa/pull/393) (draft) | - | D-R5-1 | APPROVE after verified and fixed follow-ups | Rust 824 passed/17 ignored; Slint 770 passed/15 ignored; frontend `npm run check` + 491 vitest; clippy `-D warnings` both crates; fmt --check both; native build | Implements D-R5-1. Stacked on R4. Conflict wire **values** are folder-qualified (field names unchanged). |
| R6 | pr-open | `fix/audit-r6-crash-safe-installer` | [#399](https://github.com/ESO-Toolkit/kalpa/pull/399) (draft) | - | D-R6-1 | REVISE; all verified findings addressed; follow-up APPROVE | Root 490; Rust 840 passed/18 ignored; strict clippy both crates; native build; transaction 10; cross-instance copy 3; focused editor/restore tests | Crash-safe merged staging, folder swaps, hash promotion, and deterministic recovery; stacked on P0-A2. Slint test link and destructive sandbox have environment blockers recorded below. |
| R7 | pr-open | `fix/audit-r7-build-evidence-bound` | [#372](https://github.com/ESO-Toolkit/kalpa/pull/372) (draft) | - | - | APPROVE | Focused 1; evidence 16; frontend 490; Rust 808 | D-R7-1: one-shot evidence uses the encoder's exact `scanned_len` byte bound; stacked on W1. |
| R8 | pr-open | `fix/audit-r8-protected-edits-disclosure` | [#381](https://github.com/ESO-Toolkit/kalpa/pull/381) (draft) | - | D-R8-1 | REVISE; all in-scope findings addressed | Frontend check; 497 tests; Rust 809/17 ignored; Slint 757/15 ignored; clippy/fmt; native release build; Luna PASS | Non-blocking disclosure for absent/invalid baselines across React and shipped Slint; stacked on W1. |
| R9 | pr-open | `fix/audit-r9-downloaded-version` | [#376](https://github.com/ESO-Toolkit/kalpa/pull/376) (draft, stacked) | - | not required | REVISE → REVISE; all verified findings addressed | 810 Rust; 490 frontend; clippy/fmt/check green | Persist checksum-bound filedetails version and invalidate stale update observations only after an applied update. |
| F1 | todo | - | - | - | - | - | - | Import-source sequencing. |
| F2 | pr-open | `fix/audit-f2-log-directory-sequencing` | [#373](https://github.com/ESO-Toolkit/kalpa/pull/373) (draft) | - | D-F2-1 | REVISE x2; all verified findings addressed | Frontend check; 496 tests | Stacked on W1; no wire or persisted-data changes. |
| F3 | pr-open | `fix/audit-f3-fresh-import-metadata` | [#375](https://github.com/ESO-Toolkit/kalpa/pull/375) (draft) | - | D-F3-1 | APPROVE | Frontend check; 498 tests | Stacked on F2; no wire or persisted-data changes. |
| F4 | pr-open | `fix/audit-f4-logout-invalidation` | [#374](https://github.com/ESO-Toolkit/kalpa/pull/374) (draft, stacked on F1) | - | D-F4-1 | APPROVE | Focused 1 test; pack sequencing 4 tests; frontend check; 494 tests | Successful logout invalidates every private-list page request before clearing signed-in state; no wire/persisted-data change. |
| F5 | pr-open | `fix/audit-f5-controlled-state-veto` | [#370](https://github.com/ESO-Toolkit/kalpa/pull/370) (draft, stacked on W1) | - | not required | APPROVE | Focused 10/10; frontend check; 493 tests | Controlled values remain authoritative when a parent vetoes a change; uncontrolled behavior is preserved. |
| F6 | pr-open | `fix/audit-f6-optimistic-sequencing` | [#377](https://github.com/ESO-Toolkit/kalpa/pull/377) (draft, stacked) | - | D-F6-1; Luna PASS | REVISE follow-up; no findings, requested tests addressed | Frontend check; 500 tests | Sequenced optimistic settings/library state and latest-only SavedVariables file/character refreshes implemented. |
| H1 | pr-open | `fix/audit-h1-release-copy` | [#384](https://github.com/ESO-Toolkit/kalpa/pull/384) (draft, stacked on W1) | - | D-H1-1 | REVISE follow-up; all verified findings addressed | Release/Discord 16; frontend 490; check; versions | Generates exact tagged CHANGELOG copy; no tag, release, merge, or deployment. |
| H2 | pr-open | `fix/audit-h2-theme-provenance` | [#383](https://github.com/ESO-Toolkit/kalpa/pull/383) | - | not required | REVISE x2; all verified findings addressed | Root check; frontend check; 490 tests | Evidence supports treating the directory as local visual-review output; ignore narrowly without modifying it. |
| H3 | pr-open | `fix/audit-h3-worker-version-policy` | [#386](https://github.com/ESO-Toolkit/kalpa/pull/386) (draft) | - | D-H3-1 | APPROVE | Worker policy/check; 218 tests; root check/490 tests/version gate; Wrangler dry-run | Stacked on W3; independent Worker uses sentinel `0.0.0`; no real deployment. |
| H5 | pr-open | `fix/audit-h5-branch-pruning-proposal` | [#385](https://github.com/ESO-Toolkit/kalpa/pull/385) | - | D-H5-1 | APPROVE after one follow-up; refreshed 2026-08-28 | `npm run check`; `git diff --check`; freshness guard | Point-in-time proposal; PR now targets `main` directly; no branches deleted. |
| H6 | pr-open | `fix/audit-h6-quick-xml-advisory` | [#387](https://github.com/ESO-Toolkit/kalpa/pull/387) (draft, stacked on W1) | - | D-H6-1 | Initial REVISE resolved; follow-up no findings | Main audit clean; main/Slint clippy, test, fmt, native build green | Compatible upstream lock updates remove quick-xml advisories; CI ignores removed. |

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

### D-H1-1 — CHANGELOG is the release-copy authority

- Chosen: a dependency-free Node generator selects the exact tagged `CHANGELOG.md` section, fails closed for missing, empty, malformed, or duplicate targets, and supplies the complete established release body to `tauri-action` through multiline `GITHUB_OUTPUT`. `Unreleased` is accepted only when explicitly requested for local preview/testing.
- Markdown compatibility: content-owned trailing reference definitions are retained using CommonMark-style case-insensitive, whitespace-normalized labels; unrelated global changelog definitions are excluded. The existing `## Changed:` shape remains compatible with the Discord announcement helper.
- Rejected: leaving per-release prose in shared workflow YAML, because it can silently publish the previous release's text. Rejected: permissive fallback to `Unreleased` or generic copy for a missing tagged section, because a release should stop rather than publish unmatched notes.
- Scope: install, verification, known-issues, draft/publish, updater-manifest, application runtime, and Discord delivery behavior are unchanged. Rollback is a normal commit revert to the prior hand-maintained YAML body.

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

### D-P0-A3-FATAL — Losing UI authority is fatal, and "lost" must mean the whole window elapsed

- Context: the third and final Fable consultation (`docs/audits/consultations/p0-a3-authority-loss-fable.md`) reviewed the last change before merge and returned `SHIP: YES`. This decision records that change and the two follow-ups Fable asked for in the same PR.
- **Why fatal rather than "silent but tolerable".** Sol's findings 2 and 3 were initially recorded under D-P0-A3-ACTIVATION as *not* producing two writers. That was wrong, and re-analysis (confirmed by Fable) shows the full chain: a process that released authority and failed to reclaim stays visible and keeps writing; it holds no lock, so `live_native_authority_exists` is false for everyone; the next launch of `kalpa.exe` therefore sees no live owner, no pending marker and `performanceMode` still native, and **spawns a sidecar**; that sidecar finds the lock free and claims it. Two independent writers, reachable with no crash anywhere. On the Slint side there is a second mechanism: the shutdown timer reads the launch ID out of `native_authority()`, so an unauthoritative shell is also permanently deaf to `request_active_shutdown` and would never yield.
- Chosen: both handoff directions now follow the rule the forward-handoff child already followed — a shell that cannot hold UI authority stops rendering. The WebView sets `WEBVIEW_AUTHORITY_LOST` and the command calls `app.exit(1)`; the Slint shell sets `NATIVE_AUTHORITY_LOST` and calls `quit_event_loop`. Exit is the only state that is provably not a writer.
- Rejected (Fable): "reveal a modal and refuse writes". Every write site in both processes would need a gate, and missing one is the lock-less writer with extra steps; it also leaves the lock free, so the next launch still spawns a sidecar to sit alongside the "read-only" window. Rejected: retrying the reclaim on a timer after the window — that is a visible lock-less WebView polling next to an unknown owner, i.e. the exact state being removed.
- **Follow-up 1 (Fable DECISION 2), adopted.** Making failure fatal is only sound if failure means the whole window really elapsed, and it did not: `claim_webview_after_shutdown` and `claim_after_ready_release` both did `try_claim_authority(...)?`, which returns `Err` on `create_dir_all` / `open` / `TryLockError::Error` / `write_record` failure. Any one aborted the loop on the **first** poll with no retry and no shutdown re-request — so a transient sharing violation from a virus scanner touching app-data was enough to exit the user's window. Both loops now retry an IO error to the deadline and report the last error only when it expires; `request_active_shutdown` failures are retried the same way. Pinned by `a_transient_claim_error_is_retried_until_the_deadline`, which obstructs the lock path with a directory, clears it after 200ms, and asserts the claim still succeeds and took at least that long.
- **Follow-up 2 (Fable DECISION 3), adopted.** `on_settings_performance_mode_changed` persists `performanceMode=webview` *before* calling `return_to_webview_shell` and, on `Err`, wrote native back and persisted again. On the fatal path that is both a write after authority loss and the worse recovery value: leaving it on webview means the next launch boots the WebView, whose startup `claim_webview_authority` re-requests shutdown of whatever holds the lock — the strongest recovery path available. Restoring native means the next launch spawns a sidecar that sees the mystery holder and exits as a duplicate, showing the user nothing. The caller now skips the restore when `native_authority_was_lost()`. The other three `return_to_webview_shell` callers only set a status message and needed no change.
- **Withdrawn by Fable.** Its earlier item 3 (startup path should accept `acquired_matches || existing_native_ready`) was refuted here and Fable confirmed the refutation: `A || B` accepts strictly more than `B`, and `native-boot.acquired` survives its publisher's death, so it would newly admit exactly the case `a_crashed_child_that_published_ready_is_not_a_live_owner` exists to reject. The lock remains the sole positive proof.

### D-R4-1 — Separately tracked siblings keep their identity; parents record provenance

- Full record: `docs/audits/consultations/r4-fable.md` and `r4-fable-decision.md`.
- Chosen: add `AddonMetadata.bundled_by: Vec<u32>` (mirrors the existing `HashManifest.esoui_ids` precedent, `#[serde(default)]` so old files round-trip) and a new `metadata::record_bundled_folder` primitive holding the ownership rule. Because `metadata.rs` is `#[path]`-included by Slint, the rule itself is written once.
  - If an existing entry has a nonzero `esoui_id` that differs from the installing parent, it is **separately tracked**: keep `esoui_id`, `download_url`, `esoui_last_update` and `tags`, add the parent to `bundled_by`, and set `installed_version` from the on-disk manifest. The files really were overwritten, so metadata must state what is actually installed — this is the master prompt's "do not blindly preserve stale version metadata" clause.
  - Otherwise the folder is **genuinely bundled**: `esoui_id = 0`, parent URL, `bundled_by = {parent}`.
- Update checks keep their existing `esoui_id == 0 -> skip` condition, so a separately tracked sibling stays checked. A bundled-older sibling now reports the older version and its own update offers the upgrade back: a visible, user-fixable downgrade instead of a silent one.
- Determinism fix: `installer.rs` returns its folder set **sorted**, and `determine_primary_folder` prefers a folder already tracked under this id (so an update cannot flip the primary), then exact title match, then longest containment, then sorted-first. Today the source is a `HashSet` and the fallback is `.first()`, so which folder gets demoted can differ between runs.
- Migration for users already demoted: `is_bundled_secondary` (`commands.rs:4053`) currently keys on `esoui_id == 0` plus a shared `download_url`, which makes demotion permanent. It is relaxed to also require `bundled_by.is_empty()`, plus a conservative one-time heal that re-links only when the on-disk manifest version equals the ESOUI version; anything else is surfaced in the unlinked-addons list rather than guessed.
- Rejected: preserving a nonzero id inside `record_install_ext` alone (no signal distinguishes "primary passing a real id" from "sibling passing 0"); and skipping or refusing the bundled install (many ESOUI addons legitimately vendor their libraries, so blocking installs breaks normal use).

### D-R5-1 — Folder-qualify the transport, keep per-folder storage keys

- Full record: `docs/audits/consultations/r5-fable.md` and `r5-fable-decision.md`.
- Inventory corrected the finding's shape: sibling **storage already works** (`record_hashes_with_zip_baseline` writes a `.kalpa-hashes/<Folder>.json` per folder, and the file browser already flags sibling edits). What is single-folder-scoped is the **conflict pipeline** — `build_conflict_report` takes one `folder_name`. So a modified sibling file has a baseline, is displayed as modified, and is then silently overwritten on update with no prompt and no backup.
- Chosen design C: every string that **crosses a folder boundary** (Tauri/Slint wire, `PendingUpdate`, decisions, skip set) becomes `Folder/relative/path`; every string that **lives inside a folder** (`HashManifest.files`, backup paths, classification input) stays bare. This meets the boundary that already exists — the skip-key layer is folder-qualified today and `installer.rs:425` already tolerates both shapes — so there is **zero storage migration**.
- New `hash_zip_entries_by_folder` returns a `ZipHashSet` of per-folder maps plus a single `flat_wrap` flag, and `ZipHashSet::zip_entry_name` becomes the one place the flat-archive divergence is encoded. `hash_zip_entries(zip, folder)` stays as a wrapper so existing call sites compile unchanged.
- `ConflictReport` gains `folders: Vec<String>`; field names are unchanged but `relative_path` **values** become qualified, so the frontend must land in lockstep.
- Note `file_hashes.rs:912` `hash_zip_ignores_other_folders` asserts `hashes.len() == 1` for a two-folder ZIP: it encodes the bug and must be rewritten, not preserved.

### D-R6-1 — Per-folder staged merge, tombstone swap, journal-backed recovery

- Full record: `docs/audits/consultations/r6-fable.md` and `r6-fable-decision.md`. The master prompt forbids an ad hoc directory swap without this review.
- Chosen "C-lite": stage each top-level folder as a **merge** of fresh ZIP bytes plus a residual copy of every live file the archive does not cover, then swap with a tombstone under an on-disk journal. The residual copy is exactly what keep-mine, upstream-removed and user-added files require, and peak disk is new-version + residual rather than 2x the addon.
- Everything a crash can leave lives under one `<addons_dir>/.kalpa-staging/<txn>/` directory, so scanning needs one exclusion rule and recovery needs one directory listing. The whole transaction runs under the P0-A2 lock, with recovery first under the same lock, so no new process-wide statics are introduced (they would be per-process, and the two binaries can run concurrently).
- Baselines are computed against the merged stage image and promoted **only after** a successful swap, which is what breaks the corruption chain: today a crash leaves a truncated file that the next scan records as a user edit (`file_hashes.rs:472`) and that keep-mine then blesses with the upstream hash (`file_hashes.rs:653`).
- **New issue surfaced by this review, since verified and fixed separately.** Nothing validated a ZIP's top-level folder name against the `.kalpa-` reserved prefix, so an archive whose top folder was `.kalpa-hashes` or `.kalpa-backups` wrote straight into Kalpa's own state directories. Overwriting a hash baseline is not cosmetic: the baseline decides what Protected Edits treats as a user edit, so a forged one can make Kalpa bless changes the user never made. Reproduced with a failing-first test against `main` and fixed in [#391](https://github.com/ESO-Toolkit/kalpa/pull/391). **#391 must merge before this design record (#390)**; R6 therefore inherits the guard rather than adding it.
- Supersedes `installer.rs:951` `cancel_midway_preserves_pre_existing_addon_files`, which currently asserts the in-place semantics ("no file is removed, only some are overwritten") as a requirement.

## Session Log

### 2026-08-28 — R4 removal-state review follow-up

- A failed metadata/hash cleanup no longer converts an already successful on-disk removal into a failed removal. Cleanup is attempted comprehensively, metadata is persisted before hash cleanup, and warnings are returned through the Tauri wire contract and surfaced by the frontend.
- Slint multi-folder removal now updates each successfully removed folder immediately, retains failed folders, and reports the exact failures instead of presenting an all-or-nothing result.
- Added deterministic injected-operation regressions for partial removal and cleanup failures.
- Gates: Rust 820 passed/17 ignored; Slint 766 passed/15 ignored; `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean on both crates; `npm run check` and `git diff --check` pass.
- Mandatory local Sol review (`gpt-5.5`, `xhigh`, read-only) returned exact `VERDICT: APPROVE`.
### 2026-08-28 — R5 final revalidation and approval

- Apply-time revalidation now refuses a conflict that appears after the prompt was opened, including a newly conflicting bundled sibling, in both the Tauri and Slint implementations. The user must review the refreshed conflict set rather than having a new edit overwritten under stale decisions.
- The Tauri command lifecycle retains the pending session and downloaded ZIP specifically for this conflict-changed refusal, so the user can cancel the old panel or retry from a refreshed scan. Other apply errors keep their existing cleanup behavior.
- The Slint classifier now treats a user-added sibling file that is newly shipped upstream as a conflict instead of unmodified content.
- Frontend regression coverage pins folder-qualified diff requests and independent decisions for same-named files in different folders. Auto-kept paths are no longer sent as synthetic `keep_mine` decisions, so an auto-kept file that becomes conflicted while the prompt is open cannot be masked from backend revalidation.
- Final gates: Rust 824 passed/17 ignored; Slint 770 passed/15 ignored; all 491 frontend tests pass; `npm run check`, clippy `-D warnings`, and fmt checks are clean for both Rust crates; the production native sidecar build passes.
- Sol's first follow-up found the pending-session cleanup bug and its next completed review found the synthetic auto-kept decision bug. Both were verified and fixed. The final genuine `gpt-5.5`/xhigh review found no actionable issues and returned `APPROVE`.

### 2026-08-28 — R5 stale-decision revalidation follow-up

- Apply now recomputes the archive's current conflict set and honors cached `keep_mine`/`take_update` decisions only while the named file remains conflicted. A file reverted to its baseline while the prompt is open therefore receives the update instead of being skipped and incorrectly blessed with the upstream baseline.
- Added `native_pending_conflict_apply_ignores_stale_keep_mine_after_revert`; all three native pending-conflict apply regressions pass.
- Merged the completed R4 ownership branch into the R5 stack before final verification.
- Gates re-run: Rust 823 passed/17 ignored; Slint 768 passed/15 ignored; clippy `-D warnings` clean both crates; fmt clean both; native Slint sidecar build passed. The first unconstrained Slint link exhausted system memory; the serialized shared-target retry passed with zero test failures.

### 2026-08-27 — Sol review of R5

- Codex quota recovered, so Sol reviewed R5. Verdict `REVISE`, two findings, both verified against the code before adoption and both fixed.
- **Finding 1 (medium, confirmed).** `native_pending_conflict_diff_blocking` still treated the now-qualified conflict path as bare, so a sibling conflict resolved to `AddOns/<primary>/LibFoo/init.lua` and looked for `<primary>/LibFoo/init.lua` in the archive. Both paths exist nowhere: the diff showed empty local content and then failed with "file not found in update ZIP". This is the **same qualified-under-primary bug class** already found and fixed in `backup_native_conflicting_files`; Sol's propagate-search caught the site the implementation had missed. Fixed by splitting the path and using the flat-aware `zip_entry_name`.
- **Finding 2 (high, confirmed, pre-existing).** A stale `keep_mine` decision survived server-side reclassification. The user marks a conflict keep-mine, then reverts the file to its baseline before applying; reclassification says the file is safe, but the stale decision still added it to the extraction skip set, so the file stayed on the old version while the update reported success, and a hash override then recorded the upstream hash as its baseline so the divergence would never be flagged again.
  - Not introduced by R5: the same derivation exists on `fix/audit-r4-sibling-ownership`. R5 widens the surface to siblings and touches this exact code, so it was fixed here rather than deferred. Decisions are now intersected with the current conflict set, for both `keep_mine` and `take_update`.
- Both requested regressions added: `native_diff_resolves_a_sibling_conflict_under_its_own_folder` and `a_stale_keep_mine_decision_does_not_survive_reclassification`.
- `WIRE_CONTRACT: OK`. `BUG_CLASS_SWEEP` confirmed the Tauri diff, backup grouping, hash overrides and installer skip-key paths were already consistent; the sidecar diff was the one remaining site.
- Gates re-run: Rust 822 passed/17 ignored; Slint 766 passed/15 ignored; clippy `-D warnings` clean both crates; fmt clean both.

### 2026-08-27 — R5 implemented

- Implements D-R5-1. The finding needed restating before it could be fixed: sibling **storage already worked** (`record_hashes_with_zip_baseline` writes one manifest per folder, and the file browser already flagged sibling edits). What was single-folder-scoped was the **conflict pipeline** - `build_conflict_report` took one `folder_name`. So a modified file in a bundled sibling had a baseline, was displayed as modified, and was then overwritten on update with no prompt and no backup.
- Chosen split: strings that **cross a folder boundary** (wire, `PendingUpdate`, decisions, skip set) are `Folder/relative/path`; strings that **live inside a folder** (`HashManifest.files`, backup paths, classification input) stay bare. That meets the boundary that already existed - the skip-key layer was folder-qualified and `installer.rs` already tolerated both shapes - so there is **zero storage migration**.
- `hash_zip_entries_by_folder` returns a `ZipHashSet` of per-folder maps plus one `flat_wrap` flag, and `ZipHashSet::zip_entry_name` is now the single place the flat-archive divergence is encoded. `hash_zip_entries(zip, folder)` remains as a thin wrapper so existing call sites compile unchanged.
- `classify_update_files` stays per-folder and bare; the new `classify_update_archive` loops folders and qualifies the results. The apply step re-derives through it, so a sibling edited **while the user was deliberating** is caught rather than silently overwritten - the same server-side re-derive guarantee the primary already had.
- The re-derive guard is extended to the folder axis: a decision naming a folder the archive does not write is rejected outright.
- Backups are grouped per folder, each read against **that folder's** recorded version. Labelling a sibling's backup with the primary's `from` version would misreport what the user is restoring from.
- `record_hashes_with_zip_baseline` takes the whole archive set, which also removes its per-sibling ZIP re-open, and `hash_overrides` is keyed by folder.
- `get_conflict_diff` now splits the requested path instead of re-qualifying it with the pending update's primary, which is what made a sibling diff look for the file under the wrong folder on both sides.
- Sidecar mirrored. Note `backup_native_conflicting_files` **compiled fine** while being silently wrong: it passed qualified paths under the primary folder, producing paths like `MainAddon/LibFoo/init.lua` that exist nowhere, so the backup would have found nothing and the edits would have been overwritten unprotected. Fixed to group by folder.
- `ConflictReport` gains `folders: Vec<String>`. Field names are unchanged but `relative_path` **values** change meaning, so the frontend types were updated in the same change; nothing in TypeScript constructs a `ConflictReport`, it is only read from `invoke`.
- Tests: a bundled-sibling edit now produces a conflict rather than being overwritten; two folders shipping `init.lua` stay distinct; the archive-wide hash view returns every folder and ignores a loose root file; a flat archive keys under its synthesised wrap while its entry names stay unprefixed; and `qualify`/`split_qualified`/`group_by_folder` are pinned directly. Four sidecar tests and two Tauri tests were updated from bare to qualified paths - that assertion change **is** the behaviour change.
- One fixture bug worth recording: a two-folder archive whose folders carry no `<Folder>.txt` manifest, plus a root `readme.txt`, is treated as a **flat** archive and wrapped under `readme`. That is correct pre-existing behaviour of `flat_archive_wrap_name`; the first draft of the test simply built an unrealistic archive.
- Gates: Rust 821 passed/17 ignored; Slint 765 passed/15 ignored; `npm run check` clean; 490 vitest tests pass; clippy `-D warnings` clean on both crates; `cargo fmt --check` clean on both; `npm run build:native-slint` succeeds.

### 2026-08-27 — R4 implemented

- Implements D-R4-1. The ownership rule itself lives in `metadata::record_bundled_folder`, which the Slint crate shares verbatim by `#[path]`, so only the primary/bundled dispatch is duplicated between the two binaries.
- `AddonMetadata` gains `bundled_by: Vec<u32>`, `#[serde(default, skip_serializing_if = "Vec::is_empty")]`. `metadata_without_provenance_round_trips_unchanged` pins both directions: a pre-change file loads, and an empty set is not written back into the JSON.
- A separately tracked sibling keeps its ID, URL, ESOUI timestamp and tags, and gains the installing addon as provenance. `installed_version` is still refreshed from the manifest on disk, because the archive really did overwrite the files.
- `record_install_ext` clears `bundled_by` when recording under a real ID: a folder recorded as owned is not simultaneously bundled.
- Determinism: `extract_addon_zip` now returns its folder list **sorted** (it was `HashSet` order), and `determine_primary_folder` prefers a folder already recorded under this ID, then an exact case-insensitive title match, then the longest contained name, then the first entry. Previously an update whose title stopped matching could hand ownership to a bundled library.
- The `auto_link` guard that made demotion permanent now keys on provenance. Entries written since `bundled_by` exists say outright who shipped them; legacy entries keep the old ID-0-plus-shared-URL shape but are healed when the on-disk manifest version equals the ESOUI version, which means the bundled copy is the standalone release and can be relinked with no mismatch risk. Anything else is left for the user rather than guessed.
- Dependency installs route through the same rule. They previously stamped the dependency ID onto **every** extracted folder, which both overwrote separately tracked identities and made a two-folder dependency emit two identical update rows.
- `write_folder_manifest` now unions `esoui_ids` instead of replacing it, so the hash store and the metadata store cannot disagree about who owns a folder.
- Removing an addon drops only its provenance from other folders (`forget_bundled_parent`). Folders stay on disk because other addons may declare a dependency on them.
- Review follow-up prevents the primary-folder fallback from claiming a separately tracked sibling when an unowned extracted folder is available, and pins the selection in both binaries.
- Healing legacy ID-0 metadata now replaces a bundled parent's download URL with the standalone addon's URL instead of retaining the parent's identity.
- Removing an owning addon now also removes that ID from every hash sidecar manifest while preserving any remaining owners.
- **Failing-first verified after the fact, by re-running against the old behaviour.** With the sibling branch reverted to `record_install_ext(store, folder, 0, ..)`, `bundling_a_separately_tracked_library_does_not_take_it_over` and `removing_a_bundling_addon_leaves_the_library_tracked` both fail with `left: 0, right: 7` — exactly the demotion the finding describes. The inventory found **zero** existing coverage on any function R4 touches.
- Gates: Rust 817 passed/17 ignored; Slint 763 passed/15 ignored; `cargo clippy --all-targets -- -D warnings` clean on both crates; `cargo fmt --check` clean on both; `npm run build:native-slint` succeeds.

### 2026-08-28 — Codex (H5 evidence refresh)

- At that time, rebased PR #385 onto the W1 base `origin/fix/audit-w1-worker-consistency`; the PR was later retargeted to `main` after W1 merged.
- Refreshed the inventory against `origin/main` `fb92cb92` after `git fetch origin --prune`: 38 local safe, 57 local retain, 56 local review; 17 remote safe, 47 remote retain, 59 remote review (274 refs total).
- GitHub reported no protected branches; open PR heads and all attached worktrees were retained. No branch, ref, worktree, or commit was deleted or moved.
- Updated the H5 proposal and this tracker; safe candidates are 55 local/remote refs counted separately. Review refs remain explicitly non-candidates pending maintainer inspection.
- Verification at that time: `git diff --check` and `npm run check` passed; the branch was pushed with force-with-lease. GitHub then reported PR #385 open/draft, `mergeable=true`, `mergeable_state=clean`, base `16f76144`, head `4d3c57f3`; no checks were reported for the docs-only branch. These values are historical observations, not current PR metadata.

### 2026-08-26 — Codex (H1)

- Active branch: `fix/audit-h1-release-copy`, isolated worktree, stacked on `fix/audit-w1-worker-consistency`.
- Completed: inventoried release, changelog, version, and Discord workflows; captured the missing-generator failure before implementation; added exact version/explicit `Unreleased` parsing and fail-closed validation; generated the stable release body around matching changelog copy; removed only redundant release-specific YAML text; updated CI and release instructions.
- Sol: initial `REVISE` found stripped trailing content-owned reference definitions. The required follow-up `REVISE` found case/whitespace normalization gaps. Both were reproduced, fixed, and covered; follow-up wire contract was `OK` and bug-class sweep was `CLEAN`. See `docs/audits/consultations/h1-sol.md`.
- Gates: release/Discord Node tests 16/16; root Vitest 37 files/490 tests; `npm run check`; `npm run check:versions` (6/6); beta.18 generator preview; `git diff --check`.
- Handoff: pushed the branch and opened draft stacked PR [#384](https://github.com/ESO-Toolkit/kalpa/pull/384). No tag, release, merge, or deployment was performed.
- Exact next action: land W1 first, retarget PR #384 to `main`, confirm CI remains green, then mark it ready for maintainer review.

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
- Active branch: `fix/audit-r6-crash-safe-installer`, stacked on `fix/audit-p0-a2-cross-process-locking`.
- Design: completed the required fresh read-only Fable consultation and recorded D-R6-1. The selected design is a copied-residual merged stage, journaled whole-folder tombstone swap, post-swap hash promotion, and deterministic lock-protected restart recovery. The maintainer choices were to refuse links and use copies rather than hard links.
- Implementation: added the shared installer transaction module to Tauri and Slint; moved ZIP extraction and keep-mine baseline construction into staged images; migrated install, update, batch update, remove, scan, profile activation, restore, and native equivalents to recover or hold the transaction lock; and excluded reserved transaction state from addon scans.
- Verification: `npm ci`, `npm test` (37 files, 490 tests), `npm run check`, formatting for both Rust crates, strict all-target/all-feature clippy for both crates, the native Slint release-sidecar build, and the full main Rust suite (840 passed, 18 ignored) pass. The focused transaction suite passes 10 tests, covering rollback after one of multiple folder swaps, abandoned staged/swapped markers, missing expected live folders and hash baselines, invalid journal paths, locked files, transient rename retries, successful cleanup, cancellation preservation, traversal/reserved names, residual files, and keep-mine behavior. Three focused cross-instance copy tests and focused editor/Protected Edits restore tests also pass; strict main-crate clippy and all 10 transaction tests were rerun after the final batch enable/disable guard.
- Environment limitations: two single-job full Slint test attempts reached the final link and were terminated by Windows/LLVM memory exhaustion; the same crate passes strict clippy and its optimized native sidecar build completes. Destructive sandbox verification is blocked because PID 52768 is a user-owned Kalpa process in another T3 worktree and the runner cannot isolate app state; it was not terminated.
- Review: adversarial Sol review returned REVISE for unguarded live AddOns mutations in the manual editor, Protected Edits restore, cross-instance copy, and batch enable/disable paths. All verified sites were migrated to the shared transaction guard; the final focused follow-up returned APPROVE with no findings, missing tests, or wire-contract changes.
- Handoff: opened stacked draft PR [#399](https://github.com/ESO-Toolkit/kalpa/pull/399) against `fix/audit-p0-a2-cross-process-locking`. No merge or deployment was performed; the next action is review and merge in stack order after P0-A2.

### 2026-08-26 — Codex (H5)

- Active branch at that time: `fix/audit-h5-branch-pruning-proposal` (then stacked on W1; now retargeted to `main`).
- Completed: refreshed and pruned stale remote-tracking refs; inventoried 129 local and 111 remote branches against `origin/main`; correlated open PR heads and all attached worktrees; classified 54 safe candidates, 76 retain refs, and 110 needs-human-review refs in `2026-08-h5-branch-pruning-proposal.md`. Refreshed the snapshot after concurrent H1 PR #384, the H3 worktree, and this published H5 branch appeared.
- Safety: no local or remote branch was deleted, and no branch tip or worktree was moved.
- Reviews: initial Sol verdict `REVISE` after concurrent H1 PR #384 made the snapshot stale. Refreshed H1 and H3 state, added a mandatory freshness guard, and clarified protection/worktree-counterpart handling. The single follow-up verdict was `APPROVE` with exact count and candidate-set parity.
- Verification: `npm run check` and `git diff --check` pass after installing locked dependencies in the isolated worktree.
- Handoff: the final freshness guard matched the proposal exactly, then draft PR [#385](https://github.com/ESO-Toolkit/kalpa/pull/385) was opened against W1.
- Active work: maintainer review of the proposal only; no cleanup is authorized by this PR.
- Blockers: none for the proposal. A final pre-PR freshness guard must remain clean; any later cleanup requires explicit maintainer approval naming the branches and another fresh guard recheck.
- Exact next action: maintainer reviews the 54 proposed candidates and explicitly approves all or a named subset, after which a separate cleanup operation must rerun every guard.
- Blockers: final Slint test linking and destructive sandbox execution are externally blocked as described above; no implementation blocker.
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

### 2026-08-28 — Claude (P0-A3 shipped; sandbox gate closed)

- Pushed the branch with `--force-with-lease` (maintainer-authorised) after the other session's rebase, and marked PR #389 ready for review. `origin` and local are identical; the PR is `MERGEABLE` on base `b63354de`. Not merged — the stack order (#389 → #388 → #380 → #369) belongs to the session handling the PR stack.
- **`npm run test:e2e:sandbox` now run and passing, 3/3.** The blocker cleared: no `kalpa.exe` was running and nothing was listening on the CDP port. Developer state was protected the documented way — the real `%APPDATA%\com.kalpa.desktop` was moved aside before the run, the sandbox's output preserved separately as `…claude-p0a3-sandbox-output-20260828-150517`, and the original moved back. Restoration verified byte-for-byte: identical file list, `settings.json` SHA256 `F211FD72…` unchanged, and `manifest-cache.db` still 122880 bytes rather than the empty DB the run would otherwise have left.
- The sandbox launch log also exercised this branch's protocol end to end in the real debug binary: `[native-shell] WebView acquired UI authority`.
- P0-A3 now has no outstanding work. Every required gate has been run.

### 2026-08-28 — Claude (P0-A3 ship review)

- Re-analysed Sol's findings 2 and 3 and **reversed my earlier partial refutation**: they do produce two independent writers, by way of the next launch spawning a sidecar into a free lock. Fixed under D-P0-A3-FATAL, plus the two follow-ups the final Fable consultation asked for.
- Final Fable consultation (`p0-a3-authority-loss-fable.md`) returned `SHIP: YES - P0-A3 meets its acceptance criteria`, withdrew its own earlier `acquired_matches ||` suggestion, and confirmed the concurrent grace-retry change is fail-safe.
- Gates after the fixes: main Rust strict clippy/fmt clean; Slint strict clippy/fmt clean; `native_boot` alone carries 26 tests.
- **Concurrent-agent activity in this worktree.** Another agent edited these same files during the session and committed `ed407c66 fix(native): retry transient authority probes` (a `try_claim_authority_with_grace` used by the sidecar's `acquire_native_shell_lock`, so a child racing the parent's liveness probe is not misread as a duplicate). I did not author it and did not commit it; to avoid absorbing in-flight work I staged my own hunks only, by rebuilding each file as "HEAD + my edits" rather than slicing a diff. Fable reviewed the change on request and judged it sound and fail-safe (a miss makes the child exit and the parent keep the WebView; it can never turn a duplicate into two writers).
- **Branch history was rewritten by the other session and local/origin have diverged — not pushed.** See Open Questions; this is the one thing blocking handoff.

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

### 2026-08-26 — Codex (H4)

- Active branch: `fix/audit-h4-claude-structure-tree`; originally stacked on `fix/audit-w1-worker-consistency`, later retargeted to `main` after W1 merged.
- Scope: H4 only. Inventory tracked paths and update the `claude.md` project tree for the existing `animate-ui`, frontend test, E2E, and Worker test directories.
- Test-first note: a failing runtime regression test is mechanically inapplicable to a documentation-only inventory correction. Before editing, `rg --files` established that the directories exist while the `claude.md` project tree contained none of the `animate-ui`, `__tests__`, `e2e`, or Worker `test` paths.
- Sol review: `REVISE`. The bug-class sweep verified the same omissions in the public `README.md` structure tree. Applied the same tracked-path correction there; no runtime or wire-contract changes.
- Completed: updated both maintained project trees from tracked paths, passed documentation gates, addressed the verified README sweep finding, and opened draft PR [#382](https://github.com/ESO-Toolkit/kalpa/pull/382).
- Sol follow-up: `APPROVE`; no findings or missing tests, wire contract OK, and bug-class sweep clean.
- Blockers: none.
- Exact next action: review PR #382 after its current-`main` validation passes.

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

- R4: the dependency-install paths currently stamp one `esoui_id` onto every extracted folder, so a multi-folder dependency update-checks N times. The chosen model makes it check once. That is an install-outcome change and needs explicit sign-off.
- R4: the demoted-user heal lives in `auto_link`, which **only the main app has** — the Slint sidecar has no equivalent. Slint-only users stay demoted until they open the main app once. Porting `auto_link` to Slint is a separate, larger task.
- R5: conflict wire values change meaning (bare -> folder-qualified) **without a field rename**, so a stale frontend bundle or Tauri/Slint skew would mis-route silently. Decide whether to accept monorepo lockstep or rename the field to `qualifiedPath` and take the churn.
- ~~R5 must be sequenced after R4~~ — done: R5 is stacked directly on R4.
- R6: addon folders that are symlinks or junctions (developers pointing `AddOns/Foo` at a git checkout). A swap replaces the link with a real directory. Recommendation is to refuse with an explanatory message rather than silently fall back to the legacy in-place path. Needs a decision.
- R6: directory rename on Windows fails if any file inside is open without `FILE_SHARE_DELETE`. Behaviour becomes a clean refusal instead of a half-overwrite, which is strictly better, but users will see a **new error** where they previously saw a silently corrupting success. Needs the CFA-style explanatory message and sign-off.
- R6 stacks on both P0-A1 and P0-A2 and cannot merge independently.

- P0-A1: adopt Fable item 2a, widening the rename budget from ~200ms (`RENAME_ATTEMPTS = 5` at a flat 40ms) to a bounded geometric backoff of roughly 2.5s, and adding `ERROR_USER_MAPPED_FILE` (1224) to `is_transient_rename_error`. Two P0-A2 concurrency tests flake under load and one was definitively in the rename. The same budget serves the real `settings.json` write path in both binaries, so the trade is bounded extra latency against a hard failure. Needs a maintainer decision on acceptable worst-case settings-save latency.

- ~~P0-A3 branch divergence~~ **Resolved 2026-08-28.** The other session rebased this branch inside the worktree onto a P0-A2 that gained `b63354de fix(locking): serialize same-process transactions`, rewriting every pre-rebase commit and leaving local/origin at 11/16. The maintainer explicitly authorised a `--force-with-lease` push, pinned to the expected origin SHA `6f1c90bb` so a concurrent push would have aborted it. Published cleanly; `origin` and local are identical and PR #389 sits on `b63354de`. Recorded because the standing rule is otherwise never to force-push.
- P0-A3: the fatal-authority-loss decision (D-P0-A3-FATAL) means an unrecoverable reclaim failure now drops the user's window. Fable and I both judge the path effectively unreachable on Windows (it needs `TerminateProcess` on our own already-reaped child to fail), and the cost is a lost view rather than lost data since every write is atomic. Flagging it because it is a user-visible behaviour change.

- W2: maintainer approval is required before merging any reconciliation path that can delete rows from shared D1.
- W1: decide whether moving vote-record authority from KV/in-memory memo into DO storage belongs in W1 or W3. The current W1 code prevents resurrection but retains the pre-existing eviction/double-toggle limitation for later hardening.
- W1: owner sign-off is required before the later manual `kv` → `do` authority flip and must accept backup restore as the post-flip rollback path.
- P0-A2: D-P0-1 establishes the lock invariants; a fresh Fable consultation must choose the concrete cross-platform dependency/API and user-visible timeout behavior before implementation.
- R4/R5: ownership/conflict behavior that changes install outcomes requires explicit design review before implementation.
