import type { Env, AddonIndexStats, AddonSearchHit, AddonSearchResult } from "./types";

/**
 * Full-text index over the ESOUI addon catalogue.
 *
 * Kalpa's Discover search hits `esoui.com/downloads/search.php`, which matches
 * on titles only. That is why "in combat indicator" finds nothing useful even
 * though both CombatIndicator and FightingDisplay say exactly that in their
 * descriptions — the descriptions are never indexed, because the bulk
 * `filelist.json` endpoint does not carry them (only `filedetails/{id}` does).
 *
 * This module owns a dedicated D1 database (binding `ADDON_INDEX`). It is
 * deliberately NOT in `roster-hub-db`: those tables are shared with the ESO
 * Toolkit website and CLAUDE.md requires every schema change there to be
 * coordinated. An index that only Kalpa reads has no business in a shared
 * database.
 */

/** Bump when the schema or the text pipeline changes in a way that invalidates
 *  stored rows. Read by the Ask cache key so answers do not outlive their
 *  corpus. */
export const INDEX_VERSION = 1;

const MAX_QUERY_LENGTH = 200;
const MAX_LIMIT = 50;
const DEFAULT_LIMIT = 25;
const SNIPPET_TOKENS = 24;

/**
 * BM25 column weights, in the FTS5 column order below.
 *
 * Title is weighted hardest because an addon whose *name* is "CombatIndicator"
 * is a better answer than one that merely mentions combat in prose. Author is
 * low: matching it is almost always incidental, except when someone searches
 * for an author by name deliberately, which still works because nothing else
 * will match.
 *
 * Category is deliberately NOT high. ESOUI has a literal "Combat Mods"
 * category, so at weight 3 every one of its ~150 addons outscored a
 * description that actually explained combat behaviour — the category word
 * flooded the results for any query term that doubles as a category name
 * (combat, chat, map, guild, crafting, pvp).
 */
const BM25_WEIGHTS = "10.0, 1.5, 1.5, 1.0";

/**
 * Ranking expression: BM25 relevance with a small popularity prior.
 *
 * bm25() returns negative values, more negative being better, so subtracting a
 * positive bonus promotes a row.
 *
 * The prior exists because BM25 alone does not discriminate inside a cluster.
 * Measured on the live index for the query "combat": ranks 1-12 spanned scores
 * 4.123 to 4.008 — a 2.8% band — while their download counts spanned 197 to
 * 28,114, a 140x range. The top hit had 399 downloads; "Combat Indicator"
 * (28,114 downloads, the canonical answer) sat at rank 9, and an addon titled
 * "DEPRECATED: ..." outranked it. Within that band the ordering is noise.
 *
 * A roughly logarithmic bucket ladder rather than log10(): **D1 does not
 * authorize log10** ("not authorized to use function: log10"), and a stored
 * precomputed column would need an ALTER TABLE on a live index. Buckets need
 * neither, and they are easier to reason about and tune than a curve.
 *
 * The maximum nudge is 0.15 — the same order as the 0.115 noise band measured
 * above, and far smaller than the ~2.0 gap between a title match and a
 * description-only match. So it reorders ties without letting a popular addon
 * outrank a genuinely better textual match.
 */
const POPULARITY_LADDER = `CASE
         WHEN a.downloads >= 20000 THEN 0.15
         WHEN a.downloads >= 10000 THEN 0.12
         WHEN a.downloads >= 5000  THEN 0.09
         WHEN a.downloads >= 1000  THEN 0.06
         WHEN a.downloads >= 200   THEN 0.03
         ELSE 0
       END`;
const RANK_EXPRESSION = `bm25(addons_fts, ${BM25_WEIGHTS}) - (${POPULARITY_LADDER}) ASC`;

