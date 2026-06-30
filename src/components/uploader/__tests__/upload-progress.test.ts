import { describe, expect, it } from "vitest";

import { etaLabel, formatEta, uploadTargetFraction } from "../upload-progress";
import type { UploadProgressState } from "../upload-progress";

const state = (over: Partial<UploadProgressState>): UploadProgressState => ({
  phase: "uploading",
  segmentsDone: 0,
  segmentsTotal: 0,
  startMs: 0,
  ...over,
});

describe("uploadTargetFraction", () => {
  it("creeps from 0 toward the 0.15 cap while preparing, never past it", () => {
    const f0 = uploadTargetFraction(state({ phase: "preparing" }), 0);
    const fLater = uploadTargetFraction(state({ phase: "preparing" }), 4000);
    const fHuge = uploadTargetFraction(state({ phase: "preparing" }), 1_000_000);
    expect(f0).toBeCloseTo(0, 5);
    expect(fLater).toBeGreaterThan(f0);
    expect(fHuge).toBeLessThanOrEqual(0.15);
    expect(fHuge).toBeGreaterThan(0.14);
  });

  it("anchors the band floor at the segments already accepted", () => {
    // At phase-elapsed 0 there is no creep yet, so the target is the accepted-segment
    // floor.
    expect(uploadTargetFraction(state({ segmentsDone: 0, segmentsTotal: 4 }), 0)).toBeCloseTo(0.15);
    expect(uploadTargetFraction(state({ segmentsDone: 4, segmentsTotal: 4 }), 0)).toBeCloseTo(0.93);
    expect(uploadTargetFraction(state({ segmentsDone: 2, segmentsTotal: 4 }), 0)).toBeCloseTo(0.54);
  });

  it("creeps the single in-flight segment instead of freezing at its floor", () => {
    // The native manual path: uploading {done: 0, total: 1} held for the whole POST.
    // Must move past 0.15 over time, but stop short of the segment's 0.93 checkpoint
    // so the real "accepted" tick still advances the bar visibly.
    const atStart = uploadTargetFraction(state({ segmentsDone: 0, segmentsTotal: 1 }), 0);
    const after5s = uploadTargetFraction(state({ segmentsDone: 0, segmentsTotal: 1 }), 5000);
    const after30s = uploadTargetFraction(state({ segmentsDone: 0, segmentsTotal: 1 }), 30_000);
    expect(atStart).toBeCloseTo(0.15);
    expect(after5s).toBeGreaterThan(0.15);
    expect(after30s).toBeGreaterThan(after5s);
    expect(after30s).toBeLessThan(0.93);
  });

  it("treats an unknown segment count as a single in-flight segment", () => {
    expect(uploadTargetFraction(state({ segmentsTotal: 0 }), 0)).toBeCloseTo(0.15);
    expect(uploadTargetFraction(state({ segmentsTotal: 0 }), 30_000)).toBeGreaterThan(0.15);
  });

  it("creeps only within the current segment's band on a multi-segment upload", () => {
    // One of four accepted: floor 0.34, next checkpoint 0.535 — creep stays under it.
    const crept = uploadTargetFraction(state({ segmentsDone: 1, segmentsTotal: 4 }), 1_000_000);
    expect(crept).toBeGreaterThan(0.34);
    expect(crept).toBeLessThan(0.535);
  });

  it("reserves the tail for finalizing and completes when done", () => {
    expect(uploadTargetFraction(state({ phase: "finalizing" }), 0)).toBeCloseTo(0.96);
    expect(uploadTargetFraction(state({ phase: "done" }), 0)).toBe(1);
  });
});

describe("formatEta", () => {
  it("renders sub-second and second ranges compactly", () => {
    expect(formatEta(400)).toBe("<1s");
    expect(formatEta(4200)).toBe("4s");
    expect(formatEta(45_000)).toBe("45s");
  });

  it("switches to m:ss past a minute", () => {
    expect(formatEta(75_000)).toBe("1:15");
    expect(formatEta(605_000)).toBe("10:05");
  });
});

describe("etaLabel", () => {
  it("shows soft copy at the boundaries where a number would be noise", () => {
    expect(etaLabel("done", 1, 5000)).toBe("Done");
    expect(etaLabel("finalizing", 0.96, 5000)).toBe("Finishing up…");
    expect(etaLabel("uploading", 0.95, 5000)).toBe("Finishing up…");
    expect(etaLabel("uploading", 0.02, 5000)).toBe("Estimating…");
    expect(etaLabel("uploading", 0.5, 100)).toBe("Estimating…");
  });

  it("derives a time-remaining estimate from elapsed and fraction", () => {
    // Half done after 10s → roughly 10s left.
    expect(etaLabel("uploading", 0.5, 10_000)).toBe("about 10s left");
  });
});
