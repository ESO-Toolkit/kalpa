# Fable Reconsultation — W2 After Two Verified Sol Revisions

## Finding and acceptance criteria

W2 repairs permanent drift in the Pack Hub mirror after awaited D1 writes fail.
Reconciliation must compare authoritative Pack Hub state with only the owned
`packs` and `pack_tags` rows, distinguish a valid empty corpus from an authority
read failure, leave durable diagnostics, fail closed on suspicious divergence,
and support a dry-run first deployment. It must not change D1 schema or touch
unrelated shared tables.

This is a mandatory architectural reconsultation. Sol returned two verified
`REVISE` verdicts. Implementation is preserved but W2 is blocked until Fable
assesses whether the corrected final architecture is sound.

## Repository and deployment constraints

- `roster-hub-db` is shared with `roster-hub-api`. This Worker may read and
  mutate only the existing `packs` and `pack_tags` tables and may not assume it
  owns an arbitrary row merely because that row uses the shared table.
- W2 is stacked on W1. W1's singleton `PackIndexDO` becomes authoritative only
  after its guarded shadow migration. While W1 is in shadow mode,
  `loadPacks()` repeatedly merges KV witnesses into DO storage; in DO mode it
  reads stored packs. Durable DO tombstones and live IDs are the only W2
  deletion-ownership witnesses.
- W1 must merge before W2. W2 must not be rebased onto `main` without the W1
  authority/lifecycle implementation.
- Merge to `main` auto-deploys `kalpa-pack-hub`. Checked-in W2 mode is exact
  `dry-run`; exact `apply` is the only mutation-capable setting.
- A maintainer must explicitly approve merging deletion-capable code. A second,
  later approval is required before changing production mode to `apply` after a
  dry-run soak and manual plan inspection.
- No real deployment, schema change, new dependency, or unrelated cleanup is in
  scope.

## Original Fable decision

Decision D-W2-1 chose one bounded scheduled full comparison. It required a
validated DO authority snapshot before D1 SQL; explicit-column reads of only
`packs` and `pack_tags`; a deterministic complete plan rejected rather than
truncated when any cap is exceeded; hard deletion ownership proof; exact mode
matching; durable result/error breadcrumbs; idempotent upsert, tag replacement,
then deletion; and a fresh authority check immediately before deletion.

It rejected a KV repair queue because KV cannot atomically append concurrent
failure IDs or discover historical drift, and rejected a multi-run cursor
because W1 exposes no stable authority generation proving a complete sweep.

## Sol review 1 — exact verified findings and pre-fix evidence

Verdict: `REVISE`.

1. Authority validation accepted any string status. Planning treated every
   status other than `published` as deletion. A corrupt or future status such as
   `archived` could therefore delete a valid D1 row instead of failing closed.
   Pre-fix evidence:

   ```ts
   result.packs.some((pack) =>
     !pack || typeof pack.id !== "string" || !pack.id ||
     typeof pack.status !== "string"
   )

   if (pack.status !== "published") {
     if (actual) deletes.add(pack.id);
     continue;
   }
   ```

2. Planning enumerated only D1 `packs` rows when finding extras. A historical
   partial delete could remove the parent row but leave `pack_tags`; that
   tag-only zombie was invisible and permanent.

Verified corrections: authority validation now permits only exact `draft` or
`published` and validates the complete mirrored Pack shape. Planning separately
enumerates tag IDs and deletes only draft/live-owned or tombstone-owned orphans;
unowned tag IDs are retained and reported. Boundary-plus-one tests cover every
safety cap.

## Sol follow-up — exact verified findings and pre-fix evidence

Verdict: `REVISE`.

