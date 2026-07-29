import { expect, test } from "@playwright/test";
import {
  connectToPackagedTauri,
  createChunkLoadRecorder,
  importBuiltChunk,
  resolveBuiltChunkUrls,
} from "./helpers";

const EXPECTED_CHUNKS = [
  ["packs", "Packs"],
  ["profiles", "Profiles"],
  ["backups", "Backups"],
  ["api-compat", "ApiCompat"],
  ["characters", "Characters"],
  ["settings", "Settings"],
  ["keyboard-shortcuts", "KeyboardShortcuts"],
  ["saved-variables", "SavedVariables"],
  ["migration-wizard", "MigrationWizard"],
  ["safety-center", "SafetyCenter"],
  ["uploader-workspace", "UploaderWorkspace"],
  ["addon-detail", "AddonDetail"],
  ["addon-file-browser", "AddonFileBrowser"],
  ["addon-file-editor", "AddonFileEditor"],
  ["update-conflict-panel", "UpdateConflictPanel"],
] as const;

test.describe.serial("packaged lazy chunks", () => {
  test("loads all emitted lazy chunks from tauri.localhost @packaged", async () => {
    const { browser, page } = await connectToPackagedTauri();
    const recorder = createChunkLoadRecorder(page);

    try {
      await expect
        .poll(() => page.evaluate(() => window.location.href))
        .toBe("http://tauri.localhost/");
      await expect(page.locator("body")).toBeVisible();

      const chunkUrls = resolveBuiltChunkUrls(EXPECTED_CHUNKS.map(([stem]) => stem));

      for (const [stem, exportName] of EXPECTED_CHUNKS) {
        const result = await importBuiltChunk(page, chunkUrls[stem]);
        expect(result.exportNames, stem).toContain(exportName);
      }

      expect(recorder.errors).toEqual([]);
    } finally {
      recorder.dispose();
      await browser.close();
    }
  });
});
