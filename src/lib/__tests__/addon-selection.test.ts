import { describe, expect, it } from "vitest";
import { pruneSelection, reconcileSelectedAddon } from "../addon-selection";
import type { AddonManifest } from "../../types";

function addon(folderName: string, version = "1.0"): AddonManifest {
  return { folderName, title: folderName, version } as unknown as AddonManifest;
}

describe("pruneSelection", () => {
  it("keeps the selection across the rescan a dependency install triggers", () => {
    // Regression: installing a dependency ends in a rescan, and clearing the
    // selection there threw away a multi-select the user had built up before the
    // install — every one of those addons was still installed.
    const selected = new Set(["Alpha", "Beta"]);
    const afterInstall = [addon("Alpha"), addon("Beta"), addon("LibAddonMenu-2.0")];

    expect([...pruneSelection(selected, afterInstall)].sort()).toEqual(["Alpha", "Beta"]);
  });

  it("drops only the folders the rescan no longer found", () => {
    const selected = new Set(["Alpha", "Removed", "Beta"]);

    const pruned = pruneSelection(selected, [addon("Alpha"), addon("Beta")]);

    expect([...pruned].sort()).toEqual(["Alpha", "Beta"]);
  });

  it("preserves identity when nothing was pruned so React can skip the render", () => {
    const selected = new Set(["Alpha"]);
    expect(pruneSelection(selected, [addon("Alpha"), addon("Beta")])).toBe(selected);

    const empty = new Set<string>();
    expect(pruneSelection(empty, [addon("Alpha")])).toBe(empty);
  });
});

describe("reconcileSelectedAddon", () => {
  it("re-points the detail pane at the freshly scanned copy", () => {
    // Keeping the pre-scan manifest left the pane showing a stale version and
    // dependency status for an addon that had just changed on disk.
    const before = addon("Alpha", "1.0");
    const scanned = [addon("Alpha", "2.0"), addon("Beta")];

    const reconciled = reconcileSelectedAddon(before, scanned);

    expect(reconciled).toBe(scanned[0]);
    expect(reconciled?.version).toBe("2.0");
  });

  it("closes the pane when the open addon is gone", () => {
    expect(reconcileSelectedAddon(addon("Alpha"), [addon("Beta")])).toBeNull();
  });

  it("opens nothing when nothing was open", () => {
    expect(reconcileSelectedAddon(null, [addon("Alpha")])).toBeNull();
  });
});
