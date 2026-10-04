#!/usr/bin/env node
/**
 * END-TO-END eval for the Kalpa `/ask` assistant.
 *
 * `eval-search.mjs` measures RETRIEVAL: whether the right addon is somewhere in
 * the 20 candidates handed to the model. That is an upper bound, not an answer.
 * Everything downstream of retrieval — the model's pick, grounding, the
 * also_considered slice — can still drop the right addon on the floor, and for
 * days it did: semantic candidates were retrieved correctly and then sliced off
 * by `alsoConsidered()`, so no user ever saw one, and no metric noticed.
 *
 * This script measures the LAST hop. It replays the SAME fixture
 * (`test/fixtures/search-eval.json`) against a live `POST /ask` and asks the
 * only question that matters to a user: did the expected addon come back?
 *
 *   hit@rec         fraction of rows where an expected addon appears in
 *                   `recommendations`. THE HEADLINE. It is what the user reads.
 *   hit@rec+also    same, counting `also_considered` too — the softer bar, and
 *                   the gap between the two is the model's picking quality.
 *   precision       of the recommendations returned, how many were expected.
 *                   Five near-misses is a worse answer than one correct one,
 *                   and a hit rate alone cannot see the difference.
 *   degraded        rows where the model was skipped (over budget, erroring, or
 *                   ungroundable). A degraded run is an OUTAGE, not a ranking
 *                   regression, and is called out rather than scored as one.
 *   no_good_match   rows where the model explicitly declined.
 *   semantic        rows that returned at least one addon absent from the pure
 *                   BM25 top-20 — the only direct measure of whether the
 *                   embedding work reaches a user at all.
 *
 * Split by row `kind` ("name" vs "concept"), because the two fail for different
 * reasons and averaging them hides both.
 *
 * COST. Every row is a live model call. Two DIFFERENT ceilings apply and
 * conflating them produces a false alarm: ASK_DAILY_BUDGET counts model CALLS
 * (350/day), while neurons bill against Workers AI's ~10k/day free allocation.
 * One row is one call and ~25 neurons, so the full 60-row fixture is 60/350
 * calls and ~1500/10000 neurons — comfortable on both. The estimate is printed
 * up front and anything over
 * MAX_ROWS_WITHOUT_CONSENT rows requires `--yes`. Prefer `--limit-rows 10`
 * while iterating.
 *
 * POLITENESS. `/ask` is rate-limited by ASK_LIMITER at 8 requests/min/IP, and
 * each row also makes one `/addons/search` call for the BM25 baseline. The run
 * is strictly sequential with a default 8s delay for that reason. Do not
 * parallelise it.
 *
 * AUTH. The cache bypass is admin-only (a public one would force a model call
 * per request and drain the daily budget), so `ADMIN_API_KEY` must be set in
 * the environment. Without the bypass every row after the first run would be
 * served from a seven-day KV cache and the eval would measure nothing.
 *
 * Usage:
 *   ADMIN_API_KEY=... node scripts/eval-ask.mjs --limit-rows 10
 *   ADMIN_API_KEY=... npm run eval:ask -- --yes
 *
 * Options:
 *   --base <url>          Worker base URL (default: production)
 *   --fixture <path>      Fixture path (default: test/fixtures/search-eval.json)
 *   --kind <name|concept> Only run rows of this kind
 *   --limit-rows <n>      Only run the first n rows (cheap smoke run)
 *   --delay <ms>          Delay between rows (default: 8000, ASK_LIMITER=8/min)
 *   --min-hit <0..1>      Exit non-zero if overall hit@rec is below this
 *   --yes                 Consent to spending neurons on more than 20 rows
 *   --json                Machine-readable output on stdout, nothing else
 *   --help                This text
 */

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_DIR = resolve(HERE, "..");

/** Roughly what one /ask model call costs on Workers AI. */
const NEURONS_PER_ROW = 25;

/** Default ASK_DAILY_BUDGET in wrangler.toml — for the cost estimate only. */
const DAILY_BUDGET = 350;

/** Workers AI free allocation. Billed separately from the call budget above. */
const FREE_NEURONS_PER_DAY = 10000;

