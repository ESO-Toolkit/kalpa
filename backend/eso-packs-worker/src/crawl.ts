import type { Env, CrawlOutcome } from "./types";
import {
  applyDetail,
  ensureSchema,
  getMeta,
  markRemoved,
  pendingDetailUids,
  setMeta,
  sweepUnseen,
  upsertMetaBatch,
  type AddonMetaRow,
} from "./addon-index";

/**
 * Populates the addon index from ESOUI's public JSON API.
 *
 * Two endpoints, deliberately used in the cheapest possible ratio:
 *
 *   filelist.json     — the entire catalogue (~4000 entries) in ONE request,
 *                       but metadata only. No descriptions.
 *   filedetails/{id}  — one request per addon, and the only source of
 *                       `description`.
 *
 * So the daily job is: one bulk request, diff `lastUpdate` against what we
 * stored, and fetch details ONLY for entries that actually changed — typically
 * a few dozen a day. The expensive part is the initial backfill, which is a
 * one-time ~4000-request walk driven from an admin route, paged and throttled.
 *
 * On CLAUDE.md's "keep all scraping in esoui.rs" rule: that rule is about the
 * desktop client, and this does not scrape — it uses the same public JSON API
 * (api.mmoui.com) that esoui.rs already calls, from the server, once, on behalf
 * of every user instead of each user separately. Net ESOUI load goes DOWN,
 * because search stops hitting esoui.com/downloads/search.php per keystroke.
 */

const API_BASE = "https://api.mmoui.com/v4/game/ESO";

/** Identifies the crawler to ESOUI so they can see who we are and contact us. */
const USER_AGENT = "Kalpa-PackHub-Indexer/1.0 (+https://github.com/ESO-Toolkit/Kalpa)";

/**
 * Refuse to tombstone the catalogue when a "successful" bulk fetch returns
 * implausibly few entries.
 *
 * `fetchFilelist` throwing is handled — but a 200 response carrying `[]` or a
 * truncated list is not an error, and would tombstone everything. That is not
 * self-correcting either: removal deletes the FTS rows, and while the next sync
 * un-tombstones the addons, they only become searchable again after their
 * descriptions are re-fetched. A bad five minutes upstream would cost a full
 * re-crawl.
 */
const MIN_ENTRIES_RATIO = 0.5;

/** Below this, ratio checks are meaningless — a small index is normal early on. */
const SANITY_FLOOR_MIN_LIVE = 50;

/** Titles and authors go into the FTS index AND the Ask prompt, where a newline
 *  would forge extra candidate lines. Descriptions are already flattened and
 *  capped by stripMarkup; these were not. */
const MAX_TITLE_LENGTH = 200;

/** Ceiling on one backfill page. Workers cap outbound subrequests per
 *  invocation, and each addon costs one fetch plus a few D1 statements, so a
 *  page has to stay well clear of that limit. */
export const MAX_DETAIL_BATCH = 40;

/** Delay between `filedetails` calls inside a page. ~4 req/s sustained is
 *  polite for a one-off walk of a public API that exists to be read by Minion. */
const DETAIL_DELAY_MS = 250;

const FETCH_TIMEOUT_MS = 15_000;

/** Collapse to a single line and cap. Not stripMarkup: titles are plain text
 *  upstream, and running them through markup stripping would eat legitimate
 *  angle brackets in names. */
