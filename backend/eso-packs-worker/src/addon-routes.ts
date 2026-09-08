import type { Env } from "./types";
import { corsHeaders } from "./cors";
import { indexStats, searchAddons } from "./addon-index";
import { crawlDetails, reprocessDescriptions, syncFilelist, MAX_DETAIL_BATCH } from "./crawl";
import { answerQuestion, retrieveCandidates } from "./ask";
import { DEFAULT_EMBED_PAGE, MAX_EMBED_PAGE, embedIndexPage } from "./embeddings";
import { readJsonBody } from "./validate";

/**
 * HTTP surface for the addon index.
 *
 * Kept out of index.ts, which is already ~1900 lines and entirely about packs.
 * These routes share nothing with the pack handlers except CORS.
 */

const MAX_QUERY_LENGTH = 200;

function jsonResponse(
  request: Request,
  data: unknown,
  status = 200,
  cacheMaxAge?: number,
): Response {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...corsHeaders(request),
  };
  if (cacheMaxAge !== undefined) {
    headers["Cache-Control"] = `public, max-age=${cacheMaxAge}`;
  }
  return new Response(JSON.stringify(data), { status, headers });
}

/** The index binding is optional, so every route has to answer for its absence.
 *  503 rather than 404: the route exists, the backing store is not attached. */
function indexUnavailable(request: Request): Response {
  return jsonResponse(
    request,
    { error: "Addon index is not configured on this deployment" },
    503,
  );
}

function parsePositiveInt(raw: string | null, fallback: number, max: number): number {
  const value = Number.parseInt(raw ?? "", 10);
  if (!Number.isFinite(value) || value < 0) return fallback;
  return Math.min(value, max);
}

/**
 * GET /addons/search?q=&limit=&offset=&libraries=
 *
 * Public and anonymous by design. This replaces a call the client currently
 * makes straight to esoui.com, so requiring auth here would be a regression in
 * capability, not just in convenience.
 */
export async function handleAddonSearch(
  request: Request,
  env: Env,
  url: URL,
): Promise<Response> {
  const db = env.ADDON_INDEX;
  if (!db) return indexUnavailable(request);

  const query = (url.searchParams.get("q") ?? "").trim();
  if (query.length === 0) {
    return jsonResponse(request, { error: "Missing query parameter 'q'" }, 400);
  }
  if (query.length > MAX_QUERY_LENGTH) {
    return jsonResponse(
      request,
      { error: `Query must be ${MAX_QUERY_LENGTH} characters or fewer` },
      400,
    );
  }

  const limit = parsePositiveInt(url.searchParams.get("limit"), 25, 50) || 25;
  const offset = parsePositiveInt(url.searchParams.get("offset"), 0, 5000);
  const includeLibraries = url.searchParams.get("libraries") === "true";
  const includeDiscontinued = url.searchParams.get("discontinued") === "true";
  // Opt-in, default OFF. The Discover search box stays pure BM25 — free, and
  // already at 96.9% recall on name lookups. This flag exists so the eval
  // harness can score the fused retrieval that /ask actually uses; enabling it
  // by default would put a metered embedding call behind every keystroke pause.
  const semantic = url.searchParams.get("semantic") === "true";

  try {
    if (semantic) {
      const hits = await retrieveCandidates(env, db, query);
      return jsonResponse(request, { hits: hits.slice(0, limit), matched: hits.length, mode: "fused" }, 200, 300);
    }

    const result = await searchAddons(db, query, {
      limit,
      offset,
      includeLibraries,
      includeDiscontinued,
    });
    // Five minutes: long enough to absorb a user retyping the same query,
    // short enough that a freshly indexed addon shows up the same session.
    return jsonResponse(request, result, 200, 300);
  } catch (err) {
    console.error("addon search failed:", err);
    return jsonResponse(request, { error: "Search failed" }, 500);
  }
}

/** GET /addons/stats — index freshness, for the UI's "indexed <date>" line. */
export async function handleAddonStats(request: Request, env: Env): Promise<Response> {
  const db = env.ADDON_INDEX;
  if (!db) return indexUnavailable(request);
  try {
    return jsonResponse(request, await indexStats(db), 200, 300);
  } catch (err) {
    console.error("addon stats failed:", err);
    return jsonResponse(request, { error: "Stats unavailable" }, 500);
  }
}

/**
 * POST /admin/index/sync — reconcile the bulk filelist.
 *
 * One upstream request. Safe to re-run; the daily cron calls the same code.
 */