/**
 * ESOUI category 157, "Discontinued & Outdated" — 981 of ~4170 addons, roughly
 * a quarter of the catalogue.
 *
 * Excluded from search and from Ask candidates by default. These are addons
 * their authors have retired; surfacing one as the answer to "is there an addon
 * that..." is worse than returning nothing, because it looks like a live
 * recommendation. They stay indexed so a direct lookup by name still finds them
 * via `includeDiscontinued`.
 */
export const DISCONTINUED_CATEGORY_ID = 157;

const SCHEMA_STATEMENTS = [
  `CREATE TABLE IF NOT EXISTS addons (
     uid INTEGER PRIMARY KEY,
     title TEXT NOT NULL,
     author TEXT NOT NULL DEFAULT '',
     category_id INTEGER NOT NULL DEFAULT 0,
     category_name TEXT NOT NULL DEFAULT '',
     description TEXT NOT NULL DEFAULT '',
     downloads INTEGER NOT NULL DEFAULT 0,
     downloads_monthly INTEGER NOT NULL DEFAULT 0,
     favorites INTEGER NOT NULL DEFAULT 0,
     is_library INTEGER NOT NULL DEFAULT 0,
     file_info_uri TEXT NOT NULL DEFAULT '',
     last_update INTEGER NOT NULL DEFAULT 0,
     detail_fetched_at INTEGER,
     detail_stale INTEGER NOT NULL DEFAULT 1,
     removed INTEGER NOT NULL DEFAULT 0,
     indexed_at INTEGER NOT NULL DEFAULT 0
   )`,
  // The backfill and the daily delta both ask the same question: "which live
  // rows still need a description?" Without this they degrade to a full scan
  // on every page of a ~4000-row walk.
  `CREATE INDEX IF NOT EXISTS idx_addons_pending
     ON addons (detail_stale, removed, downloads DESC)`,
  `CREATE INDEX IF NOT EXISTS idx_addons_removed ON addons (removed)`,
  // Non-external-content FTS5: rows are written twice (once here, once in
  // `addons`). At ~4000 rows the duplication is irrelevant, and it avoids the
  // trigger-synchronisation failure mode of `content=` tables, where a partial
  // migration silently leaves the index and the table disagreeing.
  //
  // rowid is pinned to the ESOUI uid so updates are a delete-then-insert by
  // primary key rather than a search.
  `CREATE VIRTUAL TABLE IF NOT EXISTS addons_fts USING fts5(
     title, author, category, description,
     tokenize = 'porter unicode61 remove_diacritics 2'
   )`,
  `CREATE TABLE IF NOT EXISTS index_meta (
     key TEXT PRIMARY KEY,
     value TEXT NOT NULL
   )`,
] as const;

/**
 * True when a query failed because the index has never been built.
 *
 * A freshly provisioned D1 has no tables until the first sync runs, and the
 * read paths must treat that as "nothing indexed yet" rather than as an error —
 * otherwise the very first deploy serves 500s from `/addons/search` and
 * `/addons/stats` until someone runs a crawl.
 */
export function isMissingTable(err: unknown): boolean {
  const message = err instanceof Error ? err.message : String(err);
  return /no such table/i.test(message);
}

/** Idempotent. Cheap enough to call before any indexing operation. */
export async function ensureSchema(db: D1Database): Promise<void> {
  for (const statement of SCHEMA_STATEMENTS) {
    await db.prepare(statement).run();
  }
}

export async function getMeta(db: D1Database, key: string): Promise<string | null> {
  const row = await db
    .prepare("SELECT value FROM index_meta WHERE key = ?")
    .bind(key)
    .first<{ value: string }>();
  return row?.value ?? null;
}

export async function setMeta(db: D1Database, key: string, value: string): Promise<void> {
  await db
    .prepare(
      `INSERT INTO index_meta (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value`,
    )
    .bind(key, value)
    .run();
}

