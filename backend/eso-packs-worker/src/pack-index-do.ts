import { DurableObject } from "cloudflare:workers";
import type { Env, Pack, PackIndex, VoteRecord } from "./types";
import { deleteVote, deleteVotesForPack, getVote, putVote } from "./kv";
import { recordD1MirrorFailure, toD1PackRow } from "./d1-reconcile";
import { rememberBounded } from "./bounded-map";

const INDEX_KEY = "index:packs";
const STORAGE_PACK_PREFIX = "pack:";
const TOMBSTONE_PREFIX = "tomb:";
const OWNERSHIP_PREFIX = "own:";
const OPERATION_PREFIX = "op:";
const PENDING_PREFIX = "pending:";
const DIRTY_MIRROR_PREFIX = "dirty:";
const DELETED_AUTHOR_PREFIX = "deleted-author:";
const AUTHORITY_KEY = "meta:authority";
const RECONCILIATION_LEASE_KEY = "meta:d1-reconciliation-lease";
const RECONCILIATION_LEASE_MS = 48 * 60 * 60 * 1000;
const VOTE_MEMO_LIMIT = 5000;
const RETRY_DELAY_MS = 30_000;
const BACKUP_SIZE_WARN_BYTES = 20 * 1024 * 1024;
const DELETED_AUTHOR_TTL_MS = 97 * 86400 * 1000;
const RESTORE_ACTIVE_KEY = "restore:active";
const RESTORE_JOB_PREFIX = "restore:job:";
const RESTORE_SNAPSHOT_PREFIX = "restore:snapshot:";
const RESTORE_JOB_TTL_MS = 24 * 60 * 60 * 1000;
const RESTORE_CLAIM_TTL_MS = 5 * 60 * 1000;

interface BackupSnapshot {
  created_at: string;
  packs: Pack[];
  packBodies: Record<string, Pack>;
  votes: Record<string, VoteRecord>;
}

interface BackupMeta {
  last_success: number;
  last_backup_key: string;
  pack_count: number;
  pack_body_count: number;
  vote_count: number;
}

interface DeletedAuthorMarker {
  v: 1;
  userId: string;
  deletedAt: string;
  expiresAt: number;
}

type DeletedAuthorStored = DeletedAuthorMarker | string;

interface RestoreActiveJob {
  v: 1;
  jobId: string;
  expiresAt: number;
}

interface RestoreInFlight {
  claimId: string;
  start: number;
  end: number;
  expiresAt: number;
}

interface RestoreJobRecord {
  v: 1;
  jobId: string;
  tokenHash: string;
  backupKey: string;
  snapshotCreatedAt: string | null;
  snapshotFingerprint: string;
  total: number;
  nextCursor: number;
  status: "running" | "done" | "cancelled";
  createdAt: number;
  updatedAt: number;
  expiresAt: number;
  inFlight?: RestoreInFlight;
}

export interface RestoreJobState {
  jobId: string;
  backupKey: string;
  snapshotCreatedAt: string | null;
  snapshotFingerprint: string;
  total: number;
  nextCursor: number;
  status: "running" | "done" | "cancelled";
  expiresAt: number;
  inFlight?: RestoreInFlight;
}

export type BeginRestoreJobResult =
  | { ok: true; token: string; job: RestoreJobState }
  | { ok: false; reason: "active"; job: RestoreJobState };

export type RestoreClaimResult =
  | { ok: true; job: RestoreJobState; claimId: string; start: number; end: number; final: boolean }
  | {
      ok: false;
      reason: "not-found" | "expired" | "done" | "cancelled" | "cursor-mismatch" | "in-flight";
      job?: RestoreJobState;
    };

export type RestoreCompleteResult =
  | { ok: true; job: RestoreJobState }
  | {
      ok: false;
      reason: "not-found" | "expired" | "done" | "cancelled" | "claim-mismatch";
      job?: RestoreJobState;
    };

const INSTALL_WINDOW_MS = 60 * 60 * 1000;
const INSTALL_RING_SIZE = 5000;
const INSTALL_DELETE_BATCH_SIZE = 100;
const INSTALL_SEQUENCE_KEY = "meta:install-sequence";
const INSTALL_MARKER_PREFIX = "install-marker:";
const INSTALL_SLOT_PREFIX = "install-slot:";

interface InstallSlot {
  markerKey: string;
  recordedAt: number;
}

interface InstallMarker extends InstallSlot {
  slotKey: string;
}

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

export interface ReconciliationAuthority {
  authority: Authority;
  packs: Pack[];
  tombstones: string[];
}

interface ReconciliationLease {
  token: string;
  expires_at: number;
}

