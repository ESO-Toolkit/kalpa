import { env } from "cloudflare:workers";
import { runDurableObjectAlarm, runInDurableObject } from "cloudflare:test";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { handleCreateShare, handleResolveShare, resetTokenCache } from "../src/shares";
import { ShareStore } from "../src/share-store";
import type { Env, SharePackData, ShareRecord } from "../src/types";
import { authedRequest, esoLogsResponse, TEST_USER } from "./helpers";

const e = env as unknown as Env;
const BASE = "https://kalpa-pack-hub.eso-toolkit.workers.dev/shares";
const pack: SharePackData = {
  title: "Share", description: "", packType: "addon-pack", tags: [],
  addons: [{ esouiId: 1, name: "Addon", required: true }],
};
const index = () => e.PACK_INDEX.get(e.PACK_INDEX.idFromName("singleton"));

beforeEach(async () => {
  await runInDurableObject(index(), async (_instance, state) => {
    await state.storage.deleteAlarm();
    await state.storage.deleteAll();
  });
  for (const prefix of ["share:", "share-user:"]) {
    let cursor: string | undefined;
    do {
      const page = await e.ESO_PACKS.list({ prefix, cursor });
      await Promise.all(page.keys.map(({ name }) => e.ESO_PACKS.delete(name)));
      cursor = page.list_complete ? undefined : page.cursor;
    } while (cursor);
  }
});

afterEach(() => { vi.restoreAllMocks(); resetTokenCache(); });

function legacy(code: string, expiresAt: number): ShareRecord {
  return { code, pack, createdBy: String(TEST_USER.id), createdByName: TEST_USER.name,
    createdAt: new Date().toISOString(), expiresAt: new Date(expiresAt).toISOString() };
}

