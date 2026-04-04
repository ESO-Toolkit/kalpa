import type { Env, Pack, VoteResponse } from "./types";
import { getPack, getPackIndex, packToIndexItem, putPack, putPackIndex, getVote, putVote, deleteVote } from "./kv";
import { corsHeaders, handlePreflight } from "./cors";
import { validatePack } from "./validate";
import { SEED_PACKS } from "./seed";
import { handleCreateShare, handleResolveShare, validateBearerToken } from "./shares";

function json(
  request: Request,
  data: unknown,
  status = 200,
  cacheMaxAge = 0,
  cacheScope: "public" | "private" = "public",
): Response {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...corsHeaders(request),
  };
  if (cacheMaxAge > 0) {
    headers["Cache-Control"] = `${cacheScope}, max-age=${cacheMaxAge}`;
  }
  return new Response(JSON.stringify(data), { status, headers });
}

function notFound(request: Request, message = "Not found"): Response {
  return json(request, { error: message }, 404);
}

function badRequest(request: Request, errors: unknown): Response {
  return json(request, { error: "Validation failed", details: errors }, 400);
}

function unauthorized(request: Request): Response {
  return json(request, { error: "Invalid or missing API key" }, 401);
}

function requireAuth(request: Request, env: Env): boolean {
  const key = request.headers.get("X-API-Key");
  return key === env.ADMIN_API_KEY;
}

/** Purge the CDN-cached pack list after a mutation.
 *
 * Only unfiltered `GET /packs` responses are cached (see `handleListPacks`).
 * The cache key must match the exact URL used for `cache.put`, which is the
 * bare `/packs` path with no query params. If `handleListPacks` is ever
 * changed to cache filtered requests, this function must be updated to
 * purge those keys as well.
 */
async function invalidatePackListCache(url: URL): Promise<void> {
  const cacheKey = new URL("/packs", url.origin);
  await caches.default.delete(new Request(cacheKey));
}

// ── GET /packs ─────────────────────────────────────────────────────
async function handleListPacks(request: Request, env: Env, url: URL): Promise<Response> {
  // Try the Cache API first for the full pack index (unfiltered requests only)
  const hasFilters =
    url.searchParams.has("type") ||
    url.searchParams.has("tag") ||
    url.searchParams.has("q") ||
    url.searchParams.has("status") ||
    url.searchParams.has("author");
  const cache = caches.default;

  if (!hasFilters) {
    const cached = await cache.match(request);
    if (cached) return cached;
  }

  const index = await getPackIndex(env);
  if (!index) {
    return json(request, { items: [] }, 200, 30);
  }

  let items = index.items;

  // Status filter — default to "published" so browse only shows public packs.
  // Pass ?status=all to include drafts (used by "my packs" queries).
  const statusFilter = url.searchParams.get("status");
  if (statusFilter !== "all") {
    const target = statusFilter ?? "published";
    items = items.filter((p) => (p.status ?? "published") === target);
  }

  const authorFilter = url.searchParams.get("author");
  if (authorFilter) {
    items = items.filter((p) => p.createdBy === authorFilter);
  }

  const typeFilter = url.searchParams.get("type");
  if (typeFilter) {
    items = items.filter((p) => p.type === typeFilter);
  }

  const tagFilter = url.searchParams.get("tag");
  if (tagFilter) {
    items = items.filter((p) => p.tags.includes(tagFilter));
  }

  const query = url.searchParams.get("q")?.toLowerCase();
  if (query) {
    items = items.filter(
      (p) =>
        p.name.toLowerCase().includes(query) ||
        p.description.toLowerCase().includes(query),
    );
  }

  const response = json(request, { items }, 200, 30);

  // Fire-and-forget: cache unfiltered responses at the CDN edge for 30s.
  // If the put fails (quota, transient error) the response still reaches the
  // caller — subsequent requests will just miss the cache and re-fetch from KV.
  if (!hasFilters && request.method === "GET") {
    cache.put(request, response.clone()).catch(console.error);
  }

  return response;
}

// ── GET /packs/:id ─────────────────────────────────────────────────
async function handleGetPack(request: Request, env: Env, id: string): Promise<Response> {
  const pack = await getPack(env, id);
  if (!pack) {
    return notFound(request, `Pack "${id}" not found`);
  }
  if (pack.status === "draft" && !requireAuth(request, env)) {
    return notFound(request, `Pack "${id}" not found`);
  }
  // Pack data is not user-specific — use public so CDN/shared caches can serve it.
  return json(request, pack, 200, 300, "public");
}

