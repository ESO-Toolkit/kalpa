import { env } from "cloudflare:workers";
import { runDurableObjectAlarm, runInDurableObject } from "cloudflare:test";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Env, Pack, VoteRecord } from "../src/types";
import { makePack } from "./helpers";

const e = env as unknown as Env;
const index = () => e.PACK_INDEX.get(e.PACK_INDEX.idFromName("singleton"));
const snapshot = (packs: Pack[], votes: Record<string, VoteRecord> = {}) => ({
  created_at: "2025-02-01T00:00:00.000Z", packs,
  packBodies: Object.fromEntries(packs.map((pack) => [pack.id, pack])), votes,
});

beforeEach(async () => {
  await runInDurableObject(index(), async (_instance, state) => {
    await state.storage.deleteAlarm();
    await state.storage.deleteAll();
  });
  await index().setAuthority("do", []);
  await e.ESO_PACKS.delete("backup:latest");
});
afterEach(() => vi.restoreAllMocks());

describe("serialized backup privacy cleanup", () => {
  it("removes every deleted user when a later cleanup reads a stale snapshot", async () => {
    const a = makePack("privacy-a", { author_id: "privacy-a" });
    const b = makePack("privacy-b", { author_id: "privacy-b" });
    const keep = makePack("privacy-keep", { author_id: "privacy-keep" });
    const stale = JSON.stringify(snapshot([a, b, keep]));
    await e.ESO_PACKS.put("backup:latest", stale);
    await index().removePacksByAuthor(a.author_id);
    await index().purgeDeletedUsersFromLatestBackup();
    // Model a stale KV read by putting back the original pre-deletion value.
    await e.ESO_PACKS.put("backup:latest", stale);
    await index().removePacksByAuthor(b.author_id);
    await index().purgeDeletedUsersFromLatestBackup();
    expect(await e.ESO_PACKS.get("backup:latest", "json")).toEqual(snapshot([keep]));
  });

  it("retains failed cleanup durably and retries it from an alarm", async () => {
    const pack = makePack("privacy-retry", { author_id: "privacy-retry" });
    await e.ESO_PACKS.put("backup:latest", JSON.stringify(snapshot([pack])));
    await index().removePacksByAuthor(pack.author_id);
    const original = e.ESO_PACKS.put.bind(e.ESO_PACKS);
    const put = vi.spyOn(e.ESO_PACKS, "put").mockImplementation((key, value, options) => {
      if (key === "backup:latest") return Promise.reject(new Error("backup unavailable"));
      return original(key, value, options);
    });
    await index().purgeDeletedUsersFromLatestBackup();
    await runInDurableObject(index(), async (_instance, state) => {
      expect(await state.storage.get("meta:backup-purge-pending")).toBe(true);
    });
    put.mockRestore();
    expect(await runDurableObjectAlarm(index())).toBe(true);
    expect(await e.ESO_PACKS.get("backup:latest", "json")).toEqual(snapshot([]));
  });

  it("preserves new content while preventing a returning user's old data from being restored", async () => {
    const old = makePack("privacy-old", { author_id: "privacy-return" });
    const keep = makePack("privacy-other", { author_id: "privacy-other", vote_count: 1 });
    const vote = { packId: keep.id, userId: old.author_id, votedAt: old.created_at };
    await index().removePacksByAuthor(old.author_id);
    const fresh = makePack("privacy-new", { author_id: old.author_id, created_at: new Date(Date.now() + 1000).toISOString() });
    expect((await index().addPack(fresh)).ok).toBe(true);
    await index().stageRestoredPack(old);
    expect(await e.ESO_PACKS.get(`pack:${old.id}`)).toBeNull();
    await index().replaceIndexPreserving({ packs: [old, keep] }, [old.id, keep.id], [vote]);
    expect(await index().getPack(old.id)).toBeNull();
    expect(await index().getPack(fresh.id)).toMatchObject({ id: fresh.id });
    expect(await index().getPack(keep.id)).toMatchObject({ vote_count: 0 });
    expect(await index().getVotedPackIds(old.author_id, [keep.id])).toEqual(new Set());
    await index().writeBackup("backup:privacy-return", snapshot([old, fresh, keep], { [`${keep.id}:${old.author_id}`]: vote }));
    const saved = await e.ESO_PACKS.get<ReturnType<typeof snapshot>>("backup:privacy-return", "json");
    expect(saved!.packs.map(({ id }) => id).sort()).toEqual([fresh.id, keep.id].sort());
    expect(saved!.votes).toEqual({});
  });

  it("rejects deleted records from restore pages, stale KV indexes, and witness adoption", async () => {
    const old = makePack("privacy-legacy", { author_id: "privacy-legacy" });
    await index().removePacksByAuthor(old.author_id);
    await index().stageRestoredPack(old);
    expect(await e.ESO_PACKS.get(`pack:${old.id}`)).toBeNull();

    // A legacy mirror can appear after the deletion's KV scan has finished.
    await e.ESO_PACKS.put(`pack:${old.id}`, JSON.stringify(old));
    await e.ESO_PACKS.put("index:packs", JSON.stringify({ packs: [old] }));
    await index().setAuthority("kv", []);
    expect((await index().getIndex()).packs.some(({ id }) => id === old.id)).toBe(false);
    expect(await index().getPack(old.id)).toBeNull();
    expect(await index().adoptWitnesses([old.id])).toMatchObject({ adopted: [], tombstoned: [old.id] });
  });

  it("does not rewrite the bytes or timestamp of an unaffected backup", async () => {
    const raw = JSON.stringify(snapshot([makePack("privacy-unaffected", { author_id: "privacy-unaffected" })]), null, 2);
    await e.ESO_PACKS.put("backup:latest", raw);
    await index().removePacksByAuthor("privacy-absent");
    const put = vi.spyOn(e.ESO_PACKS, "put");
    await index().purgeDeletedUsersFromLatestBackup();
    expect(put.mock.calls.filter(([key]) => key === "backup:latest")).toHaveLength(0);
    expect(await e.ESO_PACKS.get("backup:latest")).toBe(raw);
  });
});
