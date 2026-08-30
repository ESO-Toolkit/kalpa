# Fable Consultation — W2 D1 Mirror Reliability

## Finding and acceptance criteria

Pack Hub awaits D1 mirror operations but catches and logs failures. A failed
upsert, tag replacement, vote update, or delete can therefore leave a permanent
missing, stale, or zombie row in the shared `roster-hub-db` database.

The fix must leave a durable diagnostic breadcrumb for mirror failures and add
a reconciliation path that:

- compares authoritative Pack Hub state with only the `packs` and `pack_tags`
  rows owned by this Worker;
- treats a valid empty authoritative corpus as data, while treating a failed
  authority read as an error that permits no D1 mutation;
- restores missing/stale rows and removes zombies;
- has mutation-count safety limits and fails closed on suspicious divergence;
- requires no D1 schema change and never modifies unrelated shared tables;
- supports an initial dry-run/log-only mode;
- remains unmerged if deletion-capable until a maintainer explicitly approves.

Required tests cover a missing D1 pack, a D1 zombie, an empty authoritative
corpus, an authority-read failure, a partial D1 failure with durable error state,
and proof that unrelated shared tables are untouched.

## Repository constraints

- Worker name remains `kalpa-pack-hub`; merge to `main` auto-deploys it.
- `roster-hub-db` is shared with `roster-hub-api`; no schema or unrelated-table
  changes are allowed.
- W1 makes the singleton `PackIndexDO` the mutation authority after a guarded
  migration. `getIndex()` returns its merged/canonical corpus. A thrown RPC is
  distinct from the valid `{ packs: [] }` result.
- The Worker currently has a daily scheduled backup and no enabled production
  observability. KV is available for durable operational breadcrumbs.
- The initial shipped reconciliation mode must be log-only/dry-run. Any mode
  capable of deleting D1 rows requires explicit maintainer approval before merge.
- Keep subrequest use bounded and do not add D1 tables, columns, migrations, or
  dependencies.

## Current code excerpt 1 — swallowed mirror failures

```ts
async function d1UpsertPack(env: Env, pack: Pack): Promise<void> {
  if (!env.ROSTER_HUB_DB) return;
  const isPublished = (pack.status ?? "published") === "published";
  try {
    if (isPublished) {
      await env.ROSTER_HUB_DB.prepare(`INSERT INTO packs (...) VALUES (...)
        ON CONFLICT(id) DO UPDATE SET ...`).bind(...).run();
      await env.ROSTER_HUB_DB.batch([
        env.ROSTER_HUB_DB.prepare("DELETE FROM pack_tags WHERE pack_id = ?").bind(pack.id),
        ...pack.tags.map((tag) => env.ROSTER_HUB_DB!.prepare(
          "INSERT OR IGNORE INTO pack_tags (pack_id, tag) VALUES (?, ?)"
        ).bind(pack.id, tag)),
      ]);
    } else {
      await env.ROSTER_HUB_DB.batch([
        env.ROSTER_HUB_DB.prepare("DELETE FROM pack_tags WHERE pack_id = ?").bind(pack.id),
        env.ROSTER_HUB_DB.prepare("DELETE FROM packs WHERE id = ?").bind(pack.id),
      ]);
    }
  } catch (err) {
    console.error(`D1 sync failed [${pack.id}]:`, err);
  }
}

async function d1UpdateVoteCount(env: Env, id: string, voteCount: number): Promise<void> {
  if (!env.ROSTER_HUB_DB) return;
  try {
    await env.ROSTER_HUB_DB.prepare(
      "UPDATE packs SET vote_count = ? WHERE id = ?"
    ).bind(voteCount, id).run();
  } catch (err) {
    console.error(`D1 vote_count sync failed [${id}]:`, err);
  }
}
```

## Recorded Fable response

