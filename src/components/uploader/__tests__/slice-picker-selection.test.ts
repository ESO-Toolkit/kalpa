import { describe, expect, it } from "vitest";
import { nightCluster } from "../slice-picker";
import type { FightSummary, LogSession } from "@/types/uploader";

const HOUR = 60 * 60 * 1000;
const BASE = 1_800_000_000_000;

function session(index: number, startTimeMs: number, fightCount = 1): LogSession {
  return {
    index,
    startOffset: index * 1000,
    endOffset: index * 1000 + 900,
    startTimeMs,
    logVersion: "15",
    realm: "NA Megaserver",
    fightCount,
    sizeBytes: 1024,
  };
}

/** One fight ending `durationMs` after its session began. */
function fight(index: number, durationMs: number): FightSummary {
  return {
    index,
    startOffset: 0,
    endOffset: 100,
    startMs: 0,
    endMs: durationMs,
    zoneName: "Sunspire",
    bossName: "Lokkestiiz",
  };
}

function fightsFor(entries: Array<[number, FightSummary[]]>) {
  return new Map<number, FightSummary[]>(entries);
}

describe("nightCluster", () => {
  it("keeps a crash-and-relaunch in the same night", () => {
    // Raid starts, client dies 90 minutes in, player is back 4 minutes later.
    const sessions = [session(0, BASE), session(1, BASE + 94 * 60 * 1000)];
    const fights = fightsFor([
      [0, [fight(0, 90 * 60 * 1000)]],
      [1, [fight(1, 60 * 60 * 1000)]],
    ]);

    expect(nightCluster(sessions, fights)).toEqual(new Set([0, 1]));
  });

  it("does not chain across unrelated logins spaced under the old six-hour rule", () => {
    // The regression: five short sessions five hours apart. The previous rule
    // linked anything within six hours of anything already selected, so all five
    // merged into a single twenty-hour "night".
    const sessions = [
      session(0, BASE),
      session(1, BASE + 5 * HOUR),
      session(2, BASE + 10 * HOUR),
      session(3, BASE + 15 * HOUR),
      session(4, BASE + 20 * HOUR),
    ];
    const fights = fightsFor(
      sessions.map((s) => [s.index, [fight(s.index, 20 * 60 * 1000)]] as [number, FightSummary[]])
    );

    expect(nightCluster(sessions, fights)).toEqual(new Set([4]));
  });

  it("measures the gap from when a session ended, not when it started", () => {
    // A four-hour raid, then a new session one hour after it ended. Start-to-start
    // is five hours; the real break is one.
    const sessions = [session(0, BASE), session(1, BASE + 5 * HOUR)];
    const fights = fightsFor([
      [0, [fight(0, 4 * HOUR)]],
      [1, [fight(1, 30 * 60 * 1000)]],
    ]);

    expect(nightCluster(sessions, fights)).toEqual(new Set([0, 1]));
  });

  it("stops at the first real break instead of hopping over it", () => {
    // Sessions 2 and 3 are one night; session 0 is the night before. Session 1 is
    // separated from 2 by a full day and must break the chain even though 0 and 1
    // are close to each other.
    const sessions = [
      session(0, BASE),
      session(1, BASE + 30 * 60 * 1000),
      session(2, BASE + 26 * HOUR),
      session(3, BASE + 27 * HOUR),
    ];
    const fights = fightsFor(
      sessions.map((s) => [s.index, [fight(s.index, 20 * 60 * 1000)]] as [number, FightSummary[]])
    );

    expect(nightCluster(sessions, fights)).toEqual(new Set([2, 3]));
  });

  it("falls back to start times when the fight list was omitted", () => {
    // Very large logs ship no fight list. With no end times the measured gap can
    // only be larger, which may split a night but never merges unrelated ones.
    const sessions = [session(0, BASE), session(1, BASE + 2 * HOUR)];

    expect(nightCluster(sessions, new Map())).toEqual(new Set([0, 1]));
  });

  it("returns the single session for a one-session log, and nothing for none", () => {
    expect(nightCluster([session(0, BASE)], new Map())).toEqual(new Set([0]));
    expect(nightCluster([], new Map())).toEqual(new Set());
  });

  it("selects the latest night regardless of the order sessions arrive in", () => {
    const sessions = [
      session(2, BASE + 26 * HOUR),
      session(0, BASE),
      session(1, BASE + 26.5 * HOUR),
    ];
    const fights = fightsFor(
      sessions.map((s) => [s.index, [fight(s.index, 15 * 60 * 1000)]] as [number, FightSummary[]])
    );

    expect(nightCluster(sessions, fights)).toEqual(new Set([2, 1]));
  });
});
