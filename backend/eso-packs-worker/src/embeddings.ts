import type { AddonVectorHit, Env } from "./types";
import {
  DISCONTINUED_CATEGORY_ID,
  ensureSchema,
  expandIdentifier,
  getMeta,
  isMissingTable,
  setMeta,
} from "./addon-index";

/**
 * Semantic retrieval over the ESOUI catalogue, fused with BM25 by `/ask`.
 *
 * BM25 cannot answer "which addon flags me when I'm in combat" against a
 * description that says "turns your compass outline red while you are in
 * combat" — the useful words do not overlap. That vocabulary gap is what put
 * concept recall@20 at ~73% while name lookups sat at ~97%. Embeddings close
 * it, and fusing rather than replacing keeps the name lookups intact.
 *
 * Storage is deliberately dumb: one KV value holding every vector, quantised to
 * int8, loaded once per isolate and scanned in memory. 4174 x 384 int8 is
 * ~1.6MB — far under KV's 25MB value limit, and a full scan is ~1.6M
 * multiply-adds, single-digit milliseconds. A vector database would be a
 * dependency and a bill for a corpus that fits in a rounding error.
 *
 * Cost: the one-time corpus embed is ~4174 docs x ~200 tokens ~= 0.8M tokens,
 * and bge-small bills ~6058 neurons per million tokens — a few thousand
 * neurons, inside a single day's 10k free allocation. A per-query embed is one
 * short sentence, so it is negligible next to the ~25 neurons the Ask model
 * call already costs.
 *
 * Every function here tolerates a MISSING blob and returns nothing, so `/ask`
 * behaves exactly as it did before the index is ever built.
 */

/** VERIFIED present on this account. Do not substitute without first checking
 *  `wrangler ai models` — an unknown id fails at call time, and this module
 *  swallows that into "no semantic hits", which is silent by design. */
export const EMBED_MODEL = "@cf/baai/bge-small-en-v1.5";

/** bge-small-en-v1.5 output width. Stored blobs are (n * VECTOR_DIM) int8. */
export const VECTOR_DIM = 384;

/**
 * bge is an ASYMMETRIC model: documents are embedded bare, queries are
 * embedded with this instruction prefix. Omitting it measurably degrades
 * retrieval, and it is invisible when it happens — the numbers just get worse.
 */
export const QUERY_PREFIX = "Represent this sentence for searching relevant passages: ";

/** Live blob and its uid ordering. Read on every isolate cold start. */
export const VECTORS_KEY = "vec:index:v1";
export const VECTOR_UIDS_KEY = "vec:index:v1:uids";

/** Accumulator for an in-progress build. Never read by search — the whole-corpus
 *  blob is only swapped into the live key once the walk completes, so a paused
 *  or failed build cannot leave a half-corpus live. */
const BUILD_KEY = "vec:build:v1";
const BUILD_UIDS_KEY = "vec:build:v1:uids";

/** D1 cursor for the resumable walk, alongside the crawl's own cursors. */
const CURSOR_META_KEY = "embed_cursor";

/** Texts per AI call. Small enough that a page is 2-3 calls, which is what
 *  keeps an invocation clear of Cloudflare's `error code: 1102` resource kill —
 *  the same constraint that pins MAX_DETAIL_BATCH at 12. */
const EMBED_BATCH = 48;

/** Docs per admin request. Overridable with `?limit=`. */
export const DEFAULT_EMBED_PAGE = 100;
export const MAX_EMBED_PAGE = 200;

/** bge-small truncates at 512 tokens; anything past roughly this many
 *  characters is discarded by the model anyway, so pay for it nowhere. */
const MAX_DOC_CHARS = 1200;

/**
 * The text that represents one addon in vector space.
 *
 * `expandIdentifier` matters as much here as it does in FTS: "CombatIndicator"
 * is one glued token to a tokeniser and a meaningless string to an embedding
 * model, while "Combat Indicator" carries the concept.
 */
