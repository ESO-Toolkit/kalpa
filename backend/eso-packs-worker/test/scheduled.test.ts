import { env } from "cloudflare:workers";
import {
  createScheduledController,
  createExecutionContext,
  waitOnExecutionContext,
} from "cloudflare:test";
import { describe, it, expect } from "vitest";
import worker from "../src/index";
import { putPackIndex } from "../src/kv";
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
});