1. The plan held old Pack objects, but upserts and tag replacements executed
   directly against D1 outside `PackIndexDO`. A pack could be deleted and the
   same slug recreated after planning; the stale write would then overwrite the
   new lifecycle's D1 row/tags. Only deletion had a late liveness RPC, and its
   check and D1 delete were also initially separate operations.

   ```ts
   for (const pack of plan.upserts) await upsert(env, pack);
   for (const pack of plan.tag_replacements) await replaceTags(env, pack);
   const current = await stub.getPack(id);
   if (current?.status === "published") continue;
   await remove(env, id);
   ```

2. The first tag-orphan correction skipped tags for an authoritative draft: it
   only deleted tag-only IDs found in tombstones and therefore left a draft's
   stale website tags behind.

Verified corrections: reconciliation now sends the ID, planned `created_at`
lifecycle, and requested operations to `PackIndexDO`. Under the same serialized
lifecycle gate used by ordinary create/update/delete/vote mirror writes, the DO
reloads current authority, rejects a missing/draft/different-lifecycle write,
and writes current Pack data rather than the stale planned object. Deletion is
also rechecked and executed within the DO gate. Tag-only authoritative drafts
are planned for deletion; unowned IDs remain untouched.

## Current code excerpt 1 — planning, ownership, and fail-closed bounds

```ts
const LIMITS = {
  corpus: 2_000, upserts: 100, tags: 100,
  deletes: 25, emptyDeletes: 5, total: 150,
};

const expected = new Map(authority.packs.map((pack) => [pack.id, pack]));
const owned = new Set([...expected.keys(), ...authority.tombstones]);

for (const pack of authority.packs) {
  const actual = rows.get(pack.id);
  if (pack.status !== "published") {
    if (actual) deletes.add(pack.id);
    continue;
  }
  if (!actual || !rowsEqual(actual, toD1PackRow(pack))) upserts.push(pack);
  if (!actual || !sameTags(tags.get(pack.id) ?? [], pack.tags)) replacements.push(pack);
}
for (const { id } of d1Rows) {
  if (expected.has(id)) continue;
  if (owned.has(id)) deletes.add(id);
  else unowned.add(id);
}
for (const { pack_id: id } of d1Tags) {
  const pack = expected.get(id);
  if (pack?.status === "published") continue;
  if (pack || owned.has(id)) deletes.add(id);
  else unowned.add(id);
}

if (count.upserts > LIMITS.upserts) return "upserts";
if (count.tag_replacements > LIMITS.tags) return "tag-replacements";
if (count.deletes > LIMITS.deletes) return "deletes";
if (count.total > LIMITS.total) return "total";
if (authorityCount === 0 && count.deletes > LIMITS.emptyDeletes)
  return "empty-authority-deletes";
if (authorityCount > 0 &&
    count.deletes > Math.max(5, Math.ceil(authorityCount * 0.1)))
  return "delete-ratio";
```

## Recorded Fable response

