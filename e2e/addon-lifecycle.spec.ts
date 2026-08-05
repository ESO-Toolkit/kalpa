import { test, expect, type Page } from "@playwright/test";
import {
  addonFilterTab,
  addonList,
  connectToTauri,
  expectAddonListCount,
  resetAppState,
} from "./helpers";

/**
 * The first e2e spec that MUTATES an AddOns folder.
 *
 * Every other spec attaches to whatever `npm run tauri dev` is running, which is
 * the developer's real ESO install — so the whole suite has only ever read. This
 * one runs under `npm run test:e2e:sandbox`, which owns the app and points it at
 * a throwaway folder via the debug-only `KALPA_ADDONS_DIR` override.
 *
 * It refuses to run otherwise. A destructive spec that silently fell back to the
 * live install would delete a real addon, so the guard below is the most
 * important assertion in the file.
 */

const sandboxDir = process.env.KALPA_E2E_SANDBOX_DIR ?? "";
const fixtureZip = process.env.KALPA_E2E_FIXTURE_ZIP ?? "";
const fixtureFolder = process.env.KALPA_E2E_FIXTURE_FOLDER ?? "";
const fixtureTitle = "Kalpa E2E Fixture";

/** Call a Tauri command from inside the webview. */
async function invoke<T>(page: Page, command: string, args: Record<string, unknown>): Promise<T> {
  return page.evaluate(
    ([cmd, payload]) => {
      const internals = (
        window as unknown as {
          __TAURI_INTERNALS__?: { invoke: (c: string, a: unknown) => Promise<unknown> };
        }
      ).__TAURI_INTERNALS__;
      if (!internals) throw new Error("Tauri IPC is not exposed on this page.");
      return internals.invoke(cmd as string, payload) as Promise<T>;
    },
    [command, args] as const
  ) as Promise<T>;
}

const row = (page: Page) =>
  addonList(page).locator(`[role="option"][aria-label^="${fixtureTitle}"]`);

/**
 * Put the list on the unfiltered "All" tab.
 *
 * The sandbox overrides the AddOns folder but shares the app's settings.json,
 * and `filterMode` is persisted — so a developer whose last session ended on
 * Libs or Outdated would boot the sandbox with that filter live. The fixture is
 * a plain, up-to-date addon, so it would be filtered out and every row-count
 * assertion below would read 0 for a reason that has nothing to do with the
 * lifecycle under test.
 */
async function showAllAddons(page: Page): Promise<void> {
  await resetAppState(page);
  const allTab = addonFilterTab(page, "All");
  await expect(allTab, '"All" filter tab not found').toBeVisible({ timeout: 10_000 });
  if ((await allTab.getAttribute("aria-selected")) !== "true") {
    await allTab.click();
  }
  await expect(allTab, "could not select the All filter").toHaveAttribute("aria-selected", "true", {
    timeout: 5_000,
  });
}

test.describe.serial("Addon lifecycle @sandbox", () => {
  test.skip(
    !sandboxDir || !fixtureZip || !fixtureFolder,
    "Run via `npm run test:e2e:sandbox` — this spec mutates the AddOns folder and needs a sandbox."
  );

  test("refuses to run against anything but the sandbox folder", async () => {
    const { browser, page } = await connectToTauri();
    try {
      // The app must have booted onto the override, not the developer's install.
      // If this ever passes against a real folder, every assertion below is
      // deleting someone's addons.
      const active = await invoke<string | null>(page, "debug_addons_dir_override", {});
      expect(active, "app is not running under KALPA_ADDONS_DIR").toBeTruthy();
      expect(active?.toLowerCase()).toBe(sandboxDir.toLowerCase());
    } finally {
      await browser.close();
    }
  });

  test("installs a fixture, removes it, and undo restores it", async () => {
    const { browser, page } = await connectToTauri();

    try {
      await showAllAddons(page);

      const before = await addonList(page).getAttribute("aria-label");
      expect(before, "sandbox should start with no addons").toBe("Installed addons, 0 items");

      // Install through the real extractor, from a local zip so the spec never
      // depends on the network or on ESOUI's current release.
      const extracted = await invoke<string[]>(page, "debug_install_fixture_zip", {
        addonsPath: sandboxDir,
        zipPath: fixtureZip,
      });
      expect(extracted.length, "fixture zip extracted nothing").toBeGreaterThan(0);

      // Refresh brings the new folder into the list the way the UI would after
      // an install.
      await page.keyboard.press("Control+r");
      await expectAddonListCount(page, 1, "installed fixture did not appear in the list");
      await expect(row(page), "fixture row is missing").toBeVisible({ timeout: 5_000 });

      // --- the destructive part ---
      await row(page).click();
      const removeButton = page.getByRole("button", { name: "Remove Addon" });
      await expect(removeButton, "detail pane has no Remove Addon button").toBeVisible({
        timeout: 5_000,
      });
      await removeButton.click();

      await expectAddonListCount(page, 0, "removing the fixture did not hide its row");

      // Undo inside the 3s window must put the row back — including its update
      // state, which is what the removal queue carries per entry.
      const undo = page.getByRole("button", { name: "Undo" });
      await expect(undo, "no Undo affordance on the removal toast").toBeVisible({ timeout: 2_000 });
      await undo.click();

      await expectAddonListCount(page, 1, "undo did not restore the removed addon");
      await expect(row(page), "restored fixture row is missing").toBeVisible({ timeout: 5_000 });

      // And the undo must have cancelled the real deletion, not just re-rendered
      // a row over a folder that is already gone. A rescan proves the folder is
      // still on disk.
      await page.waitForTimeout(3_500); // outlive the 3s removal timer
      await page.keyboard.press("Control+r");
      await expectAddonListCount(page, 1, "the addon folder was deleted despite undo");

      await page.screenshot({ path: "e2e/screenshots/addon-lifecycle-restored.png" });
    } finally {
      await browser.close();
    }
  });

  test("a committed removal actually deletes the folder", async () => {
    const { browser, page } = await connectToTauri();

    try {
      await showAllAddons(page);
      await expect(row(page), "fixture should still be installed").toBeVisible({ timeout: 5_000 });

      await row(page).click();
      await page.getByRole("button", { name: "Remove Addon" }).click();
      await expectAddonListCount(page, 0, "removing the fixture did not hide its row");

      // Let the undo window lapse so the removal commits, then prove it stuck.
      await page.waitForTimeout(4_000);
      await page.keyboard.press("Control+r");
      await expectAddonListCount(page, 0, "the addon came back after the undo window lapsed");
    } finally {
      await browser.close();
    }
  });
});
