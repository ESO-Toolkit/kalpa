import type { Env, Pack, PackType, PackStatus, VoteResponse } from "./types";
import { getPack, getPackIndex, putPack, putPackIndex, getVote, putVote, deleteVote } from "./kv";
import { corsHeaders, handlePreflight } from "./cors";
import { validatePack } from "./validate";
import { SEED_PACKS } from "./seed";
import { handleCreateShare, handleResolveShare, validateBearerToken } from "./shares";

const PACKS_PER_PAGE = 20;

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
  return json(request, { error: "Authentication required" }, 401);
}

function requireAuth(request: Request, env: Env): boolean {
  const key = request.headers.get("X-API-Key");
  if (!key || !env.ADMIN_API_KEY) return false;
  const encoder = new TextEncoder();
  const keyBytes = encoder.encode(key);
  const expectedBytes = encoder.encode(env.ADMIN_API_KEY);
  // timingSafeEqual requires equal-length buffers; compare against self if lengths differ
  // so the call always takes the same time regardless of length mismatch.
  if (keyBytes.byteLength !== expectedBytes.byteLength) {
    crypto.subtle.timingSafeEqual(keyBytes, keyBytes);
    return false;
  }
  return crypto.subtle.timingSafeEqual(keyBytes, expectedBytes);
}

/** Purge the CDN-cached pack list after a mutation. */
async function invalidatePackListCache(url: URL): Promise<void> {
  const cacheKey = new URL("/packs", url.origin);
  await caches.default.delete(new Request(cacheKey));
}

/** Generate a URL-safe slug from a title. */
function slugify(title: string): string {
  return title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 80);
}

