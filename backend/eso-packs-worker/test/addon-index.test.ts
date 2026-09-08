import { env } from "cloudflare:workers";
import { describe, it, expect, beforeEach } from "vitest";
import {
  applyDetail,
  contentTokens,
  ensureSchema,
  expandIdentifier,
  isMissingTable,
  indexStats,
  markRemoved,
  pendingDetailUids,
  searchAddons,
  setMeta,
  sweepUnseen,
  toMatchTokens,
  upsertMetaBatch,
  upsertMeta,
  type AddonMetaRow,
} from "../src/addon-index";
import type { Env } from "../src/types";

const testEnv = env as unknown as Env;

function db(): D1Database {
  const binding = testEnv.ADDON_INDEX;
  if (!binding) throw new Error("ADDON_INDEX binding missing in test env");
  return binding;
}

function meta(uid: number, overrides: Partial<AddonMetaRow> = {}): AddonMetaRow {
  return {
    uid,
    title: `Addon ${uid}`,
    author: "SomeAuthor",
    categoryId: 25,
    categoryName: "Combat Mods",
    downloads: 1000,
    downloadsMonthly: 100,
    favorites: 10,
    isLibrary: false,
    fileInfoUri: `https://www.esoui.com/downloads/info${uid}.html`,
    lastUpdate: 1_700_000_000_000,
    ...overrides,
  };
}

/** Seed one fully-indexed addon (metadata + description + FTS row). */
async function seed(
  uid: number,
  title: string,
  description: string,
  overrides: Partial<AddonMetaRow> = {},
): Promise<void> {
  const row = meta(uid, { title, ...overrides });
  await upsertMeta(db(), row, Date.now());
  // Use the row's own category. Hard-coding "Combat Mods" for every fixture
  // meant unrelated addons matched a combat query on their category alone —
  // an artifact that hid nothing under AND but showed up under the union pass.
  await applyDetail(db(), uid, description, row.categoryName, Date.now());
}

beforeEach(async () => {
  await ensureSchema(db());
  await db().prepare("DELETE FROM addons").run();
  await db().prepare("DELETE FROM addons_fts").run();
  await db().prepare("DELETE FROM index_meta").run();
});

describe("toMatchTokens", () => {
  it("reduces a question to bare lowercase tokens", () => {
    expect(toMatchTokens("In Combat Indicator!")).toEqual(["in", "combat", "indicator"]);
  });

  it("strips FTS5 operator syntax that would otherwise throw", () => {
    // A raw MATCH of this string is a syntax error, not an empty result.
    expect(toMatchTokens('combat* AND "flag" OR NEAR(x) ^y -z')).not.toContain("*");
    expect(toMatchTokens('foo "bar" (baz)').join(" ")).toBe("foo bar baz");
  });

  it("drops single characters", () => {
    expect(toMatchTokens("a b combat")).toEqual(["combat"]);
  });

  it("caps tokens AFTER stopwords, so filler cannot crowd out real words", () => {
    // The cap used to run first, so a wordy question spent its budget on
    // filler: "...that can tell me when i am in combat" truncated before
    // "combat", the only word that mattered.
    const wordy = "hi is there any good addon that can tell me when i am in combat";
    expect(contentTokens(toMatchTokens(wordy))).toContain("combat");

    const many = Array.from({ length: 30 }, (_, i) => `word${i}`).join(" ");
    expect(contentTokens(toMatchTokens(many))).toHaveLength(12);
  });

  it("returns nothing for punctuation-only input", () => {
    expect(toMatchTokens("!!! ??? ***")).toEqual([]);
  });
});

describe("contentTokens", () => {
  it("removes filler words when real terms remain", () => {
    expect(contentTokens(["is", "there", "a", "good", "combat", "indicator"])).toEqual([
      "combat",
      "indicator",
    ]);
  });

  it("keeps the original tokens when everything is a stopword", () => {
    // Otherwise an all-filler query searches for nothing and 500s or 400s,
    // when it should just search badly.
    const all = ["is", "there", "any", "good", "addon"];
    expect(contentTokens(all)).toEqual(all);
  });
});