/** Above this, the run needs explicit `--yes`. */
const MAX_ROWS_WITHOUT_CONSENT = 20;

/** The candidate cut-off /ask feeds the model, and the BM25 baseline width. */
const BM25_LIMIT = 20;

const DEFAULTS = {
  base: "https://kalpa-pack-hub.eso-toolkit.workers.dev",
  fixture: resolve(REPO_DIR, "test/fixtures/search-eval.json"),
  // ASK_LIMITER is 8/min per IP. 8s keeps one row per limiter slot with the
  // /addons/search call riding along under the separate read limiter.
  delay: 8000,
  minHit: 0,
  kind: null,
  limitRows: 0,
  yes: false,
  json: false,
};

const HELP = `
eval-ask - measure what Kalpa's /ask assistant actually delivers to a user.

  ADMIN_API_KEY=... node scripts/eval-ask.mjs
        [--base <url>] [--fixture <path>] [--kind name|concept]
        [--limit-rows <n>] [--delay <ms>] [--min-hit <0..1>] [--yes] [--json]

Reports hit@rec (headline), hit@rec+also, precision, degraded and
no_good_match rates, and semantic delivery - overall and per kind - plus a
per-row FAIL list. Exits 1 if overall hit@rec < --min-hit (default 0).

Costs real neurons: one model call and ~${NEURONS_PER_ROW} neurons per row. The full fixture
(~60 rows) is 60 of the ${DAILY_BUDGET}/day call budget and ~1500 of the
~${FREE_NEURONS_PER_DAY}/day free neuron allocation - comfortable on both. Runs over
${MAX_ROWS_WITHOUT_CONSENT} rows require --yes. Sequential with an ${DEFAULTS.delay}ms delay because
ASK_LIMITER allows 8 requests/min/IP.

Requires ADMIN_API_KEY: the answer-cache bypass is admin-only, and without it
every row would be served a cached answer and nothing would be measured.
`.trim();

function parseArgs(argv) {
  const opts = { ...DEFAULTS };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = () => {
      const v = argv[i + 1];
      if (v === undefined) throw new Error(`${arg} requires a value`);
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
      case "--yes":
        opts.yes = true;
        break;
      case "--base":
        opts.base = next().replace(/\/+$/, "");
        break;
      case "--fixture":
        opts.fixture = resolve(process.cwd(), next());
        break;
      case "--kind":
        opts.kind = next();
        break;
      case "--limit-rows":
        opts.limitRows = Number(next());
        break;
      case "--delay":
        opts.delay = Number(next());
        break;
      case "--min-hit":
        opts.minHit = Number(next());
        break;
      default:
        throw new Error(`unknown option: ${arg}`);
    }
  }
  if (!Number.isFinite(opts.delay) || opts.delay < 0) {
    throw new Error("--delay must be a non-negative number");
  }
  if (!Number.isInteger(opts.limitRows) || opts.limitRows < 0) {
    throw new Error("--limit-rows must be a non-negative integer");
  }
  if (!Number.isFinite(opts.minHit) || opts.minHit < 0 || opts.minHit > 1) {
    throw new Error("--min-hit must be between 0 and 1");
  }
  if (opts.kind && opts.kind !== "name" && opts.kind !== "concept") {
    throw new Error('--kind must be "name" or "concept"');
  }
  return opts;
}

/** Same shape and same validation as eval-search: one fixture, two harnesses. */
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

/**
 * One /ask call with the admin cache bypass.
 *
 * One retry on 429/5xx with a flat wait: the limiter is per-minute, so waiting
 * it out turns a rate limit back into a measurement instead of a fake failure.
 */
