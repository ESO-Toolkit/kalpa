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
    getPack: vi.fn().mockResolvedValue(null),
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
  it("plans deletion only for a proven-owned zombie", () => {
    const zombie = row(makePack("zombie"));
    expect(
      buildD1ReconciliationPlan({ packs: [], tombstones: ["zombie"] }, [zombie], []).deletes
    ).toEqual(["zombie"]);
    const unowned = buildD1ReconciliationPlan({ packs: [], tombstones: [] }, [zombie], []);
    expect(unowned.deletes).toEqual([]);
    expect(unowned.unowned_extra).toEqual(["zombie"]);
  });
  it("plans removal of a draft still mirrored in D1", () => {
    const draft = makePack("draft", { status: "draft" });
    expect(
      buildD1ReconciliationPlan({ packs: [draft], tombstones: [] }, [row(draft)], []).deletes
    ).toEqual(["draft"]);
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
  });
  it("prepares no D1 SQL when authority read fails", async () => {
    const fixture = fakeEnv({ authorityError: new Error("DO unavailable"), mode: "apply" });
    expect((await reconcileD1(fixture.env)).stage).toBe("authority");
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
