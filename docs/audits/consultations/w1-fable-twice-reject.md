# Fable Reconsultation — W1 After Two Revisions

## Finding and acceptance criteria

W1 must eliminate lost updates and lifecycle corruption across Worker KV, the
SQLite-backed `PackIndexDO`, the shared D1 mirror, vote records, backups, seed,
restore, and account deletion without changing the Worker name, public JSON
wire shapes, or D1 schema. Merge to `main` auto-deploys, so the branch must be
safe immediately in its default rollout state. No reconciliation may delete D1
rows without separate maintainer approval.

The design must additionally guarantee retry-safe behavior when any KV or D1
operation fails between durable steps. A failed request must not leave a live
pack with partially deleted votes, a deleted pack publicly readable through a
stale detail key, or a canonical create that can neither be acknowledged nor
repaired by retry.

## Current design

The current branch uses a single Durable Object as mutation authority. During
the default `kv` shadow phase, each serialized operation additively imports KV
index records into per-ID DO storage, while per-ID tombstones prevent deleted
records from being reimported. It does not rewrite the full KV index before an
operator performs a parity-gated `kv` to `do` flip. Public list and backup reads
use the merged DO view; public detail reads still use KV detail keys.

Mutation methods currently commit DO state and synchronously mirror KV/D1. A
delete currently removes vote keys, deletes D1, commits the DO tombstone, and
then deletes KV detail. A create currently commits the DO pack and then writes
KV detail/D1. Shadow merging treats an existing DO ID as authoritative without
comparing record freshness.

## First Sol review — REVISE

1. A one-shot bootstrap can permanently omit a live pack when KV returns a
   stale index.
2. Stale vote/install requests can cross delete-and-recreate and mutate the new
   lifecycle.
3. Update/delete authorization trusts stale KV ownership rather than canonical
   DO ownership.
4. Restore preservation reads KV rather than being atomic inside the DO.
5. Account deletion leaves other users' vote records attached to removed pack
   IDs, allowing a reused ID to inherit votes.

## Second Sol review — REVISE

1. Writing the full KV index from an incomplete shadow can erase unseen packs.
2. Scheduled backup filtering can restore another user's vote on a deleted pack.
3. Vote cleanup outside the DO can race delete/recreate and erase votes from the
   new lifecycle.

## New PR review evidence

1. **Stale version frozen by ID-only merge.** A first shadow read can import an
   old KV `Pack`; later, newer KV versions are ignored because the ID exists.
   Vote/install/update then treats stale content and counters as canonical and
   can mirror them over the newer detail. ID-only parity reports clean.
2. **Partial vote cleanup while the pack remains live.** If deleting one of
   several vote keys fails, earlier deletions persist but the canonical pack and
   its unchanged `vote_count` remain visible.
3. **Delete cleanup is not retryable.** If KV detail deletion fails after the DO
   tombstone commits, retry returns `not-found` before retrying cleanup. Public
   detail remains readable and slug reuse remains blocked.
4. **Create mirror failure is not recoverable.** If the DO create commits before
   KV detail write fails, list exposes the pack while detail returns 404; retry
   returns duplicate rather than completing or acknowledging the create.

## Failure modes to decide explicitly

1. Shadow sees version A from the KV index, then version B from the index or
   detail key, with no monotonic revision field in the wire contract.
2. Vote-key enumeration/deletion fails after deleting one key.
3. D1 deletion fails before or after canonical lifecycle deletion.
4. KV detail deletion fails after canonical lifecycle deletion, then the same
   authenticated delete retries.
5. KV detail write or D1 write fails after canonical create, then the same
   authenticated create retries.
6. The process dies between any two steps above and a later request uses a new
   isolate.
7. A slug is reused while cleanup for its old lifecycle remains pending.
8. Rollback occurs during the default shadow phase.

## Candidate directions

### A — Durable operation journal/outbox in DO storage

Commit canonical lifecycle state plus a durable operation record. Reconcile
external KV/D1/vote effects idempotently on every relevant request and/or DO
alarm. Return success only under a clearly specified acknowledgement rule.
Define whether public detail must move to DO authority immediately, how create
retries are identified without adding a client idempotency key, and how slug
reuse is gated until old-lifecycle cleanup completes.

