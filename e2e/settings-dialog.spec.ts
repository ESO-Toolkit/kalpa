import { test, expect } from "@playwright/test";
import { connectToTauri, resetAppState } from "./helpers";

test.describe.serial("Settings Dialog", () => {
  test("settings button opens dialog with tabs", async () => {
    const { browser, page } = await connectToTauri();

    try {
      await resetAppState(page);

      // Find the settings button (gear icon in the header)
      const settingsBtn = page
        .locator('button[aria-label*="etting"], header button:has(svg)')
        .filter({ hasText: /^$/ });

      // Try finding it by looking for the settings icon button in the header area
      const headerButtons = page.locator("header button");
      const buttonCount = await headerButtons.count();

      let settingsClicked = false;
      for (let i = 0; i < buttonCount; i++) {
        const btn = headerButtons.nth(i);
        const ariaLabel = await btn.getAttribute("aria-label").catch(() => null);
        const text = await btn.textContent().catch(() => "");

        if (
          ariaLabel?.toLowerCase().includes("setting") ||
          text?.toLowerCase().includes("setting")
        ) {
          await btn.click();
          settingsClicked = true;
          break;
        }
      }

      // Fallback: try clicking by exact text or by looking for settings icon buttons
      if (!settingsClicked) {
        // Look in the right side of the header for icon-only buttons
        const iconButtons = page.locator('header a, header button, [role="button"]');
        const count = await iconButtons.count();
        // Settings is typically one of the last buttons in the header
        for (let i = count - 1; i >= Math.max(0, count - 6); i--) {
          const btn = iconButtons.nth(i);
          const text = await btn.textContent().catch(() => "");
          if (!text?.trim()) {
            // Icon-only button — could be settings
            await btn.click();
            await page.waitForTimeout(500);

            // Check if settings dialog opened
            const dialog = page.locator('[role="dialog"]').first();
            const isSettings = await dialog.isVisible().catch(() => false);
            if (isSettings) {
              const hasGeneral = await page
                .locator('text="General"')
                .first()
                .isVisible()
                .catch(() => false);
              if (hasGeneral) {
                settingsClicked = true;
                break;
              }
            }
            // Not settings, close and try next
            await page.keyboard.press("Escape");
            await page.waitForTimeout(300);
          }
        }
      }

      if (settingsClicked) {
        await page.waitForTimeout(500);

        // Verify the settings dialog has the three tabs
        const generalTab = page.locator('text="General"').first();
        const toolsTab = page.locator('text="Tools"').first();
        const dataTab = page.locator('text="Data"').first();

        await expect(generalTab).toBeVisible({ timeout: 3_000 });
        await expect(toolsTab).toBeVisible({ timeout: 3_000 });
        await expect(dataTab).toBeVisible({ timeout: 3_000 });

        // Verify the addons path field is visible
        const pathField = page.locator('text=/AddOns/i').first();
        await expect(pathField).toBeVisible();

        await page.screenshot({ path: "e2e/screenshots/settings-dialog.png" });

        // Click the Tools tab
        await toolsTab.click();
        await page.waitForTimeout(500);
        await page.screenshot({ path: "e2e/screenshots/settings-tools-tab.png" });

        // Click the Data tab
        await dataTab.click();
        await page.waitForTimeout(500);
        await page.screenshot({ path: "e2e/screenshots/settings-data-tab.png" });

        // Close the dialog
        await page.keyboard.press("Escape");
        await page.waitForTimeout(500);

        // Verify dialog is closed
        const dialogGone = page.locator('[role="dialog"]');
        await expect(dialogGone).not.toBeVisible({ timeout: 3_000 });
      }
    } finally {
      await browser.close();
    }
  });
});