describe("expandIdentifier", () => {
  it("splits PascalCase while keeping the glued original", () => {
    expect(expandIdentifier("CombatIndicator")).toBe("CombatIndicator Combat Indicator");
  });

  it("splits acronym boundaries", () => {
    expect(expandIdentifier("ESOUIHelper")).toBe("ESOUIHelper ESOUI Helper");
  });

  it("separates letters from digits", () => {
    expect(expandIdentifier("Bar5Steps")).toBe("Bar5Steps Bar 5 Steps");
  });

  it("leaves an already-spaced title untouched", () => {
    expect(expandIdentifier("Combat Indicator")).toBe("Combat Indicator");
  });
});

describe("searchAddons", () => {
  it("matches a glued CamelCase title on separated words", async () => {
    // The regression that motivated expandIdentifier: without it "combat
    // indicator" cannot match the title token `combatindicator` at all.
    await seed(1543, "CombatIndicator", "No useful prose here.");
    expect((await searchAddons(db(), "combat indicator")).hits[0].esoui_id).toBe(1543);
  });

  it("still matches the glued spelling", async () => {
    await seed(1543, "CombatIndicator", "No useful prose here.");
    expect((await searchAddons(db(), "CombatIndicator")).hits[0].esoui_id).toBe(1543);
  });

  it("finds an addon by words that appear only in its description", async () => {
    // The whole reason this index exists: esoui.com's search matches titles,
    // so a title-only search cannot connect "flagged in combat" to either of
    // these addons.
    await seed(
      1543,
      "CombatIndicator",
      "Shows a small icon when you are flagged in combat so you do not have to check manually.",
    );
    await seed(4246, "FightingDisplay", "Displays fighting status when in combat.");
    await seed(9999, "Bag Space", "Increases inventory display and bank sorting options.", {
      categoryName: "Bags, Bank, Inventory",
    });

    const result = await searchAddons(db(), "flagged in combat");
    expect(result.hits.map((h) => h.esoui_id)).toContain(1543);
    expect(result.hits.map((h) => h.esoui_id)).not.toContain(9999);
  });

  it("returns BOTH answers when one lacks a generic verb the user typed", async () => {
    // The reported bug. "an addon that shows when I am in combat" returned
    // Fighting Display (its description says "shows") but not Combat
    // Indicator, which says "turns your compass outline red". Two causes: "am"
    // was not a stopword and appears in only 4% of descriptions, so the AND
    // pass required it; and "shows" acted as a hard filter despite appearing
    // in 28% of the corpus and carrying almost no ranking signal.
    await seed(1543, "Combat Indicator", "Turns your compass outline red when you are in combat.");
    await seed(4246, "Fighting Display", "Shows the word Fighting! when in combat.");

    const ids = (await searchAddons(db(), "an addon that shows when I am in combat")).hits.map(
      (h) => h.esoui_id,
    );
    expect(ids).toContain(1543);
    expect(ids).toContain(4246);
  });

  it("ranks the real addon above a patch that mentions it", async () => {
    // Searching an addon by name returned derivatives: "Master Merchant"
    // ranked a 12k-download plugin first and the 3.9M-download addon fourth.
    // FTS5 normalises bm25 by the row's TOTAL length across all columns, so a
    // major addon with a long description loses to a small patch with a short
    // one. The title-phrase boost is scaled by how much of the title the query
    // accounts for, which separates "Master Merchant 3.0" from
    // "Importers for Master Merchant 3.0".
    await seed(1, "Master Merchant 3.0", "A".repeat(1500) + " guild sales history tool", {
      downloads: 3954764,
    });
    await seed(2, "AGS-SortOrder - MasterMerchant Deal", "Sorts deals.", { downloads: 12636 });
    await seed(3, "Importers for Master Merchant 3.0", "Imports data.", { downloads: 9068 });

    expect((await searchAddons(db(), "Master Merchant")).hits[0].esoui_id).toBe(1);
  });

  it("leaves concept searches untouched by the title boost", async () => {
    // No title contains a whole sentence, so the boost must not fire.
    await seed(1, "Combat Indicator", "Turns your compass red when you are in combat.");
    await seed(2, "An addon that shows when I am in combat", "irrelevant filler text");

    const hits = await searchAddons(db(), "an addon that shows when I am in combat");
    expect(hits.hits.length).toBeGreaterThan(0);
  });

  it("promotes a far more popular addon within a near-tied score band", async () => {
    // Measured live: for "combat" the top 12 spanned a 2.8% score band while
    // downloads spanned 197 to 28,114. Inside that noise the ordering was
    // arbitrary, so a 399-download addon outranked the canonical answer.
    await seed(1, "Combat Alpha", "combat helper", { downloads: 200 });
    await seed(2, "Combat Beta", "combat helper", { downloads: 30000 });

    expect((await searchAddons(db(), "combat")).hits[0].esoui_id).toBe(2);
  });

  it("ranks a title match above a description-only match", async () => {
    await seed(1, "CombatIndicator", "Generic helper.");
    await seed(2, "Something Else", "This addon mentions combat indicator in passing.");

    const result = await searchAddons(db(), "combat indicator");
    expect(result.hits[0].esoui_id).toBe(1);
  });

  it("falls back from AND to OR when no addon matches every token", async () => {
    await seed(1, "CombatIndicator", "Shows combat state.");

    const strict = await searchAddons(db(), "combat unrelatedword");
    expect(strict.mode).toBe("or");
    expect(strict.hits.map((h) => h.esoui_id)).toEqual([1]);
  });

  it("reports mode 'none' rather than throwing on an unmatched query", async () => {
    await seed(1, "CombatIndicator", "Shows combat state.");
    const result = await searchAddons(db(), "zzzznothingmatches");
    expect(result).toEqual({ hits: [], matched: 0, mode: "none" });
  });

  it("returns an empty result for punctuation-only input", async () => {
    await seed(1, "CombatIndicator", "Shows combat state.");
    expect(await searchAddons(db(), "!!! ???")).toEqual({ hits: [], matched: 0, mode: "none" });
  });

  it("survives raw FTS5 operator characters in the query", async () => {
    await seed(1, "CombatIndicator", "Shows combat state.");
    // Unbalanced quote + wildcard + operator: a straight MATCH would throw.
    const result = await searchAddons(db(), 'combat" OR * NEAR(');
    expect(result.hits.map((h) => h.esoui_id)).toEqual([1]);
  });

  it("excludes discontinued addons by default and includes them on request", async () => {
    // ~24% of the ESOUI catalogue is category 157 "Discontinued & Outdated".
    // Offering one as the answer to "is there an addon that..." reads as a live
    // recommendation, which is worse than returning nothing.
    await seed(1, "OldCombatThing", "shows combat state", { categoryId: 157 });

    expect((await searchAddons(db(), "combat state")).hits).toHaveLength(0);
    expect(
      (await searchAddons(db(), "combat state", { includeDiscontinued: true })).hits,
    ).toHaveLength(1);
  });

  it("excludes libraries by default and includes them on request", async () => {
    await seed(1, "LibStub", "A combat library for addon authors.", { isLibrary: true });

    expect((await searchAddons(db(), "combat")).hits).toHaveLength(0);
    expect((await searchAddons(db(), "combat", { includeLibraries: true })).hits).toHaveLength(1);
  });

  it("omits removed addons", async () => {
    await seed(1, "CombatIndicator", "Shows combat state.");
    await markRemoved(db(), [1]);
    expect((await searchAddons(db(), "combat")).hits).toHaveLength(0);
  });

  it("honours limit and offset", async () => {
    for (let i = 1; i <= 5; i++) {
      await seed(i, `Combat Addon ${i}`, "combat helper", { downloads: 1000 - i });
    }
    const page1 = await searchAddons(db(), "combat", { limit: 2 });
    const page2 = await searchAddons(db(), "combat", { limit: 2, offset: 2 });
    expect(page1.hits).toHaveLength(2);
    expect(page2.hits).toHaveLength(2);
    expect(page1.hits.map((h) => h.esoui_id)).not.toEqual(page2.hits.map((h) => h.esoui_id));
  });

  it("builds a fallback esoui.com link when the upstream uri is blank", async () => {
    await upsertMeta(db(), meta(1543, { fileInfoUri: "" }), Date.now());
    await applyDetail(db(), 1543, "combat indicator", "Combat Mods", Date.now());

    const result = await searchAddons(db(), "combat");
    expect(result.hits[0].file_info_uri).toBe("https://www.esoui.com/downloads/info1543.html");
  });
});

