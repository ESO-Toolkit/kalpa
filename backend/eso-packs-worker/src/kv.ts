import type { Env, Pack, PackIndex, VoteRecord } from "./types";

const PACK_PREFIX = "pack:";
const INDEX_KEY = "index:packs";
const VOTE_PREFIX = "vote:";

export interface VersionedIndex {
  index: PackIndex;
  version: number;
}

export async function getPackIndex(env: Env): Promise<PackIndex | null> {
  return env.ESO_PACKS.get<PackIndex>(INDEX_KEY, "json");
}

export async function getVersionedIndex(env: Env): Promise<VersionedIndex> {
  const { value, metadata } = await env.ESO_PACKS.getWithMetadata<PackIndex>(INDEX_KEY, "json");
  const version = (metadata as { version?: number } | null)?.version ?? 0;
  return { index: value ?? { packs: [] }, version };
}

export async function putPackIndexVersioned(
  env: Env,
  index: PackIndex,
  expectedVersion: number,
): Promise<boolean> {
  const { metadata } = await env.ESO_PACKS.getWithMetadata(INDEX_KEY);
  const currentVersion = (metadata as { version?: number } | null)?.version ?? 0;
  if (currentVersion !== expectedVersion) {
    return false;
  }
  await env.ESO_PACKS.put(INDEX_KEY, JSON.stringify(index), {
    metadata: { version: expectedVersion + 1 },
  });
  return true;
}

const MAX_RETRIES = 3;

/**
 * Atomically update the pack index with retry on version conflict.
 * The mutator receives a mutable copy of the current index.
 */
export async function updatePackIndex(
  env: Env,
  mutator: (index: PackIndex) => void,
): Promise<void> {
  for (let attempt = 0; attempt < MAX_RETRIES; attempt++) {
    const { index, version } = await getVersionedIndex(env);
    mutator(index);
    const success = await putPackIndexVersioned(env, index, version);
    if (success) return;
  }
  // Final attempt without version check as fallback
  const { index } = await getVersionedIndex(env);
  mutator(index);
  await env.ESO_PACKS.put(INDEX_KEY, JSON.stringify(index), {
    metadata: { version: Date.now() },
  });
}

export async function getPack(env: Env, id: string): Promise<Pack | null> {
  return env.ESO_PACKS.get<Pack>(`${PACK_PREFIX}${id}`, "json");
}

export async function putPack(env: Env, pack: Pack): Promise<void> {
  await env.ESO_PACKS.put(`${PACK_PREFIX}${pack.id}`, JSON.stringify(pack));
}

export async function putPackIndex(
  env: Env,
  index: PackIndex,
): Promise<void> {
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
}

export async function deleteVote(
  env: Env,
  packId: string,
  userId: string,
): Promise<void> {
  await env.ESO_PACKS.delete(voteKey(packId, userId));
}
