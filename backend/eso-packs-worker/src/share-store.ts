import type { Env, SharePackData, ShareRecord } from "./types";

export const MAX_SHARES_PER_USER = 10;
const TTL_MS = 7 * 86400 * 1000;
const RETRY_MS = 30_000;
const BATCH_SIZE = 10;
const ALPHABET = "23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const STATE = "share-state:";
const BODY = "share-body:";
const OWNER = "share-owner:";
const DIRTY = "share-dirty:";
const EXPIRY = "share-expiry:";
const DELETED_USER = "share-deleted-user:";
const PENDING_DELETE = "share-pending-delete:";

interface ShareState {
  code: string;
  userId: string;
  expiresAt: number;
  record: Omit<ShareRecord, "pack"> | null;
  bodyChunks: number;
  deletionGeneration?: string;
}

type Ledger = Record<string, number>;
type CreateResult = { status: "ok"; record: ShareRecord } | { status: "limit" | "collision" };

function generateCode(): string {
  let code = "";
  while (code.length < 6) {
    for (const byte of crypto.getRandomValues(new Uint8Array(12))) {
      if (byte < 240) code += ALPHABET[byte % ALPHABET.length];
      if (code.length === 6) break;
    }
  }
  return code;
}

function expiryKey(state: ShareState): string {
  return `${EXPIRY}${String(state.expiresAt).padStart(13, "0")}:${state.code}`;
}

/** All calls run inside PackIndexDO's serialization boundary. KV is a mirror;
 * reservations, deletion, and expiry remain correct when either KV write fails. */
export class ShareStore {
  constructor(private storage: DurableObjectStorage, private env: Env) {}

  async create(user: { id: number; name: string }, pack: SharePackData): Promise<CreateResult> {
    const userId = String(user.id);
    if (await this.storage.get(`${PENDING_DELETE}${userId}`)) await this.finishDeleteUser(userId);
    const ledger = await this.ledger(userId);
    if (Object.keys(ledger).length >= MAX_SHARES_PER_USER) return { status: "limit" };
    let code = "";
    for (let attempt = 0; attempt < 3; attempt++) {
      const candidate = generateCode();
      if (!await this.storage.get(`${STATE}${candidate}`) && !await this.env.ESO_PACKS.get(`share:${candidate}`)) {
        code = candidate;
        break;
      }
    }
    if (!code) return { status: "collision" };

    const now = Date.now();
    const metadata = {
      code, createdBy: userId, createdByName: user.name,
      createdAt: new Date(now).toISOString(), expiresAt: new Date(now + TTL_MS).toISOString(),
    };
    // A valid HTTP payload can exceed a DO value's 128 KiB limit. Keep the
    // JSON in small string chunks rather than narrowing the existing API.
    const serialized = JSON.stringify(pack);
    const chunks = serialized.match(/[\s\S]{1,20000}/g)!;
    const deletionGeneration = await this.storage.get<string>(`${DELETED_USER}${userId}`);
    const state: ShareState = { code, userId, expiresAt: now + TTL_MS, record: metadata, bodyChunks: chunks.length, deletionGeneration };
    ledger[code] = state.expiresAt;
    await this.storage.transaction(async (txn) => {
      await txn.put(`${OWNER}${userId}`, ledger);
      await txn.put(`${STATE}${code}`, state);
      for (let i = 0; i < chunks.length; i++) await txn.put(`${BODY}${code}:${i}`, chunks[i]);
      await txn.put(`${DIRTY}${code}`, true);
      await txn.put(expiryKey(state), code);
      await this.arm(txn, now + RETRY_MS);
    });
    await this.mirror(state);
    return { status: "ok", record: { ...metadata, pack } };
  }

