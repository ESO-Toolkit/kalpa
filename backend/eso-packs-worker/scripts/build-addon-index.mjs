#!/usr/bin/env node
/**
 * Build (or top up) the ESOUI addon full-text index.
 *
 * Drives the worker's own admin routes rather than talking to ESOUI directly,
 * so the crawl runs through exactly the code the tests cover — no second
 * implementation to drift.
 *
 *   ADMIN_API_KEY=... node scripts/build-addon-index.mjs
 *
 * Two phases:
 *   1. POST /admin/index/sync      — one bulk filelist request, ~4000 rows of
 *                                    metadata, tombstones anything removed.
 *   2. POST /admin/index/backfill  — repeated until complete. Each page fetches
 *                                    up to 40 descriptions at ~4 req/s.
 *
 * The initial run is roughly 4000 requests / ~100 pages / ~20 minutes. Later
 * runs are near-instant, because only addons whose `lastUpdate` moved are
 * re-fetched. Safe to re-run and safe to interrupt: progress is committed per
 * page, and `detail_stale` is only cleared on a successful fetch, so a resumed
 * run picks up exactly where it stopped.
 *
 * Options:
 *   --base <url>   Worker base URL (default: production)
 *   --max <n>      Stop after n backfill pages (for a smoke test)
 *   --sync-only    Run phase 1 and stop
 */

const DEFAULT_BASE = "https://kalpa-pack-hub.eso-toolkit.workers.dev";

function arg(name, fallback = undefined) {
  const i = process.argv.indexOf(`--${name}`);
  return i !== -1 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const BASE = arg("base", DEFAULT_BASE).replace(/\/$/, "");
const MAX_PAGES = Number.parseInt(arg("max", "0"), 10) || Infinity;
const SYNC_ONLY = process.argv.includes("--sync-only");
const KEY = process.env.ADMIN_API_KEY;

if (!KEY) {
  console.error("ADMIN_API_KEY is not set.\n");
  console.error("  ADMIN_API_KEY=<key> node scripts/build-addon-index.mjs");
  process.exit(1);
}

async function post(path) {
  const response = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers: { "X-API-Key": KEY },
  });
  const text = await response.text();
  let body;
  try {
    body = JSON.parse(text);
  } catch {
    throw new Error(`${path} returned non-JSON (HTTP ${response.status}): ${text.slice(0, 200)}`);
  }
  if (!response.ok) {
    throw new Error(`${path} failed (HTTP ${response.status}): ${body.error ?? text}`);
  }
  return body;
}

const started = Date.now();
const elapsed = () => `${Math.round((Date.now() - started) / 1000)}s`;

console.log(`Target: ${BASE}`);
console.log("Phase 1/2 — syncing the bulk filelist…");
const sync = await post("/admin/index/sync");
console.log(`  ${sync.seen} addons seen, ${sync.removed} removed [${elapsed()}]`);

if (SYNC_ONLY) {
  console.log("--sync-only set; stopping before the description backfill.");
  process.exit(0);
}

console.log("Phase 2/2 — fetching descriptions…");
let page = 0;
let fetched = 0;
let removed = 0;
let failed = 0;

while (page < MAX_PAGES) {
  const outcome = await post("/admin/index/backfill");
  page++;
  fetched += outcome.fetched;
  removed += outcome.removed;
  failed += outcome.failed;

  console.log(
    `  page ${page}: +${outcome.fetched} descriptions` +
      (outcome.removed ? `, ${outcome.removed} removed` : "") +
      (outcome.failed ? `, ${outcome.failed} failed` : "") +
      ` (total ${fetched}) [${elapsed()}]`,
  );

  if (outcome.complete) {
    console.log(`\nDone. ${fetched} described, ${removed} removed, ${failed} failed in ${elapsed()}.`);
    console.log("\nNext: set ADDON_INDEX_SYNC = \"enabled\" in wrangler.toml and redeploy");
    console.log("to turn on the nightly delta.");
    process.exit(0);
  }

  // A page that fetched nothing but is not complete means every addon in it
  // failed. Retrying immediately would just hammer a struggling upstream.
  if (outcome.fetched === 0 && outcome.failed > 0) {
    console.error(`\nStopping: page ${page} failed entirely. Re-run later to resume.`);
    process.exit(1);
  }
}

console.log(`\nStopped after ${page} pages (--max). Re-run to continue; progress is saved.`);
