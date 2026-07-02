import { describe, expect, it, vi } from "vitest";

import { applyLiveEvent, MAX_LIVE_FIGHTS, type LiveEventContext } from "../live-event-reducer";
import type { LiveEvent, LiveFight } from "@/types/uploader";

/** Build a reducer context backed by an in-memory liveFights/liveFightCount store.
 *  The store setters DOUBLE-INVOKE any updater function to mimic React StrictMode's
 *  dev-mode double invocation (committing the second result), so a test can prove the
 *  reducer's updaters are pure and its counting never double-fires. Everything else is
 *  a spy; pass `overrides` to seed refs (session id, active flag, prior count/report). */
function makeCtx(overrides: Partial<LiveEventContext> = {}) {
  let fights: LiveFight[] = [];
  let count = 0;
  const setLiveFights = vi.fn((u: LiveFight[] | ((prev: LiveFight[]) => LiveFight[])) => {
    if (typeof u === "function") {
      u(fights); // discarded first invocation (StrictMode)
      fights = u(fights); // committed second invocation
    } else {
      fights = u;
    }
  });
  const setLiveFightCount = vi.fn((u: number | ((c: number) => number)) => {
    if (typeof u === "function") {
      u(count); // discarded first invocation (StrictMode)
      count = u(count); // committed second invocation
    } else {
      count = u;
    }
  });
  const ctx: LiveEventContext = {
    sessionId: "live-1",
    liveActiveRef: { current: true },
    liveSessionIdRef: { current: "live-1" },
    liveFightCountRef: { current: 0 },
    seenFightIndicesRef: { current: new Set<number>() },
    liveReportRef: { current: null },
    liveWasRunningRef: { current: true },
    setLiveNeedsAttention: vi.fn(),
    setLiveReport: vi.fn(),
    setSessionAnchored: vi.fn(),
    setLiveFightCount,
    setLiveFights,
    setLiveSessionId: vi.fn(),
    autoOpenLiveAnalysisOnce: vi.fn(),
    toast: { info: vi.fn(), warning: vi.fn(), success: vi.fn(), error: vi.fn() },
    warn: vi.fn(),
    stopLive: vi.fn(),
    ...overrides,
  };
  return { ctx, getFights: () => fights, getCount: () => count };
}

function fight(index: number): Extract<LiveEvent, { type: "fightDetected" }> {
  return { type: "fightDetected", index, zoneName: "Zone", bossName: "Boss", durationMs: 1000 };
}

describe("applyLiveEvent — fight dedup", () => {
  it("counts a re-delivered fight (same index) only once", () => {
    const { ctx, getFights, getCount } = makeCtx();
    applyLiveEvent(fight(0), ctx);
    applyLiveEvent(fight(0), ctx); // re-delivered duplicate

    expect(ctx.liveFightCountRef.current).toBe(1);
    expect(getCount()).toBe(1);
    expect(getFights()).toHaveLength(1);
    expect(ctx.seenFightIndicesRef.current.has(0)).toBe(true);
  });

  it("counts distinct fight indices, appended in order", () => {
    const { ctx, getFights, getCount } = makeCtx();
    applyLiveEvent(fight(0), ctx);
    applyLiveEvent(fight(1), ctx);
    applyLiveEvent(fight(2), ctx);

    expect(ctx.liveFightCountRef.current).toBe(3);
    expect(getCount()).toBe(3);
    expect(getFights().map((f) => f.index)).toEqual([0, 1, 2]);
  });
});

