#!/usr/bin/env node
/**
 * Search-quality eval harness for the Kalpa addon index.
 *
 * Every ranking judgement before this was anecdotal: one query, eyeballed, at a
 * time. Two ranking changes were made on hunches and a hand-rolled accuracy
 * check reported 10/12 when the truth was ~6/12. This script exists so that no
 * further tuning happens without a measured baseline.
 *
 * It replays `test/fixtures/search-eval.json` against a live
 * `GET /addons/search` and reports:
 *
 *   recall@20 - the metric that matters for Ask, which hands 20 candidates to
 *               the model. If the right addon is not in the 20, no amount of
 *               prompting can recover it.
 *   recall@5  - and MRR@5, which matter for the Search tab, where the user
 *               reads a short list and mostly looks at rank 1.
 *
 * Overall and split by row `kind` ("name" vs "concept"), because the two fail
 * for different reasons and averaging them hides both.
 *
 * The fixture's expected ids are ground truth decided from each addon's title
 * and description, NOT from what search returns today. Some rows are expected
 * to fail; that is the point.
 *
 * Usage:
 *   node scripts/eval-search.mjs [options]
 *   npm run eval:search -- [options]
 *
 * Options:
 *   --base <url>        Worker base URL (default: production)
 *   --fixture <path>    Fixture path (default: test/fixtures/search-eval.json)
 *   --limit <n>         Hits requested per query (default: 20)
 *   --delay <ms>        Delay between queries (default: 2200)
 *   --kind <name|concept>  Only run rows of this kind
 *   --min-recall <0..1> Exit non-zero if overall recall@20 is below this
 *                       (default: 0, i.e. reporting only)
 *   --json              Machine-readable output on stdout, nothing else
 *   --help              This text
 *
 * Politeness: the worker's READ_LIMITER is per-IP and per-minute, so this runs
 * strictly sequentially with a delay (default ~27 queries/min) and backs off on
 * 429 rather than hammering. Do not parallelise it beyond a handful of workers.
 */

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_DIR = resolve(HERE, "..");

const DEFAULTS = {
  base: "https://kalpa-pack-hub.eso-toolkit.workers.dev",
  fixture: resolve(REPO_DIR, "test/fixtures/search-eval.json"),
  limit: 20,
  delay: 2200,
  minRecall: 0,
  kind: null,
  json: false,
};

const HELP = `
eval-search - measure Kalpa addon-index search quality against a fixture.

  node scripts/eval-search.mjs [--base <url>] [--fixture <path>] [--limit <n>]
                               [--delay <ms>] [--kind name|concept]
                               [--min-recall <0..1>] [--json] [--help]

Reports recall@20, recall@5 and MRR@5, overall and per kind, plus a per-row
FAIL list. Exits 1 if overall recall@20 < --min-recall (default 0).
`.trim();

function parseArgs(argv) {
  const opts = { ...DEFAULTS };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = () => {
      const v = argv[i + 1];
      if (v === undefined) {
        throw new Error(`${arg} requires a value`);
      }
      i += 1;
      return v;
    };
    switch (arg) {
      case "--help":
      case "-h":
        console.log(HELP);
        process.exit(0);
        break;
      case "--json":
        opts.json = true;
        break;
      case "--base":
        opts.base = next().replace(/\/+$/, "");
        break;
      case "--fixture":
        opts.fixture = resolve(process.cwd(), next());
        break;
      case "--limit":
        opts.limit = Number(next());
        break;
      case "--delay":
        opts.delay = Number(next());
        break;
      case "--kind":
        opts.kind = next();
        break;
      case "--min-recall":
        opts.minRecall = Number(next());
        break;
      default:
        throw new Error(`unknown option: ${arg}`);
    }
  }
  if (!Number.isFinite(opts.limit) || opts.limit < 1) {
    throw new Error("--limit must be a positive number");
  }
  if (!Number.isFinite(opts.delay) || opts.delay < 0) {
    throw new Error("--delay must be a non-negative number");
  }
  if (!Number.isFinite(opts.minRecall) || opts.minRecall < 0 || opts.minRecall > 1) {
    throw new Error("--min-recall must be between 0 and 1");
  }
  if (opts.kind && opts.kind !== "name" && opts.kind !== "concept") {
    throw new Error('--kind must be "name" or "concept"');
  }
  return opts;
}

