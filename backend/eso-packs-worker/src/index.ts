import type { Env, Pack, PackType, PackStatus, PackView, VoteRecord, VoteResponse } from "./types";
import {
  getPack,
  getPackIndex,
  putPack,
  getVotedPackIds,
  getVote,
  deleteVotesForPack,
  restoreVote,
  listAllVotes,
} from "./kv";
import { corsHeaders, handlePreflight } from "./cors";
import { redactAnonymousPack, ANONYMOUS_AUTHOR_NAME } from "./redact";
import { readJsonBody, sanitizeAddons, validatePack } from "./validate";
import { SEED_PACKS } from "./seed";
import { handleCreateShare, handleResolveShare, validateBearerToken } from "./shares";
export { PackIndexDO } from "./pack-index-do";

// ── D1 dual-write helpers ─────────────────────────────────────────
// Both workers share the same Cloudflare account. kalpa-pack-hub binds
// directly to roster-hub-db (D1) so every KV mutation is atomically
// mirrored — no async sync, no reconciliation, no deployment ordering.

async function d1UpsertPack(env: Env, pack: Pack): Promise<void> {
  if (!env.ROSTER_HUB_DB) return;
  const isPublished = (pack.status ?? "published") === "published";
  try {
    if (isPublished) {
      const addonsJson = JSON.stringify(pack.addons.map((a) => ({
        esouiId: a.esouiId,
        name: a.name,
        required: a.required,
        note: a.note,
      })));
      await env.ROSTER_HUB_DB
        .prepare(
          `INSERT INTO packs (id, author_id, author_name, is_anonymous, title, description, pack_type, addons, vote_count, created_at, updated_at)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
           ON CONFLICT(id) DO UPDATE SET
             title = excluded.title,
             description = excluded.description,
             pack_type = excluded.pack_type,
             addons = excluded.addons,
             is_anonymous = excluded.is_anonymous,
             author_name = excluded.author_name,
             vote_count = excluded.vote_count,
             updated_at = datetime('now')`,
        )
        .bind(
          pack.id,
          pack.author_id,
          // The D1 mirror feeds the ESO Toolkit website; never hand it the
          // real display name of an anonymous pack's author. author_id stays
          // for ownership joins but is not rendered there.
          pack.is_anonymous ? ANONYMOUS_AUTHOR_NAME : pack.author_name,
          pack.is_anonymous ? 1 : 0,
          pack.title,
          pack.description,
          pack.pack_type,
          addonsJson,
          // Inserting a literal 0 here (and omitting vote_count from the
          // upsert) froze the website's counters at zero and reset them on
          // every author edit.
          pack.vote_count ?? 0,
        )
        .run();

      // Replace tags
      const tagStmts = [
        env.ROSTER_HUB_DB.prepare("DELETE FROM pack_tags WHERE pack_id = ?").bind(pack.id),
        ...pack.tags.map((tag) =>
          env.ROSTER_HUB_DB!.prepare("INSERT OR IGNORE INTO pack_tags (pack_id, tag) VALUES (?, ?)").bind(pack.id, tag),
        ),
      ];
      await env.ROSTER_HUB_DB.batch(tagStmts);
    } else {
      await env.ROSTER_HUB_DB.batch([
        env.ROSTER_HUB_DB.prepare("DELETE FROM pack_tags WHERE pack_id = ?").bind(pack.id),
        env.ROSTER_HUB_DB.prepare("DELETE FROM packs WHERE id = ?").bind(pack.id),
      ]);
    }
  } catch (err) {
    console.error(`D1 sync failed [${pack.id}]:`, err);
  }
}

/**
 * Mirror a counter bump into D1. The vote endpoint goes through the DO rather
 * than d1UpsertPack, so without this the website's vote counts never move.
 * Best-effort, like the other D1 writes.
 */
async function d1UpdateVoteCount(env: Env, id: string, voteCount: number): Promise<void> {
  if (!env.ROSTER_HUB_DB) return;
  try {
    await env.ROSTER_HUB_DB
      .prepare("UPDATE packs SET vote_count = ? WHERE id = ?")
      .bind(voteCount, id)
      .run();
  } catch (err) {
    console.error(`D1 vote_count sync failed [${id}]:`, err);
  }
}

async function d1DeletePack(env: Env, id: string): Promise<void> {
  if (!env.ROSTER_HUB_DB) return;
  try {
    await env.ROSTER_HUB_DB.batch([
      env.ROSTER_HUB_DB.prepare("DELETE FROM pack_tags WHERE pack_id = ?").bind(id),
      env.ROSTER_HUB_DB.prepare("DELETE FROM packs WHERE id = ?").bind(id),
    ]);
  } catch (err) {
    console.error(`D1 delete failed [${id}]:`, err);
  }
}

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

/**
 * The single cache key the default landing view is stored under.
 *
 * The incoming URL for that view varies (`sort` and `page` may be omitted, or
 * spelled in either order) but the Cache API matches on the full URL including
 * the query string, so caching under the request URL and deleting a bare
 * "/packs" never lined up — mutations silently failed to invalidate. Every
 * match/put/delete goes through this one canonical key instead.
 */
function defaultViewCacheKey(url: URL): Request {
  return new Request(new URL("/packs?default=1", url.origin));
}

/** Purge the CDN-cached pack list after a mutation. Exported so tests can
 *  reset the shared cache between cases. */
export async function invalidatePackListCache(url: URL): Promise<void> {
  await caches.default.delete(defaultViewCacheKey(url));
}