export function docText(title: string, category: string, description: string): string {
  return `${expandIdentifier(title)} ${category} ${description}`
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, MAX_DOC_CHARS);
}

/** In-place L2 normalisation. Zero vectors are left alone rather than made NaN. */
export function normalise(vec: number[]): number[] {
  let sum = 0;
  for (const value of vec) sum += value * value;
  const norm = Math.sqrt(sum);
  if (!Number.isFinite(norm) || norm === 0) return vec.map(() => 0);
  return vec.map((value) => value / norm);
}

/**
 * Unit-normalise, then scale to int8.
 *
 * Normalising FIRST is what makes a fixed 127 scale safe: every component then
 * lies in [-1, 1] regardless of the model's raw magnitudes, so nothing clips
 * and the quantisation error is uniform. Cosine similarity is unchanged by
 * scaling, so the dot product of two dequantised rows is still the cosine.
 */
export function quantise(vec: number[]): Int8Array {
  const unit = normalise(vec);
  const out = new Int8Array(unit.length);
  for (let i = 0; i < unit.length; i++) {
    const scaled = Math.round(unit[i] * 127);
    out[i] = scaled > 127 ? 127 : scaled < -127 ? -127 : scaled;
  }
  return out;
}

/** Inverse of `quantise`, up to ~1/127 per component. */
export function dequantise(bytes: Int8Array | ArrayLike<number>): number[] {
  const out: number[] = new Array(bytes.length);
  for (let i = 0; i < bytes.length; i++) out[i] = Number(bytes[i]) / 127;
  return normalise(out);
}

interface VectorIndex {
  uids: number[];
  /** Row-major (uids.length * VECTOR_DIM). */
  matrix: Int8Array;
  /** Per-row L2 norms of the int8 rows, precomputed so search is one pass. */
  norms: Float64Array;
}

/**
 * Module-scope cache: the blob is fetched once per isolate, not once per
 * request. A cold isolate pays one KV read; every request after it pays none.
 * The TTL exists so a rebuild propagates without a deploy.
 */
let cached: { index: VectorIndex | null; at: number } | null = null;
const CACHE_TTL_MS = 5 * 60 * 1000;

/** Tests (and a freshly swapped build) need the next read to go to KV. */
export function resetVectorCache(): void {
  cached = null;
}

function buildIndex(uids: number[], matrix: Int8Array): VectorIndex | null {
  if (uids.length === 0) return null;
  if (matrix.length !== uids.length * VECTOR_DIM) return null;
  const norms = new Float64Array(uids.length);
  for (let row = 0; row < uids.length; row++) {
    const base = row * VECTOR_DIM;
    let sum = 0;
    for (let j = 0; j < VECTOR_DIM; j++) {
      const value = matrix[base + j];
      sum += value * value;
    }
    norms[row] = Math.sqrt(sum);
  }
  return { uids, matrix, norms };
}

/**
 * Load the corpus vectors, or null when they have never been built.
 *
 * Null is a completely normal state — it is what every deployment looks like
 * before the embed job is run — so callers must treat it as "no semantic
 * signal", not as an error.
 */
export async function loadVectors(env: Env): Promise<VectorIndex | null> {
  const now = Date.now();
  if (cached && now - cached.at < CACHE_TTL_MS) return cached.index;

  let index: VectorIndex | null = null;
  try {
    const [uidsRaw, blob] = await Promise.all([
      env.ESO_PACKS.get(VECTOR_UIDS_KEY, "json"),
      env.ESO_PACKS.get(VECTORS_KEY, "arrayBuffer"),
    ]);
    if (Array.isArray(uidsRaw) && blob) {
      const uids = (uidsRaw as unknown[]).filter(
        (value): value is number => typeof value === "number",
      );
      index = buildIndex(uids, new Int8Array(blob));
    }
  } catch (err) {
    console.error("vector index load failed:", err);
    index = null;
  }

  cached = { index, at: now };
  return index;
}

