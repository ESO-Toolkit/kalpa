import type { Env, Pack, PackType, PackStatus, VoteRecord, VoteResponse } from "./types";
import {
  getPack,
  getPackIndex,
  putPack,
  getVote,
  putVote,
  deleteVote,
  restoreVote,
  listAllPackBodies,
  listAllVotes,
} from "./kv";
import { corsHeaders, handlePreflight } from "./cors";
import { validatePack } from "./validate";
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
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, datetime('now'), datetime('now'))
           ON CONFLICT(id) DO UPDATE SET
             title = excluded.title,
             description = excluded.description,
             pack_type = excluded.pack_type,
             addons = excluded.addons,
             is_anonymous = excluded.is_anonymous,
             author_name = excluded.author_name,
             updated_at = datetime('now')`,
        )
        .bind(
          pack.id,
          pack.author_id,
          pack.author_name,
          pack.is_anonymous ? 1 : 0,
          pack.title,
          pack.description,
          pack.pack_type,
          addonsJson,
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

/** Purge the CDN-cached pack list after a mutation. */
async function invalidatePackListCache(url: URL): Promise<void> {
  const cacheKey = new URL("/packs", url.origin);
  await caches.default.delete(new Request(cacheKey));
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
  // client's default sort. The client always sends sort=votes&page=1 for that
  // view (pack-constants.ts), so match that exact request shape for both the
  // read and the write below — otherwise the keys never align and we either
  // never hit or cache a non-canonical response.
  const sortParam = url.searchParams.get("sort");
  const pageParam = url.searchParams.get("page");
  const isDefaultView =
    !hasFilters &&
    (pageParam === null || pageParam === "1") &&
    (sortParam === null || sortParam === "votes");

  if (isDefaultView) {
    const cached = await cache.match(request);
    if (cached) return cached;
  }

  const index = await getPackIndex(env);
  if (!index) {
    return json(request, { packs: [], page: 1, sort: sortParam ?? "updated" }, 200, 30);
  }

  let packs = index.packs;

  // Status filter — default to "published"; draft/all require auth + ownership
  const statusFilter = url.searchParams.get("status");
  if (statusFilter === "all" || statusFilter === "draft") {
    const user = await validateBearerToken(request);
    if (!user) {
      packs = packs.filter((p) => (p.status ?? "published") === "published");
    } else {
      const userId = String(user.id);
      if (statusFilter === "draft") {
        packs = packs.filter((p) => p.author_id === userId && (p.status ?? "published") === "draft");
      } else {
        packs = packs.filter((p) => (p.status ?? "published") === "published" || p.author_id === userId);
      }
    }
  } else {
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

  const response = json(request, { packs: paginated, page, sort }, 200, 30);

  if (isDefaultView && request.method === "GET") {
    cache.put(request, response.clone()).catch(console.error);
  }

  return response;
}

// ── GET /packs/:id ─────────────────────────────────────────────────
async function handleGetPack(request: Request, env: Env, id: string): Promise<Response> {
  const pack = await getPack(env, id);
  if (!pack) {
    return notFound(request);
  }
  if (pack.status === "draft") {
    const user = await validateBearerToken(request);
    if (!user || String(user.id) !== pack.author_id) {
      return notFound(request);
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
    addons: input.addons as Pack["addons"],
    tags: input.tags as string[],
    vote_count: 0,
    install_count: 0,
    created_at: now,
    updated_at: now,
    status: "draft",
  };

  await putPack(env, pack);
  await getPackIndexDO(env).addPack(pack);

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

  const user = await validateBearerToken(request);
  if (!user) {
    return json(request, { error: "Sign in to vote" }, 401);
  }
  const userId = String(user.id);

  const existingVote = await getVote(env, id, userId);
  let voted: boolean;
  let delta: number;

  if (existingVote) {
    await deleteVote(env, id, userId);
    voted = false;
    delta = -1;
  } else {
    await putVote(env, id, userId);
    voted = true;
    delta = 1;
  }

  // Mutate the counter inside the DO (fresh, single-threaded) so we neither
  // lose concurrent votes nor revert a recent author edit by writing back a
  // stale cached snapshot. The DO also syncs the per-pack KV detail.
  const updated = await getPackIndexDO(env).bumpPackCounter(id, "vote_count", delta, pack);

  await invalidatePackListCache(url);

  const voteCount = updated?.vote_count ?? Math.max(0, (pack.vote_count ?? 0) + delta);
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
 * backups; `packBodies` and `votes` additionally capture the per-key
 * `pack:<id>` and `vote:<id>:<user>` records directly (via KV `list()`) so the
 * backup is restorable even if the index and per-key data were ever to drift.
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

  // Enumerate the full corpus — the index already carries full pack fields,
  // but capture the per-key pack bodies and votes too so the backup is
  // actually restorable rather than just an index-only snapshot.
  const [packBodies, votes] = await Promise.all([
    listAllPackBodies(env),
    listAllVotes(env),
  ]);

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
 * Restore the pack corpus from a `backup:YYYY-MM-DD` (or `backup:latest`)
 * snapshot written by the scheduled backup (handleScheduled). Unlike
 * /admin/seed this is a production incident-recovery tool, so it is gated
 * only behind requireAuth (the admin API key) — NOT env.ALLOW_SEED, which
 * exists specifically to disable seed-with-fake-data in production and
 * would defeat the purpose of a restore endpoint if reused here.
 *
 * Replays every pack body (+ D1 mirror) and vote record directly to KV,
 * then atomically replaces the index via the PackIndexDO — never via raw
 * putPackIndex, which would race a concurrent mutation (see kv.ts's
 * getPackIndex comment on why counter/index writes go through the DO). The
 * replacement index is built from only the ids we actually restored a body
 * for (so drifted "ghost" ids in snapshot.packs don't reappear) unioned with
 * any pack in the current live index that predates or postdates the
 * snapshot entirely (so a pack created after the backup isn't deleted).
 */
async function handleRestore(request: Request, env: Env, url: URL): Promise<Response> {
  if (!requireAuth(request, env)) {
    return unauthorized(request);
  }

  let dateInput: unknown;
  try {
    const body = (await request.json()) as Record<string, unknown> | null;
    dateInput = body?.date;
  } catch {
    // No/invalid JSON body — fall back to backup:latest below.
  }

  const backupKey =
    typeof dateInput === "string" && /^\d{4}-\d{2}-\d{2}$/.test(dateInput)
      ? `backup:${dateInput}`
      : "backup:latest";

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
  for (const pack of packs) {
    await putPack(env, pack);
    await d1UpsertPack(env, pack);
  }

  const votes = snapshot.votes ?? {};
  for (const record of Object.values(votes)) {
    // Use the record's own packId/userId fields rather than parsing the
    // "<packId>:<userId>" map key, since userId could itself contain ":".
    await restoreVote(env, record.packId, record.userId, record);
  }

  // Rebuild the index from only the packs we actually have bodies for (drops
  // "ghost" entries that are in snapshot.packs but absent from packBodies —
  // exactly the index/per-key drift this backup's packBodies capture exists
  // to repair), then union in any pack from the CURRENT live index that isn't
  // part of this snapshot at all, so packs created after the backup was taken
  // aren't deleted by the restore.
  const restoredIds = new Set(Object.keys(packBodies));
  const liveIndex = await getPackIndex(env);
  const preservedPacks = (liveIndex?.packs ?? []).filter((p) => !restoredIds.has(p.id));
  await getPackIndexDO(env).replaceIndex({ packs: [...packs, ...preservedPacks] });

  await invalidatePackListCache(url);

  return json(request, {
    ok: true,
    restored_packs: packs.length,
    restored_votes: Object.keys(votes).length,
  });
}

// ── DELETE /account ────────────────────────────────────────────
async function handleDeleteAccount(request: Request, env: Env, url: URL): Promise<Response> {
  const user = await validateBearerToken(request);
  if (!user) return unauthorized(request);

  const userId = String(user.id);

  // 1. Find and delete all user's packs
  const index = await getPackIndex(env);
  const userPacks = index?.packs.filter((p) => p.author_id === userId) ?? [];

  // Delete individual pack KV entries
  for (const pack of userPacks) {
    await env.ESO_PACKS.delete(`pack:${pack.id}`);
  }

  // Batch-remove from DO index in a single read-write cycle
  if (userPacks.length > 0) {
    await getPackIndexDO(env).removePacksByAuthor(userId);
  }

  // Batch-delete from D1
  if (userPacks.length > 0 && env.ROSTER_HUB_DB) {
    try {
      const stmts = userPacks.flatMap((p) => [
        env.ROSTER_HUB_DB!.prepare("DELETE FROM pack_tags WHERE pack_id = ?").bind(p.id),
        env.ROSTER_HUB_DB!.prepare("DELETE FROM packs WHERE id = ?").bind(p.id),
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

  if (userPacks.length > 0) {
    await invalidatePackListCache(url);
  }

  return json(request, {
    deleted: {
      packs: userPacks.length,
      votes: voteCount,
      shares: shareCount,
    },
  });
}

// ── Router ─────────────────────────────────────────────────────────
export default {
  async fetch(request: Request, env: Env): Promise<Response> {
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

  async scheduled(_controller: ScheduledController, env: Env): Promise<void> {
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
  if (ip) {
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
