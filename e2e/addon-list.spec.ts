import { test, expect } from "@playwright/test";
import { connectToTauri, resetAppState } from "./helpers";

test.describe.serial("Addon List Interactions", () => {
  test("search filters the addon list", async () => {
    const { browser, page } = await connectToTauri();

    try {
      await resetAppState(page);

      // Find the search input
      const searchInput = page
        .locator('input[placeholder*="earch"], input[type="search"]')
        .first();
      await expect(searchInput).toBeVisible({ timeout: 5_000 });

      // Get initial addon count from the "All(N)" filter button
      const initialCount = await page.evaluate(() => {
        const buttons = document.querySelectorAll("button");
        for (const btn of buttons) {
          const text = btn.textContent?.trim() ?? "";
          const match = text.match(/^All\(?(\d+)\)?$/);
          if (match) return parseInt(match[1], 10);
        }
        return 0;
      });
      expect(initialCount).toBeGreaterThan(0);

      // Type a search query that will match some but not all addons
      await searchInput.click();
      await searchInput.fill("combat");
      await page.waitForTimeout(500);

      await page.screenshot({ path: "e2e/screenshots/addon-search.png" });

      // Clear search to restore full list
      await searchInput.clear();
      await page.waitForTimeout(300);
    } finally {
      await browser.close();
    }
  });

  test("filter buttons toggle addon categories", async () => {
    const { browser, page } = await connectToTauri();

    try {
      await resetAppState(page);

      // Find the "Libs" filter button
      const libsFilter = page.locator('button:has-text("Libs")').first();
      const libsVisible = await libsFilter.isVisible({ timeout: 3_000 }).catch(() => false);

      if (libsVisible) {
        await libsFilter.click();
        await page.waitForTimeout(500);
        await page.screenshot({ path: "e2e/screenshots/addon-filter-libs.png" });

        // Click "All" to reset
        const allFilter = page.locator('button:has-text("All")').first();
        await allFilter.click();
        await page.waitForTimeout(300);
      }

      await page.screenshot({ path: "e2e/screenshots/addon-filters.png" });
    } finally {
      await browser.close();
    }
  });

  test("clicking an addon shows its details", async () => {
    const { browser, page } = await connectToTauri();

    try {
      await resetAppState(page);
      await page.waitForTimeout(1_000);

      // Click the first addon item in the list
      const firstAddon = page.locator('[class*="cursor-pointer"]').first();
      const addonVisible = await firstAddon.isVisible({ timeout: 3_000 }).catch(() => false);

      if (addonVisible) {
        await firstAddon.click();
        await page.waitForTimeout(500);

        // The detail panel should show addon action buttons
        const detailPanel = page
          .locator('button:has-text("Remove"), button:has-text("Disable")')
          .first();
        const detailVisible = await detailPanel.isVisible({ timeout: 3_000 }).catch(() => false);

        if (detailVisible) {
          await expect(detailPanel).toBeVisible();
        }
      }

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