describe("applyLiveEvent — StrictMode double-invocation safety", () => {
  it("counts each fight exactly once even when React double-invokes the state updaters", () => {
    // The store setters here double-invoke every updater (StrictMode dev mode).
    const { ctx, getFights, getCount } = makeCtx();
    applyLiveEvent(fight(0), ctx);

    // The ref is the source of truth and is incremented once in the handler body —
    // not inside a (double-invoked) updater — so it never over-counts.
    expect(ctx.liveFightCountRef.current).toBe(1);
    // The pure `(c) => c + 1` / append updaters yield the same result on the second
    // invocation, so the committed state is 1 fight, count 1 — not 2.
    expect(getCount()).toBe(1);
    expect(getFights()).toHaveLength(1);
    // Each setter is *called* once per event (the double invocation is React's, inside
    // the setter), so the count is not driven by a nested double-firing setState.
    expect(ctx.setLiveFightCount).toHaveBeenCalledTimes(1);
    expect(ctx.setLiveFights).toHaveBeenCalledTimes(1);
  });

  it("caps the rendered window at MAX_LIVE_FIGHTS while the true count keeps rising", () => {
    const { ctx, getFights, getCount } = makeCtx();
    const total = MAX_LIVE_FIGHTS + 10;
    for (let i = 0; i < total; i++) applyLiveEvent(fight(i), ctx);

    expect(getCount()).toBe(total);
    expect(ctx.liveFightCountRef.current).toBe(total);
    const rendered = getFights();
    expect(rendered).toHaveLength(MAX_LIVE_FIGHTS);
    // The window keeps the MOST RECENT fights.
    expect(rendered[0]?.index).toBe(total - MAX_LIVE_FIGHTS);
    expect(rendered[rendered.length - 1]?.index).toBe(total - 1);
  });
});

describe("applyLiveEvent — session reset", () => {
  it("clears fights, count, and the dedup set so a repeated index counts again", () => {
    const { ctx, getFights, getCount } = makeCtx();
    applyLiveEvent(fight(0), ctx);
    expect(getCount()).toBe(1);

    applyLiveEvent({ type: "sessionReset" }, ctx);

    expect(ctx.toast.info).toHaveBeenCalledWith(
      "A new logging session started — continuing to watch."
    );
    expect(getFights()).toEqual([]);
    expect(getCount()).toBe(0);
    expect(ctx.liveFightCountRef.current).toBe(0);
    expect(ctx.seenFightIndicesRef.current.size).toBe(0);

    // Index 0 is no longer "seen", so the same fight index counts again post-reset.
    applyLiveEvent(fight(0), ctx);
    expect(getCount()).toBe(1);
    expect(ctx.liveFightCountRef.current).toBe(1);
  });
});

describe("applyLiveEvent — stale-event guard", () => {
  it("drops an event whose session id no longer matches (a prior session's queued event)", () => {
    const { ctx, getCount } = makeCtx({ liveSessionIdRef: { current: "live-2" } });
    applyLiveEvent(fight(0), ctx);

    expect(ctx.liveFightCountRef.current).toBe(0);
    expect(getCount()).toBe(0);
    expect(ctx.setLiveFights).not.toHaveBeenCalled();
    expect(ctx.setSessionAnchored).not.toHaveBeenCalled();
  });

  it("drops an event when the session is no longer active", () => {
    const { ctx } = makeCtx({ liveActiveRef: { current: false } });
    applyLiveEvent({ type: "sessionAnchored" }, ctx);
    expect(ctx.setSessionAnchored).not.toHaveBeenCalled();
  });
});

describe("applyLiveEvent — stopped-session eviction", () => {
  it("a clean stop evicts the session and calls stopLive with the snapshot count", () => {
    const { ctx } = makeCtx();
    applyLiveEvent(fight(0), ctx);
    applyLiveEvent(fight(1), ctx);

    applyLiveEvent({ type: "stopped", reason: "", clean: true }, ctx);

    expect(ctx.liveActiveRef.current).toBe(false);
    expect(ctx.liveSessionIdRef.current).toBeNull();
    expect(ctx.liveWasRunningRef.current).toBe(false);
    expect(ctx.setLiveSessionId).toHaveBeenCalledWith(null);
    expect(ctx.setLiveNeedsAttention).toHaveBeenLastCalledWith(false); // !clean === false
    expect(ctx.stopLive).toHaveBeenCalledWith("live-1", 2);
    expect(ctx.toast.error).not.toHaveBeenCalled();
  });

  it("an unclean stop flags attention and surfaces the reason", () => {
    const { ctx } = makeCtx();
    applyLiveEvent({ type: "stopped", reason: "watcher died", clean: false }, ctx);

    expect(ctx.setLiveNeedsAttention).toHaveBeenCalledWith(true);
    expect(ctx.toast.error).toHaveBeenCalledWith("watcher died");
    expect(ctx.stopLive).toHaveBeenCalledWith("live-1", 0);
  });

  it("a STALE stop (wrong session) never evicts the current session", () => {
    const { ctx } = makeCtx({ liveSessionIdRef: { current: "live-2" } });
    applyLiveEvent({ type: "stopped", reason: "", clean: true }, ctx);

    // The guard dropped it: refs untouched, no stop issued, no state reset.
    expect(ctx.liveActiveRef.current).toBe(true);
    expect(ctx.liveSessionIdRef.current).toBe("live-2");
    expect(ctx.stopLive).not.toHaveBeenCalled();
    expect(ctx.setLiveSessionId).not.toHaveBeenCalled();
  });
});