async function ask(base, apiKey, question) {
  for (let attempt = 0; attempt < 2; attempt += 1) {
    let res;
    try {
      res = await fetch(`${base}/ask`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          accept: "application/json",
          "X-API-Key": apiKey,
        },
        // Admin-only; ignored (and the answer served from cache) without a key.
        body: JSON.stringify({ question, no_cache: true }),
      });
    } catch (err) {
      if (attempt === 0) {
        await sleep(5000);
        continue;
      }
      return { error: `network: ${err.message}` };
    }
    if (res.status === 429 || res.status >= 500) {
      if (attempt === 0) {
        await sleep(res.status === 429 ? 20000 : 5000);
        continue;
      }
      return { error: `HTTP ${res.status}` };
    }
    if (!res.ok) return { error: `HTTP ${res.status}` };
    try {
      return { body: await res.json() };
    } catch (err) {
      return { error: `bad JSON: ${err.message}` };
    }
  }
  return { error: "exhausted retries" };
}

/** Pure-BM25 top-20 for the same question — the baseline semantic delivery is
 *  measured against. Failure here is not a row failure; it only makes the
 *  semantic column unknown for that row. */
async function bm25Ids(base, question) {
  const url = `${base}/addons/search?q=${encodeURIComponent(question)}&limit=${BM25_LIMIT}`;
  try {
    const res = await fetch(url, { headers: { accept: "application/json" } });
    if (!res.ok) return null;
    const body = await res.json();
    if (!Array.isArray(body.hits)) return null;
    return new Set(body.hits.map((h) => h.esoui_id));
  } catch {
    return null;
  }
}

const ids = (list) => (Array.isArray(list) ? list.map((r) => r.esoui_id) : []);
const mean = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : 0);
const pct = (x) => `${(x * 100).toFixed(1)}%`;

function aggregate(results) {
  // Precision is undefined for a row that returned nothing, so it is averaged
  // over the rows that actually recommended something. Scoring an empty answer
  // as precision 0 would make "declined to answer" look like "answered wrong".
  const withRecs = results.filter((r) => r.recommended.length > 0);
  return {
    rows: results.length,
    hit_at_rec: mean(results.map((r) => (r.hitRec ? 1 : 0))),
    hit_at_rec_also: mean(results.map((r) => (r.hitRecAlso ? 1 : 0))),
    precision: mean(withRecs.map((r) => r.precision)),
    rows_with_recommendations: withRecs.length,
    degraded_rate: mean(results.map((r) => (r.degraded ? 1 : 0))),
    no_good_match_rate: mean(results.map((r) => (r.noGoodMatch ? 1 : 0))),
    // Denominator is rows where the BM25 baseline was actually retrieved.
    semantic_delivery: mean(
      results.filter((r) => r.semanticDelivered !== null).map((r) => (r.semanticDelivered ? 1 : 0)),
    ),
    errors: results.filter((r) => r.error).length,
  };
}