/** Get the singleton PackIndexDO stub for atomic index mutations. */
function getPackIndexDO(env: Env) {
  const id = env.PACK_INDEX.idFromName("singleton");
  return env.PACK_INDEX.get(id);
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

  // Only the default landing view is cacheable: no filters, page 1, and the
  // client's default sort (pack-constants.ts sends sort=votes&page=1). Every
  // spelling of that view shares one canonical cache key — see
  // defaultViewCacheKey.
  const sortParam = url.searchParams.get("sort");
  const pageParam = url.searchParams.get("page");
  const isDefaultView =
    !hasFilters &&
    (pageParam === null || pageParam === "1") &&
    (sortParam === null || sortParam === "votes");

  // Resolve the viewer up front: draft/all filtering, the author filter,
  // anonymity redaction and user_voted all key off it, and whether the shared
  // cache may be used depends on it. Free when no Authorization header is
  // present (validateBearerToken returns null without an upstream call).
  const viewer = await validateBearerToken(request);
  const viewerId = viewer ? String(viewer.id) : undefined;

  // Only an anonymous, origin-less request may read or populate the shared
  // entry: an authed response carries that viewer's user_voted (and possibly
  // their own anonymous packs), and corsHeaders echoes the caller's Origin, so
  // either would be replayed to the wrong caller. The desktop client — the
  // only consumer today — sends neither.
  const isSharedCacheable =
    isDefaultView && viewerId === undefined && request.headers.get("Origin") === null;

  if (isSharedCacheable) {
    const cached = await cache.match(defaultViewCacheKey(url));
    if (cached) return cached;
  }

  const index = await getPackIndex(env);
  if (!index) {
    return json(request, { packs: [], page: 1, sort: sortParam ?? "updated" }, 200, 30);
  }

  let packs = index.packs;

  const statusFilter = url.searchParams.get("status");
  const authorFilter = url.searchParams.get("author");

  // Status filter — default to "published"; draft/all require auth + ownership
  if (statusFilter === "all" || statusFilter === "draft") {
    if (viewerId === undefined) {
      packs = packs.filter((p) => (p.status ?? "published") === "published");
    } else if (statusFilter === "draft") {
      packs = packs.filter(
        (p) => p.author_id === viewerId && (p.status ?? "published") === "draft",
      );
    } else {
      packs = packs.filter(
        (p) => (p.status ?? "published") === "published" || p.author_id === viewerId,
      );
    }
  } else {
    const target = statusFilter ?? "published";
    packs = packs.filter((p) => (p.status ?? "published") === target);
  }

  if (authorFilter) {
    // Anonymous packs must not be discoverable by author — that filter is a
    // de-anonymization vector. Only the author themselves (authed) sees them.
    packs = packs.filter(
      (p) => p.author_id === authorFilter && (!p.is_anonymous || viewerId === authorFilter),
    );
  }

  const typeFilter = url.searchParams.get("type");
  if (typeFilter) {
    packs = packs.filter((p) => p.pack_type === typeFilter);
  }

  const tagFilter = url.searchParams.get("tag");
  if (tagFilter) {
    packs = packs.filter((p) => p.tags.includes(tagFilter));
  }

  const query = url.searchParams.get("q")?.slice(0, 200).toLowerCase();
  if (query) {
    packs = packs.filter(
      (p) =>
        p.title.toLowerCase().includes(query) ||
        p.description.toLowerCase().includes(query),
    );
  }

  // Sort. The client (pack-constants.ts SortOption) sends votes|newest|updated;
  // popular/installs are kept for backward compatibility.
  const sort = sortParam ?? "updated";
  if (sort === "votes" || sort === "popular") {
    packs.sort((a, b) => b.vote_count - a.vote_count);
  } else if (sort === "installs") {
    packs.sort((a, b) => b.install_count - a.install_count);
  } else if (sort === "newest") {
    packs.sort((a, b) => b.created_at.localeCompare(a.created_at));
  } else {
    // "updated" (and default) — sort by updated_at descending
    packs.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  }

  // Paginate
  const page = Math.max(1, parseInt(url.searchParams.get("page") ?? "1", 10) || 1);
  const start = (page - 1) * PACKS_PER_PAGE;
  const paginated = packs.slice(start, start + PACKS_PER_PAGE);

  // Enforce anonymity at the edge. A shared-cacheable response is always fully
  // redacted regardless of who populated the cache; the owner exception only
  // applies to responses served to one identified viewer.
  const redacted = paginated.map((p) =>
    redactAnonymousPack(p, isSharedCacheable ? undefined : viewerId),
  );

  // Tell the viewer which of these they have already voted on. Without it the
  // client renders every pack as unvoted and its toggle deletes real votes.
  const votedIds =
    viewerId === undefined
      ? null
      : await getVotedPackIds(env, viewerId, paginated.map((p) => p.id));
  const visible: PackView[] = votedIds
    ? redacted.map((p) => ({ ...p, user_voted: votedIds.has(p.id) }))
    : redacted;

  const response = json(request, { packs: visible, page, sort }, 200, 30);

  if (isSharedCacheable && request.method === "GET") {
    cache.put(defaultViewCacheKey(url), response.clone()).catch(console.error);
  }

  return response;
}

// ── GET /packs/:id ─────────────────────────────────────────────────
async function handleGetPack(request: Request, env: Env, id: string): Promise<Response> {
  const pack = await getPack(env, id);
  if (!pack) {
    return notFound(request);
  }

  const user = await validateBearerToken(request);
  const viewerId = user ? String(user.id) : undefined;

  if (pack.status === "draft" && viewerId !== pack.author_id) {
    return notFound(request);
  }

  if (viewerId !== undefined) {
    // A viewer-specific response: it carries their user_voted, and for the
    // author it carries the real fields of their own anonymous pack. Never
    // cacheable.
    const voted = (await getVote(env, id, viewerId)) !== null;
    const view: PackView = { ...redactAnonymousPack(pack, viewerId), user_voted: voted };
    return json(request, { pack: view }, 200, 0);
  }

  // Anonymous viewer: the redacted pack is identical for everyone, so it stays
  // safe to cache.
  return json(request, { pack: redactAnonymousPack(pack) }, 200, 300, "public");
}