/**
 * FTS5's query language treats `"`, `*`, `:`, `^`, `-`, `NEAR`, `AND`, `OR` and
 * unbalanced parens as syntax. A raw user question fed to MATCH therefore
 * throws rather than returning nothing, which turns a bad search into a 500.
 *
 * Everything is reduced to bare alphanumeric tokens and re-quoted. Tokens are
 * lowercased so the bare words `and`/`or`/`not`/`near` cannot be re-parsed as
 * operators once quoted.
 */
export function toMatchTokens(query: string): string[] {
  return (query.match(/[\p{L}\p{N}]+/gu) ?? [])
    .map((token) => token.toLowerCase())
    .filter((token) => token.length >= 2);
}

/** Ceiling on tokens sent to MATCH, applied AFTER stopwords are dropped.
 *  Capping first meant a wordy question ("hi, is there any good addon that can
 *  tell me when i am in combat") spent its whole budget on filler and truncated
 *  the only word that mattered. */
const MAX_MATCH_TOKENS = 12;

/** Stopwords worth dropping only when the query has other content to stand on.
 *  "is there a good addon that shows combat" should search for "shows combat". */
const STOPWORDS = new Set([
  "the", "a", "an", "is", "are", "was", "were", "be", "been", "being",
  "any", "some", "good", "best", "there", "that", "this", "these", "those",
  "for", "with", "and", "or", "not", "but", "you", "your", "my", "me", "i",
  "it", "its", "of", "to", "in", "on", "at", "by", "from", "as", "if",
  "do", "does", "did", "can", "could", "would", "should", "will", "just",
  "have", "has", "had", "what", "which", "who", "how", "when", "where",
  "addon", "addons", "eso", "please", "thanks", "question", "stupid",
  // "am" was the acute bug: not a stopword, 2 chars so it survived the length
  // filter, and present in only 184 of 4170 descriptions. As an AND term it
  // excluded 96% of the catalogue, which is why "an addon that shows when I am
  // in combat" could not return Combat Indicator.
  "am", "im", "ive", "id", "ill", "were", "youre", "its", "hi", "hey", "ok",
  // Generic UI/action verbs. "show" alone appears in 1162 of 4170 descriptions
  // (28%), so it contributes almost nothing to RANKING while acting as a hard
  // filter under AND. Listed unstemmed because the check runs on the raw JS
  // token, before FTS5's porter stemmer sees it.
  "show", "shows", "showing", "shown", "display", "displays", "displaying",
  "tell", "tells", "let", "lets", "make", "makes", "give", "gives", "get",
  "gets", "use", "uses", "using", "want", "wants", "need", "needs", "know",
  "see", "find", "looking", "help", "helps", "recommend", "something",
  "anything", "way", "one", "possible", "good", "better",
]);

/**
 * Drop stopwords, but never all of them — a query made entirely of stopwords
 * ("is there any good addon") still has to search for something, and the
 * unfiltered tokens are a better answer than an empty result.
 */
export function contentTokens(tokens: string[]): string[] {
  const kept = tokens.filter((token) => !STOPWORDS.has(token));
  return (kept.length > 0 ? kept : tokens).slice(0, MAX_MATCH_TOKENS);
}

/**
 * Split CamelCase / PascalCase runs into separate words, keeping the original.
 *
 * ESOUI addon names are overwhelmingly glued identifiers — "CombatIndicator",
 * "FightingDisplay", "AwesomeGuildStore". FTS5's unicode61 tokenizer has no
 * concept of a case boundary, so "CombatIndicator" indexes as the single token
 * `combatindicator` and a search for "combat indicator" does not match the
 * TITLE at all. It would still match any addon that happened to spell the words
 * out in its description, so the symptom is not an empty result — it is the
 * best answer ranking below worse ones, which is much harder to notice.
 *
 * Emitting "CombatIndicator Combat Indicator" makes both spellings hit.
 */
