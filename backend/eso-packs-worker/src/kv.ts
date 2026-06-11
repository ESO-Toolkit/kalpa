import type { Env, Pack, PackIndex, VoteRecord } from "./types";

const PACK_PREFIX = "pack:";
const INDEX_KEY = "index:packs";
const VOTE_PREFIX = "vote:";

export async function getPackIndex(env: Env): Promise<PackIndex | null> {
  // cacheTtl lets the KV edge cache serve the index for 60s. The Cache API is a
  // no-op on workers.dev, so this is the only read cache that actually applies.
  return env.ESO_PACKS.get<PackIndex>(INDEX_KEY, { type: "json", cacheTtl: 60 });
}

export async function getPack(env: Env, id: string): Promise<Pack | null> {
  // Pack detail is far more stable than the index; cache for 300s.
  return env.ESO_PACKS.get<Pack>(`${PACK_PREFIX}${id}`, { type: "json", cacheTtl: 300 });
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