  async get(code: string): Promise<ShareRecord | null> {
    const state = await this.storage.get<ShareState>(`${STATE}${code}`);
    if (state) {
      const generation = await this.storage.get<string>(`${DELETED_USER}${state.userId}`);
      return state.record && state.expiresAt > Date.now() && state.deletionGeneration === generation ? this.record(state) : null;
    }
    const legacy = await this.env.ESO_PACKS.get<ShareRecord>(`share:${code}`, "json");
    if (!legacy || Date.parse(legacy.expiresAt) <= Date.now() || !Number.isFinite(Date.parse(legacy.expiresAt))) return null;
    // A legacy KV listing can omit recent writes. Keep a deletion marker so
    // such records cannot become visible again after account deletion.
    return await this.storage.get(`${DELETED_USER}${legacy.createdBy}`) ? null : legacy;
  }

  async deleteUser(userId: string): Promise<number> {
    await this.storage.transaction(async (txn) => {
      await txn.put(`${DELETED_USER}${userId}`, crypto.randomUUID());
      await txn.put(`${PENDING_DELETE}${userId}`, true);
      await this.arm(txn, Date.now() + RETRY_MS);
    });
    return this.finishDeleteUser(userId);
  }

  private async finishDeleteUser(userId: string): Promise<number> {
    const ledger = await this.ledger(userId);
    const erased: ShareState[] = [];
    for (const [code, expiresAt] of Object.entries(ledger)) {
      const state = await this.storage.get<ShareState>(`${STATE}${code}`) ?? {
        code, userId, expiresAt, record: null, bodyChunks: 0,
      };
      await this.erase(state);
      erased.push({ ...state, record: null, bodyChunks: 0 });
    }
    await this.storage.transaction(async (txn) => {
      await txn.put(`${OWNER}${userId}`, {});
      await txn.delete(`${PENDING_DELETE}${userId}`);
    });
    for (const state of erased) await this.mirror(state);
    return Object.keys(ledger).length;
  }

  async retry(): Promise<void> {
    const pending = await this.storage.list({ prefix: PENDING_DELETE, limit: BATCH_SIZE });
    for (const key of pending.keys()) {
      try {
        await this.finishDeleteUser(key.slice(PENDING_DELETE.length));
      } catch (error) {
        console.error("Share account cleanup failed; retained for retry", error);
      }
    }
    const due = await this.storage.list<string>({
      prefix: EXPIRY, end: `${EXPIRY}${String(Date.now()).padStart(13, "0")}:\uffff`, limit: BATCH_SIZE,
    });
    for (const [key, code] of due) {
      const state = await this.storage.get<ShareState>(`${STATE}${code}`);
      if (state) {
        await this.erase(state);
        await this.mirror({ ...state, record: null, bodyChunks: 0 });
      } else await this.storage.delete(key);
    }
    const dirty = await this.storage.list({ prefix: DIRTY, limit: BATCH_SIZE });
    for (const key of dirty.keys()) {
      const state = await this.storage.get<ShareState>(`${STATE}${key.slice(DIRTY.length)}`);
      if (state) await this.mirror(state);
      else await this.storage.delete(key);
    }
    const next = await this.storage.list<string>({ prefix: EXPIRY, limit: 1 });
    if (next.size) await this.arm(this.storage, Math.max(Date.now() + RETRY_MS, Number([...next.keys()][0].slice(EXPIRY.length).split(":")[0])));
    if ((await this.storage.list({ prefix: DIRTY, limit: 1 })).size) await this.arm(this.storage, Date.now() + RETRY_MS);
    if ((await this.storage.list({ prefix: PENDING_DELETE, limit: 1 })).size) await this.arm(this.storage, Date.now() + RETRY_MS);
  }

