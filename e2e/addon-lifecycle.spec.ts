import { existsSync } from "node:fs";
import path from "node:path";
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
/**
 * Is the fixture's folder actually on disk?
 *
 * The load-bearing assertion for every "prove it really happened" claim below.
 * Asserting the UI's row count instead does NOT work: `expectAddonListCount`
 * polls until the count MATCHES, so when the list already shows the expected
 * number it resolves immediately — before the rescan it is supposed to be
 * waiting for. A deletion-despite-undo regression would leave the restored row
 * on screen just long enough for that assertion to pass, and go green.
 */
function fixtureOnDisk(): boolean {
  return existsSync(path.join(sandboxDir, fixtureFolder));
}

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

  // NOT an install-path test. `debug_install_fixture_zip` runs the real
  // extractor but deliberately skips `install_addon` — no download, no metadata
  // record, no pre-operation snapshot, no dependency resolution. It is fixture
  // SETUP so the removal flow has something real on disk to delete. The product
  // install path remains uncovered here; see the follow-up on the PR.
  test("removes an addon and restores it with undo", async () => {
    const { browser, page } = await connectToTauri();

    try {
      await showAllAddons(page);

      const before = await addonList(page).getAttribute("aria-label");
      expect(before, "sandbox should start with no addons").toBe("Installed addons, 0 items");

      // Fixture setup, not the product install: the real extractor against a
      // local zip, so the spec depends on neither the network nor ESOUI's
      // current release. What is under test starts at the Remove click below.
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
      // a row over a folder that is already gone. Outlive the 3s removal timer,
      // then read the FILESYSTEM — the UI already shows 1, so a row-count
      // assertion here would resolve without waiting for anything.
      await page.waitForTimeout(3_500);
      expect(fixtureOnDisk(), "the addon folder was deleted despite undo").toBe(true);

      // The rescan then confirms the app agrees with the disk.
      await page.keyboard.press("Control+r");
      await expectAddonListCount(page, 1, "rescan lost the restored addon");

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
      expect(fixtureOnDisk(), "fixture folder missing before the removal").toBe(true);

      await row(page).click();
      await page.getByRole("button", { name: "Remove Addon" }).click();
      await expectAddonListCount(page, 0, "removing the fixture did not hide its row");

      // Let the undo window lapse so the removal commits, then prove it reached
      // the disk. Again the filesystem, not the row count: the UI already shows
      // 0 from the optimistic hide, so a count assertion proves nothing here.
      await page.waitForTimeout(4_000);
      expect(fixtureOnDisk(), "the undo window lapsed but the folder is still there").toBe(false);

      await page.keyboard.press("Control+r");
      await expectAddonListCount(page, 0, "the addon came back after the undo window lapsed");
    } finally {
      await browser.close();
    }
  });
});