export function flattenField(value: unknown, fallback = ""): string {
  if (typeof value !== "string") return fallback;
  const flat = value.replace(/\s+/g, " ").trim();
  if (flat.length === 0) return fallback;
  return flat.length > MAX_TITLE_LENGTH ? flat.slice(0, MAX_TITLE_LENGTH).trimEnd() : flat;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function apiFetch(path: string): Promise<Response> {
  return fetch(`${API_BASE}/${path}`, {
    headers: { "User-Agent": USER_AGENT, Accept: "application/json" },
    signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
  });
}

// ── Upstream shapes (v4, camelCase — mirrors ApiFileEntry in esoui.rs) ──────

interface ApiFileEntry {
  id: number;
  categoryId?: number;
  lastUpdate?: number;
  title?: string;
  author?: string;
  fileInfoUri?: string;
  downloads?: number;
  downloadsMonthly?: number;
  favorites?: number;
  library?: boolean;
}

interface ApiFileDetail {
  id: number;
  description?: string;
}

interface ApiCategory {
  /** v4 sends this as a STRING ("17") on categorylist.json, but as a number on
   *  filelist.json. A `typeof === "number"` guard here silently dropped every
   *  category and left every addon with a blank category name. */
  id: number | string;
  title?: string;
}

/** Accept either representation and reject anything that is not a real id. */
function toCategoryId(value: unknown): number | null {
  const n = typeof value === "number" ? value : Number.parseInt(String(value ?? ""), 10);
  return Number.isFinite(n) && n > 0 ? n : null;
}

// ── Text normalisation ─────────────────────────────────────────────────────

/**
 * BBCode tags: `[b]`, `[/b]`, `[size=3]`, `[URL="https://..."]`.
 *
 * The attribute value is frequently a full URL, so it cannot be capped tightly
 * — the previous 60-character limit meant `[URL="https://www.esoui.com/..."]`
 * was left in verbatim, leaking markup into search snippets and into the Ask
 * prompt. Bounded at 300 and forbidden from spanning lines so it cannot run
 * away across a whole description.
 */
const BBCODE_PATTERN = new RegExp(
  // [b] [/b] [size=3] [URL="https://..."] and the bare list marker [*].
  String.raw`\[(?:\*|/?[a-z][a-z0-9]*(?:=[^\]\n]{0,300})?)\]`,
  "gi",
);

/**
 * Bare URLs left in description prose.
 *
 * These are not markup, so tag-stripping never touched them, and they were
 * indexed verbatim. A language-flag icon URL like
 * `http://icons.iconarchive.com/icons/iconscity/flags/32/usa-icon.png` injects
 * the tokens `icons`, `flags`, `usa` and `png` into an addon's searchable text,
 * so a search for "flagged in combat" matched addons that merely displayed a
 * flag image. 468 of ~2900 descriptions carried at least one.
 */
const BARE_URL_PATTERN = new RegExp(String.raw`(?:https?://|www\.)\S+`, "gi");

const ENTITIES: Record<string, string> = {
  amp: "&",
  lt: "<",
  gt: ">",
  quot: '"',
  apos: "'",
  nbsp: " ",
  "#39": "'",
  "#x27": "'",
  "#160": " ",
};

/**
 * ESOUI descriptions are author-written HTML with BBCode remnants. They go into
 * an FTS index and, later, into an LLM prompt, so they get flattened to plain
 * text here rather than at read time.
 *
 * This is also the prompt-injection boundary: an addon author can write
 * "ignore previous instructions" in their description and we will happily index
 * it. Stripping markup does not neutralise that — what neutralises it is that
 * the Ask route constrains the model to a closed set of candidate IDs and
 * rebuilds every link server-side. This function only has to make the text
 * searchable and bounded.
 */
export function stripMarkup(input: string, maxLength = 2000): string {
  const withoutTags = input
    // <br> and block ends become spaces so words either side do not fuse into
    // one token ("combat</b><b>indicator" must not index as one word).
    .replace(/<\s*br\s*\/?\s*>/gi, " ")
    .replace(/<\/\s*(p|div|li|tr|h[1-6])\s*>/gi, " ")
    .replace(/<[^>]*>/g, "")
    // BBCode survives in older descriptions. The attribute value is often a
    // full URL, so it must not be length-capped tightly — a [URL="https://..."]
    // tag routinely exceeds 60 characters and was being left in verbatim,
    // leaking markup into snippets and into the Ask prompt.
    .replace(BBCODE_PATTERN, " ")
    .replace(BARE_URL_PATTERN, " ");

  const decoded = withoutTags.replace(/&([a-z]+|#x?[0-9a-f]+);/gi, (match, name: string) => {
    const key = name.toLowerCase();
    if (key in ENTITIES) return ENTITIES[key];
    const numeric = /^#x([0-9a-f]+)$/i.exec(key) ?? /^#(\d+)$/.exec(key);
    if (numeric) {
      const code = Number.parseInt(numeric[1], /^#x/i.test(key) ? 16 : 10);
      if (Number.isFinite(code) && code > 0 && code < 0x10ffff) {
        return String.fromCodePoint(code);
      }
    }
    return " ";
  });

  const collapsed = decoded.replace(/\s+/g, " ").trim();
  return collapsed.length > maxLength ? `${collapsed.slice(0, maxLength).trimEnd()}…` : collapsed;
}

// ── Fetch helpers ──────────────────────────────────────────────────────────

export async function fetchFilelist(): Promise<ApiFileEntry[]> {
  const response = await apiFetch("filelist.json");
  if (!response.ok) {
    throw new Error(`filelist.json returned ${response.status}`);
  }
  const parsed: unknown = await response.json();
  if (!Array.isArray(parsed)) {
    throw new Error("filelist.json did not return an array");
  }
  return parsed.filter(
    (entry): entry is ApiFileEntry =>
      typeof entry === "object" && entry !== null && typeof (entry as ApiFileEntry).id === "number",
  );
}

export async function fetchCategories(): Promise<Map<number, string>> {
  const map = new Map<number, string>();
  try {
    const response = await apiFetch("categorylist.json");
    if (!response.ok) return map;
    const parsed: unknown = await response.json();
    if (!Array.isArray(parsed)) return map;
    for (const raw of parsed as ApiCategory[]) {
      const id = toCategoryId(raw?.id);
      if (id !== null && typeof raw.title === "string") {
        map.set(id, flattenField(raw.title));
      }
    }
  } catch {
    // Category names are a ranking nicety, not a correctness requirement — an
    // index built without them still searches title and description fine.
  }
  return map;
}

/**
 * Fetch one addon's description.
 *
 * Returns `null` for a 404, which the caller treats as "removed upstream".
 * The v4 endpoint wraps its result in a single-element array (same quirk
 * esoui.rs documents on ApiFileDetail).
 */
export async function fetchDetail(uid: number): Promise<string | null> {
  const response = await apiFetch(`filedetails/${uid}.json`);
  if (response.status === 404) return null;
  if (!response.ok) {
    throw new Error(`filedetails/${uid} returned ${response.status}`);
  }
  const parsed: unknown = await response.json();
  const detail: ApiFileDetail | undefined = Array.isArray(parsed)
    ? (parsed[0] as ApiFileDetail | undefined)
    : (parsed as ApiFileDetail);
  if (!detail || typeof detail !== "object") return null;
  return stripMarkup(typeof detail.description === "string" ? detail.description : "");
}

// ── Sync passes ────────────────────────────────────────────────────────────

/**
 * Pass 1: reconcile the bulk list into `addons`.
 *
 * Cheap (one upstream request) and safe to run daily. Marks entries that
 * disappeared upstream as removed, which is the staleness that actually matters
 * — recommending an addon that was pulled is worse than recommending one whose
 * version moved.
 */
export async function syncFilelist(db: D1Database): Promise<{
  seen: number;
  removed: number;
  queued: number;
}> {
  await ensureSchema(db);

  const [entries, categories] = await Promise.all([fetchFilelist(), fetchCategories()]);

  // Every row written by this run carries the same stamp, so "not seen in this
  // run" is later expressible as `indexed_at < now` — no per-addon bookkeeping,
  // and no id list to bind.
  const now = Date.now();

  const rows: AddonMetaRow[] = entries.map((entry) => ({
    uid: entry.id,
    title: flattenField(entry.title, `Addon ${entry.id}`),
    author: flattenField(entry.author),
    categoryId: toCategoryId(entry.categoryId) ?? 0,
    categoryName: categories.get(toCategoryId(entry.categoryId) ?? -1) ?? "",
    downloads: typeof entry.downloads === "number" ? entry.downloads : 0,
    downloadsMonthly: typeof entry.downloadsMonthly === "number" ? entry.downloadsMonthly : 0,
    favorites: typeof entry.favorites === "number" ? entry.favorites : 0,
    isLibrary: entry.library === true,
    fileInfoUri:
      typeof entry.fileInfoUri === "string" && entry.fileInfoUri.startsWith("https://")
        ? entry.fileInfoUri
        : `https://www.esoui.com/downloads/info${entry.id}.html`,
    lastUpdate: typeof entry.lastUpdate === "number" ? entry.lastUpdate : 0,
  }));

  // Deduplicate by uid. A multi-row upsert that names the same primary key twice
  // in one statement fails ("ON CONFLICT DO UPDATE command cannot affect row a
  // second time"), so one duplicated upstream entry would abort the whole sync.
  const deduped = [...new Map(rows.map((row) => [row.uid, row])).values()];

  // Sanity floor before any destructive step.
  const liveRow = await db
    .prepare("SELECT COUNT(*) AS n FROM addons WHERE removed = 0")
    .first<{ n: number }>();
  const liveBefore = liveRow?.n ?? 0;
  if (liveBefore >= SANITY_FLOOR_MIN_LIVE && deduped.length < liveBefore * MIN_ENTRIES_RATIO) {
    throw new Error(
      `filelist.json returned ${deduped.length} entries against ${liveBefore} live addons; ` +
        `refusing to sync (looks truncated). Re-run when upstream recovers.`,
    );
  }

  await upsertMetaBatch(db, deduped, now);

  // Only reached if the bulk fetch above resolved, so a network failure throws
  // earlier and cannot be mistaken for "the entire catalogue was deleted".
  const removed = await sweepUnseen(db, now);

  const pending = await pendingDetailUids(db, 1);
  await setMeta(db, "last_sync", new Date(now).toISOString());

  return { seen: deduped.length, removed, queued: pending.length };
}

/**
 * Pass 2: fetch descriptions for up to `limit` addons that need one.
 *
 * Bounded per call so it fits a Worker invocation. The caller repeats until
 * `remaining` is 0 — the daily cron does one page (a normal day queues far
 * fewer than a page), and the initial backfill is driven page-by-page from a
 * maintainer's machine.
 */
export async function crawlDetails(db: D1Database, limit: number): Promise<CrawlOutcome> {
  await ensureSchema(db);

  const batch = Math.min(Math.max(limit, 1), MAX_DETAIL_BATCH);
  const uids = await pendingDetailUids(db, batch);

  let fetched = 0;
  let removed = 0;
  let failed = 0;
  const now = Date.now();

  for (const [position, uid] of uids.entries()) {
    if (position > 0) await sleep(DETAIL_DELAY_MS);
    try {
      const description = await fetchDetail(uid);
      if (description === null) {
        await markRemoved(db, [uid]);
        removed++;
        continue;
      }
      const row = await db
        .prepare("SELECT category_name FROM addons WHERE uid = ?")
        .bind(uid)
        .first<{ category_name: string }>();
      await applyDetail(db, uid, description, row?.category_name ?? "", now);
      fetched++;
    } catch (err) {
      // A single bad entry must not abort the page — the next run retries it,
      // because detail_stale is only cleared by a successful applyDetail.
      console.error(`filedetails[${uid}] failed:`, err);
      failed++;
    }
  }

  const stillPending = await pendingDetailUids(db, MAX_DETAIL_BATCH);

  return {
    fetched,
    removed,
    failed,
    remaining: stillPending.length,
    // `remaining` is capped by the probe above, so it answers "is there more
    // work?" not "how much". `complete` is the flag callers should loop on.
    complete: stillPending.length === 0,
  };
}

/** Is the nightly crawl switched on for this deployment? */
export function syncEnabled(env: Env): boolean {
  return env.ADDON_INDEX_SYNC === "enabled";
}

/**
 * Daily job: reconcile the bulk list, then spend one page on descriptions.
 *
 * No-ops unless BOTH the index binding exists and the sync var is explicitly
 * "enabled". Binding-only is the normal state between provisioning the database
 * and finishing the backfill, and during that window the cron must stay quiet.
 */
export async function runDailySync(env: Env): Promise<void> {
  const db = env.ADDON_INDEX;
  if (!db || !syncEnabled(env)) return;
  const summary = await syncFilelist(db);
  const outcome = await crawlDetails(db, MAX_DETAIL_BATCH);
  console.log(
    `addon-index sync: seen=${summary.seen} removed=${summary.removed} ` +
      `details=${outcome.fetched} failed=${outcome.failed} remaining=${outcome.remaining}`,
  );
}

/**
 * Rows re-cleaned per call. Each costs an UPDATE plus an FTS delete+insert
 * (3 queries), so 150 rows is 450 — comfortably inside D1's 1000-per-invocation
 * budget with room for the read.
 */
const REPROCESS_PAGE = 150;

/**
 * Re-run the text pipeline over descriptions ALREADY stored, with no upstream
 * traffic at all.
 *
 * `stripMarkup` is a pure function of the raw text, but what is stored is its
 * OUTPUT — so when the pipeline gains a rule (bare-URL removal, `[*]` markers),
 * every previously-indexed row keeps the old contamination. Re-fetching ~4000
 * descriptions from ESOUI to fix our own parser would be rude and slow; the
 * stored text still contains the offending substrings, so re-applying the
 * current pipeline to it is enough.
 *
 * Paged and resumable via an `index_meta` cursor. Safe to re-run: rows whose
 * text does not change are skipped, so a second pass is nearly free.
 */
export async function reprocessDescriptions(
  db: D1Database,
  limit = REPROCESS_PAGE,
): Promise<{ scanned: number; changed: number; complete: boolean }> {
  await ensureSchema(db);

  const cursor = Number.parseInt((await getMeta(db, "reprocess_cursor")) ?? "0", 10) || 0;
  const page = Math.min(Math.max(limit, 1), REPROCESS_PAGE);

  const rows = await db
    .prepare(
      `SELECT uid, title, author, category_name, description
         FROM addons
        WHERE uid > ? AND removed = 0 AND detail_stale = 0
        ORDER BY uid
        LIMIT ?`,
    )
    .bind(cursor, page)
    .all<{
      uid: number;
      title: string;
      author: string;
      category_name: string;
      description: string;
    }>();

  const results = rows.results ?? [];
  if (results.length === 0) {
    // A finished pass resets the cursor so the next one starts from the top.
    await setMeta(db, "reprocess_cursor", "0");
    return { scanned: 0, changed: 0, complete: true };
  }

  let changed = 0;
  for (const row of results) {
    const cleaned = stripMarkup(row.description);
    if (cleaned === row.description) continue;
    // applyDetail rewrites both the stored text and the FTS row, and is the
    // single place that knows how to build one.
    await applyDetail(db, row.uid, cleaned, row.category_name, Date.now());
    changed++;
  }

  await setMeta(db, "reprocess_cursor", String(results[results.length - 1].uid));
  return { scanned: results.length, changed, complete: false };
}
