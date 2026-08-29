import { toast } from "sonner";

export const FEEDBACK_ISSUES_URL = "https://github.com/ESO-Toolkit/kalpa/issues/new/choose";
export const FEEDBACK_DISCORD_URL = "https://discord.gg/cMumdw6cSE";
export const FEEDBACK_DISCORD_SUPPORT_URL =
  "https://discord.com/channels/1375703719995244686/1480845158584025148";

/** Longest address still worth reading out of a toast. */
const READABLE_URL_LENGTH = 120;

/** A fragment can carry the encoded support report, so an address that has one
 *  is never repeated back to the user regardless of its length. */
function isQuotableUrl(url: string): boolean {
  return url.length <= READABLE_URL_LENGTH && !url.includes("#");
}

/** Open a feedback link and report whether the browser accepted it. */
export async function openFeedbackUrl(
  url: string,
  options: { toastOnError?: boolean } = {}
): Promise<boolean> {
  try {
    const m = await import("@tauri-apps/plugin-opener");
    await m.openUrl(url);
    return true;
  } catch {
    if (options.toastOnError !== false) {
      // A button that silently does nothing reads as a broken app, so name the
      // destination whenever it is short enough to type by hand.
      toast.error(
        isQuotableUrl(url)
          ? `Couldn't open the link — visit ${url} in your browser.`
          : "Couldn't open the link. Try again, or use the manual ticket desk."
      );
    }
    return false;
  }
}
