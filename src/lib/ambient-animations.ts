import { getSetting, setSetting } from "@/lib/store";

/**
 * The "Ambient animations" appearance setting: decorative motion (orb drift,
 * dialog accent shimmer). Kalpa often runs beside the game, so the decorative
 * layer must be able to go fully static — with it off, the app produces zero
 * animation frames at rest instead of a continuous compositor load.
 *
 * State lives as a root class (`ambient-off`) so CSS can zero the animations
 * (`animation: none`) and the WAAPI tickers see no animations to drive. The
 * class is the synchronous source of truth after hydration; the store key is
 * the durable copy.
 */
const STORE_KEY = "appearance.ambientAnimations";
const OFF_CLASS = "ambient-off";

/** Apply the persisted preference at startup (called from main.tsx). */
export async function hydrateAmbientAnimations(): Promise<void> {
  const enabled = await getSetting<boolean>(STORE_KEY, true);
  document.documentElement.classList.toggle(OFF_CLASS, !enabled);
}

/** Fired on window whenever the preference flips, so the animation tickers
 *  (orb drift, dialog sweep) can stop/restart without polling. */
export const AMBIENT_CHANGE_EVENT = "kalpa:ambient-animations";

/** Flip the preference: updates the DOM immediately, persists in background. */
export function setAmbientAnimations(enabled: boolean): void {
  document.documentElement.classList.toggle(OFF_CLASS, !enabled);
  window.dispatchEvent(new Event(AMBIENT_CHANGE_EVENT));
  void setSetting(STORE_KEY, enabled);
}

/** Current effective state (valid once hydrated — synchronous read). */
export function ambientAnimationsEnabled(): boolean {
  return !document.documentElement.classList.contains(OFF_CLASS);
}
