# Fable Reconsultation — W3 After Two Verified Sol Revisions

## Finding and acceptance criteria

W3 hardens low-severity Worker edge paths after W1 established lifecycle-safe
PackIndexDO authority and W2 added lifecycle-gated D1 reconciliation. It must:

- count request-body bytes rather than UTF-16 units and stop buffering once the
  256 KiB JSON limit is exceeded where the Workers stream API permits;
- replace wholesale vote/auth memo clearing with bounded oldest-entry eviction;
- make install counting atomic or explicitly bounded/idempotent;
- prevent stale list responses from repopulating cache after invalidation; and
- decide whether public `/health` should expose corpus size or backup freshness.

This is a mandatory architectural reconsultation. Sol returned two verified
`REVISE` verdicts. The implementation and draft PR are preserved, but W3 is
marked `blocked` until Fable assesses the corrected final architecture. No
further W3 code may be changed before this decision.

## Repository and deployment constraints

- W3 is stacked on W2 decision D-W2-2 and W1 decisions D-W1-1/D-W1-2. W3 must
  preserve the continuously merged KV shadow, explicit parity-gated DO flip,
  tombstones, per-pack lifecycle gates, and authority-gated D1 reconciliation.
- The W2 admin authority request now accepts up to 100 manually adjudicated
  `unowned_d1_ids`. W3's byte-bounded JSON reader must preserve that exact
  validation and the D-W2-2 ownership rules.
- Merge to `main` auto-deploys `kalpa-pack-hub`; no intermediate unsafe state,
  real deployment, authority flip, D1 schema change, or Worker rename is allowed.
- Pack list/detail/vote/install JSON shapes consumed by Rust must remain stable.
- `roster-hub-db` is shared; W3 must not change schema or reconciliation scope.
- H3 (whether the Worker package version should be synchronized) is a separate
  child/hygiene dependency. W3 must not silently bump or redefine package version
  policy.
- The W2 branch now includes D-W2-2 and its fresh Sol APPROVE. W3 has been
  merged forward from exact W2 commit `4b2c18d0` without rebasing or rewriting.

## Original executor decision D-W3-1

1. Stream every JSON body with `ReadableStreamDefaultReader`, add each chunk's
   `byteLength`, cancel and return 413 above 256 KiB, then decode and parse only
   the bounded bytes.
2. Use insertion-ordered Maps with delete/reinsert on access and evict exactly
   the oldest entry at a fixed bound. Generation-gate asynchronous auth fills so
   a reset during hashing/fetch cannot repopulate stale entries.
3. Remove manual Cache API list storage entirely. It cannot be invalidated
   coherently across isolates. Retain only a 30-second public `Cache-Control`
   header for anonymous default-list responses; authenticated/personalized lists
   receive no cache lifetime.
4. Serialize install identity claim and counter increment in one DO storage
   transaction. HMAC the client IP with `ADMIN_API_KEY`, keep a fixed 5,000-slot
   oldest-eviction ring with a one-hour alarm, bind markers to pack `created_at`,
   delete markers with the pack lifecycle, and mirror even duplicate retries so
   a previously failed KV mirror heals.
5. Honor a pre-W3 `install-rate:<pack>:<ip>` KV key only when the independently
   read KV detail and canonical DO pack have the same `created_at`, so a stale key
   cannot suppress the first install of a recreated slug.
6. Public `/health` exposes only status, KV reachability, and timestamp. Backup
   failure breadcrumbs remain durable operator state rather than public metadata.

## Sol review 1 — verified findings and corrections

Verdict: `REVISE`.

1. The first attempted canonical Cache API key still could not coordinate
   invalidation across Worker isolates; a stale response could be inserted after
   another isolate invalidated. Correction: remove manual list Cache API storage.
2. A shared cached list response could alias viewer-derived fields (`user_voted`,
   anonymous-owner redaction, reflected CORS origin). Correction: no manual
   shared response; authenticated lists receive `max-age=0`.