```text
DECISION:
Candidate A — accept corrected D-W2-1, with four bounded hardening steps before the maintainer's merge approval.

1. Gate deletion on W1 authority mode. While W1 is still in shadow mode, plan/report everything but retain every deletion candidate as unowned. Upserts and tag replacements remain allowed because the DO gate writes only authority-current data.
2. Bound both D1 reads: select at most the authority corpus ceiling plus one pack row and a fixed tag ceiling plus one; reject before planning with `d1-corpus` or `d1-tags` when exceeded.
3. Single-flight scheduled reconciliation with a DO-owned expiring lease. A concurrent run records `skipped-overlap`; use a token so a stale run cannot release a newer lease.
4. Keep one `blockConcurrencyWhile` gate per pack rather than holding one gate for the whole plan. Keep tag replacement in its existing D1 batch; partial pack/tag convergence is repaired on the next run.
5. Preserve the merge sequence: W1 first, W2 in checked-in `dry-run`, a human-inspected soak, then separate explicit approval for `apply`.

REJECTED:
1. Candidate B fails the W2 zombie-removal acceptance criterion and adds another design/deploy cycle without a safety property beyond ownership proof, caps, and dry-run soak.
2. Candidate C requires a shared schema migration and W1 generation contract outside W2 constraints.
3. Applying stale planned Pack objects can overwrite a recreated slug lifecycle; the corrected DO-gated `created_at` match is required.
4. A whole-plan DO gate can block ordinary mutations for the entire plan and risks a blocking-limit reset without useful partial accounting.

CRASH_RECOVERY:
1. Every operation is independently idempotent and is re-derived from fresh authority and D1 reads after a crash; no plan or cursor is resumed.
2. A successful upsert followed by failed tag replacement leaves a converging state. The tag batch is atomic and the next run repairs it.
3. The single-flight lease expires after roughly two scheduled intervals; a token prevents a stale release from clearing a newer lease.
4. Authority or D1 read failure records an error and performs zero mutations.
5. Losing tombstones degrades zombies to retained/unowned rather than unsafe deletion. Tombstones must not be pruned while a corresponding D1 row may remain.

TESTS:
1. Shadow mode plus a tombstone-owned zombie retains/reports it; DO mode deletes the same fixture.
2. Pack and tag D1 ceilings plus one reject before planning or mutation.
3. Concurrent reconciliations permit one lease holder and return `skipped-overlap` for the other.
4. Delete/recreate as draft or published preserves the correct new lifecycle behavior.
5. A missing live witness without a tombstone remains unowned and retained.
6. Draft tag-only orphans are removed while never-owned tag IDs remain unowned.
7. Applied breadcrumbs count only DO-confirmed operations.

RISKS:
1. Confirm with the website owner that it never independently reuses a Pack Hub ID in `packs`; a historical tombstone otherwise cannot distinguish that collision without a future ownership column.
2. Per-operation D1 round trips hold the DO gate and may cause cron-correlated latency; monitor before raising plan limits.
3. Safety caps are policy. A dry-run `limit_hit` is a design signal, not permission to raise the cap.
4. Merge and `apply` remain two distinct maintainer approvals and cannot be inferred from CI.
5. Exceeding fixed corpus ceilings fails closed and requires a separate versioned/paginated design.
```

## Executor verification and implementation

Code inspection confirmed all three gaps. D-W2-2 therefore extends D-W2-1 with
an exact W1 authority-mode field in the reconciliation snapshot, deletion
suppression during KV shadow mode, `LIMIT ceiling+1` reads for both shared D1
tables, and a token-owned 48-hour DO lease. The lease release is token-checked,
so an expired older run cannot release a newer holder.

Failing-before evidence captured three exact failures: shadow mode planned one
delete instead of zero; 2,001 D1 pack rows completed instead of returning
`plan-rejected`; and two overlapping calls both completed. The new tests, plus a
20,001-tag boundary and real-DO lease/authority-mode tests, pass. The complete
Worker suite now has 203 tests.

Before these operations, the implementation validates every Pack field and
exact status, rejects authority corpora over 2,000, and completes both explicit
D1 reads before constructing or applying a plan. Authority or D1-read failure
returns after writing a best-effort error breadcrumb and performs no mutation.

## Current code excerpt 2 — serialized lifecycle-safe apply

