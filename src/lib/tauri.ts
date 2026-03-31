import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";

export type TauriResult<T> = { ok: true; data: T } | { ok: false; error: string };

/**
 * Maps common backend error patterns to friendlier user-facing messages with context.
 */
const ERROR_HINTS: [RegExp, string][] = [
  [/zip extraction aborted.*zip bomb/i, "The archive is too large or may be corrupt. Try re-downloading it."],
  [/zip archive contained no addon folders/i, "This file doesn't look like a valid ESO addon archive."],
  [/failed to open zip file/i, "Could not open the downloaded file. It may be corrupt or incomplete — try again."],
  [/failed to read zip archive/i, "The downloaded file is not a valid ZIP. It may be corrupt — try re-downloading."],
  [/addons folder not found/i, "Your AddOns folder could not be found. It may have been moved or the drive disconnected."],
  [/could not reach esoui/i, "ESOUI could not be reached. Check your internet connection and try again."],
  [/too many requests to esoui/i, "ESOUI rate limit reached. Wait a moment and try again."],
  [/esoui is currently unavailable/i, "ESOUI appears to be down. Try again in a few minutes."],
  [/addon not found on esoui/i, "This addon was not found on ESOUI — it may have been removed by its author."],
  [/permission denied \(os error 13\)|access is denied/i, "Permission denied — antivirus or another program may be blocking the file."],
];

export function getTauriErrorMessage(error: unknown): string {
  let raw: string;
  if (error instanceof Error && error.message) {
    raw = error.message;
  } else if (typeof error === "string" && error.trim()) {
    raw = error;
  } else {
    return "Something went wrong";
  }

  // Return a friendlier message if we match a known pattern
  for (const [pattern, hint] of ERROR_HINTS) {
    if (pattern.test(raw)) return hint;
  }

  return raw;
}

export async function invokeResult<T>(
  command: string,
  args?: Record<string, unknown>
): Promise<TauriResult<T>> {
  try {
    return { ok: true, data: await invoke<T>(command, args) };
  } catch (error) {
    const message = getTauriErrorMessage(error);
    // Always log the original error for debugging; note when the user sees a mapped message
    const raw = error instanceof Error ? error.message : String(error);
    if (message !== raw) {
      console.error(`[tauri:${command}]`, raw, `(shown to user: "${message}")`);
    } else {
      console.error(`[tauri:${command}]`, error);
    }
    return { ok: false, error: message };
  }
}

export async function invokeOrThrow<T>(
  command: string,
  args?: Record<string, unknown>
): Promise<T> {
  const result = await invokeResult<T>(command, args);
  if (!result.ok) {
    throw new Error(result.error);
  }
  return result.data;
}

export function toastTauriError(action: string, error: unknown) {
  toast.error(`${action}: ${getTauriErrorMessage(error)}`);
}