interface AiEmbeddingResponse {
  data?: unknown;
  result?: { data?: unknown };
}

/** The binding's response shape has moved between model families, so read it
 *  defensively rather than trusting one layout. */
function extractVectors(raw: unknown, expected: number): number[][] {
  const candidates: unknown[] = [];
  if (raw && typeof raw === "object") {
    const shaped = raw as AiEmbeddingResponse;
    if (Array.isArray(shaped.data)) candidates.push(shaped.data);
    if (Array.isArray(shaped.result?.data)) candidates.push(shaped.result?.data);
  }
  if (Array.isArray(raw)) candidates.push(raw);

  for (const candidate of candidates) {
    const rows = candidate as unknown[];
    const vectors = rows.filter((row): row is number[] => Array.isArray(row));
    if (vectors.length === expected) return vectors;
  }
  throw new Error("Unrecognised embedding response shape");
}

/** Embed a list of texts, batched. Throws — callers on the read path catch. */
export async function embedDocs(env: Env, texts: string[]): Promise<number[][]> {
  if (!env.AI) throw new Error("AI binding is not configured");
  if (texts.length === 0) return [];

  const out: number[][] = [];
  for (let i = 0; i < texts.length; i += EMBED_BATCH) {
    const slice = texts.slice(i, i + EMBED_BATCH);
    const raw = await env.AI.run(EMBED_MODEL as never, { text: slice } as never);
    out.push(...extractVectors(raw, slice.length));
  }
  return out;
}

/**
 * Rank the whole corpus against one question.
 *
 * Returns `[]` for every failure mode there is — no AI binding, no blob, a
 * model error, a malformed response — because the caller's job is to fall back
 * to BM25 silently, never to fail the request.
 */
export async function semanticSearch(
  env: Env,
  question: string,
  limit: number,
): Promise<AddonVectorHit[]> {
  const index = await loadVectors(env);
  if (!index || !env.AI) return [];

  let query: number[];
  try {
    const [vector] = await embedDocs(env, [QUERY_PREFIX + question]);
    if (!vector || vector.length !== VECTOR_DIM) return [];
    query = normalise(vector);
  } catch (err) {
    console.error("query embedding failed:", err);
    return [];
  }

  const scored: AddonVectorHit[] = [];
  for (let row = 0; row < index.uids.length; row++) {
    const norm = index.norms[row];
    if (norm === 0) continue;
    const base = row * VECTOR_DIM;
    let dot = 0;
    for (let j = 0; j < VECTOR_DIM; j++) dot += query[j] * index.matrix[base + j];
    scored.push({ uid: index.uids[row], cosine: dot / norm });
  }

  scored.sort((a, b) => b.cosine - a.cosine);
  return scored.slice(0, Math.max(limit, 0));
}

// ── Build ────────────────────────────────────────────────────────────

interface EmbedRow {
  uid: number;
  title: string;
  category_name: string;
  description: string;
}

export interface EmbedOutcome {
  embedded: number;
  remaining: number;
  complete: boolean;
}

async function readBuild(env: Env): Promise<{ uids: number[]; bytes: Int8Array }> {
  const [uidsRaw, blob] = await Promise.all([
    env.ESO_PACKS.get(BUILD_UIDS_KEY, "json"),
    env.ESO_PACKS.get(BUILD_KEY, "arrayBuffer"),
  ]);
  const uids = Array.isArray(uidsRaw)
    ? (uidsRaw as unknown[]).filter((value): value is number => typeof value === "number")
    : [];
  const bytes = blob ? new Int8Array(blob) : new Int8Array(0);
  // A torn pair (uids written, blob not) would corrupt every later append.
  // Starting the accumulator over costs one re-walk, which is cheap and safe.
  if (bytes.length !== uids.length * VECTOR_DIM) return { uids: [], bytes: new Int8Array(0) };
  return { uids, bytes };
}