export function expandIdentifier(text: string): string {
  const split = text
    // lower|digit -> Upper  ("combatIndicator", "eso5Bar")
    .replace(/([\p{Ll}\p{N}])(\p{Lu})/gu, "$1 $2")
    // acronym boundary: "ESOUIHelper" -> "ESOUI Helper"
    .replace(/(\p{Lu}+)(\p{Lu}\p{Ll})/gu, "$1 $2")
    // letter <-> digit  ("Bar5" / "5Bar")
    .replace(/(\p{L})(\p{N})/gu, "$1 $2")
    .replace(/(\p{N})(\p{L})/gu, "$1 $2");
  return split === text ? text : `${text} ${split}`;
}

function buildMatchExpression(tokens: string[], mode: "and" | "or"): string {
  const quoted = tokens.map((token) => `"${token}"`);
  return quoted.join(mode === "and" ? " AND " : " OR ");
}

interface SearchRow {
  uid: number;
  title: string;
  author: string;
  category_name: string;
  downloads: number;
  favorites: number;
  last_update: number;
  file_info_uri: string;
  is_library: number;
  snippet: string;
  score: number;
}

function rowToHit(row: SearchRow): AddonSearchHit {
  return {
    esoui_id: row.uid,
    title: row.title,
    author: row.author,
    category: row.category_name,
    downloads: row.downloads,
    favorites: row.favorites,
    last_update: row.last_update,
    file_info_uri: row.file_info_uri || `https://www.esoui.com/downloads/info${row.uid}.html`,
    is_library: row.is_library === 1,
    snippet: row.snippet ?? "",
    // bm25() returns 0 for no match and increasingly negative values for better
    // ones. Flip the sign so callers can treat "higher is better" normally.
    score: -row.score,
  };
}

export interface SearchOptions {
  limit?: number;
  offset?: number;
  /** Libraries are dependencies, not things a player chooses. Excluded by
   *  default, matching how `browse_popular` in esoui.rs filters them. */
  includeLibraries?: boolean;
  /** Include retired addons. Off by default — see DISCONTINUED_CATEGORY_ID. */
  includeDiscontinued?: boolean;
}

/**
 * BM25 search over the indexed catalogue.
 *
 * Runs the tokens as AND first, because a query with several content words
 * almost always wants all of them. If that returns nothing it retries as OR so
 * a single unmatched word cannot zero out an otherwise good query.
 */
export async function searchAddons(
  db: D1Database,
  query: string,
  options: SearchOptions = {},
): Promise<AddonSearchResult> {
  const limit = Math.min(Math.max(options.limit ?? DEFAULT_LIMIT, 1), MAX_LIMIT);
  const offset = Math.max(options.offset ?? 0, 0);
  const trimmed = query.trim().slice(0, MAX_QUERY_LENGTH);

  const tokens = contentTokens(toMatchTokens(trimmed));
  if (tokens.length === 0) {
    return { hits: [], matched: 0, mode: "none" };
  }

  const buildSql = (mode: "and" | "or") => `
      SELECT a.uid, a.title, a.author, a.category_name, a.downloads, a.favorites,
             a.last_update, a.file_info_uri, a.is_library,
             snippet(addons_fts, 3, '', '', '…', ${SNIPPET_TOKENS}) AS snippet,
             bm25(addons_fts, ${BM25_WEIGHTS}) AS score
        FROM addons_fts
        JOIN addons a ON a.uid = addons_fts.rowid
       WHERE addons_fts MATCH ?
         AND a.removed = 0
         ${options.includeLibraries ? "" : "AND a.is_library = 0"}
         ${options.includeDiscontinued ? "" : `AND a.category_id != ${DISCONTINUED_CATEGORY_ID}`}
       ORDER BY ${RANK_EXPRESSION}
       LIMIT ? OFFSET ?`;

  // One token makes AND and OR identical, so there is nothing to union.
  const singleToken = tokens.length === 1;

  let andRows: SearchRow[] = [];
  let orRows: SearchRow[] = [];
  try {
    // Reserve half the page for the strict pass so today's best results are
    // preserved verbatim, then fill the remainder from the permissive pass.
    // Appending OR *after* a full AND page would have changed nothing: when AND
    // already returns `limit` rows the extra recall is invisible, which is
    // exactly how Combat Indicator stayed hidden.
    const andLimit = singleToken ? limit : Math.ceil(limit / 2);
    const statements = [
      db.prepare(buildSql("and")).bind(buildMatchExpression(tokens, "and"), andLimit, offset),
    ];
    if (!singleToken) {
      statements.push(
        db.prepare(buildSql("or")).bind(buildMatchExpression(tokens, "or"), limit, offset),
      );
    }
    // One round trip for both passes.
    const [andResult, orResult] = await db.batch<SearchRow>(statements);
    andRows = andResult?.results ?? [];
    orRows = orResult?.results ?? [];
  } catch (err) {
    if (isMissingTable(err)) return { hits: [], matched: 0, mode: "none" };
    throw err;
  }

  const seen = new Set<number>();
  const merged: SearchRow[] = [];
  for (const row of [...andRows, ...orRows]) {
    if (seen.has(row.uid)) continue;
    seen.add(row.uid);
    merged.push(row);
    if (merged.length >= limit) break;
  }

  if (merged.length === 0) return { hits: [], matched: 0, mode: "none" };

  const mode: AddonSearchResult["mode"] =
    singleToken || orRows.length === 0 ? "and" : andRows.length === 0 ? "or" : "union";
  return { hits: merged.map(rowToHit), matched: merged.length, mode };
}

