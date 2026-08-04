/**
 * How a rescan reconciles the user's selection.
 *
 * Every install, update, remove and dependency install ends in a rescan, so the
 * rescan's selection handling decides whether the user keeps their place.
 * Clearing the selection wholesale is the tempting simplification and it is
 * wrong: installing a dependency rescans the folder, and a blanket clear threw
 * away a multi-select the user had built up before the install — the addons they
 * had ticked were all still installed.
 *
 * The rule is prune, not reset: keep every selected folder that still exists,
 * drop only the ones that no longer do. A failed scan is the one exception —
 * with no list at all, there is nothing for a selection to refer to.
 */

import type { AddonManifest } from "../types";

/**
 * Drop selected folders that the rescan no longer found, keeping the rest.
 *
 * Returns `prev` unchanged when nothing was pruned so React can skip the
 * re-render — worth caring about because this runs after every scan.
 */
export function pruneSelection(prev: Set<string>, addons: AddonManifest[]): Set<string> {
  if (prev.size === 0) return prev;
  const validFolders = new Set(addons.map((a) => a.folderName));
  const pruned = new Set([...prev].filter((f) => validFolders.has(f)));
  return pruned.size === prev.size ? prev : pruned;
}

/**
 * Re-point the detail pane at the rescanned copy of whatever was open.
 *
 * Returning the FRESH object matters: the detail pane would otherwise keep
 * rendering the pre-scan manifest (stale version, tags, dependency status) for
 * an addon that just changed on disk. A null result closes the pane, which is
 * correct when the open addon is gone.
 *
 * `null` in means nothing was open, and nothing should be opened.
 */
export function reconcileSelectedAddon(
  selected: AddonManifest | null,
  addons: AddonManifest[]
): AddonManifest | null {
  if (!selected) return null;
  return addons.find((addon) => addon.folderName === selected.folderName) ?? null;
}