// ── POST /packs ────────────────────────────────────────────────────
async function handleCreatePack(request: Request, env: Env, url: URL): Promise<Response> {
  const user = await validateBearerToken(request);
  if (!user) {
    return unauthorized(request);
  }

  const parsed = await readJsonBody(request);
  if (!parsed.ok) {
    return parsed.reason === "too-large"
      ? json(request, { error: "Request body is too large" }, 413)
      : badRequest(request, [{ field: "body", message: "Invalid JSON" }]);
  }

  const errors = validatePack(parsed.body);
  if (errors.length > 0) {
    return badRequest(request, errors);
  }

  const input = parsed.body as Record<string, unknown>;
  const userId = String(user.id);

  // Generate ID from title if not provided
  let id = typeof input.id === "string" && input.id.length > 0
    ? input.id
    : slugify(input.title as string);

  // A title with no ASCII alphanumerics (CJK, Cyrillic, emoji) slugifies to
  // "", which every /packs/:id route rejects — the pack would be listed but
  // unreachable, un-editable and un-deletable forever.
  if (!id) {
    id = `pack-${Date.now().toString(36)}`;
  }

  // Ensure unique (fresh read so a recently-created id isn't missed)
  const existing = await getPack(env, id, { fresh: true });
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
    addons: sanitizeAddons(input.addons),
    tags: input.tags as string[],
    vote_count: 0,
    install_count: 0,
    created_at: now,
    updated_at: now,
    // Honour the requested status. Hardcoding "draft" made the client's
    // Publish flow report success for a pack nobody — including the author's
    // own browse view and the D1 mirror — could ever see.
    status: (input.status as PackStatus) ?? "draft",
  };

  // The index goes first: the per-user cap is enforced inside the DO, which is
  // the only place that reads the index uncached and single-threaded, so a
  // rejected create must not have written a pack body already.
  const MAX_PACKS_PER_USER = 25;
  const added = await getPackIndexDO(env).addPack(pack, MAX_PACKS_PER_USER);
  if (!added.ok) {
    return json(
      request,
      { error: `Maximum of ${MAX_PACKS_PER_USER} packs reached. Delete some packs to create new ones.` },
      429,
    );
  }
  await putPack(env, pack);

  await invalidatePackListCache(url);
  await d1UpsertPack(env, pack);

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

  // Fresh read: this handler carries vote_count/install_count forward from
  // `existing`, so a stale cached snapshot would revert recent counter changes.
  const existing = await getPack(env, id, { fresh: true });
  if (!existing) {
    return notFound(request);
  }

  if (!existing.author_id || String(user.id) !== existing.author_id) {
    return json(request, { error: "Only the pack creator can update it" }, 403);
  }

  const parsed = await readJsonBody(request);
  if (!parsed.ok) {
    return parsed.reason === "too-large"
      ? json(request, { error: "Request body is too large" }, 413)
      : badRequest(request, [{ field: "body", message: "Invalid JSON" }]);
  }

  const errors = validatePack(parsed.body);
  if (errors.length > 0) {
    return badRequest(request, errors);
  }

  const input = parsed.body as Record<string, unknown>;

  const pack: Pack = {
    id,
    title: input.title as string,
    description: input.description as string,
    pack_type: input.pack_type as PackType,
    author_id: existing.author_id,
    author_name: existing.author_name,
    is_anonymous: Boolean(input.is_anonymous),
    addons: sanitizeAddons(input.addons),
    tags: input.tags as string[],
    vote_count: existing.vote_count,
    install_count: existing.install_count,
    created_at: existing.created_at,
    updated_at: new Date().toISOString(),
    status: (input.status as PackStatus) ?? existing.status ?? "published",
  };

  await putPack(env, pack);
  await getPackIndexDO(env).updatePack(id, pack);

  await invalidatePackListCache(url);
  await d1UpsertPack(env, pack);

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

  // Fresh read so a just-created pack isn't seen as missing and ownership is
  // checked against current data.
  const existing = await getPack(env, id, { fresh: true });
  if (!existing) {
    return notFound(request);
  }

  if (!existing.author_id || String(user.id) !== existing.author_id) {
    return json(request, { error: "Only the pack creator can delete it" }, 403);
  }

  await env.ESO_PACKS.delete(`pack:${id}`);
  await getPackIndexDO(env).removePack(id);
  // Slugs become available again once a pack is deleted, so its vote records
  // must go with it — otherwise a pack that reuses the id inherits them and a
  // previous voter's first vote is treated as an unvote.
  await deleteVotesForPack(env, id);

  await invalidatePackListCache(url);
  await d1DeletePack(env, id);

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
  await getPackIndexDO(env).replaceIndex(index);

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
    return notFound(request);
  }

  // Drafts are hidden from everyone but their author, so this endpoint must
  // not answer for one either — the auth-distinguishable 401-vs-404 otherwise
  // confirms a hidden slug exists. Checked before any auth work runs.
  if ((pack.status ?? "published") !== "published") {
    return notFound(request);
  }

  const user = await validateBearerToken(request);
  if (!user) {
    return json(request, { error: "Sign in to vote" }, 401);
  }
  const userId = String(user.id);

  // Toggle the record and apply the counter delta in one serialized step
  // inside the DO. Deciding vote-vs-unvote out here read the record through
  // KV's edge cache, so a rapid vote/unvote could see the same stale state
  // twice and inflate vote_count permanently. The DO also syncs the per-pack
  // KV detail from its own fresh copy.
  const { voted, pack: updated } = await getPackIndexDO(env).toggleVote(id, userId, pack);

  await invalidatePackListCache(url);

  const voteCount =
    updated?.vote_count ?? Math.max(0, (pack.vote_count ?? 0) + (voted ? 1 : -1));
  await d1UpdateVoteCount(env, id, voteCount);

  const response: VoteResponse = { voted, voteCount };
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
    return notFound(request);
  }

  // This endpoint needs no auth at all, so without a status gate anyone who
  // guessed a draft's slug could bump its install_count and read the count
  // back — mutating and disclosing a pack the API otherwise denies exists.
  if ((pack.status ?? "published") !== "published") {
    return notFound(request);
  }

  // Rate limit: one install track per IP per pack per hour
  const ip = request.headers.get("CF-Connecting-IP") ?? "unknown";
  const rateLimitKey = `install-rate:${id}:${ip}`;
  const existing = await env.ESO_PACKS.get(rateLimitKey);
  if (existing) {
    return json(request, { installCount: pack.install_count ?? 0 });
  }
  await env.ESO_PACKS.put(rateLimitKey, "1", { expirationTtl: 3600 });

  // Increment inside the DO (fresh, single-threaded) instead of writing back a
  // possibly-stale cached snapshot, which would lose concurrent installs and
  // revert recent author edits. The DO also syncs the per-pack KV detail.
  const updated = await getPackIndexDO(env).bumpPackCounter(id, "install_count", 1, pack);

  await invalidatePackListCache(url);

  const installCount = updated?.install_count ?? (pack.install_count ?? 0) + 1;
  return json(request, { installCount });
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

  // Surface scheduled-backup health so monitoring can detect a silently
  // failing cron even with Workers observability disabled.
  let lastBackupAt: string | null = null;
  let lastBackupOk = false;
  try {
    const meta = await env.ESO_PACKS.get<BackupMeta>("backup:meta", "json");
    if (meta?.last_success) {
      lastBackupAt = new Date(meta.last_success).toISOString();
      // Cron runs daily; allow slack for a missed/delayed run before flagging
      // the backup as stale.
      lastBackupOk = Date.now() - meta.last_success < 36 * 3600 * 1000;
    }
  } catch {
    // backup:meta read failed — leave last_backup_at null / last_backup_ok false
  }

  return json(request, {
    status: kvOk ? "ok" : "degraded",
    kv: kvOk,
    packCount,
    last_backup_at: lastBackupAt,
    last_backup_ok: lastBackupOk,
    timestamp: new Date().toISOString(),
  });
}