describe("applyLiveEvent — report + auto-open", () => {
  it("reportOpened stores the report but does not auto-open an empty report", () => {
    const { ctx } = makeCtx();
    applyLiveEvent({ type: "reportOpened", code: "abc", url: "https://x/abc" }, ctx);

    expect(ctx.liveReportRef.current).toEqual({ code: "abc", url: "https://x/abc" });
    expect(ctx.setLiveReport).toHaveBeenCalledWith({ code: "abc", url: "https://x/abc" });
    expect(ctx.autoOpenLiveAnalysisOnce).not.toHaveBeenCalled();
  });

  it("reportOpened auto-opens immediately when fights already streamed", () => {
    const { ctx } = makeCtx({ liveFightCountRef: { current: 3 } });
    applyLiveEvent({ type: "reportOpened", code: "abc", url: "u" }, ctx);
    expect(ctx.autoOpenLiveAnalysisOnce).toHaveBeenCalledWith({ code: "abc", url: "u" });
  });

  it("fightDetected auto-opens the live analysis once a report code is known", () => {
    const { ctx } = makeCtx({ liveReportRef: { current: { code: "abc", url: "u" } } });
    applyLiveEvent(fight(0), ctx);
    expect(ctx.autoOpenLiveAnalysisOnce).toHaveBeenCalledWith({ code: "abc", url: "u" });
  });
});

describe("applyLiveEvent — status arms", () => {
  it("reauthRequired pauses (attention) with the server message; reauthResolved clears it", () => {
    const { ctx } = makeCtx();
    applyLiveEvent({ type: "reauthRequired", message: "sign in again" }, ctx);
    expect(ctx.setLiveNeedsAttention).toHaveBeenCalledWith(true);
    expect(ctx.toast.warning).toHaveBeenCalledWith("sign in again", { duration: 12000 });

    applyLiveEvent({ type: "reauthResolved" }, ctx);
    expect(ctx.setLiveNeedsAttention).toHaveBeenLastCalledWith(false);
    expect(ctx.toast.success).toHaveBeenCalled();
  });

  it("warning routes to warn (never a toast); fightSkipped toasts its reason", () => {
    const { ctx } = makeCtx();
    applyLiveEvent({ type: "warning", message: "retrying read" }, ctx);
    expect(ctx.warn).toHaveBeenCalledWith("retrying read");
    expect(ctx.toast.info).not.toHaveBeenCalled();

    applyLiveEvent({ type: "fightSkipped", reason: "fight too large" }, ctx);
    expect(ctx.toast.info).toHaveBeenCalledWith("fight too large");
  });

  it("started clears attention; sessionAnchored flips the anchored flag", () => {
    const { ctx } = makeCtx();
    applyLiveEvent({ type: "started", file: "Encounter.log", startOffset: 0 }, ctx);
    expect(ctx.setLiveNeedsAttention).toHaveBeenCalledWith(false);

    applyLiveEvent({ type: "sessionAnchored" }, ctx);
    expect(ctx.setSessionAnchored).toHaveBeenCalledWith(true);
  });
});