describe("upsertMeta", () => {
  it("re-queues a description fetch only when lastUpdate moves", async () => {
    await seed(1, "CombatIndicator", "Shows combat state.");
    expect(await pendingDetailUids(db(), 10)).toEqual([]);

    // Download counts churn constantly and say nothing about the text.
    await upsertMeta(db(), meta(1, { title: "CombatIndicator", downloads: 99999 }), Date.now());
    expect(await pendingDetailUids(db(), 10)).toEqual([]);

    await upsertMeta(
      db(),
      meta(1, { title: "CombatIndicator", lastUpdate: 1_800_000_000_000 }),
      Date.now(),
    );
    expect(await pendingDetailUids(db(), 10)).toEqual([1]);
  });

  it("resurrects a previously removed addon", async () => {
    await seed(1, "CombatIndicator", "Shows combat state.");
    await markRemoved(db(), [1]);

    await upsertMeta(db(), meta(1), Date.now());
    const row = await db()
      .prepare("SELECT removed FROM addons WHERE uid = ?")
      .bind(1)
      .first<{ removed: number }>();
    expect(row?.removed).toBe(0);
  });

  it("orders pending work by downloads so a partial backfill is useful first", async () => {
    await upsertMeta(db(), meta(1, { downloads: 10 }), Date.now());
    await upsertMeta(db(), meta(2, { downloads: 5000 }), Date.now());
    await upsertMeta(db(), meta(3, { downloads: 900 }), Date.now());
    expect(await pendingDetailUids(db(), 10)).toEqual([2, 3, 1]);
  });
});