### B — External effects first with compensating rollback

Perform KV/D1/vote changes before canonical DO commit and compensate on error.
Explain how compensation can be reliable when the same eventually consistent
or failing stores caused the original error.

### C — Retain synchronous mirroring with durable pending markers

Keep the present architecture but persist per-pack pending-create/delete state
before external effects. All methods and reads resume the pending transition
before acting. Define version reconciliation and crash behavior precisely.

## Required output

Return only:

```text
DECISION:
1. Chosen design and numbered implementation/deployment steps

REJECTED:
1. Alternative and the concrete failure that rejects it

CRASH_RECOVERY:
1. Behavior after process kill, rollback, partial mirror, timeout, or stale read

TESTS:
1. Exact failure-injection and retry tests that distinguish the design

RISKS:
1. Remaining risks and required human decisions
```

## Consultation outcome — 2026-08-26

```text
DECISION:
1. Candidate A — a durable operation journal/outbox in DO storage — with public
   detail reads moved to DO authority in this same deploy. All four PR findings
   are the same defect: canonical commit and external effects are not atomic and
   the operation carries no identity, so it cannot be resumed or re-acknowledged.
2. Storage additions (DO-internal only; no KV/D1/wire shape change):
   `own:<id>` = true once the DO has mutated the record (absent = KV-derived
   shadow copy, still refreshable); `tomb:<id>` widened from an ISO string to
   `{ deletedAt, authorId, lifecycle }` (read path tolerates the old string);
   `op:<opId>` = `{ kind, packId, lifecycle, actorId, phase, steps, voteCursor,
   attempts, startedAt }`; `pending:<packId>` = opId. opId is deterministic:
   `${kind}:${packId}:${actorId}:${lifecycle}`, so an authenticated retry maps to
   the same journal entry with no client idempotency key.
3. Replace the ID-only merge with version reconciliation. `mergeFromKv`: skip
   tombstoned; import if absent; overwrite if present, not owned, and incoming
   `updated_at` is strictly newer. Add `resolveShadow(id)`, called at the top of
   every mutation and of `getPack` only while `own:<id>` is absent: one
   `cacheTtl: 30` read of `pack:<id>`, taken when its `updated_at` is >= stored.
   One KV get for the single id being acted on — no corpus fanout, so the
   subrequest ceiling is unaffected. Every DO mutation sets `own:<id>`; after
   that KV can never overwrite it. Parity gains `stale_shadow` (ids where KV and
   an unowned DO record disagree) and the `do` flip blocks while it is non-empty.
4. Journaled delete. Resolve shadow; if no live pack, consult `tomb:<id>` — a
   pending delete op whose `actorId` matches the tombstone's `authorId` resumes
   and returns `ok`; a done tombstone returns `not-found`. Authorize against the
   DO record. Then one `blockConcurrencyWhile` transaction writes the op (phase
   pending), `pending:<id>`, deletes `pack:<id>` and writes the tombstone. After
   that transaction the pack is canonically gone, so no partial external cleanup
   can ever coexist with a live pack. External steps follow, each latched in the
   journal: vote-key deletion with the `list()` cursor persisted per page, KV
   detail delete, D1 delete, index mirror when authority is `do`.
5. Journaled create. The duplicate check additionally consults `pending:<id>`: a
   pending delete returns 409 (never delete the tombstone out from under
   unfinished cleanup — today `addPack` clears it unconditionally); a pending
   create by the same actor resumes and returns 201 with the stored pack; by a
   different actor returns 409. Canonical pack + `own:<id>` + op + `pending:<id>`
   commit in one transaction, then KV detail put, D1 upsert, index mirror.
6. Acknowledgement rule. 2xx only after the canonical commit AND the KV detail
   step confirms. Failure of that step returns 503 + `Retry-After: 5` with the
   journal pending. D1 and the index mirror do not block the response — D1 is a
   website mirror and is already best-effort — but they are journaled and retried.
7. Public detail moves to DO authority now, not at the flip: `handleGetPack`
   calls `getPackIndexDO(env).getPack(id)` instead of `getPack(env, id)`. A
   tombstone then hides a deleted pack while its KV detail delete is still
   pending, and a created pack is readable while its detail put is pending. This
   is what converts findings 3 and 4 from correctness bugs into bounded latency.
   Anonymous responses keep the 300s edge cache; authenticated stay uncacheable.
8. Alarm reconciler. Writing a pending entry schedules `setAlarm(now + 30s)` if
   no earlier alarm exists. `alarm()` resumes every pending op idempotently with
   exponential backoff to 30 min and reschedules while any remain.
9. Vote/install keep the `expectedLifecycle` (`created_at`) check and add a
   tombstone check, so a request that crossed a delete cannot resurrect the pack
   through `getPack`'s KV detail fallback.
10. Backup filters `votes` to live untombstoned ids from `getIndex()`; restore
    replays only those, and `replaceIndexPreserving` refuses to clear a tombstone
    whose delete op is still pending.
11. `removePacksByAuthor` journals one delete op per id in the shape of step 4,
    so an interrupted account deletion resumes instead of orphaning vote records.
12. Deployment is unchanged: ships in the default `kv` shadow phase, no
    full-index write before the flip, no operator action at merge. The `do` flip
    still requires two parity observations more than the KV cache window apart,
    now with both `missing_from_do` and `stale_shadow` empty.

REJECTED:
1. B (external effects first, compensating rollback) — compensation runs against
   the store that just failed. Create writes `pack:<id>`, D1 fails, the
   compensating KV delete also fails, and because nothing canonical committed
   there is no DO record to reconcile from: the slug is publicly readable,
   unowned, untombstoned and permanently unclaimable. A crash between the
   external write and the canonical commit yields the same orphan with no
   compensator running at all. Strictly worse than the current branch.
2. C (pending markers, no operation identity) — a marker records that a
   transition is in flight but not which request owns it, so an authenticated
   create retry stays indistinguishable from a third party claiming the slug and
   finding 4 remains open. Without an alarm, a pending transition on a pack
   nobody requests again is never resumed, leaving a stale detail key for an
   unbounded time.
3. A client idempotency key — changes the public wire contract. The deterministic
   `(kind, packId, actorId, lifecycle)` opId supplies the same identity from data
   the request already carries.
4. A monotonic `rev` field on `Pack` — changes the wire shape and the D1 mirror.
   `updated_at` plus the `own:<id>` latch is sufficient because after deploy the
   only writer of any `pack:<id>` detail key is the DO itself.
5. Auto-adopting D1 rows as witnesses — unchanged from the prior consultation: a
   D1 row may be a zombie from a failed delete. Adoption stays explicit and
   backed by an independently propagated detail record.
6. Deferring the public-detail switch to the authority flip — leaves findings 3
   and 4 open for the whole shadow phase, which is the phase this branch merges in.

CRASH_RECOVERY:
1. Kill between canonical commit and any external step: canonical state is
   already correct, `op:`/`pending:` survive, and the next request touching that
   pack or the alarm resumes. No success response was ever sent.
2. Kill mid vote-key deletion: the persisted `voteCursor` resumes the `list()`
   walk from the last completed page. The pack is already tombstoned, so a live
   pack with partially deleted votes is unreachable by construction.
3. Kill after every external step but before `phase: "done"`: all steps are
   idempotent (deleting an absent key, putting an identical body, `DELETE ...
   WHERE id = ?`), so the resume re-runs them harmlessly and latches done.
4. Delete retried while cleanup is pending: matched by opId, resumed, returns
   `ok` — never `not-found`.
5. Create retried while the mirror is pending: matched by opId and actor,
   resumed, returns 201 with the same canonical pack — never `duplicate`.
6. Rollback during the shadow phase: unchanged from the prior consultation —
   post-deploy creates are deliberately absent from the legacy full-index value,
   so rollback needs corpus recovery from `backup:latest` plus the `pack:<id>`
   detail keys, not a flag flip. Journal entries persist and resume on redeploy.
7. Rollback after the `do` flip: restore-from-backup, unchanged.
8. Stale KV read during shadow: an unowned record is refreshed from its detail
   key by `updated_at` on next use; an owned record is never overwritten;
   `stale_shadow` blocks the flip while any disagreement remains.
9. Slug reuse while old-lifecycle cleanup is pending: create is refused 409 until
   the delete journal reaches done.
10. D1 down throughout: mutations still succeed, journal entries stay pending,
    the alarm keeps retrying, and parity/the flip are unaffected because no
    witness requires a D1 write to have landed.

TESTS (a KV/D1 wrapper that fails a named operation on the Nth call;
vitest-pool-workers gives real DO storage, so these run in
`test/pack-index-do.test.ts` and `test/routes.test.ts`):
1. Create with `ESO_PACKS.put("pack:<id>")` failing: expect 503, the DO holding
   the pack, `op:` pending; retry the identical authenticated POST and expect 201
   with the same `created_at` and exactly one canonical record — not 409. A build
   without journal identity returns duplicate and fails.
2. Same setup, but invoke `alarm()` instead of retrying: expect the detail key to
   appear and the journal to latch done.
3. Delete with `ESO_PACKS.delete("pack:<id>")` failing: expect 503, expect
   `GET /packs/:id` to already 404 (proves detail reads are DO-authoritative),
   then retry the DELETE and expect `ok` with the key gone. A build that returns
   `not-found` on retry fails.
4. Delete a pack with three votes, failing the second vote-key delete: expect the
   pack already absent from list and detail and no `vote_count` observable
   anywhere; resume and expect all three `vote:` and all three `user-votes:` keys
   gone.
5. Slug reuse under pending delete: fail vote cleanup, POST the same slug as a
   different user → 409; resume; POST again → 201 with a fresh `created_at`, zero
   votes, and no surviving vote key from the old lifecycle.
6. Stale shadow: seed `index:packs` with `X@T1` and `pack:X` with `X@T2 > T1`,
   drive a mutation on an unrelated pack, then read X and expect T2 served and
   mirrored, `/admin/migration/parity` to list X in `stale_shadow`, and
   `setAuthority("do")` to 409 while it is non-empty. ID-only merge reports clean
   parity and fails this test.
7. Ownership latch: mutate X through the DO, rewrite `index:packs` with an older
   X, run a mutation, expect the DO to still hold the newer owned record.
8. Vote crossing a delete: capture a pack, delete it, call the vote path with the
   old `created_at`, expect no counter movement and no vote key written —
   including when `pack:<id>` still exists in KV.
9. Backup/restore: tombstone a pack, run `scheduled()`, expect its votes absent
   from `backup:latest`; restore that snapshot and expect the tombstone to
   survive with no vote key recreated.
10. Account deletion interrupted: fail D1 on the second pack; expect the first
    pack's votes fully gone, the second tombstoned with a pending op, and the
    alarm to complete it.
11. Retained regression: merge runs on every KV-authority mutation rather than
    once behind a latch, and a tombstoned id is never re-imported.

RISKS:
1. Every public detail read now originates at a single Durable Object. The
   anonymous 300s edge cache absorbs most of it, but authenticated reads and cold
   entries serialize. Recommend accepting at beta traffic and watching DO CPU and
   queueing after deploy; a DO-side response cache is the fallback. Human call.
2. Success now means "canonical + KV mirrored", so callers see 503 where they
   previously saw 200 over a silently broken mirror. The Rust client must treat
   503 on create/delete as retryable — needs a paired client change, or an
   explicit decision to leave it to user-initiated retry.
3. D1 stays best-effort for the response, so esotk.com can lag a mutation by up
   to one alarm cycle. Needs maintainer sign-off, as does the standing rule that
   reconciliation never deletes D1 rows.
4. `updated_at` is not a true monotonic revision. Two writes inside the same
   timestamp on an unowned record order arbitrarily. The window is the shadow
   phase only and the ownership latch closes it at the first DO mutation, but it
   is not zero.
5. Journal entries are bounded (one per in-flight mutation, deleted on
   completion), but a persistently failing KV accumulates pending ops. Backoff
   caps retry cost, not entry count — a pending-op ceiling surfaced in `/health`
   is a follow-up, not a merge blocker.
6. `tomb:<id>` widens from string to object. Only this unmerged branch has ever
   written tombstones, so there is no production data in the old shape, but the
   read path must still tolerate it for DO storage created during local or
   preview testing.
```