// ── GET /packs ─────────────────────────────────────────────────────
async function handleListPacks(request: Request, env: Env, url: URL): Promise<Response> {
  const hasFilters =
    url.searchParams.has("type") ||
    url.searchParams.has("tag") ||
    url.searchParams.has("q") ||
    url.searchParams.has("status") ||
    url.searchParams.has("author");
  const cache = caches.default;

  if (!hasFilters && !url.searchParams.has("page") && !url.searchParams.has("sort")) {
    const cached = await cache.match(request);
    if (cached) return cached;
  }

  const index = await getPackIndex(env);
  if (!index) {
    return json(request, { packs: [], page: 1, sort: "latest" }, 200, 30);
  }

  let packs = index.packs;

  // Status filter — default to "published"
  const statusFilter = url.searchParams.get("status");
  if (statusFilter !== "all") {
    const target = statusFilter ?? "published";
    packs = packs.filter((p) => (p.status ?? "published") === target);
  }

  const authorFilter = url.searchParams.get("author");
  if (authorFilter) {
    packs = packs.filter((p) => p.author_id === authorFilter);
  }

  const typeFilter = url.searchParams.get("type");
  if (typeFilter) {
    packs = packs.filter((p) => p.pack_type === typeFilter);
  }

  const tagFilter = url.searchParams.get("tag");
  if (tagFilter) {
    packs = packs.filter((p) => p.tags.includes(tagFilter));
  }

  const query = url.searchParams.get("q")?.toLowerCase();
  if (query) {
    packs = packs.filter(
      (p) =>
        p.title.toLowerCase().includes(query) ||
        p.description.toLowerCase().includes(query),
    );
  }

  // Sort
  const sort = url.searchParams.get("sort") ?? "latest";
  if (sort === "popular") {
    packs.sort((a, b) => b.vote_count - a.vote_count);
  } else if (sort === "installs") {
    packs.sort((a, b) => b.install_count - a.install_count);
  } else {
    // "latest" — sort by updated_at descending
    packs.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  }

  // Paginate
  const page = Math.max(1, parseInt(url.searchParams.get("page") ?? "1", 10) || 1);
  const start = (page - 1) * PACKS_PER_PAGE;
  const paginated = packs.slice(start, start + PACKS_PER_PAGE);

  const response = json(request, { packs: paginated, page, sort }, 200, 30);

  if (!hasFilters && page === 1 && sort === "latest" && request.method === "GET") {
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
  if (pack.status === "draft") {
    const user = await validateBearerToken(request);
    if (!user) {
      return notFound(request, `Pack "${id}" not found`);
    }
    return json(request, { pack }, 200, 0);
  }
  return json(request, { pack }, 200, 300, "public");
}

// ── POST /packs ────────────────────────────────────────────────────
async function handleCreatePack(request: Request, env: Env, url: URL): Promise<Response> {
  const user = await validateBearerToken(request);
  if (!user) {
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

  const input = body as Record<string, unknown>;

  // Per-user pack limit
  const MAX_PACKS_PER_USER = 25;
  const userId = String(user.id);
  const index = (await getPackIndex(env)) ?? { packs: [] };
  const userPackCount = index.packs.filter((p) => p.author_id === userId).length;
  if (userPackCount >= MAX_PACKS_PER_USER) {
    return json(
      request,
      { error: `Maximum of ${MAX_PACKS_PER_USER} packs reached. Delete some packs to create new ones.` },
      429,
    );
  }

  // Generate ID from title if not provided
  let id = typeof input.id === "string" && input.id.length > 0
    ? input.id
    : slugify(input.title as string);

  // Ensure unique
  const existing = await getPack(env, id);
  if (existing) {
    id = `${id}-${Date.now().toString(36)}`;
  }

  const now = new Date().toISOString();
  const pack: Pack = {
    id,
    title: input.title as string,
    description: input.description as string,
    pack_type: input.pack_type as PackType,
    author_id: userId,
    author_name: user.name,
    is_anonymous: Boolean(input.is_anonymous),
    addons: input.addons as Pack["addons"],
    tags: input.tags as string[],
    vote_count: 0,
    install_count: 0,
    created_at: now,
    updated_at: now,
    status: (input.status as PackStatus) ?? "draft",
  };

  await putPack(env, pack);

  index.packs.push(pack);
  await putPackIndex(env, index);

  await invalidatePackListCache(url);

  return json(request, { pack }, 201);
}

// ── PUT /packs/:id ─────────────────────────────────────────────────
async function handleUpdatePack(
  request: Request,
  env: Env,
  id: string,
  url: URL,
): Promise<Response> {
  const user = await validateBearerToken(request);
  if (!user) {
    return unauthorized(request);
  }

  const existing = await getPack(env, id);
  if (!existing) {
    return notFound(request, `Pack "${id}" not found`);
  }

  if (existing.author_id && String(user.id) !== existing.author_id) {
    return json(request, { error: "Only the pack creator can update it" }, 403);
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

  const input = body as Record<string, unknown>;

  const pack: Pack = {
    id,
    title: input.title as string,
    description: input.description as string,
    pack_type: input.pack_type as PackType,
    author_id: existing.author_id,
    author_name: existing.author_name,
    is_anonymous: Boolean(input.is_anonymous),
    addons: input.addons as Pack["addons"],
    tags: input.tags as string[],
    vote_count: existing.vote_count,
    install_count: existing.install_count,
    created_at: existing.created_at,
    updated_at: new Date().toISOString(),
    status: (input.status as PackStatus) ?? existing.status ?? "published",
  };

  await putPack(env, pack);

  // Update index
  const index = (await getPackIndex(env)) ?? { packs: [] };
  const idx = index.packs.findIndex((p) => p.id === id);
  if (idx >= 0) {
    index.packs[idx] = pack;
  } else {
    index.packs.push(pack);
  }
  await putPackIndex(env, index);

  await invalidatePackListCache(url);

  return json(request, { pack });
}

// ── DELETE /packs/:id ──────────────────────────────────────────────
async function handleDeletePack(
  request: Request,
  env: Env,
  id: string,
  url: URL,
): Promise<Response> {
  const user = await validateBearerToken(request);
  if (!user) {
    return unauthorized(request);
  }

  const existing = await getPack(env, id);
  if (!existing) {
    return notFound(request, `Pack "${id}" not found`);
  }

  if (existing.author_id && String(user.id) !== existing.author_id) {
    return json(request, { error: "Only the pack creator can delete it" }, 403);
  }

  await env.ESO_PACKS.delete(`pack:${id}`);

  const index = (await getPackIndex(env)) ?? { packs: [] };
  index.packs = index.packs.filter((p) => p.id !== id);
  await putPackIndex(env, index);

  await invalidatePackListCache(url);

  return json(request, { ok: true });
}

// ── POST /admin/seed ───────────────────────────────────────────────
async function handleSeed(request: Request, env: Env): Promise<Response> {
  if (env.ALLOW_SEED !== "true") {
    return json(request, { error: "Seed endpoint is disabled in production" }, 403);
  }
  if (!requireAuth(request, env)) {
    return unauthorized(request);
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

  const index = { packs: [...SEED_PACKS] };
  await putPackIndex(env, index);

  return json(request, { ok: true, seeded: SEED_PACKS.length, errors });
}

// ── POST /packs/:id/vote ──────────────────────────────────────────
async function handleVotePack(
  request: Request,
  env: Env,
  id: string,
  url: URL,
): Promise<Response> {
  const pack = await getPack(env, id);
  if (!pack) {
    return notFound(request, `Pack "${id}" not found`);
  }

  const user = await validateBearerToken(request);
  if (!user) {
    return json(request, { error: "Sign in to vote" }, 401);
  }
  const userId = String(user.id);

  const existingVote = await getVote(env, id, userId);
  let voted: boolean;

  if (existingVote) {
    await deleteVote(env, id, userId);
    pack.vote_count = Math.max(0, (pack.vote_count ?? 0) - 1);
    voted = false;
  } else {
    await putVote(env, id, userId);
    pack.vote_count = (pack.vote_count ?? 0) + 1;
    voted = true;
  }

  await putPack(env, pack);

  // Update index
  const index = (await getPackIndex(env)) ?? { packs: [] };
  const idx = index.packs.findIndex((p) => p.id === id);
  if (idx >= 0) {
    index.packs[idx] = pack;
  }
  await putPackIndex(env, index);

  await invalidatePackListCache(url);

  const response: VoteResponse = { voted, voteCount: pack.vote_count };
  return json(request, response);
}

// ── POST /packs/:id/install ────────────────────────────────────────
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

  // Rate limit: one install track per IP per pack per hour
  const ip = request.headers.get("CF-Connecting-IP") ?? "unknown";
  const rateLimitKey = `install-rate:${id}:${ip}`;
  const existing = await env.ESO_PACKS.get(rateLimitKey);
  if (existing) {
    return json(request, { installCount: pack.install_count ?? 0 });
  }
  await env.ESO_PACKS.put(rateLimitKey, "1", { expirationTtl: 3600 });

  pack.install_count = (pack.install_count ?? 0) + 1;
  await putPack(env, pack);

  // Update index
  const index = (await getPackIndex(env)) ?? { packs: [] };
  const idx = index.packs.findIndex((p) => p.id === id);
  if (idx >= 0) {
    index.packs[idx] = pack;
  }
  await putPackIndex(env, index);

  await invalidatePackListCache(url);

  return json(request, { installCount: pack.install_count });
}

// ── GET /health ────────────────────────────────────────────────────
async function handleHealth(request: Request, env: Env): Promise<Response> {
  let kvOk = false;
  try {
    await env.ESO_PACKS.get("health-check");
    kvOk = true;
  } catch {
    // KV read failed
  }

  const index = await getPackIndex(env);
  const packCount = index?.packs.length ?? 0;

  return json(request, {
    status: kvOk ? "ok" : "degraded",
    kv: kvOk,
    packCount,
    timestamp: new Date().toISOString(),
  });
}

// ── Scheduled backup ──────────────────────────────────────────────
async function handleScheduled(env: Env): Promise<void> {
  const index = await getPackIndex(env);
  if (!index || index.packs.length === 0) return;

  const timestamp = new Date().toISOString().slice(0, 10); // YYYY-MM-DD
  const backupKey = `backup:${timestamp}`;

  // Skip if today's backup already exists
  const existing = await env.ESO_PACKS.get(backupKey);
  if (existing) return;

  // Write backup with 90-day TTL (keeps last ~90 daily snapshots)
  await env.ESO_PACKS.put(backupKey, JSON.stringify(index), {
    expirationTtl: 90 * 86400,
  });
  console.log(`Backup written: ${backupKey} (${index.packs.length} packs)`);
}

// ── Router ─────────────────────────────────────────────────────────
export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    try {
      return await handleRequest(request, env);
    } catch (err) {
      console.error(err);
      const message = err instanceof Error ? err.message : "Internal server error";
      return new Response(JSON.stringify({ error: message }), {
        status: 500,
        headers: { "Content-Type": "application/json", ...corsHeaders(request) },
      });
    }
  },

  async scheduled(_controller: ScheduledController, env: Env): Promise<void> {
    try {
      await handleScheduled(env);
    } catch (err) {
      console.error("Scheduled backup failed:", err);
    }
  },
} satisfies ExportedHandler<Env>;

async function handleRequest(request: Request, env: Env): Promise<Response> {
  const url = new URL(request.url);
  const { pathname } = url;
  const method = request.method;

  // CORS preflight
  if (method === "OPTIONS") {
    return handlePreflight(request);
  }

  // Health check
  if (method === "GET" && pathname === "/health") {
    return handleHealth(request, env);
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
  if (method === "POST" && pathname === "/shares") {
    return handleCreateShare(request, env);
  }

  const shareMatch = pathname.match(/^\/shares\/([23456789ABCDEFGHJKMNPQRSTUVWXYZ]{6})$/);
  if (shareMatch && method === "GET") {
    return handleResolveShare(request, env, shareMatch[1]);
  }

  // POST /admin/seed — dev-only seeding route
  if (method === "POST" && pathname === "/admin/seed") {
    return handleSeed(request, env);
  }

  return notFound(request);
}
