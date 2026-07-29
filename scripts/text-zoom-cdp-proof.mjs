import { chromium } from "@playwright/test";
import { execFileSync, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const CDP_ENDPOINT = "http://localhost:9222";
const STOPS = [1, 1.1, 1.25, 1.5];
const MIN_CSS_WIDTH_AT_MAX_ZOOM = 800;
const MIN_CSS_HEIGHT_AT_MAX_ZOOM = 500;

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "kalpa.exe");
const webviewUserDataDir = path.join(process.env.TEMP ?? repoRoot, "kalpa-text-zoom-webview2-proof");

function killKalpa() {
  try {
    execFileSync("taskkill", ["/IM", "kalpa.exe", "/F", "/T"], { stdio: "ignore" });
  } catch {
    // No stale Kalpa process was running.
  }
}

async function waitForCdp(timeoutMs = 30_000) {
  const started = Date.now();
  let lastError;
  while (Date.now() - started < timeoutMs) {
    try {
      const response = await fetch(`${CDP_ENDPOINT}/json/version`);
      if (response.ok) return;
      lastError = new Error(`CDP returned ${response.status}`);
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`Timed out waiting for CDP on ${CDP_ENDPOINT}: ${lastError}`);
}

async function getPage(browser) {
  const context = browser.contexts()[0];
  if (!context) throw new Error("No CDP browser context found.");

  for (let attempt = 0; attempt < 80; attempt++) {
    const page = context.pages().find((candidate) => !candidate.url().startsWith("devtools://"));
    if (page) {
      await page.waitForLoadState("domcontentloaded", { timeout: 15_000 }).catch(() => {});
      await page.waitForFunction(() => Boolean(window.__TAURI_INTERNALS__?.invoke), null, {
        timeout: 15_000,
      });
      return page;
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }

  throw new Error("No Tauri webview page found over CDP.");
}

async function invoke(page, cmd, args = {}) {
  return page.evaluate(
    async ({ cmd: command, args: invokeArgs }) => window.__TAURI_INTERNALS__.invoke(command, invokeArgs),
    { cmd, args },
  );
}

async function readMetrics(page) {
  const dom = await page.evaluate(() => ({
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
    outerWidth: window.outerWidth,
    outerHeight: window.outerHeight,
    dpr: window.devicePixelRatio,
    visualViewportScale: window.visualViewport?.scale ?? null,
  }));
  const label = await page.evaluate(() => window.__TAURI_INTERNALS__.metadata.currentWindow.label);
  const nativeInner = await invoke(page, "plugin:window|inner_size", { label });
  return { ...dom, nativeInner };
}

async function waitForMetricChange(page, previous) {
  if (!previous) {
    await page.waitForTimeout(350);
    return readMetrics(page);
  }

  const started = Date.now();
  let metrics = await readMetrics(page);
  while (
    Date.now() - started < 5_000 &&
    metrics.innerWidth === previous.innerWidth &&
    metrics.innerHeight === previous.innerHeight
  ) {
    await page.waitForTimeout(100);
    metrics = await readMetrics(page);
  }
  return metrics;
}

function formatNativeInner(nativeInner) {
  if (nativeInner?.Physical) {
    return `${nativeInner.Physical.width}x${nativeInner.Physical.height}`;
  }
  if (typeof nativeInner?.width === "number") {
    return `${nativeInner.width}x${nativeInner.height}`;
  }
  return JSON.stringify(nativeInner);
}

function printTable(rows) {
  console.log("| factor | css viewport | dpr | outer | native inner | visualViewport.scale |");
  console.log("| ---: | ---: | ---: | ---: | ---: | ---: |");
  for (const row of rows) {
    console.log(
      `| ${row.factor} | ${row.innerWidth}x${row.innerHeight} | ${row.dpr} | ${row.outerWidth}x${row.outerHeight} | ${formatNativeInner(row.nativeInner)} | ${row.visualViewportScale ?? "n/a"} |`,
    );
  }
}

if (!existsSync(exePath)) {
  throw new Error(`Missing ${exePath}. Build it first with: npm run tauri -- build --debug --no-bundle`);
}

killKalpa();
const child = spawn(exePath, [], {
  cwd: path.dirname(exePath),
  stdio: "ignore",
  windowsHide: true,
  env: {
    ...process.env,
    KALPA_FORCE_WEBVIEW: "1",
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:
      "--remote-debugging-port=9222 --enable-features=msWebView2CodeCache",
    WEBVIEW2_USER_DATA_FOLDER: webviewUserDataDir,
  },
});

let browser;
try {
  await waitForCdp();
  browser = await chromium.connectOverCDP(CDP_ENDPOINT);
  const page = await getPage(browser);
  const label = await page.evaluate(() => window.__TAURI_INTERNALS__.metadata.currentWindow.label);
  const scaleFactor = await invoke(page, "plugin:window|scale_factor", { label });
  const fixedPhysicalSize = {
    Physical: {
      width: Math.ceil(1200 * scaleFactor),
      height: Math.ceil(750 * scaleFactor),
    },
  };

  await invoke(page, "plugin:window|set_min_size", { label, value: null });
  await invoke(page, "plugin:window|set_size", { label, value: fixedPhysicalSize });
  await page.waitForTimeout(500);

  const rows = [];
  let previous;
  for (const factor of STOPS) {
    await invoke(page, "set_text_zoom", { factor });
    const metrics = await waitForMetricChange(page, previous);
    const row = { factor, ...metrics };
    rows.push(row);
    previous = row;
  }

  printTable(rows);

  for (let i = 1; i < rows.length; i++) {
    const prev = rows[i - 1];
    const current = rows[i];
    if (current.innerWidth === prev.innerWidth && current.innerHeight === prev.innerHeight) {
      throw new Error(
        `CSS viewport did not change between ${prev.factor} and ${current.factor}: ${current.innerWidth}x${current.innerHeight}`,
      );
    }
  }

  const maxZoom = rows.find((row) => row.factor === 1.5);
  if (!maxZoom) throw new Error("Missing 1.5 zoom row.");
  if (
    maxZoom.innerWidth < MIN_CSS_WIDTH_AT_MAX_ZOOM ||
    maxZoom.innerHeight < MIN_CSS_HEIGHT_AT_MAX_ZOOM
  ) {
    throw new Error(
      `1.5 zoom viewport too small: ${maxZoom.innerWidth}x${maxZoom.innerHeight}; expected at least ${MIN_CSS_WIDTH_AT_MAX_ZOOM}x${MIN_CSS_HEIGHT_AT_MAX_ZOOM}`,
    );
  }
} finally {
  if (browser) await browser.close().catch(() => {});
  child.kill();
  killKalpa();
}