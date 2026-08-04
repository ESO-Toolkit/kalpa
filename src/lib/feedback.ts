import { toast } from "sonner";

export const FEEDBACK_ISSUES_URL = "https://github.com/ESO-Toolkit/kalpa/issues/new/choose";
export const FEEDBACK_DISCORD_URL = "https://discord.gg/cMumdw6cSE";

/** Open a feedback link in the user's browser, surfacing failures instead of
 *  swallowing them. The opener plugin rejects any URL outside the capability's
 *  allow-scope, and a user-clicked button that silently does nothing reads as a
 *  broken app — toast the address so the user can still reach it. */
export async function openFeedbackUrl(url: string): Promise<void> {
  try {
    const m = await import("@tauri-apps/plugin-opener");
    await m.openUrl(url);
  } catch {
    toast.error(`Couldn't open the link — visit ${url} in your browser.`);
  }
}
