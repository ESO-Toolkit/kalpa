# Fable Consultation — W1 Atomic Worker Consistency

## Finding and acceptance criteria

Three coupled defects exist in the Pack Hub Worker:

1. A vote/install request may carry a stale KV detail body into `applyCounter`; if the DO index no longer contains the pack, the seed is appended and recreates a deleted pack.
2. Create checks slug uniqueness outside the DO, while `addPack` enforces only the author cap. Concurrent same-slug creates can both succeed.
3. Update builds a whole `Pack` from a KV detail read and then replaces the DO index entry, so a vote/install serialized before the update can have its fresh counter overwritten.

Acceptance criteria:

- A stale KV detail body cannot recreate a deleted pack.
- ID uniqueness is enforced inside the Durable Object mutation.
- Updates preserve the latest DO-owned vote/install counters.
- Duplicate creation returns HTTP 409.
- Existing successful JSON remains compatible with Rust `HubPack`.
- Mutation checks remain inside `blockConcurrencyWhile`.
- The three changes must be safe to deploy together when merged to `main`.

Required tests cover delete-then-vote/install with a stale seed, concurrent duplicate create, and update racing vote/install.

## Repository constraints

- Worker name must remain `kalpa-pack-hub`; merge to `main` deploys automatically.
- KV currently serves list/detail reads. One global `PackIndexDO` serializes mutations.
- No shared-D1 schema change is allowed.
- Response field names must remain compatible with the Rust `HubPack` deserializer.
- Keep the change minimal and avoid an unsafe intermediate deployment state.

## Current code excerpt 1 — DO mutations

```ts
async addPack(pack: Pack, maxPerAuthor?: number): Promise<{ ok: boolean }> {
  return this.ctx.blockConcurrencyWhile(async () => {
    const index = await this.getIndex();
    if (maxPerAuthor !== undefined) {
      const owned = index.packs.filter((p) => p.author_id === pack.author_id).length;
      if (owned >= maxPerAuthor) return { ok: false };
    }
    index.packs.push(pack);
    await this.putIndex(index);
    return { ok: true };
  });
}

async updatePack(id: string, pack: Pack): Promise<void> {
  await this.ctx.blockConcurrencyWhile(async () => {
    const index = await this.getIndex();
    const pos = index.packs.findIndex((p) => p.id === id);
    if (pos >= 0) index.packs[pos] = pack;
    else index.packs.push(pack);
    await this.putIndex(index);
  });
}

private async applyCounter(id: string, field: "vote_count" | "install_count", delta: number,
  seed?: Pack | null): Promise<Pack | null> {
  const index = await this.getIndex();
  const pos = index.packs.findIndex((p) => p.id === id);
  if (pos < 0) {
    if (!seed) return null;
    const healed = { ...seed, [field]: Math.max(0, (seed[field] ?? 0) + delta) };
    index.packs.push(healed);
    await this.putIndex(index);
    await this.env.ESO_PACKS.put(`pack:${id}`, JSON.stringify(healed));
    return healed;
  }
  const pack = index.packs[pos];
  pack[field] = Math.max(0, (pack[field] ?? 0) + delta);
  await this.putIndex(index);
  await this.env.ESO_PACKS.put(`pack:${id}`, JSON.stringify(pack));
  return pack;
}

private async getIndex(): Promise<PackIndex> {
  return (await this.env.ESO_PACKS.get<PackIndex>("index:packs", "json")) ?? { packs: [] };
}
```

## Current code excerpt 2 — handlers

```ts
let id = typeof input.id === "string" && input.id.length > 0
  ? input.id : slugify(input.title as string);
const existing = await getPack(env, id, { fresh: true });
if (existing) id = `${id}-${Date.now().toString(36)}`;
const added = await getPackIndexDO(env).addPack(pack, MAX_PACKS_PER_USER);
if (!added.ok) return json(request, { error: "Maximum ..." }, 429);
await putPack(env, pack);

// Update constructs `pack` using counters from a KV detail read.
await putPack(env, pack);
await getPackIndexDO(env).updatePack(id, pack);

// Vote/install pass the handler's potentially stale KV detail as seed.
const result = await getPackIndexDO(env).toggleVote(id, userId, pack);
const updated = await getPackIndexDO(env).bumpPackCounter(id, "install_count", 1, pack);
```

## Candidate designs

### Candidate A — DO storage is authoritative for the index

- Lazily initialize a DO-storage index from KV once, then persist every mutation to DO storage first and mirror the resulting canonical index/body to KV inside the same concurrency gate.
- `addPack` checks ID and author cap inside the gate and returns a typed reason (`duplicate` or `limit`).
- `updatePack` finds the canonical entry, merges editable fields while preserving canonical counters and creation identity, mirrors it, and returns it; it never upserts a missing ID.
- Counter methods operate only on the canonical entry and never accept a seed that can create state.
- `replaceIndex` updates both authoritative DO storage and KV for seed/restore.

Concern: migration/bootstrap correctness when an already-deployed DO has no storage state, and ordering/failure behavior when DO storage succeeds but KV mirroring fails.

### Candidate B — Keep KV index canonical and add DO tombstones

- Persist per-ID tombstones/existence records in DO storage.
- Reject seed healing for tombstoned IDs; clear a tombstone only during an intentional successful create.
- Add ID uniqueness inside `addPack`.
- Make `updatePack` merge canonical counters from the index rather than replace them.

Concern: KV eventual consistency can still make the index read stale between serialized mutations; a recreated slug can also allow an old queued operation to affect the new lifecycle unless generations are added.

### Candidate C — Keep KV canonical, remove healing, and merge only

- Remove `seed` and return null whenever the ID is absent from the current index.
- Check duplicates in `addPack` and preserve counters in `updatePack`.

Concern: simplest change, but correctness still assumes DO-origin KV reads observe prior DO writes. If KV can return stale data, serialization alone does not establish an authoritative state machine.

## Failure modes to evaluate

1. Process/DO eviction after a mutation but before a later request.
2. KV returns an older index after the DO previously wrote a newer one.
3. Delete completes, then a stale vote/install RPC arrives with an old detail body.
4. Two creates for one ID enter concurrently.
5. Update and counter bump arrive in either order.
6. DO storage write succeeds but KV index/body mirroring fails, or vice versa.
7. First request after deployment bootstraps while another mutation is queued.
8. Admin seed/restore replaces the index while regular mutations are queued.
9. A deleted slug is intentionally reused.
10. Existing clients deserialize success and error responses.

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
