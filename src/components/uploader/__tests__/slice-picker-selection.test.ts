import { describe, expect, it } from "vitest";
import {
  buildDefaultState,
  buildPlan,
  nightCluster,
  outstandingAfter,
  sessionCheckState,
} from "../slice-picker";
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

describe("outstandingAfter", () => {
  const files = ["a", "b", "c"];

  it("returns the slices still owed, never the ones that succeeded", () => {
    // The regression: this used to return files.slice(0, uploaded) — the
    // successful prefix — so "upload the remaining files" re-published finished
    // reports and never retried the one that failed.
    expect(outstandingAfter(files, 1)).toEqual(["b", "c"]);
    expect(outstandingAfter(files, 2)).toEqual(["c"]);
  });

  it("returns everything when nothing uploaded, and nothing when all did", () => {
    expect(outstandingAfter(files, 0)).toEqual(["a", "b", "c"]);
    expect(outstandingAfter(files, 3)).toEqual([]);
  });

  it("never re-offers work when the count is out of range", () => {
    expect(outstandingAfter(files, 99)).toEqual([]);
    expect(outstandingAfter(files, -1)).toEqual(["a", "b", "c"]);
    expect(outstandingAfter([], 0)).toEqual([]);
  });
});

describe("buildPlan", () => {
  it("promotes a fully-ticked COMPLETE fight list to the whole session", () => {
    const s = session(0, BASE, 2);
    const plans = buildPlan(
      [s],
      fightsFor([[0, [fight(0, 60_000), fight(1, 60_000)]]]),
      new Map([[0, new Set([0, 1])]]),
      new Set()
    );

    expect(plans).toHaveLength(1);
    expect(plans[0]!.kind).toBe("whole");
    expect(plans[0]!.fightCount).toBe(2);
    expect(plans[0]!.sizeBytes).toBe(s.sizeBytes);
  });

  it("never promotes when the listed fights are fewer than the session's real total", () => {
    // The truncated-list case (fightsOmitted): the picker shows 2 of 40 pulls, so
    // ticking everything on screen is still a subset. Promoting it would write and
    // upload the entire night instead of the two pulls that were chosen.
    const plans = buildPlan(
      [session(0, BASE, 40)],
      fightsFor([[0, [fight(0, 60_000), fight(1, 60_000)]]]),
      new Map([[0, new Set([0, 1])]]),
      new Set()
    );

    expect(plans[0]!.kind).toBe("subset");
    expect(plans[0]!.fightCount).toBe(2);
  });

  it("honours an explicit whole-session tick when no fights were listed", () => {
    const plans = buildPlan([session(0, BASE, 40)], fightsFor([[0, []]]), new Map(), new Set([0]));

    expect(plans[0]!.kind).toBe("whole");
    expect(plans[0]!.fightCount).toBe(40);
  });

  it("separates a lone fight from a multi-fight subset and skips untouched sessions", () => {
    const s = session(0, BASE, 3);
    const bySession = fightsFor([[0, [fight(0, 1), fight(1, 1), fight(2, 1)]]]);

    expect(buildPlan([s], bySession, new Map([[0, new Set([1])]]), new Set())[0]!.kind).toBe(
      "single-fight"
    );
    expect(buildPlan([s], bySession, new Map([[0, new Set([0, 2])]]), new Set())[0]!.kind).toBe(
      "subset"
    );
    expect(buildPlan([s], bySession, new Map(), new Set())).toEqual([]);
  });

  it("keeps plan ids distinct when two sessions suggest the same file name", () => {
    // A crash-and-relaunch night yields two whole-session plans in one zone on one
    // date, so their suggested stems are identical by design — each split is
    // written into its own per-invocation output directory, so the files never
    // collide. The ids must still differ: they key the rename drafts and the list.
    const sessions = [session(0, BASE, 1), session(1, BASE + 30 * 60 * 1000, 1)];
    const plans = buildPlan(
      sessions,
      fightsFor([
        [0, [fight(0, 60_000)]],
        [1, [fight(1, 60_000)]],
      ]),
      new Map([
        [0, new Set([0])],
        [1, new Set([1])],
      ]),
      new Set()
    );

    expect(plans).toHaveLength(2);
    expect(plans[0]!.suggestedName).toBe(plans[1]!.suggestedName);
    expect(new Set(plans.map((p) => p.id)).size).toBe(2);
  });
});

describe("sessionCheckState", () => {
  const fights = [fight(0, 1), fight(1, 1)];

  it("reads the whole-session tick alone when fights were omitted or absent", () => {
    expect(sessionCheckState(fights, true, true, new Set())).toBe("all");
    expect(sessionCheckState(fights, true, false, new Set([0, 1]))).toBe("none");
    expect(sessionCheckState([], false, true, new Set())).toBe("all");
    expect(sessionCheckState([], false, false, new Set())).toBe("none");
  });

  it("reports a partial pick as indeterminate", () => {
    expect(sessionCheckState(fights, false, false, new Set())).toBe("none");
    expect(sessionCheckState(fights, false, false, new Set([0]))).toBe("some");
    expect(sessionCheckState(fights, false, false, new Set([0, 1]))).toBe("all");
  });
});

describe("buildDefaultState", () => {
  it("ticks every fight of tonight's sessions and expands them", () => {
    const sessions = [session(0, BASE, 2), session(1, BASE + 30 * 60 * 1000, 1)];
    const state = buildDefaultState(
      sessions,
      fightsFor([
        [0, [fight(0, 1), fight(1, 1)]],
        [1, [fight(2, 1)]],
      ]),
      false
    );

    expect(state.whole).toEqual(new Set());
    expect(state.fights.get(0)).toEqual(new Set([0, 1]));
    expect(state.fights.get(1)).toEqual(new Set([2]));
    expect(state.expanded).toEqual(new Set([0, 1]));
  });

  it("falls back to the whole session when the fight list was omitted", () => {
    const state = buildDefaultState([session(0, BASE, 40)], fightsFor([[0, []]]), true);

    expect(state.whole).toEqual(new Set([0]));
    expect(state.fights.size).toBe(0);
  });

  it("falls back to the whole session when it recorded no fights", () => {
    const state = buildDefaultState([session(0, BASE, 0)], fightsFor([[0, []]]), false);

    expect(state.whole).toEqual(new Set([0]));
  });

  it("leaves earlier nights unticked and collapsed", () => {
    const sessions = [session(0, BASE, 1), session(1, BASE + 26 * HOUR, 1)];
    const state = buildDefaultState(
      sessions,
      fightsFor([
        [0, [fight(0, 60_000)]],
        [1, [fight(1, 60_000)]],
      ]),
      false
    );

    expect(state.fights.has(0)).toBe(false);
    expect(state.fights.get(1)).toEqual(new Set([1]));
    expect(state.expanded).toEqual(new Set([1]));
  });
});
