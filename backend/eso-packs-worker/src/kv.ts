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
  // Always read at KV's freshest available TTL. A stale (or stale-negative)
  // read here is what makes a vote toggle apply the same delta twice and what
  // makes the client render an already-voted pack as unvoted.
  return env.ESO_PACKS.get<VoteRecord>(voteKey(packId, userId), {
    type: "json",
    cacheTtl: 30,
  });
}

/**
 * Which of `packIds` the viewer has already voted on, for populating
 * `user_voted` on list/detail responses. Bounded by the caller's page size.
 */
export async function getVotedPackIds(
  env: Env,
  userId: string,
  packIds: string[],
): Promise<Set<string>> {
  const records = await Promise.all(packIds.map((id) => getVote(env, id, userId)));
  const voted = new Set<string>();
  packIds.forEach((id, i) => {
    if (records[i]) voted.add(id);
  });
  return voted;
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
  // The record is mirrored into the key's metadata so the daily backup can
  // enumerate every vote from list() alone — see listAllVotes.
  await env.ESO_PACKS.put(voteKey(packId, userId), JSON.stringify(record), {
    metadata: record,
  });
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
 * Delete every vote record for a pack (and each record's `user-votes` reverse
 * key). Deleted slugs become available again, so leaving the records behind
 * meant a recycled id inherited them and the first vote a previous voter cast
 * on the new pack was silently treated as an unvote. Returns the count removed.
 */
export async function deleteVotesForPack(env: Env, packId: string): Promise<number> {
  const prefix = `${VOTE_PREFIX}${packId}:`;
  let cursor: string | undefined;
  let removed = 0;
  do {
    const page = await env.ESO_PACKS.list({ prefix, cursor });
    for (const key of page.keys) {
      const userId = key.name.slice(prefix.length);
      await env.ESO_PACKS.delete(key.name);
      if (userId) await env.ESO_PACKS.delete(userVoteKey(userId, packId));
      removed++;
    }
    cursor = page.list_complete ? undefined : page.cursor;
  } while (cursor);
  return removed;
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
  await env.ESO_PACKS.put(voteKey(packId, userId), JSON.stringify(record), {
    metadata: record,
  });
  await env.ESO_PACKS.put(userVoteKey(userId, packId), "1");
}

// ── Full-corpus enumeration (scheduled backup) ─────────────────────
// Workers cap subrequests per invocation and KV operations count against it,
// so the backup must not fan out one get() per record — that ceiling would
// eventually make every nightly run throw and stop producing backups
// entirely. Vote records are therefore read from each key's list() metadata
// (written by putVote/restoreVote), and pack bodies come from the index,
// which already carries full Pack objects. KV `list()` caps at 1000 keys per
// call, so the loop follows the cursor until `list_complete`.

export async function listAllVotes(env: Env): Promise<Record<string, VoteRecord>> {
  const votes: Record<string, VoteRecord> = {};
  let cursor: string | undefined;
  do {
    const page = await env.ESO_PACKS.list<VoteRecord>({ prefix: VOTE_PREFIX, cursor });
    for (const key of page.keys) {
      // Records written before vote metadata existed still need a read; that
      // set only shrinks as those votes are toggled or expire.
      const record =
        key.metadata ?? (await env.ESO_PACKS.get<VoteRecord>(key.name, "json"));
      if (record) votes[key.name.slice(VOTE_PREFIX.length)] = record;
    }
    cursor = page.list_complete ? undefined : page.cursor;
  } while (cursor);
  return votes;
}