export async function handleIndexSync(request: Request, env: Env): Promise<Response> {
  const db = env.ADDON_INDEX;
  if (!db) return indexUnavailable(request);
  try {
    return jsonResponse(request, await syncFilelist(db));
  } catch (err) {
    console.error("index sync failed:", err);
    return jsonResponse(
      request,
      { error: err instanceof Error ? err.message : "Sync failed" },
      502,
    );
  }
}

/**
 * POST /admin/index/backfill?limit=N — fetch one page of descriptions.
 *
 * Paged because the initial walk is ~4000 upstream requests and a single Worker
 * invocation cannot make that many. The operator loops on `complete`.
 */
export async function handleIndexBackfill(
  request: Request,
  env: Env,
  url: URL,
): Promise<Response> {
  const db = env.ADDON_INDEX;
  if (!db) return indexUnavailable(request);

  const limit = parsePositiveInt(url.searchParams.get("limit"), MAX_DETAIL_BATCH, MAX_DETAIL_BATCH);
  try {
    return jsonResponse(request, await crawlDetails(db, limit || MAX_DETAIL_BATCH));
  } catch (err) {
    console.error("index backfill failed:", err);
    return jsonResponse(
      request,
      { error: err instanceof Error ? err.message : "Backfill failed" },
      502,
    );
  }
}

/**
 * POST /ask  { "question": "..." }
 *
 * Anonymous by design, like `/addons/search`. Requiring sign-in for the first
 * "is there an addon that…" question would gate the exact moment the feature is
 * most useful to someone who has not invested in Kalpa yet.
 */
export async function handleAsk(request: Request, env: Env): Promise<Response> {
  if (!env.ADDON_INDEX) return indexUnavailable(request);

  const body = await readJsonBody(request);
  if (!body.ok) {
    return jsonResponse(
      request,
      { error: body.reason === "too-large" ? "Question too large" : "Invalid JSON body" },
      400,
    );
  }
  const question = (body.body as { question?: unknown })?.question;
  if (typeof question !== "string") {
    return jsonResponse(request, { error: "Missing 'question'" }, 400);
  }

  try {
    const result = await answerQuestion(env, question);
    if (!result.ok) {
      if (result.reason === "no-index") return indexUnavailable(request);
      return jsonResponse(
        request,
        {
          error:
            result.reason === "question-too-long"
              ? "Question is too long"
              : "Question is empty",
        },
        400,
      );
    }
    return jsonResponse(request, result.response);
  } catch (err) {
    console.error("ask failed:", err);
    return jsonResponse(request, { error: "Ask failed" }, 500);
  }
}

/**
 * POST /admin/index/embed?limit=N — embed one page of the corpus.
 *
 * Paged and resumable like `/admin/index/backfill`, and for the same reason: a
 * Worker invocation that does too much is killed with `error code: 1102`. The
 * operator loops on `complete`.
 *
 * The vectors accumulate under a build key and are only swapped into the live
 * blob when the walk finishes, so an interrupted run leaves the previous index
 * serving rather than half a corpus.
 */
export async function handleIndexEmbed(
  request: Request,
  env: Env,
  url: URL,
): Promise<Response> {
  const db = env.ADDON_INDEX;
  if (!db) return indexUnavailable(request);
  if (!env.AI) {
    return jsonResponse(request, { error: "AI binding is not configured" }, 503);
  }

  const limit = parsePositiveInt(url.searchParams.get("limit"), DEFAULT_EMBED_PAGE, MAX_EMBED_PAGE);
  try {
    return jsonResponse(request, await embedIndexPage(env, db, limit || DEFAULT_EMBED_PAGE));
  } catch (err) {
    console.error("index embed failed:", err);
    return jsonResponse(
      request,
      { error: err instanceof Error ? err.message : "Embed failed" },
      502,
    );
  }
}

/**
 * POST /admin/index/reprocess — re-clean stored descriptions in place.
 *
 * No upstream requests. Run this after changing the text pipeline instead of
 * re-crawling ESOUI. The operator loops until `complete`.
 */
export async function handleIndexReprocess(
  request: Request,
  env: Env,
  url: URL,
): Promise<Response> {
  const db = env.ADDON_INDEX;
  if (!db) return indexUnavailable(request);
  const limit = parsePositiveInt(url.searchParams.get("limit"), 0, 200);
  try {
    return jsonResponse(request, await reprocessDescriptions(db, limit || undefined));
  } catch (err) {
    console.error("index reprocess failed:", err);
    return jsonResponse(
      request,
      { error: err instanceof Error ? err.message : "Reprocess failed" },
      500,
    );
  }
}
