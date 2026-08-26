import { env } from "cloudflare:workers";
import { beforeEach, describe, expect, it } from "vitest";
import { getPackIndex, putPack } from "../src/kv";
import type { Env, Pack } from "../src/types";
import { makePack } from "./helpers";

const e = env as unknown as Env;

function packIndex() {
  return e.PACK_INDEX.get(e.PACK_INDEX.idFromName("singleton"));
}

describe("PackIndexDO authoritative mutations", () => {
  beforeEach(async () => {
    await packIndex().replaceIndex({ packs: [] });
  });

  it("accepts only one concurrent create for the same id", async () => {
    const pack = makePack("w1-duplicate-create");

    const results = await Promise.all([
      packIndex().addPack(pack, 25),
      packIndex().addPack({ ...pack }, 25),
    ]);

    expect(results.filter((result) => result.ok)).toHaveLength(1);
    expect(results.find((result) => !result.ok)).toMatchObject({ reason: "duplicate" });
    expect((await getPackIndex(e))!.packs.filter(({ id }) => id === pack.id)).toHaveLength(1);
  });

  it("does not resurrect a deleted pack when a vote carries a stale detail body", async () => {
    const pack = makePack("w1-delete-vote");
    await packIndex().replaceIndex({ packs: [pack] });
    await putPack(e, pack);
    const index = packIndex();

    await index.removePack(pack.id);
    const result = await index.toggleVote(pack.id, "stale-voter", pack);

    expect(result.pack).toBeNull();
    expect((await getPackIndex(e))!.packs.some(({ id }) => id === pack.id)).toBe(false);
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
    expect((await getPackIndex(e))!.packs.some(({ id }) => id === pack.id)).toBe(false);
    expect(await e.ESO_PACKS.get(`pack:${pack.id}`)).toBeNull();
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

    const stored = (await getPackIndex(e))!.packs.find(({ id }) => id === pack.id);
    expect(stored).toMatchObject({ title: "Updated title", [field]: 1 });
    },
  );
});
