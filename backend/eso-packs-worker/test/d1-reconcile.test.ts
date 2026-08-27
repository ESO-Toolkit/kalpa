import { describe, expect, it, vi } from "vitest";
import {
  buildD1ReconciliationPlan,
  reconcileD1,
  recordD1MirrorFailure,
  type D1PackRow,
  type ReconciliationAuthority,
} from "../src/d1-reconcile";
import type { Env, Pack } from "../src/types";
import { makePack } from "./helpers";

function row(pack: Pack): D1PackRow {
  return {
    id: pack.id,
    author_id: pack.author_id,
    author_name: pack.author_name,
    is_anonymous: pack.is_anonymous ? 1 : 0,
    title: pack.title,
    description: pack.description,
    pack_type: pack.pack_type,
    addons: JSON.stringify(
      pack.addons.map(({ esouiId, name, required, note }) => ({ esouiId, name, required, note }))
    ),
    vote_count: pack.vote_count,
  };
}
function fakeEnv(
  options: {
    authority?: ReconciliationAuthority;
    authorityError?: Error;
    rows?: D1PackRow[];
    tags?: Array<{ pack_id: string; tag: string }>;
    mode?: string;
    failMutation?: number;
    d1ReadError?: Error;
    currentLifecycle?: string;
  } = {}
) {
  const writes = new Map<string, string>(),
    sql: string[] = [];
  let mutations = 0;
  const rows = options.rows ?? [],
    tags = options.tags ?? [];
  const db = {
    prepare(statement: string) {
      sql.push(statement);
      return {
        bind(..._values: unknown[]) {
          return this;
        },
        async all() {
          if (options.d1ReadError) throw options.d1ReadError;
          return { results: statement.includes("FROM pack_tags") ? tags : rows };
        },
        async run() {
          mutations++;
          if (options.failMutation === mutations) throw new Error("partial D1 failure");
          return { success: true };
        },
      };
    },
    async batch(statements: Array<{ run(): Promise<unknown> }>) {
      for (const statement of statements) await statement.run();
      return [];
    },
  };
  const stub = {
    getReconciliationState: options.authorityError
      ? vi.fn().mockRejectedValue(options.authorityError)
      : vi.fn().mockResolvedValue(options.authority ?? { packs: [], tombstones: [] }),
    reconcileWriteD1: vi
      .fn()
      .mockImplementation(
        async (_id: string, expectedLifecycle: string, writePack: boolean, writeTags: boolean) => {
          if (options.currentLifecycle && options.currentLifecycle !== expectedLifecycle) {
            return { upserted: false, tags_replaced: false };
          }
          mutations++;
          if (options.failMutation === mutations) throw new Error("partial D1 failure");
          return { upserted: writePack, tags_replaced: writeTags };
        }
      ),
    reconcileDeleteD1: vi.fn().mockImplementation(async () => {
      mutations++;
      if (options.failMutation === mutations) throw new Error("partial D1 failure");
      return true;
    }),
  };
  const env = {
    D1_RECONCILIATION_MODE: options.mode,
    ROSTER_HUB_DB: db,
    ESO_PACKS: {
      async put(key: string, value: string) {
        writes.set(key, value);
      },
    },
    PACK_INDEX: { idFromName: () => "singleton", get: () => stub },
  } as unknown as Env;
  return {
    env,
    writes,
    sql,
    get mutations() {
      return mutations;
    },
    stub,
  };
}

