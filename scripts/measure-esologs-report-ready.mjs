#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

async function loadChromium() {
  try {
    return (await import("@playwright/test")).chromium;
  } catch (error) {
    const detail = error?.code === "ERR_MODULE_NOT_FOUND" ? "" : ` (${error.message || error})`;
    throw new Error(
      "Playwright is not installed in this worktree. Run `npm install` first, " +
        "then retry `npm run measure:esologs-report -- <report-url>`." +
        detail
    );
  }
}

function usage() {
  return `Usage:
  node scripts/measure-esologs-report-ready.mjs <report-url> [options]

Options:
  --cookie <header>      ESO Logs Cookie header. Defaults to KALPA_BENCH_ESOLOGS_COOKIE.
  --timeout-ms <ms>      Maximum wait. Defaults to KALPA_BENCH_REPORT_READY_TIMEOUT_MS or 180000.
  --poll-ms <ms>         DOM poll interval. Defaults to 500.
  --headed               Show the browser.
  --json                 Print only JSON.

The script waits until ESO Logs' report UI renders the encounter/fight list for
the report. It does not upload.

KALPA_BENCH_* values can also be supplied in .env.bench.local at the repo root,
the main checkout for a linked worktree, or KALPA_BENCH_ENV_FILE.`;
}

function parseEnvValue(raw) {
  const value = raw.trim();
  if (value.length >= 2) {
    const first = value[0];
    const last = value[value.length - 1];
    if ((first === '"' && last === '"') || (first === "'" && last === "'")) {
      return value.slice(1, -1);
    }
  }
  return value;
}

function loadBenchEnvFile() {
  const scriptDir = dirname(fileURLToPath(import.meta.url));
  const repoRoot = join(scriptDir, "..");
  const candidates = [
    process.env.KALPA_BENCH_ENV_FILE || "",
    join(repoRoot, ".env.bench.local"),
    gitCommonWorktreeEnvFile(repoRoot),
    join(process.cwd(), ".env.bench.local"),
  ];
  const seen = new Set();
  for (const file of candidates) {
    if (seen.has(file)) continue;
    seen.add(file);
    if (!existsSync(file)) continue;
    for (const rawLine of readFileSync(file, "utf8").split(/\r?\n/u)) {
      const trimmed = rawLine.trim();
      if (!trimmed || trimmed.startsWith("#")) continue;
      const line = trimmed.startsWith("export ") ? trimmed.slice("export ".length) : trimmed;
      const eq = line.indexOf("=");
      if (eq <= 0) continue;
      const key = line.slice(0, eq).trim();
      if (!key || !(process.env[key] == null || process.env[key] === "")) continue;
      process.env[key] = parseEnvValue(line.slice(eq + 1));
    }
    break;
  }
}

function gitCommonWorktreeEnvFile(repoRoot) {
  try {
    const commonDir = execFileSync(
      "git",
      ["rev-parse", "--path-format=absolute", "--git-common-dir"],
      {
        cwd: repoRoot,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      }
    ).trim();
    if (!commonDir) return "";
    return join(dirname(commonDir), ".env.bench.local");
  } catch {
    return "";
  }
}

loadBenchEnvFile();

function parseArgs(argv) {
  const args = {
    url: process.env.KALPA_BENCH_REPORT_URL || "",
    cookie: process.env.KALPA_BENCH_ESOLOGS_COOKIE || "",
    timeoutMs: Number(process.env.KALPA_BENCH_REPORT_READY_TIMEOUT_MS || 180000),
    pollMs: 500,
    headed: process.env.KALPA_BENCH_REPORT_READY_HEADED === "1",
    json: false,
  };

  const positional = [];
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      console.log(usage());
      process.exit(0);
    }
    if (arg === "--cookie") {
      args.cookie = argv[++i] || "";
    } else if (arg === "--timeout-ms") {
      args.timeoutMs = Number(argv[++i]);
    } else if (arg === "--poll-ms") {
      args.pollMs = Number(argv[++i]);
    } else if (arg === "--headed") {
      args.headed = true;
    } else if (arg === "--json") {
      args.json = true;
    } else if (arg.startsWith("--")) {
      throw new Error(`Unknown option: ${arg}`);
    } else {
      positional.push(arg);
    }
  }
  if (positional[0]) args.url = positional[0];
  if (!args.url) throw new Error("Missing report URL.");
  if (!Number.isFinite(args.timeoutMs) || args.timeoutMs <= 0) {
    throw new Error("timeout-ms must be a positive number.");
  }
  if (!Number.isFinite(args.pollMs) || args.pollMs <= 0) {
    throw new Error("poll-ms must be a positive number.");
  }
  return args;
}