3. Seed and migration-adopt were missing cache invalidation. Correction became
   structural: with no manual list storage, mutation invalidation is a no-op
   compatibility hook and cannot miss a mutation site.
4. The first install limiter design separated claim from increment, so failures
   or concurrency could lose/duplicate counts. Correction: one DO transaction
   persists claim, ring slot, counter, sequence, and alarm.
5. Durable raw/IP-derived identities created privacy retention and offline IPv4
   enumeration risk. Correction: admin-keyed HMAC identities, one-hour cleanup,
   fixed storage bound, and lifecycle deletion.
6. Replacing KV limiting immediately allowed every still-live legacy key to
   double count during rollout. Correction: honor legacy keys until TTL expiry,
   but only when the KV detail and canonical DO lifecycle match.

## Sol review 2 — verified findings and corrections

Verdict: `REVISE`.

1. The review snapshot identified public caching of personalized lists. During
   review this was corrected by assigning cache lifetime only when no viewer was
   resolved; route tests prove an authenticated default list has `max-age=0` and
   viewer-specific fields are never shared.
2. The review snapshot identified alarm scheduling outside the install claim
   transaction. During review this was corrected: `getAlarm`/`setAlarm`, marker,
   slot, sequence, and counter are one storage transaction. A persisted-alarm
   regression distinguishes this from a post-commit best-effort alarm.
3. The remaining verified finding was that a live legacy KV limiter could cross
   deletion/recreation and suppress the new lifecycle. Correction: compare
   `detail.created_at` with the current DO pack before honoring the key; tests
   cover tombstoned and recreated slugs.

Both review rounds contained verified correctness findings. The prescribed
review count was exhausted, so no third Sol review was requested at that point.

## Current code excerpt 1 — bounded body reader and cache policy

```ts
export async function readJsonBody(request: Request): Promise<JsonBodyResult> {
  const reader = request.body?.getReader();
  if (!reader) return { ok: false, reason: "invalid-json" };
  const chunks: Uint8Array[] = [];
  let total = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > MAX_BODY_BYTES) {
      await reader.cancel("request body too large");
      return { ok: false, reason: "too-large" };
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
  try { return { ok: true, body: JSON.parse(new TextDecoder().decode(bytes)) }; }
  catch { return { ok: false, reason: "invalid-json" }; }
}

return json(
  request,
  { packs: visible, page, sort },
  200,
  isDefaultView && viewerId === undefined ? 30 : 0,
);
```

The merged W2 authority route uses this reader and then independently validates
exact `authority` plus bounded, syntactically valid `unowned_d1_ids`.

## Current code excerpt 2 — transactional bounded install claim

```ts
async recordInstall(id, identity, expectedLifecycle, now = Date.now()) {
  return this.ctx.blockConcurrencyWhile(async () => {
    await this.loadPacks();
    const result = await this.ctx.storage.transaction(async (txn) => {
      const pack = await txn.get(this.packKey(id));
      if (!this.lifecycleMatches(pack, expectedLifecycle)) return null;
      const markerKey = `install-marker:${id}:${pack.created_at}:${identity}`;
      const marker = await txn.get(markerKey);
      if (marker && now - marker.recordedAt < INSTALL_WINDOW_MS) return pack;
      if (marker) await txn.delete(marker.slotKey);
      const sequence = (await txn.get(INSTALL_SEQUENCE_KEY)) ?? 0;
      const slotKey = `install-slot:${sequence % INSTALL_RING_SIZE}`;
      const evicted = await txn.get(slotKey);
      if (evicted) await txn.delete(evicted.markerKey);
      const updated = { ...pack, install_count: (pack.install_count ?? 0) + 1 };
      await txn.put(this.packKey(id), updated);
      await txn.put(markerKey, { markerKey, recordedAt: now, slotKey });
      await txn.put(slotKey, { markerKey, recordedAt: now });
      await txn.put(INSTALL_SEQUENCE_KEY, sequence + 1);
      const cleanupAt = Math.max(now, Date.now()) + INSTALL_WINDOW_MS;
      const alarm = await txn.getAlarm();
      if (alarm === null || alarm > cleanupAt) await txn.setAlarm(cleanupAt);
      return updated;
    });
    if (!result) return null;
    await this.mirror(await this.getStoredPacks(), result);
    return result;
  });
}
```

