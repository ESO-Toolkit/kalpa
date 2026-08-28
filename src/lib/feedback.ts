import { toast } from "sonner";

export const FEEDBACK_ISSUES_URL = "https://github.com/ESO-Toolkit/kalpa/issues/new/choose";
export const FEEDBACK_DISCORD_URL = "https://discord.gg/cMumdw6cSE";
export const FEEDBACK_DISCORD_SUPPORT_URL =
  "https://discord.com/channels/1375703719995244686/1480845158584025148";

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
      toast.error("Couldn't open the link. Try again or open it manually.");
    }
    return false;
  }
}