// ── Scheduled backup ──────────────────────────────────────────────

/**
 * Full-corpus snapshot shape written to `backup:YYYY-MM-DD` / `backup:latest`.
 * `packs` mirrors the legacy index-only backup shape (an array of full Pack
 * objects) for backward compatibility with anything that reads old daily
 * backups; `packBodies` keys the same objects by id so restore can replay the
 * per-key `pack:<id>` records, and `votes` carries every `vote:<id>:<user>`
 * record so votes survive a restore too.
 */
interface PackBackupSnapshot {
  created_at: string;
  packs: Pack[];
  packBodies: Record<string, Pack>;
  votes: Record<string, VoteRecord>;
}

interface BackupMeta {
  last_success: number;
  last_backup_key: string;
  pack_count: number;
  pack_body_count: number;
  vote_count: number;
}

// Warn (not fail) if the snapshot is approaching KV's 25MB per-value limit.
const BACKUP_SIZE_WARN_BYTES = 20 * 1024 * 1024;

async function handleScheduled(env: Env): Promise<void> {
  // Fresh (uncached) read so the snapshot reflects the latest mutation rather
  // than a stale up-to-60s-cached index.
  const index = await getPackIndex(env, { fresh: true });
  if (!index || index.packs.length === 0) return;

  const timestamp = new Date().toISOString().slice(0, 10); // YYYY-MM-DD
  const backupKey = `backup:${timestamp}`;

  // Skip only if today's backup is FULLY complete — i.e. the daily key
  // exists AND backup:meta already points at it. If an isolate is
  // interrupted after the daily key is written but before backup:latest /
  // backup:meta, a same-day retry must not early-return here, or those two
  // keys permanently lag a day behind. Re-writing the daily key below is
  // idempotent, so it's safe to just fall through and redo the rest.
  const existing = await env.ESO_PACKS.get(backupKey);
  if (existing) {
    const metaRaw = await env.ESO_PACKS.get("backup:meta");
    const meta = metaRaw ? (JSON.parse(metaRaw) as BackupMeta) : null;
    if (meta?.last_backup_key === backupKey) return;
  }

  // Enumerate the full corpus so the backup is actually restorable rather
  // than index-only. Neither half may fan out one KV get per record: Workers
  // cap subrequests per invocation, and once packs + votes crossed that cap
  // the cron threw every night and durable backups silently stopped. Pack
  // bodies come straight from the index (which already carries full Pack
  // objects) and votes come from their keys' list() metadata.
  const packBodies: Record<string, Pack> = Object.fromEntries(
    index.packs.map((p): [string, Pack] => [p.id, p]),
  );
  const votes = await listAllVotes(env);

  const snapshot: PackBackupSnapshot = {
    created_at: new Date().toISOString(),
    packs: index.packs,
    packBodies,
    votes,
  };

  const serialized = JSON.stringify(snapshot);
  if (serialized.length > BACKUP_SIZE_WARN_BYTES) {
    console.warn(
      `Backup snapshot for ${backupKey} is ${serialized.length} bytes, approaching KV's 25MB value limit`,
    );
  }

  // Write backup with 90-day TTL (keeps last ~90 daily snapshots)
  await env.ESO_PACKS.put(backupKey, serialized, {
    expirationTtl: 90 * 86400,
  });

  // Also write a non-expiring "latest" pointer so a silent multi-day failure
  // gap can't erase all history once the 90-day-old daily snapshots roll off.
  await env.ESO_PACKS.put("backup:latest", serialized);

  const meta: BackupMeta = {
    last_success: Date.now(),
    last_backup_key: backupKey,
    pack_count: index.packs.length,
    pack_body_count: Object.keys(packBodies).length,
    vote_count: Object.keys(votes).length,
  };
  await env.ESO_PACKS.put("backup:meta", JSON.stringify(meta));

  console.log(
    `Backup written: ${backupKey} (${index.packs.length} packs, ${meta.pack_body_count} bodies, ${meta.vote_count} votes)`,
  );
}

// ── POST /admin/restore ────────────────────────────────────────────

/**
 * Records restored per call, and how many of those writes run at once.
 *
 * A restore used to walk the whole snapshot in one request, awaiting each write
 * on its own and strictly serialized. A corpus of any size therefore ran into
 * the per-request subrequest ceiling — and there was no way to resume, so the
 * endpoint simply stopped working at exactly the scale where an incident
 * recovery matters.
 *
 * `RESTORE_CONCURRENCY` only affects wall-clock; it does not change how many
 * subrequests a page spends.
 */