/** Metadata row as it arrives from the bulk filelist, before descriptions. */
export interface AddonMetaRow {
  uid: number;
  title: string;
  author: string;
  categoryId: number;
  /** Resolved from categorylist.json at sync time and written with the row, so
   *  no second UPDATE pass is needed (which used to cost one query per
   *  category, against a hard 1000-queries-per-invocation budget). */
  categoryName: string;
  downloads: number;
  downloadsMonthly: number;
  favorites: number;
  isLibrary: boolean;
  fileInfoUri: string;
  lastUpdate: number;
}

/**
 * D1 caps a query at 100 bound parameters. Each row binds 12, so 8 rows
 * (96 params) is the largest multi-row statement that fits with headroom.
 */
const ROW_PARAMS = 12;
const ROWS_PER_STATEMENT = Math.floor(96 / ROW_PARAMS);

/** Statements per `batch()` call. Only affects round trips, not any hard limit. */
const STATEMENTS_PER_BATCH = 50;

const UPSERT_TAIL = `ON CONFLICT(uid) DO UPDATE SET
         title = excluded.title,
         author = excluded.author,
         category_id = excluded.category_id,
         category_name = excluded.category_name,
         downloads = excluded.downloads,
         downloads_monthly = excluded.downloads_monthly,
         favorites = excluded.favorites,
         is_library = excluded.is_library,
         file_info_uri = excluded.file_info_uri,
         removed = 0,
         indexed_at = excluded.indexed_at,
         detail_stale = CASE
           -- Un-tombstoning MUST re-queue a description fetch. Removal deletes
           -- the addons_fts row, and applyDetail is the only thing that ever
           -- writes one back. Without this clause a resurrected addon (upstream
           -- blip, or removed-then-restored) stays live in the addons table with
           -- no FTS row and is permanently unsearchable -- silently, because
           -- search just returns fewer results.
           WHEN addons.removed = 1 THEN 1
           -- The FTS row embeds category_name, and only applyDetail rewrites
           -- it. Without this, a corrected category never reaches the search
           -- index -- the addons table and addons_fts just quietly disagree.
           WHEN addons.category_name != excluded.category_name THEN 1
           -- Otherwise only a moved last_update re-queues. Download counts
           -- change constantly and say nothing about the text.
           WHEN addons.last_update != excluded.last_update THEN 1
           ELSE addons.detail_stale
         END,
         last_update = excluded.last_update`;

