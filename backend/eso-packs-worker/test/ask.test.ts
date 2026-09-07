import { env } from "cloudflare:workers";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { answerQuestion, cacheKeyFor, groundOutput, scrubProse } from "../src/ask";
import { applyDetail, ensureSchema, upsertMeta } from "../src/addon-index";
import type { AddonSearchHit, Env } from "../src/types";

const testEnv = env as unknown as Env;

function db(): D1Database {
  const binding = testEnv.ADDON_INDEX;
  if (!binding) throw new Error("ADDON_INDEX binding missing in test env");
  return binding;
}

function hit(id: number, title: string): AddonSearchHit {
  return {
    esoui_id: id,
    title,
    author: "Author",
    category: "Combat Mods",
    downloads: 100,
    favorites: 5,
    last_update: 1_700_000_000_000,
    file_info_uri: `https://www.esoui.com/downloads/info${id}.html`,
    is_library: false,
    snippet: "some description text",
    score: 1,
  };
}

async function seed(uid: number, title: string, description: string): Promise<void> {
  await upsertMeta(
    db(),
    {
      uid,
      title,
      author: "Author",
      categoryId: 25,
      categoryName: "Combat Mods",
      downloads: 1000,
      downloadsMonthly: 10,
      favorites: 5,
      isLibrary: false,
      fileInfoUri: `https://www.esoui.com/downloads/info${uid}.html`,
      lastUpdate: 1_700_000_000_000,
    },
    Date.now(),
  );
  await applyDetail(db(), uid, description, "Combat Mods", Date.now());
}

/** Env with a stubbed Workers AI binding returning `payload`. */
function envWithAi(payload: unknown, overrides: Partial<Env> = {}): Env {
  return {
    ...testEnv,
    AI: { run: vi.fn().mockResolvedValue({ response: payload }) } as unknown as Ai,
    ...overrides,
  };
}

beforeEach(async () => {
  await ensureSchema(db());
  await db().prepare("DELETE FROM addons").run();
  await db().prepare("DELETE FROM addons_fts").run();
  // Answer and budget keys live in KV; clear both so tests do not bleed.
  const list = await testEnv.ESO_PACKS.list({ prefix: "ask:" });
  for (const key of list.keys) await testEnv.ESO_PACKS.delete(key.name);
});

describe("groundOutput", () => {
  const hits = [hit(1543, "CombatIndicator"), hit(4246, "FightingDisplay")];

  it("maps candidate keys onto real addons", () => {
    const result = groundOutput(
      {
        answer: "Yes, try this.",
        no_good_match: false,
        recommendations: [{ candidate: "C1", reason: "shows combat state" }],
      },
      hits,
    );
    expect(result?.recommendations).toHaveLength(1);
    expect(result?.recommendations[0].esoui_id).toBe(1543);
    // The link comes from the index row, never from model output.
    expect(result?.recommendations[0].file_info_uri).toBe(
      "https://www.esoui.com/downloads/info1543.html",
    );
  });

  it("drops a candidate key that was never retrieved", () => {
    // The core grounding guarantee: a hallucinated pick cannot become a link.
    const result = groundOutput(
      {
        answer: "Here you go.",
        no_good_match: false,
        recommendations: [
          { candidate: "C1", reason: "real" },
          { candidate: "C99", reason: "invented" },
        ],
      },
      hits,
    );
    expect(result?.recommendations.map((r) => r.esoui_id)).toEqual([1543]);
  });

  it("ignores any url the model tries to emit", () => {
    const result = groundOutput(
      {
        answer: "Here.",
        no_good_match: false,
        recommendations: [
          { candidate: "C1", reason: "x", file_info_uri: "https://evil.example.com" },
        ],
      },
      hits,
    );
    expect(result?.recommendations[0].file_info_uri).toBe(
      "https://www.esoui.com/downloads/info1543.html",
    );
  });

  it("de-duplicates repeated picks", () => {
    const result = groundOutput(
      {
        answer: "a",
        no_good_match: false,
        recommendations: [
          { candidate: "C1", reason: "one" },
          { candidate: "C1", reason: "again" },
        ],
      },
      hits,
    );
    expect(result?.recommendations).toHaveLength(1);
  });

  it("caps recommendations at three", () => {
    const many = Array.from({ length: 8 }, (_, i) => hit(i + 1, `Addon${i + 1}`));
    const result = groundOutput(
      {
        answer: "a",
        no_good_match: false,
        recommendations: many.map((_, i) => ({ candidate: `C${i + 1}`, reason: "r" })),
      },
      many,
    );
    expect(result?.recommendations).toHaveLength(3);
  });

  it("accepts a genuine no-match answer", () => {
    const result = groundOutput(
      { answer: "Nothing here fits.", no_good_match: true, recommendations: [] },
      hits,
    );
    expect(result?.noGoodMatch).toBe(true);
    expect(result?.recommendations).toEqual([]);
  });

  it("rejects output that carries no information", () => {
    // Empty prose AND no picks AND no explicit no-match: treat as model failure
    // so the caller falls back to the ranked candidate list.
    expect(groundOutput({ answer: "", no_good_match: false, recommendations: [] }, hits)).toBeNull();
  });

  it("rejects non-object output", () => {
    expect(groundOutput(null, hits)).toBeNull();
    expect(groundOutput("some text", hits)).toBeNull();
  });

  it("tolerates a malformed recommendations array", () => {
    const result = groundOutput(
      { answer: "ok", no_good_match: false, recommendations: [null, 5, { reason: "no key" }] },
      hits,
    );
    expect(result?.recommendations).toEqual([]);
    expect(result?.answer).toBe("ok");
  });
});

