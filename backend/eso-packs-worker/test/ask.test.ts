import { env } from "cloudflare:workers";
import { createExecutionContext, waitOnExecutionContext } from "cloudflare:test";
import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  alsoConsidered,
  answerQuestion,
  applyCosineFloor,
  cacheKeyFor,
  groundOutput,
  scrubProse,
} from "../src/ask";
import { applyDetail, ensureSchema, upsertMeta } from "../src/addon-index";
import {
  VECTORS_KEY,
  VECTOR_DIM,
  VECTOR_UIDS_KEY,
  quantise,
  resetVectorCache,
} from "../src/embeddings";
import worker from "../src/index";
import type { AddonSearchHit, AskResponse, Env } from "../src/types";

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

/** A well-formed model reply picking the first candidate. */
const MODEL_ANSWER = {
  answer: "Yes.",
  no_good_match: false,
  recommendations: [{ candidate: "C1", reason: "fits" }],
};

beforeEach(async () => {
  await ensureSchema(db());
  await db().prepare("DELETE FROM addons").run();
  await db().prepare("DELETE FROM addons_fts").run();
  // Answer and budget keys live in KV; clear both so tests do not bleed.
  const list = await testEnv.ESO_PACKS.list({ prefix: "ask:" });
  for (const key of list.keys) await testEnv.ESO_PACKS.delete(key.name);
  // Vector blobs are module-cached per isolate, so a test that writes one must
  // not leak into the next.
  const vectors = await testEnv.ESO_PACKS.list({ prefix: "vec:" });
  for (const key of vectors.keys) await testEnv.ESO_PACKS.delete(key.name);
  resetVectorCache();
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

  it("caps recommendations at five", () => {
    const many = Array.from({ length: 8 }, (_, i) => hit(i + 1, `Addon${i + 1}`));
    const result = groundOutput(
      {
        answer: "a",
        no_good_match: false,
        recommendations: many.map((_, i) => ({ candidate: `C${i + 1}`, reason: "r" })),
      },
      many,
    );
    // Raised from 3: a hard 3 truncated genuinely relevant results when more
    // than three addons legitimately solve the problem.
    expect(result?.recommendations).toHaveLength(5);
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


describe("applyCosineFloor", () => {
  it("keeps only neighbours close to the best one", () => {
    const kept = applyCosineFloor([
      { uid: 1, cosine: 0.9 },
      { uid: 2, cosine: 0.82 },
      { uid: 3, cosine: 0.7 },
    ]);
    // Floor is max(0.9 - 0.12, 0.5) = 0.78.
    expect(kept.map((h) => h.uid)).toEqual([1, 2]);
  });

  it("drops everything when nothing is semantically near", () => {
    // A question with no match in the corpus still gets 20 nearest neighbours
    // back. Importing them would swamp the keyword hits.
    expect(
      applyCosineFloor([
        { uid: 1, cosine: 0.44 },
        { uid: 2, cosine: 0.4 },
      ]),
    ).toEqual([]);
  });

  it("uses the absolute floor rather than the relative one near the bottom", () => {
    const kept = applyCosineFloor([
      { uid: 1, cosine: 0.55 },
      { uid: 2, cosine: 0.51 },
      { uid: 3, cosine: 0.49 },
    ]);
    expect(kept.map((h) => h.uid)).toEqual([1, 2]);
  });

  it("handles an empty list", () => {
    expect(applyCosineFloor([])).toEqual([]);
  });
});

describe("semantic fusion in answerQuestion", () => {
  function unit(i: number): number[] {
    const vec = new Array(VECTOR_DIM).fill(0);
    vec[i] = 1;
    return vec;
  }

  /** Publish a vector index for the given uids, one basis axis apiece. */
  async function publishVectors(entries: Array<[number, number]>): Promise<void> {
    const blob = new Int8Array(entries.length * VECTOR_DIM);
    entries.forEach(([, axis], row) => blob.set(quantise(unit(axis)), row * VECTOR_DIM));
    await testEnv.ESO_PACKS.put(VECTOR_UIDS_KEY, JSON.stringify(entries.map(([uid]) => uid)));
    await testEnv.ESO_PACKS.put(VECTORS_KEY, blob.buffer as ArrayBuffer);
    resetVectorCache();
  }

  /** AI stub that answers embedding calls with `queryVector` and chat calls
   *  with `payload`. */
  function hybridEnv(queryVector: number[], payload: unknown): Env {
    return {
      ...testEnv,
      AI: {
        run: vi.fn(async (_model: unknown, input: { text?: string[] }) =>
          input.text ? { data: [queryVector] } : { response: payload },
        ),
      } as unknown as Ai,
    } as Env;
  }

  it("surfaces an addon that keyword search never retrieved", async () => {
    await seed(10, "BagSpace", "Inventory management and bag space.");
    // No word here overlaps the question — this is the vocabulary gap BM25
    // cannot cross, and the only route to this addon is the vector index.
    await seed(20, "Compass Tint", "Turns your compass outline red while you fight.");
    await publishVectors([
      [20, 0],
      [10, 5],
    ]);

    const e = hybridEnv(unit(0), {
      answer: "This one.",
      no_good_match: false,
      recommendations: [{ candidate: "C1", reason: "fits" }],
    });

    const result = await answerQuestion(e, "am i flagged in combat");
    expect(result.ok).toBe(true);
    if (!result.ok) return;

    const surfaced = [...result.response.recommendations, ...result.response.also_considered];
    expect(surfaced.map((r) => r.esoui_id)).toContain(20);
    // Rehydrated candidates carry real index data, never anything model-authored.
    const compass = surfaced.find((r) => r.esoui_id === 20);
    expect(compass?.file_info_uri).toBe("https://www.esoui.com/downloads/info20.html");
  });

  it("still answers when the vector blob has never been built", async () => {
    await seed(1543, "CombatIndicator", "Shows an icon when you are flagged in combat.");
    const e = envWithAi({
      answer: "Yes.",
      no_good_match: false,
      recommendations: [{ candidate: "C1", reason: "fits" }],
    });

    const result = await answerQuestion(e, "in combat indicator");
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.response.degraded).toBe(false);
    expect(result.response.recommendations[0].esoui_id).toBe(1543);
    // Exactly one model call: the answer. No blob means no query embed.
    expect(e.AI!.run).toHaveBeenCalledTimes(1);
  });

  it("falls back to bm25 when the embedding call fails", async () => {
    await seed(1543, "CombatIndicator", "Shows an icon when you are flagged in combat.");
    await publishVectors([[1543, 0]]);

    const e = {
      ...testEnv,
      AI: {
        run: vi.fn(async (_model: unknown, input: { text?: string[] }) => {
          if (input.text) throw new Error("embedding model down");
          return {
            response: {
              answer: "Yes.",
              no_good_match: false,
              recommendations: [{ candidate: "C1", reason: "fits" }],
            },
          };
        }),
      } as unknown as Ai,
    } as Env;

    const result = await answerQuestion(e, "in combat indicator");
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    // A broken embedding model must never be visible in the answer.
    expect(result.response.degraded).toBe(false);
    expect(result.response.recommendations[0].esoui_id).toBe(1543);
  });

  it("ignores distant neighbours rather than importing them", async () => {
    await seed(1543, "CombatIndicator", "Shows an icon when you are flagged in combat.");
    await seed(4242, "Fishing Helper", "Tells you which bait to use at each hole.");
    // The stored vectors are orthogonal to the query, so every cosine is 0 and
    // the floor rejects the whole vector list.
    await publishVectors([
      [4242, 3],
      [1543, 4],
    ]);

    const e = hybridEnv(unit(0), {
      answer: "Yes.",
      no_good_match: false,
      recommendations: [{ candidate: "C1", reason: "fits" }],
    });

    const result = await answerQuestion(e, "in combat indicator");
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    // Pure BM25 ordering survives: the combat addon leads, not the fish one.
    expect(result.response.recommendations[0].esoui_id).toBe(1543);
  });
});

describe("alsoConsidered", () => {
  /** 20 keyword hits then one semantic extra — the real shape of a fused list. */
  const fused = [
    ...Array.from({ length: 20 }, (_, i) => hit(i + 1, `Keyword Addon ${i + 1}`)),
    { ...hit(999, "Semantic Only"), semantic: true },
  ];

  it("surfaces a semantic extra that a plain slice could never reach", () => {
    // The delivery bug: extras are appended AFTER up to 20 keyword hits, so
    // slice(0, 8) over the unpicked tail stopped long before them and the whole
    // embedding feature was invisible unless the model picked one itself.
    const out = alsoConsidered(fused, [{ esoui_id: 1 }]);
    expect(out.map((r) => r.esoui_id)).toContain(999);
  });

  it("puts semantic finds first, then keyword hits in rank order", () => {
    const out = alsoConsidered(fused, [{ esoui_id: 1 }]);
    // The semantic-only match leads: it is the one keyword search could not
    // find, so burying it behind hits the user could have typed is backwards.
    expect(out[0].esoui_id).toBe(999);
    expect(out.slice(1).map((r) => r.esoui_id)).toEqual([
      2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
    ]);
  });

  it("keeps semantic extras beyond the front-loaded three, in place", () => {
    // The 20+1 fixture cannot tell ALSO_CONSIDERED_LIMIT (26) apart from the
    // CANDIDATE_COUNT (20) it replaced, and never exercises more semantic
    // extras than the 3 that get front-loaded. This pins both: the full
    // retrieved set survives, and extras 4-6 stay where retrieval put them
    // rather than being dropped for missing the front-load quota.
    const full = [
      ...Array.from({ length: 20 }, (_, i) => hit(i + 1, `Keyword Addon ${i + 1}`)),
      ...Array.from({ length: 6 }, (_, i) => ({
        ...hit(900 + i, `Semantic Only ${i + 1}`),
        semantic: true,
      })),
    ];

    const out = alsoConsidered(full, []).map((r) => r.esoui_id);

    expect(out).toHaveLength(26);
    expect(out.slice(0, 3)).toEqual([900, 901, 902]);
    expect(out.slice(3, 23)).toEqual(Array.from({ length: 20 }, (_, i) => i + 1));
    expect(out.slice(23)).toEqual([903, 904, 905]);
  });

  it("drops nothing that retrieval found and the model did not pick", () => {
    // The list is collapsed behind a disclosure whose promise is that a short
    // answer never looks like it missed something. A display cap broke that
    // promise silently: on a real question "Combat Indicator" was BM25 rank 8,
    // the 8th unpicked hit, and fell off the end of an 8-slot list that had
    // already committed 3 slots to semantic extras.
    const out = alsoConsidered(fused, [{ esoui_id: 1 }]);
    expect(out).toHaveLength(fused.length - 1);
  });

  it("gives every slot to keyword hits when there are no semantic extras", () => {
    const keywordOnly = fused.filter((h) => !h.semantic);
    const out = alsoConsidered(keywordOnly, []);
    expect(out).toHaveLength(keywordOnly.length);
    expect(out.every((r) => r.esoui_id <= 20)).toBe(true);
  });

  it("omits anything the model already picked", () => {
    const out = alsoConsidered(fused, [{ esoui_id: 999 }]);
    expect(out.map((r) => r.esoui_id)).not.toContain(999);
  });
});

describe("admin cache bypass", () => {
  it("skips the cache read so an eval re-run measures fresh answers", async () => {
    await seed(1543, "CombatIndicator", "Shows when you are in combat.");
    const e = envWithAi(MODEL_ANSWER);

    await answerQuestion(e, "in combat indicator");
    const second = await answerQuestion(e, "in combat indicator", { bypassCache: true });

    expect(second.ok).toBe(true);
    if (!second.ok) return;
    expect(second.response.cached).toBe(false);
    // The whole point: the model ran again rather than replaying last week.
    expect(e.AI!.run).toHaveBeenCalledTimes(2);
  });

  it("does not write a bypassed answer back into the cache", async () => {
    // An eval run must not populate the cache real users are served from.
    await seed(1543, "CombatIndicator", "Shows when you are in combat.");
    const e = envWithAi(MODEL_ANSWER);

    await answerQuestion(e, "in combat indicator", { bypassCache: true });
    expect(await testEnv.ESO_PACKS.get(cacheKeyFor("in combat indicator"))).toBeNull();

    // And a subsequent normal ask is therefore still a miss, not a hit.
    const normal = await answerQuestion(e, "in combat indicator");
    expect(normal.ok).toBe(true);
    if (!normal.ok) return;
    expect(normal.response.cached).toBe(false);
  });

  it("still respects the daily budget when bypassing", async () => {
    // A bypass that ignored the budget could exhaust the free allocation in
    // one eval run.
    await seed(1543, "CombatIndicator", "Shows when you are in combat.");
    const e = envWithAi(MODEL_ANSWER, { ASK_DAILY_BUDGET: "1" });

    await answerQuestion(e, "in combat indicator", { bypassCache: true });
    const second = await answerQuestion(e, "show me a combat flag please", {
      bypassCache: true,
    });

    expect(second.ok).toBe(true);
    if (!second.ok) return;
    expect(second.response.degraded).toBe(true);
    expect(e.AI!.run).toHaveBeenCalledTimes(1);
  });
});

describe("POST /ask no_cache gating", () => {
  const BASE = "https://kalpa-pack-hub.eso-toolkit.workers.dev";

  function askRequest(question: string, opts: { noCache?: boolean; key?: string } = {}) {
    const headers: Record<string, string> = { "Content-Type": "application/json" };
    if (opts.key) headers["X-API-Key"] = opts.key;
    return new Request(`${BASE}/ask`, {
      method: "POST",
      headers,
      body: JSON.stringify(
        opts.noCache ? { question, no_cache: true } : { question },
      ),
    });
  }

  async function ask(
    e: Env,
    question: string,
    opts: { noCache?: boolean; key?: string } = {},
  ): Promise<AskResponse> {
    const ctx = createExecutionContext();
    const res = await worker.fetch(askRequest(question, opts), e, ctx);
    await waitOnExecutionContext(ctx);
    expect(res.status).toBe(200);
    return (await res.json()) as AskResponse;
  }

  it("ignores no_cache from an unauthenticated caller", async () => {
    await seed(1543, "CombatIndicator", "Shows when you are in combat.");
    const e = envWithAi(MODEL_ANSWER);

    await ask(e, "in combat indicator");
    // Anonymous no_cache must not buy a model call — that is the budget hole.
    const second = await ask(e, "in combat indicator", { noCache: true });
    expect(second.cached).toBe(true);
    expect(e.AI!.run).toHaveBeenCalledTimes(1);
  });

  it("honours no_cache from an admin caller", async () => {
    await seed(1543, "CombatIndicator", "Shows when you are in combat.");
    const e = envWithAi(MODEL_ANSWER);

    await ask(e, "in combat indicator");
    const second = await ask(e, "in combat indicator", {
      noCache: true,
      key: "test-api-key",
    });
    expect(second.cached).toBe(false);
    expect(e.AI!.run).toHaveBeenCalledTimes(2);
  });

  it("ignores no_cache carrying the wrong key", async () => {
    await seed(1543, "CombatIndicator", "Shows when you are in combat.");
    const e = envWithAi(MODEL_ANSWER);

    await ask(e, "in combat indicator");
    const second = await ask(e, "in combat indicator", {
      noCache: true,
      key: "not-the-key",
    });
    expect(second.cached).toBe(true);
    expect(e.AI!.run).toHaveBeenCalledTimes(1);
  });
});
