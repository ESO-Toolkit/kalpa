// ── Pack addon entry ────────────────────────────────────────────────
export interface PackAddonEntry {
  esouiId: number;
  name: string;
  required: boolean;
  defaultEnabled?: boolean;
  note?: string;
}

// ── Pack types ────────────────────────────────────────────────────────
export type PackType = "addon-pack" | "build-pack" | "roster-pack";
export type PackStatus = "draft" | "published";

// ── Full pack (snake_case to match Rust HubPack) ─────────────────────
export interface Pack {
  id: string;
  title: string;
  description: string;
  pack_type: PackType;
  author_id: string;
  author_name: string;
  is_anonymous: boolean;
  addons: PackAddonEntry[];
  tags: string[];
  vote_count: number;
  install_count: number;
  created_at: string;
  updated_at: string;
  status: PackStatus;
}

/**
 * A pack as returned to one specific viewer. `user_voted` is derived per
 * request from `vote:{id}:{userId}` and is never stored — the Rust HubPack
 * declares it `#[serde(default)] Option<bool>` and the client renders its vote
 * button from it, so omitting it made every pack look unvoted and turned the
 * client's toggle into an unvote. Responses carrying it must not be cached.
 */
export interface PackView extends Pack {
  user_voted?: boolean;
}

// ── Index (stores full packs for list queries) ────────────────────────
export interface PackIndex {
  packs: Pack[];
}

// ── Vote tracking ─────────────────────────────────────────────────────
export interface VoteRecord {
  userId: string;
  packId: string;
  votedAt: string;
}

export interface VoteResponse {
  voted: boolean;
  voteCount: number;
}

// ── Validation ────────────────────────────────────────────────────────
export interface ValidationError {
  field: string;
  message: string;
}

// ── Share types ───────────────────────────────────────────────────────
export interface SharePackData {
  title: string;
  description: string;
  packType: PackType;
  tags: string[];
  addons: PackAddonEntry[];
}

export interface ShareRecord {
  code: string;
  pack: SharePackData;
  createdBy: string;
  createdByName: string;
  createdAt: string;
  expiresAt: string;
}

export interface ShareCodeResponse {
  code: string;
  expiresAt: string;
  deepLink: string;
}

// ── Addon index (ESOUI catalogue full-text search) ────────────────────
/** One search hit. snake_case to match the Rust AddonSearchHit struct. */
export interface AddonSearchHit {
  esoui_id: number;
  title: string;
  author: string;
  category: string;
  downloads: number;
  favorites: number;
  /** Epoch millis, straight from the ESOUI filelist. */
  last_update: number;
  file_info_uri: string;
  is_library: boolean;
  snippet: string;
  /** Higher is better. Sign-flipped bm25 — comparable within one result set
   *  only, never across queries. */
  score: number;
}

export interface AddonSearchResult {
  hits: AddonSearchHit[];
  matched: number;
  /** Which pass produced the hits: the strict pass only, the permissive pass
   *  only, both merged, or nothing matched. */
  mode: "and" | "or" | "union" | "fused" | "none";
}

export interface AddonIndexStats {
  version: number;
  total: number;
  live: number;
  described: number;
  /** Live rows still awaiting a description fetch — how far behind the crawl is. */
  pending_details: number;
  indexed_at: number;
  last_sync: string | null;
  /** Hours since the last successful filelist sync, or null if never synced. */
  stale_hours: number | null;
}

/**
 * One semantic-retrieval hit: an addon uid and its cosine similarity to the
 * question. Deliberately carries no addon fields — the row is rehydrated from
 * D1, so the vector store can never be the source of a title or a link.
 */
export interface AddonVectorHit {
  uid: number;
  /** -1..1. Comparable within one question's result set. */
  cosine: number;
}

// ── Ask (natural-language addon assistant) ───────────────────────────
/** One recommended addon. Links are always rebuilt from the index, never
 *  taken from model output. */
export interface AskRecommendation {
  esoui_id: number;
  title: string;
  author: string;
  category: string;
  file_info_uri: string;
  reason: string;
}

export interface AskResponse {
  /** Prose answer. Empty when the model was skipped — see `degraded`. */
  answer: string;
  recommendations: AskRecommendation[];
  /** Ranked candidates the model did not pick. Free (no extra model call) and
   *  shown collapsed, so a short answer does not look like it missed things. */
  also_considered: AskRecommendation[];
  no_good_match: boolean;
  /** True when the ranked candidates are shown without model prose (model
   *  unavailable, over budget, or output failed grounding). */
  degraded: boolean;
  cached: boolean;
}

export interface CrawlOutcome {
  fetched: number;
  removed: number;
  failed: number;
  remaining: number;
  complete: boolean;
}

// ── Env bindings ──────────────────────────────────────────────────────
export interface Env {
  ESO_PACKS: KVNamespace;
  ADMIN_API_KEY: string;
  ALLOW_SEED?: string;
  /** Shared D1 binding to roster-hub-db — same database roster-hub-api uses */
  ROSTER_HUB_DB?: D1Database;
  /** Exact values: off, dry-run (default/fail-closed), or apply. */
  D1_RECONCILIATION_MODE?: string;
  /** Built-in atomic rate limit bindings (GA Sep 2025) */
  READ_LIMITER: RateLimit;
  WRITE_LIMITER: RateLimit;
  VOTE_LIMITER: RateLimit;
  /**
   * DELETE /account only. Erasure is paged: ACCOUNT_DELETE_VOTE_BUDGET caps one
   * request at ~450 votes and returns `complete: false` for the caller to
   * repeat, so an account with a few thousand votes needs a dozen or more
   * rounds. On WRITE_LIMITER's 10/min the user was 429'd partway through
   * deleting their own data and could never finish the erasure.
   *
   * Optional so a deployment that has not added the binding yet still falls
   * back to WRITE_LIMITER rather than losing rate limiting on the route.
   */
  ERASURE_LIMITER?: RateLimit;
  /** Durable Object for atomic pack index mutations */
  PACK_INDEX: DurableObjectNamespace<import("./pack-index-do").PackIndexDO>;
  /**
   * Dedicated D1 for the ESOUI addon full-text index.
   *
   * Deliberately NOT roster-hub-db: that database is shared with the ESO
   * Toolkit website and CLAUDE.md requires coordinating every schema change
   * there. Optional so the worker keeps serving Pack Hub if the binding is
   * absent — the addon routes 503 instead of the whole worker failing.
   */
  ADDON_INDEX?: D1Database;
  /**
   * Gate on the nightly ESOUI crawl. Exact value "enabled" turns it on;
   * anything else (including unset) leaves it off.
   *
   * Fail-closed on purpose. The cron reaches out to a third party, and it must
   * not start doing that the moment the D1 binding is added — the initial
   * backfill has to be run and checked first. It also keeps the scheduled
   * tests off the network.
   */
  ADDON_INDEX_SYNC?: string;
  /** Bounds the addon search route independently of pack reads. */
  ADDON_SEARCH_LIMITER?: RateLimit;
  /** Workers AI binding for the Ask assistant. Optional: without it /ask still
   *  answers, returning ranked candidates with no prose. */
  AI?: Ai;
  /** Tighter budget than search — an Ask costs a model call, not just a query. */
  ASK_LIMITER?: RateLimit;
  /** Workers AI model id. Overridable so swapping models is config, not code. */
  ASK_MODEL?: string;
  /** Max model calls per UTC day before /ask degrades to candidates-only. */
  ASK_DAILY_BUDGET?: string;
}
