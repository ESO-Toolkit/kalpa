/**
 * OS detection for platform-specific UI behavior (shortcut modifiers,
 * titlebar controls, Windows-only guidance dialogs).
 *
 * Uses @tauri-apps/plugin-os, which resolves synchronously from metadata the
 * Tauri runtime injects into the webview. Outside a Tauri context (vitest
 * jsdom), detection falls back to "windows" — the app's historical default.
 */
import { platform } from "@tauri-apps/plugin-os";

export type OsType = "windows" | "macos" | "linux";

let cached: OsType | null = null;

export function osType(): OsType {
  if (cached === null) {
    try {
      const p = platform();
      cached = p === "macos" ? "macos" : p === "linux" ? "linux" : "windows";
    } catch {
      cached = "windows";
    }
  }
  return cached;
}

export const isMac = (): boolean => osType() === "macos";
export const isWindows = (): boolean => osType() === "windows";
export const isLinux = (): boolean => osType() === "linux";

/** Display label for the primary shortcut modifier ("⌘" on macOS, else "Ctrl"). */
export function modKeyLabel(): string {
  return isMac() ? "⌘" : "Ctrl";
}

/**
 * True when the platform's primary modifier is held for a shortcut —
 * Cmd on macOS, Ctrl elsewhere. Checks only the platform's own modifier so
 * Ctrl-combos don't double-fire on macOS.
 */
export function isModKey(e: { ctrlKey: boolean; metaKey: boolean }): boolean {
  return isMac() ? e.metaKey : e.ctrlKey;
}

/** Typical ESO AddOns location on this OS, for placeholder/help text. */
export function exampleAddonsPath(): string {
  switch (osType()) {
    case "macos":
      return "~/Documents/Elder Scrolls Online/live/AddOns";
    case "linux":
      // Steam Proton prefix (ESO App ID 306130)
      return "~/.steam/steam/steamapps/compatdata/306130/pfx/drive_c/users/steamuser/Documents/Elder Scrolls Online/live/AddOns";
    default:
      return "C:\\Users\\you\\Documents\\Elder Scrolls Online\\live\\AddOns";
  }
}
