// Shared, side-effectful helpers used across the uploader workspace and its
// extracted modules (the live-session hook, the history panel). Kept in one place
// so the manual, live, and history paths route/open reports identically and can
// never drift. Plus the shared "raised work panel" surface token used by the
// several mid-tier panels.

import { toast } from "sonner";
import { getSetting, getSettingChecked, settingsWritesSettled } from "@/lib/store";
import { invokeOrThrow } from "@/lib/tauri";
import type { ReportRef, Visibility } from "@/types/uploader";
import { primaryReportUrl } from "./uploader-shared";

/** A mid-tier "raised work panel" — sits clearly above the dark canvas but
 *  quieter than the primary picker/action. Used for fights, options, history so
 *  the elevation order reads: canvas < these < picker/action. */
export const WORK_PANEL =
  "rounded-2xl border border-white/[0.08] bg-gradient-to-b from-white/[0.045] to-white/[0.015] shadow-[0_8px_28px_-14px_rgba(0,0,0,0.65),inset_0_1px_0_rgba(255,255,255,0.05)]";

/** Open a report URL in the user's browser, surfacing failures instead of
 *  swallowing them. The opener plugin rejects a URL outside the capability's
 *  allow-scope (now includes esologs.com/reports/*); a rejection should toast,
 *  not vanish into an unhandled promise. */
export async function openReportUrl(url: string): Promise<void> {
  try {
    const m = await import("@tauri-apps/plugin-opener");
    await m.openUrl(url);
  } catch {
    toast.error("Couldn't open the report — copy the link and open it manually.");
  }
}

/** The effective "use the official ESO Logs uploader" opt-out, read FAIL-CLOSED.
 *  Returns true (use official) if EITHER opt-out key is set OR a store read fails —
 *  the native path speaks ESO Logs' private endpoints, so a degraded store that
 *  can't confirm the opt-out must NOT silently route there against the user. The two
 *  keys are written as one unit (the unified Settings toggle), so either set ⇒
 *  opted out. Used by both manual and live routing so they can never disagree. */
export async function usesOfficialUploader(): Promise<boolean> {
  // Order this read AFTER any pending settings write: the Settings toggle writes the
  // opt-out fire-and-forget, so reading the store immediately could see stale values
  // and route native against a just-set opt-out.
  await settingsWritesSettled();
  // A TAINTED settings store (opened empty over an unreadable settings file) returns
  // default values WITHOUT error, so getSettingChecked's `ok` can't catch it. Consult
  // the backend taint flag and fail closed; a failed taint check also fails closed.
  const tainted = await invokeOrThrow<boolean>("settings_tainted").catch(() => true);
  if (tainted) return true;
  const [manual, live] = await Promise.all([
    getSettingChecked<boolean>("manualUseOfficialUploader", false),
    getSettingChecked<boolean>("liveUseOfficialUploader", false),
  ]);
  return !manual.ok || !live.ok || manual.value || live.value;
}

/** Open the ESO Log Aggregator analysis for a report IFF the user enabled auto-open
 *  (the `autoOpenAnalysis` setting, default off). Best-effort: a disabled setting,
 *  a read failure, or an opener-scope rejection is silent — the always-present
 *  "View analysis" button covers the manual case. `live` opens raw ESO Logs with
 *  fight=last for an in-progress native session. */
export async function maybeAutoOpenAnalysis(
  report: ReportRef,
  visibility: Visibility,
  opts?: { live?: boolean }
): Promise<void> {
  try {
    const auto = await getSetting<boolean>("autoOpenAnalysis", false);
    if (!auto) return;
    // Open directly (not via openReportUrl) so a failure stays SILENT: the user
    // didn't click anything, so an opener-scope rejection or read error must not
    // pop a "couldn't open" toast. The always-present "View analysis" button covers
    // the manual path.
    const m = await import("@tauri-apps/plugin-opener");
    await m.openUrl(primaryReportUrl(report, visibility, opts));
  } catch {
    /* best-effort — the manual button still works */
  }
}