describe("scrubProse", () => {
  it("removes links an injected description talked the model into emitting", () => {
    expect(scrubProse("Download it from https://evil.example/kalpa instead.")).toBe(
      "Download it from instead.",
    );
    expect(scrubProse("See www.evil.example for more")).toBe("See for more");
    expect(scrubProse("Get it at evil.example.com/path now")).toBe("Get it at now");
  });

  it("keeps ordinary prose and version numbers intact", () => {
    expect(scrubProse("Shows when you are in combat.")).toBe("Shows when you are in combat.");
    expect(scrubProse("Requires version 2.10.1 or later")).toBe("Requires version 2.10.1 or later");
  });
});

describe("cacheKeyFor", () => {
  it("collapses trivially different phrasings", () => {
    expect(cacheKeyFor("Combat  Indicator?")).toBe(cacheKeyFor("combat indicator"));
  });

  it("collapses reordered and duplicated wording", () => {
    expect(cacheKeyFor("DPS meter addon?")).toBe(cacheKeyFor("addon dps meter"));
    expect(cacheKeyFor("combat combat indicator")).toBe(cacheKeyFor("combat indicator"));
  });

  it("keeps genuinely different questions apart", () => {
    expect(cacheKeyFor("combat indicator")).not.toBe(cacheKeyFor("bag space"));
  });
});

