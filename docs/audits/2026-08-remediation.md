# August 2026 Audit Remediation Tracker

This file is the durable execution record for `2026-08-remediation-master-prompt.md`.

| ID | Status | Branch | PR | Merged SHA | Fable decision | Sol verdict | Tests | Notes |
|---|---|---|---|---|---|---|---|---|
| W1 | pr-open | `fix/audit-w1-worker-consistency` | [#369](https://github.com/ESO-Toolkit/kalpa/pull/369) (draft) | - | D-W1-1, D-W1-2, D-W1-3 | REVISE; follow-up REVISE; all verified findings addressed | Worker check; 184 tests; Wrangler dry-run; name guard | Fable twice-reject redesign implemented; awaiting refreshed CI/maintainer sign-off. |
| W2 | pr-open | `fix/audit-w2-d1-reconciliation` | [#378](https://github.com/ESO-Toolkit/kalpa/pull/378) (draft) | - | D-W2-1, D-W2-2 | Fresh REVISE; follow-up APPROVE | Worker check; 208 tests; Wrangler dry-run | Technically ready and stacked on W1. Draft/merge hold remains: deletion-capable code and later `apply` each require separate maintainer approval. |
| W3 | in-review | `fix/audit-w3-worker-hardening` | [#379](https://github.com/ESO-Toolkit/kalpa/pull/379) (draft) | - | D-W3-1 reaccepted after reconsultation | Fresh Sol pending | Worker check; 230 tests; Wrangler dry-run | Stacked on refreshed W2/D-W2-2; Fable unblocked the design and required body-abort/cache-header clarifications, now covered. No real deployment. |
| P0-A1 | todo | - | - | - | pending | - | - | Shared crash-safe atomic writer. |
| P0-A2 | todo | - | - | - | pending | - | - | Cross-process read-modify-write locking. |
| P0-A3 | todo | - | - | - | pending | - | - | Native sidecar ready handshake. |
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

## Session Log

### 2026-08-27 — W1 twice-reject escalation

- PR #369 was returned to draft after review identified four additional correctness failures: stale KV versions can be frozen by ID-only shadow merging; partial vote cleanup can corrupt a still-live pack; a failed KV detail deletion is not retryable after the DO tombstone commits; and a failed KV detail write leaves a committed create that retries as a duplicate.
- Per the twice-reject rule, W1 is blocked and implementation is paused while Fable is reconsulted with both prior Sol reviews and the new review evidence.
- Fable selected D-W1-3: journaled and resumable lifecycle transitions, tombstone-first delete, DO-authoritative detail reads, version-aware shadow reconciliation, and alarm-based effect repair.
- Fresh Sol review returned `REVISE` with four findings: orphan detail adoption during account purge; stale same-author operations crossing slug reuse; vote/install counters committing before a fallible KV mirror; and a backup write racing account deletion. The single prescribed follow-up also returned `REVISE` after verifying those cases.
- Addressed every follow-up finding with author-scoped orphan hydration, created-at lifecycle compare-and-swap for update/delete, durable dirty-mirror markers repaired by alarm, and DO-serialized backup/account deletion guarded by a deleted-author latch. Added exact failure/retry regressions.
- Final local evidence: Worker TypeScript check passes; all 184 tests pass; Wrangler dry-run passes; `wrangler.toml` remains `kalpa-pack-hub`. No deploy, merge, schema change, or Worker rename was performed.
### 2026-08-27 — Codex W3 revalidation

- Merged updated W2 commit `4b2c18d0` forward with a normal merge. The authority-route conflict was resolved additively: W3's bounded streaming reader now protects W2's exact `authority` and `unowned_d1_ids` validation; D-W2-2 and W3 tracker history were both preserved.
- Marked W3 blocked after its two verified Sol `REVISE` rounds and reconsulted Fable with both reviews, the final diff/tests, D-W2-2 dependency, and separate H3 package-version boundary. Fable accepted Candidate A and reaccepted D-W3-1.
- Failing-before revalidation reproduced an escaped rejected body stream and missing explicit viewer cache metadata. Fixed by catching stream aborts as invalid JSON, decoding only the fully assembled bounded byte buffer, emitting explicit `public, max-age=0` for viewer-bearing list/detail responses, and varying all CORS responses on `Origin, Authorization`.
- Focused body/list/CORS tests pass 146/146; full Worker tests pass 230/230; Worker check, Wrangler dry-run, name guard, and `git diff --check` pass. Repository search found no in-repo consumer of removed health detail fields; the deploy workflow reads only `status` and `kv`. No real deployment, merge, authority flip, schema change, or H3 version decision occurred.

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
- P0-A2: lock dependency and user-visible timeout behavior require a Fable recommendation and may require maintainer input.
- R4/R5: ownership/conflict behavior that changes install outcomes requires explicit design review before implementation.
- H2: `kalpa-elder-scrolls-themes/` is currently untracked; provenance must be inspected before tracking or ignoring it.
