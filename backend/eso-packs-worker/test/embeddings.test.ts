import { env } from "cloudflare:workers";
import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  VECTORS_KEY,
  VECTOR_DIM,
  VECTOR_UIDS_KEY,
  QUERY_PREFIX,
  dequantise,
  docText,
  embedDocs,
  embedIndexPage,
  loadVectors,
  normalise,
  quantise,
  resetVectorCache,
  semanticSearch,
} from "../src/embeddings";
import { applyDetail, ensureSchema, upsertMeta } from "../src/addon-index";
import type { Env } from "../src/types";

const testEnv = env as unknown as Env;

function db(): D1Database {
  const binding = testEnv.ADDON_INDEX;
  if (!binding) throw new Error("ADDON_INDEX binding missing in test env");
  return binding;
}

/** Unit basis vector, so cosines against other bases are exactly 0 or 1. */
function basis(i: number): number[] {
  const vec = new Array(VECTOR_DIM).fill(0);
  vec[i] = 1;
  return vec;
}

/** AI stub: embeddings for `{ text }` inputs, taken from `vectors` in order. */
function aiStub(vectors: number[][]) {
  let call = 0;
  return {
    run: vi.fn(async (_model: unknown, input: { text?: string[] }) => {
      const count = input.text?.length ?? 0;
      const data = Array.from({ length: count }, () => vectors[call++] ?? basis(0));
      return { data };
    }),
  } as unknown as Ai;
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
      downloads: 100,
      downloadsMonthly: 10,
      favorites: 1,
      isLibrary: false,
      fileInfoUri: `https://www.esoui.com/downloads/info${uid}.html`,
      lastUpdate: 1_700_000_000_000,
    },
    Date.now(),
  );
  await applyDetail(db(), uid, description, "Combat Mods", Date.now());
}

beforeEach(async () => {
  await ensureSchema(db());
  await db().prepare("DELETE FROM addons").run();
  await db().prepare("DELETE FROM addons_fts").run();
  await db().prepare("DELETE FROM index_meta").run();
  const list = await testEnv.ESO_PACKS.list({ prefix: "vec:" });
  for (const key of list.keys) await testEnv.ESO_PACKS.delete(key.name);
  resetVectorCache();
});

describe("quantise / dequantise", () => {
  it("round-trips a vector within int8 tolerance", () => {
    const original = normalise(
      Array.from({ length: VECTOR_DIM }, (_, i) => Math.sin(i * 0.37) * (i % 5 ? 1 : 3)),
    );
    const restored = dequantise(quantise(original));

    expect(restored).toHaveLength(VECTOR_DIM);
    for (let i = 0; i < VECTOR_DIM; i++) {
      // One int8 step is 1/127 ≈ 0.0079; allow a rounding step either way.
      expect(Math.abs(restored[i] - original[i])).toBeLessThan(0.01);
    }
  });

  it("preserves cosine similarity through quantisation", () => {
    const a = normalise(Array.from({ length: VECTOR_DIM }, (_, i) => Math.cos(i * 0.11)));
    const b = normalise(Array.from({ length: VECTOR_DIM }, (_, i) => Math.cos(i * 0.11 + 0.4)));
    const dot = (x: number[], y: number[]) => x.reduce((sum, v, i) => sum + v * y[i], 0);

    const before = dot(a, b);
    const after = dot(dequantise(quantise(a)), dequantise(quantise(b)));
    expect(Math.abs(after - before)).toBeLessThan(0.01);
  });

  it("does not produce NaN for a zero vector", () => {
    const zero = new Array(VECTOR_DIM).fill(0);
    expect(quantise(zero).every((v) => v === 0)).toBe(true);
    expect(dequantise(quantise(zero)).every((v) => v === 0)).toBe(true);
  });
});

describe("docText", () => {
  it("expands a glued identifier so the model sees words", () => {
    const text = docText("CombatIndicator", "Combat Mods", "Shows combat state.");
    expect(text).toContain("Combat Indicator");
    expect(text).toContain("Combat Mods");
  });

  it("truncates to the model's context", () => {
    expect(docText("A", "B", "x".repeat(5000)).length).toBeLessThanOrEqual(1200);
  });
});

