import { existsSync, readdirSync } from "node:fs";
import path from "node:path";
import {
  chromium,
  expect,
  type Browser,
  type ConsoleMessage,
  type Locator,
  type Page,
  type Request,
} from "@playwright/test";

// 127.0.0.1, not `localhost`: WebView2's debug server binds IPv4 loopback only,
// and a host that resolves `localhost` to ::1 first refuses every connection.
const CDP_ENDPOINT = process.env.KALPA_CDP_ENDPOINT ?? "http://127.0.0.1:9222";
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

  // No single-shot pre-checks for "are there contexts / pages yet". They used to
  // sit here and they defeated the poll below: both threw before the loop was
  // ever entered, on exactly the startup race the loop exists to survive. The
  // deadline path already reports the URLs it saw and names the start command,
  // which is strictly more useful than "No pages found".

  // Pick the page that actually IS the app, rather than trusting index 0.
  // WebView2 exposes extra targets (about:blank, the sign-in webview, devtools),
  // and taking the first one intermittently attached to a page with no Tauri IPC
  // — a flake that reads like a product failure: "Tauri IPC is not exposed on
  // this page".
  //
  // Polled, because CDP answering does not mean the app's page exists yet. When
  // the runner owns the process it connects the instant the debug port opens,
  // and for a moment the only target is about:blank. Searching once turned that
  // startup race into a hard failure.
  // IPC presence alone is not enough to identify the main window. Kalpa opens a
  // second WebView2 for the ESO Logs sign-in, and that one carries Tauri
  // internals too — attaching to it produced "Origin header is not a valid URL"
  // from the first `evaluate` that passed arguments, which reads like a product
  // bug rather than the harness holding the wrong window. Require the app's own
  // origin as well.
  const isAppUrl = (url: string) =>
    url.startsWith(PACKAGED_ORIGIN) || /^https?:\/\/(127\.0\.0\.1|localhost):\d+\//.test(url);

  const deadline = Date.now() + 20_000;
  let seen: string[] = [];
  while (Date.now() < deadline) {
    const candidates = browser.contexts().flatMap((context) => context.pages());
    seen = candidates.map((p) => p.url());
    for (const candidate of candidates) {
      if (!isAppUrl(candidate.url())) continue;
      try {
        await candidate.waitForLoadState("domcontentloaded");
        const hasIpc = await candidate.evaluate(
          () => "__TAURI_INTERNALS__" in (window as unknown as Record<string, unknown>)
        );
        if (hasIpc) return { browser, page: candidate };
      } catch {
        // A target that vanished or refuses evaluation is not the app page.
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }

  throw new Error(
    `No page exposing Tauri IPC appeared within 20s — is the app running via ${startCommand}? ` +
      `Last saw: ${seen.join(", ") || "no pages"}`
  );
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

const ADDON_LIST = '[role="listbox"][aria-roledescription="addon list"]';

/** The installed-addon list container. */
export function addonList(page: Page): Locator {
  return page.locator(ADDON_LIST);
}

/**
 * How many rows the installed-addon list is currently showing.
 *
 * Read from the list's own aria-label because that reflects the FILTERED list.
 * The "All(N)" filter tab counts the unfiltered set (addon-list.tsx builds
 * filterCounts from `allAddons`), so it does not move when a search narrows the
 * rows and cannot be used to prove filtering happened.
 *
 * Throws when the list is absent or its label carries no count, so a caller can
 * never silently read a fabricated zero.
 */
export async function readAddonListCount(page: Page): Promise<number> {
  const label = await addonList(page).getAttribute("aria-label", { timeout: 10_000 });
  const match = /Installed addons, (\d+) items/.exec(label ?? "");
  if (!match) {
    throw new Error(
      `Addon list carried no row count (aria-label: ${label ?? "missing"}). ` +
        "The list markup changed or the app is not on the installed view."
    );
  }
  return Number(match[1]);
}

/** Wait for the list to settle on `expected` rows — search filtering is deferred. */
export async function expectAddonListCount(
  page: Page,
  expected: number,
  message: string
): Promise<void> {
  await expect(addonList(page), message).toHaveAttribute(
    "aria-label",
    `Installed addons, ${expected} items`,
    { timeout: 5_000 }
  );
}

/** A filter tab in the installed-addon list, e.g. "All", "Libs", "Outdated". */
export function addonFilterTab(page: Page, label: string): Locator {
  return page.locator(`[role="tab"][aria-label="Filter by ${label}"]`);
}

/** The count a filter tab advertises in its own text, e.g. "Libs(49)" -> 49. */
export async function readFilterTabCount(page: Page, label: string): Promise<number> {
  const text = await addonFilterTab(page, label).textContent({ timeout: 5_000 });
  const match = /\((\d+)\)\s*$/.exec(text ?? "");
  if (!match) {
    throw new Error(`Filter tab "${label}" carried no count (text: ${text ?? "missing"}).`);
  }
  return Number(match[1]);
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