function base64Url(bytes: ArrayBuffer | Uint8Array): string {
  const array = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let binary = "";
  for (const byte of array) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

async function sha256Base64Url(value: string): Promise<string> {
  return base64Url(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value)));
}

function randomBase64Url(length: number): string {
  const bytes = new Uint8Array(length);
  crypto.getRandomValues(bytes);
  return base64Url(bytes);
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
        await txn.delete(`${DELETED_AUTHOR_PREFIX}${pack.author_id}`);
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
      if (pack.created_at !== existing.created_at) return { status: "not-found" };

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
      await this.mirrorChangedBestEffort(updated);
      await this.mirrorD1Pack(updated);
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

  /**
   * Atomically claim an install identity and increment its pack once per hour.
   * The fixed-size persistent ring bounds DO storage. Eviction can admit an
   * older identity before its hour elapses only after 5,000 newer identities,
   * an explicit bound preferable to unbounded per-IP records.
   */
  async recordInstall(
    id: string,
    identity: string,
    expectedLifecycle?: string | Pack | null,
    now = Date.now(),
  ): Promise<Pack | null> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.loadPacks();
      const result = await this.ctx.storage.transaction(async (txn) => {
        const pack = await txn.get<Pack>(this.packKey(id));
        if (!this.lifecycleMatches(pack, expectedLifecycle)) return null;

        const markerKey = `${INSTALL_MARKER_PREFIX}${id}:${pack!.created_at}:${identity}`;
        const marker = await txn.get<InstallMarker>(markerKey);
        if (marker && now - marker.recordedAt < INSTALL_WINDOW_MS) {
          return { pack: pack!, claimed: false };
        }

        if (marker) await txn.delete(marker.slotKey);
        const sequence = (await txn.get<number>(INSTALL_SEQUENCE_KEY)) ?? 0;
        const slotKey = `${INSTALL_SLOT_PREFIX}${sequence % INSTALL_RING_SIZE}`;
        const evicted = await txn.get<InstallSlot>(slotKey);
        if (evicted) await txn.delete(evicted.markerKey);

        const updated = { ...pack!, install_count: (pack!.install_count ?? 0) + 1 };
        await txn.put(this.packKey(id), updated);
        await txn.put(markerKey, { markerKey, recordedAt: now, slotKey } satisfies InstallMarker);
        await txn.put(slotKey, { markerKey, recordedAt: now } satisfies InstallSlot);
        await txn.put(INSTALL_SEQUENCE_KEY, sequence + 1);
        const cleanupAt = Math.max(now, Date.now()) + INSTALL_WINDOW_MS;
        const currentAlarm = await txn.getAlarm();
        if (currentAlarm === null || currentAlarm > cleanupAt) await txn.setAlarm(cleanupAt);
        return { pack: updated, claimed: true };
      });
      if (!result) return null;

      if (result.claimed) {
        await this.mirror(await this.getStoredPacks(), result.pack);
      } else {
        // A duplicate is normally a read-only idempotent response. Only
        // repair a missing or stale detail mirror left by an earlier failure;
        // harmless retries must not rewrite the KV index unconditionally.
        const mirrored = await this.env.ESO_PACKS.get<Pack>(this.packKey(id), "json");
        if (
          !mirrored ||
          mirrored.created_at !== result.pack.created_at ||
          mirrored.updated_at !== result.pack.updated_at ||
          mirrored.install_count !== result.pack.install_count
        ) {
          await this.mirror(await this.getStoredPacks(), result.pack);
        }
      }
      return result.pack;
    });
  }

  async cleanupInstallClaims(now = Date.now()): Promise<number> {
    return this.ctx.blockConcurrencyWhile(async () => {
      let removed = 0;
      let nextExpiry: number | undefined;
      let startAfter: string | undefined;
      while (true) {
        const slots = await this.ctx.storage.list<InstallSlot>({
          prefix: INSTALL_SLOT_PREFIX,
          startAfter,
          limit: 1000,
        });
        for (const [slotKey, slot] of slots) {
          const expiresAt = slot.recordedAt + INSTALL_WINDOW_MS;
          if (expiresAt <= now) {
            const marker = await this.ctx.storage.get<InstallMarker>(slot.markerKey);
            await this.ctx.storage.delete(slotKey);
            if (marker?.slotKey === slotKey) await this.ctx.storage.delete(slot.markerKey);
            removed += 1;
          } else {
            nextExpiry = Math.min(nextExpiry ?? expiresAt, expiresAt);
          }
        }
        if (slots.size < 1000) break;
        startAfter = [...slots.keys()].at(-1);
      }
      if (nextExpiry === undefined) await this.ctx.storage.deleteAlarm();
      else await this.ctx.storage.setAlarm(nextExpiry);
      return removed;
    });
  }

  async beginRestoreJob(input: {
    backupKey: string;
    snapshotCreatedAt: string | null;
    snapshotFingerprint: string;
    total: number;
    restart?: boolean;
    now?: number;
    ttlMs?: number;
  }): Promise<BeginRestoreJobResult> {
    return this.ctx.blockConcurrencyWhile(async () => {
      const now = input.now ?? Date.now();
      const ttlMs = input.ttlMs ?? RESTORE_JOB_TTL_MS;
      const active = await this.ctx.storage.get<RestoreActiveJob>(RESTORE_ACTIVE_KEY);
      if (active) {
        const activeKey = this.restoreJobKey(active.jobId);
        const activeJob = await this.ctx.storage.get<RestoreJobRecord>(activeKey);
        const stillRunning =
          active.expiresAt > now &&
          activeJob !== undefined &&
          activeJob.expiresAt > now &&
          activeJob.status === "running";
        if (stillRunning) {
          if (!input.restart) {
            return { ok: false, reason: "active", job: this.publicRestoreState(activeJob) };
          }
          await this.ctx.storage.put(activeKey, {
            ...activeJob,
            status: "cancelled",
            updatedAt: now,
          });
          await this.env.ESO_PACKS.delete(`${RESTORE_SNAPSHOT_PREFIX}${active.jobId}`);
        }
        await this.ctx.storage.delete(RESTORE_ACTIVE_KEY);
      }

      const jobId = crypto.randomUUID();
      const token = `rst_v1_${jobId}.${randomBase64Url(32)}`;
      const job: RestoreJobRecord = {
        v: 1,
        jobId,
        tokenHash: await sha256Base64Url(token),
        backupKey: input.backupKey,
        snapshotCreatedAt: input.snapshotCreatedAt,
        snapshotFingerprint: input.snapshotFingerprint,
        total: Math.max(0, Math.floor(input.total)),
        nextCursor: 0,
        status: "running",
        createdAt: now,
        updatedAt: now,
        expiresAt: now + ttlMs,
      };

      await this.ctx.storage.put(this.restoreJobKey(job.jobId), job);
      await this.ctx.storage.put(RESTORE_ACTIVE_KEY, {
        v: 1,
        jobId: job.jobId,
        expiresAt: job.expiresAt,
      } satisfies RestoreActiveJob);
      await this.scheduleAlarmAt(job.expiresAt);
      return { ok: true, token, job: this.publicRestoreState(job) };
    });
  }

  async resolveRestoreJob(tokenHash: string, now = Date.now()): Promise<RestoreJobState | null> {
    return this.ctx.blockConcurrencyWhile(async () => {
      const found = await this.findRestoreJobByHash(tokenHash);
      if (!found) return null;
      if (found.job.expiresAt <= now || found.job.status === "cancelled") {
        await this.ctx.storage.delete(found.key);
        await this.deleteActiveRestoreIf(found.job.jobId);
        await this.env.ESO_PACKS.delete(`${RESTORE_SNAPSHOT_PREFIX}${found.job.jobId}`);
        return null;
      }
      return this.publicRestoreState(found.job);
    });
  }

  async claimRestorePage(input: {
    tokenHash: string;
    limit: number;
    cursor?: number;
    now?: number;
  }): Promise<RestoreClaimResult> {
    return this.ctx.blockConcurrencyWhile(async () => {
      const now = input.now ?? Date.now();
      const found = await this.findRestoreJobByHash(input.tokenHash);
      if (!found) return { ok: false, reason: "not-found" };
      const job = found.job;
      const state = () => this.publicRestoreState(job);
      if (job.expiresAt <= now) return { ok: false, reason: "expired", job: state() };
      if (job.status === "cancelled") return { ok: false, reason: "cancelled", job: state() };
      if (job.status === "done") return { ok: false, reason: "done", job: state() };

      const requestedCursor = input.cursor ?? job.nextCursor;
      if (requestedCursor !== job.nextCursor) {
        return { ok: false, reason: "cursor-mismatch", job: state() };
      }
      if (job.inFlight && job.inFlight.expiresAt > now) {
        return { ok: false, reason: "in-flight", job: state() };
      }

      const start = job.nextCursor;
      const pageSize = Math.max(1, Math.floor(input.limit));
      const end = Math.min(start + pageSize, job.total);
      const claimId = crypto.randomUUID();
      job.inFlight = {
        claimId,
        start,
        end,
        expiresAt: now + RESTORE_CLAIM_TTL_MS,
      };
      job.updatedAt = now;
      await this.ctx.storage.put(found.key, job);
      await this.scheduleAlarmAt(Math.min(job.inFlight.expiresAt, job.expiresAt));
      return {
        ok: true,
        job: this.publicRestoreState(job),
        claimId,
        start,
        end,
        final: end >= job.total,
      };
    });
  }

  async completeRestorePage(input: {
    tokenHash: string;
    claimId: string;
    end: number;
    finalReplacement?: { packs: Pack[]; restoredIds: string[] };
    now?: number;
  }): Promise<RestoreCompleteResult> {
    return this.ctx.blockConcurrencyWhile(async () => {
      const now = input.now ?? Date.now();
      const found = await this.findRestoreJobByHash(input.tokenHash);
      if (!found) return { ok: false, reason: "not-found" };
      const job = found.job;
      const state = () => this.publicRestoreState(job);
      if (job.expiresAt <= now) return { ok: false, reason: "expired", job: state() };
      if (job.status === "cancelled") return { ok: false, reason: "cancelled", job: state() };
      if (job.status === "done") return { ok: false, reason: "done", job: state() };
      if (
        !job.inFlight ||
        job.inFlight.claimId !== input.claimId ||
        job.inFlight.end !== input.end
      ) {
        return { ok: false, reason: "claim-mismatch", job: state() };
      }
      if (job.inFlight.expiresAt <= now) {
        delete job.inFlight;
        job.updatedAt = now;
        await this.ctx.storage.put(found.key, job);
        return { ok: false, reason: "expired", job: this.publicRestoreState(job) };
      }

      if (input.finalReplacement) {
        const current = await this.loadPacks();
        const restored = new Set(input.finalReplacement.restoredIds);
        const preserved = current.filter(({ id }) => !restored.has(id));
        const desired = new Map<string, Pack>();
        for (const pack of [...input.finalReplacement.packs, ...preserved]) {
          desired.set(pack.id, pack);
        }
        await this.applyReplacement([...desired.values()], false);
      }

      job.nextCursor = input.end;
      job.updatedAt = now;
      delete job.inFlight;
      if (input.end >= job.total) {
        job.status = "done";
        await this.deleteActiveRestoreIf(job.jobId);
      }
      await this.ctx.storage.put(found.key, job);
      return { ok: true, job: this.publicRestoreState(job) };
    });
  }

  async cancelActiveRestoreJob(tokenHash?: string, now = Date.now()): Promise<boolean> {
    return this.ctx.blockConcurrencyWhile(async () => {
      let found: { key: string; job: RestoreJobRecord } | null = null;
      if (tokenHash) found = await this.findRestoreJobByHash(tokenHash);
      if (!found) {
        const active = await this.ctx.storage.get<RestoreActiveJob>(RESTORE_ACTIVE_KEY);
        if (!active) return false;
        const job = await this.ctx.storage.get<RestoreJobRecord>(this.restoreJobKey(active.jobId));
        if (!job) {
          await this.ctx.storage.delete(RESTORE_ACTIVE_KEY);
          return false;
        }
        found = { key: this.restoreJobKey(active.jobId), job };
      }

      found.job.status = "cancelled";
      found.job.updatedAt = now;
      delete found.job.inFlight;
      await this.ctx.storage.put(found.key, found.job);
      await this.deleteActiveRestoreIf(found.job.jobId);
      await this.env.ESO_PACKS.delete(`${RESTORE_SNAPSHOT_PREFIX}${found.job.jobId}`);
      return true;
    });
  }

  async cleanupDeletedAuthors(now = Date.now()): Promise<number> {
    return this.ctx.blockConcurrencyWhile(async () => {
      let removed = 0;
      let nextExpiry: number | undefined;
      let startAfter: string | undefined;
      while (true) {
        const markers = await this.ctx.storage.list<DeletedAuthorStored>({
          prefix: DELETED_AUTHOR_PREFIX,
          startAfter,
          limit: 1000,
        });
        for (const [key, marker] of markers) {
          const expiresAt = this.deletedAuthorExpiresAt(marker);
          if (expiresAt <= now) {
            await this.ctx.storage.delete(key);
            removed += 1;
          } else if (Number.isFinite(expiresAt)) {
            nextExpiry = Math.min(nextExpiry ?? expiresAt, expiresAt);
          }
        }
        if (markers.size < 1000) break;
        startAfter = [...markers.keys()].at(-1);
      }
      if (nextExpiry !== undefined) await this.scheduleAlarmAt(nextExpiry);
      return removed;
    });
  }

  async cleanupRestoreJobs(now = Date.now()): Promise<number> {
    return this.ctx.blockConcurrencyWhile(async () => {
      let removed = 0;
      let nextExpiry: number | undefined;
      let startAfter: string | undefined;
      while (true) {
        const jobs = await this.ctx.storage.list<RestoreJobRecord>({
          prefix: RESTORE_JOB_PREFIX,
          startAfter,
          limit: 1000,
        });
        for (const [key, job] of jobs) {
          if (job.expiresAt <= now) {
            await this.ctx.storage.delete(key);
            await this.deleteActiveRestoreIf(job.jobId);
            await this.env.ESO_PACKS.delete(`${RESTORE_SNAPSHOT_PREFIX}${job.jobId}`);
            removed += 1;
            continue;
          }
          nextExpiry = Math.min(nextExpiry ?? job.expiresAt, job.expiresAt);
          if (job.inFlight) {
            nextExpiry = Math.min(nextExpiry, job.inFlight.expiresAt);
          }
        }
        if (jobs.size < 1000) break;
        startAfter = [...jobs.keys()].at(-1);
      }
      const active = await this.ctx.storage.get<RestoreActiveJob>(RESTORE_ACTIVE_KEY);
      if (active?.expiresAt !== undefined && active.expiresAt <= now) {
        await this.ctx.storage.delete(RESTORE_ACTIVE_KEY);
      }
      if (nextExpiry !== undefined) await this.scheduleAlarmAt(nextExpiry);
      return removed;
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

      rememberBounded(this.voteMemo, memoKey, voted, VOTE_MEMO_LIMIT);
      const pack = await this.applyCounter(existing!, "vote_count", voted ? 1 : -1);
      return { voted, pack };
    });
  }

  async removePack(
    id: string,
    actorId?: string,
    expectedLifecycle?: string,
  ): Promise<RemoveResult> {
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
      if (expectedLifecycle !== undefined && expectedLifecycle !== existing.created_at) {
        return "not-found";
      }

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
      // This latch shares the same serialization boundary as backup writes.
      // Once present, no stale cron snapshot can publish this author's data.
      const now = Date.now();
      const deletedAt = new Date(now).toISOString();
      const marker = {
        v: 1,
        userId: authorId,
        deletedAt,
        expiresAt: now + DELETED_AUTHOR_TTL_MS,
      } satisfies DeletedAuthorMarker;
      await this.ctx.storage.put(`${DELETED_AUTHOR_PREFIX}${authorId}`, marker);
      await this.scheduleAlarmAt(marker.expiresAt);
      await this.loadPacks();
      await this.hydrateDetailsByAuthor(authorId);
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

  async writeBackup(
    backupKey: string,
    incoming: BackupSnapshot,
  ): Promise<BackupMeta> {
    return this.ctx.blockConcurrencyWhile(async () => {
      const deletedAuthors = await this.getDeletedAuthorIds();
      const incomingIds = new Set(incoming.packs.map(({ id }) => id));
      const packs = (await this.loadPacks()).filter(
        (pack) => incomingIds.has(pack.id) && !deletedAuthors.has(String(pack.author_id)),
      );
      const liveIds = new Set(packs.map(({ id }) => id));
      const votes = Object.fromEntries(
        Object.entries(incoming.votes).filter(([, vote]) =>
          liveIds.has(vote.packId) && !deletedAuthors.has(String(vote.userId)),
        ),
      );
      const snapshot: BackupSnapshot = {
        created_at: incoming.created_at,
        packs,
        packBodies: Object.fromEntries(packs.map((pack) => [pack.id, pack])),
        votes,
      };
      const serialized = JSON.stringify(snapshot);
      if (serialized.length > BACKUP_SIZE_WARN_BYTES) {
        console.warn(
          `Backup snapshot for ${backupKey} is ${serialized.length} bytes, approaching KV's 25MB value limit`,
        );
      }
      await this.env.ESO_PACKS.put(backupKey, serialized, { expirationTtl: 90 * 86400 });
      await this.env.ESO_PACKS.put("backup:latest", serialized);
      const meta: BackupMeta = {
        last_success: Date.now(),
        last_backup_key: backupKey,
        pack_count: packs.length,
        pack_body_count: packs.length,
        vote_count: Object.keys(votes).length,
      };
      await this.env.ESO_PACKS.put("backup:meta", JSON.stringify(meta));
      return meta;
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
      // Never auto-adopt an orphan detail key. Restore pages and failed legacy
      // deletes can leave a valid-looking body outside the live index; only
      // shadow index reconciliation or the adjudicated admin adoption endpoint
      // may make one canonical.
      return stored ?? null;
    });
  }

  async getIndex(): Promise<PackIndex> {
    return this.ctx.blockConcurrencyWhile(async () => ({ packs: await this.loadPacks() }));
  }

  async alarm(): Promise<void> {
    await this.cleanupInstallClaims();
    await this.cleanupDeletedAuthors();
    await this.cleanupRestoreJobs();
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
      const dirty = await this.ctx.storage.list<string>({ prefix: DIRTY_MIRROR_PREFIX });
      for (const [key, lifecycle] of dirty) {
        const packId = key.slice(DIRTY_MIRROR_PREFIX.length);
        const pack = await this.ctx.storage.get<Pack>(this.packKey(packId));
        if (!pack || pack.created_at !== lifecycle) {
          await this.ctx.storage.delete(key);
          continue;
        }
        try {
          await this.mirror(await this.getStoredPacks(), pack);
          await this.ctx.storage.delete(key);
        } catch (error) {
          console.error(`KV dirty mirror retry failed [${packId}]:`, error);
        }
      }
      if (
        (await this.ctx.storage.list({ prefix: PENDING_PREFIX })).size > 0 ||
        (await this.ctx.storage.list({ prefix: DIRTY_MIRROR_PREFIX })).size > 0
      ) {
        await this.scheduleRetry();
      }
    });
  }

  async getReconciliationState(): Promise<ReconciliationAuthority> {
    return this.ctx.blockConcurrencyWhile(async () => {
      const packs = await this.loadPacks();
      const authority = await this.getAuthority();
      const entries = await this.ctx.storage.list<string>({ prefix: TOMBSTONE_PREFIX });
      return {
        authority,
        packs,
        tombstones: [...entries.keys()]
          .map((key) => key.slice(TOMBSTONE_PREFIX.length))
          .sort(),
      };
    });
  }

  async beginReconciliation(token: string): Promise<boolean> {
    return this.ctx.blockConcurrencyWhile(async () => {
      const now = Date.now();
      const current = await this.ctx.storage.get<ReconciliationLease>(RECONCILIATION_LEASE_KEY);
      if (current && current.expires_at > now) return false;
      await this.ctx.storage.put(RECONCILIATION_LEASE_KEY, {
        token,
        expires_at: now + RECONCILIATION_LEASE_MS,
      } satisfies ReconciliationLease);
      return true;
    });
  }

  async endReconciliation(token: string): Promise<void> {
    await this.ctx.blockConcurrencyWhile(async () => {
      const current = await this.ctx.storage.get<ReconciliationLease>(RECONCILIATION_LEASE_KEY);
      if (current?.token === token) await this.ctx.storage.delete(RECONCILIATION_LEASE_KEY);
    });
  }

  /** Recheck liveness and remove an obsolete D1 row under the same lifecycle
   * gate as create/update. This closes the check-then-delete window where a
   * reused slug could otherwise be published between an RPC check and D1 I/O. */
  async reconcileDeleteD1(id: string): Promise<boolean> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.loadPacks();
      const current = await this.ctx.storage.get<Pack>(this.packKey(id));
      if (current?.status === "published" || !this.env.ROSTER_HUB_DB) return false;
      await this.env.ROSTER_HUB_DB.batch([
        this.env.ROSTER_HUB_DB.prepare("DELETE FROM pack_tags WHERE pack_id = ?").bind(id),
        this.env.ROSTER_HUB_DB.prepare("DELETE FROM packs WHERE id = ?").bind(id),
      ]);
      return true;
    });
  }

  async reconcileWriteD1(
    id: string,
    expectedLifecycle: string,
    writePack: boolean,
    writeTags: boolean,
  ): Promise<{ upserted: boolean; tags_replaced: boolean }> {
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.loadPacks();
      const current = await this.ctx.storage.get<Pack>(this.packKey(id));
      if (
        !current ||
        current.status !== "published" ||
        current.created_at !== expectedLifecycle ||
        !this.env.ROSTER_HUB_DB
      ) return { upserted: false, tags_replaced: false };
      if (writePack) await this.upsertD1PackRow(current);
      if (writeTags) await this.replaceD1Tags(current);
      return { upserted: writePack, tags_replaced: writeTags };
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

  private async hydrateDetailsByAuthor(authorId: string): Promise<void> {
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
        if (detail?.id === id && detail.author_id === authorId) {
          await this.ctx.storage.put(this.packKey(id), detail);
        }
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
    await this.mirrorChangedBestEffort(pack);
    if (field === "vote_count") await this.mirrorD1VoteCount(pack.id, pack.vote_count);
    return pack;
  }

  private async mirrorChangedBestEffort(pack: Pack): Promise<void> {
    try {
      await this.mirror(await this.getStoredPacks(), pack);
      await this.ctx.storage.delete(`${DIRTY_MIRROR_PREFIX}${pack.id}`);
    } catch (error) {
      console.error(`KV pack mirror deferred [${pack.id}]:`, error);
      await this.ctx.storage.put(`${DIRTY_MIRROR_PREFIX}${pack.id}`, pack.created_at);
      await this.scheduleRetry();
    }
  }

  private lifecycleMatches(pack: Pack | undefined, expected?: string | Pack | null): boolean {
    if (!pack || pack.status !== "published") return false;
    if (!expected) return true;
    const createdAt = typeof expected === "string" ? expected : expected.created_at;
    return pack.created_at === createdAt;
  }

  private restoreJobKey(jobId: string): string {
    return `${RESTORE_JOB_PREFIX}${jobId}`;
  }

  private publicRestoreState(job: RestoreJobRecord): RestoreJobState {
    return {
      jobId: job.jobId,
      backupKey: job.backupKey,
      snapshotCreatedAt: job.snapshotCreatedAt,
      snapshotFingerprint: job.snapshotFingerprint,
      total: job.total,
      nextCursor: job.nextCursor,
      status: job.status,
      expiresAt: job.expiresAt,
      ...(job.inFlight ? { inFlight: job.inFlight } : {}),
    };
  }

  private async findRestoreJobByHash(
    tokenHash: string,
  ): Promise<{ key: string; job: RestoreJobRecord } | null> {
    let startAfter: string | undefined;
    while (true) {
      const jobs = await this.ctx.storage.list<RestoreJobRecord>({
        prefix: RESTORE_JOB_PREFIX,
        startAfter,
        limit: 1000,
      });
      for (const [key, job] of jobs) {
        if (job.tokenHash === tokenHash) return { key, job };
      }
      if (jobs.size < 1000) return null;
      startAfter = [...jobs.keys()].at(-1);
    }
  }

  private async deleteActiveRestoreIf(jobId: string): Promise<void> {
    const active = await this.ctx.storage.get<RestoreActiveJob>(RESTORE_ACTIVE_KEY);
    if (active?.jobId === jobId) await this.ctx.storage.delete(RESTORE_ACTIVE_KEY);
  }

  private deletedAuthorExpiresAt(marker: DeletedAuthorStored): number {
    if (
      typeof marker === "object" &&
      marker !== null &&
      "expiresAt" in marker &&
      typeof marker.expiresAt === "number"
    ) {
      return marker.expiresAt;
    }
    return Number.POSITIVE_INFINITY;
  }

  private async getDeletedAuthorIds(now = Date.now()): Promise<Set<string>> {
    const deleted = new Set<string>();
    let startAfter: string | undefined;
    while (true) {
      const entries = await this.ctx.storage.list<DeletedAuthorStored>({
        prefix: DELETED_AUTHOR_PREFIX,
        startAfter,
        limit: 1000,
      });
      for (const [key, marker] of entries) {
        const expiresAt = this.deletedAuthorExpiresAt(marker);
        if (expiresAt <= now) {
          await this.ctx.storage.delete(key);
          continue;
        }
        const userId =
          typeof marker === "object" &&
          marker !== null &&
          "userId" in marker &&
          typeof marker.userId === "string"
            ? marker.userId
            : key.slice(DELETED_AUTHOR_PREFIX.length);
        deleted.add(userId);
        if (Number.isFinite(expiresAt)) await this.scheduleAlarmAt(expiresAt);
      }
      if (entries.size < 1000) break;
      startAfter = [...entries.keys()].at(-1);
    }
    return deleted;
  }

  private async scheduleAlarmAt(timestamp: number): Promise<void> {
    if (!Number.isFinite(timestamp)) return;
    const current = await this.ctx.storage.getAlarm();
    if (current === null || current > timestamp) await this.ctx.storage.setAlarm(timestamp);
  }

  private async applyReplacement(packs: Pack[], forceIndex: boolean): Promise<void> {
    const current = await this.getStoredPacks();
    const deletedAuthors = await this.getDeletedAuthorIds();
    const accepted: Pack[] = [];
    for (const pack of packs) {
      const pending = await this.getPending(pack.id);
      if (pending?.kind !== "delete" && !deletedAuthors.has(String(pack.author_id))) {
        accepted.push(pack);
      }
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
      if (error instanceof Error && error.message.includes("no such table")) return true;
      await recordD1MirrorFailure(this.env, "delete", id, error);
      return false;
    }
  }

  private async deleteStoredPack(id: string): Promise<void> {
    await this.deleteInstallClaims(id);
    await this.ctx.storage.delete(this.packKey(id));
    await this.ctx.storage.put(this.tombstoneKey(id), new Date().toISOString());
    this.forgetVotes(id);
  }

  private async mirrorD1Pack(pack: Pack): Promise<void> {
    if (!this.env.ROSTER_HUB_DB) return;
    try {
      if (pack.status === "published") {
        await this.upsertD1PackRow(pack);
        await this.replaceD1Tags(pack);
      } else {
        await this.env.ROSTER_HUB_DB.batch([
          this.env.ROSTER_HUB_DB.prepare("DELETE FROM pack_tags WHERE pack_id = ?").bind(pack.id),
          this.env.ROSTER_HUB_DB.prepare("DELETE FROM packs WHERE id = ?").bind(pack.id),
        ]);
      }
    } catch (error) {
      console.error(`D1 sync failed [${pack.id}]:`, error);
      await recordD1MirrorFailure(this.env, pack.status === "published" ? "upsert" : "delete", pack.id, error);
    }
  }

  private async mirrorD1VoteCount(id: string, voteCount: number): Promise<void> {
    if (!this.env.ROSTER_HUB_DB) return;
    try {
      await this.env.ROSTER_HUB_DB.prepare("UPDATE packs SET vote_count = ? WHERE id = ?")
        .bind(voteCount, id)
        .run();
    } catch (error) {
      console.error(`D1 vote_count sync failed [${id}]:`, error);
      await recordD1MirrorFailure(this.env, "vote-count", id, error);
    }
  }

  private async upsertD1PackRow(pack: Pack): Promise<void> {
    const row = toD1PackRow(pack);
    await this.env.ROSTER_HUB_DB!.prepare(
      `INSERT INTO packs (id, author_id, author_name, is_anonymous, title, description, pack_type, addons, vote_count, created_at, updated_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
       ON CONFLICT(id) DO UPDATE SET author_id = excluded.author_id, author_name = excluded.author_name,
         is_anonymous = excluded.is_anonymous, title = excluded.title, description = excluded.description,
         pack_type = excluded.pack_type, addons = excluded.addons, vote_count = excluded.vote_count,
         updated_at = datetime('now')`,
    ).bind(row.id, row.author_id, row.author_name, row.is_anonymous, row.title, row.description,
      row.pack_type, row.addons, row.vote_count).run();
  }

  private async replaceD1Tags(pack: Pack): Promise<void> {
    await this.env.ROSTER_HUB_DB!.batch([
      this.env.ROSTER_HUB_DB!.prepare("DELETE FROM pack_tags WHERE pack_id = ?").bind(pack.id),
      ...pack.tags.map((tag) => this.env.ROSTER_HUB_DB!.prepare(
        "INSERT OR IGNORE INTO pack_tags (pack_id, tag) VALUES (?, ?)",
      ).bind(pack.id, tag)),
    ]);
  }

  private forgetVotes(packId: string): void {
    const prefix = `${packId}:`;
    for (const key of this.voteMemo.keys()) {
      if (key.startsWith(prefix)) this.voteMemo.delete(key);
    }
  }

  private async deleteInstallClaims(packId: string): Promise<void> {
    let startAfter: string | undefined;
    while (true) {
      const markers = await this.ctx.storage.list<InstallMarker>({
        prefix: `${INSTALL_MARKER_PREFIX}${packId}:`,
        startAfter,
        limit: 1000,
      });
      const keys = new Set<string>();
      for (const [markerKey, marker] of markers) {
        keys.add(markerKey);
        if (marker.slotKey.startsWith(INSTALL_SLOT_PREFIX)) keys.add(marker.slotKey);
      }
      const deletions = [...keys];
      for (let offset = 0; offset < deletions.length; offset += INSTALL_DELETE_BATCH_SIZE) {
        await this.ctx.storage.delete(deletions.slice(offset, offset + INSTALL_DELETE_BATCH_SIZE));
      }
      if (markers.size < 1000) break;
      startAfter = [...markers.keys()].at(-1);
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
    const canonical = await this.ctx.storage.get<Pack>(this.packKey(operation.packId));
    if (!canonical || canonical.created_at !== operation.lifecycle) {
      await this.ctx.storage.delete(this.operationKey(operation.id));
      if (await this.ctx.storage.get<string>(this.pendingKey(operation.packId)) === operation.id) {
        await this.ctx.storage.delete(this.pendingKey(operation.packId));
      }
      return false;
    }
    // An update may land while a create is waiting for its first KV mirror.
    // Resume from canonical state, never the create-time body captured by the
    // journal entry.
    operation.pack = canonical;
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
    // Install claims are durable per-pack state too. Remove them before the
    // delete journal can complete; deleteInstallClaims uses bounded batches so
    // a pack with a full ring cannot turn this into thousands of sequential
    // storage operations.
    try {
      await this.deleteInstallClaims(operation.packId);
    } catch (error) {
      console.error(`Install claim cleanup failed [${operation.packId}]:`, error);
      await this.saveOperation(operation);
      await this.scheduleRetry();
      return false;
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
      await this.upsertD1PackRow(pack);
      await this.replaceD1Tags(pack);
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
