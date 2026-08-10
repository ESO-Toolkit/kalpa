import { env } from "cloudflare:workers";
import {
  createScheduledController,
  createExecutionContext,
  waitOnExecutionContext,
} from "cloudflare:test";
import { describe, it, expect } from "vitest";
import worker from "../src/index";
import { putPackIndex, putVote } from "../src/kv";
import type { Env, PackIndex } from "../src/types";
import { makePack } from "./helpers";

const e = env as unknown as Env;

describe("scheduled backup", () => {
  // The handler uses new Date() internally, so the backup key is always today's date
  const today = new Date().toISOString().slice(0, 10);

  it("writes a backup key when index exists", async () => {
    // Remove any existing backup for today so this test is idempotent
    await e.ESO_PACKS.delete(`backup:${today}`);

    const index: PackIndex = { packs: [makePack("backup-a")] };
    await putPackIndex(e, index);

    const ctrl = createScheduledController({
      scheduledTime: new Date(),
      cron: "0 0 * * *",
    });
    const ctx = createExecutionContext();
    await worker.scheduled(ctrl, e, ctx);
    await waitOnExecutionContext(ctx);

    const backup = await e.ESO_PACKS.get(`backup:${today}`);
    expect(backup).toBeTruthy();
    const parsed = JSON.parse(backup!) as PackIndex;
    expect(parsed.packs).toHaveLength(1);
  });

  it("skips backup when index is empty", async () => {
    await e.ESO_PACKS.delete(`backup:${today}`);
    await e.ESO_PACKS.delete("index:packs");

    const ctrl = createScheduledController({
      scheduledTime: new Date(),
      cron: "0 0 * * *",
    });
    const ctx = createExecutionContext();
    await worker.scheduled(ctrl, e, ctx);
    await waitOnExecutionContext(ctx);

    const backup = await e.ESO_PACKS.get(`backup:${today}`);
    expect(backup).toBeNull();
  });

  it("does not overwrite existing backup", async () => {
    await putPackIndex(e, { packs: [makePack("first")] });
    await e.ESO_PACKS.put(`backup:${today}`, '{"packs":[]}');

    const ctrl = createScheduledController({
      scheduledTime: new Date(),
      cron: "0 0 * * *",
    });
    const ctx = createExecutionContext();
    await worker.scheduled(ctrl, e, ctx);
    await waitOnExecutionContext(ctx);

    const backup = await e.ESO_PACKS.get(`backup:${today}`);
    const parsed = JSON.parse(backup!) as PackIndex;
    // Should still be the old backup (empty packs), not overwritten
    expect(parsed.packs).toHaveLength(0);
  });

  it("excludes a deleted user even when the index still lists them", async () => {
    // The race this guards: handleScheduled reads the index and votes, then
    // writes. An account deletion completing in between scrubs backup:latest,
    // but nothing orders the two — so a cron holding the earlier read could put
    // the deleted records straight back, into the one backup key with no TTL,
    // where a later restore replays them.
    //
    // Seeding the index WITH the user and tombstoning them models exactly that:
    // a read that predates the deletion, a write that follows it.
    await e.ESO_PACKS.delete(`backup:${today}`);
    await e.ESO_PACKS.delete("backup:meta");

    const doomed = makePack("purge-me", { author_id: "9001" });
    const kept = makePack("keep-me", { author_id: "9002" });
    await putPackIndex(e, { packs: [doomed, kept] });
    await putVote(e, "purge-me", "9001");
    await putVote(e, "keep-me", "9002");

    await e.ESO_PACKS.put("deleted:9001", new Date().toISOString());

    const ctrl = createScheduledController({
      scheduledTime: new Date(),
      cron: "0 0 * * *",
    });
    const ctx = createExecutionContext();
    await worker.scheduled(ctrl, e, ctx);
    await waitOnExecutionContext(ctx);

    for (const key of [`backup:${today}`, "backup:latest"]) {
      const raw = await e.ESO_PACKS.get(key);
      expect(raw, `${key} was not written`).toBeTruthy();
      const snapshot = JSON.parse(raw!) as {
        packs: Array<{ id: string }>;
        packBodies: Record<string, unknown>;
        votes: Record<string, { userId: string }>;
      };

      expect(snapshot.packs.map((p) => p.id), key).toEqual(["keep-me"]);
      expect(Object.keys(snapshot.packBodies), key).toEqual(["keep-me"]);
      expect(
        Object.values(snapshot.votes).map((v) => v.userId),
        key
      ).not.toContain("9001");
    }

    await e.ESO_PACKS.delete("deleted:9001");
  });
});