```ts
for (const pack of plan.upserts) {
  const replace = plan.tag_replacements.some(({ id }) => id === pack.id);
  const applied = await stub.reconcileWriteD1(
    pack.id, pack.created_at, true, replace
  );
  // count only operations the DO actually applied
}
for (const pack of plan.tag_replacements) {
  if (upsertIds.has(pack.id)) continue;
  await stub.reconcileWriteD1(pack.id, pack.created_at, false, true);
}
for (const id of plan.deletes) await stub.reconcileDeleteD1(id);

async reconcileWriteD1(id, expectedLifecycle, writePack, writeTags) {
  return this.ctx.blockConcurrencyWhile(async () => {
    await this.loadPacks();
    const current = await this.ctx.storage.get<Pack>(this.packKey(id));
    if (!current || current.status !== "published" ||
        current.created_at !== expectedLifecycle || !this.env.ROSTER_HUB_DB)
      return { upserted: false, tags_replaced: false };
    if (writePack) await this.upsertD1Pack(current);
    if (writeTags) await this.replaceD1Tags(current);
    return { upserted: writePack, tags_replaced: writeTags };
  });
}

async reconcileDeleteD1(id) {
  return this.ctx.blockConcurrencyWhile(async () => {
    await this.loadPacks();
    const current = await this.ctx.storage.get<Pack>(this.packKey(id));
    if (current?.status === "published" || !this.env.ROSTER_HUB_DB) return false;
    await this.env.ROSTER_HUB_DB.batch([
      this.env.ROSTER_HUB_DB.prepare(
        "DELETE FROM pack_tags WHERE pack_id = ?"
      ).bind(id),
      this.env.ROSTER_HUB_DB.prepare(
        "DELETE FROM packs WHERE id = ?"
      ).bind(id),
    ]);
    return true;
  });
}
```

Ordinary mutations now perform their inline D1 operations inside the same DO
serialization boundary. D1 failures remain soft for authoritative Pack Hub
writes and record `d1-mirror:last_error`; the scheduled comparison later
converges them.

## Final implementation and test evidence

The W2 diff adds a dedicated reconciliation module, lifecycle-safe DO RPCs,
independent scheduled invocation, a `dry-run` Wrangler default, two Env fields,
and focused tests. It changes no schema, public Worker/Rust JSON contract, or
unrelated table SQL.

The final suite has 197 passing Worker tests. Focused coverage includes missing
D1 restore, owned/unowned pack zombies, owned/unowned tag-only zombies,
authoritative drafts, valid empty authority, thrown/malformed authority, D1-read
failure, exact mode parsing, exact and plus-one cases for all limits, partial
apply breadcrumbs, stale slug lifecycle rejection in both coordinator and real
DO tests, inline breadcrumb failure isolation, serializer/redaction parity, and
an SQL table whitelist. Worker check and Wrangler dry-run pass; Wrangler name
remains `kalpa-pack-hub`. No real deployment ran.

## Candidate decisions for reconsultation

### Candidate A — accept corrected D-W2-1

Keep the current bounded full comparison and serialized lifecycle-safe apply.
Leave the PR draft and mode `dry-run` until explicit maintainer approval, then
inspect dry-run breadcrumbs during a soak before separately approving `apply`.

### Candidate B — remove all deletion capability from this PR

Ship breadcrumbs and upsert/tag repair only. Add deletion later behind a manual
admin workflow that reviews individual tombstone-proven IDs. This reduces first
merge risk but does not satisfy W2 zombie reconciliation and leaves stale rows
until a second design and deployment.

### Candidate C — add an authority generation or D1 ownership column

Version snapshots and mark every owned D1 row, then paginate reconciliation.
This could make large future corpora easier to sweep, but it requires W1
authority changes and/or a shared D1 schema migration coordinated with the
website, both outside W2 constraints.

## Failure modes to evaluate

1. W1 shadow authority misses a delayed KV witness during one scheduled run.
2. A pack is deleted/recreated before a planned upsert, tag replacement, or
   delete executes; the recreated lifecycle may be draft or published.
3. Two scheduled events overlap, or ordinary mutation interleaves with apply.
4. D1 contains an unowned website row or tag with a colliding/shared-table ID.
5. Authority is validly empty; authority RPC fails; D1 read partially fails.
6. Divergence hits a cap exactly or exceeds it by one.
7. Upsert succeeds and tag batch fails; tag delete succeeds and insert fails;
   process dies partway through the plan.
8. Tombstones accumulate indefinitely or are lost through a future migration.
9. `blockConcurrencyWhile` holds the DO lifecycle gate during slow D1 I/O.
10. Corpus exceeds the fixed full-read ceiling.

## Required output

Return only:

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