describe("durable share reservations", () => {
  it("allows only one of two concurrent requests for the final slot", async () => {
    const stub = index();
    for (let i = 0; i < 9; i++) expect((await stub.createShare(TEST_USER, pack)).status).toBe("ok");
    const results = await Promise.all([stub.createShare(TEST_USER, pack), stub.createShare(TEST_USER, pack)]);
    expect(results.map(({ status }) => status).sort()).toEqual(["limit", "ok"]);
    expect((await e.ESO_PACKS.list({ prefix: `share-user:${TEST_USER.id}:` })).keys).toHaveLength(10);
  });

  it("keeps an acknowledged share and its slot after a partial KV write, then repairs it", async () => {
    const stub = index();
    const originalPut = e.ESO_PACKS.put.bind(e.ESO_PACKS);
    const put = vi.spyOn(e.ESO_PACKS, "put").mockImplementation((key, value, options) => {
      if (key.startsWith("share-user:")) return Promise.reject(new Error("tracking unavailable"));
      return originalPut(key, value, options);
    });
    const result = await stub.createShare(TEST_USER, pack);
    expect(result.status).toBe("ok");
    if (result.status !== "ok") throw new Error("share creation failed");
    expect(await stub.getShare(result.record.code)).toEqual(result.record);
    expect(await e.ESO_PACKS.get(`share-user:${TEST_USER.id}:${result.record.code}`)).toBeNull();
    put.mockRestore();
    for (let i = 0; i < 9; i++) expect((await stub.createShare(TEST_USER, pack)).status).toBe("ok");
    expect((await stub.createShare(TEST_USER, pack)).status).toBe("limit");
    expect(await runDurableObjectAlarm(stub)).toBe(true);
    expect(await e.ESO_PACKS.get(`share-user:${TEST_USER.id}:${result.record.code}`)).toBe("1");
  });

  it("round-trips a valid HTTP payload larger than a single DO storage value", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(esoLogsResponse(TEST_USER));
    const large: SharePackData = { ...pack, addons: Array.from({ length: 200 }, (_, i) => ({
      esouiId: i + 1, name: "n".repeat(200), note: "x".repeat(500), required: true,
    })) };
    expect(new TextEncoder().encode(JSON.stringify(large)).byteLength).toBeGreaterThan(128 * 1024);
    const response = await handleCreateShare(authedRequest(BASE, { method: "POST", body: JSON.stringify(large) }), e);
    expect(response.status).toBe(201);
    expect(response.headers.get("Cache-Control")).toBe("private, no-store");
    const { code } = await response.json<{ code: string }>();
    expect((await index().getShare(code))?.pack).toEqual(large);
  });

  it("keeps deleted shares inaccessible during KV failures and retries cleanup", async () => {
    const stub = index();
    const result = await stub.createShare(TEST_USER, pack);
    if (result.status !== "ok") throw new Error("share creation failed");
    const remove = vi.spyOn(e.ESO_PACKS, "delete").mockRejectedValue(new Error("KV unavailable"));
    expect(await stub.deleteUserShares(String(TEST_USER.id))).toBe(1);
    expect(await stub.getShare(result.record.code)).toBeNull();
    await runInDurableObject(stub, async (_instance, state) => {
      expect((await state.storage.list({ prefix: `share-body:${result.record.code}:` })).size).toBe(0);
    });
    remove.mockRestore();
    expect(await runDurableObjectAlarm(stub)).toBe(true);
    expect(await e.ESO_PACKS.get(`share:${result.record.code}`)).toBeNull();
    const fresh = await stub.createShare(TEST_USER, pack);
    if (fresh.status !== "ok") throw new Error("share recreation failed");
    expect(await stub.getShare(fresh.record.code)).toEqual(fresh.record);
    expect(await stub.getShare(result.record.code)).toBeNull();
  });

  it("blocks an old share omitted by the legacy ownership listing after account deletion", async () => {
    await e.ESO_PACKS.put("share:ABCDEF", JSON.stringify(legacy("ABCDEF", Date.now() + 3600_000)));
    expect(await index().getShare("ABCDEF")).not.toBeNull();
    await index().deleteUserShares(String(TEST_USER.id));
    expect(await index().getShare("ABCDEF")).toBeNull();
  });

  it("resumes account cleanup after the legacy ownership lookup fails", async () => {
    await e.ESO_PACKS.put("share:ABCDEF", JSON.stringify(legacy("ABCDEF", Date.now() + 3600_000)));
    await e.ESO_PACKS.put(`share-user:${TEST_USER.id}:ABCDEF`, "1");
    const list = vi.spyOn(e.ESO_PACKS, "list").mockRejectedValue(new Error("listing unavailable"));
    await runInDurableObject(index(), async (_instance, state) => {
      await expect(new ShareStore(state.storage, e).deleteUser(String(TEST_USER.id)))
        .rejects.toThrow("listing unavailable");
    });
    expect(await index().getShare("ABCDEF")).toBeNull();
    list.mockRestore();
    expect(await runDurableObjectAlarm(index())).toBe(true);
    expect(await e.ESO_PACKS.get("share:ABCDEF")).toBeNull();
    expect((await index().createShare(TEST_USER, pack)).status).toBe("ok");
  });

  it("reclaims expired durable bodies and reservations", async () => {
    const stub = index();
    const result = await stub.createShare(TEST_USER, pack);
    if (result.status !== "ok") throw new Error("share creation failed");
    const code = result.record.code;
    await runInDurableObject(stub, async (_instance, state) => {
      const stored = await state.storage.get<{ code: string; expiresAt: number; record: Omit<ShareRecord, "pack">; bodyChunks: number }>(`share-state:${code}`);
      if (!stored) throw new Error("share missing");
      await state.storage.delete(`share-expiry:${String(stored.expiresAt).padStart(13, "0")}:${code}`);
      stored.expiresAt = Date.now() - 1000;
      stored.record.expiresAt = new Date(stored.expiresAt).toISOString();
      await state.storage.put(`share-state:${code}`, stored);
      await state.storage.put(`share-owner:${TEST_USER.id}`, { [code]: stored.expiresAt });
      await state.storage.put(`share-expiry:${String(stored.expiresAt).padStart(13, "0")}:${code}`, code);
    });
    expect(await stub.getShare(code)).toBeNull();
    expect(await runDurableObjectAlarm(stub)).toBe(true);
    await runInDurableObject(stub, async (_instance, state) => {
      expect(await state.storage.get(`share-state:${code}`)).toBeUndefined();
      expect((await state.storage.list({ prefix: `share-body:${code}:` })).size).toBe(0);
    });
    expect(await e.ESO_PACKS.get(`share:${code}`)).toBeNull();
    expect((await stub.createShare(TEST_USER, pack)).status).toBe("ok");
  });
});

describe("share expiry and response caching", () => {
  it.each(["expired", "invalid"])("rejects an %s expiry even if KV still holds the record", async (kind) => {
    const record = legacy("ABCDEF", Date.now() - 1000);
    if (kind === "invalid") record.expiresAt = "not a date";
    await e.ESO_PACKS.put("share:ABCDEF", JSON.stringify(record));
    const response = await handleResolveShare(new Request(`${BASE}/ABCDEF`), e, "ABCDEF");
    expect(response.status).toBe(404);
    expect(response.headers.get("Cache-Control")).toBe("private, no-store");
  });

  it("caps cache lifetime at the remaining share lifetime", async () => {
    await e.ESO_PACKS.put("share:ABCDEF", JSON.stringify(legacy("ABCDEF", Date.now() + 10_000)));
    const response = await handleResolveShare(new Request(`${BASE}/ABCDEF`), e, "ABCDEF");
    expect(response.status).toBe(200);
    const ttl = Number(response.headers.get("Cache-Control")?.match(/max-age=(\d+)/)?.[1]);
    expect(ttl).toBeGreaterThan(0);
    expect(ttl).toBeLessThanOrEqual(10);
  });

  it("marks authentication failures private and non-cacheable", async () => {
    const response = await handleCreateShare(new Request(BASE, { method: "POST" }), e);
    expect(response.status).toBe(401);
    expect(response.headers.get("Cache-Control")).toBe("private, no-store");
  });
});
