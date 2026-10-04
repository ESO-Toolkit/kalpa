#!/usr/bin/env node
/**
 * Sweep a relevance floor for the `also_considered` tail.
 *
 * Widening the tail to the whole retrieved set (ALSO_CONSIDERED_LIMIT =
 * CANDIDATE_COUNT + SEMANTIC_EXTRA) stopped it dropping good addons, but it
 * removed relevance filtering entirely: a weak query now trails obvious junk.
 * The fix is a score floor rather than a count cap, and this picks the ratio.
 *
 * It costs ZERO model calls. Because the tail is now fully determined by
 * retrieval, every candidate `/ask` would show is exactly what
 * `/addons/search?semantic=true&limit=26` returns — so the fixture is fetched
 * ONCE, cached to disk, and every ratio is scored offline against that cache.
 *
 *   ADMIN_API_KEY=... node scripts/sweep-tail-floor.mjs
 *   node scripts/sweep-tail-floor.mjs --cache-only   # re-score, no requests
 *
 * Two things about the score shape the arithmetic:
 *
 *   1. `AddonSearchHit.score` is NEGATED at the mapping layer
 *      (addon-index.ts:342), so it is positive and higher-is-better here even
 *      though SQLite's bm25() is negative and lower-is-better.
 *   2. Semantic-only hits carry `0 AS score` (they never went through FTS), so
 *      they are EXEMPT from the floor. Applying a ratio to them would delete
 *      every semantic match — the exact delivery bug this tail already had
 *      once. They are already filtered by the cosine floor.
 *
 * A caveat this cannot measure away: `score` is bm25 only, while the ORDER BY
 * also subtracts the popularity ladder and the title-phrase boost. So score
 * order and rank order can disagree, and a floor can cut a row that ranked
 * above one it keeps. The sweep reports whether that costs anything real.
 *
 * Options:
 *   --base <url>     Worker base URL (default: production)
 *   --fixture <path> Fixture (default: test/fixtures/search-eval.json)
 *   --cache <path>   Where to store fetched hits (default: .tail-sweep-cache.json)
 *   --cache-only     Fail rather than fetch if the cache is missing a row
 *   --delay <ms>     Delay between queries (default: 2400)
 *   --limit <n>      Candidates per query (default: 26)
 */

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_DIR = resolve(HERE, "..");

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i !== -1 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const BASE = arg("base", "https://kalpa-pack-hub.eso-toolkit.workers.dev").replace(/\/+$/, "");
const FIXTURE = resolve(REPO_DIR, arg("fixture", "test/fixtures/search-eval.json"));
const CACHE = resolve(REPO_DIR, arg("cache", ".tail-sweep-cache.json"));
const DELAY = Number(arg("delay", "2400"));
const LIMIT = Number(arg("limit", "26"));
const CACHE_ONLY = process.argv.includes("--cache-only");

/** Ratios to score. 0 is the current shipped behaviour (no floor at all). */
const RATIOS = [0, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7];

/** Mirrors MAX_RECOMMENDATIONS in ask.ts — the tail is what the model did not
 *  pick, and the model picks at most this many. */
const MAX_RECOMMENDATIONS = 5;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function fetchHits(question) {
  const url = `${BASE}/addons/search?q=${encodeURIComponent(question)}&limit=${LIMIT}&semantic=true`;
  const key = process.env.ADMIN_API_KEY;
  if (!key) throw new Error("ADMIN_API_KEY is required (?semantic=true is admin-only)");

  for (let attempt = 0; attempt < 3; attempt += 1) {
    if (attempt > 0) await sleep(attempt === 1 ? 20000 : 40000);
    let res;
    try {
      res = await fetch(url, { headers: { accept: "application/json", "X-API-Key": key } });
    } catch {
      continue;
    }
    if (res.status === 429 || res.status >= 500) continue;
    if (!res.ok) throw new Error(`HTTP ${res.status} for ${question}`);
    const body = await res.json();
    return (body.hits ?? []).map((h) => ({
      esoui_id: h.esoui_id,
      title: h.title,
      score: h.score,
      semantic: Boolean(h.semantic),
    }));
  }
  throw new Error(`exhausted retries for: ${question}`);
}