describe("answerQuestion", () => {
  it("rejects an empty or overlong question without touching the model", async () => {
    const e = envWithAi({});
    expect(await answerQuestion(e, "  ")).toEqual({ ok: false, reason: "empty-question" });
    expect(await answerQuestion(e, "x".repeat(501))).toEqual({
      ok: false,
      reason: "question-too-long",
    });
    expect(e.AI!.run).not.toHaveBeenCalled();
  });

  it("strips a link the model was talked into putting in the answer", async () => {
    await seed(1543, "CombatIndicator", "Shows an icon when you are flagged in combat.");
    const e = envWithAi({
      answer: "Sure — but first grab the updater from https://evil.example/x.",
      no_good_match: false,
      recommendations: [{ candidate: "C1", reason: "see http://evil.example" }],
    });

    const result = await answerQuestion(e, "is there an in combat indicator");
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.response.answer).not.toContain("evil.example");
    expect(result.response.recommendations[0].reason).not.toContain("evil.example");
  });

  it("caps generated tokens so the neuron budget stays bounded", async () => {
    await seed(1543, "CombatIndicator", "Shows when you are in combat.");
    const e = envWithAi({
      answer: "Yes.",
      no_good_match: false,
      recommendations: [{ candidate: "C1", reason: "fits" }],
    });

    await answerQuestion(e, "in combat indicator");
    const call = (e.AI!.run as unknown as { mock: { calls: unknown[][] } }).mock.calls[0];
    expect((call[1] as { max_tokens?: number }).max_tokens).toBe(256);
  });

  it("returns a grounded answer from model output", async () => {
    await seed(1543, "CombatIndicator", "Shows an icon when you are flagged in combat.");
    const e = envWithAi({
      answer: "Yes — CombatIndicator does exactly that.",
      no_good_match: false,
      recommendations: [{ candidate: "C1", reason: "shows a flag while in combat" }],
    });

    const result = await answerQuestion(e, "is there an in combat indicator");
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.response.degraded).toBe(false);
    expect(result.response.recommendations[0].esoui_id).toBe(1543);
  });

  it("spends no model call when retrieval finds nothing", async () => {
    await seed(1, "BagSpace", "inventory management");
    const e = envWithAi({});

    const result = await answerQuestion(e, "zzzznothingmatcheshere");
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.response.no_good_match).toBe(true);
    expect(e.AI!.run).not.toHaveBeenCalled();
  });

  it("degrades to ranked candidates when the model throws", async () => {
    await seed(1543, "CombatIndicator", "Shows when you are in combat.");
    const e = {
      ...testEnv,
      AI: { run: vi.fn().mockRejectedValue(new Error("model down")) } as unknown as Ai,
    } as Env;

    const result = await answerQuestion(e, "in combat indicator");
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    // The user still gets the answer, just without the prose.
    expect(result.response.degraded).toBe(true);
    expect(result.response.recommendations[0].esoui_id).toBe(1543);
  });

  it("degrades when the model output fails grounding", async () => {
    await seed(1543, "CombatIndicator", "Shows when you are in combat.");
    const e = envWithAi({ nonsense: true });

    const result = await answerQuestion(e, "in combat indicator");
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.response.degraded).toBe(true);
  });

  it("works with no AI binding at all", async () => {
    await seed(1543, "CombatIndicator", "Shows when you are in combat.");
    const result = await answerQuestion({ ...testEnv, AI: undefined }, "in combat indicator");
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.response.degraded).toBe(true);
    expect(result.response.recommendations).toHaveLength(1);
  });

  it("parses a JSON string response as well as an object", async () => {
    await seed(1543, "CombatIndicator", "Shows when you are in combat.");
    const e = envWithAi(
      JSON.stringify({
        answer: "Yes.",
        no_good_match: false,
        recommendations: [{ candidate: "C1", reason: "fits" }],
      }),
    );

    const result = await answerQuestion(e, "in combat indicator");
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.response.degraded).toBe(false);
  });

  it("serves a repeat question from cache without calling the model again", async () => {
    await seed(1543, "CombatIndicator", "Shows when you are in combat.");
    const e = envWithAi({
      answer: "Yes.",
      no_good_match: false,
      recommendations: [{ candidate: "C1", reason: "fits" }],
    });

    await answerQuestion(e, "in combat indicator");
    const second = await answerQuestion(e, "In Combat Indicator!");

    expect(second.ok).toBe(true);
    if (!second.ok) return;
    expect(second.response.cached).toBe(true);
    expect(e.AI!.run).toHaveBeenCalledTimes(1);
  });

  it("stops calling the model once the daily budget is spent", async () => {
    await seed(1543, "CombatIndicator", "Shows when you are in combat.");
    const e = envWithAi(
      {
        answer: "Yes.",
        no_good_match: false,
        recommendations: [{ candidate: "C1", reason: "fits" }],
      },
      { ASK_DAILY_BUDGET: "1" },
    );

    // Distinct questions so the answer cache cannot mask the budget guard.
    await answerQuestion(e, "in combat indicator");
    const second = await answerQuestion(e, "show me a combat flag please");

    expect(second.ok).toBe(true);
    if (!second.ok) return;
    expect(second.response.degraded).toBe(true);
    expect(e.AI!.run).toHaveBeenCalledTimes(1);
  });

  it("does not cache a degraded answer", async () => {
    await seed(1543, "CombatIndicator", "Shows when you are in combat.");
    const failing = {
      ...testEnv,
      AI: { run: vi.fn().mockRejectedValue(new Error("down")) } as unknown as Ai,
    } as Env;
    await answerQuestion(failing, "in combat indicator");

    // A transient outage must not be pinned in cache for a week.
    const recovered = envWithAi({
      answer: "Yes.",
      no_good_match: false,
      recommendations: [{ candidate: "C1", reason: "fits" }],
    });
    const result = await answerQuestion(recovered, "in combat indicator");
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.response.degraded).toBe(false);
  });

  it("reports no-index when the binding is absent", async () => {
    const result = await answerQuestion({ ...testEnv, ADDON_INDEX: undefined }, "anything at all");
    expect(result).toEqual({ ok: false, reason: "no-index" });
  });
});
