import { describe, expect, it, vi } from "vitest";
import {
  RemovalQueue,
  hideAddon,
  hideUpdateResult,
  removeFolderFromSelection,
  restoreAddon,
  restoreUpdateResult,
  sameAddonsFolder,
  shouldRestoreAfterFailure,
  type PendingRemoval,
  type PendingRemovalGroup,
  type TimerHandle,
} from "../removal-queue";
import type { AddonManifest, UpdateCheckResult } from "../../types";

function addon(folderName: string): AddonManifest {
  return {
    folderName,
    title: folderName,
    author: "Author",
    version: "1.0",
    description: "",
    apiVersion: "101044",
    dependsOn: [],
    optionalDependsOn: [],
    isLibrary: false,
    esouiId: 1,
    tags: [],
    disabled: false,
    esouiLastUpdate: 0,
    installedAt: 0,
  } as unknown as AddonManifest;
}

function updateRow(folderName: string): UpdateCheckResult {
  return {
    folderName,
    esouiId: 1,
    currentVersion: "1.0",
    remoteVersion: "2.0",
    hasUpdate: true,
    remoteLastUpdate: 0,
  } as unknown as UpdateCheckResult;
}

/** Build a group of folders sharing one timer, the way a batch removal does. */
function group(folderNames: string[], timer: TimerHandle = 1 as unknown as TimerHandle) {
  return { timer, folderNames: new Set(folderNames) } satisfies PendingRemovalGroup;
}

function entry(
  folderName: string,
  opts: {
    addonsPath?: string;
    updateResult?: UpdateCheckResult | null;
    group?: PendingRemovalGroup;
  } = {}
): PendingRemoval {
  return {
    addon: addon(folderName),
    updateResult: opts.updateResult ?? null,
    addonsPath: opts.addonsPath ?? "C:\\Games\\ESO\\AddOns",
    group: opts.group ?? group([folderName]),
  };
}

describe("RemovalQueue", () => {
  it("cancels a group's timer only when its last member leaves", () => {
    // Regression: clearing the timer as soon as ONE folder left killed the
    // shared batch timer, stranding the rest — queued forever with a dead
    // timer, then silently deleted by the close-time flush with no undo.
    const clearTimer = vi.fn();
    const queue = new RemovalQueue(clearTimer);
    const batch = group(["A", "B", "C"]);
    for (const name of ["A", "B", "C"]) queue.add(name, entry(name, { group: batch }));

    queue.drop("A");
    expect(clearTimer).not.toHaveBeenCalled();
    queue.drop("B");
    expect(clearTimer).not.toHaveBeenCalled();

    queue.drop("C");
    expect(clearTimer).toHaveBeenCalledTimes(1);
    expect(queue.size).toBe(0);
  });

  it("does not store the bare global clearTimeout as its default", () => {
    // Regression, and the only shape a unit test can assert here. Storing the
    // Window method itself means `drop()` invokes it as `this.clearTimer(h)`,
    // whose receiver is a RemovalQueue rather than a Window — WebIDL brand-checks
    // that and throws `TypeError: Illegal invocation` in Chromium/WebView2. The
    // throw aborts drop() before it returns, so the removal never reaches the
    // backend and an undo leaves the group timer alive to delete the addon anyway.
    //
    // This cannot be caught by exercising the default: jsdom does not
    // brand-check, so it passes there either way. Asserting the wrapper exists
    // is the closest a unit test can get; the real coverage is the @sandbox e2e
    // running in an actual WebView2.
    const queue = new RemovalQueue();
    const stored = (queue as unknown as { clearTimer: unknown }).clearTimer;
    expect(stored, "default must wrap clearTimeout, not be it").not.toBe(clearTimeout);
    expect(typeof stored).toBe("function");
  });

  it("returns null and touches nothing for a folder it does not hold", () => {
    const clearTimer = vi.fn();
    const queue = new RemovalQueue(clearTimer);
    queue.add("A", entry("A"));

    expect(queue.drop("Missing")).toBeNull();
    expect(clearTimer).not.toHaveBeenCalled();
    expect(queue.size).toBe(1);
  });

  it("drains every entry and cancels every group timer", () => {
    // What an AddOns-path switch and the beforeunload flush both rely on.
    const clearTimer = vi.fn();
    const queue = new RemovalQueue(clearTimer);
    const batch = group(["A", "B"], 10 as unknown as TimerHandle);
    queue.add("A", entry("A", { group: batch }));
    queue.add("B", entry("B", { group: batch }));
    queue.add("C", entry("C", { group: group(["C"], 20 as unknown as TimerHandle) }));

    const drained = queue.drain();

    expect(drained.map((e) => e.addon.folderName).sort()).toEqual(["A", "B", "C"]);
    expect(queue.size).toBe(0);
    expect(clearTimer.mock.calls.map((c) => c[0]).sort()).toEqual([10, 20]);
    // A second drain has nothing left to commit — entries cannot be removed twice.
    expect(queue.drain()).toEqual([]);
  });

  it("carries the AddOns folder each removal was queued against", () => {
    // Regression: an AddOns-path switch left pending removals pointed at
    // whichever folder was live when their timer fired, so the removal deleted
    // a same-named addon in the instance the user had just moved to.
    const oldPath = "C:\\Games\\ESO\\live\\AddOns";
    const newPath = "C:\\Games\\ESO\\pts\\AddOns";
    const queue = new RemovalQueue(vi.fn());
    queue.add("Shared", entry("Shared", { addonsPath: oldPath }));

    const drained = queue.drain();
    expect(drained).toHaveLength(1);
    const flushed = drained[0]!;

    expect(flushed.addonsPath).toBe(oldPath);
    expect(flushed.addonsPath).not.toBe(newPath);
    // And a failure must not resurrect the row in the folder now on screen.
    expect(shouldRestoreAfterFailure(flushed, newPath)).toBe(false);
    expect(shouldRestoreAfterFailure(flushed, oldPath)).toBe(true);
  });
});

