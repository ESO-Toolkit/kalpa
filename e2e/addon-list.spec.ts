import { test, expect } from "@playwright/test";
import {
  addonFilterTab,
  addonList,
  connectToTauri,
  expectAddonListCount,
  readAddonListCount,
  readFilterTabCount,
  resetAppState,
} from "./helpers";

/** A query no real addon title, folder or author can contain. */
const NO_MATCH_QUERY = "zzq-kalpa-e2e-no-such-addon";

test.describe.serial("Addon List Interactions", () => {
  test("search filters the addon list", async () => {
    const { browser, page } = await connectToTauri();

    try {
      await resetAppState(page);

      const searchInput = page.locator('input[aria-label="Search addons"]');
      await expect(searchInput, "addon search box not found").toBeVisible({ timeout: 5_000 });

      // Start from an unfiltered list — a query left behind by an earlier run
      // would make every count below disagree with the filter tabs.
      await searchInput.clear();
      await page.waitForTimeout(500);

      const initialCount = await readAddonListCount(page);
      expect(
        initialCount,
        "addon list is empty — E2E needs a populated AddOns folder"
      ).toBeGreaterThan(0);

      // A query nothing can match must empty the list. If filtering is broken the
      // count simply stays at initialCount, which is the failure this asserts.
      await searchInput.click();
      await searchInput.fill(NO_MATCH_QUERY);
      await expectAddonListCount(page, 0, `searching "${NO_MATCH_QUERY}" did not filter the list`);
      await expect(page.getByText("No addons match")).toBeVisible({ timeout: 3_000 });

      await page.screenshot({ path: "e2e/screenshots/addon-search.png" });

      // Clearing the query must restore exactly the rows we started with.
      await searchInput.clear();
      await expectAddonListCount(
        page,
        initialCount,
        "clearing the search did not restore the list"
      );
    } finally {
      await browser.close();
    }
  });

  test("filter buttons toggle addon categories", async () => {
    const { browser, page } = await connectToTauri();

    try {
      await resetAppState(page);

      const allTab = addonFilterTab(page, "All");
      const libsTab = addonFilterTab(page, "Libs");
      await expect(allTab, '"All" filter tab not found').toBeVisible({ timeout: 5_000 });
      await expect(libsTab, '"Libs" filter tab not found').toBeVisible({ timeout: 5_000 });

      // Normalize: filterMode is persisted across sessions, so an earlier run
      // could have left any tab selected.
      await allTab.click();
      await expect(allTab).toHaveAttribute("aria-selected", "true");
      const allCount = await readFilterTabCount(page, "All");
      const libsCount = await readFilterTabCount(page, "Libs");
      await expectAddonListCount(
        page,
        allCount,
        '"All" is selected but the list shows a different row count'
      );

      await libsTab.click();
      await expect(libsTab, '"Libs" did not become the selected filter').toHaveAttribute(
        "aria-selected",
        "true"
      );
      await expect(allTab, '"All" stayed selected after clicking "Libs"').toHaveAttribute(
        "aria-selected",
        "false"
      );
      await expectAddonListCount(
        page,
        libsCount,
        '"Libs" filter did not narrow the list to the library count'
      );
      await page.screenshot({ path: "e2e/screenshots/addon-filter-libs.png" });

      await allTab.click();
      await expect(allTab, '"All" did not become the selected filter again').toHaveAttribute(
        "aria-selected",
        "true"
      );
      await expectAddonListCount(page, allCount, '"All" filter did not restore the full list');
      await page.screenshot({ path: "e2e/screenshots/addon-filters.png" });
    } finally {
      await browser.close();
    }
  });

  test("clicking an addon shows its details", async () => {
    const { browser, page } = await connectToTauri();

    try {
      await resetAppState(page);

      const rowCount = await readAddonListCount(page);
      expect(rowCount, "addon list is empty — nothing to open a detail pane for").toBeGreaterThan(
        0
      );

      const firstAddon = addonList(page).locator('[role="option"]').first();
      await expect(firstAddon, "first addon row not found").toBeVisible({ timeout: 5_000 });

      // The row's accessible name starts with the addon title; the detail pane
      // heading must end up showing that same title.
      const rowLabel = (await firstAddon.getAttribute("aria-label")) ?? "";
      const title = (rowLabel.split(",")[0] ?? "").trim();
      expect(title, "addon row carried no title in its aria-label").not.toBe("");

      await firstAddon.click();

      await expect(
        page.getByRole("heading", { level: 2, name: title }),
        `detail pane never showed "${title}"`
      ).toBeVisible({ timeout: 5_000 });
      await expect(
        page.getByText("No addon selected"),
        "detail pane stayed on its empty state after clicking an addon"
      ).toHaveCount(0);

      await page.screenshot({ path: "e2e/screenshots/addon-detail.png" });
    } finally {
      await browser.close();
    }
  });

  test("keyboard shortcut Ctrl+R triggers refresh", async () => {
    const { browser, page } = await connectToTauri();

    try {
      await resetAppState(page);

      await page.screenshot({ path: "e2e/screenshots/before-refresh.png" });

      // Ctrl+R triggers addon refresh (app intercepts it)
      await page.keyboard.press("Control+r");
      await page.waitForTimeout(2_000);

      // App should still be functional
      const header = page.locator("header").first();
      await expect(header).toBeVisible();

      await page.screenshot({ path: "e2e/screenshots/after-refresh.png" });
    } finally {
      await browser.close();
    }
  });
});
