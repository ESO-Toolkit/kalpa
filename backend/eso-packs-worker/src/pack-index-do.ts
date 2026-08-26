import { DurableObject } from "cloudflare:workers";
import type { Env, Pack, PackIndex } from "./types";
import { deleteVote, getVote, putVote } from "./kv";

const INDEX_KEY = "index:packs";
const STORAGE_PACK_PREFIX = "pack:";
const TOMBSTONE_PREFIX = "tomb:";
const AUTHORITY_KEY = "meta:authority";
const VOTE_MEMO_LIMIT = 5000;

type Authority = "kv" | "do";
type MutationResult =
  | { status: "ok"; pack: Pack }
  | { status: "not-found" }
  | { status: "forbidden" };

export interface MigrationParity {
  authority: Authority;
  kv_count: number;
  do_count: number;
  tombstones: string[];
  do_only: string[];
  missing_from_do: string[];
}

export interface WitnessAdoption {
  adopted: string[];
  already_present: string[];
  tombstoned: string[];
  unavailable: string[];
}

/** Serializes mutations while migrating authority from KV to DO storage. */
export class PackIndexDO extends DurableObject<Env> {
  private readonly voteMemo = new Map<string, boolean>();

  async addPack(
    pack: Pack,
    maxPerAuthor?: number,
  ): Promise<{ ok: true } | { ok: false; reason: "duplicate" | "limit" }> {
    return this.ctx.blockConcurrencyWhile(async () => {
      const packs = await this.loadPacks();
      if (packs.some(({ id }) => id === pack.id)) {
        return { ok: false, reason: "duplicate" };
      }
      if (
        maxPerAuthor !== undefined &&
        packs.filter(({ author_id }) => author_id === pack.author_id).length >= maxPerAuthor
      ) {
        return { ok: false, reason: "limit" };
      }

      await this.ctx.storage.delete(this.tombstoneKey(pack.id));
      await this.ctx.storage.put(this.packKey(pack.id), pack);
      packs.push(pack);
      await this.mirror(packs, pack);
      return { ok: true };
    });
  }

