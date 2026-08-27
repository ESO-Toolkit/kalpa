import type { AddonManifest, UpdateCheckResult } from "@/types";

export const PROTECTED_EDITS_UNAVAILABLE =
  "Protected Edits unavailable: no trusted file baseline exists, so Kalpa cannot detect which files you changed. Updating may overwrite those edits.";

export function countUpdatesWithoutProtectedEditsBaseline(
  updates: UpdateCheckResult[],
  addons: AddonManifest[]
): number {
  const protectedFolders = new Set(
    addons
      .filter((addon) => addon.hasProtectedEditsBaseline === true)
      .map((addon) => addon.folderName)
  );
  return updates.filter((update) => !protectedFolders.has(update.folderName)).length;
}
