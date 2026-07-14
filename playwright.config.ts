import { defineConfig } from "@playwright/test";

/**
 * Playwright E2E tests connect to the live Tauri app via Chrome DevTools Protocol.
 *
 * Prerequisites:
 * 1. Run `npm run tauri dev` (debug builds expose CDP on port 9222)
 * 2. Wait for the app to fully load
 * 3. Run `npm run test:e2e`
 *
 * Tests connect via CDP — they drive the actual Tauri webview with real
 * IPC, real Rust backend, and real plugins. This is genuine E2E testing.
 *
 * IMPORTANT: Tests run serially (workers: 1) because they share a single
 * Tauri webview instance via CDP. Parallel execution causes state leaks.
 *
 * PLATFORM: E2E is Windows-only for now. The CDP endpoint is exposed through
 * WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS (WebView2-specific). WebKitGTK/WKWebView
 * expose the WebKit inspector protocol instead of CDP, so porting E2E to
 * macOS/Linux needs a different driver and is tracked as follow-up work.
 */
export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  retries: 0,
  workers: 1,
  fullyParallel: false,
});