  async updatePack(id: string, pack: Pack, actorId?: string): Promise<MutationResult> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.loadPacks();
      const existing = await this.ctx.storage.get<Pack>(this.packKey(id));
      if (!existing) return { status: "not-found" };
      if ((actorId ?? pack.author_id) !== existing.author_id) return { status: "forbidden" };

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
      await this.mirror(await this.getStoredPacks(), updated);
      return { status: "ok", pack: updated };
    });
  }

  async bumpPackCounter(
    id: string,
    field: "vote_count" | "install_count",
    delta: number,
    expectedLifecycle?: string | Pack | null,
  ): Promise<Pack | null> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.loadPacks();
      const pack = await this.ctx.storage.get<Pack>(this.packKey(id));
      if (!this.lifecycleMatches(pack, expectedLifecycle)) return null;
      return this.applyCounter(pack!, field, delta);
    });
  }

  async toggleVote(
    packId: string,
    userId: string,
    expectedLifecycle?: string | Pack | null,
  ): Promise<{ voted: boolean; pack: Pack | null }> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.loadPacks();
      const existing = await this.ctx.storage.get<Pack>(this.packKey(packId));
      if (!this.lifecycleMatches(existing, expectedLifecycle)) {
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
      const pack = await this.applyCounter(existing!, "vote_count", voted ? 1 : -1);
      return { voted, pack };
    });
  }

  async removePack(id: string, actorId?: string): Promise<"ok" | "not-found" | "forbidden"> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.loadPacks();
      const existing = await this.ctx.storage.get<Pack>(this.packKey(id));
      if (!existing) return "not-found";
      if (actorId !== undefined && actorId !== existing.author_id) return "forbidden";

      await this.deleteStoredPack(id);
      await this.mirror(await this.getStoredPacks(), undefined, id);
      return "ok";
    });
  }

  async removePacksByAuthor(authorId: string): Promise<string[]> {
    return this.ctx.blockConcurrencyWhile(async () => {
      const packs = await this.loadPacks();
      const removed = packs.filter((pack) => pack.author_id === authorId).map((pack) => pack.id);
      for (const id of removed) await this.deleteStoredPack(id);
      if (removed.length > 0) {
        await this.mirror(await this.getStoredPacks(), undefined, undefined, removed);
      }
      return removed;
    });
  }

  async replaceIndex(index: PackIndex): Promise<void> {
    await this.ctx.blockConcurrencyWhile(async () => {
      await this.loadPacks();
      await this.applyReplacement(index.packs);
    });
  }

  async replaceIndexPreserving(index: PackIndex, restoredIds: string[]): Promise<void> {
    await this.ctx.blockConcurrencyWhile(async () => {
      const current = await this.loadPacks();
      const restored = new Set(restoredIds);
      const preserved = current.filter(({ id }) => !restored.has(id));
      const desired = new Map<string, Pack>();
      for (const pack of [...index.packs, ...preserved]) desired.set(pack.id, pack);
      await this.applyReplacement([...desired.values()]);
    });
  }

  async getPack(id: string): Promise<Pack | null> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.loadPacks();
      return (await this.ctx.storage.get<Pack>(this.packKey(id))) ?? null;
    });
  }

  async migrationParity(witnessIds: string[]): Promise<MigrationParity> {
    return this.ctx.blockConcurrencyWhile(async () => {
      const kv = await this.readKvIndex();
      await this.mergeFromKv(kv);
      return this.buildParity(kv, witnessIds);
    });
  }

  async adoptWitnesses(ids: string[]): Promise<WitnessAdoption> {
    return this.ctx.blockConcurrencyWhile(async () => {
      const result: WitnessAdoption = {
        adopted: [],
        already_present: [],
        tombstoned: [],
        unavailable: [],
      };
      for (const id of [...new Set(ids)]) {
        if (await this.ctx.storage.get<Pack>(this.packKey(id))) {
          result.already_present.push(id);
          continue;
        }
        if (await this.ctx.storage.get<string>(this.tombstoneKey(id))) {
          result.tombstoned.push(id);
          continue;
        }
        const detail = await this.env.ESO_PACKS.get<Pack>(`pack:${id}`, {
          type: "json",
          cacheTtl: 30,
        });
        if (!detail || detail.id !== id) {
          result.unavailable.push(id);
          continue;
        }
        await this.ctx.storage.put(this.packKey(id), detail);
        result.adopted.push(id);
      }
      if (result.adopted.length > 0) await this.mirror(await this.getStoredPacks());
      return result;
    });
  }

  async setAuthority(
    authority: Authority,
    witnessIds: string[],
  ): Promise<{ ok: true; parity: MigrationParity } | { ok: false; parity: MigrationParity }> {
    return this.ctx.blockConcurrencyWhile(async () => {
      if (authority === "kv") {
        await this.ctx.storage.put(AUTHORITY_KEY, "kv");
        const kv = await this.readKvIndex();
        return { ok: true, parity: await this.buildParity(kv, witnessIds) };
      }

      const kv = await this.readKvIndex();
      await this.mergeFromKv(kv);
      const parity = await this.buildParity(kv, witnessIds);
      if (parity.missing_from_do.length > 0) return { ok: false, parity };
      await this.ctx.storage.put(AUTHORITY_KEY, "do");
      await this.mirror(await this.getStoredPacks());
      return { ok: true, parity: { ...parity, authority: "do" } };
    });
  }

  private async loadPacks(): Promise<Pack[]> {
    const authority = await this.getAuthority();
    if (authority === "do") return this.getStoredPacks();

    const kv = await this.readKvIndex();
    const stored = await this.getStoredPacks();
    if (kv.packs.length === 0 && stored.length > 0) {
      throw new Error("KV index is unexpectedly empty while the DO shadow contains packs");
    }
    await this.mergeFromKv(kv);
    return this.getStoredPacks();
  }

  private async readKvIndex(): Promise<PackIndex> {
    const index = await this.env.ESO_PACKS.get<PackIndex>(INDEX_KEY, {
      type: "json",
      cacheTtl: 30,
    });
    // A namespace that has never been seeded is a valid empty index. loadPacks()
    // still fails closed if a previously populated DO shadow sees an empty KV
    // index, which distinguishes a fresh deployment from accidental KV loss.
    return index ?? { packs: [] };
  }

  private async mergeFromKv(index: PackIndex): Promise<void> {
    const stored = await this.ctx.storage.list<Pack>({ prefix: STORAGE_PACK_PREFIX });
    const tombstones = await this.ctx.storage.list<string>({ prefix: TOMBSTONE_PREFIX });
    for (const pack of index.packs) {
      if (!stored.has(this.packKey(pack.id)) && !tombstones.has(this.tombstoneKey(pack.id))) {
        await this.ctx.storage.put(this.packKey(pack.id), pack);
      }
    }
  }

  private async applyCounter(
    pack: Pack,
    field: "vote_count" | "install_count",
    delta: number,
  ): Promise<Pack> {
    pack[field] = Math.max(0, (pack[field] ?? 0) + delta);
    await this.ctx.storage.put(this.packKey(pack.id), pack);
    await this.mirror(await this.getStoredPacks(), pack);
    return pack;
  }

  private lifecycleMatches(pack: Pack | undefined, expected?: string | Pack | null): boolean {
    if (!pack || pack.status !== "published") return false;
    if (!expected) return true;
    const createdAt = typeof expected === "string" ? expected : expected.created_at;
    return pack.created_at === createdAt;
  }

  private async applyReplacement(packs: Pack[]): Promise<void> {
    const current = await this.getStoredPacks();
    const desiredIds = new Set(packs.map(({ id }) => id));
    const removed = current.filter(({ id }) => !desiredIds.has(id)).map(({ id }) => id);
    for (const id of removed) await this.deleteStoredPack(id);
    for (const pack of packs) {
      await this.ctx.storage.delete(this.tombstoneKey(pack.id));
      await this.ctx.storage.put(this.packKey(pack.id), pack);
    }
    await this.mirror(packs, undefined, undefined, removed);
    this.voteMemo.clear();
  }

  private async deleteStoredPack(id: string): Promise<void> {
    await this.ctx.storage.delete(this.packKey(id));
    await this.ctx.storage.put(this.tombstoneKey(id), new Date().toISOString());
    this.forgetVotes(id);
  }

  private forgetVotes(packId: string): void {
    const prefix = `${packId}:`;
    for (const key of this.voteMemo.keys()) {
      if (key.startsWith(prefix)) this.voteMemo.delete(key);
    }
  }

  private async getAuthority(): Promise<Authority> {
    return (await this.ctx.storage.get<Authority>(AUTHORITY_KEY)) ?? "kv";
  }

  private async getStoredPacks(): Promise<Pack[]> {
    const entries = await this.ctx.storage.list<Pack>({ prefix: STORAGE_PACK_PREFIX });
    return [...entries.values()];
  }

  private async buildParity(kv: PackIndex, witnessIds: string[]): Promise<MigrationParity> {
    const authority = await this.getAuthority();
    const packs = await this.getStoredPacks();
    const doIds = new Set(packs.map(({ id }) => id));
    const kvIds = new Set(kv.packs.map(({ id }) => id));
    const tombstoneEntries = await this.ctx.storage.list<string>({ prefix: TOMBSTONE_PREFIX });
    const tombstones = [...tombstoneEntries.keys()].map((key) => key.slice(TOMBSTONE_PREFIX.length));
    const tombstoneIds = new Set(tombstones);
    const witnesses = new Set([...kvIds, ...witnessIds]);
    return {
      authority,
      kv_count: kvIds.size,
      do_count: doIds.size,
      tombstones: tombstones.sort(),
      do_only: [...doIds].filter((id) => !kvIds.has(id)).sort(),
      missing_from_do: [...witnesses]
        .filter((id) => !doIds.has(id) && !tombstoneIds.has(id))
        .sort(),
    };
  }

  private packKey(id: string): string {
    return `${STORAGE_PACK_PREFIX}${id}`;
  }

  private tombstoneKey(id: string): string {
    return `${TOMBSTONE_PREFIX}${id}`;
  }

  private async mirror(
    packs: Pack[],
    changed?: Pack,
    deletedId?: string,
    deletedIds: string[] = [],
  ): Promise<void> {
    await this.env.ESO_PACKS.put(INDEX_KEY, JSON.stringify({ packs }));
    if (changed) await this.env.ESO_PACKS.put(`pack:${changed.id}`, JSON.stringify(changed));
    for (const id of [...deletedIds, ...(deletedId ? [deletedId] : [])]) {
      await this.env.ESO_PACKS.delete(`pack:${id}`);
    }
  }
}