  private async ledger(userId: string): Promise<Ledger> {
    let ledger = await this.storage.get<Ledger>(`${OWNER}${userId}`);
    if (!ledger) {
      ledger = {};
      // One-time compatibility import. Subsequent reservations never depend on
      // eventually consistent KV listing. Expired legacy entries do not count.
      let cursor: string | undefined;
      do {
        const page = await this.env.ESO_PACKS.list({ prefix: `share-user:${userId}:`, cursor });
        for (const key of page.keys) {
          const code = key.name.slice(`share-user:${userId}:`.length);
          const state = await this.storage.get<ShareState>(`${STATE}${code}`);
          if (state && !state.record) continue;
          const record = await this.env.ESO_PACKS.get<ShareRecord>(`share:${code}`, "json");
          const expiresAt = state?.expiresAt ?? (key.expiration ? key.expiration * 1000 : Date.parse(record?.expiresAt ?? ""));
          if (expiresAt > Date.now()) ledger[code] = expiresAt;
        }
        cursor = page.list_complete ? undefined : page.cursor;
      } while (cursor);
      await this.storage.put(`${OWNER}${userId}`, ledger);
    }
    return Object.fromEntries(Object.entries(ledger).filter(([, expiresAt]) => expiresAt > Date.now()));
  }

  private async record(state: ShareState): Promise<ShareRecord> {
    let serialized = "";
    for (let i = 0; i < state.bodyChunks; i++) {
      const part = await this.storage.get<string>(`${BODY}${state.code}:${i}`);
      if (part === undefined) throw new Error("Missing durable share body");
      serialized += part;
    }
    return { ...state.record!, pack: JSON.parse(serialized) as SharePackData };
  }

  private async erase(state: ShareState): Promise<void> {
    await this.storage.transaction(async (txn) => {
      await txn.put(`${STATE}${state.code}`, { ...state, record: null, bodyChunks: 0 });
      for (let i = 0; i < state.bodyChunks; i++) await txn.delete(`${BODY}${state.code}:${i}`);
      await txn.put(`${DIRTY}${state.code}`, true);
      await txn.put(expiryKey(state), state.code);
      await this.arm(txn, Date.now() + RETRY_MS);
    });
  }

  private async mirror(state: ShareState): Promise<void> {
    try {
      if (state.record && state.deletionGeneration !== await this.storage.get(`${DELETED_USER}${state.userId}`)) {
        await this.erase(state);
        state = { ...state, record: null, bodyChunks: 0 };
      }
      // KV requires expirations at least 60 seconds away. The DO can still
      // serve the last minute, without extending the original seven-day TTL.
      if (state.record && state.expiresAt > Date.now() + 60_000) {
        const record = await this.record(state);
        const options = { expiration: Math.ceil(state.expiresAt / 1000) };
        await this.env.ESO_PACKS.put(`share:${state.code}`, JSON.stringify(record), options);
        await this.env.ESO_PACKS.put(`share-user:${state.userId}:${state.code}`, "1", options);
      } else {
        await this.env.ESO_PACKS.delete(`share:${state.code}`);
        await this.env.ESO_PACKS.delete(`share-user:${state.userId}:${state.code}`);
      }
      await this.storage.transaction(async (txn) => {
        await txn.delete(`${DIRTY}${state.code}`);
        if (state.expiresAt <= Date.now()) {
          await txn.delete(`${STATE}${state.code}`);
          await txn.delete(expiryKey(state));
          for (let i = 0; i < state.bodyChunks; i++) await txn.delete(`${BODY}${state.code}:${i}`);
          const existing = await txn.get<Ledger>(`${OWNER}${state.userId}`) ?? {};
          const ledger = Object.fromEntries(Object.entries(existing).filter(([, expiry]) => expiry > Date.now()));
          delete ledger[state.code];
          if (Object.keys(ledger).length) await txn.put(`${OWNER}${state.userId}`, ledger);
          else await txn.delete(`${OWNER}${state.userId}`);
        }
      });
    } catch (error) {
      console.error("Share mirror failed; durable state retained for retry", error);
      await this.arm(this.storage, Date.now() + RETRY_MS);
    }
  }

  private async arm(storage: DurableObjectStorage | DurableObjectTransaction, desired: number): Promise<void> {
    const current = await storage.getAlarm();
    if (current === null || current > desired) await storage.setAlarm(desired);
  }
}