describe("applyDetail", () => {
  it("replaces the old FTS row instead of leaving both searchable", async () => {
    await seed(1, "CombatIndicator", "originaldescriptionword");
    await applyDetail(db(), 1, "replacementdescriptionword", "Combat Mods", Date.now());

    expect((await searchAddons(db(), "originaldescriptionword")).hits).toHaveLength(0);
    expect((await searchAddons(db(), "replacementdescriptionword")).hits).toHaveLength(1);
  });

  it("is a no-op for an addon with no metadata row", async () => {
    await applyDetail(db(), 404, "orphan", "Combat Mods", Date.now());
    expect((await searchAddons(db(), "orphan")).hits).toHaveLength(0);
  });
});

describe("indexStats", () => {
  it("counts live and described rows separately", async () => {
    await seed(1, "CombatIndicator", "Shows combat state.");
    await upsertMeta(db(), meta(2), Date.now()); // metadata only, no description
    await seed(3, "Gone", "removed soon");
    await markRemoved(db(), [3]);

    const stats = await indexStats(db());
    expect(stats.total).toBe(3);
    expect(stats.live).toBe(2);
    expect(stats.described).toBe(1);
  });

  it("reports the description backlog as pending_details", async () => {
    // The depth of this queue is the whole reason the crawl is driven
    // externally: one in-worker page is 12 addons a day.
    await seed(1, "CombatIndicator", "Shows combat state.");
    await upsertMeta(db(), meta(2), Date.now());
    await upsertMeta(db(), meta(3), Date.now());
    // A tombstoned row is not a backlog item — nobody will ever fetch it.
    await seed(4, "Gone", "removed soon");
    await markRemoved(db(), [4]);

    const stats = await indexStats(db());
    expect(stats.pending_details).toBe(2);
    expect(stats.described).toBe(1);
    expect(stats.live).toBe(3);
  });

  it("drops pending_details back to zero once details land", async () => {
    await upsertMeta(db(), meta(1), Date.now());
    expect((await indexStats(db())).pending_details).toBe(1);

    await applyDetail(db(), 1, "now described", "Combat Mods", Date.now());
    expect((await indexStats(db())).pending_details).toBe(0);
  });

  it("reports stale_hours as null when the index has never been synced", async () => {
    // Never-synced and just-synced are opposite alerts, so the never case must
    // not collapse to 0.
    await seed(1, "CombatIndicator", "Shows combat state.");

    const stats = await indexStats(db());
    expect(stats.last_sync).toBeNull();
    expect(stats.stale_hours).toBeNull();
  });

  it("derives stale_hours from last_sync", async () => {
    await seed(1, "CombatIndicator", "Shows combat state.");
    await setMeta(db(), "last_sync", new Date(Date.now() - 30 * 3_600_000).toISOString());

    const stats = await indexStats(db());
    expect(stats.stale_hours).toBeGreaterThanOrEqual(29.9);
    expect(stats.stale_hours).toBeLessThanOrEqual(30.1);
  });

  it("clamps a future last_sync to zero rather than going negative", async () => {
    await seed(1, "CombatIndicator", "Shows combat state.");
    await setMeta(db(), "last_sync", new Date(Date.now() + 60_000).toISOString());

    expect((await indexStats(db())).stale_hours).toBe(0);
  });

  it("reports stale_hours as null when last_sync is unparseable", async () => {
    await seed(1, "CombatIndicator", "Shows combat state.");
    await setMeta(db(), "last_sync", "not-a-date");

    expect((await indexStats(db())).stale_hours).toBeNull();
  });
});

