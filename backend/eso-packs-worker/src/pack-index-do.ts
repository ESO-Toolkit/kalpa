import { DurableObject } from "cloudflare:workers";
import type { Env, Pack, PackIndex } from "./types";

const INDEX_KEY = "index:packs";

/**
 * Durable Object that serializes all pack index mutations.
 *
 * KV doesn't support compare-and-swap, so concurrent read-modify-write
 * operations on the pack index can lose updates. This DO provides
 * single-threaded execution guarantees — all mutations are serialized
 * through one instance, making read-modify-write safe.
 *
 * The DO reads/writes the canonical index from/to KV so the existing
 * GET /packs read path (which only reads KV) is unaffected.
 */
export class PackIndexDO extends DurableObject<Env> {
  async addPack(pack: Pack): Promise<void> {
    const index = await this.getIndex();
    index.packs.push(pack);
    await this.putIndex(index);
  }

  async updatePack(id: string, pack: Pack): Promise<void> {
    const index = await this.getIndex();
    const pos = index.packs.findIndex((p) => p.id === id);
    if (pos >= 0) {
      index.packs[pos] = pack;
    } else {
      index.packs.push(pack);
    }
    await this.putIndex(index);
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

  async removePack(id: string): Promise<void> {
    const index = await this.getIndex();
    index.packs = index.packs.filter((p) => p.id !== id);
    await this.putIndex(index);
  }

  /** Remove all packs by a given author in a single read-write cycle. */
  async removePacksByAuthor(authorId: string): Promise<number> {
    const index = await this.getIndex();
    const before = index.packs.length;
    index.packs = index.packs.filter((p) => p.author_id !== authorId);
    const removed = before - index.packs.length;
    if (removed > 0) {
      await this.putIndex(index);
    }
    return removed;
  }

  async replaceIndex(index: PackIndex): Promise<void> {
    await this.putIndex(index);
  }

  private async getIndex(): Promise<PackIndex> {
    return (await this.env.ESO_PACKS.get<PackIndex>(INDEX_KEY, "json")) ?? { packs: [] };
  }

  private async putIndex(index: PackIndex): Promise<void> {
    await this.env.ESO_PACKS.put(INDEX_KEY, JSON.stringify(index));
  }
}