```text
DECISION:
1. Choose Candidate A, with inline failure breadcrumbs but no repair queue. Put reconciliation in a dedicated module and invoke it independently after scheduled backup so a valid empty index still reaches it.
2. Persist `d1-mirror:last_error` for caught inline upsert, counter, and delete failures. Breadcrumb writes must be nested best-effort operations and must not turn D1 mirror outages into Pack Hub write failures.
3. Resolve the mode by exact match only. `apply` mutates, `off` skips, and `dry-run` plans/logs. Missing or invalid values fail to dry-run; invalid non-empty values are reported.
4. Read and validate Durable Object authority before preparing any D1 statement. A throw or malformed value records an authority-stage error and performs no D1 work; `{ packs: [] }` is valid data.
5. Read only explicit columns from `packs` and `pack_tags`. Do not compare generated timestamps or fields D1 does not mirror. Use the inline mirror's exact redaction and addon serializer for expected rows.
6. Build the complete deterministic plan before mutation. Bound corpus, upserts, tag replacements, deletes, total mutations, and suspicious delete ratios. Exceeding a limit rejects the whole plan rather than truncating it.
7. Gate deletion on durable proof that this Worker owned the ID. Rows without ownership proof are counted as `unowned_extra` and never deleted.
8. Apply only in exact `apply` mode, in upsert, tag-replacement, then delete order. Keep tag replacement in a D1 batch and recheck authority immediately before every deletion.
9. Persist `d1-recon:last` for every normal run and `d1-recon:last_error` with planned/applied counts for every failed stage.
10. Keep the deletion-capable PR unmerged until explicit maintainer approval, even though its checked-in deployment mode is dry-run.

REJECTED:
1. Candidate B: KV has no atomic append, so a repair queue loses concurrent failure IDs and cannot discover stale writes that returned success or historical drift.
2. Candidate C: a multi-run cursor cannot prove a stable corpus without an authority generation or schema change; stale/incomplete sweeps can misclassify live rows as zombies.
3. Propagating D1 errors to callers: D1 is a soft website mirror, so coupling its availability to authoritative Pack Hub writes creates an unnecessary write outage.

CRASH_RECOVERY:
1. Upserts and per-pack tag batches are idempotent; a killed or timed-out run is recomputed from current authority on the next cron.
2. Deletes run last, so interruption before deletion leaves excess data rather than missing authoritative data.
3. A partial apply records exact planned/applied counts. Stored plans/cursors are never trusted, so there is no stale marker to recover.
4. If a breadcrumb write fails, log it without changing mutation results; reconciliation depends on comparing state, not on breadcrumb history.
5. Overlapping cron runs converge through idempotent writes and a fresh authority check before deletion.

TESTS:
1. Authority throw or malformed result prepares no D1 SQL; valid empty authority remains distinguishable and reaches planning.
2. Missing authoritative rows are restored; proven-owned zombies are deleted; unowned extras are never deleted.
3. Every prepared statement targets only `packs` or `pack_tags`, and unrelated shared tables remain untouched.
4. Each mutation cap fails closed at the boundary plus one without applying a truncated prefix.
5. Anonymous-author redaction and addon JSON serialization match the inline mirror; generated timestamps cause no drift.
6. Partial D1 failure records applied counts and a later run converges idempotently.
7. A pack that reappears in authority immediately before deletion is skipped.
8. Only exact `apply` mutates; missing, misspelled, or differently cased modes do not.
9. A failed breadcrumb write cannot change the caller response, and tag replacement failure cannot leave an empty half-applied tag set.

RISKS:
1. Maintainer approval is required before merge because the code contains deletion capability, and separate approval is required before changing production mode to `apply`. Fable recommends a dry-run soak and manual spot checks first.
2. A historical zombie without a surviving ownership witness remains `unowned_extra` and requires manual adjudication; retaining a stale row is safer than deleting a website-owned row.
3. Initial non-empty plans may expose comparator bugs; spot-check representative IDs before apply.
4. Full-table reads have a corpus ceiling; hitting it should produce a loud skip and trigger a versioned design revisit rather than truncation.
5. Confirm with the website owner that it does not independently own Pack Hub rows or vote counts before enabling apply.
```