/**
 * Worst-case binding calls one restored record costs. Cloudflare counts every
 * KV/D1/DO binding call against the same per-request subrequest ceiling as
 * `fetch`, so this is what actually bounds a page:
 *
 * - a published pack: `putPack` (1 KV) + `d1UpsertPack` (the upsert, then the
 *   tag batch) = 3
 * - a draft pack: `putPack` + one D1 batch = 2
 * - a vote: `restoreVote` writes both `vote:` and the user index = 2
 *
 * Derive the caps from this rather than picking a round number: a page cap of
 * 400 was ~1200 subrequests in production, comfortably over the ceiling, which
 * is the failure the paging was added to avoid in the first place.
 */
export const SUBREQUESTS_PER_RECORD = 3;
/** Per-request subrequest ceiling on Workers Paid. */
export const SUBREQUEST_CEILING = 1000;
/** Held back for the backup read, the fresh index read, the DO index swap and
 *  the cache purge — everything a page does outside the record loop. */
export const SUBREQUEST_RESERVE = 100;

export const RESTORE_MAX_PAGE_SIZE = Math.floor(
  (SUBREQUEST_CEILING - SUBREQUEST_RESERVE) / SUBREQUESTS_PER_RECORD,
);
/** Default page: half the cap, so an operator who passes no limit stays well
 *  clear of the ceiling even if the per-record cost grows. */
const RESTORE_PAGE_SIZE = Math.floor(RESTORE_MAX_PAGE_SIZE / 2);
const RESTORE_CONCURRENCY = 10;

/**
 * A cursor's position, or 0 for "start from the beginning".
 *
 * Deliberately does NOT clamp to the work length. Clamping a too-large cursor
 * down to the end made `start === end`, so the call wrote nothing yet still took
 * the final-page branch and republished the index — advertising pack bodies it
 * had never written. Out-of-range is now an error, not a silent no-op.
 */
function readCursor(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) return 0;
  return Math.floor(value);
}

/**
 * Fingerprint of the snapshot AND the exact cursor a page was issued for.
 *
 * Not a security token — it is an incident-recovery consistency check, behind
 * the admin API key. `created_at` catches a snapshot rewritten in place (the
 * midnight cron overwriting `backup:latest`), the record count catches a
 * different snapshot with the same timestamp, and `cursor` stops a token from
 * validating a position it was never issued for: a snapshot-wide token let any
 * in-range cursor through, so a mistyped offset could skip whole pages and the
 * final page would still publish an index for bodies that were never replayed.
 *
 * It catches ACCIDENTS, not reconstruction. The value is plaintext and its
 * derivation is right here, so a caller who decides to skip pages can recompute
 * a matching token for any in-range cursor and the corpus ends up advertising
 * bodies that were never written.
 *
 * HMAC does not fix that, which is why it is not used: the only secret this
 * worker holds is `ADMIN_API_KEY`, and every caller who can reach this endpoint
 * already presents it. Signing with a secret the forger holds buys nothing.
 * Real tamper-evidence needs continuation state the CALLER does not own —
 * server-side issued cursors — which is the server-owned restore job already
 * recorded as the follow-up on this PR, together with staged writes and an
 * atomic promote. Until that exists, an operator must finish a restore they
 * start, and the 409 says so rather than inviting a token edit.
 */
function restoreToken(
  backupKey: string,
  snapshot: PackBackupSnapshot,
  total: number,
  cursor: number,
): string {
  return `${backupKey}|${snapshot.created_at ?? "unknown"}|${total}|${cursor}`;
}

function clampLimit(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
    return RESTORE_PAGE_SIZE;
  }
  // At least 1. A fractional limit (0 < limit < 1) floored to 0, which made
  // `end === start`: the page wrote nothing and returned the same cursor with
  // done:false, so a caller looping until done never advanced.
  return Math.min(Math.max(1, Math.floor(value)), RESTORE_MAX_PAGE_SIZE);
}

/** `backup:latest` or `backup:YYYY-MM-DD` — the only keys a restore may read. */
const BACKUP_KEY_SHAPE = /^backup:(latest|\d{4}-\d{2}-\d{2})$/;

/**
 * The snapshot a continuation refers to, read back out of its own token.
 *
 * A paged restore of a DATED snapshot was impossible without this. The response
 * carried `cursor` and `token` but not `date`, and the docs said to pass the
 * response straight back — so the next call fell through to `backup:latest`,
 * compared it against a token minted for `backup:YYYY-MM-DD`, and 409'd. Making
 * the token carry the snapshot identity means one field round-trips instead of
 * three, and a resume cannot silently retarget a different backup.
 *
 * Shape-checked rather than trusted: the token picks which KV key gets read, and
 * even behind the admin key that should not be arbitrary.
 */
function backupKeyFromToken(token: unknown): string | null {
  if (typeof token !== "string") return null;
  const key = token.split("|")[0] ?? "";
  return BACKUP_KEY_SHAPE.test(key) ? key : null;
}