function upsertStatement(db: D1Database, rows: AddonMetaRow[], now: number): D1PreparedStatement {
  // 12 bound values per row; `removed` and `detail_stale` are literals.
  const placeholders = rows.map(() => "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, 1)").join(", ");
  const sql = `INSERT INTO addons (
         uid, title, author, category_id, category_name, downloads,
         downloads_monthly, favorites, is_library, file_info_uri, last_update,
         indexed_at, removed, detail_stale
       ) VALUES ${placeholders}
       ${UPSERT_TAIL}`;

  const values: (string | number)[] = [];
  for (const row of rows) {
    values.push(
      row.uid,
      row.title,
      row.author,
      row.categoryId,
      row.categoryName,
      row.downloads,
      row.downloadsMonthly,
      row.favorites,
      row.isLibrary ? 1 : 0,
      row.fileInfoUri,
      row.lastUpdate,
      now,
    );
  }
  return db.prepare(sql).bind(...values);
}

/**
 * Write bulk-list metadata for many addons.
 *
 * Batched into multi-row statements because D1 allows only **1000 queries per
 * Worker invocation**. One statement per addon meant ~4000 queries for a full
 * ESOUI sync, which fails outright — this brings the same work down to ~500.
 *
 * `detail_stale` is set when the addon is new or its `last_update` moved, which
 * is what schedules a `filedetails` fetch later. It is deliberately NOT cleared
 * here — only {@link applyDetail} clears it, so an interrupted backfill resumes
 * exactly where it stopped instead of silently leaving rows description-less.
 *
 * Returns the number of D1 queries spent, so the caller can stay under budget.
 */
export async function upsertMetaBatch(
  db: D1Database,
  rows: AddonMetaRow[],
  now: number,
): Promise<number> {
  if (rows.length === 0) return 0;

  const statements: D1PreparedStatement[] = [];
  for (let i = 0; i < rows.length; i += ROWS_PER_STATEMENT) {
    statements.push(upsertStatement(db, rows.slice(i, i + ROWS_PER_STATEMENT), now));
  }
  for (let i = 0; i < statements.length; i += STATEMENTS_PER_BATCH) {
    await db.batch(statements.slice(i, i + STATEMENTS_PER_BATCH));
  }
  return statements.length;
}

/** Single-row convenience wrapper. */
export async function upsertMeta(
  db: D1Database,
  row: AddonMetaRow,
  now: number,
): Promise<void> {
  await upsertMetaBatch(db, [row], now);
}

/**
 * Attach a fetched description and (re)build the addon's FTS row.
 *
 * The FTS row is deleted first because a non-external-content FTS5 table has no
 * uniqueness constraint on rowid conflicts to lean on — inserting twice would
 * leave the addon matching under both its old and new text.
 */
export async function applyDetail(
  db: D1Database,
  uid: number,
  description: string,
  categoryName: string,
  now: number,
): Promise<void> {
  const meta = await db
    .prepare("SELECT title, author FROM addons WHERE uid = ?")
    .bind(uid)
    .first<{ title: string; author: string }>();
  if (!meta) return;

  await db.batch([
    db
      .prepare(
        `UPDATE addons
            SET description = ?, category_name = ?, detail_fetched_at = ?, detail_stale = 0
          WHERE uid = ?`,
      )
      .bind(description, categoryName, now, uid),
    db.prepare("DELETE FROM addons_fts WHERE rowid = ?").bind(uid),
    db
      .prepare(
        `INSERT INTO addons_fts (rowid, title, author, category, description)
         VALUES (?, ?, ?, ?, ?)`,
      )
      .bind(uid, expandIdentifier(meta.title), meta.author, categoryName, description),
  ]);
}