describe("sameAddonsFolder", () => {
  it("treats casing, separators and trailing slashes as the same folder", () => {
    // Paths reach us from settings as a bare trim, so one physical folder
    // arrives in several spellings; raw equality split it into two.
    expect(sameAddonsFolder("C:\\ESO\\AddOns", "c:/eso/addons")).toBe(true);
    expect(sameAddonsFolder("C:\\ESO\\AddOns\\", "C:\\ESO\\AddOns")).toBe(true);
    expect(sameAddonsFolder("  C:\\ESO\\AddOns  ", "C:\\ESO\\AddOns")).toBe(true);
    expect(sameAddonsFolder("C:\\ESO\\live\\AddOns", "C:\\ESO\\pts\\AddOns")).toBe(false);
  });
});

describe("restore reducers", () => {
  it("restores the update row alongside the addon", () => {
    // Regression: undo put the addon back but dropped its UpdateCheckResult, so
    // a restored addon silently lost its Update badge, its place in the update
    // banner count and its "Outdated" filter membership until the next check.
    const removed = entry("BigWigs", { updateResult: updateRow("BigWigs") });

    const addons = restoreAddon([addon("Other")], removed.addon);
    const updates = restoreUpdateResult([], removed.updateResult);

    expect(addons.map((a) => a.folderName)).toEqual(["Other", "BigWigs"]);
    expect(updates.map((r) => r.folderName)).toEqual(["BigWigs"]);
    expect(updates[0]?.hasUpdate).toBe(true);
  });

  it("is idempotent when a rescan already put the row back", () => {
    const restored = addon("BigWigs");
    const existing = [restored];
    expect(restoreAddon(existing, restored)).toBe(existing);

    const row = updateRow("BigWigs");
    const existingRows = [row];
    expect(restoreUpdateResult(existingRows, row)).toBe(existingRows);
  });

  it("leaves update rows alone for an addon that had no update", () => {
    const rows = [updateRow("Other")];
    expect(restoreUpdateResult(rows, null)).toBe(rows);
  });
});

describe("optimistic hide reducers", () => {
  it("hides the addon and its update row together", () => {
    expect(hideAddon([addon("A"), addon("B")], "A").map((a) => a.folderName)).toEqual(["B"]);
    expect(
      hideUpdateResult([updateRow("A"), updateRow("B")], "A").map((r) => r.folderName)
    ).toEqual(["B"]);
  });

  it("keeps the selection set's identity when the folder was not selected", () => {
    const selection = new Set(["A"]);
    expect(removeFolderFromSelection(selection, "B")).toBe(selection);
    expect([...removeFolderFromSelection(selection, "A")]).toEqual([]);
  });
});
