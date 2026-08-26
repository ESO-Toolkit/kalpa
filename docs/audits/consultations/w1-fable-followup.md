# Fable Follow-up — W1 Safe Migration After Sol Review

## Finding and acceptance criteria

The first W1 implementation made per-pack SQLite-backed `PackIndexDO` storage authoritative and kept KV as the public read mirror. Sol returned `REVISE` because the first post-deploy bootstrap reads `index:packs` once from eventually consistent KV. If that read misses a just-created pre-deploy pack, the DO permanently omits it and its next mirror removes it from listings.

The final design must:

- Preserve every live pre-deploy pack without trusting one potentially stale KV read.
- Never restore a pre-deploy deletion from stale KV.
- Avoid silent write loss during rollout.
- Be safe with `main` auto-deploying the Worker.
- Keep the Worker name, KV/Rust JSON shapes, and D1 schema unchanged.
- Either remain one deploy or establish a concretely safe staged order, as permitted by the master prompt when Fable and Sol explicitly validate it.

## Current code excerpts

```ts
private async ensureBootstrapped(): Promise<void> {
  if (await this.ctx.storage.get<boolean>(BOOTSTRAPPED_KEY)) return;
  const index = (await this.env.ESO_PACKS.get<PackIndex>(INDEX_KEY, "json")) ?? { packs: [] };
  for (const pack of index.packs) {
    await this.ctx.storage.put(this.packKey(pack.id), pack);
  }
  await this.ctx.storage.put(BOOTSTRAPPED_KEY, true);
}
```

```toml
name = "kalpa-pack-hub"
[[durable_objects.bindings]]
name = "PACK_INDEX"
class_name = "PackIndexDO"
[[migrations]]
tag = "v1"
new_sqlite_classes = ["PackIndexDO"]
```

## Sol review evidence

Sol verified five issues:

1. The bootstrap race above is high severity and makes the current commit unsafe to merge.
2. Stale vote/install requests can cross delete-and-recreate and mutate the new lifecycle.
3. Update/delete authorization trusts stale KV ownership rather than canonical DO ownership.
4. Restore preservation reads KV rather than being atomic inside the DO.
5. Account deletion leaves other users' vote records attached to removed pack IDs.

Items 2–5 are locally actionable. This consultation is specifically for item 1 and safe rollout order.

## Candidate designs

### Candidate A — Coordinated single-deploy maintenance bootstrap

- Deploy code with an uninitialized authority that rejects mutations with 503.
- An authenticated admin bootstrap waits for the old KV propagation window, imports a verified snapshot into DO storage, then flips a durable ready flag.
- Public reads remain available; mutation downtime is explicit and observable.

Concern: merge auto-deploys before the operator action, so the maintenance window and bootstrap invocation must be coordinated. Define exact verification and rollback.

### Candidate B — Two-phase shadow/backfill then authority flip

- Phase 1 keeps the current KV authority while dual-writing every mutation to DO storage and repeatedly reconciles/backfills until a durable high-water mark proves the shadow caught up.
- After an observation period and parity check, phase 2 switches mutations to DO authority.

Concern: there is no existing monotonic mutation sequence in KV. Define how a high-water mark can prove completeness without trusting a stale index, and whether phase 1 leaves any W1 defect worse than current production.

### Candidate C — Automatic fail-closed initialization gate

- First mutation starts a durable initialization state, rejects writes temporarily, waits beyond KV's cache window using a DO alarm, then imports and marks ready.
- Subsequent mutations succeed automatically; no manual admin call.

Concern: determine whether an alarm plus a quiet interval actually proves there were no pre-gate writes still propagating, how callers observe/retry 503, and whether deployment can guarantee the gate starts before any old-code mutation.

## Failure modes to evaluate

1. Old Worker mutation commits immediately before deployment.
2. New Worker receives a mutation immediately after deployment.
3. KV index and detail keys propagate at different times.
4. Bootstrap process/alarm/admin request crashes halfway.
5. Deployment is rolled back after DO storage contains newer mutations.
6. Two deployments overlap across isolates.
7. Admin bootstrap is invoked twice or with a stale snapshot.
8. A delete and slug reuse straddle the migration.
9. The first request after deploy is seed, restore, account deletion, vote, or install.

## Required output

Return only:

```text
DECISION:
1. Chosen design and numbered implementation/deployment steps

REJECTED:
1. Alternative and the concrete failure that rejects it

CRASH_RECOVERY:
1. Behavior after process kill, rollback, partial bootstrap, timeout, or stale read

TESTS:
1. Tests distinguishing a correct migration from a plausible but incorrect one

RISKS:
1. Remaining risks and required human decisions
```

## Consultation outcome — 2026-08-26

Fable chose Candidate B: deploy a continuously re-merging KV-authority shadow,
retain per-id tombstones, and perform a later explicit authority flip only after
two clean parity observations separated by more than the KV cache window. It
rejected one-shot and timed bootstrap gates because KV propagation has no
provable upper bound.

Implementation requirements adopted from the review:

1. Read the KV index with the 30-second minimum cache TTL on every KV-authority mutation.
2. Compare the DO shadow with D1 and a post-deploy `backup:latest` witness set.
3. Block the authority flip while an untombstoned witness is missing.
4. Permit only explicit, admin-authenticated adoption from an independently propagated `pack:<id>` detail record; never auto-adopt a D1 row because it may be a zombie from a failed delete.
5. Treat rollback after the `do` authority flip as restore-from-backup, not a flag-only rollback.

Fable also required tests that distinguish repeated backfill from a latched
bootstrap, prove tombstones reject stale re-import, prove a failed parity check
leaves KV authority active, and cover explicit detail adoption.