/**
 * Embed one page of live, described addons and append to the build accumulator.
 *
 * Resumable exactly like `/admin/index/backfill`: the caller loops until
 * `complete`. The page is small on purpose — a Cloudflare invocation is killed
 * with `error code: 1102` when it does too much, which is the same reason
 * `MAX_DETAIL_BATCH` is 12.
 */
export async function embedIndexPage(
  env: Env,
  db: D1Database,
  limit: number = DEFAULT_EMBED_PAGE,
): Promise<EmbedOutcome> {
  await ensureSchema(db);
  const page = Math.min(Math.max(limit, 1), MAX_EMBED_PAGE);
  const cursor = Number.parseInt((await getMeta(db, CURSOR_META_KEY)) ?? "0", 10) || 0;

  // Same visibility rules /ask retrieves under: no libraries (dependencies, not
  // choices) and no retired addons. Embedding them would only let the fusion
  // step reintroduce what search deliberately hides.
  const filters = `removed = 0 AND detail_stale = 0 AND is_library = 0
       AND category_id != ${DISCONTINUED_CATEGORY_ID} AND length(description) > 0`;

  let rows: EmbedRow[];
  try {
    const result = await db
      .prepare(
        `SELECT uid, title, category_name, description
           FROM addons
          WHERE uid > ? AND ${filters}
          ORDER BY uid
          LIMIT ?`,
      )
      .bind(cursor, page)
      .all<EmbedRow>();
    rows = result.results ?? [];
  } catch (err) {
    if (isMissingTable(err)) return { embedded: 0, remaining: 0, complete: true };
    throw err;
  }

  if (rows.length === 0) {
    const built = await readBuild(env);
    if (built.uids.length > 0) {
      // Atomic-enough swap: the uid list goes down first, so a failure between
      // the two writes leaves the OLD blob with a NEW uid list, which
      // `buildIndex` rejects on length mismatch and treats as "no index".
      await env.ESO_PACKS.put(VECTOR_UIDS_KEY, JSON.stringify(built.uids));
      await env.ESO_PACKS.put(VECTORS_KEY, built.bytes.buffer as ArrayBuffer);
      await env.ESO_PACKS.delete(BUILD_KEY);
      await env.ESO_PACKS.delete(BUILD_UIDS_KEY);
      resetVectorCache();
    }
    await setMeta(db, CURSOR_META_KEY, "0");
    return { embedded: built.uids.length, remaining: 0, complete: true };
  }

  // A fresh walk must not append to the leftovers of an abandoned one.
  const existing = cursor === 0 ? { uids: [], bytes: new Int8Array(0) } : await readBuild(env);

  const vectors = await embedDocs(
    env,
    rows.map((row) => docText(row.title, row.category_name, row.description)),
  );

  const merged = new Int8Array(existing.bytes.length + rows.length * VECTOR_DIM);
  merged.set(existing.bytes, 0);
  const uids = [...existing.uids];
  let written = 0;
  for (let i = 0; i < rows.length; i++) {
    const vector = vectors[i];
    if (!vector || vector.length !== VECTOR_DIM) continue;
    merged.set(quantise(vector), existing.bytes.length + written * VECTOR_DIM);
    uids.push(rows[i].uid);
    written++;
  }
  const trimmed = merged.subarray(0, existing.bytes.length + written * VECTOR_DIM);

  await env.ESO_PACKS.put(BUILD_KEY, trimmed.slice().buffer as ArrayBuffer);
  await env.ESO_PACKS.put(BUILD_UIDS_KEY, JSON.stringify(uids));

  const nextCursor = rows[rows.length - 1].uid;
  await setMeta(db, CURSOR_META_KEY, String(nextCursor));

  const remainingRow = await db
    .prepare(`SELECT COUNT(*) AS n FROM addons WHERE uid > ? AND ${filters}`)
    .bind(nextCursor)
    .first<{ n: number }>();

  return {
    embedded: written,
    remaining: remainingRow?.n ?? 0,
    complete: false,
  };
}