/** Run `tasks` with at most `concurrency` in flight, preserving fail-fast. */
async function runBounded(tasks: (() => Promise<void>)[], concurrency: number): Promise<void> {
  let next = 0;
  const workers = Array.from({ length: Math.min(concurrency, tasks.length) }, async () => {
    while (next < tasks.length) {
      const index = next++;
      await tasks[index]!();
    }
  });
  await Promise.all(workers);
}
/**
 * Restore the pack corpus from a `backup:YYYY-MM-DD` (or `backup:latest`)
 * snapshot written by the scheduled backup (handleScheduled). Unlike
 * /admin/seed this is a production incident-recovery tool, so it is gated
 * only behind requireAuth (the admin API key) — NOT env.ALLOW_SEED, which
 * exists specifically to disable seed-with-fake-data in production and
 * would defeat the purpose of a restore endpoint if reused here.
 *
 * Replays pack bodies (+ D1 mirror) and vote records directly to KV, then
 * atomically replaces the index via the PackIndexDO — never via raw
 * putPackIndex, which would race a concurrent mutation (see kv.ts's
 * getPackIndex comment on why counter/index writes go through the DO). The
 * replacement index is built from only the ids we actually restored a body
 * for (so drifted "ghost" ids in snapshot.packs don't reappear) unioned with
 * any pack in the current live index that predates or postdates the
 * snapshot entirely (so a pack created after the backup isn't deleted).
 *
 * Paged. A call restores `RESTORE_PAGE_SIZE` records by default, or up to
 * `RESTORE_MAX_PAGE_SIZE` — twice that — when the caller passes `limit`. Quote
 * the max, not the default, when reasoning about the subrequest budget: the
 * default is deliberately half the cap, so checking headroom against it hides
 * the factor of two an operator gets just by passing `limit`. If more records
 * remain, returns `{ done: false, cursor, token }` for the operator to pass
 * straight back in the next request body — the endpoint is a manual incident
 * tool, so a caller-driven cursor beats a background job that can fail
 * unobserved. The token both names the snapshot and binds the cursor to it, so
 * a continuation needs only `{ cursor, token }` even for a dated backup, and
 * resuming against a snapshot that changed underneath is a 409, never a partial
 * restore. The
 * index swap and cache invalidation happen only on the final page, so a restore
 * abandoned half-way leaves the previous index in place rather than publishing a
 * partial corpus. Pages are idempotent, so replaying one after a failure is
 * safe. A snapshot that fits in a single page behaves exactly as before.
 */
async function handleRestore(request: Request, env: Env, url: URL): Promise<Response> {
  if (!requireAuth(request, env)) {
    return unauthorized(request);
  }

  let dateInput: unknown;
  let cursorInput: unknown;
  let limitInput: unknown;
  let tokenInput: unknown;
  try {
    const body = (await request.json()) as Record<string, unknown> | null;
    dateInput = body?.date;
    cursorInput = body?.cursor;
    limitInput = body?.limit;
    tokenInput = body?.token;
  } catch {
    // No/invalid JSON body — fall back to backup:latest below.
  }

  // A continuation names its snapshot through the token it was issued with, so
  // resuming a dated restore does not depend on the caller also re-sending
  // `date`. `date` selects the snapshot for the FIRST page only.
  const resumeKey = readCursor(cursorInput) > 0 ? backupKeyFromToken(tokenInput) : null;
  const backupKey =
    resumeKey ??
    (typeof dateInput === "string" && /^\d{4}-\d{2}-\d{2}$/.test(dateInput)
      ? `backup:${dateInput}`
      : "backup:latest");

  const raw = await env.ESO_PACKS.get(backupKey);
  if (!raw) {
    return notFound(request, `No backup snapshot found for "${backupKey}"`);
  }

  let snapshot: PackBackupSnapshot;
  try {
    snapshot = JSON.parse(raw) as PackBackupSnapshot;
  } catch {
    return json(request, { error: `Backup "${backupKey}" is corrupt` }, 500);
  }

  // Older backups (pre-packBodies) only carry the index-mirroring `packs`
  // array — rebuild the per-id map from it so restore still works on them.
  const packBodies =
    snapshot.packBodies && Object.keys(snapshot.packBodies).length > 0
      ? snapshot.packBodies
      : Object.fromEntries((snapshot.packs ?? []).map((p): [string, Pack] => [p.id, p]));

  const packs = Object.values(packBodies);
  const votes = snapshot.votes ?? {};
  const voteRecords = Object.values(votes);

  // One flat, deterministically ordered work list so a cursor means the same
  // position on every call against the same snapshot.
  const work: (() => Promise<void>)[] = [
    ...packs.map((pack) => async () => {
      await putPack(env, pack);
      await d1UpsertPack(env, pack);
    }),
    // Use each record's own packId/userId fields rather than parsing the
    // "<packId>:<userId>" map key, since userId could itself contain ":".
    ...voteRecords.map((record) => async () => {
      await restoreVote(env, record.packId, record.userId, record);
    }),
  ];

  // A cursor only means anything against the snapshot that issued it. Two ways
  // it can go stale: the daily cron overwrites `backup:latest` at midnight UTC,
  // so a paged restore straddling midnight would silently change snapshots
  // mid-run; or an operator re-runs an old cursor by hand. Either way the
  // numeric offset then points somewhere else in a different work list, the
  // records before it are never replayed, and the final page still publishes an
  // index listing every pack in the snapshot — so the corpus ends up advertising
  // pack bodies that were never written. Bind the cursor to its snapshot and
  // refuse a mismatch rather than resuming into the wrong list.
  const start = readCursor(cursorInput);
  if (start > 0 && tokenInput !== restoreToken(backupKey, snapshot, work.length, start)) {
    // Deliberately does NOT hand back the token it expected. A snapshot-wide
    // token let any in-range cursor pass, so echoing the correct one invited an
    // operator to retry their WRONG cursor with the RIGHT token — skipping every
    // page in between while the final page still published the whole index.
    return json(
      request,
      {
        error:
          "Restore cursor and token do not match — the token was issued for a different cursor, " +
          "or the snapshot changed mid-restore. Start again with no cursor.",
      },
      409,
    );
  }
  // `>=`, not `>`. A cursor exactly equal to `total` slices to an empty page and
  // then falls straight into the final-page branch, republishing the index for
  // records this call never wrote — the same hazard as an over-long cursor, and
  // easy to hit by copying `total` out of the response instead of `cursor`.
  // `start === 0` is always legitimate: it is a fresh restore, including of an
  // empty snapshot.
  if (start > 0 && start >= work.length) {
    return json(
      request,
      {
        error:
          `Restore cursor ${start} is not inside this snapshot (${work.length} records). ` +
          "A completed restore reports done:true — pass the returned cursor, not the total.",
      },
      409,
    );
  }

  const limit = clampLimit(limitInput);
  const end = Math.min(start + limit, work.length);
  await runBounded(work.slice(start, end), RESTORE_CONCURRENCY);

  // Not done yet: hand back a cursor and stop BEFORE touching the index. Every
  // pack write is an idempotent put, so a page replayed after a network failure
  // is harmless.
  //
  // Deferring the index swap is NOT a rollback, and this comment used to imply
  // it was. Each page has already written pack bodies to KV and mirrored them
  // into D1, and `/packs/:id` reads the KV body directly rather than going
  // through the index — so an abandoned restore leaves a corpus that is
  // genuinely part-old, part-new, with the website mirror updated too. Holding
  // the index back only avoids ADDING entries for bodies that were never
  // written; it cannot un-write the ones that were.
  //
  // That property predates the paging (the single-request version wrote every
  // pack the same way before swapping the index) but paging makes abandonment
  // far more likely, since stopping between pages is now a normal thing to do.
  // Fixing it properly means a server-owned job with staged writes and an
  // atomic promote — see the follow-up note on the PR. Until then an operator
  // must finish a restore they start.
  if (end < work.length) {
    return json(request, {
      ok: true,
      done: false,
      cursor: end,
      // Minted for THIS cursor, not the snapshot at large. Pass the pair back
      // together: a token only validates the offset it was issued for, so a
      // mistyped cursor is refused instead of silently skipping the pages
      // between.
      token: restoreToken(backupKey, snapshot, work.length, end),
      total: work.length,
      restored_packs: Math.min(end, packs.length) - Math.min(start, packs.length),
      restored_votes: Math.max(0, end - Math.max(start, packs.length)),
    });
  }

  // Rebuild the index from only the packs we actually have bodies for (drops
  // "ghost" entries that are in snapshot.packs but absent from packBodies —
  // exactly the index/per-key drift this backup's packBodies capture exists
  // to repair), then union in any pack from the CURRENT live index that isn't
  // part of this snapshot at all, so packs created after the backup was taken
  // aren't deleted by the restore.
  // Fresh read: a pack created inside the 60s cache window would otherwise be
  // absent from `preservedPacks` and dropped from the rebuilt index — which is
  // exactly what the preservation above promises not to do, and restore runs
  // at incident time when recent writes are most likely in flight.
  const restoredIds = new Set(Object.keys(packBodies));
  const liveIndex = await getPackIndex(env, { fresh: true });
  const preservedPacks = (liveIndex?.packs ?? []).filter((p) => !restoredIds.has(p.id));
  await getPackIndexDO(env).replaceIndex({ packs: [...packs, ...preservedPacks] });

  await invalidatePackListCache(url);

  return json(request, {
    ok: true,
    done: true,
    cursor: null,
    total: work.length,
    restored_packs: packs.length - Math.min(start, packs.length),
    restored_votes: voteRecords.length - Math.max(0, start - packs.length),
  });
}

