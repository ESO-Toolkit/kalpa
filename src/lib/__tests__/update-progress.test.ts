import { describe, it, expect } from "vitest";

type AddonPhase = "downloading" | "scanning" | "extracting" | "completed" | "failed";

// Replicate the progress calculation and sorting logic from update-banner.tsx

function computeProgress(
  updateProgress: {
    completed: number;
    failed: number;
    total: number;
  } | null
): { doneCount: number; progressPct: string; allDone: boolean } {
  const total = updateProgress?.total ?? 0;
  const doneCount = (updateProgress?.completed ?? 0) + (updateProgress?.failed ?? 0);
  const allDone = total > 0 && doneCount === total;
  const progressPct = total > 0 ? ((doneCount / total) * 100).toFixed(0) : "0";
  return { doneCount, progressPct, allDone };
}

function sortStatuses(statuses: Map<string, AddonPhase>): [string, AddonPhase][] {
  return [...statuses.entries()].sort((a, b) => {
    const order: Record<AddonPhase, number> = {
      downloading: 0,
      scanning: 1,
      extracting: 2,
      failed: 3,
      completed: 4,
    };
    return order[a[1]] - order[b[1]];
  });
}

describe("computeProgress", () => {
  it("returns zeros for null progress", () => {
    const result = computeProgress(null);
    expect(result.doneCount).toBe(0);
    expect(result.progressPct).toBe("0");
    expect(result.allDone).toBe(false);
  });

  it("calculates percentage correctly", () => {
    expect(computeProgress({ completed: 5, failed: 0, total: 10 }).progressPct).toBe("50");
    expect(computeProgress({ completed: 10, failed: 0, total: 10 }).progressPct).toBe("100");
    expect(computeProgress({ completed: 1, failed: 0, total: 3 }).progressPct).toBe("33");
  });

  it("counts both completed and failed as done", () => {
    const result = computeProgress({ completed: 3, failed: 2, total: 10 });
    expect(result.doneCount).toBe(5);
    expect(result.progressPct).toBe("50");
  });

  it("detects all-done state", () => {
    expect(computeProgress({ completed: 8, failed: 2, total: 10 }).allDone).toBe(true);
    expect(computeProgress({ completed: 10, failed: 0, total: 10 }).allDone).toBe(true);
    expect(computeProgress({ completed: 9, failed: 0, total: 10 }).allDone).toBe(false);
  });

  it("handles zero total", () => {
    const result = computeProgress({ completed: 0, failed: 0, total: 0 });
    expect(result.progressPct).toBe("0");
    expect(result.allDone).toBe(false);
  });

  it("handles all-failed scenario", () => {
    const result = computeProgress({ completed: 0, failed: 5, total: 5 });
    expect(result.allDone).toBe(true);
    expect(result.progressPct).toBe("100");
    expect(result.doneCount).toBe(5);
  });
});

describe("sortStatuses", () => {
  it("sorts in-progress phases before completed/failed", () => {
    const statuses = new Map<string, AddonPhase>([
      ["addon1", "completed"],
      ["addon2", "downloading"],
      ["addon3", "failed"],
      ["addon4", "extracting"],
      ["addon5", "scanning"],
    ]);

    const sorted = sortStatuses(statuses);
    expect(sorted.map(([, phase]) => phase)).toEqual([
      "downloading",
      "scanning",
      "extracting",
      "failed",
      "completed",
    ]);
  });

  it("preserves order within same phase", () => {
    const statuses = new Map<string, AddonPhase>([
      ["b", "downloading"],
      ["a", "downloading"],
    ]);
    const sorted = sortStatuses(statuses);
    expect(sorted.map(([name]) => name)).toEqual(["b", "a"]);
  });

  it("handles empty map", () => {
    expect(sortStatuses(new Map())).toEqual([]);
  });

  it("handles single entry", () => {
    const statuses = new Map<string, AddonPhase>([["only", "completed"]]);
    expect(sortStatuses(statuses)).toEqual([["only", "completed"]]);
  });

  it("puts failed before completed", () => {
    const statuses = new Map<string, AddonPhase>([
      ["success", "completed"],
      ["error", "failed"],
    ]);
    const sorted = sortStatuses(statuses);
    expect(sorted[0]![0]).toBe("error");
    expect(sorted[1]![0]).toBe("success");
  });
});