/** Tombstone specific addons — used for a `filedetails` 404, which is per-addon.
 *
 *  Chunked because D1 rejects a query with more than 100 bound parameters, and
 *  an `IN (...)` clause binds one per uid. Rows are kept rather than deleted so
 *  a transient upstream blip that drops an entry for one run can be undone by
 *  the next run's upsert. */
export async function markRemoved(db: D1Database, uids: number[]): Promise<void> {
  if (uids.length === 0) return;
  // 45 per statement; the batch below issues two statements over the same list.
  const CHUNK = 45;
  for (let i = 0; i < uids.length; i += CHUNK) {
    const slice = uids.slice(i, i + CHUNK);
    const placeholders = slice.map(() => "?").join(", ");
    await db.batch([
      db.prepare(`UPDATE addons SET removed = 1 WHERE uid IN (${placeholders})`).bind(...slice),
      db.prepare(`DELETE FROM addons_fts WHERE rowid IN (${placeholders})`).bind(...slice),
    ]);
  }
}

/**
 * Tombstone every live addon the current sync did not touch.
 *
 * Replaces "read all live uids, diff in JS, delete by id list", which bound one
 * parameter per vanished addon and blew D1's 100-parameter cap the first time
 * ESOUI removed more than a hundred entries. This is three parameter-free
 * statements regardless of catalogue size.
 *
 * `since` is the timestamp stamped onto every row the sync upserted, so
 * `indexed_at < since` is exactly "not seen in this run".
 */
export async function sweepUnseen(db: D1Database, since: number): Promise<number> {
  const before = await db
    .prepare("SELECT COUNT(*) AS n FROM addons WHERE removed = 0 AND indexed_at < ?")
    .bind(since)
    .first<{ n: number }>();

  const count = before?.n ?? 0;
  if (count === 0) return 0;

  await db.batch([
    db
      .prepare(
        `DELETE FROM addons_fts
          WHERE rowid IN (SELECT uid FROM addons WHERE removed = 0 AND indexed_at < ?)`,
      )
      .bind(since),
    db.prepare("UPDATE addons SET removed = 1 WHERE removed = 0 AND indexed_at < ?").bind(since),
  ]);

  return count;
}

/** Live addons still missing a current description, most-downloaded first so a
 *  partial backfill is already useful for the addons people actually search. */
export async function pendingDetailUids(db: D1Database, limit: number): Promise<number[]> {
  const result = await db
    .prepare(
      `SELECT uid FROM addons
        WHERE detail_stale = 1 AND removed = 0
        ORDER BY downloads DESC
        LIMIT ?`,
    )
    .bind(limit)
    .all<{ uid: number }>();
  return (result.results ?? []).map((row) => row.uid);
}

export async function indexStats(db: D1Database): Promise<AddonIndexStats> {
  let row: { total: number; live: number; described: number; indexed_at: number } | null = null;
  try {
    row = await db
    .prepare(
      `SELECT
         COUNT(*) AS total,
         SUM(CASE WHEN removed = 0 THEN 1 ELSE 0 END) AS live,
         SUM(CASE WHEN removed = 0 AND detail_stale = 0 THEN 1 ELSE 0 END) AS described,
         MAX(indexed_at) AS indexed_at
       FROM addons`,
    )
    .first<{ total: number; live: number; described: number; indexed_at: number }>();
  } catch (err) {
    // An unbuilt index reports zeros, which is exactly what it contains.
    if (!isMissingTable(err)) throw err;
  }

  return {
    version: INDEX_VERSION,
    total: row?.total ?? 0,
    live: row?.live ?? 0,
    described: row?.described ?? 0,
    indexed_at: row?.indexed_at ?? 0,
    last_sync: row ? ((await getMeta(db, "last_sync")) ?? null) : null,
  };
}

/** Narrow the optional binding once, with a message that says what to do. */
export function requireIndexDb(env: Env): D1Database | null {
  return env.ADDON_INDEX ?? null;
}
