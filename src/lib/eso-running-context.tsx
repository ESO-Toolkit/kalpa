import { getStrictContext } from "@/lib/get-strict-context";

/**
 * Gate run before any write to the AddOns folder (update, install, reinstall,
 * dependency, conflict resolution). Returns true to proceed, false to cancel.
 * When ESO is running it warns the user to /reloadui (unless suppressed); writing
 * while the game is open is safe on disk, the game just won't see changes until then.
 */
export type EnsureEsoNotBlocking = () => Promise<boolean>;

const [EsoRunningProvider, useEnsureEsoNotBlocking] =
  getStrictContext<EnsureEsoNotBlocking>("EsoRunningProvider");

export { EsoRunningProvider, useEnsureEsoNotBlocking };