describe("D1 reconciliation", () => {
  it("persists inline mirror failure breadcrumbs", async () => {
    const fixture = fakeEnv();
    await recordD1MirrorFailure(fixture.env, "upsert", "broken", new Error("D1 down"));
    expect(JSON.parse(fixture.writes.get("d1-mirror:last_error")!)).toMatchObject({
      op: "upsert",
      pack_id: "broken",
      message: "D1 down",
    });
  });
  it("plans restoration for a missing authoritative pack", () => {
    const pack = makePack("missing", { tags: ["pvp"] });
    const plan = buildD1ReconciliationPlan({ packs: [pack], tombstones: [] }, [], []);
    expect(plan.upserts.map((item) => item.id)).toEqual(["missing"]);
    expect(plan.tag_replacements.map((item) => item.id)).toEqual(["missing"]);
  });
  it("applies restoration for a missing authoritative pack", async () => {
    const fixture = fakeEnv({
      authority: { packs: [makePack("missing-apply", { tags: ["pvp"] })], tombstones: [] },
      mode: "apply",
    });
    const result = await reconcileD1(fixture.env);
    expect(result.applied).toMatchObject({ upserts: 1, tag_replacements: 1 });
  });
  it("does not apply a stale write plan across slug reuse", async () => {
    const old = makePack("reused-write", { tags: ["old"] });
    const fixture = fakeEnv({
      authority: { packs: [old], tombstones: [] },
      mode: "apply",
      currentLifecycle: "2026-08-27T00:00:00.000Z",
    });
    const result = await reconcileD1(fixture.env);
    expect(result.applied).toEqual({ upserts: 0, tag_replacements: 0, deletes: 0, total: 0 });
    expect(fixture.stub.reconcileWriteD1).toHaveBeenCalledWith(old.id, old.created_at, true, true);
  });
  it("plans deletion only for a proven-owned zombie", () => {
    const zombie = row(makePack("zombie"));
    expect(
      buildD1ReconciliationPlan({ packs: [], tombstones: ["zombie"] }, [zombie], []).deletes
    ).toEqual(["zombie"]);
    const unowned = buildD1ReconciliationPlan({ packs: [], tombstones: [] }, [zombie], []);
    expect(unowned.deletes).toEqual([]);
    expect(unowned.unowned_extra).toEqual(["zombie"]);
  });
  it("removes owned tag-only zombies but leaves unowned tags untouched", async () => {
    const fixture = fakeEnv({
      authority: { packs: [], tombstones: ["owned-tag-zombie"] },
      tags: [
        { pack_id: "owned-tag-zombie", tag: "pvp" },
        { pack_id: "unowned-tag", tag: "pve" },
      ],
      mode: "apply",
    });
    const result = await reconcileD1(fixture.env);
    expect(result.planned.deletes).toBe(1);
    expect(result.applied.deletes).toBe(1);
    expect(result.unowned_extra).toBe(1);
  });
  it("plans removal of a draft still mirrored in D1", () => {
    const draft = makePack("draft", { status: "draft" });
    expect(
      buildD1ReconciliationPlan({ packs: [draft], tombstones: [] }, [row(draft)], []).deletes
    ).toEqual(["draft"]);
  });
  it("removes tag-only orphans for an authoritative draft", async () => {
    const draft = makePack("draft-tags", { status: "draft" });
    const fixture = fakeEnv({
      authority: { packs: [draft], tombstones: [] },
      tags: [
        { pack_id: draft.id, tag: "pvp" },
        { pack_id: "unowned-draft-tag", tag: "pve" },
      ],
      mode: "apply",
    });
    const result = await reconcileD1(fixture.env);
    expect(result.planned.deletes).toBe(1);
    expect(result.applied.deletes).toBe(1);
    expect(result.unowned_extra).toBe(1);
  });
  it("supports valid empty authority", async () => {
    const fixture = fakeEnv({
      authority: { packs: [], tombstones: ["zombie"] },
      rows: [row(makePack("zombie"))],
      mode: "apply",
    });
    const result = await reconcileD1(fixture.env);
    expect(result.stage).toBe("complete");
    expect(result.planned.deletes).toBe(1);
    expect(result.applied.deletes).toBe(1);
    expect(fixture.stub.reconcileDeleteD1).toHaveBeenCalledWith("zombie");
  });
  it("prepares no D1 SQL when authority read fails", async () => {
    const fixture = fakeEnv({ authorityError: new Error("DO unavailable"), mode: "apply" });
    expect((await reconcileD1(fixture.env)).stage).toBe("authority");
    expect(fixture.sql).toEqual([]);
    expect(fixture.mutations).toBe(0);
  });
  it("fails authority validation closed for an unknown pack status", async () => {
    const malformed = { ...makePack("malformed-status"), status: "archived" } as unknown as Pack;
    const fixture = fakeEnv({ authority: { packs: [malformed], tombstones: [] }, mode: "apply" });
    const result = await reconcileD1(fixture.env);
    expect(result.stage).toBe("authority");
    expect(fixture.sql).toEqual([]);
    expect(fixture.mutations).toBe(0);
  });
  it("performs no mutation when D1 read fails", async () => {
    const fixture = fakeEnv({
      authority: { packs: [], tombstones: ["zombie"] },
      d1ReadError: new Error("D1 unavailable"),
      mode: "apply",
    });
    expect((await reconcileD1(fixture.env)).stage).toBe("d1-read");
    expect(fixture.mutations).toBe(0);
  });
  it("defaults unknown modes to dry-run", async () => {
    const fixture = fakeEnv({
      authority: { packs: [makePack("missing")], tombstones: [] },
      mode: "APPLY",
    });
    const result = await reconcileD1(fixture.env);
    expect(result.mode).toBe("dry-run");
    expect(result.mode_invalid).toBe("APPLY");
    expect(result.applied.total).toBe(0);
    expect(fixture.mutations).toBe(0);
  });
  it("fails closed above safety limits", async () => {
    const packs = Array.from({ length: 101 }, (_, i) => makePack(`missing-${i}`));
    const fixture = fakeEnv({ authority: { packs, tombstones: [] }, mode: "apply" });
    const result = await reconcileD1(fixture.env);
    expect(result.stage).toBe("plan-rejected");
    expect(result.limit_hit).toBe("upserts");
    expect(fixture.mutations).toBe(0);
  });
  it.each([
    {
      name: "delete cap",
      expected: "deletes",
      build: () => {
        const packs = Array.from({ length: 300 }, (_, i) => makePack(`live-${i}`));
        const zombies = Array.from({ length: 26 }, (_, i) => makePack(`dead-${i}`));
        return {
          authority: { packs, tombstones: zombies.map(({ id }) => id) },
          rows: [...packs, ...zombies].map(row),
        };
      },
    },
    {
      name: "empty-authority delete cap",
      expected: "empty-authority-deletes",
      build: () => {
        const zombies = Array.from({ length: 6 }, (_, i) => makePack(`empty-dead-${i}`));
        return {
          authority: { packs: [], tombstones: zombies.map(({ id }) => id) },
          rows: zombies.map(row),
        };
      },
    },
    {
      name: "delete ratio",
      expected: "delete-ratio",
      build: () => {
        const packs = Array.from({ length: 50 }, (_, i) => makePack(`ratio-live-${i}`));
        const zombies = Array.from({ length: 6 }, (_, i) => makePack(`ratio-dead-${i}`));
        return {
          authority: { packs, tombstones: zombies.map(({ id }) => id) },
          rows: [...packs, ...zombies].map(row),
        };
      },
    },
    {
      name: "total cap",
      expected: "total",
      build: () => {
        const missing = Array.from({ length: 75 }, (_, i) => makePack(`total-missing-${i}`));
        const stale = makePack("total-stale");
        return {
          authority: { packs: [...missing, stale], tombstones: [] },
          rows: [{ ...row(stale), title: "Old title" }],
        };
      },
    },
    {
      name: "tag replacement cap",
      expected: "tag-replacements",
      build: () => {
        const packs = Array.from({ length: 101 }, (_, i) =>
          makePack(`tag-stale-${i}`, { tags: ["pvp"] })
        );
        return { authority: { packs, tombstones: [] }, rows: packs.map(row) };
      },
    },
  ])("rejects the entire plan at $name plus one", async ({ expected, build }) => {
    const data = build();
    const fixture = fakeEnv({ ...data, mode: "apply" });
    const result = await reconcileD1(fixture.env);
    expect(result.stage).toBe("plan-rejected");
    expect(result.limit_hit).toBe(expected);
    expect(fixture.mutations).toBe(0);
  });
  it.each([
    {
      name: "delete cap",
      build: () => {
        const packs = Array.from({ length: 300 }, (_, i) => makePack(`ok-live-${i}`));
        const zombies = Array.from({ length: 25 }, (_, i) => makePack(`ok-dead-${i}`));
        return {
          authority: { packs, tombstones: zombies.map(({ id }) => id) },
          rows: [...packs, ...zombies].map(row),
        };
      },
    },
    {
      name: "empty-authority delete cap",
      build: () => {
        const zombies = Array.from({ length: 5 }, (_, i) => makePack(`ok-empty-${i}`));
        return {
          authority: { packs: [], tombstones: zombies.map(({ id }) => id) },
          rows: zombies.map(row),
        };
      },
    },
    {
      name: "delete ratio threshold",
      build: () => {
        const packs = Array.from({ length: 50 }, (_, i) => makePack(`ok-ratio-live-${i}`));
        const zombies = Array.from({ length: 5 }, (_, i) => makePack(`ok-ratio-dead-${i}`));
        return {
          authority: { packs, tombstones: zombies.map(({ id }) => id) },
          rows: [...packs, ...zombies].map(row),
        };
      },
    },
    {
      name: "total cap",
      build: () => {
        const packs = Array.from({ length: 75 }, (_, i) => makePack(`ok-total-${i}`));
        return { authority: { packs, tombstones: [] }, rows: [] };
      },
    },
    {
      name: "tag replacement cap",
      build: () => {
        const packs = Array.from({ length: 100 }, (_, i) =>
          makePack(`ok-tag-${i}`, { tags: ["pvp"] })
        );
        return { authority: { packs, tombstones: [] }, rows: packs.map(row) };
      },
    },
  ])("accepts the exact $name", async ({ build }) => {
    const fixture = fakeEnv({ ...build(), mode: "apply" });
    const result = await reconcileD1(fixture.env);
    expect(result.stage).toBe("complete");
    expect(result.limit_hit).toBeUndefined();
    expect(result.applied.total).toBe(result.planned.total);
  });
  it("records durable partial failure state", async () => {
    const fixture = fakeEnv({
      authority: { packs: [makePack("a"), makePack("b")], tombstones: [] },
      mode: "apply",
      failMutation: 2,
    });
    const result = await reconcileD1(fixture.env);
    expect(result.stage).toBe("apply");
    expect(result.applied.upserts).toBe(1);
    expect(JSON.parse(fixture.writes.get("d1-recon:last_error")!)).toMatchObject({
      stage: "apply",
      applied: { upserts: 1 },
    });
  });
  it("only issues statements against packs and pack_tags", async () => {
    const fixture = fakeEnv({
      authority: { packs: [makePack("expected", { tags: ["pve"] })], tombstones: ["zombie"] },
      rows: [row(makePack("zombie"))],
      mode: "apply",
    });
    await reconcileD1(fixture.env);
    expect(fixture.sql.length).toBeGreaterThan(0);
    expect(fixture.sql.every((statement) => /\b(?:packs|pack_tags)\b/.test(statement))).toBe(true);
    expect(fixture.sql.join(" ")).not.toContain("users");
  });
});
