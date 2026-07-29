import { existsSync, readdirSync } from "node:fs";
import path from "node:path";
import {
  chromium,
  type Browser,
  type ConsoleMessage,
  type Page,
  type Request,
} from "@playwright/test";

const CDP_ENDPOINT = process.env.KALPA_CDP_ENDPOINT ?? "http://localhost:9222";
const PACKAGED_ORIGIN = "http://tauri.localhost/";

/**
 * Connect to the running Tauri app via Chrome DevTools Protocol.
 * The app must be running with `npm run tauri dev` (debug builds expose CDP on port 9222).
 *
 * Returns the browser and page. The caller should NOT close the browser between
 * tests - we're connecting to a live app, not launching one.
 */
export async function connectToTauri(): Promise<{ browser: Browser; page: Page }> {
  return connectToTauriAt(CDP_ENDPOINT, "npm run tauri dev");
}

/**
 * Connect to the packaged-build verifier's app instance.
 *
 * The verifier owns the Kalpa process lifetime. This helper refuses the dev
 * server origin so a habitual `tauri dev` process cannot satisfy release-gate tests.
 */
export async function connectToPackagedTauri(): Promise<{ browser: Browser; page: Page }> {
  const { browser, page } = await connectToTauriAt(CDP_ENDPOINT, "npm run test:packaged");
  const origin = await page.evaluate(() => window.location.origin + "/");
  if (origin !== PACKAGED_ORIGIN) {
    await browser.close();
    throw new Error(
      `Packaged gate connected to ${origin}, expected ${PACKAGED_ORIGIN}. This is probably the Vite dev server.`
    );
  }
  return { browser, page };
}

async function connectToTauriAt(
  endpoint: string,
  startCommand: string
): Promise<{ browser: Browser; page: Page }> {
  const browser = await chromium.connectOverCDP(endpoint);
  const contexts = browser.contexts();

  if (contexts.length === 0) {
    throw new Error(
      `No browser contexts found. Make sure the Tauri app is running with ${startCommand}.`
    );
  }

  const pages = contexts[0].pages();
  if (pages.length === 0) {
    throw new Error("No pages found in the Tauri webview.");
  }

  const page = pages[0];
  await page.waitForLoadState("domcontentloaded");

  return { browser, page };
}

export interface ChunkLoadRecorder {
  errors: string[];
  dispose: () => void;
}

export function createChunkLoadRecorder(page: Page): ChunkLoadRecorder {
  const errors: string[] = [];
  const onConsole = (message: ConsoleMessage) => {
    if (["error", "warning"].includes(message.type())) {
      errors.push(`[console:${message.type()}] ${message.text()}`);
    }
  };
  const onPageError = (error: Error) => {
    errors.push(`[pageerror] ${error.message}`);
  };
  const onRequestFailed = (request: Request) => {
    if (request.resourceType() === "script" || request.url().includes("/assets/")) {
      errors.push(`[requestfailed] ${request.url()}: ${request.failure()?.errorText ?? "unknown"}`);
    }
  };

  page.on("console", onConsole);
  page.on("pageerror", onPageError);
  page.on("requestfailed", onRequestFailed);

  return {
    errors,
    dispose: () => {
      page.off("console", onConsole);
      page.off("pageerror", onPageError);
      page.off("requestfailed", onRequestFailed);
    },
  };
}

export function resolveBuiltChunkUrls(chunkStems: readonly string[]): Record<string, string> {
  const assetsDir = path.resolve(process.cwd(), "dist", "assets");
  if (!existsSync(assetsDir)) {
    throw new Error("dist/assets does not exist. Run the packaged build verifier first.");
  }

  const assets = readdirSync(assetsDir).filter((name) => name.endsWith(".js"));
  const urls: Record<string, string> = {};

  for (const stem of chunkStems) {
    const escaped = stem.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const matches = assets.filter((name) => new RegExp(`^${escaped}-.*\\.js$`).test(name));
    if (matches.length !== 1) {
      throw new Error(
        `Expected exactly one built JS chunk for ${stem}, found ${matches.length}: ${matches.join(", ") || "none"}`
      );
    }
    urls[stem] = new URL(`/assets/${matches[0]}`, PACKAGED_ORIGIN).toString();
  }

  return urls;
}

export async function importBuiltChunk(
  page: Page,
  url: string
): Promise<{ exportNames: string[] }> {
  return page.evaluate(async (chunkUrl) => {
    const imported = await import(/* @vite-ignore */ chunkUrl);
    return { exportNames: Object.keys(imported).sort() };
  }, url);
}

/**
 * Dismiss any open dialogs/modals and return the app to its base state.
 * Call this at the start of each test to prevent state leaks.
 */
export async function resetAppState(page: Page): Promise<void> {
  // Press Escape a few times to close any open dialogs/menus/popovers
  for (let i = 0; i < 3; i++) {
    await page.keyboard.press("Escape");
    await page.waitForTimeout(200);
  }

  // Wait for dialogs to close
  await page.waitForTimeout(300);

  // Clear any focused input
  await page.evaluate(() => {
    if (document.activeElement instanceof HTMLElement) {
      document.activeElement.blur();
    }
  });
}