function printSummary(label, agg) {
  console.log(
    `  ${label.padEnd(9)} n=${String(agg.rows).padStart(3)}  ` +
      `hit@rec ${pct(agg.hit_at_rec).padStart(6)}  ` +
      `hit@rec+also ${pct(agg.hit_at_rec_also).padStart(6)}  ` +
      `precision ${pct(agg.precision).padStart(6)}  ` +
      `degraded ${pct(agg.degraded_rate).padStart(6)}  ` +
      `no-match ${pct(agg.no_good_match_rate).padStart(6)}  ` +
      `semantic ${pct(agg.semantic_delivery).padStart(6)}`,
  );
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));

  const apiKey = process.env.ADMIN_API_KEY;
  if (!apiKey) {
    throw new Error(
      "ADMIN_API_KEY is not set. The /ask cache bypass is admin-only; without it " +
        "every row is served a cached answer and the run measures nothing.",
    );
  }

  let rows = loadFixture(opts.fixture);
  if (opts.kind) rows = rows.filter((r) => r.kind === opts.kind);
  if (opts.limitRows) rows = rows.slice(0, opts.limitRows);
  if (rows.length === 0) throw new Error("no rows to evaluate");

  const neurons = rows.length * NEURONS_PER_ROW;
  const log = opts.json ? () => {} : (...args) => console.log(...args);

  // Cost guard. Printed even in --json mode (on stderr) because a run that
  // spends real neurons should never be silent about it.
  //
  // Two DIFFERENT ceilings, and conflating them produces a false alarm:
  // ASK_DAILY_BUDGET counts MODEL CALLS (the KV counter increments by one per
  // call), while neurons are billed against Workers AI's ~10k/day free
  // allocation. One row is one call and ~25 neurons, so a 60-row run is 60/350
  // calls and ~1500/10000 neurons — comfortable on both.
  const callsLine =
    `${rows.length} model calls of the ${DAILY_BUDGET}/day budget` +
    (rows.length > DAILY_BUDGET ? " — THIS RUN WILL EXHAUST IT" : "");
  const neuronsLine =
    `~${neurons} neurons of the ~${FREE_NEURONS_PER_DAY}/day free allocation` +
    (neurons > FREE_NEURONS_PER_DAY ? " — THIS RUN MAY EXCEED IT" : "");
  const costLine = `Estimated cost: ${callsLine}; ${neuronsLine}.`;
  if (opts.json) console.error(costLine);

  if (rows.length > MAX_ROWS_WITHOUT_CONSENT && !opts.yes) {
    throw new Error(
      `${costLine}\n` +
        `Refusing to run more than ${MAX_ROWS_WITHOUT_CONSENT} rows without --yes. ` +
        `Use --limit-rows ${MAX_ROWS_WITHOUT_CONSENT} for a cheap smoke run, or pass --yes.`,
    );
  }

  const eta = Math.round(((rows.length - 1) * opts.delay) / 1000);
  log(`Kalpa /ask end-to-end eval`);
  log(`  base:    ${opts.base}`);
  log(`  fixture: ${opts.fixture}`);
  log(`  rows:    ${rows.length}  (delay=${opts.delay}ms, ~${eta}s, cache bypass ON)`);
  log(`  ${costLine}`);
  log("");

  const results = [];
  let servedFromCache = 0;

  for (let i = 0; i < rows.length; i += 1) {
    const row = rows[i];
    const expected = new Set(row.expected_esoui_ids);

    // Sequential on purpose: two limiters, one IP.
    const { body, error } = await ask(opts.base, apiKey, row.question);
    const baseline = await bm25Ids(opts.base, row.question);

    const recommended = ids(body?.recommendations);
    const also = ids(body?.also_considered);
    const union = [...recommended, ...also];
    const hitRec = recommended.some((id) => expected.has(id));
    const hitRecAlso = union.some((id) => expected.has(id));
    const precision = recommended.length
      ? recommended.filter((id) => expected.has(id)).length / recommended.length
      : 0;
    const semanticDelivered = baseline ? union.some((id) => !baseline.has(id)) : null;
    if (body?.cached) servedFromCache += 1;

    results.push({
      kind: row.kind,
      question: row.question,
      note: row.note,
      expected_esoui_ids: row.expected_esoui_ids,
      recommended,
      recommended_titles: (body?.recommendations ?? []).map((r) => `${r.esoui_id} "${r.title}"`),
      also_considered: also,
      answer: body?.answer ?? "",
      degraded: body?.degraded === true,
      noGoodMatch: body?.no_good_match === true,
      cached: body?.cached === true,
      hitRec,
      hitRecAlso,
      precision,
      semanticDelivered,
      error,
    });

    // A degraded row is flagged as such even when it hits: it scored the ranked
    // fallback, not the assistant.
    const flag = error
      ? "ERR "
      : hitRec
        ? body?.degraded
          ? "P/dg"
          : "PASS"
        : hitRecAlso
          ? "also"
          : body?.degraded
            ? "DEGR"
            : "FAIL";
    log(
      `  [${String(i + 1).padStart(2)}/${rows.length}] ${flag} ${row.kind.padEnd(7)} ` +
        `prec=${precision.toFixed(2)} recs=${recommended.length}` +
        `${semanticDelivered ? " +sem" : ""}  ${row.question}`,
    );
    if (i < rows.length - 1) await sleep(opts.delay);
  }

  const overall = aggregate(results);
  const byKind = {};
  for (const kind of [...new Set(results.map((r) => r.kind))].sort()) {
    byKind[kind] = aggregate(results.filter((r) => r.kind === kind));
  }
  const failures = results.filter((r) => !r.hitRec);
  const degraded = results.filter((r) => r.degraded);

  // A degraded run measured the retrieval fallback, not the assistant. Say so
  // loudly: without this, an exhausted neuron budget reads as a ranking
  // collapse and someone "fixes" ranking that was never broken.
  const warnings = [];
  if (degraded.length) {
    warnings.push(
      `${degraded.length}/${results.length} rows were DEGRADED (model skipped: over ` +
        `ASK_DAILY_BUDGET, erroring, or ungroundable). Those rows scored the ranked ` +
        `fallback, not the assistant. If the degraded rows are a contiguous tail, the ` +
        `daily budget ran out mid-run - re-run tomorrow or with fewer rows.`,
    );
  }
  if (servedFromCache) {
    warnings.push(
      `${servedFromCache}/${results.length} rows came back cached:true, so the admin ` +
        `cache bypass did NOT take effect - check ADMIN_API_KEY. Those rows may be up ` +
        `to seven days stale.`,
    );
  }

  if (opts.json) {
    console.log(
      JSON.stringify(
        {
          base: opts.base,
          fixture: opts.fixture,
          generated_at: new Date().toISOString(),
          estimated_neurons: neurons,
          warnings,
          overall,
          by_kind: byKind,
          failures: failures.map((f) => ({
            kind: f.kind,
            question: f.question,
            expected_esoui_ids: f.expected_esoui_ids,
            recommended: f.recommended_titles,
            also_considered: f.also_considered,
            degraded: f.degraded,
            no_good_match: f.noGoodMatch,
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
    console.log("SUMMARY  (hit@rec is the headline - what the user actually sees)");
    printSummary("overall", overall);
    for (const [kind, agg] of Object.entries(byKind)) printSummary(kind, agg);

    for (const warning of warnings) {
      console.log("");
      console.log(`WARNING: ${warning}`);
    }

    console.log("");
    if (failures.length === 0) {
      console.log("FAILURES: none - every row recommended an expected addon.");
    } else {
      console.log(
        `FAILURES (no expected addon in recommendations): ${failures.length}/${results.length}`,
      );
      for (const f of failures) {
        console.log("");
        console.log(`  ${f.kind}: "${f.question}"`);
        if (f.error) console.log(`    request error: ${f.error}`);
        if (f.degraded) console.log("    DEGRADED - the model was skipped for this row");
        if (f.noGoodMatch) console.log(`    model returned no_good_match`);
        console.log(`    expected: ${f.expected_esoui_ids.join(", ")}`);
        console.log(
          `    recommended: ${f.recommended_titles.length ? f.recommended_titles.join(", ") : "(none)"}`,
        );
        const rescued = f.also_considered.filter((id) => f.expected_esoui_ids.includes(id));
        if (rescued.length) {
          console.log(`    but present in also_considered: ${rescued.join(", ")}`);
        } else if (f.also_considered.length) {
          console.log(`    also_considered: ${f.also_considered.join(", ")}`);
        }
        if (f.answer) console.log(`    answer: ${f.answer}`);
        if (f.note) console.log(`    note: ${f.note}`);
      }
    }

    // Rows the model had in hand and did not pick: invisible to retrieval
    // recall, and the exact shape of the alsoConsidered delivery bug.
    const missedPicks = results.filter((r) => !r.hitRec && r.hitRecAlso);
    if (missedPicks.length) {
      console.log("");
      console.log(
        `NOT PICKED (expected addon reached the response, but only in also_considered): ` +
          `${missedPicks.length}`,
      );
      for (const m of missedPicks) {
        console.log(`  ${m.kind}: "${m.question}" - expected ${m.expected_esoui_ids.join(", ")}`);
      }
    }
  }

  if (overall.hit_at_rec < opts.minHit) {
    if (!opts.json) {
      console.error(`\nhit@rec ${pct(overall.hit_at_rec)} is below --min-hit ${pct(opts.minHit)}`);
    }
    process.exitCode = 1;
  }
}

main().catch((err) => {
  console.error(`eval-ask: ${err.message}`);
  process.exitCode = 2;
});
