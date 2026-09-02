import { describe, expect, it, vi } from "vitest";
import {
  RemovalQueue,
  hideAddon,
  hidePendingRemovals,
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

const LIVE = "C:\\Games\\ESO\\live\\AddOns";
const PTS = "C:\\Games\\ESO\\pts\\AddOns";

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

describe("hidePendingRemovals", () => {
  it("keeps a rescan inside the undo window from resurrecting the row", () => {
    // The removal has hidden the row but NOT deleted the folder — the real
    // delete is 3s away. A rescan in that window reads the addon straight back
    // off disk, and once the timer fires and the delete succeeds nothing hides
    // it again: the list shows an addon that no longer exists.
    const queue = new RemovalQueue(vi.fn());
    queue.add("Doomed", entry("Doomed", { addonsPath: LIVE }));

    const scanned = [addon("Kept"), addon("Doomed")];
    expect(hidePendingRemovals(scanned, queue, LIVE).map((a) => a.folderName)).toEqual(["Kept"]);

    // Update rows go through the same rule, so a badge cannot outlive its row.
    const rows = [updateRow("Kept"), updateRow("Doomed")];
    expect(hidePendingRemovals(rows, queue, LIVE).map((r) => r.folderName)).toEqual(["Kept"]);
  });

  it("keeps masking across the delete itself, not just the undo window", () => {
    // Leaving the queue ends UNDO eligibility, not the folder's existence. The
    // timer drops the entry and only then starts the async delete, so a scan
    // landing in that gap reads the folder off disk — and nothing hides the
    // restored row once the delete succeeds. Visibility has to outlast undo.
    const queue = new RemovalQueue(vi.fn());
    queue.add("Doomed", entry("Doomed", { addonsPath: LIVE }));
    queue.drop("Doomed");
    queue.beginCommit("Doomed", LIVE);

    expect(queue.has("Doomed"), "undo no longer applies").toBe(false);
    expect(queue.isHidden("Doomed", LIVE), "but it must stay hidden").toBe(true);
    expect(hidePendingRemovals([addon("Doomed")], queue, LIVE)).toEqual([]);

    // Once the backend resolves — deleted, or failed and restored — masking ends.
    queue.endCommit("Doomed", LIVE);
    const scanned = [addon("Doomed")];
    expect(hidePendingRemovals(scanned, queue, LIVE)).toBe(scanned);
  });

  it("stops masking the moment the entry leaves the queue", () => {
    // Undo and failure-restore both drop the entry first, so the row must come
    // back on the next scan rather than staying invisible.
    const queue = new RemovalQueue(vi.fn());
    queue.add("Undone", entry("Undone"));
    queue.drop("Undone");

    const scanned = [addon("Undone")];
    expect(hidePendingRemovals(scanned, queue, LIVE)).toBe(scanned);
  });

  it("preserves identity when nothing is queued", () => {
    const queue = new RemovalQueue(vi.fn());
    const scanned = [addon("A"), addon("B")];
    expect(hidePendingRemovals(scanned, queue, LIVE)).toBe(scanned);
    expect(hidePendingRemovals([], queue, LIVE)).toEqual([]);
  });

  it("never masks a same-named addon in a different game instance", () => {
    // Folder names are not unique across instances — the same addon is
    // installed under the same name in live, PTS and every Steam copy. A delete
    // in flight against live must not hide the untouched PTS copy the user just
    // switched to, because nothing would bring it back: endCommit lifts the mask
    // but does not re-run the scan that was filtered while it was up.
    const queue = new RemovalQueue(vi.fn());
    queue.add("Shared", entry("Shared", { addonsPath: LIVE }));

    // Queued against live: hidden there, visible in PTS.
    expect(hidePendingRemovals([addon("Shared")], queue, LIVE)).toEqual([]);
    expect(hidePendingRemovals([addon("Shared")], queue, PTS).map((a) => a.folderName)).toEqual([
      "Shared",
    ]);

    // Same once the timer has fired and the delete is in flight against live.
    queue.drop("Shared");
    queue.beginCommit("Shared", LIVE);
    expect(queue.isHidden("Shared", LIVE)).toBe(true);
    expect(queue.isHidden("Shared", PTS), "PTS copy must stay visible").toBe(false);
    expect(hidePendingRemovals([addon("Shared")], queue, PTS).map((a) => a.folderName)).toEqual([
      "Shared",
    ]);
  });
});

describe("completed-removal masking (the phantom-row race)", () => {
  it("masks a row a scan enumerated before the delete and delivered after it", () => {
    // t=0.0  the scan is issued            -> stamp
    // t=0.1  the backend enumerates Foo, still on disk
    // t=0.2  user removes Foo              -> queued
    // t=3.2  timer fires                   -> drop + beginCommit
    // t=3.3  delete confirmed              -> markRemoved, endCommit
    // t=3.4  the scan resolves             -> nothing queued, nothing committing
    const queue = new RemovalQueue(vi.fn());
    const since = queue.stamp();

    queue.add("Foo", entry("Foo", { addonsPath: LIVE }));
    queue.drop("Foo");
    queue.beginCommit("Foo", LIVE);
    queue.markRemoved("Foo", LIVE);
    queue.endCommit("Foo", LIVE);

    // The queue-only answer is the bug: visible.
    expect(queue.isHidden("Foo", LIVE)).toBe(false);
    // Stamped against the in-flight request, it stays hidden.
    expect(queue.isHidden("Foo", LIVE, since)).toBe(true);
    expect(hidePendingRemovals([addon("Foo")], queue, LIVE, since)).toEqual([]);
  });

  it("stops masking once a request is issued after the removal landed", () => {
    const queue = new RemovalQueue(vi.fn());
    queue.markRemoved("Foo", LIVE);

    const since = queue.stamp();
    expect(queue.isHidden("Foo", LIVE, since)).toBe(false);
    // So a reinstall shows through on the very next scan.
    const scanned = [addon("Foo")];
    expect(hidePendingRemovals(scanned, queue, LIVE, since)).toBe(scanned);
  });

  it("scopes completed removals to the instance they happened in", () => {
    const queue = new RemovalQueue(vi.fn());
    const since = queue.stamp();
    queue.markRemoved("Shared", LIVE);

    expect(queue.isHidden("Shared", LIVE, since)).toBe(true);
    expect(queue.isHidden("Shared", PTS, since)).toBe(false);
  });

  it("does not mask when the caller passes no stamp", () => {
    // `runBatchUpdates` re-masks a list it already holds rather than applying an
    // in-flight result, so it must keep the queue-only answer.
    const queue = new RemovalQueue(vi.fn());
    queue.markRemoved("Foo", LIVE);
    const scanned = [addon("Foo")];
    expect(hidePendingRemovals(scanned, queue, LIVE)).toBe(scanned);
  });

  it("keeps every removal a batch bigger than any fixed window produces", () => {
    // A count-capped log would evict the earliest of these while the scan that
    // needs them masked was still in flight — the phantom rows come straight
    // back for the addons removed first.
    const queue = new RemovalQueue(vi.fn());
    const since = queue.stamp();
    for (let i = 0; i < 1000; i++) queue.markRemoved(`Addon${i}`, LIVE);

    expect(queue.isHidden("Addon0", LIVE, since)).toBe(true);
    expect(queue.isHidden("Addon500", LIVE, since)).toBe(true);
    expect(queue.isHidden("Addon999", LIVE, since)).toBe(true);
  });

  it("forgets the log once nothing is in flight", () => {
    const queue = new RemovalQueue(vi.fn());
    const since = queue.stamp();
    queue.markRemoved("Foo", LIVE);
    expect(queue.isHidden("Foo", LIVE, since)).toBe(true);

    queue.release(since);

    // The stale stamp is now meaningless, and a fresh request sees the folder
    // exactly as the backend reports it.
    expect(queue.isHidden("Foo", LIVE, queue.stamp())).toBe(false);
  });

  it("holds entries the OLDEST in-flight request still needs", () => {
    const queue = new RemovalQueue(vi.fn());
    const early = queue.stamp();
    queue.markRemoved("Foo", LIVE);
    const late = queue.stamp();

    // Releasing the later request must not drop what the earlier one needs.
    queue.release(late);
    expect(queue.isHidden("Foo", LIVE, early)).toBe(true);
    expect(queue.isHidden("Foo", LIVE, late)).toBe(false);
  });

  it("refcounts concurrent requests that share a stamp", () => {
    // Two scans issued back to back with no removal between them take the same
    // generation; the first to settle must not free the second's history.
    const queue = new RemovalQueue(vi.fn());
    const a = queue.stamp();
    const b = queue.stamp();
    expect(a).toBe(b);

    queue.markRemoved("Foo", LIVE);
    queue.release(a);
    expect(queue.isHidden("Foo", LIVE, b)).toBe(true);

    queue.release(b);
    expect(queue.isHidden("Foo", LIVE, queue.stamp())).toBe(false);
  });

  it("ignores a release for a stamp it never issued", () => {
    const queue = new RemovalQueue(vi.fn());
    const since = queue.stamp();
    queue.markRemoved("Foo", LIVE);

    queue.release(9999);

    expect(queue.isHidden("Foo", LIVE, since)).toBe(true);
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
