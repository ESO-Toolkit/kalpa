import { env } from "cloudflare:workers";
import { runInDurableObject, runDurableObjectAlarm } from "cloudflare:test";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { putPack, putVote } from "../src/kv";
import type { Env, Pack, VoteRecord } from "../src/types";
import { makePack } from "./helpers";

const e = env as unknown as Env;

function packIndex() {
  return e.PACK_INDEX.get(e.PACK_INDEX.idFromName("singleton"));
}

function bytesToBase64Url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

async function restoreTokenHash(token: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(token));
  return bytesToBase64Url(new Uint8Array(digest));
}

interface BackupForTest {
  packs: Pack[];
  packBodies: Record<string, Pack>;
  votes: Record<string, VoteRecord>;
  created_at: string;
}

describe("PackIndexDO authoritative mutations", () => {
  beforeEach(async () => {
    const index = packIndex();
    await index.cancelActiveRestoreJob();
    await runInDurableObject(index, async (_instance, state) => {
      const restoreKeys = [...(await state.storage.list({ prefix: "restore:" })).keys()];
      const deletedAuthorKeys = [...(await state.storage.list({ prefix: "deleted-author:" })).keys()];
      await Promise.all([...restoreKeys, ...deletedAuthorKeys].map((key) => state.storage.delete(key)));
    });
    await index.setAuthority("kv", []);
    await index.replaceIndex({ packs: [] });
  });

  it("accepts only one concurrent create for the same id", async () => {
    const pack = makePack("w1-duplicate-create");

    const results = await Promise.all([
      packIndex().addPack(pack, 25),
      packIndex().addPack({ ...pack }, 25),
    ]);

    expect(results.filter((result) => result.ok)).toHaveLength(1);
    expect(results.find((result) => !result.ok)).toMatchObject({ reason: "duplicate" });
    expect((await packIndex().getIndex()).packs.filter(({ id }) => id === pack.id)).toHaveLength(1);
  });

  it("does not overwrite an omitted pre-deploy pack during shadow mutation", async () => {
    const visible = makePack("w1-visible-shadow");
    const delayed = makePack("w1-omitted-shadow");
    await e.ESO_PACKS.put("index:packs", JSON.stringify({ packs: [visible] }));
    await putPack(e, delayed);

    const duplicate = await packIndex().addPack({ ...delayed, title: "Collision" }, 25);
    expect(duplicate).toMatchObject({ ok: false, reason: "duplicate" });
    expect(await e.ESO_PACKS.get<Pack>(`pack:${delayed.id}`, "json"))
      .toMatchObject({ title: delayed.title });
    expect((await e.ESO_PACKS.get<{ packs: Pack[] }>("index:packs", "json"))!.packs)
      .toEqual([visible]);

    await e.ESO_PACKS.put("index:packs", JSON.stringify({ packs: [visible, delayed] }));
    expect((await packIndex().getIndex()).packs).toEqual(
      expect.arrayContaining([expect.objectContaining({ id: delayed.id })]),
    );
  });

  it("refreshes an unowned stale shadow record from the newer detail body", async () => {
    const stale = makePack("w1-shadow-version", {
      title: "Stale index title",
      updated_at: "2026-08-26T00:00:00.000Z",
    });
    const fresh = {
      ...stale,
      title: "Fresh detail title",
      updated_at: "2026-08-26T01:00:00.000Z",
    };
    await e.ESO_PACKS.put("index:packs", JSON.stringify({ packs: [stale] }));
    await putPack(e, fresh);

    expect(await packIndex().getPack(stale.id)).toMatchObject({
      title: "Fresh detail title",
      updated_at: fresh.updated_at,
    });
  });

  it("resumes a create after its first KV detail mirror fails", async () => {
    const pack = makePack("w1-create-retry");
    const put = vi.spyOn(e.ESO_PACKS, "put");
    put.mockRejectedValueOnce(new Error("injected detail put failure"));

    expect(await packIndex().addPack(pack, 25)).toMatchObject({
      ok: false,
      reason: "retry",
    });
    put.mockRestore();

    expect(await packIndex().addPack({ ...pack }, 25)).toMatchObject({ ok: true });
    expect(await e.ESO_PACKS.get<Pack>(`pack:${pack.id}`, "json")).toMatchObject({
      created_at: pack.created_at,
    });
    expect((await packIndex().getIndex()).packs.filter(({ id }) => id === pack.id)).toHaveLength(1);
  });

  it("retries a pending create from the latest canonical body", async () => {
    const pack = makePack("w1-create-retry-latest");
    const put = vi.spyOn(e.ESO_PACKS, "put").mockRejectedValueOnce(
      new Error("injected initial detail put failure"),
    );
    expect(await packIndex().addPack(pack, 25)).toMatchObject({ reason: "retry" });
    put.mockRestore();

    const updated = {
      ...pack,
      title: "Updated while create pending",
      updated_at: "2026-08-27T01:00:00.000Z",
    };
    expect(await packIndex().updatePack(pack.id, updated, pack.author_id)).toMatchObject({
      status: "ok",
    });
    expect(await runDurableObjectAlarm(packIndex())).toBe(true);

    expect(await e.ESO_PACKS.get<Pack>(`pack:${pack.id}`, "json")).toMatchObject({
      title: updated.title,
      updated_at: updated.updated_at,
    });
  });

  it("resumes KV detail cleanup when delete is retried after mirror failure", async () => {
    const pack = makePack("w1-delete-retry");
    const index = packIndex();
    await index.replaceIndex({ packs: [pack] });
    await putPack(e, pack);
    const remove = vi.spyOn(e.ESO_PACKS, "delete");
    remove.mockRejectedValueOnce(new Error("injected detail delete failure"));

    expect(await index.removePack(pack.id, pack.author_id)).toBe("retry");
    remove.mockRestore();

    expect(await index.getPack(pack.id)).toBeNull();
    expect(await index.removePack(pack.id, pack.author_id)).toBe("ok");
    expect(await e.ESO_PACKS.get(`pack:${pack.id}`)).toBeNull();
  });

  it("rejects stale same-author update and delete operations after slug reuse", async () => {
    const oldPack = makePack("w1-same-author-reuse");
    const replacement = makePack(oldPack.id, {
      title: "Replacement",
      created_at: "2026-08-27T02:00:00.000Z",
      updated_at: "2026-08-27T02:00:00.000Z",
    });
    const index = packIndex();
    await index.replaceIndex({ packs: [oldPack] });
    await index.removePack(oldPack.id, oldPack.author_id, oldPack.created_at);
    await index.addPack(replacement);

    expect(await index.updatePack(oldPack.id, { ...oldPack, title: "Late update" }, oldPack.author_id))
      .toMatchObject({ status: "not-found" });
    expect(await index.removePack(oldPack.id, oldPack.author_id, oldPack.created_at))
      .toBe("not-found");
    expect(await index.getPack(oldPack.id)).toMatchObject({
      title: "Replacement",
      created_at: replacement.created_at,
    });
  });

  it.each(["vote_count", "install_count"] as const)(
    "commits a %s once and repairs a failed KV detail mirror by alarm",
    async (field) => {
      const pack = makePack(`w1-dirty-${field}`);
      const index = packIndex();
      await index.replaceIndex({ packs: [pack] });
      await putPack(e, pack);
      const originalPut = e.ESO_PACKS.put.bind(e.ESO_PACKS);
      const put = vi.spyOn(e.ESO_PACKS, "put").mockImplementation(async (key, value, options) => {
        if (key === `pack:${pack.id}`) throw new Error("injected detail mirror failure");
        return originalPut(key, value, options);
      });

      const result = field === "vote_count"
        ? (await index.toggleVote(pack.id, "dirty-voter", pack.created_at)).pack
        : await index.bumpPackCounter(pack.id, field, 1, pack.created_at);
      expect(result?.[field]).toBe(1);
      expect((await e.ESO_PACKS.get<Pack>(`pack:${pack.id}`, "json"))?.[field]).toBe(0);
      put.mockRestore();

      expect(await runDurableObjectAlarm(index)).toBe(true);
      expect((await e.ESO_PACKS.get<Pack>(`pack:${pack.id}`, "json"))?.[field]).toBe(1);
      expect((await index.getPack(pack.id))?.[field]).toBe(1);
    },
  );

  it("deletes only the target author's orphan detail records", async () => {
    const mine = makePack("w1-account-mine", { author_id: "account-target" });
    const unrelated = makePack("w1-unrelated-orphan", { author_id: "other-author" });
    const index = packIndex();
    await index.replaceIndex({ packs: [mine] });
    await putPack(e, mine);
    await putPack(e, unrelated);

    expect(await index.removePacksByAuthor(mine.author_id)).toEqual([mine.id]);
    expect(await index.getPack(unrelated.id)).toBeNull();
    expect(await e.ESO_PACKS.get<Pack>(`pack:${unrelated.id}`, "json"))
      .toMatchObject({ id: unrelated.id });
  });

  it("tombstones before resumable vote cleanup can partially fail", async () => {
    const pack = makePack("w1-vote-cleanup-retry", { vote_count: 3 });
    const index = packIndex();
    await index.replaceIndex({ packs: [pack] });
    await putPack(e, pack);
    for (const user of ["one", "two", "three"]) await putVote(e, pack.id, user);

    const originalDelete = e.ESO_PACKS.delete.bind(e.ESO_PACKS);
    let voteDeletes = 0;
    const remove = vi.spyOn(e.ESO_PACKS, "delete").mockImplementation(async (key) => {
      if (key.startsWith(`vote:${pack.id}:`) && ++voteDeletes === 2) {
        throw new Error("injected second vote delete failure");
      }
      return originalDelete(key);
    });

    expect(await index.removePack(pack.id, pack.author_id)).toBe("retry");
    expect(await index.getPack(pack.id)).toBeNull();
    expect((await index.getIndex()).packs.some(({ id }) => id === pack.id)).toBe(false);
    remove.mockRestore();

    expect(await index.removePack(pack.id, pack.author_id)).toBe("ok");
    for (const user of ["one", "two", "three"]) {
      expect(await e.ESO_PACKS.get(`vote:${pack.id}:${user}`)).toBeNull();
      expect(await e.ESO_PACKS.get(`user-votes:${user}:${pack.id}`)).toBeNull();
    }
  });

  it("does not resurrect a deleted pack when a vote carries a stale detail body", async () => {
    const pack = makePack("w1-delete-vote");
    await packIndex().replaceIndex({ packs: [pack] });
    await putPack(e, pack);
    const index = packIndex();

    await index.removePack(pack.id);
    const result = await index.toggleVote(pack.id, "stale-voter", pack);

    expect(result.pack).toBeNull();
    expect((await index.getIndex()).packs.some(({ id }) => id === pack.id)).toBe(false);
    expect(await e.ESO_PACKS.get(`pack:${pack.id}`)).toBeNull();
    expect(await e.ESO_PACKS.get(`vote:${pack.id}:stale-voter`)).toBeNull();
  });

  it("does not resurrect a deleted pack when an install carries a stale detail body", async () => {
    const pack = makePack("w1-delete-install");
    await packIndex().replaceIndex({ packs: [pack] });
    await putPack(e, pack);
    const index = packIndex();

    await index.removePack(pack.id);
    const result = await index.bumpPackCounter(pack.id, "install_count", 1, pack);

    expect(result).toBeNull();
    expect((await index.getIndex()).packs.some(({ id }) => id === pack.id)).toBe(false);
    expect(await e.ESO_PACKS.get(`pack:${pack.id}`)).toBeNull();
  });

  it("counts concurrent installs from one identity only once", async () => {
    const pack = makePack("w3-idempotent-install");
    const index = packIndex();
    await index.replaceIndex({ packs: [pack] });

    const results = await Promise.all([
      index.recordInstall(pack.id, "same-identity", pack.created_at, 1_000),
      index.recordInstall(pack.id, "same-identity", pack.created_at, 1_000),
    ]);

    expect(results.map((result) => result?.install_count)).toEqual([1, 1]);
    expect(await index.getPack(pack.id)).toMatchObject({ install_count: 1 });
  });

  it("allows the same install identity after the one-hour window", async () => {
    const pack = makePack("w3-install-window");
    const index = packIndex();
    await index.replaceIndex({ packs: [pack] });

    await index.recordInstall(pack.id, "repeat-identity", pack.created_at, 1_000);
    const result = await index.recordInstall(
      pack.id,
      "repeat-identity",
      pack.created_at,
      1_000 + 3_600_001,
    );

    expect(result).toMatchObject({ install_count: 2 });
  });

  it("expires persisted install identity data after one hour", async () => {
    const pack = makePack("w3-install-expiry");
    const index = packIndex();
    await index.replaceIndex({ packs: [pack] });
    await index.recordInstall(pack.id, "expiring-identity", pack.created_at, 1_000);

    expect(await index.cleanupInstallClaims(1_000 + 3_600_001)).toBe(1);
    const keys = await runInDurableObject(index, async (_instance, state) =>
      [...(await state.storage.list({ prefix: "install-" })).keys()]);
    expect(keys).toEqual([]);
  });

  it("expires install identities across durable-list page boundaries", async () => {
    const index = packIndex();
    await runInDurableObject(index, async (_instance, state) => {
      const entries: Record<string, { markerKey: string; recordedAt: number }> = {};
      for (let i = 0; i < 1_001; i++) {
        const slotKey = `install-slot:${String(i).padStart(4, "0")}`;
        entries[slotKey] = { markerKey: `missing-marker:${i}`, recordedAt: 1_000 };
      }
      await state.storage.put(entries);
    });

    expect(await index.cleanupInstallClaims(1_000 + 3_600_001)).toBe(1_001);
    const remaining = await runInDurableObject(index, async (_instance, state) =>
      state.storage.list({ prefix: "install-slot:" }));
    expect(remaining.size).toBe(0);
  });

  it("evicts the oldest claim at the exact 5,001st ring slot", async () => {
    const pack = makePack("w3-install-ring-boundary");
    const index = packIndex();
    await index.replaceIndex({ packs: [pack] });
    const oldMarker = `install-marker:${pack.id}:${pack.created_at}:oldest`;
    await runInDurableObject(index, async (_instance, state) => {
      await state.storage.put("meta:install-sequence", 5_000);
      await state.storage.put("install-slot:0", { markerKey: oldMarker, recordedAt: 1_000 });
      await state.storage.put(oldMarker, {
        markerKey: oldMarker,
        recordedAt: 1_000,
        slotKey: "install-slot:0",
      });
    });

    await index.recordInstall(pack.id, "newest", pack.created_at, 2_000);
    const claims = await runInDurableObject(index, async (_instance, state) => ({
      old: await state.storage.get(oldMarker),
      slot: await state.storage.get<{ markerKey: string }>("install-slot:0"),
    }));
    expect(claims.old).toBeUndefined();
    expect(claims.slot?.markerKey).toContain(":newest");
  });

  it("does not rewrite the KV mirror for a duplicate claim", async () => {
    const pack = makePack("w3-install-duplicate");
    const index = packIndex();
    await index.replaceIndex({ packs: [pack] });
    await index.recordInstall(pack.id, "duplicate-identity", pack.created_at, 1_000);
    const put = vi.spyOn(e.ESO_PACKS, "put");

    const retry = await index.recordInstall(pack.id, "duplicate-identity", pack.created_at, 2_000);

    expect(retry).toMatchObject({ install_count: 1 });
    expect(put).not.toHaveBeenCalled();
    put.mockRestore();
  });

  it("heals the KV detail when a duplicate claim retries after mirror loss", async () => {
    const pack = makePack("w3-install-heal");
    const index = packIndex();
    await index.replaceIndex({ packs: [pack] });
    await index.recordInstall(pack.id, "healing-identity", pack.created_at, 1_000);
    await e.ESO_PACKS.delete(`pack:${pack.id}`);

    const retry = await index.recordInstall(pack.id, "healing-identity", pack.created_at, 2_000);

    expect(retry).toMatchObject({ install_count: 1 });
    expect(await e.ESO_PACKS.get<Pack>(`pack:${pack.id}`, "json"))
      .toMatchObject({ install_count: 1 });
  });

  it("deletes install claims in bounded batches during pack cleanup", async () => {
    const pack = makePack("w3-install-cleanup-batches");
    const index = packIndex();
    await index.replaceIndex({ packs: [pack] });
    await runInDurableObject(index, async (_instance, state) => {
      for (let i = 0; i < 201; i++) {
        const markerKey = `install-marker:${pack.id}:${pack.created_at}:identity-${i}`;
        const slotKey = `install-slot:cleanup-${i}`;
        await state.storage.put(markerKey, { markerKey, slotKey, recordedAt: 1_000 });
        await state.storage.put(slotKey, { markerKey, recordedAt: 1_000 });
      }
    });

    await index.removePack(pack.id);

    const remaining = await runInDurableObject(index, async (_instance, state) => ({
      markers: await state.storage.list({ prefix: `install-marker:${pack.id}:` }),
      slots: await state.storage.list({ prefix: "install-slot:cleanup-" }),
    }));
    expect(remaining.markers.size).toBe(0);
    expect(remaining.slots.size).toBe(0);
  });

  it("persists an expiry alarm with the install claim", async () => {
    const pack = makePack("w3-install-alarm-atomic");
    const index = packIndex();
    await index.replaceIndex({ packs: [pack] });
    await index.recordInstall(pack.id, "alarm-identity", pack.created_at);
    const state = await runInDurableObject(index, async (_instance, durableState) => ({
      alarm: await durableState.storage.getAlarm(),
      markerCount: (await durableState.storage.list({ prefix: "install-marker:" })).size,
    }));

    expect(state.alarm).not.toBeNull();
    expect(state.markerCount).toBe(1);
  });

  it("does not carry install suppression into a recreated slug", async () => {
    const oldPack = makePack("w3-install-reuse");
    const newPack = makePack(oldPack.id, {
      created_at: "2026-08-27T00:00:00.000Z",
      updated_at: "2026-08-27T00:00:00.000Z",
    });
    const index = packIndex();
    await index.replaceIndex({ packs: [oldPack] });
    await index.recordInstall(oldPack.id, "same-identity", oldPack.created_at, 1_000);
    await index.removePack(oldPack.id);
    await index.addPack(newPack);

    const result = await index.recordInstall(
      newPack.id,
      "same-identity",
      newPack.created_at,
      2_000,
    );

    expect(result).toMatchObject({ install_count: 1 });
    const oldClaims = await runInDurableObject(index, async (_instance, state) =>
      state.storage.list({ prefix: `install-marker:${oldPack.id}:${oldPack.created_at}:` }));
    expect(oldClaims.size).toBe(0);
  });

  it.each(["vote_count", "install_count"] as const)(
    "preserves a fresh %s when an update carries stale counters",
    async (field) => {
    const pack = makePack(`w1-update-${field}`);
    await packIndex().replaceIndex({ packs: [pack] });
    await putPack(e, pack);
    const index = packIndex();

    await index.bumpPackCounter(pack.id, field, 1);
    const staleUpdate: Pack = { ...pack, title: "Updated title", [field]: 0 };
    await index.updatePack(pack.id, staleUpdate);

    const stored = (await index.getIndex()).packs.find(({ id }) => id === pack.id);
    expect(stored).toMatchObject({ title: "Updated title", [field]: 1 });
    },
  );

  it.each(["vote", "install"] as const)(
    "does not apply a stale %s to a recreated slug",
    async (operation) => {
      const oldPack = makePack(`w1-reused-${operation}`);
      const newPack = makePack(oldPack.id, {
        author_id: "new-owner",
        created_at: "2026-08-26T00:00:00.000Z",
        updated_at: "2026-08-26T00:00:00.000Z",
      });
      const index = packIndex();
      await index.replaceIndex({ packs: [oldPack] });
      await index.removePack(oldPack.id);
      await index.addPack(newPack);

      const result = operation === "vote"
        ? (await index.toggleVote(oldPack.id, "late-voter", oldPack.created_at)).pack
        : await index.bumpPackCounter(oldPack.id, "install_count", 1, oldPack.created_at);

      expect(result).toBeNull();
      expect(await index.getPack(oldPack.id)).toMatchObject({
        author_id: "new-owner",
        vote_count: 0,
        install_count: 0,
      });
      expect(await e.ESO_PACKS.get(`vote:${oldPack.id}:late-voter`)).toBeNull();
    },
  );

  it("rejects a reconciliation write from an earlier slug lifecycle", async () => {
    const oldPack = makePack("w2-reused-write");
    const newPack = makePack(oldPack.id, {
      created_at: "2026-08-27T00:00:00.000Z",
      updated_at: "2026-08-27T00:00:00.000Z",
    });
    const index = packIndex();
    await index.replaceIndex({ packs: [oldPack] });
    await index.removePack(oldPack.id);
    await index.addPack(newPack);

    expect(await index.reconcileWriteD1(oldPack.id, oldPack.created_at, true, true)).toEqual({
      upserted: false,
      tags_replaced: false,
    });
    expect(await index.getPack(oldPack.id)).toMatchObject({ created_at: newPack.created_at });
  });

  it("allows only one reconciliation lease and ignores a stale release", async () => {
    const index = packIndex();
    expect(await index.beginReconciliation("first")).toBe(true);
    expect(await index.beginReconciliation("second")).toBe(false);
    await index.endReconciliation("second");
    expect(await index.beginReconciliation("third")).toBe(false);
    await index.endReconciliation("first");
    expect(await index.beginReconciliation("third")).toBe(true);
    await index.endReconciliation("third");
  });

  it("reports whether reconciliation authority is shadow or DO", async () => {
    const index = packIndex();
    expect((await index.getReconciliationState()).authority).toBe("kv");
    expect((await index.setAuthority("do", [])).ok).toBe(true);
    expect((await index.getReconciliationState()).authority).toBe("do");
  });

  it("preserves packs created while a restore page is being applied", async () => {
    const restored = makePack("w1-restored", { title: "Old title" });
    const concurrent = makePack("w1-concurrent");
    const index = packIndex();
    await index.replaceIndex({ packs: [restored] });
    await index.addPack(concurrent);

    await index.replaceIndexPreserving(
      { packs: [{ ...restored, title: "Restored title" }] },
      [restored.id],
    );

    expect((await index.getIndex()).packs).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ id: restored.id, title: "Restored title" }),
        expect.objectContaining({ id: concurrent.id }),
      ]),
    );
  });

  // Restore-job tests anchor their injected clocks to the real Date.now():
  // job/claim expiries derived from a synthetic epoch land in the past on the
  // real clock, so the alarm the DO schedules fires immediately and its
  // cleanup (which runs on real time) deletes the job mid-test.
  it("allows only one active restore job", async () => {
    const T0 = Date.now();
    const index = packIndex();
    const first = await index.beginRestoreJob({
      backupKey: "backup:latest",
      snapshotCreatedAt: "2026-01-01T00:00:00.000Z",
      snapshotFingerprint: "first",
      total: 2,
      now: T0,
    });
    expect(first.ok).toBe(true);
    if (!first.ok) throw new Error("first restore job did not start");

    const second = await index.beginRestoreJob({
      backupKey: "backup:2026-01-02",
      snapshotCreatedAt: "2026-01-02T00:00:00.000Z",
      snapshotFingerprint: "second",
      total: 1,
      now: T0 + 1_000,
    });
    expect(second).toMatchObject({
      ok: false,
      reason: "active",
      job: { jobId: first.job.jobId },
    });
  });

  it("rejects a concurrent restore page claim while one is in flight", async () => {
    const T0 = Date.now();
    const index = packIndex();
    const started = await index.beginRestoreJob({
      backupKey: "backup:latest",
      snapshotCreatedAt: "2026-01-01T00:00:00.000Z",
      snapshotFingerprint: "claim",
      total: 4,
      now: T0,
    });
    expect(started.ok).toBe(true);
    if (!started.ok) throw new Error("restore job did not start");
    const tokenHash = await restoreTokenHash(started.token);

    const claims = await Promise.all([
      index.claimRestorePage({ tokenHash, limit: 2, now: T0 + 1_000 }),
      index.claimRestorePage({ tokenHash, limit: 2, now: T0 + 1_000 }),
    ]);

    expect(claims.filter((claim) => claim.ok)).toHaveLength(1);
    expect(claims.find((claim) => !claim.ok)).toMatchObject({ ok: false, reason: "in-flight" });
  });

  it("allows an expired restore page claim to be retried", async () => {
    const T0 = Date.now();
    const index = packIndex();
    const started = await index.beginRestoreJob({
      backupKey: "backup:latest",
      snapshotCreatedAt: "2026-01-01T00:00:00.000Z",
      snapshotFingerprint: "retry",
      total: 4,
      now: T0,
    });
    expect(started.ok).toBe(true);
    if (!started.ok) throw new Error("restore job did not start");
    const tokenHash = await restoreTokenHash(started.token);

    const firstClaim = await index.claimRestorePage({ tokenHash, limit: 2, now: T0 + 1_000 });
    expect(firstClaim).toMatchObject({ ok: true, start: 0, end: 2, final: false });

    const retry = await index.claimRestorePage({
      tokenHash,
      limit: 2,
      now: T0 + 1_000 + 5 * 60 * 1_000 + 1,
    });
    expect(retry).toMatchObject({ ok: true, start: 0, end: 2, final: false });
  });

  it("atomically promotes the final restore page while preserving concurrent packs", async () => {
    const preserved = makePack("w1-restore-preserved");
    const restored = makePack("w1-restore-final");
    const index = packIndex();
    await index.replaceIndex({ packs: [preserved] });
    const T0 = Date.now();
    const started = await index.beginRestoreJob({
      backupKey: "backup:latest",
      snapshotCreatedAt: "2026-01-01T00:00:00.000Z",
      snapshotFingerprint: "final",
      total: 1,
      now: T0,
    });
    expect(started.ok).toBe(true);
    if (!started.ok) throw new Error("restore job did not start");
    const tokenHash = await restoreTokenHash(started.token);
    const claim = await index.claimRestorePage({ tokenHash, limit: 1, now: T0 + 1_000 });
    expect(claim).toMatchObject({ ok: true, start: 0, end: 1, final: true });
    if (!claim.ok) throw new Error("restore page was not claimed");

    const completed = await index.completeRestorePage({
      tokenHash,
      claimId: claim.claimId,
      end: claim.end,
      finalReplacement: { packs: [restored], restoredIds: [restored.id] },
      now: T0 + 2_000,
    });

    expect(completed).toMatchObject({ ok: true, job: { status: "done", nextCursor: 1 } });
    const active = await runInDurableObject(index, async (_instance, state) => state.storage.get("restore:active"));
    expect(active).toBeUndefined();
    expect((await index.getIndex()).packs).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ id: preserved.id }),
        expect.objectContaining({ id: restored.id }),
      ]),
    );
  });

  it("removes expired restore jobs from alarm cleanup", async () => {
    const index = packIndex();
    const started = await index.beginRestoreJob({
      backupKey: "backup:latest",
      snapshotCreatedAt: "2026-01-01T00:00:00.000Z",
      snapshotFingerprint: "expired",
      total: 2,
    });
    expect(started.ok).toBe(true);
    if (!started.ok) throw new Error("restore job did not start");

    // Age the job in place, then schedule the alarm in the near future so
    // runDurableObjectAlarm (not an auto-fired past-dated alarm) runs cleanup.
    await runInDurableObject(index, async (_instance, state) => {
      const key = `restore:job:${started.job.jobId}`;
      const job = await state.storage.get<Record<string, unknown>>(key);
      expect(job).toBeTruthy();
      await state.storage.put(key, { ...(job ?? {}), expiresAt: Date.now() - 1 });
      await state.storage.setAlarm(Date.now() + 60_000);
    });

    expect(await runDurableObjectAlarm(index)).toBe(true);
    const remaining = await runInDurableObject(index, async (_instance, state) => ({
      active: await state.storage.get("restore:active"),
      jobCount: (await state.storage.list({ prefix: "restore:job:" })).size,
    }));
    expect(remaining.active).toBeUndefined();
    expect(remaining.jobCount).toBe(0);
  });

  it("filters deleted authors while writing backups", async () => {
    const kept = makePack("w1-backup-kept", { author_id: "backup-kept-author" });
    const doomed = makePack("w1-backup-deleted", { author_id: "backup-deleted-author" });
    const index = packIndex();
    await index.replaceIndex({ packs: [kept, doomed] });
    await index.removePacksByAuthor(doomed.author_id);

    const keptVote: VoteRecord = {
      packId: kept.id,
      userId: "backup-voter",
      votedAt: "2026-01-03T00:00:00.000Z",
    };
    const droppedVoter: VoteRecord = {
      packId: kept.id,
      userId: doomed.author_id,
      votedAt: "2026-01-03T00:00:00.000Z",
    };
    const droppedPackVote: VoteRecord = {
      packId: doomed.id,
      userId: "backup-voter",
      votedAt: "2026-01-03T00:00:00.000Z",
    };
    const incoming: BackupForTest = {
      packs: [kept, doomed],
      created_at: "2026-01-03T00:00:00.000Z",
      packBodies: { [kept.id]: kept, [doomed.id]: doomed },
      votes: {
        [`${kept.id}:backup-voter`]: keptVote,
        [`${kept.id}:${doomed.author_id}`]: droppedVoter,
        [`${doomed.id}:backup-voter`]: droppedPackVote,
      },
    };

    await index.writeBackup("backup:2026-01-03", incoming);

    const latest = await e.ESO_PACKS.get<BackupForTest>("backup:latest", "json");
    const dated = await e.ESO_PACKS.get<BackupForTest>("backup:2026-01-03", "json");
    for (const snapshot of [latest, dated]) {
      expect(snapshot!.packs.map((pack) => pack.id)).toEqual([kept.id]);
      expect(Object.keys(snapshot!.packBodies)).toEqual([kept.id]);
      expect(Object.keys(snapshot!.votes)).toEqual([`${kept.id}:backup-voter`]);
    }
  });

  it("filters a stale backup snapshot after an author deletion", async () => {
    const kept = makePack("w1-stale-backup-kept", { author_id: "stale-kept-author" });
    const doomed = makePack("w1-stale-backup-deleted", { author_id: "stale-deleted-author" });
    const stale: BackupForTest = {
      packs: [kept, doomed],
      created_at: "2026-01-03T00:00:00.000Z",
      packBodies: { [kept.id]: kept, [doomed.id]: doomed },
      votes: {},
    };
    const index = packIndex();
    await index.replaceIndex({ packs: [kept, doomed] });
    await index.writeBackup("backup:2026-01-03", stale);
    expect((await e.ESO_PACKS.get<BackupForTest>("backup:latest", "json"))!.packs.map((pack) => pack.id).sort())
      .toEqual([doomed.id, kept.id].sort());

    await index.removePacksByAuthor(doomed.author_id);
    await index.writeBackup("backup:2026-01-04", stale);

    const latest = await e.ESO_PACKS.get<BackupForTest>("backup:latest", "json");
    const dated = await e.ESO_PACKS.get<BackupForTest>("backup:2026-01-04", "json");
    expect(latest!.packs.map((pack) => pack.id)).toEqual([kept.id]);
    expect(dated!.packs.map((pack) => pack.id)).toEqual([kept.id]);
  });

  it("expires deleted-author markers from alarm cleanup", async () => {
    const index = packIndex();
    await index.removePacksByAuthor("old-author");
    await runInDurableObject(index, async (_instance, state) => {
      const marker = await state.storage.get<Record<string, unknown>>("deleted-author:old-author");
      expect(marker).toBeTruthy();
      await state.storage.put("deleted-author:old-author", { ...(marker ?? {}), expiresAt: 1 });
      // Near-future alarm: a past-dated one auto-fires before
      // runDurableObjectAlarm gets to run it, making the assertion racy.
      await state.storage.setAlarm(Date.now() + 60_000);
    });

    expect(await runDurableObjectAlarm(index)).toBe(true);
    const marker = await runInDurableObject(index, async (_instance, state) =>
      state.storage.get("deleted-author:old-author"),
    );
    expect(marker).toBeUndefined();
  });

  it("backfills repeatedly, keeps DO mutations, and flips only after parity", async () => {
    const index = packIndex();
    const first = makePack("w1-shadow-first");
    const delayed = makePack("w1-shadow-delayed");
    await index.addPack(first);

    // Model a pre-deploy KV write becoming visible only after the first new-code
    // mutation. A latched one-shot bootstrap never imports this second pack.
    await e.ESO_PACKS.put("index:packs", JSON.stringify({ packs: [first, delayed] }));

    expect(await index.migrationParity([first.id, delayed.id])).toMatchObject({
      authority: "kv",
      kv_count: 2,
      do_count: 2,
      missing_from_do: [],
    });

    await index.updatePack(first.id, { ...first, title: "DO wins" });
    await e.ESO_PACKS.put("index:packs", JSON.stringify({ packs: [first, delayed] }));
    expect(await index.getPack(first.id)).toMatchObject({ title: "DO wins" });

    const flipped = await index.setAuthority("do", [first.id, delayed.id]);
    expect(flipped.ok).toBe(true);
    expect(flipped.parity.authority).toBe("do");
  });

  it("keeps KV authority when an untombstoned witness is missing", async () => {
    const result = await packIndex().setAuthority("do", ["w1-missing-witness"]);

    expect(result.ok).toBe(false);
    expect(result.parity.missing_from_do).toEqual(["w1-missing-witness"]);
    expect((await packIndex().migrationParity([])).authority).toBe("kv");
  });

  it("explicitly adopts an independently propagated detail witness", async () => {
    const pack = makePack("w1-detail-witness");
    await putPack(e, pack);

    expect(await packIndex().getPack(pack.id)).toBeNull();

    expect(await packIndex().adoptWitnesses([pack.id])).toMatchObject({
      adopted: [pack.id],
      tombstoned: [],
      unavailable: [],
    });
    expect(await packIndex().getPack(pack.id)).toMatchObject({ id: pack.id });
  });

  it("uses tombstones to reject stale KV resurrection during backfill", async () => {
    const pack = makePack("w1-shadow-tombstone");
    const index = packIndex();
    await index.replaceIndex({ packs: [pack] });
    await index.removePack(pack.id);
    await e.ESO_PACKS.put("index:packs", JSON.stringify({ packs: [pack] }));

    const parity = await index.migrationParity([pack.id]);
    expect(parity).toMatchObject({ do_count: 0, missing_from_do: [] });
    expect(parity.tombstones).toContain(pack.id);
    expect(await index.getPack(pack.id)).toBeNull();
    expect(await index.adoptWitnesses([pack.id])).toMatchObject({
      adopted: [],
      tombstoned: [pack.id],
    });
  });

  it("serializes old vote cleanup before a recreated slug can accept votes", async () => {
    const oldPack = makePack("w1-cleanup-reuse");
    const newPack = makePack(oldPack.id, {
      created_at: "2026-08-26T01:00:00.000Z",
      updated_at: "2026-08-26T01:00:00.000Z",
    });
    const index = packIndex();
    await index.replaceIndex({ packs: [oldPack] });
    await putVote(e, oldPack.id, "same-voter");

    const removing = index.removePack(oldPack.id);
    const recreating = index.addPack(newPack);
    expect(await removing).toBe("ok");
    expect(await recreating).toMatchObject({ ok: true });
    const result = await index.toggleVote(newPack.id, "same-voter", newPack.created_at);
    expect(result).toMatchObject({ voted: true, pack: { vote_count: 1 } });
    expect(await e.ESO_PACKS.get(`vote:${newPack.id}:same-voter`)).not.toBeNull();
  });
});

