import { DurableObject } from "cloudflare:workers";
import type { Env, Pack, PackIndex } from "./types";
import { deleteVote, deleteVotesForPack, getVote, putVote } from "./kv";

const INDEX_KEY = "index:packs";
const STORAGE_PACK_PREFIX = "pack:";
const TOMBSTONE_PREFIX = "tomb:";
const OWNERSHIP_PREFIX = "own:";
const OPERATION_PREFIX = "op:";
const PENDING_PREFIX = "pending:";
const AUTHORITY_KEY = "meta:authority";
const VOTE_MEMO_LIMIT = 5000;
const RETRY_DELAY_MS = 30_000;

type Authority = "kv" | "do";
type MutationResult =
  | { status: "ok"; pack: Pack }
  | { status: "not-found" }
  | { status: "forbidden" };

type AddResult =
  | { ok: true; pack: Pack }
  | { ok: false; reason: "duplicate" | "limit" | "retry" };
type RemoveResult = "ok" | "not-found" | "forbidden" | "retry";

interface Tombstone {
  deletedAt: string;
  authorId: string;
  lifecycle: string;
  operationId: string;
}

interface PendingOperation {
  id: string;
  kind: "create" | "delete";
  packId: string;
  actorId: string;
  lifecycle: string;
  pack: Pack;
  kvDetailDone: boolean;
  votesDone: boolean;
  d1Done: boolean;
  indexDone: boolean;
  attempts: number;
}

