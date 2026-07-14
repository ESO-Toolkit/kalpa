// The pure application of one live-session channel event to the session's refs,
// state setters, and injected side effects. Extracted from the uploader
// workspace's per-session `channel.onmessage` so the dedup / StrictMode
// double-count / session-reset / stale-event-guard / stopped-session-eviction
// logic is unit-testable in isolation.
//
// Every dependency (the ownership/dedup refs, the state setters, the toasts, the
// `uploader_stop_live` invoke, the console warn, and the once-only live
// auto-open) is threaded through `ctx`, so this module imports only types and has
// no side effects of its own — a test can pass mock refs/setters/effects and
// assert exactly what each event does.

import type { LiveEvent, LiveFight, ReportRef } from "@/types/uploader";

/** Max live fights kept in React state / the DOM at once. A long raid night can
 *  produce hundreds of fights; we keep a rolling window of the most recent ones
 *  (full history lives on esologs.com) and report the true total separately. */
export const MAX_LIVE_FIGHTS = 150;

/** A minimal mutable-ref shape (`{ current }`) so tests can pass plain objects
 *  instead of real React refs. Structurally matches `MutableRefObject<T>`. */
export interface MutableRef<T> {
  current: T;
}

/** A React-style setState accepting either a value or an updater function, so the
 *  reducer preserves the workspace's exact mix of direct sets (session reset) and
 *  pure updaters (fight append/count) verbatim. */
type SetState<T> = (value: T | ((prev: T) => T)) => void;

/** The toast surface the reducer needs — a subset of sonner's `toast`, injected
 *  so the reducer stays pure and a test can assert on the toasts an event emits. */
export interface LiveEventToast {
  info: (message: string, opts?: { duration?: number }) => void;
  warning: (message: string, opts?: { duration?: number }) => void;
  success: (message: string, opts?: { duration?: number }) => void;
  error: (message: string, opts?: { duration?: number }) => void;
}

/** Everything `applyLiveEvent` reads or drives for ONE live session. Built fresh
 *  per `handleStartLive` (capturing that start's `sessionId` and its once-only
 *  auto-open closure) and handed to the channel's `onmessage`. The refs are the
 *  SAME objects the start/stop handlers and the unmount cleanup share, so
 *  `ctx.<ref>.current` reads live at event time — identical to the old closure. */
export interface LiveEventContext {
  /** The session id this channel belongs to; the stale-event guard compares it
   *  against `liveSessionIdRef.current`. */
  sessionId: string;
  // Ownership / dedup refs (shared with the start/stop handlers + unmount cleanup).
  liveActiveRef: MutableRef<boolean>;
  liveSessionIdRef: MutableRef<string | null>;
  liveFightCountRef: MutableRef<number>;
  seenFightIndicesRef: MutableRef<Set<number>>;
  liveReportRef: MutableRef<ReportRef | null>;
  liveWasRunningRef: MutableRef<boolean>;
  // State setters.
  setLiveNeedsAttention: (v: boolean) => void;
  setLiveReport: (r: ReportRef | null) => void;
  setSessionAnchored: (v: boolean) => void;
  setLiveFightCount: SetState<number>;
  setLiveFights: SetState<LiveFight[]>;
  setLiveSessionId: (v: string | null) => void;
  // Injected side effects.
  autoOpenLiveAnalysisOnce: (report: ReportRef) => void;
  toast: LiveEventToast;
  warn: (message: string) => void;
  /** Best-effort `uploader_stop_live` invoke (fire-and-forget, own `.catch`). */
  stopLive: (sessionId: string, fightCount: number) => void;
}

/** Apply a single live-channel event. Mirrors the original per-session
 *  `channel.onmessage` switch exactly; see `LiveEventContext` for how the refs
 *  and effects are threaded in. */