// ── DELETE /account ────────────────────────────────────────────

/**
 * Scrub a deleted user's records out of the non-expiring `backup:latest`
 * snapshot.
 *
 * The daily `backup:YYYY-MM-DD` snapshots carry a 90-day TTL, so a deleted
 * user's data ages out of those on its own. `backup:latest` is deliberately
 * written WITHOUT a TTL (it is the floor that survives a >90-day backup gap),
 * so without this it would retain the packs and votes of a user who asked for
 * deletion — indefinitely, and invisibly to them. Rewriting this one key bounds
 * the retention of deleted data to the dailies' 90-day window, which is what
 * PRIVACY.md commits to.
 *
 * Mirrors handleDeleteAccount's treatment of live data exactly: it drops the
 * user's own packs and their own votes, and deliberately does NOT drop votes
 * other users cast on those packs (live deletion leaves those in place, and a
 * restore must not silently discard other people's records).
 *
 * Best-effort. The live data is already gone by the time this runs, so a
 * failure here must not fail the deletion request — it is logged and swallowed.
 * A concurrent scheduled backup cannot reintroduce the user, because it
 * snapshots the live index, which no longer contains them.
 */
async function purgeUserFromLatestBackup(env: Env, userId: string): Promise<void> {
  try {
    const raw = await env.ESO_PACKS.get("backup:latest");
    if (!raw) return;

    const snapshot = JSON.parse(raw) as PackBackupSnapshot;

    const keptPacks = (snapshot.packs ?? []).filter((p) => p.author_id !== userId);
    const keptBodies: Record<string, Pack> = {};
    for (const [id, pack] of Object.entries(snapshot.packBodies ?? {})) {
      if (pack?.author_id !== userId) keptBodies[id] = pack;
    }
    const keptVotes: Record<string, VoteRecord> = {};
    for (const [key, vote] of Object.entries(snapshot.votes ?? {})) {
      if (vote?.userId !== userId) keptVotes[key] = vote;
    }

    const removed =
      (snapshot.packs?.length ?? 0) - keptPacks.length +
      (Object.keys(snapshot.packBodies ?? {}).length - Object.keys(keptBodies).length) +
      (Object.keys(snapshot.votes ?? {}).length - Object.keys(keptVotes).length);
    if (removed === 0) return; // nothing of theirs in the snapshot — skip the write

    const scrubbed: PackBackupSnapshot = {
      created_at: snapshot.created_at,
      packs: keptPacks,
      packBodies: keptBodies,
      votes: keptVotes,
    };
    await env.ESO_PACKS.put("backup:latest", JSON.stringify(scrubbed));
    console.log(`Purged ${removed} record(s) for deleted user from backup:latest`);
  } catch (err) {
    console.error("Failed to purge deleted user from backup:latest:", err);
  }
}