export interface MigrationParity {
  authority: Authority;
  kv_count: number;
  do_count: number;
  tombstones: string[];
  do_only: string[];
  missing_from_do: string[];
  stale_shadow: string[];
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
  ): Promise<AddResult> {
    return this.ctx.blockConcurrencyWhile(async () => {
      const packs = await this.loadPacks();
      const pending = await this.getPending(pack.id);
      if (pending) {
        if (pending.kind === "create" && pending.actorId === pack.author_id) {
          return await this.finishCreate(pending)
            ? { ok: true, pack: pending.pack }
            : { ok: false, reason: "retry" };
        }
        return { ok: false, reason: "duplicate" };
      }
      const detail = await this.env.ESO_PACKS.get<Pack>(`pack:${pack.id}`, {
        type: "json",
        cacheTtl: 30,
      });
      if (packs.some(({ id }) => id === pack.id) || detail) {
        return { ok: false, reason: "duplicate" };
      }
      if (
        maxPerAuthor !== undefined &&
        packs.filter(({ author_id }) => author_id === pack.author_id).length >= maxPerAuthor
      ) {
        return { ok: false, reason: "limit" };
      }

      const operation = this.createOperation("create", pack, pack.author_id);
      await this.ctx.storage.transaction(async (txn) => {
        await txn.delete(this.tombstoneKey(pack.id));
        await txn.put(this.packKey(pack.id), pack);
        await txn.put(this.ownershipKey(pack.id), pack.updated_at);
        await txn.put(this.operationKey(operation.id), operation);
        await txn.put(this.pendingKey(pack.id), operation.id);
      });
      return await this.finishCreate(operation)
        ? { ok: true, pack }
        : { ok: false, reason: "retry" };
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
      await this.ctx.storage.put(this.ownershipKey(id), updated.updated_at);
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

  async removePack(id: string, actorId?: string): Promise<RemoveResult> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.loadPacks();
      const existing = await this.ctx.storage.get<Pack>(this.packKey(id));
      if (!existing) {
        const pending = await this.getPending(id);
        if (!pending || pending.kind !== "delete") return "not-found";
        if (actorId !== undefined && pending.actorId !== actorId) return "forbidden";
        return await this.finishDelete(pending) ? "ok" : "retry";
      }
      if (actorId !== undefined && actorId !== existing.author_id) return "forbidden";

      const operation = this.createOperation("delete", existing, actorId ?? existing.author_id);
      const tombstone: Tombstone = {
        deletedAt: new Date().toISOString(),
        authorId: existing.author_id,
        lifecycle: existing.created_at,
        operationId: operation.id,
      };
      await this.ctx.storage.transaction(async (txn) => {
        await txn.delete(this.packKey(id));
        await txn.delete(this.ownershipKey(id));
        await txn.put(this.tombstoneKey(id), tombstone);
        await txn.put(this.operationKey(operation.id), operation);
        await txn.put(this.pendingKey(id), operation.id);
      });
      this.forgetVotes(id);
      return await this.finishDelete(operation) ? "ok" : "retry";
    });
  }

  async removePacksByAuthor(authorId: string): Promise<string[]> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.loadPacks();
      await this.hydrateDetailCorpus();
      const packs = await this.getStoredPacks();
      const removedPacks = packs.filter((pack) => pack.author_id === authorId);
      for (const pack of removedPacks) {
        const operation = this.createOperation("delete", pack, authorId);
        const tombstone: Tombstone = {
          deletedAt: new Date().toISOString(),
          authorId,
          lifecycle: pack.created_at,
          operationId: operation.id,
        };
        await this.ctx.storage.transaction(async (txn) => {
          await txn.delete(this.packKey(pack.id));
          await txn.delete(this.ownershipKey(pack.id));
          await txn.put(this.tombstoneKey(pack.id), tombstone);
          await txn.put(this.operationKey(operation.id), operation);
          await txn.put(this.pendingKey(pack.id), operation.id);
        });
        this.forgetVotes(pack.id);
        await this.finishDelete(operation);
      }
      return removedPacks.map(({ id }) => id);
    });
  }

  async replaceIndex(index: PackIndex): Promise<void> {
    await this.ctx.blockConcurrencyWhile(async () => {
      await this.loadPacks();
      await this.applyReplacement(index.packs, true);
    });
  }

  async replaceIndexPreserving(index: PackIndex, restoredIds: string[]): Promise<void> {
    await this.ctx.blockConcurrencyWhile(async () => {
      const current = await this.loadPacks();
      const restored = new Set(restoredIds);
      const preserved = current.filter(({ id }) => !restored.has(id));
      const desired = new Map<string, Pack>();
      for (const pack of [...index.packs, ...preserved]) desired.set(pack.id, pack);
      await this.applyReplacement([...desired.values()], false);
    });
  }

  async getPack(id: string): Promise<Pack | null> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.loadPacks();
      const stored = await this.ctx.storage.get<Pack>(this.packKey(id));
      if (stored || await this.ctx.storage.get<string>(this.tombstoneKey(id))) {
        return stored ?? null;
      }
      const detail = await this.env.ESO_PACKS.get<Pack>(`pack:${id}`, {
        type: "json",
        cacheTtl: 30,
      });
      if (!detail || detail.id !== id) return null;
      await this.ctx.storage.put(this.packKey(id), detail);
      return detail;
    });
  }

  async getIndex(): Promise<PackIndex> {
    return this.ctx.blockConcurrencyWhile(async () => ({ packs: await this.loadPacks() }));
  }

  async alarm(): Promise<void> {
    await this.ctx.blockConcurrencyWhile(async () => {
      const pending = await this.ctx.storage.list<string>({ prefix: PENDING_PREFIX });
      for (const operationId of pending.values()) {
        const operation = await this.ctx.storage.get<PendingOperation>(
          this.operationKey(operationId),
        );
        if (!operation) continue;
        if (operation.kind === "create") await this.finishCreate(operation);
        else await this.finishDelete(operation);
      }
      if ((await this.ctx.storage.list({ prefix: PENDING_PREFIX })).size > 0) {
        await this.scheduleRetry();
      }
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
      if (parity.missing_from_do.length > 0 || parity.stale_shadow.length > 0) {
        return { ok: false, parity };
      }
      await this.ctx.storage.put(AUTHORITY_KEY, "do");
      await this.mirror(await this.getStoredPacks());
      return { ok: true, parity: { ...parity, authority: "do" } };
    });
  }

  private async loadPacks(): Promise<Pack[]> {
    const authority = await this.getAuthority();
    if (authority === "do") return this.getStoredPacks();

    const kv = await this.readKvIndex();
    await this.mergeFromKv(kv);
    return this.getStoredPacks();
  }

  private async readKvIndex(): Promise<PackIndex> {
    const index = await this.env.ESO_PACKS.get<PackIndex>(INDEX_KEY, {
      type: "json",
      cacheTtl: 30,
    });
    // A namespace that has never been seeded is a valid empty index. In shadow
    // mode an empty/stale index is merged additively and is never mirrored back,
    // so it cannot erase already observed DO records.
    return index ?? { packs: [] };
  }

  private async mergeFromKv(index: PackIndex): Promise<void> {
    const stored = await this.ctx.storage.list<Pack>({ prefix: STORAGE_PACK_PREFIX });
    const tombstones = await this.ctx.storage.list<unknown>({ prefix: TOMBSTONE_PREFIX });
    for (const pack of index.packs) {
      if (tombstones.has(this.tombstoneKey(pack.id))) continue;
      const ownership = await this.ctx.storage.get<string>(this.ownershipKey(pack.id));
      if (ownership) continue;
      const detail = await this.env.ESO_PACKS.get<Pack>(`pack:${pack.id}`, {
        type: "json",
        cacheTtl: 30,
      });
      const candidate = this.newerPack(pack, detail);
      const current = stored.get(this.packKey(pack.id));
      if (!current || this.isNewerOrDifferent(candidate, current)) {
        await this.ctx.storage.put(this.packKey(pack.id), candidate);
      }
    }
  }

  private async hydrateDetailCorpus(): Promise<void> {
    let cursor: string | undefined;
    do {
      const page = await this.env.ESO_PACKS.list({ prefix: "pack:", cursor });
      for (const { name } of page.keys) {
        const id = name.slice("pack:".length);
        if (
          !id ||
          await this.ctx.storage.get<Pack>(this.packKey(id)) ||
          await this.ctx.storage.get<string>(this.tombstoneKey(id))
        ) continue;
        const detail = await this.env.ESO_PACKS.get<Pack>(name, {
          type: "json",
          cacheTtl: 30,
        });
        if (detail?.id === id) await this.ctx.storage.put(this.packKey(id), detail);
      }
      cursor = page.list_complete ? undefined : page.cursor;
    } while (cursor);
  }

  private async applyCounter(
    pack: Pack,
    field: "vote_count" | "install_count",
    delta: number,
  ): Promise<Pack> {
    pack[field] = Math.max(0, (pack[field] ?? 0) + delta);
    await this.ctx.storage.put(this.packKey(pack.id), pack);
    await this.ctx.storage.put(this.ownershipKey(pack.id), pack.updated_at);
    await this.mirror(await this.getStoredPacks(), pack);
    return pack;
  }

  private lifecycleMatches(pack: Pack | undefined, expected?: string | Pack | null): boolean {
    if (!pack || pack.status !== "published") return false;
    if (!expected) return true;
    const createdAt = typeof expected === "string" ? expected : expected.created_at;
    return pack.created_at === createdAt;
  }

  private async applyReplacement(packs: Pack[], forceIndex: boolean): Promise<void> {
    const current = await this.getStoredPacks();
    const accepted: Pack[] = [];
    for (const pack of packs) {
      const pending = await this.getPending(pack.id);
      if (pending?.kind !== "delete") accepted.push(pack);
    }
    const desiredIds = new Set(accepted.map(({ id }) => id));
    const removed = current.filter(({ id }) => !desiredIds.has(id));
    const deleteOperations: PendingOperation[] = [];
    for (const pack of removed) {
      const operation = this.createOperation("delete", pack, pack.author_id);
      const tombstone: Tombstone = {
        deletedAt: new Date().toISOString(),
        authorId: pack.author_id,
        lifecycle: pack.created_at,
        operationId: operation.id,
      };
      await this.ctx.storage.transaction(async (txn) => {
        await txn.delete(this.packKey(pack.id));
        await txn.delete(this.ownershipKey(pack.id));
        await txn.put(this.tombstoneKey(pack.id), tombstone);
        await txn.put(this.operationKey(operation.id), operation);
        await txn.put(this.pendingKey(pack.id), operation.id);
      });
      this.forgetVotes(pack.id);
      deleteOperations.push(operation);
    }
    for (const pack of accepted) {
      await this.ctx.storage.delete(this.tombstoneKey(pack.id));
      await this.ctx.storage.put(this.packKey(pack.id), pack);
      await this.ctx.storage.put(this.ownershipKey(pack.id), pack.updated_at);
    }
    await this.mirror(accepted, undefined, undefined, [], forceIndex);
    for (const operation of deleteOperations) await this.finishDelete(operation);
    this.voteMemo.clear();
  }

  private async deleteD1Pack(id: string): Promise<boolean> {
    if (!this.env.ROSTER_HUB_DB) return true;
    try {
      await this.env.ROSTER_HUB_DB.batch([
        this.env.ROSTER_HUB_DB.prepare("DELETE FROM pack_tags WHERE pack_id = ?").bind(id),
        this.env.ROSTER_HUB_DB.prepare("DELETE FROM packs WHERE id = ?").bind(id),
      ]);
      return true;
    } catch (error) {
      console.error(`D1 delete failed [${id}]:`, error);
      // Local/preview namespaces may bind an empty D1 database. There is no
      // external row to reconcile in that case; production's shared database
      // has both tables and every other failure remains journaled for retry.
      return error instanceof Error && error.message.includes("no such table");
    }
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
    const tombstoneEntries = await this.ctx.storage.list<unknown>({ prefix: TOMBSTONE_PREFIX });
    const tombstones = [...tombstoneEntries.keys()].map((key) => key.slice(TOMBSTONE_PREFIX.length));
    const tombstoneIds = new Set(tombstones);
    const witnesses = new Set([...kvIds, ...witnessIds]);
    const staleShadow: string[] = [];
    for (const kvPack of kv.packs) {
      if (tombstoneIds.has(kvPack.id)) continue;
      const stored = packs.find(({ id }) => id === kvPack.id);
      const detail = await this.env.ESO_PACKS.get<Pack>(`pack:${kvPack.id}`, {
        type: "json",
        cacheTtl: 30,
      });
      const witness = this.newerPack(kvPack, detail);
      if (!stored || this.isNewerOrDifferent(witness, stored)) staleShadow.push(kvPack.id);
    }
    return {
      authority,
      kv_count: kvIds.size,
      do_count: doIds.size,
      tombstones: tombstones.sort(),
      do_only: [...doIds].filter((id) => !kvIds.has(id)).sort(),
      missing_from_do: [...witnesses]
        .filter((id) => !doIds.has(id) && !tombstoneIds.has(id))
        .sort(),
      stale_shadow: staleShadow.sort(),
    };
  }

  private packKey(id: string): string {
    return `${STORAGE_PACK_PREFIX}${id}`;
  }

  private tombstoneKey(id: string): string {
    return `${TOMBSTONE_PREFIX}${id}`;
  }

  private ownershipKey(id: string): string {
    return `${OWNERSHIP_PREFIX}${id}`;
  }

  private pendingKey(id: string): string {
    return `${PENDING_PREFIX}${id}`;
  }

  private operationKey(id: string): string {
    return `${OPERATION_PREFIX}${id}`;
  }

  private createOperation(
    kind: "create" | "delete",
    pack: Pack,
    actorId: string,
  ): PendingOperation {
    const lifecycle = pack.created_at;
    return {
      id: `${kind}:${pack.id}:${actorId}:${lifecycle}`,
      kind,
      packId: pack.id,
      actorId,
      lifecycle,
      pack,
      kvDetailDone: false,
      votesDone: kind === "create",
      d1Done: false,
      indexDone: false,
      attempts: 0,
    };
  }

  private async getPending(packId: string): Promise<PendingOperation | null> {
    const operationId = await this.ctx.storage.get<string>(this.pendingKey(packId));
    if (!operationId) return null;
    return (await this.ctx.storage.get<PendingOperation>(this.operationKey(operationId))) ?? null;
  }

  private async saveOperation(operation: PendingOperation): Promise<void> {
    await this.ctx.storage.put(this.operationKey(operation.id), operation);
  }

  private async finishCreate(operation: PendingOperation): Promise<boolean> {
    operation.attempts++;
    if (!operation.kvDetailDone) {
      try {
        await this.env.ESO_PACKS.put(
          `pack:${operation.packId}`,
          JSON.stringify(operation.pack),
        );
        operation.kvDetailDone = true;
        await this.saveOperation(operation);
      } catch (error) {
        console.error(`KV create mirror failed [${operation.packId}]:`, error);
        await this.saveOperation(operation);
        await this.scheduleRetry();
        return false;
      }
    }

    if (!operation.d1Done) {
      operation.d1Done = await this.upsertD1Pack(operation.pack);
      await this.saveOperation(operation);
    }
    if (!operation.indexDone) {
      operation.indexDone = await this.mirrorIndexIfAuthoritative();
      await this.saveOperation(operation);
    }
    await this.finishOperationIfComplete(operation);
    return true;
  }

  private async finishDelete(operation: PendingOperation): Promise<boolean> {
    operation.attempts++;
    if (!operation.votesDone) {
      try {
        await deleteVotesForPack(this.env, operation.packId);
        operation.votesDone = true;
        await this.saveOperation(operation);
      } catch (error) {
        console.error(`KV vote cleanup failed [${operation.packId}]:`, error);
        await this.saveOperation(operation);
        await this.scheduleRetry();
        return false;
      }
    }
    if (!operation.kvDetailDone) {
      try {
        await this.env.ESO_PACKS.delete(`pack:${operation.packId}`);
        operation.kvDetailDone = true;
        await this.saveOperation(operation);
      } catch (error) {
        console.error(`KV delete mirror failed [${operation.packId}]:`, error);
        await this.saveOperation(operation);
        await this.scheduleRetry();
        return false;
      }
    }
    if (!operation.d1Done) {
      operation.d1Done = await this.deleteD1Pack(operation.packId);
      await this.saveOperation(operation);
    }
    if (!operation.indexDone) {
      operation.indexDone = await this.mirrorIndexIfAuthoritative();
      await this.saveOperation(operation);
    }
    await this.finishOperationIfComplete(operation);
    return true;
  }

  private async finishOperationIfComplete(operation: PendingOperation): Promise<void> {
    if (
      !operation.kvDetailDone ||
      !operation.votesDone ||
      !operation.d1Done ||
      !operation.indexDone
    ) {
      await this.scheduleRetry();
      return;
    }
    await this.ctx.storage.transaction(async (txn) => {
      await txn.delete(this.pendingKey(operation.packId));
      await txn.delete(this.operationKey(operation.id));
    });
  }

  private async mirrorIndexIfAuthoritative(): Promise<boolean> {
    if (await this.getAuthority() !== "do") return true;
    try {
      await this.env.ESO_PACKS.put(
        INDEX_KEY,
        JSON.stringify({ packs: await this.getStoredPacks() }),
      );
      return true;
    } catch (error) {
      console.error("KV index mirror failed:", error);
      return false;
    }
  }

  private async scheduleRetry(): Promise<void> {
    const current = await this.ctx.storage.getAlarm();
    const desired = Date.now() + RETRY_DELAY_MS;
    if (current === null || current > desired) await this.ctx.storage.setAlarm(desired);
  }

  private newerPack(indexPack: Pack, detail?: Pack | null): Pack {
    if (!detail || detail.id !== indexPack.id) return indexPack;
    return detail.updated_at >= indexPack.updated_at ? detail : indexPack;
  }

  private isNewerOrDifferent(candidate: Pack, current: Pack): boolean {
    return candidate.updated_at > current.updated_at ||
      (candidate.updated_at === current.updated_at &&
        JSON.stringify(candidate) !== JSON.stringify(current));
  }

  private async upsertD1Pack(pack: Pack): Promise<boolean> {
    if (!this.env.ROSTER_HUB_DB) return true;
    try {
      if (pack.status !== "published") {
        return await this.deleteD1Pack(pack.id);
      }
      const addons = JSON.stringify(pack.addons.map((addon) => ({
        esouiId: addon.esouiId,
        name: addon.name,
        required: addon.required,
        note: addon.note,
      })));
      await this.env.ROSTER_HUB_DB.prepare(
        `INSERT INTO packs (id, author_id, author_name, is_anonymous, title, description, pack_type, addons, vote_count, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
         ON CONFLICT(id) DO UPDATE SET title = excluded.title, description = excluded.description,
           pack_type = excluded.pack_type, addons = excluded.addons,
           is_anonymous = excluded.is_anonymous, author_name = excluded.author_name,
           vote_count = excluded.vote_count, updated_at = datetime('now')`,
      ).bind(
        pack.id,
        pack.author_id,
        pack.is_anonymous ? "Anonymous" : pack.author_name,
        pack.is_anonymous ? 1 : 0,
        pack.title,
        pack.description,
        pack.pack_type,
        addons,
        pack.vote_count ?? 0,
      ).run();
      await this.env.ROSTER_HUB_DB.batch([
        this.env.ROSTER_HUB_DB.prepare("DELETE FROM pack_tags WHERE pack_id = ?").bind(pack.id),
        ...pack.tags.map((tag) =>
          this.env.ROSTER_HUB_DB!.prepare(
            "INSERT OR IGNORE INTO pack_tags (pack_id, tag) VALUES (?, ?)",
          ).bind(pack.id, tag),
        ),
      ]);
      return true;
    } catch (error) {
      console.error(`D1 upsert failed [${pack.id}]:`, error);
      return error instanceof Error && error.message.includes("no such table");
    }
  }

  private async mirror(
    packs: Pack[],
    changed?: Pack,
    deletedId?: string,
    deletedIds: string[] = [],
    forceIndex = false,
  ): Promise<void> {
    // During shadow mode, an eventually consistent KV read may omit a live
    // pre-deploy pack. Never publish that incomplete shadow back over the full
    // index. Reads go through getIndex() until the parity-gated authority flip.
    if (forceIndex || await this.getAuthority() === "do") {
      await this.env.ESO_PACKS.put(INDEX_KEY, JSON.stringify({ packs }));
    }
    if (changed) await this.env.ESO_PACKS.put(`pack:${changed.id}`, JSON.stringify(changed));
    for (const id of [...deletedIds, ...(deletedId ? [deletedId] : [])]) {
      await this.env.ESO_PACKS.delete(`pack:${id}`);
    }
  }
}
