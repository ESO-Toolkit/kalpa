# August 2026 Audit Remediation Tracker

This file is the durable execution record for `2026-08-remediation-master-prompt.md`.

| ID | Status | Branch | PR | Merged SHA | Fable decision | Sol verdict | Tests | Notes |
|---|---|---|---|---|---|---|---|---|
| W1 | in-progress | `fix/audit-w1-worker-consistency` | - | - | D-W1-1 | pending | focused 8 pass; Worker 159 pass | Implementation complete locally; rollout bootstrap decision remains open. |
| W2 | todo | - | - | - | pending | - | - | Requires maintainer approval before merge if reconciliation can delete D1 rows. |
| W3 | todo | - | - | - | - | - | - | Worker low-severity hardening. |
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

## Session Log

### 2026-08-26 — Codex

- Active branch: `fix/audit-w1-worker-consistency`
- Completed: read repository guidance, master prompt, and audit memory; fetched and fast-forward checked `main`; created the persistent tracker; started Kalpa successfully in Tauri dev mode; completed the W1 Fable review; captured five failing-before DO tests; implemented DO-authoritative mutations; added route-level duplicate, stale-update, and delete/vote race coverage; Worker typecheck and 159 tests pass.
- Active work: W1 dry-run build, Sol review, and rollout decision.
- Blockers: none.
- Exact next action: run the Worker dry-run, obtain the required Sol review, resolve verified findings, and decide the safe first-bootstrap rollout before opening a PR.

## Open Questions

- W2: maintainer approval is required before merging any reconciliation path that can delete rows from shared D1.
- W1: choose between a coordinated quiet first mutation and an explicit authenticated bootstrap operation; do not merge until the stale-KV bootstrap risk is resolved.
- W1: decide whether moving vote-record authority from KV/in-memory memo into DO storage belongs in W1 or W3. The current W1 code prevents resurrection but retains the pre-existing eviction/double-toggle limitation for later hardening.
- P0-A2: lock dependency and user-visible timeout behavior require a Fable recommendation and may require maintainer input.
- R4/R5: ownership/conflict behavior that changes install outcomes requires explicit design review before implementation.
- H2: `kalpa-elder-scrolls-themes/` is currently untracked; provenance must be inspected before tracking or ignoring it.