/**
 * Apply the floor the way ask.ts would: keep every semantic hit, and every
 * keyword hit scoring at least `ratio` of the top KEYWORD score. Rank 1 is
 * always kept so a query whose scores all cluster low still answers.
 */
function applyFloor(hits, ratio) {
  if (ratio <= 0) return hits;
  const keywordScores = hits.filter((h) => !h.semantic).map((h) => h.score);
  const top = keywordScores.length ? Math.max(...keywordScores) : 0;
  if (!(top > 0)) return hits;
  return hits.filter((h, i) => i === 0 || h.semantic || h.score >= top * ratio);
}

const rows = JSON.parse(readFileSync(FIXTURE, "utf8"));
const cache = existsSync(CACHE) ? JSON.parse(readFileSync(CACHE, "utf8")) : {};

let fetched = 0;
for (let i = 0; i < rows.length; i += 1) {
  const q = rows[i].question;
  if (cache[q]) continue;
  if (CACHE_ONLY) throw new Error(`--cache-only but cache is missing: ${q}`);
  cache[q] = await fetchHits(q);
  fetched += 1;
  writeFileSync(CACHE, JSON.stringify(cache, null, 2));
  process.stderr.write(`  fetched ${fetched} (${i + 1}/${rows.length}) ${q.slice(0, 60)}\n`);
  if (i < rows.length - 1) await sleep(DELAY);
}

console.log(`rows=${rows.length}  fetched=${fetched}  cached=${rows.length - fetched}  limit=${LIMIT}`);
console.log("");
console.log("ratio |  recall  concept    name | mean tail | mean cut | rows losing expected");
console.log("------+-------------------------+-----------+----------+---------------------");

for (const ratio of RATIOS) {
  const per = rows.map((row) => {
    const hits = cache[row.question] ?? [];
    const kept = applyFloor(hits, ratio);
    // The model is not run here, so assume the worst case for the floor: the
    // picks are the first MAX_RECOMMENDATIONS, leaving the longest possible
    // tail for it to filter.
    const tail = kept.slice(MAX_RECOMMENDATIONS);
    const keptIds = new Set(kept.map((h) => h.esoui_id));
    const baseIds = new Set(hits.map((h) => h.esoui_id));
    // Only count a loss if the FLOOR removed it — not if retrieval never had it.
    const lost = row.expected_esoui_ids.filter((id) => baseIds.has(id) && !keptIds.has(id));
    return {
      kind: row.kind,
      delivered: row.expected_esoui_ids.some((id) => keptIds.has(id)),
      tailLen: tail.length,
      cut: hits.length - kept.length,
      lost,
    };
  });

  const rate = (xs) => (xs.length ? xs.filter((x) => x.delivered).length / xs.length : 0);
  const pct = (x) => `${(x * 100).toFixed(1)}%`;
  const concept = per.filter((p) => p.kind === "concept");
  const name = per.filter((p) => p.kind === "name");
  const meanTail = per.reduce((a, p) => a + p.tailLen, 0) / per.length;
  const meanCut = per.reduce((a, p) => a + p.cut, 0) / per.length;
  const losers = per.filter((p) => p.lost.length > 0);

  console.log(
    ` ${ratio.toFixed(2)} | ${pct(rate(per)).padStart(7)} ${pct(rate(concept)).padStart(7)} ` +
      `${pct(rate(name)).padStart(7)} | ${meanTail.toFixed(1).padStart(9)} | ` +
      `${meanCut.toFixed(1).padStart(8)} | ${String(losers.length).padStart(2)}`,
  );
}

console.log("");
console.log("Rows that lose a RETRIEVED expected addon, per ratio:");
for (const ratio of RATIOS) {
  if (ratio === 0) continue;
  const losers = rows
    .map((row) => {
      const hits = cache[row.question] ?? [];
      const keptIds = new Set(applyFloor(hits, ratio).map((h) => h.esoui_id));
      const baseIds = new Set(hits.map((h) => h.esoui_id));
      const lost = row.expected_esoui_ids.filter((id) => baseIds.has(id) && !keptIds.has(id));
      return lost.length ? `${row.kind}: "${row.question}" loses ${lost.join(", ")}` : null;
    })
    .filter(Boolean);
  console.log(`  ratio ${ratio}:${losers.length === 0 ? " none" : ""}`);
  for (const l of losers) console.log(`    ${l}`);
}