describe("embedDocs", () => {
  it("accepts the { data } response shape", async () => {
    const e = { ...testEnv, AI: aiStub([basis(0), basis(1)]) };
    const vectors = await embedDocs(e, ["one", "two"]);
    expect(vectors).toHaveLength(2);
    expect(vectors[0]).toHaveLength(VECTOR_DIM);
  });

  it("throws on an unrecognised shape rather than inventing vectors", async () => {
    const e = {
      ...testEnv,
      AI: { run: vi.fn().mockResolvedValue({ nonsense: true }) } as unknown as Ai,
    };
    await expect(embedDocs(e, ["one"])).rejects.toThrow(/response shape/i);
  });
});

describe("missing blob", () => {
  it("loadVectors reports null when nothing has been built", async () => {
    expect(await loadVectors(testEnv)).toBeNull();
  });

  it("semanticSearch returns nothing and spends no model call", async () => {
    const ai = aiStub([basis(0)]);
    const e = { ...testEnv, AI: ai };
    expect(await semanticSearch(e, "anything", 20)).toEqual([]);
    // No blob means there is nothing to compare a query against, so the query
    // embed is skipped entirely — a missing index costs nothing.
    expect((ai as unknown as { run: { mock: { calls: unknown[] } } }).run.mock.calls).toHaveLength(
      0,
    );
  });

  it("ignores a torn pair where the uid list and the blob disagree", async () => {
    await testEnv.ESO_PACKS.put(VECTOR_UIDS_KEY, JSON.stringify([1, 2, 3]));
    await testEnv.ESO_PACKS.put(VECTORS_KEY, new Int8Array(VECTOR_DIM).buffer as ArrayBuffer);
    resetVectorCache();
    expect(await loadVectors(testEnv)).toBeNull();
  });
});

describe("embedIndexPage", () => {
  it("walks the corpus in pages and only swaps in a complete blob", async () => {
    await seed(10, "CombatIndicator", "Shows an icon when you are flagged in combat.");
    await seed(20, "BagSpace", "Inventory management and bag space.");

    const e = { ...testEnv, AI: aiStub([basis(0), basis(1)]) };

    const first = await embedIndexPage(e, db(), 1);
    expect(first).toEqual({ embedded: 1, remaining: 1, complete: false });
    // Mid-walk, nothing is live yet.
    expect(await testEnv.ESO_PACKS.get(VECTORS_KEY, "arrayBuffer")).toBeNull();

    const second = await embedIndexPage(e, db(), 1);
    expect(second.complete).toBe(false);
    expect(second.remaining).toBe(0);

    const third = await embedIndexPage(e, db(), 1);
    expect(third).toEqual({ embedded: 2, remaining: 0, complete: true });

    resetVectorCache();
    const index = await loadVectors(testEnv);
    expect(index?.uids).toEqual([10, 20]);
  });

  it("makes the embedded corpus searchable by cosine", async () => {
    await seed(10, "CombatIndicator", "Shows an icon when you are flagged in combat.");
    await seed(20, "BagSpace", "Inventory management and bag space.");

    const e = { ...testEnv, AI: aiStub([basis(0), basis(1)]) };
    await embedIndexPage(e, db(), 50);
    await embedIndexPage(e, db(), 50);

    // Query embeds onto the same axis as uid 10.
    const query = { ...testEnv, AI: aiStub([basis(0)]) };
    resetVectorCache();
    const hits = await semanticSearch(query, "flagged in combat", 20);

    expect(hits[0].uid).toBe(10);
    expect(hits[0].cosine).toBeCloseTo(1, 5);
    expect(hits[1].cosine).toBeCloseTo(0, 5);

    const call = (query.AI as unknown as { run: { mock: { calls: unknown[][] } } }).run.mock
      .calls[0];
    // bge is asymmetric: the query must carry the retrieval prefix.
    expect((call[1] as { text: string[] }).text[0]).toBe(QUERY_PREFIX + "flagged in combat");
  });

  it("skips libraries and retired addons, which /ask does not surface", async () => {
    await seed(10, "LiveAddon", "A normal addon description.");
    await upsertMeta(
      db(),
      {
        uid: 30,
        title: "LibStub",
        author: "A",
        categoryId: 25,
        categoryName: "Libraries",
        downloads: 1,
        downloadsMonthly: 1,
        favorites: 0,
        isLibrary: true,
        fileInfoUri: "",
        lastUpdate: 1,
      },
      Date.now(),
    );
    await applyDetail(db(), 30, "A library.", "Libraries", Date.now());

    const e = { ...testEnv, AI: aiStub([basis(0)]) };
    await embedIndexPage(e, db(), 50);
    await embedIndexPage(e, db(), 50);

    resetVectorCache();
    expect((await loadVectors(testEnv))?.uids).toEqual([10]);
  });
});