The fixed ring intentionally permits a repeated identity within an hour only
after 5,000 newer claims evict it. This is an explicit bounded-idempotence policy,
not a claim of unlimited exact deduplication.

## Final diff and test evidence supplied for reconsultation

- Final W3-versus-W2 diff: 11 files, approximately 598 insertions and 140
  deletions. Runtime files are `bounded-map.ts`, `index.ts`, `pack-index-do.ts`,
  `shares.ts`, and `validate.ts`; the rest are focused tests and tracker updates.
- Before the W2 refresh, focused W3/W1 route + DO tests passed 107/107 and the
  complete Worker suite passed 218/218.
- The suite covers UTF-8 boundary/plus-one/cancellation, every body route,
  bounded oldest eviction, stale auth fill after reset, personalized cache
  headers, concurrent duplicate install claims, 5,001st-slot eviction,
  multi-page alarm expiry, transactional alarm persistence, mirror retry,
  delete/recreate lifecycle isolation, and both legacy limiter lifecycle cases.
- Worker `npm run check`, full `npm test`, Wrangler dry-run, `git diff --check`,
  and explicit `name = "kalpa-pack-hub"` previously passed. All gates will be
  rerun from the merged W2 base after Fable decides.
- No schema, public Pack/Rust response shape, real deployment, merge, authority
  flip, or package-version change occurred.

## Candidate decisions

### Candidate A — accept corrected D-W3-1

Keep the current design. Treat removal of manual list caching as the only
cross-isolate-correct invalidation strategy available without a shared cache
generation. Accept the 5,000-newer-claims exception as explicit bounded install
idempotence, with exact atomicity for every non-evicted identity and lifecycle.

### Candidate B — remove durable install idempotence from W3

Use only a DO-serialized counter increment plus the legacy KV limiter until a
future per-user install identity exists. This is simpler and stores no new HMAC
markers, but KV check/write is not atomic and cannot provide the requested
bounded/idempotent concurrency behavior.

### Candidate C — exact unbounded or partitioned install claims

Persist every identity for a full hour, or create per-pack/per-identity Durable
Objects. This can preserve exact one-hour deduplication beyond 5,000 claims, but
unbounded single-DO storage/cleanup or a new DO class and migration materially
increase deployment and operational scope for a low-severity counter.

## Failure modes to evaluate

1. Two isolates serve/list while a mutation invalidates between read and put.
2. An authenticated default-list request contains `user_voted` or owner-only
   anonymity data and an intermediary follows `Cache-Control`.
3. A JSON stream splits a multibyte UTF-8 code point at the limit, never ends,
   or exceeds the limit in its first/last chunk.
4. Process/DO reset occurs after install transaction commit but before KV mirror.
5. Alarm setup, marker insertion, slot eviction, or counter write partially fails.
6. More than 5,000 distinct installs arrive inside one hour, including several
   slots sharing one pack and lifecycle.
7. Cleanup paginates across more than 1,000 slots; alarm fires late or twice.
8. A pack is deleted and the same slug recreated while a legacy limiter or HMAC
   marker remains.
9. `ADMIN_API_KEY` rotates, making prior HMAC identities unreachable until their
   alarm cleanup.
10. The W2 authority route receives oversized JSON containing otherwise valid
    `unowned_d1_ids`; no D-W2-2 adjudication may be partially applied.