function loadFixture(path) {
  const rows = JSON.parse(readFileSync(path, "utf8"));
  if (!Array.isArray(rows)) throw new Error(`${path} must contain an array`);
  rows.forEach((row, i) => {
    if (typeof row.question !== "string" || !row.question.trim()) {
      throw new Error(`row ${i}: missing "question"`);
    }
    if (!Array.isArray(row.expected_esoui_ids) || row.expected_esoui_ids.length === 0) {
      throw new Error(`row ${i} (${row.question}): "expected_esoui_ids" must be a non-empty array`);
    }
    if (row.expected_esoui_ids.some((id) => !Number.isInteger(id))) {
      throw new Error(`row ${i} (${row.question}): expected ids must be integers`);
    }
    if (row.kind !== "name" && row.kind !== "concept") {
      throw new Error(`row ${i} (${row.question}): "kind" must be "name" or "concept"`);
    }
  });
  return rows;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function search(base, query, limit) {
  const url = `${base}/addons/search?q=${encodeURIComponent(query)}&limit=${limit}`;
  // One retry on 429/5xx. The limiter is per-minute, so a flat 20s wait clears
  // a burst without turning a rate limit into a fake ranking regression.
  for (let attempt = 0; attempt < 2; attempt += 1) {
    let res;
    try {
      res = await fetch(url, { headers: { accept: "application/json" } });
    } catch (err) {
      if (attempt === 0) {
        await sleep(5000);
        continue;
      }
      return { error: `network: ${err.message}`, hits: [] };
    }
    if (res.status === 429 || res.status >= 500) {
      if (attempt === 0) {
        await sleep(res.status === 429 ? 20000 : 5000);
        continue;
      }
      return { error: `HTTP ${res.status}`, hits: [] };
    }
    if (!res.ok) return { error: `HTTP ${res.status}`, hits: [] };
    let body;
    try {
      body = await res.json();
    } catch (err) {
      return { error: `bad JSON: ${err.message}`, hits: [] };
    }
    return { hits: Array.isArray(body.hits) ? body.hits : [], mode: body.mode, matched: body.matched };
  }
  return { error: "exhausted retries", hits: [] };
}

/** Fraction of a row's expected ids that appear in the first `k` hits. */
function recallAt(hits, expected, k) {
  const top = new Set(hits.slice(0, k).map((h) => h.esoui_id));
  const found = expected.filter((id) => top.has(id)).length;
  return found / expected.length;
}

/** Reciprocal rank of the FIRST expected id within the top `k`, else 0. */
function reciprocalRankAt(hits, expected, k) {
  const want = new Set(expected);
  for (let i = 0; i < Math.min(k, hits.length); i += 1) {
    if (want.has(hits[i].esoui_id)) return 1 / (i + 1);
  }
  return 0;
}

const mean = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : 0);

function aggregate(results) {
  return {
    rows: results.length,
    recall_at_20: mean(results.map((r) => r.recall20)),
    recall_at_5: mean(results.map((r) => r.recall5)),
    mrr_at_5: mean(results.map((r) => r.rr5)),
    perfect_at_20: results.filter((r) => r.recall20 === 1).length,
    rank1_hits: results.filter((r) => r.rr5 === 1).length,
  };
}

const pct = (x) => `${(x * 100).toFixed(1)}%`;

