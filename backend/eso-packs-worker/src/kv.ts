import type { Env, Pack, PackIndex, VoteRecord } from "./types";

const PACK_PREFIX = "pack:";
const INDEX_KEY = "index:packs";
const VOTE_PREFIX = "vote:";

export async function getPackIndex(
  env: Env,
  opts?: { fresh?: boolean },
): Promise<PackIndex | null> {
  // cacheTtl lets the KV edge cache serve the index for 60s. The Cache API is a
  // no-op on workers.dev, so this is the only read cache that actually applies.
  // Callers that must not observe a stale pre-mutation index (e.g. the
  // scheduled backup) pass { fresh: true } — KV's floor is 30s, so that's the
  // freshest read available, matching the { fresh } idiom used by getPack.
  const cacheTtl = opts?.fresh ? 30 : 60;
  return env.ESO_PACKS.get<PackIndex>(INDEX_KEY, { type: "json", cacheTtl });
}

export async function getPack(
  env: Env,
  id: string,
  opts?: { fresh?: boolean },
): Promise<Pack | null> {
  // The public detail view tolerates staleness and is cached at the edge for
  // 300s. Callers that read-modify-write the whole pack object back (update,
  // delete, the create-time uniqueness check) pass { fresh: true } to minimize
  // the stale window — KV's floor is 30s, so a truly fresh read is impossible;
  // counter mutations (vote/install) must therefore go through the Durable
  // Object (bumpPackCounter), which reads the index with no cacheTtl.
  const cacheTtl = opts?.fresh ? 30 : 300;
  return env.ESO_PACKS.get<Pack>(`${PACK_PREFIX}${id}`, { type: "json", cacheTtl });
}

export async function putPack(env: Env, pack: Pack): Promise<void> {
  await env.ESO_PACKS.put(`${PACK_PREFIX}${pack.id}`, JSON.stringify(pack));
}

export async function putPackIndex(env: Env, index: PackIndex): Promise<void> {
  await env.ESO_PACKS.put(INDEX_KEY, JSON.stringify(index));
}

// ── Vote helpers ──────────────────────────────────────────────────

function voteKey(packId: string, userId: string): string {
  return `${VOTE_PREFIX}${packId}:${userId}`;
}

export async function getVote(
  env: Env,
  packId: string,
  userId: string,
): Promise<VoteRecord | null> {
  return env.ESO_PACKS.get<VoteRecord>(voteKey(packId, userId), "json");
}

function userVoteKey(userId: string, packId: string): string {
  return `user-votes:${userId}:${packId}`;
}

export async function putVote(
  env: Env,
  packId: string,
  userId: string,
): Promise<void> {
  const record: VoteRecord = {
    userId,
    packId,
    votedAt: new Date().toISOString(),
  };
  await env.ESO_PACKS.put(voteKey(packId, userId), JSON.stringify(record));
  await env.ESO_PACKS.put(userVoteKey(userId, packId), "1");
}

export async function deleteVote(
  env: Env,
  packId: string,
  userId: string,
): Promise<void> {
  await env.ESO_PACKS.delete(voteKey(packId, userId));
  await env.ESO_PACKS.delete(userVoteKey(userId, packId));
}

/**
 * Rewrite a vote record (both the `vote:<packId>:<userId>` record and its
 * `user-votes:<userId>:<packId>` reverse index) verbatim, preserving the
 * original `votedAt`. Used by admin restore to replay a backup snapshot's
 * votes rather than re-stamping them via putVote.
 */
export async function restoreVote(
  env: Env,
  packId: string,
  userId: string,
  record: VoteRecord,
): Promise<void> {
  await env.ESO_PACKS.put(voteKey(packId, userId), JSON.stringify(record));
  await env.ESO_PACKS.put(userVoteKey(userId, packId), "1");
}

// ── Full-corpus enumeration (scheduled backup) ─────────────────────
// The index mirrors full pack fields for list queries, but the scheduled
// backup also captures the per-pack `pack:<id>` bodies and vote records
// directly so the backup is restorable even if the index and per-key data
// ever drift. KV `list()` caps at 1000 keys per call, so both loop on the
// cursor until `list_complete`.

export async function listAllPackBodies(env: Env): Promise<Record<string, Pack>> {
  const bodies: Record<string, Pack> = {};
  let cursor: string | undefined;
  do {
    const page = await env.ESO_PACKS.list({ prefix: PACK_PREFIX, cursor });
    for (const key of page.keys) {
      const id = key.name.slice(PACK_PREFIX.length);
      const pack = await env.ESO_PACKS.get<Pack>(key.name, "json");
      if (pack) bodies[id] = pack;
    }
    cursor = page.list_complete ? undefined : page.cursor;
  } while (cursor);
  return bodies;
}

export async function listAllVotes(env: Env): Promise<Record<string, VoteRecord>> {
  const votes: Record<string, VoteRecord> = {};
  let cursor: string | undefined;
  do {
    const page = await env.ESO_PACKS.list({ prefix: VOTE_PREFIX, cursor });
    for (const key of page.keys) {
      const record = await env.ESO_PACKS.get<VoteRecord>(key.name, "json");
      if (record) votes[key.name.slice(VOTE_PREFIX.length)] = record;
    }
    cursor = page.list_complete ? undefined : page.cursor;
  } while (cursor);
  return votes;
}
