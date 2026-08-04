import { DurableObject } from "cloudflare:workers";
import type { Env, Pack, PackIndex } from "./types";
import { deleteVote, getVote, putVote } from "./kv";

const INDEX_KEY = "index:packs";

/** Bound the vote memo so a long-lived instance can't grow it without limit. */
const VOTE_MEMO_LIMIT = 5000;

/**
 * Durable Object that serializes all pack index mutations.
 *
 * KV doesn't support compare-and-swap, so concurrent read-modify-write
 * operations on the pack index can lose updates. This DO provides
 * single-threaded execution guarantees — all mutations are serialized
 * through one instance, making read-modify-write safe.
 *
 * The DO reads/writes the canonical index from/to KV so the existing
 * GET /packs read path (which only reads KV) is unaffected. Because that
 * makes every step a subrequest rather than DO storage, input gates do NOT
 * hold across the awaits — a second RPC would start executing while the
 * first is between its read and its write, which is exactly the lost update
 * this class exists to prevent. Every mutating method therefore runs inside
 * blockConcurrencyWhile, which queues concurrent RPCs for real.
 */
export class PackIndexDO extends DurableObject<Env> {
  /**
   * Vote state this instance has itself written, authoritative over KV for the
   * edge-cache window. KV's freshest read is still up to 30s stale, so a
   * vote followed by an unvote inside that window would read back the
   * pre-write state and apply the same delta twice — one vote record, two
   * increments, and nothing ever reconciles counters against records.
   */
  private readonly voteMemo = new Map<string, boolean>();

  /**
   * Append a pack to the index, optionally enforcing a per-author cap.
   *
   * The cap has to be checked here rather than in the handler: the handler's
   * index read is edge-cached, so packs created in the last minute are
   * invisible to it and concurrent creates all pass at the limit minus one.
   * Returns `ok: false` when the author is already at `maxPerAuthor`.
   */
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
      if (pos >= 0) {
        index.packs[pos] = pack;
      } else {
        index.packs.push(pack);
      }
      await this.putIndex(index);
    });
  }

  /**
   * Increment a denormalized counter (vote_count / install_count) atomically.
   *
   * Counter mutations used to read the pack via KV getPack (300s edge cache),
   * bump one field, then write the whole object back to both the pack KV and
   * the index. A stale cached read both lost concurrent counter updates AND
   * silently reverted any pack content edited within the cache window. Doing the
   * read-modify-write here — against the DO's no-cacheTtl index read, under its
   * single-threaded guarantee — fixes both: only the counter changes, on a fresh
   * copy. `seed` is the caller's already-read pack, used to self-heal the index
   * if the pack is somehow missing from it (mirrors the old updatePack upsert).
   * Returns the updated pack, or null if it is in neither the index nor seed.
   */
  async bumpPackCounter(
    id: string,
    field: "vote_count" | "install_count",
    delta: number,
    seed?: Pack | null,
  ): Promise<Pack | null> {
    return this.ctx.blockConcurrencyWhile(() => this.applyCounter(id, field, delta, seed));
  }

  /**
   * Toggle the caller's vote and apply the matching counter delta in one
   * serialized step.
   *
   * Reading the vote record in the handler and bumping the counter here left
   * the two decoupled: the handler's read goes through KV's edge cache, so a
   * rapid vote/unvote could see the same stale state twice and inflate
   * vote_count permanently. Deciding vote-vs-unvote in the same place that
   * applies the delta lets this dedup — the counter only moves when the
   * record's existence actually changes.
   */
  async toggleVote(
    packId: string,
    userId: string,
    seed?: Pack | null,
  ): Promise<{ voted: boolean; pack: Pack | null }> {
    return this.ctx.blockConcurrencyWhile(async () => {
      const memoKey = `${packId}:${userId}`;
      const memo = this.voteMemo.get(memoKey);
      const hadVote = memo ?? (await getVote(this.env, packId, userId)) !== null;
      const voted = !hadVote;

      if (voted) {
        await putVote(this.env, packId, userId);
      } else {
        await deleteVote(this.env, packId, userId);
      }
      if (this.voteMemo.size >= VOTE_MEMO_LIMIT) this.voteMemo.clear();
      this.voteMemo.set(memoKey, voted);

      const pack = await this.applyCounter(packId, "vote_count", voted ? 1 : -1, seed);
      return { voted, pack };
    });
  }

  async removePack(id: string): Promise<void> {
    await this.ctx.blockConcurrencyWhile(async () => {
      const index = await this.getIndex();
      index.packs = index.packs.filter((p) => p.id !== id);
      await this.putIndex(index);
      this.forgetVotes(id);
    });
  }

  /**
   * Remove all packs by a given author in a single read-write cycle, returning
   * the ids removed. Account deletion deletes the bodies and D1 rows for those
   * ids, so it must learn them from here rather than from its own cached index
   * read, which can miss a pack created moments earlier.
   */
  async removePacksByAuthor(authorId: string): Promise<string[]> {
    return this.ctx.blockConcurrencyWhile(async () => {
      const index = await this.getIndex();
      const removed = index.packs.filter((p) => p.author_id === authorId).map((p) => p.id);
      if (removed.length > 0) {
        index.packs = index.packs.filter((p) => p.author_id !== authorId);
        await this.putIndex(index);
        for (const id of removed) this.forgetVotes(id);
      }
      return removed;
    });
  }

  async replaceIndex(index: PackIndex): Promise<void> {
    await this.ctx.blockConcurrencyWhile(async () => {
      await this.putIndex(index);
      // Seed and restore rewrite the corpus wholesale, so nothing the memo
      // holds can be trusted afterwards.
      this.voteMemo.clear();
    });
  }

  /**
   * Drop memoized vote state for a deleted pack. Its vote records are deleted
   * with it, and slugs are reusable, so a recycled id must start clean —
   * otherwise the memo would tell a previous voter they had already voted.
   */
  private forgetVotes(packId: string): void {
    const prefix = `${packId}:`;
    for (const key of this.voteMemo.keys()) {
      if (key.startsWith(prefix)) this.voteMemo.delete(key);
    }
  }

  /**
   * Counter read-modify-write without its own concurrency gate — every caller
   * already runs inside blockConcurrencyWhile, and nesting one inside another
   * would deadlock.
   */
  private async applyCounter(
    id: string,
    field: "vote_count" | "install_count",
    delta: number,
    seed?: Pack | null,
  ): Promise<Pack | null> {
    const index = await this.getIndex();
    const pos = index.packs.findIndex((p) => p.id === id);
    if (pos < 0) {
      if (!seed) return null;
      // Index drift: re-add from the caller's copy so the count still applies.
      const healed = { ...seed, [field]: Math.max(0, (seed[field] ?? 0) + delta) };
      index.packs.push(healed);
      await this.putIndex(index);
      await this.env.ESO_PACKS.put(`pack:${id}`, JSON.stringify(healed));
      return healed;
    }
    const pack = index.packs[pos];
    pack[field] = Math.max(0, (pack[field] ?? 0) + delta);
    await this.putIndex(index);
    // Keep the per-pack KV detail in sync from the same fresh copy.
    await this.env.ESO_PACKS.put(`pack:${id}`, JSON.stringify(pack));
    return pack;
  }

  private async getIndex(): Promise<PackIndex> {
    return (await this.env.ESO_PACKS.get<PackIndex>(INDEX_KEY, "json")) ?? { packs: [] };
  }

  private async putIndex(index: PackIndex): Promise<void> {
    await this.env.ESO_PACKS.put(INDEX_KEY, JSON.stringify(index));
  }
}