describe("unbuilt index", () => {
  it("recognises the missing-table error", () => {
    expect(isMissingTable(new Error("D1_ERROR: no such table: addons_fts"))).toBe(true);
    expect(isMissingTable(new Error("something else"))).toBe(false);
  });

  it("searches and reports stats as empty rather than throwing", async () => {
    // A freshly provisioned D1 has no tables until the first sync. The read
    // paths must treat that as "nothing indexed yet", or the first deploy
    // serves 500s until someone runs a crawl.
    await db().prepare("DROP TABLE IF EXISTS addons_fts").run();
    await db().prepare("DROP TABLE IF EXISTS addons").run();
    await db().prepare("DROP TABLE IF EXISTS index_meta").run();

    expect(await searchAddons(db(), "combat indicator")).toEqual({
      hits: [],
      matched: 0,
      mode: "none",
    });
    const stats = await indexStats(db());
    expect(stats.total).toBe(0);
    expect(stats.last_sync).toBeNull();
    expect(stats.pending_details).toBe(0);
    expect(stats.stale_hours).toBeNull();

    await ensureSchema(db());
  });
});

describe("D1 limit safety", () => {
  // D1 rejects a query with more than 100 bound parameters, and allows only
  // 1000 queries per Worker invocation. Both were violated by the original
  // one-statement-per-addon design, which would have failed on the very first
  // real sync of ~4000 addons.
  it("writes far more rows than fit in one statement", async () => {
    const rows = Array.from({ length: 250 }, (_, i) =>
      meta(i + 1, { title: `Addon ${i + 1}`, downloads: i }),
    );
    const statements = await upsertMetaBatch(db(), rows, Date.now());

    // 12 bound values per row against a 100-parameter cap means 8 rows per
    // statement, so 250 rows must not be 250 queries.
    expect(statements).toBe(Math.ceil(250 / 8));

    const count = await db()
      .prepare("SELECT COUNT(*) AS n FROM addons")
      .first<{ n: number }>();
    expect(count?.n).toBe(250);
  });

  it("tombstones more addons than the parameter cap allows in one IN clause", async () => {
    const rows = Array.from({ length: 150 }, (_, i) => meta(i + 1));
    await upsertMetaBatch(db(), rows, Date.now());

    await markRemoved(
      db(),
      rows.map((r) => r.uid),
    );

    const live = await db()
      .prepare("SELECT COUNT(*) AS n FROM addons WHERE removed = 0")
      .first<{ n: number }>();
    expect(live?.n).toBe(0);
  });
});

describe("sweepUnseen", () => {
  it("tombstones only rows the current run did not touch", async () => {
    await upsertMetaBatch(db(), [meta(1), meta(2)], 1000);
    await applyDetail(db(), 1, "combat indicator", "Combat Mods", 1000);

    // A later run sees only uid 2.
    await upsertMetaBatch(db(), [meta(2)], 2000);
    const removed = await sweepUnseen(db(), 2000);

    expect(removed).toBe(1);
    const row = await db()
      .prepare("SELECT removed FROM addons WHERE uid = 1")
      .first<{ removed: number }>();
    expect(row?.removed).toBe(1);
    // And it must leave the FTS index, or a removed addon keeps matching.
    expect((await searchAddons(db(), "combat indicator")).hits).toHaveLength(0);
  });

  it("is a no-op when every row was seen", async () => {
    await upsertMetaBatch(db(), [meta(1), meta(2)], 3000);
    expect(await sweepUnseen(db(), 3000)).toBe(0);
  });
});
