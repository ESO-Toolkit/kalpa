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
