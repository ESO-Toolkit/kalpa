import { invokeOrThrow } from "@/lib/tauri";

/**
 * Is ESO running, for the purposes of writing SavedVariables?
 *
 * Deliberately NOT `ensureEsoNotBlocking`, and the difference is the point.
 *
 * That gate warns and lets the user proceed, and honours
 * `suppressEsoRunningWarning` — which is correct for ADDON files. Writing those
 * under a running client is safe on disk; the game simply does not see them
 * until `/reloadui` or relog, which is exactly what that reminder says.
 *
 * SavedVariables are the opposite. ESO holds them in memory and rewrites them at
 * every loading screen — login, `/reloadui`, zone change and logout — so a file
 * written underneath a running client is overwritten with the game's own copy.
 * The import is not delayed, it is destroyed, and the user was told it worked.
 *
 * So this REFUSES rather than warns, and ignores the addon preference: dismissing
 * a `/reloadui` reminder is a statement about notification noise, not consent to
 * lose settings. Reusing that preference here made it a silent data-loss switch.
 *
 * Fails OPEN on a detection error, matching `ensureEsoNotBlocking` and the Slint
 * sidecar's `settings_write_eso_running`. Failing closed would make settings
 * imports impossible wherever process detection is broken, and flipping that
 * direction is a decision for every caller at once rather than this one.
 *
 * Call it IMMEDIATELY before the write. Sampling it earlier — before an addon
 * install that runs for minutes — answers a question about the wrong moment.
 */
export async function esoIsRunningForSettingsWrite(): Promise<boolean> {
  try {
    return await invokeOrThrow<boolean>("is_eso_running");
  } catch {
    return false;
  }
}

/** Shared so the two callers cannot drift into describing this differently. */
export const ESO_RUNNING_SETTINGS_REFUSAL =
  "Close ESO before applying these settings — the game keeps SavedVariables in memory and " +
  "rewrites them at the next loading screen, which would discard them.";
