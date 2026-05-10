import { chromium, type Browser, type Page } from "@playwright/test";

const CDP_ENDPOINT = "http://localhost:9222";

/**
 * Connect to the running Tauri app via Chrome DevTools Protocol.
 * The app must be running with `npm run tauri dev` (debug builds expose CDP on port 9222).
 *
 * Returns the browser and page. The caller should NOT close the browser between
 * tests — we're connecting to a live app, not launching one.
 */
export async function connectToTauri(): Promise<{ browser: Browser; page: Page }> {
  const browser = await chromium.connectOverCDP(CDP_ENDPOINT);
  const contexts = browser.contexts();

  if (contexts.length === 0) {
    throw new Error(
      "No browser contexts found. Make sure the Tauri app is running with `npm run tauri dev`.",
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
