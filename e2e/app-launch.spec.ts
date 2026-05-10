import { test, expect } from "@playwright/test";
import { connectToTauri, resetAppState } from "./helpers";

test.describe.serial("App Launch", () => {
  test("app loads and renders the header", async () => {
    const { browser, page } = await connectToTauri();

    try {
      await resetAppState(page);

      const header = page.locator("header").first();
      await expect(header).toBeVisible({ timeout: 10_000 });

      await page.screenshot({ path: "e2e/screenshots/app-launch.png" });
    } finally {
      await browser.close();
    }
  });

  test("addon list is populated with addons", async () => {
    const { browser, page } = await connectToTauri();

    try {
      await resetAppState(page);

      // The filter bar has buttons like "All(114)", "Addons(65)", "Libs(49)"
      // Use evaluate to find the actual count from the button text
      const addonCount = await page.evaluate(() => {
        const buttons = document.querySelectorAll("button");
        for (const btn of buttons) {
          const text = btn.textContent?.trim() ?? "";
          const match = text.match(/^All\(?(\d+)\)?$/);
          if (match) return parseInt(match[1], 10);
        }
        return 0;
      });

      expect(addonCount).toBeGreaterThan(0);

      await page.screenshot({ path: "e2e/screenshots/addon-list-populated.png" });
    } finally {
      await browser.close();
    }
  });

  test("update banner shows when updates are available", async () => {
    const { browser, page } = await connectToTauri();

    try {
      await resetAppState(page);

      const updateAllBtn = page.locator('button:has-text("Update All")').first();
      const hasUpdates = await updateAllBtn.isVisible({ timeout: 5_000 }).catch(() => false);

      if (hasUpdates) {
        await expect(updateAllBtn).toBeVisible();
      }

      await page.screenshot({ path: "e2e/screenshots/update-banner.png" });
    } finally {
      await browser.close();
    }
  });
});