function printSummary(label, agg) {
  console.log(
    `  ${label.padEnd(9)} n=${String(agg.rows).padStart(3)}  ` +
      `recall@20 ${pct(agg.recall_at_20).padStart(6)}  ` +
      `recall@5 ${pct(agg.recall_at_5).padStart(6)}  ` +
      `MRR@5 ${agg.mrr_at_5.toFixed(3)}  ` +
      `(all-found@20 ${agg.perfect_at_20}/${agg.rows}, rank1 ${agg.rank1_hits}/${agg.rows})`,
  );
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  let rows = loadFixture(opts.fixture);
  if (opts.kind) rows = rows.filter((r) => r.kind === opts.kind);
  if (rows.length === 0) throw new Error("no rows to evaluate");

  const log = opts.json ? () => {} : (...args) => console.log(...args);

  log(`Kalpa addon-index search eval`);
  log(`  base:    ${opts.base}`);
  log(`  fixture: ${opts.fixture}`);
  log(`  rows:    ${rows.length}  (limit=${opts.limit}, delay=${opts.delay}ms)`);
  log("");

  const results = [];
  for (let i = 0; i < rows.length; i += 1) {
    const row = rows[i];
    const { hits, error, mode } = await search(opts.base, row.question, opts.limit);
    const recall20 = error ? 0 : recallAt(hits, row.expected_esoui_ids, 20);
    const recall5 = error ? 0 : recallAt(hits, row.expected_esoui_ids, 5);
    const rr5 = error ? 0 : reciprocalRankAt(hits, row.expected_esoui_ids, 5);
    const rankOf = {};
    for (const id of row.expected_esoui_ids) {
      const idx = hits.findIndex((h) => h.esoui_id === id);
      rankOf[id] = idx === -1 ? null : idx + 1;
    }
    results.push({
      kind: row.kind,
      question: row.question,
      note: row.note,
      expected_esoui_ids: row.expected_esoui_ids,
      expected_ranks: rankOf,
      top1: hits[0] ? { esoui_id: hits[0].esoui_id, title: hits[0].title } : null,
      top5: hits.slice(0, 5).map((h) => ({ esoui_id: h.esoui_id, title: h.title })),
      mode,
      error,
      recall20,
      recall5,
      rr5,
    });

    const flag = error ? "ERR " : recall20 === 1 ? (rr5 === 1 ? "PASS" : "ok  ") : "FAIL";
    log(
      `  [${String(i + 1).padStart(2)}/${rows.length}] ${flag} ${row.kind.padEnd(7)} ` +
        `r@20=${recall20.toFixed(2)} r@5=${recall5.toFixed(2)} rr@5=${rr5.toFixed(2)}  ${row.question}`,
    );
    if (i < rows.length - 1) await sleep(opts.delay);
  }

  const overall = aggregate(results);
  const byKind = {};
  for (const kind of [...new Set(results.map((r) => r.kind))].sort()) {
    byKind[kind] = aggregate(results.filter((r) => r.kind === kind));
  }

  // A "failure" is any expected id missing from the top 20 - the Ask cut-off.
  const failures = results.filter((r) => r.recall20 < 1);

  if (opts.json) {
    console.log(
      JSON.stringify(
        {
          base: opts.base,
          fixture: opts.fixture,
          limit: opts.limit,
          generated_at: new Date().toISOString(),
          overall,
          by_kind: byKind,
          failures: failures.map((f) => ({
            kind: f.kind,
            question: f.question,
            expected_esoui_ids: f.expected_esoui_ids,
            expected_ranks: f.expected_ranks,
            top1: f.top1,
            error: f.error,
          })),
          results,
        },
        null,
        2,
      ),
    );
  } else {
    console.log("");
    console.log("SUMMARY");
    printSummary("overall", overall);
    for (const [kind, agg] of Object.entries(byKind)) printSummary(kind, agg);

    console.log("");
    if (failures.length === 0) {
      console.log(`FAILURES: none - every expected addon is inside the top ${opts.limit}.`);
    } else {
      console.log(`FAILURES (expected addon missing from top 20): ${failures.length}/${results.length}`);
      for (const f of failures) {
        console.log("");
        console.log(`  ${f.kind}: "${f.question}"`);
        if (f.error) console.log(`    request error: ${f.error}`);
        const missing = f.expected_esoui_ids.filter((id) => f.expected_ranks[id] === null);
        const present = f.expected_esoui_ids.filter((id) => f.expected_ranks[id] !== null);
        console.log(`    expected: ${f.expected_esoui_ids.join(", ")}`);
        if (missing.length) console.log(`    missing from top 20: ${missing.join(", ")}`);
        if (present.length) {
          console.log(
            `    found: ${present.map((id) => `${id}@rank${f.expected_ranks[id]}`).join(", ")}`,
          );
        }
        console.log(`    rank 1 was: ${f.top1 ? `${f.top1.esoui_id} "${f.top1.title}"` : "(no hits)"}`);
        if (f.note) console.log(`    note: ${f.note}`);
      }
    }

    // Rows where everything was retrieved but rank 1 is wrong: invisible to
    // recall@20, but exactly what the Search tab shows the user first.
    const misranked = results.filter((r) => r.recall20 === 1 && r.rr5 !== 1);
    if (misranked.length) {
      console.log("");
      console.log(`MISRANKED (in top 20, but not at rank 1): ${misranked.length}`);
      for (const m of misranked) {
        const best = Math.min(...Object.values(m.expected_ranks).filter((v) => v !== null));
        console.log(
          `  ${m.kind}: "${m.question}" - best expected at rank ${best}; ` +
            `rank 1 was ${m.top1 ? `${m.top1.esoui_id} "${m.top1.title}"` : "(none)"}`,
        );
      }
    }
  }

  if (overall.recall_at_20 < opts.minRecall) {
    if (!opts.json) {
      console.error(
        `\nrecall@20 ${pct(overall.recall_at_20)} is below --min-recall ${pct(opts.minRecall)}`,
      );
    }
    process.exitCode = 1;
  }
}

main().catch((err) => {
  console.error(`eval-search: ${err.message}`);
  process.exitCode = 2;
});