export function applyLiveEvent(ev: LiveEvent, ctx: LiveEventContext): void {
  // Drop events that don't belong to the CURRENT session. The global
  // liveActiveRef alone is not enough: a previous session's queued event
  // (e.g. the watcher's trailing `Stopped`, delivery to React lagging) can
  // arrive after a NEW session already set liveActiveRef=true and a new
  // liveSessionIdRef. Without this per-session check, that stale event would
  // contaminate the new session — clearing its timeline or, in the `stopped`
  // arm, calling uploader_stop_live for the new id. This context captures its
  // own `sessionId`, so gate on it (which also covers the stopped/closed case
  // the liveActiveRef check used to handle, since the ref is nulled on stop).
  if (!ctx.liveActiveRef.current || ctx.liveSessionIdRef.current !== ctx.sessionId) return;
  switch (ev.type) {
    case "started":
      ctx.setLiveNeedsAttention(false);
      break;
    case "reportOpened":
      // Native live only: the report now has a code (create-report returned),
      // before any fight has posted. Surface it immediately so the user can open
      // the live analysis in the ESO Log Aggregator while the raid is streaming —
      // previously the live code only appeared after the session settled.
      ctx.liveReportRef.current = { code: ev.code, url: ev.url };
      ctx.setLiveReport(ctx.liveReportRef.current);
      if (ctx.liveFightCountRef.current > 0)
        ctx.autoOpenLiveAnalysisOnce(ctx.liveReportRef.current);
      break;
    case "sessionAnchored":
      // Native: the first BEGIN_LOG landed — flip waiting→streaming instantly.
      ctx.setSessionAnchored(true);
      break;
    case "fightDetected": {
      // A fight implies the session anchored, even if the anchored event was
      // missed/coalesced — keep the UI honest.
      ctx.setSessionAnchored(true);
      const detected = ev;
      // Dedup + count in the EVENT-HANDLER body (runs once per event), NOT inside a
      // setState updater. React StrictMode double-invokes updaters in dev, so the old
      // nested `setLiveFightCount` inside the `setLiveFights` updater fired twice and
      // double-counted every fight ("2 fights" for 1). The ref Set dedups re-delivered
      // events; both setStates below use PURE updaters (StrictMode-safe).
      if (ctx.seenFightIndicesRef.current.has(detected.index)) break;
      ctx.seenFightIndicesRef.current.add(detected.index);
      if (ctx.liveReportRef.current) ctx.autoOpenLiveAnalysisOnce(ctx.liveReportRef.current);
      ctx.liveFightCountRef.current += 1;
      ctx.setLiveFightCount((c) => c + 1);
      ctx.setLiveFights((prev) => {
        const next = [
          ...prev,
          {
            index: detected.index,
            zoneName: detected.zoneName,
            bossName: detected.bossName,
            durationMs: detected.durationMs,
          },
        ];
        // Keep only the most recent MAX_LIVE_FIGHTS so a long session can't
        // grow state/DOM without bound.
        return next.length > MAX_LIVE_FIGHTS ? next.slice(-MAX_LIVE_FIGHTS) : next;
      });
      break;
    }
    case "sessionReset":
      ctx.toast.info("A new logging session started — continuing to watch.");
      ctx.setLiveFights([]);
      ctx.setLiveFightCount(0);
      ctx.liveFightCountRef.current = 0;
      ctx.seenFightIndicesRef.current = new Set();
      break;
    case "fightSkipped":
      // A genuinely oversized fight; surface once. The full log still uploads.
      ctx.toast.info(ev.reason);
      break;
    case "warning":
      // Transient (e.g. a read retry) — log but don't toast, as these recur.
      ctx.warn(ev.message);
      break;
    case "reauthRequired":
      // Native live only: the ESO Logs session expired mid-stream. Posting is
      // paused (the report stays open) until the user re-signs-in. Prompt them;
      // the driver resumes automatically once a fresh session is stored.
      ctx.setLiveNeedsAttention(true);
      ctx.toast.warning(ev.message, { duration: 12000 });
      break;
    case "reauthResolved":
      // A fresh session was captured; the driver resumed posting.
      ctx.setLiveNeedsAttention(false);
      ctx.toast.success("Signed back in — resuming the live upload.");
      break;
    case "stopped": {
      // A `stopped` event that passes the session guard above means THIS
      // session ended outside the user's Stop button path: either the watcher
      // failed, or native live finished by END_LOG / idle / server end. The backend
      // may still hold the now-dead `Running` slot, so drive the existing stop path
      // to evict it. Use this context's own `sessionId` (the guard proved it is the
      // current one) so we never settle another session.
      const stoppedFightCount = ctx.liveFightCountRef.current;
      ctx.liveActiveRef.current = false;
      ctx.liveSessionIdRef.current = null;
      ctx.liveWasRunningRef.current = false; // settled; don't re-warn on close
      ctx.setLiveSessionId(null);
      ctx.setLiveNeedsAttention(!ev.clean);
      if (!ev.clean && ev.reason) ctx.toast.error(ev.reason);
      // Best-effort: evicts the dead `Running` slot (stop_slot_in_map) and
      // settles the official-handoff record. Native live self-settles by exact id;
      // this call is still idempotent and removes the finished slot if it remains.
      ctx.stopLive(ctx.sessionId, stoppedFightCount);
      break;
    }
  }
}
