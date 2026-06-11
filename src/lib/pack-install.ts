import { listen } from "@tauri-apps/api/event";
import type { PackInstallEntry, PackInstallProgress, PackInstallResult } from "../types";
import { invokeResult } from "./tauri";

export interface PackInstallProgressState {
  completed: number;
  failed: number;
  total: number;
}

/**
 * Install a pack's addons via the batch `batch_install_pack_addons` command:
 * one parallel download + single locked extract/record pass on the backend,
 * instead of one resolve+install round-trip per addon.
 *
 * Subscribes to the `pack-install-progress` event for the duration so callers
 * can drive a progress bar (and, via `onProgress`, per-addon UI state). The
 * listener is always torn down before returning.
 *
 * Returns the backend result, or null if the command itself errored (the
 * caller decides how to surface that — typically treating every addon as
 * failed).
 */
export async function runBatchPackInstall(
  addonsPath: string,
  entries: PackInstallEntry[],
  setProgress?: (state: PackInstallProgressState) => void,
  onProgress?: (event: PackInstallProgress) => void
): Promise<PackInstallResult | null> {
  const total = entries.length;
  if (total === 0) return null;

  let completed = 0;
  let failed = 0;

  const unlisten = await listen<PackInstallProgress>("pack-install-progress", (event) => {
    const payload = event.payload;
    onProgress?.(payload);
    if (payload.phase === "completed") {
      completed += 1;
    } else if (payload.phase === "failed") {
      failed += 1;
    }
    setProgress?.({ completed, failed, total });
  });

  try {
    const result = await invokeResult<PackInstallResult>("batch_install_pack_addons", {
      addonsPath,
      entries,
    });
    return result.ok ? result.data : null;
  } finally {
    unlisten();
  }
}
