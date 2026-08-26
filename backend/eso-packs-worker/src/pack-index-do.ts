import { DurableObject } from "cloudflare:workers";
import type { Env, Pack, PackIndex } from "./types";
import { deleteVote, getVote, putVote } from "./kv";

const INDEX_KEY = "index:packs";
const STORAGE_PACK_PREFIX = "pack:";
const BOOTSTRAPPED_KEY = "meta:bootstrapped";

/** Bound the vote memo so a long-lived instance cannot grow without limit. */
const VOTE_MEMO_LIMIT = 5000;

/**
 * Serializes pack mutations and owns their canonical state.
 *
 * Durable Object storage is authoritative. KV remains the public read model
 * and is rewritten from the complete canonical set after every mutation. A
 * KV-backed authority is insufficient because serialization does not provide
 * read-after-write visibility for eventually consistent KV reads.
 */
export class PackIndexDO extends DurableObject<Env> {
  /** State written by this isolate, authoritative over stale KV vote reads. */
  private readonly voteMemo = new Map<string, boolean>();

  async addPack(
    pack: Pack,
    maxPerAuthor?: number,
  ): Promise<{ ok: true } | { ok: false; reason: "duplicate" | "limit" }> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.ensureBootstrapped();
      if (await this.ctx.storage.get<Pack>(this.packKey(pack.id))) {
        return { ok: false, reason: "duplicate" };
      }

      const packs = await this.getPacks();
      if (maxPerAuthor !== undefined) {
        const owned = packs.filter((candidate) => candidate.author_id === pack.author_id).length;
        if (owned >= maxPerAuthor) return { ok: false, reason: "limit" };
      }

      await this.ctx.storage.put(this.packKey(pack.id), pack);
      packs.push(pack);
      await this.mirror(packs, pack);
      return { ok: true };
    });
  }

  async updatePack(id: string, pack: Pack): Promise<Pack | null> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.ensureBootstrapped();
      const existing = await this.ctx.storage.get<Pack>(this.packKey(id));
      if (!existing) return null;

      // Identity and counters belong to the canonical record. The handler's
      // validated content may have been built from a stale KV detail body.
      const updated: Pack = {
        ...pack,
        id: existing.id,
        author_id: existing.author_id,
        author_name: existing.author_name,
        vote_count: existing.vote_count,
        install_count: existing.install_count,
        created_at: existing.created_at,
      };
      await this.ctx.storage.put(this.packKey(id), updated);
      await this.mirror(await this.getPacks(), updated);
      return updated;
    });
  }

  async bumpPackCounter(
    id: string,
    field: "vote_count" | "install_count",
    delta: number,
    _staleSeed?: Pack | null,
  ): Promise<Pack | null> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.ensureBootstrapped();
      return this.applyCounter(id, field, delta);
    });
  }

  async toggleVote(
    packId: string,
    userId: string,
    _staleSeed?: Pack | null,
  ): Promise<{ voted: boolean; pack: Pack | null }> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.ensureBootstrapped();
      // Check existence before changing vote records. A request may have read
      // the old KV detail body before a concurrent delete completed.
      if (!(await this.ctx.storage.get<Pack>(this.packKey(packId)))) {
        return { voted: false, pack: null };
      }

      const memoKey = `${packId}:${userId}`;
      const memo = this.voteMemo.get(memoKey);
      const hadVote = memo ?? (await getVote(this.env, packId, userId)) !== null;
      const voted = !hadVote;

      if (voted) await putVote(this.env, packId, userId);
      else await deleteVote(this.env, packId, userId);

      if (this.voteMemo.size >= VOTE_MEMO_LIMIT) this.voteMemo.clear();
      this.voteMemo.set(memoKey, voted);

      const pack = await this.applyCounter(packId, "vote_count", voted ? 1 : -1);
      return { voted, pack };
    });
  }

  async removePack(id: string): Promise<void> {
    await this.ctx.blockConcurrencyWhile(async () => {
      await this.ensureBootstrapped();
      await this.ctx.storage.delete(this.packKey(id));
      await this.mirror(await this.getPacks(), undefined, id);
      this.forgetVotes(id);
    });
  }

  async removePacksByAuthor(authorId: string): Promise<string[]> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.ensureBootstrapped();
      const packs = await this.getPacks();
      const removed = packs.filter((pack) => pack.author_id === authorId).map((pack) => pack.id);
      if (removed.length === 0) return removed;

      for (const id of removed) {
        await this.ctx.storage.delete(this.packKey(id));
        this.forgetVotes(id);
      }
      const kept = packs.filter((pack) => pack.author_id !== authorId);
      await this.env.ESO_PACKS.put(INDEX_KEY, JSON.stringify({ packs: kept }));
      for (const id of removed) await this.env.ESO_PACKS.delete(`pack:${id}`);
      return removed;
    });
  }

  async replaceIndex(index: PackIndex): Promise<void> {
    await this.ctx.blockConcurrencyWhile(async () => {
      await this.ctx.storage.deleteAll();
      for (const pack of index.packs) {
        await this.ctx.storage.put(this.packKey(pack.id), pack);
      }
      await this.ctx.storage.put(BOOTSTRAPPED_KEY, true);
      await this.env.ESO_PACKS.put(INDEX_KEY, JSON.stringify(index));
      this.voteMemo.clear();
    });
  }

  private forgetVotes(packId: string): void {
    const prefix = `${packId}:`;
    for (const key of this.voteMemo.keys()) {
      if (key.startsWith(prefix)) this.voteMemo.delete(key);
    }
  }

  /** Counter mutation without its own gate; callers already hold the gate. */
  private async applyCounter(
    id: string,
    field: "vote_count" | "install_count",
    delta: number,
  ): Promise<Pack | null> {
    const pack = await this.ctx.storage.get<Pack>(this.packKey(id));
    if (!pack) return null;

    pack[field] = Math.max(0, (pack[field] ?? 0) + delta);
    await this.ctx.storage.put(this.packKey(id), pack);
    await this.mirror(await this.getPacks(), pack);
    return pack;
  }

  private packKey(id: string): string {
    return `${STORAGE_PACK_PREFIX}${id}`;
  }

  private async ensureBootstrapped(): Promise<void> {
    if (await this.ctx.storage.get<boolean>(BOOTSTRAPPED_KEY)) return;

    const index = (await this.env.ESO_PACKS.get<PackIndex>(INDEX_KEY, "json")) ?? { packs: [] };
    for (const pack of index.packs) {
      await this.ctx.storage.put(this.packKey(pack.id), pack);
    }
    // Set last so an interrupted bootstrap is retried idempotently.
    await this.ctx.storage.put(BOOTSTRAPPED_KEY, true);
  }

  private async getPacks(): Promise<Pack[]> {
    const entries = await this.ctx.storage.list<Pack>({ prefix: STORAGE_PACK_PREFIX });
    return [...entries.values()];
  }

  private async mirror(packs: Pack[], changed?: Pack, deletedId?: string): Promise<void> {
    await this.env.ESO_PACKS.put(INDEX_KEY, JSON.stringify({ packs }));
    if (changed) await this.env.ESO_PACKS.put(`pack:${changed.id}`, JSON.stringify(changed));
    if (deletedId) await this.env.ESO_PACKS.delete(`pack:${deletedId}`);
  }
}