// ── POST /packs — create a new pack ────────────────────────────────
async function handleCreatePack(request: Request, env: Env, url: URL): Promise<Response> {
  if (!requireAuth(request, env)) {
    return unauthorized(request);
  }

  let body: unknown;
  try {
    body = await request.json();
  } catch {
    return badRequest(request, [{ field: "body", message: "Invalid JSON" }]);
  }

  const errors = validatePack(body);
  if (errors.length > 0) {
    return badRequest(request, errors);
  }

  const pack = body as Pack;

  // Default status to "draft" if not provided
  if (!pack.status) {
    pack.status = "draft";
  }

  // Check for ID conflict
  const existing = await getPack(env, pack.id);
  if (existing) {
    return json(request, { error: `Pack "${pack.id}" already exists. Use PUT to update.` }, 409);
  }

  // Per-user pack limit (25 max)
  const MAX_PACKS_PER_USER = 25;
  const userId = pack.metadata.createdBy;
  const index = (await getPackIndex(env)) ?? { items: [] };
  const userPackCount = index.items.filter((i) => i.createdBy === userId).length;
  if (userPackCount >= MAX_PACKS_PER_USER) {
    return json(
      request,
      { error: `Maximum of ${MAX_PACKS_PER_USER} packs reached. Delete some packs to create new ones.` },
      429,
    );
  }

  // Stamp metadata timestamps
  const now = new Date().toISOString();
  pack.metadata.createdAt = now;
  pack.metadata.updatedAt = now;

  await putPack(env, pack);

  // Update index (reuse the index fetched for the limit check)
  index.items.push(packToIndexItem(pack));
  await putPackIndex(env, index);

  // Invalidate CDN cache for the pack listing
  await invalidatePackListCache(url);

  return json(request, pack, 201);
}

// ── PUT /packs/:id — update an existing pack ───────────────────────
async function handleUpdatePack(
  request: Request,
  env: Env,
  id: string,
  url: URL,
): Promise<Response> {
  if (!requireAuth(request, env)) {
    return unauthorized(request);
  }

  const existing = await getPack(env, id);
  if (!existing) {
    return notFound(request, `Pack "${id}" not found`);
  }

  let body: unknown;
  try {
    body = await request.json();
  } catch {
    return badRequest(request, [{ field: "body", message: "Invalid JSON" }]);
  }

  const errors = validatePack(body);
  if (errors.length > 0) {
    return badRequest(request, errors);
  }

  const requestAuthor = (body as Pack).metadata?.createdBy;
  if (existing.metadata.createdBy && requestAuthor !== existing.metadata.createdBy) {
    return json(request, { error: "Only the pack creator can update it" }, 403);
  }

  const pack = body as Pack;
  pack.id = id; // Enforce URL id
  pack.status = pack.status ?? existing.status ?? "published"; // Preserve existing status unless explicitly changed
  pack.metadata.createdAt = existing.metadata.createdAt; // Preserve original
  pack.metadata.updatedAt = new Date().toISOString();
  pack.metadata.version = existing.metadata.version + 1;

  await putPack(env, pack);

  // Update index entry
  const index = (await getPackIndex(env)) ?? { items: [] };
  const idx = index.items.findIndex((item) => item.id === id);
  const indexItem = packToIndexItem(pack);
  if (idx >= 0) {
    index.items[idx] = indexItem;
  } else {
    index.items.push(indexItem);
  }
  await putPackIndex(env, index);

  await invalidatePackListCache(url);

  return json(request, pack);
}

// ── DELETE /packs/:id ──────────────────────────────────────────────
async function handleDeletePack(
  request: Request,
  env: Env,
  id: string,
  url: URL,
): Promise<Response> {
  if (!requireAuth(request, env)) {
    return unauthorized(request);
  }

  const existing = await getPack(env, id);
  if (!existing) {
    return notFound(request, `Pack "${id}" not found`);
  }

  const userId = request.headers.get("X-User-Id");
  if (existing.metadata.createdBy && userId !== existing.metadata.createdBy) {
    return json(request, { error: "Only the pack creator can delete it" }, 403);
  }

  await env.ESO_PACKS.delete(`pack:${id}`);

  // Remove from index
  const index = (await getPackIndex(env)) ?? { items: [] };
  index.items = index.items.filter((item) => item.id !== id);
  await putPackIndex(env, index);

  await invalidatePackListCache(url);

  return json(request, { ok: true });
}

// ── POST /admin/seed (dev only) ────────────────────────────────────
async function handleSeed(request: Request, env: Env): Promise<Response> {
  if (!requireAuth(request, env)) {
    return unauthorized(request);
  }
  if (env.ALLOW_SEED !== "true") {
    return json(request, { error: "Seed endpoint is disabled" }, 403);
  }
  const errors: string[] = [];

  for (const pack of SEED_PACKS) {
    const validationErrors = validatePack(pack);
    if (validationErrors.length > 0) {
      errors.push(`Pack "${pack.id}": ${JSON.stringify(validationErrors)}`);
      continue;
    }
    await putPack(env, pack);
  }

  const index = { items: SEED_PACKS.map(packToIndexItem) };
  await putPackIndex(env, index);

  return json(request, {
    ok: true,
    seeded: SEED_PACKS.length,
    errors,
  });
}