Executor verification: the ownership recommendation was implemented using the
DO's canonical live IDs plus its durable tombstones, avoiding D1 itself as proof
of D1 ownership. The blanket empty-authority delete rejection was narrowed to a
five-delete cap because W2 explicitly requires valid empty authority to reconcile
owned rows; larger empty-corpus divergence still fails closed.

## Current code excerpt 2 — authority and scheduler

```ts
async getIndex(): Promise<PackIndex> {
  return this.ctx.blockConcurrencyWhile(async () => ({ packs: await this.loadPacks() }));
}

private async loadPacks(): Promise<Pack[]> {
  const authority = await this.getAuthority();
  if (authority === "do") return this.getStoredPacks();
  const kv = await this.readKvIndex();
  await this.mergeFromKv(kv);
  return this.getStoredPacks();
}

async scheduled(_controller, env, _ctx): Promise<void> {
  try {
    await handleScheduled(env);
  } catch (err) {
    console.error("Scheduled backup failed:", err);
    try {
      await env.ESO_PACKS.put("backup:last_error", JSON.stringify({
        at: new Date().toISOString(),
        message: err instanceof Error ? err.message : String(err),
      }));
    } catch (writeErr) {
      console.error("Failed to record backup:last_error:", writeErr);
    }
  }
}
```

## Candidate designs

### Candidate A — scheduled full comparison, explicit mode, bounded plan

- After the daily backup, read the authoritative corpus once from `PackIndexDO`.
  A thrown or malformed result aborts before any D1 statement is prepared.
- Read only `packs` and `pack_tags`, build a deterministic plan of pack upserts,
  tag replacements, and zombie deletions, then reject the entire plan if any
  category or total exceeds fixed limits.
- Always persist a compact reconciliation result/error breadcrumb in KV. Mirror
  helpers also write `d1-mirror:last_error` on caught failures.
- `D1_RECONCILIATION_MODE` defaults to `dry-run`; only exact `apply` executes the
  bounded plan. The first PR remains draft/unmerged pending approval because the
  code contains a deletion-capable mode even though deployment defaults dry-run.

Concern: a full table read may exceed row/subrequest limits as the corpus grows;
mutation batches may partially succeed, so a durable error must retain the
planned/applied counts and the next run must converge idempotently.

### Candidate B — breadcrumb-driven targeted repair plus periodic zombie scan

- Every failed inline mirror appends the pack ID to a KV repair queue. Scheduled
  reconciliation repairs only queued IDs, then separately scans D1 IDs for
  zombies against the authoritative ID set.
- Use the same dry-run/apply switch and safety cap for the zombie scan.

Concern: KV cannot atomically append a queue, so concurrent failures can overwrite
each other; failures predating this deployment have no queue entry; and a stale
row whose inline update falsely reports success may never be compared deeply.

### Candidate C — paginated incremental cursor reconciliation

- Persist a durable cursor and reconcile bounded pages of authoritative packs and
  D1 IDs over multiple cron runs. A generation marker identifies a completed
  sweep before zombie deletion is permitted.
- Dry-run and apply modes share the same page planner.

Concern: the corpus can mutate during a multi-run sweep, making snapshot/generation
proof complex without schema or authority versioning. A stale cursor or partial
sweep must never reinterpret unseen authoritative rows as zombies.

## Failure modes to evaluate

1. `PackIndexDO.getIndex()` throws, times out, or returns malformed data.
2. The authoritative result is valid and contains zero packs.
3. D1 contains a Pack Hub zombie absent from authority.
4. D1 lacks an authoritative pack, has stale scalar fields, or has stale tags.
5. D1 read fails before planning; no writes or deletes may run.
6. Divergence exceeds a mutation cap by one and by a very large amount.
7. A D1 batch partially succeeds and then throws.
8. Writing the diagnostic breadcrumb itself fails.
9. Two scheduled events overlap or a pack mutation occurs during reconciliation.
10. A non-Pack-Hub shared table exists and must receive no prepared statement.
11. D1 has a row not actually owned by Pack Hub despite occupying `packs`.
12. The mode binding is missing, misspelled, or unexpectedly set.
13. Crash after some idempotent upserts but before zombie deletes.
14. Tag delete succeeds but tag insert fails.

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
