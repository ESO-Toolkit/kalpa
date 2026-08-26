import { env } from "cloudflare:workers";
import { beforeEach, describe, expect, it } from "vitest";
import { putPack, putVote } from "../src/kv";
import type { Env, Pack } from "../src/types";
import { makePack } from "./helpers";

const e = env as unknown as Env;

function packIndex() {
  return e.PACK_INDEX.get(e.PACK_INDEX.idFromName("singleton"));
}

describe("PackIndexDO authoritative mutations", () => {
  beforeEach(async () => {
    await packIndex().setAuthority("kv", []);
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