// ── POST /packs/:id/vote — toggle upvote ──────────────────────────
async function handleVotePack(
  request: Request,
  env: Env,
  id: string,
  url: URL,
): Promise<Response> {
  const user = await validateBearerToken(request);
  if (!user) {
    return json(request, { error: "Sign in to vote" }, 401);
  }
  const userId = String(user.id);

  const pack = await getPack(env, id);
  if (!pack) {
    return notFound(request, `Pack "${id}" not found`);
  }

  const existingVote = await getVote(env, id, userId);
  let voted: boolean;

  if (existingVote) {
    // Unvote
    await deleteVote(env, id, userId);
    pack.voteCount = Math.max(0, (pack.voteCount ?? 0) - 1);
    voted = false;
  } else {
    // Upvote
    await putVote(env, id, userId);
    pack.voteCount = (pack.voteCount ?? 0) + 1;
    voted = true;
  }

  await putPack(env, pack);

  // Update index entry
  const index = (await getPackIndex(env)) ?? { items: [] };
  const idx = index.items.findIndex((item) => item.id === id);
  const indexItem = packToIndexItem(pack);
  if (idx >= 0) {
    index.items[idx] = indexItem;
  }
  await putPackIndex(env, index);

  await invalidatePackListCache(url);

  const response: VoteResponse = { voted, voteCount: pack.voteCount };
  return json(request, response);
}

// ── POST /packs/:id/install — increment install count ─────────────
async function handleInstallPack(
  request: Request,
  env: Env,
  id: string,
  url: URL,
): Promise<Response> {
  const pack = await getPack(env, id);
  if (!pack) {
    return notFound(request, `Pack "${id}" not found`);
  }

  const ip = request.headers.get("CF-Connecting-IP") ?? "unknown";
  const rateLimitKey = `install-rate:${id}:${ip}`;
  const existing = await env.ESO_PACKS.get(rateLimitKey);
  if (existing) {
    return json(request, { installCount: pack.installCount ?? 0 });
  }
  await env.ESO_PACKS.put(rateLimitKey, "1", { expirationTtl: 3600 });

  pack.installCount = (pack.installCount ?? 0) + 1;
  await putPack(env, pack);

  // Update index entry
  const index = (await getPackIndex(env)) ?? { items: [] };
  const idx = index.items.findIndex((item) => item.id === id);
  const indexItem = packToIndexItem(pack);
  if (idx >= 0) {
    index.items[idx] = indexItem;
  }
  await putPackIndex(env, index);

  await invalidatePackListCache(url);

  return json(request, { installCount: pack.installCount });
}

// ── Router ─────────────────────────────────────────────────────────
export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const { pathname } = url;
    const method = request.method;

    // CORS preflight
    if (method === "OPTIONS") {
      return handlePreflight(request);
    }

    // GET /packs
    if (method === "GET" && pathname === "/packs") {
      return handleListPacks(request, env, url);
    }

    // POST /packs — create
    if (method === "POST" && pathname === "/packs") {
      return handleCreatePack(request, env, url);
    }

    // /packs/:id/vote route
    const voteMatch = pathname.match(/^\/packs\/([a-z0-9-]+)\/vote$/);
    if (voteMatch && method === "POST") {
      return handleVotePack(request, env, voteMatch[1], url);
    }

    // /packs/:id/install route
    const installMatch = pathname.match(/^\/packs\/([a-z0-9-]+)\/install$/);
    if (installMatch && method === "POST") {
      return handleInstallPack(request, env, installMatch[1], url);
    }

    // /packs/:id routes
    if (pathname.startsWith("/packs/")) {
      const id = pathname.slice("/packs/".length);
      if (!id || id.includes("/")) {
        return notFound(request);
      }

      if (method === "GET") return handleGetPack(request, env, id);
      if (method === "PUT") return handleUpdatePack(request, env, id, url);
      if (method === "DELETE") return handleDeletePack(request, env, id, url);
    }

    // ── Share code routes ──────────────────────────────────────────
    // POST /shares — create a share code
    if (method === "POST" && pathname === "/shares") {
      return handleCreateShare(request, env);
    }

    // GET /shares/:code — resolve a share code
    const shareMatch = pathname.match(/^\/shares\/([23456789ABCDEFGHJKMNPQRSTUVWXYZ]{6})$/);
    if (shareMatch && method === "GET") {
      return handleResolveShare(request, env, shareMatch[1]);
    }

    // POST /admin/seed — temporary dev-only seeding route
    if (method === "POST" && pathname === "/admin/seed") {
      return handleSeed(request, env);
    }

    return notFound(request);
  },
} satisfies ExportedHandler<Env>;
