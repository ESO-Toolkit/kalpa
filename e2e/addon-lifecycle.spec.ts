import { existsSync } from "node:fs";
import path from "node:path";
import { test, expect, type Browser, type Page } from "@playwright/test";
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
 * live install would delete a real addon.
 *
 * The real guard is NOT in this file. It is the `#[cfg(debug_assertions)]` block
 * in `set_addons_path`, which refuses to register any folder but the override
 * while `KALPA_ADDONS_DIR` is set. That distinction is load-bearing: by the time
 * a spec can assert anything the app has already booted, scanned, auto-linked
 * and — with `autoUpdate` on in the developer's real `settings.json` — run a
 * whole batch update. A spec cannot prevent that; failing the boot can.
 *
 * What the check below adds is proof that the guard is doing its job in THIS
 * run, asserted through the app's registered path rather than the environment.
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

/** Select the fixture through the listbox's keyboard contract. */
async function selectFixture(page: Page): Promise<void> {
  const list = addonList(page);
  await list.focus();
  await list.press("Home");
  await expect(row(page), "fixture row was not selected").toHaveAttribute("aria-selected", "true");
}

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
    await allTab.press("Enter");
  }
  await expect(allTab, "could not select the All filter").toHaveAttribute("aria-selected", "true", {
    timeout: 5_000,
  });
}

test.describe.serial("Addon lifecycle @sandbox", () => {
  // The readiness probe has an explicit 120s application-state deadline for
  // cold WebView2 machines. Let the beforeAll hook reach that deadline so a
  // genuine readiness failure reports its IPC/page diagnostics instead of
  // Playwright aborting the hook at its 30s default.
  test.describe.configure({ timeout: 130_000 });

  let browser: Browser;
  let page: Page;

  test.skip(
    !sandboxDir || !fixtureZip || !fixtureFolder,
    "Run via `npm run test:e2e:sandbox` — this spec mutates the AddOns folder and needs a sandbox."
  );

  test.beforeAll(async () => {
    ({ browser, page } = await connectToTauri());
  });

  test.afterAll(async () => {
    await browser.close();
  });

  test("refuses to run against anything but the sandbox folder", async () => {
    // Ask the app what folder it REGISTERED, not what the environment says.
    //
    // Reading `debug_addons_dir_override` back proved nothing: it is a pure
    // function of the env var this runner set moments earlier, so it returned
    // the sandbox whether or not the app had booted onto it. Deleting the
    // `sandboxPath || storedPath` line in `initializeApp` left it green.
    //
    // `scan_installed_addons` goes through `require_allowed_path`, which
    // compares against what `set_addons_path` actually stored — so it can only
    // succeed if the live registration IS the sandbox.
    // One assertion is enough, and a second would be worse than none.
    // `require_allowed_path` compares canonical equality against the single
    // registered folder, so this call succeeding means the registration IS the
    // sandbox — had the app booted onto the real install, it would be refused.
    // Asserting that some OTHER path is rejected proves nothing: a path that
    // does not exist fails in `validate_addons_path` long before the
    // allowed-path comparison, so it would pass against a totally broken
    // guard.
    await invoke<unknown[]>(page, "scan_installed_addons", { addonsPath: sandboxDir });

    // `copy_addons_to_instance` is the one command with a SECOND AddOns root.
    // Its target is validated against detected game instances — the real Live
    // and PTS folders — so `require_allowed_path` on the source does not
    // contain it, and a sandbox run could have written fixture addons into a
    // real install.
    //
    // The target here is deliberately a path that does not exist. Passing a
    // REAL instance would mean that the day this guard regresses, the test
    // performs the copy it is meant to prevent before reporting it. Matching
    // on the message is what makes this fail on regression: without the guard
    // the command still errors, but on the target being inaccessible.
    const copyError = await invoke<string>(page, "copy_addons_to_instance", {
      addonsPath: sandboxDir,
      targetAddonsPath: path.join(sandboxDir, "..", "NotAnInstance", "AddOns"),
    }).then(
      () => "resolved",
      (error: Error) => error.message
    );
    expect(copyError, "cross-instance copy was not refused by the sandbox guard").toContain(
      "KALPA_ADDONS_DIR"
    );
  });

  // NOT an install-path test. `debug_install_fixture_zip` runs the real
  // extractor but deliberately skips `install_addon` — no download, no metadata
  // record, no pre-operation snapshot, no dependency resolution. It is fixture
  // SETUP so the removal flow has something real on disk to delete. The product
  // install path remains uncovered here; see the follow-up on the PR.
  test("removes an addon and restores it with undo", async () => {
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
    // Use the listbox interaction model here. Pointer actionability waits for a
    // virtualized row's hover transition to become geometrically stable, which
    // is unrelated to the remove/undo contract and can remain unsettled under
    // WebView2/CDP even though the option is already visible and enabled.
    await selectFixture(page);
    const removeButton = page.getByRole("button", { name: "Remove Addon" });
    await expect(removeButton, "detail pane has no Remove Addon button").toBeVisible({
      timeout: 5_000,
    });
    await removeButton.press("Enter");

    await expectAddonListCount(page, 0, "removing the fixture did not hide its row");

    // Undo inside the 3s window must put the row back.
    //
    // The row only, not its update state. Nothing below reads the badge, and
    // with this fixture it could not: `debug_install_fixture_zip` writes no
    // metadata record, so the addon has no ESOUI id and never produces an
    // UpdateResult for the queue to carry. The update-row half of the restore
    // is covered by the unit tests on `restoreUpdateResult`, not here.
    const undo = page.getByRole("button", { name: "Undo" });
    await expect(undo, "no Undo affordance on the removal toast").toBeVisible({ timeout: 2_000 });
    await undo.press("Enter");

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
  });

  test("a committed removal actually deletes the folder", async () => {
    await showAllAddons(page);
    await expect(row(page), "fixture should still be installed").toBeVisible({ timeout: 5_000 });
    expect(fixtureOnDisk(), "fixture folder missing before the removal").toBe(true);

    await selectFixture(page);
    await page.getByRole("button", { name: "Remove Addon" }).press("Enter");
    await expectAddonListCount(page, 0, "removing the fixture did not hide its row");

    // Let the undo window lapse so the removal commits, then prove it reached
    // the disk. Again the filesystem, not the row count: the UI already shows
    // 0 from the optimistic hide, so a count assertion proves nothing here.
    await page.waitForTimeout(4_000);
    expect(fixtureOnDisk(), "the undo window lapsed but the folder is still there").toBe(false);

    await page.keyboard.press("Control+r");
    await expectAddonListCount(page, 0, "the addon came back after the undo window lapsed");
  });
});
