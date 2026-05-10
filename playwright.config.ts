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
 */
export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  retries: 0,
  workers: 1,
  fullyParallel: false,
});
