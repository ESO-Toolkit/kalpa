// The live-session engine for the uploader workspace: all live-mode state + the
// ref-based session-ownership protocol, the start/stop/force-handoff handlers, and
// the last-resort unmount teardown. Extracted from uploader-workspace.tsx with NO
// behavior change — the workspace passes in the selection/options context it reads
// and the refresh callbacks it drives, and renders off the returned state/refs.
//
// The per-session live channel's `onmessage` is delegated to the pure
// `applyLiveEvent` reducer (live-event-reducer.ts) so the dedup / StrictMode
// double-count / session-reset / stale-event-guard / stopped-eviction logic is
// unit-testable.
//
// Load-bearing invariants preserved verbatim (do not reorder):
//  • the ref-based ownership protocol (`liveSessionIdRef` / `startingRef` /
//    `liveActiveRef`) and handleStopLive's null-the-ref-THEN-await ordering;
//  • handleStartLive's position-sensitive pre-start abort checks;
//  • the unmount cleanup effect's empty-deps ([]) contract (app-quit teardown).

import { useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { getTauriErrorMessage, invokeOrThrow, warnIfSessionNotPersisted } from "@/lib/tauri";
import type {
  FightSummary,
  LiveEvent,
  LiveFight,
  LiveReadiness,
  LogFileInfo,
  ReportRef,
  UploadDispatch,
  UploadOptions,
  Visibility,
} from "@/types/uploader";
import { formatElapsed } from "./uploader-shared";
import { dominantZone } from "./naming";
import { maybeAutoOpenAnalysis, usesOfficialUploader } from "./uploader-actions";
import { applyLiveEvent, type LiveEventContext } from "./live-event-reducer";

/** What the workspace feeds the hook: the current selection/options context the
 *  live handlers read, plus the refresh callbacks they drive. */
export interface UseLiveSessionArgs {
  selectedLog: string | null;
  logs: LogFileInfo[];
  options: UploadOptions;
  fights: FightSummary[];
  /** Resolve + select the active Encounter.log (Go-Live fallback when nothing is
   *  selected). Returns the picked path, or null when there's no encounter log. */
  autoSelectActiveLog: () => string | null;
  /** Re-read the direct-upload opt-in + session presence into the parent's state. */
  refreshNativeState: () => void | Promise<void>;
  /** Reload the upload history after a session settles. */
  refreshHistory: () => Promise<void>;
}

export function useLiveSession({
  selectedLog,
  logs,
  options,
  fights,
  autoSelectActiveLog,
  refreshNativeState,
  refreshHistory,
}: UseLiveSessionArgs) {
  // Live-mode state
  const [liveSessionId, setLiveSessionId] = useState<string | null>(null);
  // Rendered fights are capped to a rolling window (most-recent MAX_LIVE_FIGHTS)
  // so a multi-hour raid can't grow this array / the DOM without bound. The
  // truthful "N detected" count lives in `liveFightCount`, which only counts up.
  const [liveFights, setLiveFights] = useState<LiveFight[]>([]);
  const [liveFightCount, setLiveFightCount] = useState(0);
  // Mirror of the count so the empty-deps unmount cleanup can report the true
  // fight total to the backend without re-subscribing on every fight.
  const liveFightCountRef = useRef(0);
  // Fight indices already counted this session. Used to dedup re-delivered fight
  // events in the EVENT HANDLER (not inside a setState updater): incrementing the
  // count inside the setLiveFights updater double-fires under React StrictMode (which
  // invokes updaters twice), which over-counted every fight (e.g. "2 fights" for 1).
  const seenFightIndicesRef = useRef<Set<number>>(new Set());
  const [liveReport, setLiveReport] = useState<ReportRef | null>(null);
  const [liveVisibility, setLiveVisibility] = useState<Visibility>(options.visibility);
  // Whether the running live session needs the user's attention (e.g. the ESO Logs
  // session expired mid-stream and posting is paused). The only thing the UI reads
  // off the old multi-valued status was `=== "attention"`, so a boolean is exact.
  const [liveNeedsAttention, setLiveNeedsAttention] = useState(false);
  // Which path this live session actually took: true = handed off to the official
  // uploader (a separate app keeps streaming); false = native in-process (Kalpa IS
  // the uploader, Stop ends it). Drives the live dashboard's callout/copy so it's
  // accurate per session, since the same session can route either way depending on
  // opt-in + sign-in. Set from the start dispatch's `handedOff`.
  const [liveHandedOff, setLiveHandedOff] = useState(false);
  // (Native path) whether a logging session has anchored yet — i.e. the driver saw
  // its first BEGIN_LOG and is now streaming. Until then the native path is "armed but
  // waiting" (the encoder needs a session header). Flips on the SessionAnchored event,
  // instantly (no timeout). Drives the waiting↔streaming UI.
  const [sessionAnchored, setSessionAnchored] = useState(false);
  // The pre-Go-Live readiness probe result (native only) — seeds which "waiting"
  // guidance to show first; SessionAnchored then takes over as ground truth.
  const [liveReadiness, setLiveReadiness] = useState<LiveReadiness | null>(null);
  // Wall-clock start of the current live session, for the elapsed timer. Stored
  // as state (drives the timer's mount) and set alongside the session id in
  // handleStartLive. Kept separate from the `live-${ts}` id string so the timer
  // never depends on the id format the session guards key off.
  const [liveStartMs, setLiveStartMs] = useState<number | null>(null);
  const [starting, setStarting] = useState(false);
  // Synchronous re-entry guard for start-live (state updates lag a frame).
  const startingRef = useRef(false);
  // Holds the in-flight live session id from before the start await resolves, so
  // unmounting mid-await still stops the backend watcher (state hasn't landed).
  const liveSessionIdRef = useRef<string | null>(null);
  // Mirrors "a session was actually running (handed off to the uploader)" for the
  // empty-deps unmount cleanup, which can't read `liveSessionId` state (stale
  // closure). Lets the close path show the same handoff reminder as Stop.
  const liveWasRunningRef = useRef(false);
  // Mirror of `liveHandedOff` for the empty-deps unmount cleanup (which can't read
  // the state value — stale closure), so the close toast is path-aware too: native
  // close ends the upload + closes the report, handoff leaves the official uploader
  // streaming.
  const liveHandedOffRef = useRef(false);
  // Gate for the live channel handler: late events queued during the ~poll
  // shutdown window must not fire setState/toast after stop or unmount.
  const liveActiveRef = useRef(false);
  // Native live creates the report before any fight segment exists. Keep the code
  // available for auto-open, but wait until the first accepted fight so raw ESO Logs
  // doesn't open on an intentionally empty report.
  const liveReportRef = useRef<ReportRef | null>(null);
  const liveAutoOpenedRef = useRef(false);

  // Stop any live session when the workspace unmounts (e.g. the dialog is
  // closed). Reads the ref (set before the start await) so a session started but
  // not yet reflected in state is still torn down. Empty deps: this must run
  // only on final unmount.
  //
  // `liveWasRunningRef` tracks whether a session was actually running (handed off
  // to the official uploader). This unmount path bypasses handleStopLive, so it
  // must carry the same honest reminder itself — closing the dialog stops Kalpa's
  // tracking but NOT the separate uploader. The reminder is fired here (not only
  // in handleStopLive) because the close/unmount is a real teardown path. Sonner's
  // <Toaster> is mounted globally (main.tsx), so a toast survives this unmount.
  useEffect(() => {
    return () => {
      liveActiveRef.current = false; // drop any late channel events
      const id = liveSessionIdRef.current;
      // Clear the refs AFTER capturing the id. This is load-bearing for an
      // in-flight start: handleStartLive's success/error arms gate on
      // `liveSessionIdRef.current === sessionId`, so leaving the ref set would let
      // a start that resolves after this unmount pass its guard and run its
      // success arm (setState + a "live started" toast) on an unmounted
      // component. Nulling it makes that stale start drop silently.
      liveSessionIdRef.current = null;
      liveWasRunningRef.current = false;
      if (id) {
        // Warn whenever a live session was active at close — including an
        // in-flight start (`id` set but not yet promoted). Path-aware via the ref
        // mirror (the state isn't readable in this empty-deps closure): a native
        // session's close stops the upload + closes the report; a handoff session
        // leaves the official uploader streaming. For an in-flight start that hasn't
        // resolved handedOff yet, the ref defaults to false (native) — but such a
        // start hasn't launched the official uploader either, so the native wording
        // ("nothing left running") is the honest default there too.
        toast.info(
          liveHandedOffRef.current
            ? "Closed live tracking in Kalpa. The ESO Logs uploader keeps streaming in its own window — stop it there to end the live report."
            : "Closed live tracking in Kalpa — the direct upload was stopped and its report closed.",
          { duration: 8000 }
        );
        void invokeOrThrow("uploader_stop_live", {
          sessionId: id,
          fightCount: liveFightCountRef.current,
        }).catch(() => {});
      }
    };
  }, []);

  const handleStartLive = async (forceHandoffArg: boolean = false) => {
    // Harden against an event accidentally being passed (e.g. onClick={handleStartLive}
    // instead of a wrapper): coerce to a real boolean so a leaked PointerEvent can never
    // read as a truthy `forceHandoff` and silently route to the official uploader.
    const forceHandoff = forceHandoffArg === true;
    // Resolve a target if none is selected (e.g. logs finished loading after the Live
    // tab was clicked): auto-pick the active Encounter.log. `handleSelectLog` is async
    // and won't have updated `selectedLog` state by this tick, so use the path it
    // returns directly for this start.
    let target = selectedLog;
    if (!target) target = autoSelectActiveLog();
    if (!target) {
      // autoSelectActiveLog returns null when there's no *encounter* log to stream.
      // Distinguish "the folder has logs but none are an Encounter.log" (e.g. only
      // Interface.log so far — combat logging was never turned on) from "no logs at
      // all"; both want the same /encounterlog nudge, not a "pick a file" message
      // (there's nothing valid to pick).
      const hasEncounterLog = logs.some((l) => /encounter.*\.log$/i.test(l.fileName));
      toast.error(
        hasEncounterLog
          ? "Pick the Encounter.log to stream first."
          : "No Encounter.log found yet — enable combat logging in ESO (/encounterlog), then try again."
      );
      return;
    }
    // Guard re-entry SYNCHRONOUSLY via a ref: `starting`/`liveSessionId` state
    // doesn't update until the next render, so two clicks in one frame would
    // both pass a state-only check and start two backend watchers (orphaning
    // one). The ref flips immediately.
    // Re-entry guard keyed on the REF, not the render-captured `liveSessionId` state:
    // `handleForceHandoffLive` calls this right after `await handleStopLive()`, which
    // clears `liveSessionIdRef.current` synchronously but won't have re-rendered
    // `liveSessionId` to null yet in this call stack — so a state check here would
    // wrongly no-op the restart and leave nothing running.
    if (startingRef.current || liveSessionIdRef.current) return;
    startingRef.current = true;
    setStarting(true);
    const startedAt = Date.now();
    const sessionId = `live-${startedAt}`;
    const reportVisibility = options.visibility;
    // Record the id before the await so unmount cleanup can stop the backend
    // watcher even if the dialog closes before the await resolves.
    liveSessionIdRef.current = sessionId;
    const channel = new Channel<LiveEvent>();
    setLiveFights([]);
    setLiveFightCount(0);
    liveFightCountRef.current = 0;
    seenFightIndicesRef.current = new Set();
    liveReportRef.current = null;
    setLiveVisibility(reportVisibility);
    liveAutoOpenedRef.current = false;
    setLiveReport(null);
    setSessionAnchored(false); // native: not anchored until the first BEGIN_LOG
    // Reset the handoff path indicator (state + its unmount-toast ref mirror) so this
    // new session doesn't inherit the previous session's path for its copy/toast until
    // the start dispatch resolves the real value.
    setLiveHandedOff(false);
    liveHandedOffRef.current = false;
    setLiveStartMs(startedAt);
    setLiveNeedsAttention(false);
    liveActiveRef.current = true;

    const autoOpenLiveAnalysisOnce = (report: ReportRef) => {
      if (liveAutoOpenedRef.current || liveHandedOffRef.current) return;
      liveAutoOpenedRef.current = true;
      void maybeAutoOpenAnalysis(report, reportVisibility, { live: true });
    };

    // Live events come from either the native direct uploader or the official
    // handoff watcher. Both feed the same session timeline and status UI. The
    // per-session dedup / stale-event-guard / stopped-eviction logic lives in the
    // pure `applyLiveEvent` reducer; build its context here (capturing this start's
    // `sessionId` + once-only auto-open) and delegate every event to it.
    const liveEventCtx: LiveEventContext = {
      sessionId,
      liveActiveRef,
      liveSessionIdRef,
      liveFightCountRef,
      seenFightIndicesRef,
      liveReportRef,
      liveWasRunningRef,
      setLiveNeedsAttention,
      setLiveReport,
      setSessionAnchored,
      setLiveFightCount,
      setLiveFights,
      setLiveSessionId,
      autoOpenLiveAnalysisOnce,
      toast: {
        info: (message, opts) => toast.info(message, opts),
        warning: (message, opts) => toast.warning(message, opts),
        success: (message, opts) => toast.success(message, opts),
        error: (message, opts) => toast.error(message, opts),
      },
      warn: (message) => console.warn("[uploader] live watcher:", message),
      stopLive: (sid, fightCount) =>
        void invokeOrThrow("uploader_stop_live", { sessionId: sid, fightCount }).catch(() => {}),
    };
    channel.onmessage = (ev) => applyLiveEvent(ev, liveEventCtx);

    // Native direct-streaming is the DEFAULT live path: it's faster and keeps the
    // report in-app. The only ways off it are (a) `forceHandoff` — the explicit "go
    // live via the official uploader instead" escape hatch — or (b) the UNIFIED opt-out.
    // `usesOfficialUploader` honours EITHER opt-out key and fails closed on a store read
    // error, identical to the manual routing — so live can't diverge from manual or from
    // the readouts in any split/degraded state.
    const preferOfficial = forceHandoff || (await usesOfficialUploader());

    // Native needs the in-app ESO Logs upload session (the wcl_session cookie), which
    // is SEPARATE from the profile login that gates this dialog (authUser/isLoggedIn).
    // A user can be "signed in" to the dialog yet have no upload session — the old
    // behaviour then silently handed off to the official uploader, which is exactly the
    // surprise we're fixing. So when native is wanted but there's no session, prompt the
    // capture inline and proceed native once it lands; only fall back to handoff if the
    // user cancels/the capture fails.
    let liveHasSession = await invokeOrThrow<boolean>("uploader_has_session").catch(() => false);
    if (!preferOfficial && !liveHasSession) {
      // Bail if the start was superseded while we were checking (mirror of the
      // pre-start abort check below) before opening a sign-in window.
      if (liveSessionIdRef.current !== sessionId) {
        startingRef.current = false;
        setStarting(false);
        return;
      }
      setLiveNeedsAttention(true);
      const signedIn = await invokeOrThrow<{ sessionPersisted?: boolean }>("uploader_login_esologs")
        .then((r) => {
          warnIfSessionNotPersisted(r);
          return true;
        })
        .catch(() => false);
      // Re-read the session: the capture either populated the cookie or it didn't.
      liveHasSession = signedIn
        ? await invokeOrThrow<boolean>("uploader_has_session").catch(() => false)
        : false;
      // Keep the lifted state in sync so the header readout/Direct Upload section
      // reflect the freshly captured (or still-missing) session.
      void refreshNativeState();
      // A stop / mode-switch could have landed during the sign-in window.
      if (liveSessionIdRef.current !== sessionId) {
        startingRef.current = false;
        setStarting(false);
        return;
      }
      // Toast AFTER the abort re-check (and only for the still-current session) so a
      // Stop-during-sign-in doesn't emit a "streaming via the official uploader"
      // message for a start that's about to be abandoned.
      if (!liveHasSession) {
        toast.info(
          "Streaming via the official ESO Logs uploader (sign in to ESO Logs for the faster path)."
        );
      }
      setLiveNeedsAttention(false);
    }

    // Final native decision: wanted AND we have a session (either pre-existing or just
    // captured). Without a session even after prompting, fall back to the handoff.
    const nativeOptIn = !preferOfficial && liveHasSession;

    // Native only: peek whether a fresh logging session is coming, so the waiting
    // state opens with the right guidance (/encounterlog on vs /reloadui). Best-effort
    // — on error we just show generic guidance; SessionAnchored is the ground truth.
    if (nativeOptIn) {
      const readiness = await invokeOrThrow<LiveReadiness>("uploader_probe_live_readiness", {
        filePath: target,
      }).catch(() => null);
      setLiveReadiness(readiness);
    } else {
      setLiveReadiness(null);
    }

    // PRE-START ABORT CHECK. The settings/has_session reads and the readiness probe
    // above are awaited BEFORE `uploader_start_live` registers a backend `Starting`
    // slot — so a stop / switch-to-Manual / dialog-close landing during them runs
    // `uploader_stop_live` against a slot that doesn't exist yet (a no-op), and without
    // this guard the start would then resume and launch an ORPHAN backend session the
    // UI already asked to stop. If we lost ownership (the ref was cleared/replaced),
    // bail before touching the backend. Once `uploader_start_live` runs, the backend's
    // own Starting-slot cancellation-race protocol takes over.
    if (liveSessionIdRef.current !== sessionId) {
      startingRef.current = false;
      setStarting(false);
      return;
    }

    try {
      const dispatch = await invokeOrThrow<UploadDispatch>("uploader_start_live", {
        sessionId,
        filePath: target,
        options,
        channel,
        nativeOptIn,
        // Best-effort content label from the pre-live preflight (live streams new
        // fights, but the session usually continues in the same content).
        zone: dominantZone(fights),
      });
      // The start can resolve Ok AFTER a stop / switch-to-Manual / superseding
      // start ran during the await (handleStopLive already fired
      // uploader_stop_live for this id and cleared the ref). If this start is no
      // longer the current one, do NOT resurrect it: applying the result would
      // set liveSessionId (showing a LIVE session) while liveSessionIdRef is null,
      // so the visible Stop button — which keys off the ref — would no-op, leaving
      // an unclearable phantom session. The backend stop already cancelled/will
      // settle it, so just drop the stale result silently.
      if (liveSessionIdRef.current !== sessionId) return;
      liveWasRunningRef.current = true; // running (handed off OR native); don't re-warn on close
      setLiveSessionId(sessionId);
      const handed = dispatch?.handedOff ?? true; // default to the safe handoff wording
      setLiveHandedOff(handed);
      liveHandedOffRef.current = handed; // mirror for the unmount-cleanup toast
      if (dispatch?.report) {
        liveReportRef.current = dispatch.report;
        setLiveReport(dispatch.report);
      }
      toast.success(
        dispatch?.handedOff
          ? "Live logging started in the official ESO Logs uploader."
          : nativeOptIn
            ? "Live logging started — uploading directly to ESO Logs."
            : "Live logging started."
      );
    } catch (e) {
      // Only act on the failure if THIS start is still current. If a stop /
      // unmount / superseding start replaced us during the await, clearing the
      // refs/status here would clobber that newer session (and toast on an
      // unmounted component); mirror the success arm's guard.
      if (liveSessionIdRef.current !== sessionId) return;
      // Start failed (e.g. uploader not installed): reset the gate and refs so a
      // trailing event can't be processed and the next attempt starts clean.
      liveActiveRef.current = false;
      liveSessionIdRef.current = null;
      setLiveNeedsAttention(true);
      toast.error(`Couldn't start live logging: ${getTauriErrorMessage(e)}`);
    } finally {
      startingRef.current = false;
      setStarting(false);
    }
  };

  const handleStopLive = async () => {
    // Read the session id from the REF, not state: a start sets the ref before
    // the start await resolves but sets `liveSessionId` state only after. Using
    // the ref lets us stop a session that is still starting (e.g. the user
    // switches to Manual mid-start) — the backend turns this into a cancel of
    // the in-flight Starting slot, so the start aborts instead of orphaning a
    // Running watcher with no visible Stop control.
    const id = liveSessionIdRef.current;
    if (!id) return;
    // Clear the ownership refs SYNCHRONOUSLY, before any await. A caller that stops
    // then immediately starts (handleForceHandoffLive) or switches to Manual
    // (`void handleStopLive()` then setMode) must see "no session" instantly — the
    // backend stop is awaited below, but a synchronous follow-on (handleStartLive's
    // re-entry guard, the pre-start abort re-check, the Live-tab auto-select) reads
    // these refs and would otherwise act on the just-stopped session. The post-await
    // cleanup below re-checks `=== id` so it won't clobber a newer session.
    liveSessionIdRef.current = null;
    liveActiveRef.current = false;
    // Was this session actually running (handed off to the uploader), vs still
    // starting? Only a running session left the official uploader streaming, so
    // only then do we remind the user it keeps going.
    const wasRunning = liveWasRunningRef.current;
    const wasHandedOff = liveHandedOffRef.current;
    liveWasRunningRef.current = false;
    liveHandedOffRef.current = false;

    // Snapshot the session stats NOW (before any state clears) so we can show a
    // calm end-of-session summary: how long, how many fights, what content.
    const sessionFightCount = liveFightCountRef.current;
    const sessionDurationMs = liveStartMs ? Date.now() - liveStartMs : 0;
    const sessionZones = Array.from(
      new Set(liveFights.map((f) => f.bossName || f.zoneName).filter((z): z is string => !!z))
    ).slice(0, 3);
    try {
      await invokeOrThrow("uploader_stop_live", {
        sessionId: id,
        fightCount: sessionFightCount,
      });
    } catch {
      /* best-effort */
    }
    // Clear the live UI STATE unless a NEWER session took over during the await. We
    // already nulled the refs synchronously up top; the await can take a while (it
    // joins the driver/watcher + settles history), during which the user could start a
    // NEW session (which sets liveSessionIdRef to a new id). Only skip the state reset
    // in that case — otherwise clear it. `null` means no newer session claimed it, so
    // it's ours to reset.
    if (liveSessionIdRef.current === null) {
      setLiveSessionId(null);
      setLiveNeedsAttention(false);
    }
    if (wasRunning) {
      // Path-aware: on the HANDOFF path Kalpa can't stop the separate official
      // uploader (it may still be streaming); on the NATIVE path Kalpa IS the uploader,
      // so Stop genuinely ended the upload and closed the report.
      toast.info(
        wasHandedOff
          ? "Stopped tracking in Kalpa. The ESO Logs uploader keeps streaming in its own window — stop it there to end the live report."
          : "Stopped the live upload and closed the report on ESO Logs.",
        { duration: 8000 }
      );
    }
    // A calm end-of-session recap when we actually tracked fights — what the
    // night amounted to, at a glance.
    if (sessionFightCount > 0) {
      const dur = sessionDurationMs > 0 ? ` over ${formatElapsed(sessionDurationMs)}` : "";
      const where = sessionZones.length ? ` — ${sessionZones.join(", ")}` : "";
      toast.success(
        `Session recap: ${sessionFightCount} fight${sessionFightCount === 1 ? "" : "s"}${dur}${where}.`,
        { duration: 7000 }
      );
    }
    await refreshHistory();
  };

  // The "go live anyway via the official uploader" escape hatch from the native
  // waiting state (logging already running, no fresh BEGIN_LOG coming). Stop the
  // armed-but-waiting native session and immediately restart it forcing the handoff
  // path — which CAN pick up an in-progress session. Explicit, user-chosen, disclosed
  // (the running callout then shows the handoff copy) — never a silent downgrade.
  const handleForceHandoffLive = async () => {
    await handleStopLive();
    await handleStartLive(true);
  };

  return {
    // state
    liveSessionId,
    liveFights,
    liveFightCount,
    liveReport,
    liveVisibility,
    liveNeedsAttention,
    liveHandedOff,
    sessionAnchored,
    liveReadiness,
    liveStartMs,
    starting,
    // refs the workspace reads in render (session-ownership guards)
    liveSessionIdRef,
    startingRef,
    // handlers
    handleStartLive,
    handleStopLive,
    handleForceHandoffLive,
  };
}