async function handleDeleteAccount(request: Request, env: Env, url: URL): Promise<Response> {
  const user = await validateBearerToken(request);
  if (!user) return unauthorized(request);

  const userId = String(user.id);

  // 1. Find and delete all user's packs.
  //
  // The DO is the authority here, not our own index read: it runs unconditionally
  // (it is cheap) and reports the ids it actually removed, so a pack created
  // moments before the deletion request — invisible to even a fresh cached read —
  // still has its body and D1 rows removed. Without that, the only pack of a
  // brand-new account could survive deletion entirely.
  const index = await getPackIndex(env, { fresh: true });
  const snapshotIds = index?.packs.filter((p) => p.author_id === userId).map((p) => p.id) ?? [];
  const removedIds = await getPackIndexDO(env).removePacksByAuthor(userId);
  const packIds = [...new Set([...snapshotIds, ...removedIds])];

  // Delete individual pack KV entries
  for (const packId of packIds) {
    await env.ESO_PACKS.delete(`pack:${packId}`);
  }

  // Batch-delete from D1
  if (packIds.length > 0 && env.ROSTER_HUB_DB) {
    try {
      const stmts = packIds.flatMap((packId) => [
        env.ROSTER_HUB_DB!.prepare("DELETE FROM pack_tags WHERE pack_id = ?").bind(packId),
        env.ROSTER_HUB_DB!.prepare("DELETE FROM packs WHERE id = ?").bind(packId),
      ]);
      await env.ROSTER_HUB_DB.batch(stmts);
    } catch (err) {
      console.error("D1 batch delete failed:", err);
    }
  }

  // 2. Delete all user's votes via reverse index (user-votes:{userId}:{packId})
  // Does not decrement vote_count — denormalized aggregates, acceptable for rare deletion.
  let voteCount = 0;
  let voteCursor: string | undefined;
  do {
    const list = await env.ESO_PACKS.list({ prefix: `user-votes:${userId}:`, cursor: voteCursor });
    for (const key of list.keys) {
      const packId = key.name.slice(`user-votes:${userId}:`.length);
      if (packId) {
        await env.ESO_PACKS.delete(`vote:${packId}:${userId}`);
      }
      await env.ESO_PACKS.delete(key.name);
      voteCount++;
    }
    voteCursor = list.list_complete ? undefined : list.cursor;
  } while (voteCursor);

  // 3. Delete all user's share codes
  let shareCount = 0;
  let shareCursor: string | undefined;
  do {
    const list = await env.ESO_PACKS.list({ prefix: `share-user:${userId}:`, cursor: shareCursor });
    for (const key of list.keys) {
      // Extract the share code from key format: share-user:{userId}:{code}
      const parts = key.name.split(":");
      const code = parts[parts.length - 1];
      if (code) {
        await env.ESO_PACKS.delete(`share:${code}`);
      }
      await env.ESO_PACKS.delete(key.name);
      shareCount++;
    }
    shareCursor = list.list_complete ? undefined : list.cursor;
  } while (shareCursor);

  if (packIds.length > 0) {
    await invalidatePackListCache(url);
  }

  // 4. Scrub them from the one backup key that never expires. The dated
  // snapshots keep their 90-day TTL and age out on their own.
  await purgeUserFromLatestBackup(env, userId);

  return json(request, {
    deleted: {
      packs: packIds.length,
      votes: voteCount,
      shares: shareCount,
    },
  });
}

// ── Router ─────────────────────────────────────────────────────────
export default {
  // `_ctx` is declared even though nothing uses it: the runtime always passes an
  // ExecutionContext, and the tests call these handlers directly with one. Omit
  // it and the object literal's own signature is 2-arity, so every test call is
  // a type error — which is what the stale test/tsconfig.json was hiding.
  async fetch(request: Request, env: Env, _ctx: ExecutionContext): Promise<Response> {
    try {
      return await handleRequest(request, env);
    } catch (err) {
      console.error("Unhandled error:", err);
      return new Response(JSON.stringify({ error: "Internal server error" }), {
        status: 500,
        headers: { "Content-Type": "application/json", ...corsHeaders(request) },
      });
    }
  },

  async scheduled(
    _controller: ScheduledController,
    env: Env,
    _ctx: ExecutionContext,
  ): Promise<void> {
    try {
      await handleScheduled(env);
    } catch (err) {
      console.error("Scheduled backup failed:", err);
      // Observability may be disabled in production, so persist a durable
      // breadcrumb — otherwise a failing cron is invisible until /health's
      // last_backup_ok staleness check trips up to ~36h later.
      try {
        await env.ESO_PACKS.put(
          "backup:last_error",
          JSON.stringify({
            at: new Date().toISOString(),
            message: err instanceof Error ? err.message : String(err),
          }),
        );
      } catch (writeErr) {
        console.error("Failed to record backup:last_error:", writeErr);
      }
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

  // Rate limiting via built-in atomic binding (skipped when no IP, i.e., in tests)
  const ip = request.headers.get("CF-Connecting-IP");
  // An authenticated admin call is exempt. The limiters exist to bound anonymous
  // abuse, and /admin/* already requires the shared admin key — but WRITE_LIMITER
  // allows only 10 writes a minute, and a paged restore is now one POST per page.
  // A corpus over ~10 pages would 429 partway through and never reach the final
  // page that swaps the index, breaking the large-corpus recovery the paging was
  // built for. Auth is checked here rather than assumed: a caller WITHOUT a valid
  // key is still rate-limited, so this cannot be used to bypass the limiter.
  const isAuthedAdmin = pathname.startsWith("/admin/") && requireAuth(request, env);
  if (ip && !isAuthedAdmin) {
    const isVote = pathname.endsWith("/vote") || pathname.endsWith("/install");
    const isWrite = method === "POST" || method === "PUT" || method === "DELETE";
    const limiter = isVote ? env.VOTE_LIMITER : isWrite ? env.WRITE_LIMITER : env.READ_LIMITER;
    const { success } = await limiter.limit({ key: ip });
    if (!success) {
      return new Response(JSON.stringify({ error: "Too many requests" }), {
        status: 429,
        headers: { "Content-Type": "application/json", ...corsHeaders(request) },
      });
    }
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
    if (!id || id.includes("/") || !/^[a-z0-9-]+$/.test(id) || id.length > 100) {
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

  // POST /admin/restore — incident-recovery restore from a backup snapshot
  if (method === "POST" && pathname === "/admin/restore") {
    return handleRestore(request, env, url);
  }

  // DELETE /account — delete all user data (GDPR / data portability)
  if (method === "DELETE" && pathname === "/account") {
    return handleDeleteAccount(request, env, url);
  }

  return notFound(request);
}