function cookiesFromHeader(header) {
  if (!header.trim()) return [];
  const out = [];
  for (const raw of header.split(";")) {
    const trimmed = raw.trim();
    if (!trimmed) continue;
    const eq = trimmed.indexOf("=");
    if (eq <= 0) continue;
    const name = trimmed.slice(0, eq).trim();
    const value = trimmed.slice(eq + 1).trim();
    if (!name) continue;
    for (const url of ["https://www.esologs.com", "https://esologs.com"]) {
      out.push({ name, value, url });
    }
  }
  return out;
}

async function reportState(page) {
  return page.evaluate(() => {
    const textOf = (selector) => document.querySelector(selector)?.textContent?.trim() || "";
    const bossText = textOf("#filter-fight-boss-text");
    const detailsText = textOf("#filter-fight-details-text");
    const title = document.title || "";
    const bodyText = document.body?.innerText || "";
    const lowerBody = bodyText.toLowerCase();
    const error =
      lowerBody.includes("report not found") ||
      lowerBody.includes("this report does not exist") ||
      lowerBody.includes("you do not have permission") ||
      lowerBody.includes("this report is private")
        ? bodyText.slice(0, 500)
        : "";
    const topSelectorReady =
      Boolean(bossText) && !/fetching fights|loading/i.test(bossText) && detailsText !== "None";
    const encounterListReady =
      bodyText.includes("Encounters and Trash Fights") &&
      /All Encounters\s*\(/.test(bodyText) &&
      /\b(Kill|Wipe|Trash Fights)\b/.test(bodyText);
    const ready = !error && (topSelectorReady || encounterListReady);
    const readySource = topSelectorReady
      ? "top-selector"
      : encounterListReady
        ? "encounter-list"
        : "";
    return { ready, readySource, error, bossText, detailsText, title };
  });
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const started = performance.now();
  const chromium = await loadChromium();
  const browser = await chromium.launch({ headless: !args.headed });
  try {
    const context = await browser.newContext();
    const cookies = cookiesFromHeader(args.cookie);
    if (cookies.length > 0) await context.addCookies(cookies);
    const page = await context.newPage();
    page.setDefaultTimeout(args.timeoutMs);
    await page.goto(args.url, { waitUntil: "domcontentloaded", timeout: args.timeoutMs });

    let state = await reportState(page);
    while (!state.ready) {
      if (state.error) throw new Error(`ESO Logs report error: ${state.error}`);
      const elapsedMs = performance.now() - started;
      if (elapsedMs >= args.timeoutMs) {
        throw new Error(
          `Timed out after ${Math.round(elapsedMs)} ms waiting for report readiness ` +
            `(boss="${state.bossText}", details="${state.detailsText}")`
        );
      }
      await page.waitForTimeout(args.pollMs);
      state = await reportState(page);
    }

    const readyMs = Math.round(performance.now() - started);
    const result = {
      url: args.url,
      readyMs,
      readySource: state.readySource,
      bossText: state.bossText,
      detailsText: state.detailsText,
      title: state.title,
    };
    if (args.json) {
      console.log(JSON.stringify(result));
    } else {
      console.log("\n=== ESO LOGS REPORT READY ===");
      console.log(`  url          : ${result.url}`);
      console.log(`  ready        : ${result.readyMs} ms`);
      console.log(`  source       : ${result.readySource}`);
      console.log(`  selection    : ${result.bossText}`);
      console.log(`  details      : ${result.detailsText}`);
      console.log(`  title        : ${result.title}`);
    }
  } finally {
    await browser.close();
  }
}

main().catch((err) => {
  console.error(err.message || err);
  process.exit(1);
});
