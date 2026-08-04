import { test, expect } from "@playwright/test";
import { connectToTauri, resetAppState } from "./helpers";

/** Tab labels the settings dialog renders, in order (settings.tsx `tabs`). */
const SETTINGS_TABS = ["General", "Appearance", "Tools", "Data"];

test.describe.serial("Settings Dialog", () => {
  test("settings button opens dialog with tabs", async () => {
    const { browser, page } = await connectToTauri();

    try {
      await resetAppState(page);

      // The header's gear button carries aria-label="Settings" (app-header.tsx).
      // If that name ever changes this must fail rather than skip the test body.
      const settingsBtn = page.locator('header button[aria-label="Settings"]');
      await expect(
        settingsBtn,
        'header Settings button not found (expected aria-label="Settings")'
      ).toBeVisible({ timeout: 5_000 });
      await settingsBtn.click();

      const dialog = page.locator('[role="dialog"]').first();
      await expect(dialog, "settings dialog did not open").toBeVisible({ timeout: 5_000 });

      for (const label of SETTINGS_TABS) {
        await expect(
          dialog.getByRole("button", { name: label, exact: true }),
          `settings dialog is missing the "${label}" tab`
        ).toBeVisible({ timeout: 3_000 });
      }

      // The General tab must show the configured AddOns folder.
      await expect(
        dialog.locator("#addons-path"),
        "settings did not show the configured AddOns folder"
      ).toHaveValue(/AddOns/i);

      await page.screenshot({ path: "e2e/screenshots/settings-dialog.png" });

      await dialog.getByRole("button", { name: "Tools", exact: true }).click();
      await page.waitForTimeout(500);
      await page.screenshot({ path: "e2e/screenshots/settings-tools-tab.png" });

      await dialog.getByRole("button", { name: "Data", exact: true }).click();
      await page.waitForTimeout(500);
      await page.screenshot({ path: "e2e/screenshots/settings-data-tab.png" });

      await page.keyboard.press("Escape");
      await expect(dialog, "Escape did not close the settings dialog").not.toBeVisible({
        timeout: 3_000,
      });
    } finally {
      await browser.close();
    }
  });
});