11. The service is rolled back to W2 while W3 install marker/slot records remain.
12. Corpus/backup details removed from `/health` were used by an undocumented
    external monitor; the deploy workflow itself uses only `status` and `kv`.

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

## Recorded Fable response

```text
DECISION:
1. Candidate A — accept corrected D-W3-1. Keep byte-counted stream reading, but catch a rejected/aborted read as invalid JSON and decode only after the complete bounded byte buffer is assembled.
2. Keep no manual Cache API list storage. Anonymous default lists alone receive `public, max-age=30`; every viewer-bearing response receives explicit `max-age=0` and `Vary: Authorization, Origin`.
3. Keep the one-transaction install claim, lifecycle marker, ring slot, counter, sequence, and alarm. Document the 5,000-slot guarantee as bounded idempotence and let HMAC identities made unreachable by an admin-key rotation age out normally.
4. Keep post-commit mirroring retry-healable, honor legacy limiter keys only for a matching `created_at`, and keep public `/health` limited to `status`, `kv`, and `timestamp`.
5. Rerun all Worker gates on the merged W2 base before a fresh Sol pass. Do not fold H3 package-version policy into W3.

REJECTED:
1. Counter plus legacy KV limiter only: KV check-then-write is not atomic across colos and leaves lifecycle suppression indefinitely.
2. Unbounded markers or new per-identity Durable Objects: cleanup/storage or migration risk is disproportionate for a low-severity display counter.
3. Any manual Cache API list entry: per-colo caches have no shared invalidation primitive and can reinsert stale or personalized responses.
4. Public corpus/backup fields: they expose operator state and encourage undocumented coupling.

CRASH_RECOVERY:
1. A kill before transaction commit persists nothing. A kill after commit but before KV mirror leaves the DO canonical and a duplicate/new install retry heals the detail mirror.
2. Alarm cleanup is idempotent if late or repeated. A recreated slug cannot match an old lifecycle marker, and legacy KV keys with a mismatched `created_at` are ignored until TTL expiry.
3. Rollback to W2 leaves W3 marker/slot keys inert. A later W3 deployment can clean them. Admin-key rotation permits at worst one extra count per identity during the remaining hour.
4. A body stream that never ends is bounded by the platform timeout and never applies a partial W2 authority request.

TESTS:
1. Exact and plus-one byte limits, first/last oversized chunks, split UTF-8, cancellation, and rejected reads; oversized W2 authority input performs no mutation.
2. Anonymous default, authenticated default, and filtered list cache headers; no Cache API puts.
3. Concurrent duplicate and distinct installs, exact 5,001st eviction, transactional alarm persistence/failure, mirror healing, lifecycle reuse, and multi-page alarm cleanup.
4. Minimal health shape and W2 compatibility with inert W3 storage keys.

RISKS:
1. More than 5,000 distinct hourly claims can let an early identity recount; confirm and document this policy bound.
2. Removing manual caching increases origin reads modestly; observe after deployment.
3. HMAC continuity is coupled to `ADMIN_API_KEY`; a dedicated secret is future hygiene, not W3.
4. Confirm no external monitor depends on removed health fields. In-repo deploy logic uses only `status` and `kv`.
5. H3 remains open and must not be resolved implicitly.
```

## Executor verification after decision

The two required corrections were absent and received failing-first tests. A
rejected body stream escaped from `reader.read()` instead of resolving to
`invalid-json`; authenticated viewer responses omitted explicit cache metadata
and varied only on Origin. The implementation now catches stream failures,
assembles no more than the accepted byte ceiling before one decode/parse, emits
explicit zero-age caching for viewer-bearing list/detail responses, and varies
on both Origin and Authorization.

Focused body/list/CORS coverage passes 146/146. On the merged W2 base the full
Worker suite passes 230/230; `npm run check`, Wrangler dry-run, the exact Worker
name guard, and `git diff --check` pass. No real deployment ran.
