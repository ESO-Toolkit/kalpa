import { existsSync, readFileSync, statSync, utimesSync, writeFileSync } from "node:fs";
import path from "node:path";
import { test, expect, type Browser, type Page } from "@playwright/test";
import { addonList, connectToTauri } from "./helpers";

const sandbox = process.env.KALPA_E2E_SANDBOX_DIR ?? "";
const fixtures = process.env.KALPA_E2E_FIXTURES ?? "";
const folder = "KalpaMinionFixture";
const title = "Kalpa Minion Fixture";

test.describe.serial("Minion 4 external updates @sandbox @minion4", () => {
  test.describe.configure({ timeout: 130_000 });
  test.skip(!sandbox || !fixtures, "Requires the owned Minion 4 sandbox runner.");
  let browser: Browser;
  let page: Page;

  test.beforeAll(async () => {
    // The shared connector verifies the app-data namespace and registered root
    // before this spec can overwrite any fixture file.
    ({ browser, page } = await connectToTauri());
    expect(path.dirname(path.resolve(fixtures))).toBe(path.dirname(path.resolve(sandbox)));
  });
  test.afterAll(async () => {
    await browser?.close();
  });

  function savedObservation(): { version: string; lastUpdate: number } {
    const metadataPath = path.join(sandbox, "kalpa.json");
    if (!existsSync(metadataPath)) return { version: "", lastUpdate: 0 };
    const metadata = JSON.parse(readFileSync(metadataPath, "utf8"));
    const addon = metadata.addons[folder];
    return {
      version: addon?.installedVersion ?? "",
      lastUpdate: addon?.esouiLastUpdate ?? 0,
    };
  }

  async function expectCurrent(version: number): Promise<void> {
    await expect.poll(savedObservation, { timeout: 30_000 }).toEqual({
      version: `${version}.0`,
      lastUpdate: version * 1000,
    });
    const row = addonList(page).getByRole("option", { name: new RegExp(`^${title}`) });
    await expect(row).toContainText(`manifest-${version}`);
    // The scan can finish before the update check. Wait for the header's
    // checking indicator to clear before asserting the final update state.
    await expect(
      page
        .locator('[aria-live="polite"][aria-atomic="true"]')
        .filter({ hasText: /\baddons\b/ })
        .locator(".animate-spin")
    ).toHaveCount(0, { timeout: 30_000 });
    await expect(row).not.toHaveAttribute("aria-label", /Update available/);
    const scanned = await page.evaluate(async (addonsPath) => {
      const internals = (
        window as unknown as {
          __TAURI_INTERNALS__: {
            invoke: (
              command: string,
              args: unknown
            ) => Promise<
              {
                folderName: string;
                version: string;
              }[]
            >;
          };
        }
      ).__TAURI_INTERNALS__;
      return internals.invoke("scan_installed_addons", { addonsPath });
    }, sandbox);
    expect(scanned.find((addon) => addon.folderName === folder)?.version).toBe(
      `manifest-${version}`
    );
  }

  test("prefers Minion 4 at startup and observes two further external updates", async () => {
    await expectCurrent(2);
    for (const version of [3, 4]) {
      const minionPath = path.join(fixtures, "minion", "config", "eso.json");
      const minion = JSON.parse(readFileSync(minionPath, "utf8"));
      minion.installedGames[0].installedBundles["esoui:123"].version = `${version}.0`;
      writeFileSync(minionPath, JSON.stringify(minion));
      const filelistPath = path.join(fixtures, "filelist.json");
      const entries = JSON.parse(readFileSync(filelistPath, "utf8"));
      entries[0].version = `${version}.0`;
      entries[0].lastUpdate = version * 1000;
      writeFileSync(filelistPath, JSON.stringify(entries));

      // An in-place overwrite can preserve the original timestamp and length.
      // Verify the real scan/UI sees new manifest bytes despite that cache key.
      const manifest = path.join(sandbox, folder, `${folder}.txt`);
      const before = statSync(manifest);
      writeFileSync(
        manifest,
        readFileSync(manifest, "utf8").replace(`manifest-${version - 1}`, `manifest-${version}`)
      );
      utimesSync(manifest, before.atime, before.mtime);
      // Production forced-refresh requests coalesce for five seconds.
      await page.waitForTimeout(5_100);
      await page.getByRole("button", { name: "Refresh addons", exact: true }).click();
      await expectCurrent(version);
    }
    expect(readFileSync(path.join(fixtures, "minion", "minion.xml"), "utf8")).toContain(
      'ui-version="1.0"'
    );
  });
});
