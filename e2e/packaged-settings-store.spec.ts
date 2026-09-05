import { expect, test } from "@playwright/test";
import { connectToPackagedTauri } from "./helpers";

/**
 * The capability grants `store:allow-get-store` and deliberately not
 * `store:allow-load`, because the plugin's `load` takes a caller-supplied path
 * and resolves it by pushing it onto the app-data directory with no containment
 * check — `..` and absolute paths escape, and its autosave then writes there.
 * `get_store` is a pure lookup that only returns stores native code opened.
 *
 * Neither half of that is reachable from a unit test. `src/lib/__tests__` mocks
 * the plugin module wholesale, so it proves which function the frontend calls
 * and nothing about whether the running app is allowed to call it; a mistyped or
 * missing permission surfaces only in a real app against the real ACL. The build
 * catches an identifier that does not exist, but not one that exists and is
 * wrong for what the frontend needs.
 *
 * A failure here is quiet in the worst way: `getSetting` swallows errors and
 * returns its fallback, so an unreachable store looks exactly like a first run —
 * the user's theme, AddOns folder and every other setting silently revert to
 * defaults while the app boots normally and every other packaged check passes.
 */
test.describe.serial("packaged settings store", () => {
  test("is reachable by lookup, and the webview cannot open paths @packaged", async () => {
    const { browser, page } = await connectToPackagedTauri();

    try {
      await expect
        .poll(() => page.evaluate(() => window.location.href))
        .toBe("http://tauri.localhost/");

      // `settings_store::ensure_open` runs in setup, so the lookup must find it.
      // Null here means every setting in the app silently fell back to default.
      const resourceId = await page.evaluate(() =>
        (
          window as unknown as {
            __TAURI_INTERNALS__: { invoke: (cmd: string, args: unknown) => Promise<unknown> };
          }
        ).__TAURI_INTERNALS__.invoke("plugin:store|get_store", { path: "settings.json" })
      );
      expect(resourceId, "native code must have opened settings.json").not.toBeNull();

      // ...and the permission that let the webview name its own path is gone.
      // Deliberately a plain filename rather than a traversal: if this assertion
      // ever fails, the regression should not also have written outside app data
      // to prove itself.
      const loadOutcome = await page.evaluate(async () => {
        try {
          await (
            window as unknown as {
              __TAURI_INTERNALS__: { invoke: (cmd: string, args: unknown) => Promise<unknown> };
            }
          ).__TAURI_INTERNALS__.invoke("plugin:store|load", {
            path: "kalpa-permission-probe.json",
          });
          return null;
        } catch (error) {
          return String(error);
        }
      });
      expect(loadOutcome, "the webview must not be able to open a store path").not.toBeNull();
    } finally {
      await browser.close();
    }
  });
});
