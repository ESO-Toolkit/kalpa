# August 2026 Audit Remediation Tracker

This file is the durable execution record for `2026-08-remediation-master-prompt.md`.

| ID | Status | Branch | PR | Merged SHA | Fable decision | Sol verdict | Tests | Notes |
|---|---|---|---|---|---|---|---|---|
| W1 | pr-open | `fix/audit-w1-worker-consistency` | [#369](https://github.com/ESO-Toolkit/kalpa/pull/369) (draft) | - | D-W1-1, D-W1-2, D-W1-3 | REVISE; follow-up REVISE; all verified findings addressed | Worker check; 184 tests; Wrangler dry-run; name guard | Fable twice-reject redesign implemented; awaiting refreshed CI/maintainer sign-off. |
| W2 | todo | - | - | - | pending | - | - | Requires maintainer approval before merge if reconciliation can delete D1 rows. |
| W3 | todo | - | - | - | - | - | - | Worker low-severity hardening. |
| P0-A1 | todo | - | - | - | pending | - | - | Shared crash-safe atomic writer. |
| P0-A2 | todo | - | - | - | pending | - | - | Cross-process read-modify-write locking. |
| P0-A3 | todo | - | - | - | pending | - | - | Native sidecar ready handshake. |
| R4 | design-done | - | - | - | D-R4-1 | - | - | Fable design complete (`consultations/r4-fable.md`). Implementation not started. **Must land before R5.** |
| R5 | design-done | - | - | - | D-R5-1 | - | - | Fable design complete (`consultations/r5-fable.md`). Implementation not started. Sequence **after R4**. |
| R6 | design-done | - | - | - | D-R6-1 | - | - | Fable design complete (`consultations/r6-fable.md`). Implementation not started. Stacks on P0-A1 **and** P0-A2; cannot merge independently. |
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
- **New issue surfaced by this review:** nothing currently validates a ZIP's top-level folder name against the `.kalpa-` reserved prefix, so an archive whose top folder is `.kalpa-hashes` or `.kalpa-backups` writes straight into Kalpa's own state directories. The design adds that rejection alongside the existing single-path-component check.
- Supersedes `installer.rs:951` `cancel_midway_preserves_pre_existing_addon_files`, which currently asserts the in-place semantics ("no file is removed, only some are overwritten") as a requirement.

## Session Log

### 2026-08-27 — W1 twice-reject escalation

- PR #369 was returned to draft after review identified four additional correctness failures: stale KV versions can be frozen by ID-only shadow merging; partial vote cleanup can corrupt a still-live pack; a failed KV detail deletion is not retryable after the DO tombstone commits; and a failed KV detail write leaves a committed create that retries as a duplicate.
- Per the twice-reject rule, W1 is blocked and implementation is paused while Fable is reconsulted with both prior Sol reviews and the new review evidence.
- Fable selected D-W1-3: journaled and resumable lifecycle transitions, tombstone-first delete, DO-authoritative detail reads, version-aware shadow reconciliation, and alarm-based effect repair.
- Fresh Sol review returned `REVISE` with four findings: orphan detail adoption during account purge; stale same-author operations crossing slug reuse; vote/install counters committing before a fallible KV mirror; and a backup write racing account deletion. The single prescribed follow-up also returned `REVISE` after verifying those cases.
- Addressed every follow-up finding with author-scoped orphan hydration, created-at lifecycle compare-and-swap for update/delete, durable dirty-mirror markers repaired by alarm, and DO-serialized backup/account deletion guarded by a deleted-author latch. Added exact failure/retry regressions.
- Final local evidence: Worker TypeScript check passes; all 184 tests pass; Wrangler dry-run passes; `wrangler.toml` remains `kalpa-pack-hub`. No deploy, merge, schema change, or Worker rename was performed.

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


- R4: the dependency-install paths currently stamp one `esoui_id` onto every extracted folder, so a multi-folder dependency update-checks N times. The chosen model makes it check once. That is an install-outcome change and needs explicit sign-off.
- R4: the demoted-user heal lives in `auto_link`, which **only the main app has** — the Slint sidecar has no equivalent. Slint-only users stay demoted until they open the main app once. Porting `auto_link` to Slint is a separate, larger task.
- R5: conflict wire values change meaning (bare -> folder-qualified) **without a field rename**, so a stale frontend bundle or Tauri/Slint skew would mis-route silently. Decide whether to accept monorepo lockstep or rename the field to `qualifiedPath` and take the churn.
- R5 must be sequenced **after** R4: a sibling that is also separately tracked has its manifest rewritten with the installing addon's id, so ownership precedence has to be settled first.
- R6: addon folders that are symlinks or junctions (developers pointing `AddOns/Foo` at a git checkout). A swap replaces the link with a real directory. Recommendation is to refuse with an explanatory message rather than silently fall back to the legacy in-place path. Needs a decision.
- R6: directory rename on Windows fails if any file inside is open without `FILE_SHARE_DELETE`. Behaviour becomes a clean refusal instead of a half-overwrite, which is strictly better, but users will see a **new error** where they previously saw a silently corrupting success. Needs the CFA-style explanatory message and sign-off.
- R6 stacks on both P0-A1 and P0-A2 and cannot merge independently.

- W2: maintainer approval is required before merging any reconciliation path that can delete rows from shared D1.
- W1: decide whether moving vote-record authority from KV/in-memory memo into DO storage belongs in W1 or W3. The current W1 code prevents resurrection but retains the pre-existing eviction/double-toggle limitation for later hardening.
- W1: owner sign-off is required before the later manual `kv` → `do` authority flip and must accept backup restore as the post-flip rollback path.
- P0-A2: lock dependency and user-visible timeout behavior require a Fable recommendation and may require maintainer input.
- R4/R5: ownership/conflict behavior that changes install outcomes requires explicit design review before implementation.
- H2: `kalpa-elder-scrolls-themes/` is currently untracked; provenance must be inspected before tracking or ignoring it.
