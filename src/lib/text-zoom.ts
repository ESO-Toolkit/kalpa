import { invoke } from "@tauri-apps/api/core";
import { getSetting, setSetting } from "@/lib/store";

/**
 * User text-size control backed by Tauri WebView zoom. Rust applies the durable
 * value before the page paints; this module only hydrates the synchronous value
 * that Settings and keyboard shortcuts use after React starts.
 */
const STORE_KEY = "appearance.textZoom";
const MIN_TEXT_ZOOM = 1;
const MAX_TEXT_ZOOM = 1.5;

export const TEXT_ZOOM_STOPS = [1, 1.1, 1.25, 1.5] as const;
export const TEXT_ZOOM_CHANGE_EVENT = "kalpa:text-zoom";

let currentTextZoom = MIN_TEXT_ZOOM;

function clampTextZoom(factor: number): number {
  if (!Number.isFinite(factor)) return MIN_TEXT_ZOOM;
  return Math.min(MAX_TEXT_ZOOM, Math.max(MIN_TEXT_ZOOM, factor));
}

/** Read the persisted preference at startup without applying zoom again. */
export async function hydrateTextZoom(): Promise<void> {
  currentTextZoom = clampTextZoom(await getSetting<number>(STORE_KEY, MIN_TEXT_ZOOM));
}

/** Current effective zoom factor (valid once hydrated - synchronous read). */
export function textZoomFactor(): number {
  return currentTextZoom;
}

/**
 * Apply and persist the zoom factor. The Rust command owns the WebView zoom and
 * scaled minimum window size; the settings key is the durable startup source.
 */
export function setTextZoom(factor: number): void {
  const clamped = clampTextZoom(factor);
  currentTextZoom = clamped;
  window.dispatchEvent(new Event(TEXT_ZOOM_CHANGE_EVENT));
  void invoke("set_text_zoom", { factor: clamped }).catch((err) => {
    console.warn("[text-zoom] Failed to apply text zoom:", err);
  });
  void setSetting(STORE_KEY, clamped);
}

export function nextTextZoomStop(direction: 1 | -1, from = currentTextZoom): number {
  const clamped = clampTextZoom(from);
  if (direction > 0) {
    return TEXT_ZOOM_STOPS.find((stop) => stop > clamped + 0.001) ?? MAX_TEXT_ZOOM;
  }
  return [...TEXT_ZOOM_STOPS].reverse().find((stop) => stop < clamped - 0.001) ?? MIN_TEXT_ZOOM;
}
