// The ESO Logs uploader workspace. Full-screen dialog with a glanceable status
// pill, Manual / Live mode tabs, an auto-detected log picker with preflight, a
// per-fight timeline, split-to-disk for oversized logs, upload history, and a
// first-run wizard. Uploads are handed to the official ESO Logs uploader (Kalpa
// never speaks the private upload protocol itself).

import { useCallback, useEffect, useMemo, useRef, useState, type Ref } from "react";
import { Channel } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import {
  CloudUpload,
  FileText,
  FolderSearch,
  Link as LinkIcon,
  Radio,
  RefreshCw,
  Scissors,
  Upload,
  ExternalLink,
  Copy,
  Zap,
  Check,
  AlertCircle,
  AlertTriangle,
  ChevronRight,
  Swords,
  Search,
  ArrowDownUp,
  FolderOpen,
  CheckCircle2,
  LogIn,
  Trash2,
  FolderInput,
  ClipboardCopy,
} from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { GlassPanel } from "@/components/ui/glass-panel";
import { SectionHeader } from "@/components/ui/section-header";
import { InfoPill } from "@/components/ui/info-pill";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { getTauriErrorMessage, invokeOrThrow, warnIfSessionNotPersisted } from "@/lib/tauri";
import { getSetting, setSetting } from "@/lib/store";
import { cn } from "@/lib/utils";
import type { AuthUser } from "@/types";
import {
  REGION_OPTIONS,
  type FightSummary,
  type LiveEvent,
  type LiveFight,
  type LiveReadiness,
  type LogFileInfo,
  type LogPathDetection,
  type LogPreflight,
  type ReportRef,
  type TransportInfo,
  type UploaderStatus,
  type UploadDispatch,
  type UploadOptions,
  type UploadRecord,
  type Visibility,
} from "@/types/uploader";
import {
  SessionTimer,
  StatusPill,
  WhatGetsUploaded,
  compactBytes,
  esotkReportUrl,
  formatElapsed,
  relativeFromMs,
} from "./uploader-shared";
import { UploadOptionsControl } from "./upload-options";
import { FightList, rowsFromLive, rowsFromSummaries } from "./fight-list";
import { SplitWorkbench } from "./split-workbench";
import { dominantZone } from "./naming";

interface UploaderWorkspaceProps {
  authUser: AuthUser | null;
  onAuthChange: (user: AuthUser | null) => void;
  onClose: () => void;
}

type Mode = "manual" | "live";

const DEFAULT_OPTIONS: UploadOptions = {
  region: 1,
  guildId: null,
  visibility: "unlisted",
  description: null,
  realTime: false,
  includeEntireFile: false,
};

const OPTIONS_KEY = "kalpa.uploader.options";

/** A mid-tier "raised work panel" — sits clearly above the dark canvas but
 *  quieter than the primary picker/action. Used for fights, options, history so
 *  the elevation order reads: canvas < these < picker/action. */
const WORK_PANEL =
  "rounded-2xl border border-white/[0.08] bg-gradient-to-b from-white/[0.045] to-white/[0.015] shadow-[0_8px_28px_-14px_rgba(0,0,0,0.65),inset_0_1px_0_rgba(255,255,255,0.05)]";

/** Open a report URL in the user's browser, surfacing failures instead of
 *  swallowing them. The opener plugin rejects a URL outside the capability's
 *  allow-scope (now includes esologs.com/reports/*); a rejection should toast,
 *  not vanish into an unhandled promise. */
async function openReportUrl(url: string): Promise<void> {
  try {
    const m = await import("@tauri-apps/plugin-opener");
    await m.openUrl(url);
  } catch {
    toast.error("Couldn't open the report — copy the link and open it manually.");
  }
}

/** Open the ESO Log Aggregator analysis for `code` IFF the user enabled auto-open
 *  (the `autoOpenAnalysis` setting, default off). Best-effort: a disabled setting,
 *  a read failure, or an opener-scope rejection is silent — the always-present
 *  "View analysis" button covers the manual case. `live` opens esotk's LiveLog view
 *  (for an in-progress native session). */
async function maybeAutoOpenAnalysis(code: string, opts?: { live?: boolean }): Promise<void> {
  try {
    const auto = await getSetting<boolean>("autoOpenAnalysis", false);
    if (!auto) return;
    // Open directly (not via openReportUrl) so a failure stays SILENT: the user
    // didn't click anything, so an opener-scope rejection or read error must not
    // pop a "couldn't open" toast. The always-present "View analysis" button covers
    // the manual path.
    const m = await import("@tauri-apps/plugin-opener");
    await m.openUrl(esotkReportUrl(code, opts));
  } catch {
    /* best-effort — the manual button still works */
  }
}

/** Max live fights kept in React state / the DOM at once. A long raid night can
 *  produce hundreds of fights; we keep a rolling window of the most recent ones
 *  (full history lives on esologs.com) and report the true total separately. */
const MAX_LIVE_FIGHTS = 150;

const VALID_REGIONS = new Set(REGION_OPTIONS.map((r) => r.id));
const VALID_VISIBILITY = new Set<Visibility>(["public", "unlisted", "private"]);

/** Load persisted options, validating each field so a corrupt/out-of-range
 *  localStorage value (e.g. an invalid region id, which is a u8 in Rust and
 *  would silently produce a bad upload) can't poison the upload. */
function loadSavedOptions(): UploadOptions {
  try {
    const raw = localStorage.getItem(OPTIONS_KEY);
    if (!raw) return DEFAULT_OPTIONS;
    const parsed = JSON.parse(raw) as Partial<UploadOptions>;
    return {
      region: VALID_REGIONS.has(parsed.region as number)
        ? (parsed.region as number)
        : DEFAULT_OPTIONS.region,
      guildId: typeof parsed.guildId === "string" ? parsed.guildId : null,
      visibility: VALID_VISIBILITY.has(parsed.visibility as Visibility)
        ? (parsed.visibility as Visibility)
        : DEFAULT_OPTIONS.visibility,
      description: typeof parsed.description === "string" ? parsed.description : null,
      realTime: typeof parsed.realTime === "boolean" ? parsed.realTime : false,
      includeEntireFile:
        typeof parsed.includeEntireFile === "boolean" ? parsed.includeEntireFile : false,
    };
  } catch {
    return DEFAULT_OPTIONS;
  }
}

export function UploaderWorkspace({ authUser, onAuthChange, onClose }: UploaderWorkspaceProps) {
  const [mode, setMode] = useState<Mode>("manual");
  const [detection, setDetection] = useState<LogPathDetection | null>(null);
  const [logsDir, setLogsDir] = useState<string | null>(null);
  const [logs, setLogs] = useState<LogFileInfo[]>([]);
  // Distinguish "folder read failed" from "folder is genuinely empty" so the
  // empty state doesn't misreport an access error as "No log files found" (L17).
  const [listError, setListError] = useState<string | null>(null);
  const [selectedLog, setSelectedLog] = useState<string | null>(null);
  const [preflight, setPreflight] = useState<LogPreflight | null>(null);
  const [fights, setFights] = useState<FightSummary[]>([]);
  const [scanning, setScanning] = useState(false);
  // Monotonic token guarding against an out-of-order async scan result
  // overwriting the currently-selected log's fights.
  const selectTokenRef = useRef(0);
  const [options, setOptions] = useState<UploadOptions>(loadSavedOptions);
  const [transport, setTransport] = useState<TransportInfo | null>(null);
  const [history, setHistory] = useState<UploadRecord[]>([]);
  const [uploading, setUploading] = useState(false);
  const [workbenchOpen, setWorkbenchOpen] = useState(false);
  // True while a file is dragged over the window — drives the picker drop-zone
  // visual. `importing` covers the copy-in of a dropped out-of-folder log.
  const [dragOver, setDragOver] = useState(false);
  const [importing, setImporting] = useState(false);
  // The log queued for deletion (drives the DeleteLogConfirm dialog); null = no
  // pending delete. `deleting` guards the confirm button while the move runs.
  const [deleteTarget, setDeleteTarget] = useState<LogFileInfo | null>(null);
  const [deleting, setDeleting] = useState(false);

  // Direct (native) upload state, lifted here so both the promoted Direct Upload
  // section and the upload action can reflect which transport will run. Native is
  // now the DEFAULT for manual too: `nativeOptIn` = NOT the `manualUseOfficialUploader`
  // opt-out (default false → native), mirroring live's `liveUseOfficialUploader`.
  // `hasSession` is whether the in-app esologs upload cookie is present. Direct upload
  // is the *intended* path only when nativeOptIn AND hasSession — the backend coverage
  // gate still has final say per log (an unproven event type falls back).
  const [nativeOptIn, setNativeOptIn] = useState(false);
  const [hasNativeSession, setHasNativeSession] = useState(false);
  // Direct (native) upload is the *intended* path — in BOTH modes — when the user has
  // opted in AND captured an upload session. Live now routes native under the same
  // gate (the backend's per-segment coverage + format flag still have final say), so
  // the header transport readout must reflect native in live too; previously it was
  // forced to manual-only, which wrongly claimed "Official uploader" for a live
  // session that would actually go native. The manual-only consumers (ManualActions,
  // LogSummaryCard) only render in manual mode, so broadening this is safe for them.
  const willUseNative = nativeOptIn && hasNativeSession;

  // Live mode defaults to native for everyone (the official handoff is an explicit
  // opt-out via `liveUseOfficialUploader`, default false); manual now mirrors this with
  // its own `manualUseOfficialUploader` opt-out. The readout must stay HONEST, though: native
  // also requires an upload session, and Go Live can fail/decline the sign-in prompt
  // and hand off. So gate the live readout on `hasNativeSession` exactly as manual does
  // (line above) — showing "ESO Logs uploader" until a session is captured. This
  // under-promises only in the narrow "user will sign in at Go Live" case (the safe
  // direction) and never claims "Direct from Kalpa" for a session that handed off.
  const [liveUseOfficial, setLiveUseOfficial] = useState(false);
  const liveWillUseNative = !liveUseOfficial && hasNativeSession;

  // The transport hint for the CURRENT mode. Several shared panels (the header
  // readout, LogSummaryCard's route chip, UploadOptionsControl's report-name field)
  // render in BOTH modes, so they must reflect the mode-correct flag — live uses a
  // different opt-out (`liveUseOfficialUploader`) than manual (`manualUseOfficialUploader`).
  // Passing the manual `willUseNative` into them while in live mode made the route
  // chip / report-name field contradict the (mode-aware) header.
  const activeWillUseNative = mode === "live" ? liveWillUseNative : willUseNative;

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
  const [liveStatus, setLiveStatus] = useState<UploaderStatus>("idle");
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

  // Current selection, mirrored to a ref so loadLogs can reconcile it without
  // being re-created on every selection change.
  const selectedLogRef = useRef<string | null>(null);
  useEffect(() => {
    selectedLogRef.current = selectedLog;
  }, [selectedLog]);

  const clearSelection = useCallback(() => {
    selectTokenRef.current++; // drop any in-flight scan result
    setSelectedLog(null);
    setPreflight(null);
    setFights([]);
    setScanning(false);
  }, []);

  // Persist options whenever they change.
  useEffect(() => {
    try {
      localStorage.setItem(OPTIONS_KEY, JSON.stringify(options));
    } catch {
      /* ignore */
    }
  }, [options]);

  const refreshHistory = useCallback(async () => {
    try {
      const records = await invokeOrThrow<UploadRecord[]>("uploader_list_history");
      setHistory(records);
    } catch {
      /* history is best-effort */
    }
  }, []);

  const loadLogs = useCallback(
    async (dir: string) => {
      try {
        const files = await invokeOrThrow<LogFileInfo[]>("uploader_list_logs", { logsDir: dir });
        setLogs(files);
        setListError(null);
        // If the previously-selected log is gone (rotated/deleted on relog or a
        // patch), drop the stale selection so its preflight can't be acted on.
        const sel = selectedLogRef.current;
        if (sel && !files.some((f) => f.path === sel)) {
          clearSelection();
        }
      } catch (e) {
        const msg = getTauriErrorMessage(e);
        // Record the failure so the empty state can show the real reason instead
        // of "No log files found"; also clear any stale list from a prior folder.
        setListError(msg);
        setLogs([]);
        toast.error(`Couldn't list logs: ${msg}`);
      }
    },
    [clearSelection]
  );

  // Initial detection + transport + history.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const [det, tinfo] = await Promise.all([
          invokeOrThrow<LogPathDetection>("uploader_detect_path"),
          invokeOrThrow<TransportInfo>("uploader_transport_info"),
        ]);
        if (cancelled) return;
        setDetection(det);
        setTransport(tinfo);
        if (det.path) {
          setLogsDir(det.path);
          await loadLogs(det.path);
        }
      } catch (e) {
        if (!cancelled) toast.error(getTauriErrorMessage(e));
      }
      if (!cancelled) await refreshHistory();
    })();
    return () => {
      cancelled = true;
    };
  }, [loadLogs, refreshHistory]);

  // Re-read the direct-upload opt-in + session presence. Called on mount and
  // after the user enables/signs in/out inline, so the promoted section and the
  // transport hint stay in sync with Settings and the credential store.
  const refreshNativeState = useCallback(async () => {
    try {
      const [manualOfficial, session, liveOfficial] = await Promise.all([
        // Manual now mirrors live: native is the DEFAULT, opt-OUT via
        // `manualUseOfficialUploader` (default false → native). `nativeOptIn` here
        // is the "intends native" flag the readout/section read, so it's the negation.
        getSetting<boolean>("manualUseOfficialUploader", false),
        invokeOrThrow<boolean>("uploader_has_session"),
        getSetting<boolean>("liveUseOfficialUploader", false),
      ]);
      setNativeOptIn(!manualOfficial);
      setHasNativeSession(session);
      setLiveUseOfficial(liveOfficial);
    } catch {
      /* best-effort — the upload path still reads the setting fresh per upload */
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const [manualOfficial, session, liveOfficial] = await Promise.all([
        getSetting<boolean>("manualUseOfficialUploader", false),
        invokeOrThrow<boolean>("uploader_has_session").catch(() => false),
        getSetting<boolean>("liveUseOfficialUploader", false),
      ]);
      if (cancelled) return;
      setNativeOptIn(!manualOfficial);
      setHasNativeSession(session);
      setLiveUseOfficial(liveOfficial);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

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

  const handlePickFolder = async () => {
    // The folder picker is the documented manual recovery when auto-detection
    // fails; an OS dialog error must surface, not silently no-op.
    try {
      const picked = await openDialog({ directory: true, title: "Select your ESO Logs folder" });
      if (typeof picked === "string" && picked !== logsDir) {
        clearSelection(); // switching folders invalidates the current selection
        setLogsDir(picked);
        void loadLogs(picked);
      }
    } catch (e) {
      toast.error(`Couldn't open the folder picker: ${getTauriErrorMessage(e)}`);
    }
  };

  // Reveal the current logs directory in the OS file manager. Best-effort: a
  // missing dir or opener rejection toasts rather than silently no-ops.
  const handleOpenLogsFolder = useCallback(async () => {
    if (!logsDir) return;
    try {
      // revealItemInDir is the opener API already used for split output; pointing
      // it at the directory opens the folder in the OS file manager.
      const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
      await revealItemInDir(logsDir);
    } catch {
      toast.error("Couldn't open the Logs folder.");
    }
  }, [logsDir]);

  // Reveal a single log file in the OS file manager.
  const handleRevealLog = useCallback(async (path: string) => {
    try {
      const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
      await revealItemInDir(path);
    } catch {
      toast.error("Couldn't reveal that file.");
    }
  }, []);

  // Copy a log's full path to the clipboard.
  const handleCopyPath = useCallback(async (path: string) => {
    try {
      await navigator.clipboard.writeText(path);
      toast.success("Path copied.");
    } catch {
      toast.error("Couldn't copy the path.");
    }
  }, []);

  // Restore a log from the recycle bin back into the Logs folder (the undo path
  // for a delete). Best-effort: a failure leaves the file safe in the recycle
  // folder, so we don't cascade — manual recovery still exists.
  const restoreLog = useCallback(
    async (recyclePath: string) => {
      try {
        const restored = await invokeOrThrow<string>("uploader_restore_log", { recyclePath });
        if (logsDir) await loadLogs(logsDir);
        toast.success("Log restored.");
        return restored;
      } catch (e) {
        toast.error(`Couldn't restore the log: ${getTauriErrorMessage(e)}`);
      }
    },
    [logsDir, loadLogs]
  );

  // Move the queued log to the recycle bin (soft delete). If it's the currently
  // selected log, clear the selection FIRST — clearSelection() bumps
  // selectTokenRef, orphaning any in-flight preflight scan (reuse the existing
  // guard; don't hand-roll a new one). The toast offers one-tap Restore.
  const handleConfirmDelete = useCallback(async () => {
    const target = deleteTarget;
    if (!target) return;
    setDeleting(true);
    try {
      if (selectedLogRef.current === target.path) clearSelection();
      const recyclePath = await invokeOrThrow<string>("uploader_delete_log", {
        filePath: target.path,
      });
      setLogs((prev) => prev.filter((l) => l.path !== target.path));
      setDeleteTarget(null);
      toast.success("Log moved to recycle bin.", {
        duration: 7000,
        action: { label: "Restore", onClick: () => void restoreLog(recyclePath) },
      });
      if (logsDir) void loadLogs(logsDir);
    } catch (e) {
      toast.error(`Couldn't delete the log: ${getTauriErrorMessage(e)}`);
    } finally {
      setDeleting(false);
    }
  }, [deleteTarget, restoreLog, logsDir, loadLogs, clearSelection]);

  const handleSelectLog = useCallback(async (path: string) => {
    // Guard against a slow scan of a previously-selected log resolving after a
    // newer selection and overwriting its results.
    const token = ++selectTokenRef.current;
    setSelectedLog(path);
    setPreflight(null);
    setFights([]);
    setScanning(true);
    try {
      // A single preflight scan returns both the counts and (unless the log is
      // huge) the fight list — no second scan needed.
      const pre = await invokeOrThrow<LogPreflight>("uploader_preflight", { filePath: path });
      if (selectTokenRef.current !== token) return;
      setPreflight(pre);
      setFights(pre.fights);
      // Keyboard continuity: a slow scan can blur the row (re-renders, the
      // "Scanning" pill swap). Once this scan is still the current one, restore
      // focus to the selected row so keyboard users aren't stranded. Deferred a
      // tick so it runs after the post-setState re-render.
      if (selectTokenRef.current === token) {
        const sel = CSS.escape(path);
        setTimeout(() => {
          if (selectTokenRef.current !== token) return;
          document.querySelector<HTMLButtonElement>(`[data-log-path="${sel}"]`)?.focus();
        }, 0);
      }
    } catch (e) {
      if (selectTokenRef.current !== token) return;
      toast.error(`Couldn't read that log: ${getTauriErrorMessage(e)}`);
    } finally {
      if (selectTokenRef.current === token) setScanning(false);
    }
  }, []);

  // Import a dropped .log: the backend copies it into the Logs folder (or uses it
  // in place if already there), then we refresh the list and select the result so
  // it flows through the normal preflight path. Only .log files are accepted.
  const handleImportLog = useCallback(
    async (srcPath: string) => {
      if (!/\.log$/i.test(srcPath)) {
        toast.error("Only .log files can be added.");
        return;
      }
      setImporting(true);
      try {
        const imported = await invokeOrThrow<string>("uploader_import_log", { srcPath });
        if (logsDir) await loadLogs(logsDir);
        await handleSelectLog(imported);
        toast.success("Log added — scanning it now.");
      } catch (e) {
        toast.error(`Couldn't add that log: ${getTauriErrorMessage(e)}`);
      } finally {
        setImporting(false);
      }
    },
    [logsDir, loadLogs, handleSelectLog]
  );

  // Live mode has exactly one sensible target — the live Encounter.log ESO is writing
  // right now — so there's no real choice to make. Auto-pick it (so "Go Live" works
  // without hunting for a file); the user can still override by clicking a different
  // log first. Returns the selected path, or null if no encounter log is present.
  //
  // CRUCIAL: live streaming is ONLY valid for an ESO *encounter* log. The folder also
  // holds `Interface.log` — the game's UI/error log, which it writes constantly (even
  // in menus), so it is almost always the most-recently-modified file and would win a
  // naive "newest / isActive" pick. But it has no BEGIN_LOG and the native encoder
  // can't anchor a session on it, so it must NEVER be a live target. We therefore
  // restrict to encounter logs and prefer, in order: the active `Encounter.log` (the
  // hot file) → any active encounter log (a just-rotated session that's still hot) →
  // the literal `Encounter.log` even if cold → the newest encounter log. Archives
  // (`Archive-…-Encounter-….log`) are historical and belong in manual upload, but they
  // ARE encounter logs, so they remain a last-resort candidate rather than Interface.log.
  // Called from the Live-tab click and as a Go-Live fallback (NOT from an effect — the
  // React Compiler discourages firing setState from effects; this is a user action).
  const autoSelectActiveLog = useCallback((): string | null => {
    // `fileName` is the bare name (no path). ESO encounter logs contain "encounter"
    // and end in .log; the live file is named exactly `Encounter.log`. Anything else —
    // notably `Interface.log` — is not streamable, so it's excluded outright.
    const isEncounterLog = (name: string) => /encounter.*\.log$/i.test(name);
    const isLiveEncounter = (name: string) => /^encounter\.log$/i.test(name);
    const encounterLogs = logs.filter((l) => isEncounterLog(l.fileName));
    if (encounterLogs.length === 0) return null;

    const target =
      encounterLogs.find((l) => l.isActive && isLiveEncounter(l.fileName)) ??
      encounterLogs.find((l) => l.isActive) ??
      encounterLogs.find((l) => isLiveEncounter(l.fileName)) ??
      [...encounterLogs].sort((a, b) => b.modifiedAtMs - a.modifiedAtMs)[0];

    if (target) {
      void handleSelectLog(target.path);
      return target.path;
    }
    return null;
  }, [logs, handleSelectLog]);

  // Native file drag-drop over the window. Tauri delivers real OS paths (unlike
  // HTML5 drag-drop in a webview), which the backend then copy-confines. We only
  // act on a single dropped .log; the drag-over state drives the picker visual.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;
    void (async () => {
      try {
        const { getCurrentWebview } = await import("@tauri-apps/api/webview");
        unlisten = await getCurrentWebview().onDragDropEvent((event) => {
          if (!active) return;
          const t = event.payload.type;
          if (t === "over") {
            setDragOver(true);
          } else if (t === "leave") {
            setDragOver(false);
          } else if (t === "drop") {
            setDragOver(false);
            const paths = event.payload.paths ?? [];
            const log = paths.find((p) => /\.log$/i.test(p));
            if (log) void handleImportLog(log);
            else if (paths.length > 0) toast.error("Drop a .log file to add it.");
          }
        });
      } catch {
        /* drag-drop is an enhancement; ignore if the webview API is unavailable */
      }
    })();
    return () => {
      active = false;
      unlisten?.();
    };
  }, [handleImportLog]);

  const handleManualUpload = async () => {
    if (!selectedLog) return;
    setUploading(true);
    try {
      // Read the opt-OUT AND session presence fresh per upload. Manual now mirrors
      // live: native is the DEFAULT (opt-out via `manualUseOfficialUploader`,
      // default false → native), but it still needs a captured esologs session —
      // without it the backend would route native and hard-fail "Not signed in", so
      // gate the dispatched flag on the session to keep routing consistent with the
      // UI's willUseNative hint (an opted-in user with no session still hands off).
      const [manualOfficial, hasSession] = await Promise.all([
        getSetting<boolean>("manualUseOfficialUploader", false),
        invokeOrThrow<boolean>("uploader_has_session").catch(() => false),
      ]);
      const nativeOptIn = !manualOfficial && hasSession;
      const dispatch = await invokeOrThrow<UploadDispatch>("uploader_upload_log", {
        filePath: selectedLog,
        options,
        preferCli: transport?.officialUploaderInstalled ?? false,
        // Reuse the preflight's count so the backend doesn't re-scan a multi-GB
        // log just to fill the history record.
        fightCount: preflight?.totalFights ?? null,
        nativeOptIn,
      });
      if (dispatch.report) {
        toast.success("Upload complete — report ready.");
        // Native upload produced a report code; offer to jump straight to the richer
        // ESO Log Aggregator analysis if the user opted into auto-open.
        void maybeAutoOpenAnalysis(dispatch.report.code);
      } else {
        toast.success(dispatch.detail, { duration: 7000 });
      }
      await refreshHistory();
    } catch (e) {
      toast.error(`Upload failed: ${getTauriErrorMessage(e)}`);
    } finally {
      setUploading(false);
    }
  };

  // Opening the split workbench: the rich modal authors the per-session plan and
  // performs the named split itself (uploader_split_to_disk_named). Requires the
  // preflight to be loaded so the workbench has sessions to show.
  const handleSplit = () => {
    if (!selectedLog || !preflight) return;
    setWorkbenchOpen(true);
  };

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
    // Record the id before the await so unmount cleanup can stop the backend
    // watcher even if the dialog closes before the await resolves.
    liveSessionIdRef.current = sessionId;
    const channel = new Channel<LiveEvent>();
    setLiveFights([]);
    setLiveFightCount(0);
    liveFightCountRef.current = 0;
    seenFightIndicesRef.current = new Set();
    setLiveReport(null);
    setSessionAnchored(false); // native: not anchored until the first BEGIN_LOG
    // Reset the handoff path indicator (state + its unmount-toast ref mirror) so this
    // new session doesn't inherit the previous session's path for its copy/toast until
    // the start dispatch resolves the real value.
    setLiveHandedOff(false);
    liveHandedOffRef.current = false;
    setLiveStartMs(startedAt);
    setLiveStatus("watching");
    liveActiveRef.current = true;

    // The watcher emits UI-only fight-detection events; the actual upload is the
    // single whole-file handoff performed by uploader_start_live below.
    channel.onmessage = (ev) => {
      // Drop events that don't belong to the CURRENT session. The global
      // liveActiveRef alone is not enough: a previous session's queued event
      // (e.g. the watcher's trailing `Stopped`, delivery to React lagging) can
      // arrive after a NEW session already set liveActiveRef=true and a new
      // liveSessionIdRef. Without this per-session check, that stale event would
      // contaminate the new session — clearing its timeline or, in the `stopped`
      // arm, calling uploader_stop_live for the new id. This closure captures its
      // own `sessionId`, so gate on it (which also covers the stopped/closed case
      // the liveActiveRef check used to handle, since the ref is nulled on stop).
      if (!liveActiveRef.current || liveSessionIdRef.current !== sessionId) return;
      switch (ev.type) {
        case "started":
          setLiveStatus("watching");
          break;
        case "reportOpened":
          // Native live only: the report now has a code (create-report returned),
          // before any fight has posted. Surface it immediately so the user can open
          // the live analysis in the ESO Log Aggregator while the raid is streaming —
          // previously the live code only appeared after the session settled.
          setLiveReport({ code: ev.code, url: ev.url });
          void maybeAutoOpenAnalysis(ev.code, { live: true });
          break;
        case "sessionAnchored":
          // Native: the first BEGIN_LOG landed — flip waiting→streaming instantly.
          setSessionAnchored(true);
          break;
        case "fightDetected": {
          // A fight implies the session anchored, even if the anchored event was
          // missed/coalesced — keep the UI honest.
          setSessionAnchored(true);
          const detected = ev;
          // Dedup + count in the EVENT-HANDLER body (runs once per event), NOT inside a
          // setState updater. React StrictMode double-invokes updaters in dev, so the old
          // nested `setLiveFightCount` inside the `setLiveFights` updater fired twice and
          // double-counted every fight ("2 fights" for 1). The ref Set dedups re-delivered
          // events; both setStates below use PURE updaters (StrictMode-safe).
          if (seenFightIndicesRef.current.has(detected.index)) break;
          seenFightIndicesRef.current.add(detected.index);
          liveFightCountRef.current += 1;
          setLiveFightCount((c) => c + 1);
          setLiveFights((prev) => {
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
          toast.info("A new logging session started — continuing to watch.");
          setLiveFights([]);
          setLiveFightCount(0);
          liveFightCountRef.current = 0;
          seenFightIndicesRef.current = new Set();
          break;
        case "fightSkipped":
          // A genuinely oversized fight; surface once. The full log still uploads.
          toast.info(ev.reason);
          break;
        case "warning":
          // Transient (e.g. a read retry) — log but don't toast, as these recur.
          console.warn("[uploader] live watcher:", ev.message);
          break;
        case "reauthRequired":
          // Native live only: the ESO Logs session expired mid-stream. Posting is
          // paused (the report stays open) until the user re-signs-in. Prompt them;
          // the driver resumes automatically once a fresh session is stored.
          setLiveStatus("attention");
          toast.warning(ev.message, { duration: 12000 });
          break;
        case "reauthResolved":
          // A fresh session was captured; the driver resumed posting.
          setLiveStatus("watching");
          toast.success("Signed back in — resuming the live upload.");
          break;
        case "stopped": {
          // A `stopped` event that passes the session guard above means THIS
          // session's watcher thread died on its own (lost folder access,
          // couldn't keep reading the log, etc.) — a user-initiated stop already
          // cleared liveActiveRef/the ref before this could run. Beyond tearing
          // down the UI, the backend still holds the now-dead `Running` slot and
          // the history record is still `Live`, and nothing else settles them
          // until the next-launch reconcile. Drive the existing stop path so the
          // slot is evicted and the record settled immediately. Use this
          // closure's own `sessionId` (the guard proved it is the current one) so
          // we never settle some other session's id.
          const stoppedFightCount = liveFightCountRef.current;
          liveActiveRef.current = false;
          liveSessionIdRef.current = null;
          liveWasRunningRef.current = false; // settled; don't re-warn on close
          setLiveSessionId(null);
          setLiveStatus("attention");
          if (ev.reason && !/stopped\.?$/i.test(ev.reason)) toast.error(ev.reason);
          // Best-effort: evicts the dead `Running` slot (stop_slot_in_map) and
          // settles the `Live` record to `Completed` (settle_live). Both are
          // idempotent, so this is safe even if the record was already settled.
          void invokeOrThrow("uploader_stop_live", {
            sessionId,
            fightCount: stoppedFightCount,
          }).catch(() => {});
          break;
        }
      }
    };

    // Native direct-streaming is the DEFAULT live path: it's faster and keeps the
    // report in-app. The only ways off it are (a) `forceHandoff` — the explicit "go
    // live via the official uploader instead" escape hatch — or (b) the persisted
    // `liveUseOfficialUploader` opt-OUT (default false → native). We use a dedicated
    // key (not the legacy `nativeUploadOptIn`, whose default-false meant "native off")
    // so existing users' stored values aren't silently inverted: an unset key resolves
    // to false = native, which is the new default for everyone.
    const preferOfficial =
      forceHandoff || (await getSetting<boolean>("liveUseOfficialUploader", false));

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
      setLiveStatus("attention");
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
      setLiveStatus("watching");
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
      if (dispatch?.report) setLiveReport(dispatch.report);
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
      setLiveStatus("attention");
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
    liveWasRunningRef.current = false;

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
        fightCount: liveFightCountRef.current,
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
      setLiveStatus(liveFightCount > 0 ? "upToDate" : "idle");
    }
    if (wasRunning) {
      // Path-aware: on the HANDOFF path Kalpa can't stop the separate official
      // uploader (it may still be streaming); on the NATIVE path Kalpa IS the uploader,
      // so Stop genuinely ended the upload and closed the report.
      toast.info(
        liveHandedOff
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

  const copyLink = useCallback(async (url: string) => {
    // Await the write and gate the toast on success: a rejected clipboard write
    // (permission/focus/policy) must not show a false "copied" or leak an
    // unhandled rejection. Matches the await-in-try/catch convention elsewhere.
    try {
      await navigator.clipboard.writeText(url);
      toast.success("Report link copied.");
    } catch {
      toast.error("Couldn't copy the link — copy it manually.");
    }
  }, []);

  const handleAttachReport = useCallback(
    async (id: string, reportUrl: string) => {
      try {
        await invokeOrThrow("uploader_attach_report", { id, reportUrl });
        toast.success("Report link saved.");
        await refreshHistory();
      } catch (e) {
        toast.error(getTauriErrorMessage(e));
      }
    },
    [refreshHistory]
  );

  // Remove an upload record from history. The log FILE on disk is untouched —
  // this only clears the record (the backend command already existed but had no
  // UI to reach it).
  const handleDeleteHistory = useCallback(
    async (id: string) => {
      try {
        await invokeOrThrow("uploader_delete_history", { id });
        await refreshHistory();
        toast.success("Removed from history.");
      } catch (e) {
        toast.error(getTauriErrorMessage(e));
      }
    },
    [refreshHistory]
  );

  // The headline status pill reflects live state if a session is running,
  // otherwise manual upload state.
  const headlineStatus: UploaderStatus = useMemo(() => {
    if (liveSessionId) return liveStatus;
    if (uploading) return "uploading";
    return "idle";
  }, [liveSessionId, liveStatus, uploading]);

  const isLoggedIn = authUser !== null;

  // Initial dialog focus: land on the first meaningful control (the Manual mode
  // tab) rather than the close button, so keyboard users start in the flow. When
  // logged out this ref is null and the dialog falls back to default focus (the
  // sign-in button).
  const firstTabRef = useRef<HTMLButtonElement>(null);

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      {/* Cap height to the viewport and lay the dialog out as a flex column so the
          header stays pinned while the body scrolls. Without this the shared
          DialogContent (overflow-hidden, no max-height, vertically centered)
          lets tall content spill off the top and bottom of the screen with no
          way to reach it. */}
      <DialogContent
        initialFocus={isLoggedIn ? firstTabRef : undefined}
        className="flex max-h-[90vh] flex-col gap-0 overflow-hidden sm:max-w-3xl"
      >
        <DialogHeader className="shrink-0">
          <div className="flex items-center justify-between gap-3 pr-7">
            <DialogTitle className="flex items-center gap-2">
              <CloudUpload className="size-5 text-primary" aria-hidden />
              Upload to ESO Logs
            </DialogTitle>
            <StatusPill status={headlineStatus} />
          </div>
          <DialogDescription>
            Turn your <code className="text-foreground/80">Encounter.log</code> into a shareable
            report on esologs.com — parses, rankings, and full fight breakdowns.
          </DialogDescription>
          {isLoggedIn && (
            <TransportReadout willUseNative={activeWillUseNative} transport={transport} />
          )}
        </DialogHeader>

        {!isLoggedIn ? (
          <LoggedOut onAuthChange={onAuthChange} />
        ) : (
          // The body is a DARKER inset canvas than the dialog chrome, so the
          // raised content surfaces below it actually read as raised (you can't
          // elevate from a surface the same color as everything else). The
          // negative margins bleed it to the dialog edges; a top inset shadow
          // sells the "sunken work surface" depth.
          <div className="-mx-5 -mb-5 flex-1 overflow-y-auto bg-[var(--bg-base)] px-5 pt-4 pb-5 shadow-[inset_0_8px_16px_-8px_rgba(0,0,0,0.6)]">
            <div className="space-y-3.5">
              <WhatGetsUploaded />

              {/* Mode tabs — a segmented control sitting in a recessed track, so
                  the active tab reads as raised out of the well, not as two equal
                  panels. */}
              <div className="grid grid-cols-2 gap-1.5 rounded-2xl border border-black/40 bg-black/25 p-1.5 shadow-[inset_0_2px_8px_-2px_rgba(0,0,0,0.6)]">
                <ModeTab
                  buttonRef={firstTabRef}
                  active={mode === "manual"}
                  onClick={() => {
                    // Leaving Live unmounts its only Stop control, so stop the
                    // session first rather than orphaning the watcher. Check the
                    // REF (not `liveSessionId` state): a session that is still
                    // starting has its id in the ref before state lands, and
                    // handleStopLive now keys off the ref too, so this also cancels
                    // an in-flight start.
                    if (liveSessionIdRef.current) void handleStopLive();
                    setMode("manual");
                  }}
                  Icon={Upload}
                  title="Upload a Log"
                  hint="Send a finished log after your session."
                />
                <ModeTab
                  active={mode === "live"}
                  onClick={() => {
                    setMode("live");
                    // Entering Live re-targets the ACTIVE Encounter.log, overriding any
                    // selection carried over from Manual (e.g. an archive you were
                    // about to upload) — live has exactly one correct target, and a
                    // stale manual pick would silently make Go Live stream the wrong
                    // (or a dead) file. The user can still click a different log AFTER
                    // switching to Live to override. Guard on the REFS (not the
                    // `liveSessionId` state, which lags) so this can't clobber the
                    // selection of an in-flight start.
                    if (!liveSessionIdRef.current && !startingRef.current) autoSelectActiveLog();
                  }}
                  Icon={Radio}
                  title="Live Log"
                  hint="Stream fights during an ongoing raid."
                />
              </div>

              {/* Log picker */}
              <LogPicker
                detection={detection}
                logsDir={logsDir}
                logs={logs}
                listError={listError}
                selectedLog={selectedLog}
                scanning={scanning}
                dragOver={dragOver}
                importing={importing}
                onSelect={handleSelectLog}
                onRefresh={() => logsDir && loadLogs(logsDir)}
                onPickFolder={handlePickFolder}
                onOpenFolder={handleOpenLogsFolder}
                onReveal={handleRevealLog}
                onCopyPath={handleCopyPath}
                onRequestDelete={setDeleteTarget}
              />

              {/* Selected-log summary: the confident "here's what you're uploading"
                moment before the action. */}
              {selectedLog && preflight && !scanning && (
                <LogSummaryCard
                  fileName={selectedLog.split(/[/\\]/).pop() ?? selectedLog}
                  preflight={preflight}
                  fights={fights}
                  willUseNative={activeWillUseNative}
                />
              )}

              {/* Preflight + fights */}
              {selectedLog && (
                <Preflight
                  preflight={preflight}
                  scanning={scanning}
                  scanningSizeBytes={logs.find((l) => l.path === selectedLog)?.sizeBytes ?? null}
                  onSplit={handleSplit}
                />
              )}

              {selectedLog &&
                (mode === "live" ? null : (
                  <div className={cn(WORK_PANEL, "p-3.5")}>
                    <SectionHeader className="mb-2">Fights</SectionHeader>
                    <FightList
                      fights={rowsFromSummaries(fights)}
                      emptyHint={
                        scanning ? "Scanning the log…" : "No fights found in this log yet."
                      }
                    />
                  </div>
                ))}

              {/* Upload options */}
              {selectedLog && (
                <div className={cn(WORK_PANEL, "p-4")}>
                  <UploadOptionsControl
                    options={options}
                    onChange={setOptions}
                    disabled={uploading || liveSessionId !== null}
                    willUseNative={activeWillUseNative}
                    fights={fights}
                    whenMs={logs.find((l) => l.path === selectedLog)?.modifiedAtMs ?? null}
                  />
                  {mode === "live" && (
                    <LiveToggles
                      options={options}
                      onChange={setOptions}
                      disabled={liveSessionId !== null}
                    />
                  )}
                </div>
              )}

              {/* Direct upload (recommended) — opt-in + in-app sign-in. Placed just
                before the action so the user sets up the faster path right where it
                pays off. Shown in BOTH modes (when no live session is running): live
                now also goes native when opted-in + signed-in, so this is where a
                user discovers WHY a live session would otherwise hand off (not opted
                in / not signed in) and fixes it before Go Live — the gap that made a
                "why did the official uploader open?" surprise. */}
              {liveSessionId === null && (
                <DirectUploadSection
                  // This section represents the UNIFIED direct-upload state (it's shown
                  // in both modes), so "opted in" requires native for BOTH manual and
                  // live — i.e. neither opt-out key set. A migrated user with only the
                  // live opt-out (manualUseOfficialUploader=false, liveUseOfficialUploader
                  // =true) must NOT see "ready" while Go Live hands off; they see Enable,
                  // which clears BOTH keys. Manual-only routing still uses `nativeOptIn`.
                  optIn={nativeOptIn && !liveUseOfficial}
                  hasSession={hasNativeSession}
                  onChanged={refreshNativeState}
                />
              )}

              {/* Action area. Keyed on `mode` so switching cross-fades the panel
                (content opacity only — never the glass blur). The mode-switch
                handler already stops a live session before setMode, so this
                remount never bypasses the watcher teardown. */}
              <div key={mode} className="animate-[fade-in_0.2s_ease-out]">
                {mode === "manual" ? (
                  <ManualActions
                    canUpload={
                      !!selectedLog &&
                      !uploading &&
                      !scanning &&
                      preflight !== null &&
                      liveSessionId === null
                    }
                    uploading={uploading}
                    transport={transport}
                    willUseNative={willUseNative}
                    onUpload={handleManualUpload}
                  />
                ) : (
                  <LiveDashboard
                    running={liveSessionId !== null}
                    starting={starting}
                    // Always enabled (the listError empty-state aside, handled by the
                    // panel): handleStartLive resolves the active Encounter.log itself
                    // and surfaces an honest toast when the folder is empty — so gating
                    // on a selection or logs.length>0 made both the auto-select fallback
                    // AND the empty-folder toast unreachable from the button.
                    canStart={true}
                    startMs={liveStartMs}
                    liveFights={liveFights}
                    liveFightCount={liveFightCount}
                    liveReport={liveReport}
                    // Fights already in the selected log before going live. Live
                    // streams only NEW fights (tail starts at EOF), so this drives the
                    // "earlier fights won't be uploaded" expectation note.
                    priorFightCount={preflight?.totalFights ?? 0}
                    // Which path the running session took — drives the callout/copy
                    // (handoff = a separate uploader app; native = Kalpa uploads).
                    handedOff={liveHandedOff}
                    // Native waiting↔streaming: anchored once the first BEGIN_LOG lands.
                    sessionAnchored={sessionAnchored}
                    // Best-effort pre-start guess of what's coming (which waiting copy).
                    readiness={liveReadiness}
                    // Wrap so the click PointerEvent is NOT passed as the first arg
                    // (`forceHandoff`): a bare `onStart={handleStartLive}` made every
                    // Go Live receive the event as a truthy forceHandoff → preferOfficial
                    // → silent handoff to the official uploader. This is THE "it still
                    // opened the other uploader" bug.
                    onStart={() => void handleStartLive()}
                    onStop={handleStopLive}
                    onCopyLink={copyLink}
                    onForceHandoff={handleForceHandoffLive}
                  />
                )}
              </div>

              {/* History */}
              <HistoryPanel
                history={history}
                onCopyLink={copyLink}
                onRefresh={refreshHistory}
                onAttachReport={handleAttachReport}
                onDelete={handleDeleteHistory}
              />
            </div>
          </div>
        )}
      </DialogContent>

      {/* The split workbench overlays as its own modal when invoked from the
          preflight's Split control. Rendered here so it shares the uploader's
          lifetime but layers above the main dialog. */}
      {selectedLog && (
        // key on the selected log so switching logs REMOUNTS the workbench,
        // resetting its per-session drafts (include/name) — otherwise a new log
        // with the same session indices would inherit the previous log's choices.
        <SplitWorkbench
          key={selectedLog}
          open={workbenchOpen}
          onOpenChange={setWorkbenchOpen}
          filePath={selectedLog}
          fileName={selectedLog.split(/[/\\]/).pop() ?? selectedLog}
          preflight={preflight}
        />
      )}

      {/* Delete confirmation — soft delete to the recycle bin (recoverable). */}
      <DeleteLogConfirm
        target={deleteTarget}
        deleting={deleting}
        onCancel={() => setDeleteTarget(null)}
        onConfirm={handleConfirmDelete}
      />
    </Dialog>
  );
}

// Confirmation for deleting a log. Soft delete: the file moves to Kalpa's recycle
// bin (kept 30 days) and can be restored — combat logs are irreplaceable, so this
// is never a hard unlink. The dialog stays cool glass; only the file inset and the
// confirm button carry red, per the design system's restraint on danger color.
function DeleteLogConfirm({
  target,
  deleting,
  onCancel,
  onConfirm,
}: {
  target: LogFileInfo | null;
  deleting: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Dialog open={target !== null} onOpenChange={(o) => !o && onCancel()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Delete this log?</DialogTitle>
          <DialogDescription>
            It moves to Kalpa's recycle bin and is removed from your Logs folder. You can restore it
            for 30 days.
          </DialogDescription>
        </DialogHeader>
        {target && (
          <div className="mt-3 rounded-lg border border-red-500/15 border-l-[3px] border-l-red-500 bg-red-500/[0.05] p-3">
            <div className="truncate font-mono text-sm text-foreground/90" title={target.fileName}>
              {target.fileName}
            </div>
            <div className="mt-0.5 text-xs text-muted-foreground">
              {compactBytes(target.sizeBytes)} · {relativeFromMs(target.modifiedAtMs)}
            </div>
          </div>
        )}
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="ghost" onClick={onCancel} disabled={deleting}>
            Cancel
          </Button>
          <Button variant="destructive" onClick={onConfirm} disabled={deleting}>
            <Trash2 className="size-4" />
            {deleting ? "Deleting…" : "Delete log"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

// ── Sub-components ───────────────────────────────────────────────────────────

// A compact "route" instrument that dramatizes the data path the upload takes:
// your machine → the active engine → esologs.com. The middle chip reflects which
// transport will run (direct = sky/Zap, official = muted), so the user always
// knows the route at a glance from the header — the signature element of the
// workspace, grounded in the subject (a log's journey to the site).
function TransportReadout({
  willUseNative,
  transport,
}: {
  willUseNative: boolean;
  transport: TransportInfo | null;
}) {
  const installed = transport?.officialUploaderInstalled ?? false;
  return (
    <div className="mt-2.5 flex items-center gap-2 text-[11px]">
      <span className="inline-flex items-center gap-1.5 rounded-md border border-white/[0.08] bg-white/[0.03] px-2 py-1 font-medium text-foreground/80">
        <FileText className="size-3 text-muted-foreground" aria-hidden />
        Your log
      </span>
      <ChevronRight className="size-3 shrink-0 text-muted-foreground/50" aria-hidden />
      {willUseNative ? (
        <span className="inline-flex items-center gap-1.5 rounded-md border border-accent-sky/25 bg-accent-sky/[0.06] px-2 py-1 font-medium text-accent-sky">
          <Zap className="size-3" aria-hidden />
          Direct from Kalpa
        </span>
      ) : (
        <span className="inline-flex items-center gap-1.5 rounded-md border border-white/[0.08] bg-white/[0.03] px-2 py-1 font-medium text-muted-foreground">
          <CloudUpload className="size-3" aria-hidden />
          {installed ? "Official uploader" : "ESO Logs uploader"}
        </span>
      )}
      <ChevronRight className="size-3 shrink-0 text-muted-foreground/50" aria-hidden />
      <span className="inline-flex items-center gap-1.5 rounded-md border border-primary/25 bg-primary/[0.06] px-2 py-1 font-medium text-primary">
        esologs.com
      </span>
    </div>
  );
}

// The confident "here's what you're uploading" card shown once a log is scanned.
// Leads with the content (dominant zone) so the user recognizes the night at a
// glance, then the hard facts (fights / sessions / size) and the route it takes.
function LogSummaryCard({
  fileName,
  preflight,
  fights,
  willUseNative,
}: {
  fileName: string;
  preflight: LogPreflight;
  fights: FightSummary[];
  willUseNative: boolean;
}) {
  const zone = dominantZone(fights);
  const bosses = Array.from(
    new Set(fights.map((f) => f.bossName).filter((b): b is string => !!b))
  ).slice(0, 3);
  const sessions = preflight.sessions.length;

  return (
    <GlassPanel
      variant="primary"
      className="overflow-hidden border-emerald-400/15 bg-gradient-to-b from-emerald-400/[0.05] to-white/[0.01] p-4 shadow-[0_12px_36px_-14px_rgba(0,0,0,0.7),inset_0_1px_0_rgba(255,255,255,0.06)]"
    >
      <div className="mb-2.5 flex items-center gap-1.5">
        <CheckCircle2 className="size-3.5 text-emerald-400" aria-hidden />
        <span className="font-heading text-[11px] font-semibold tracking-[0.08em] text-emerald-300/90 uppercase">
          Ready to upload
        </span>
      </div>
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-primary/12 text-primary">
              <Swords className="size-4" aria-hidden />
            </span>
            <div className="min-w-0">
              <div className="truncate text-base font-semibold text-foreground/95">
                {zone ?? "Combat log"}
              </div>
              <div
                className="truncate font-mono text-[11px] text-muted-foreground"
                title={fileName}
              >
                {fileName}
              </div>
            </div>
          </div>
          {bosses.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-1">
              {bosses.map((b) => (
                <InfoPill key={b} color="muted" className="text-[11px]">
                  {b}
                </InfoPill>
              ))}
              {fights.length > bosses.length && (
                <span className="self-center text-[11px] text-muted-foreground/70">
                  +{fights.length - bosses.length} more
                </span>
              )}
            </div>
          )}
        </div>
        {/* Transport chip — the engine this upload will use. */}
        {willUseNative ? (
          <InfoPill color="sky" className="shrink-0 gap-1">
            <Zap className="size-3" aria-hidden /> Direct
          </InfoPill>
        ) : (
          <InfoPill color="muted" className="shrink-0 gap-1">
            <CloudUpload className="size-3" aria-hidden /> Official
          </InfoPill>
        )}
      </div>

      {/* Hard facts row. */}
      <div className="mt-3 grid grid-cols-3 gap-2">
        <SummaryStat
          value={preflight.totalFights}
          label={preflight.totalFights === 1 ? "fight" : "fights"}
        />
        <SummaryStat value={sessions} label={sessions === 1 ? "session" : "sessions"} />
        <SummaryStat value={compactBytes(preflight.sizeBytes)} label="on disk" />
      </div>
    </GlassPanel>
  );
}

function SummaryStat({ value, label }: { value: string | number; label: string }) {
  return (
    <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2 text-center">
      <div className="font-heading text-lg leading-tight font-semibold text-foreground/90 tabular-nums">
        {value}
      </div>
      <div className="text-[11px] text-muted-foreground">{label}</div>
    </div>
  );
}

function LoggedOut({ onAuthChange }: { onAuthChange: (user: AuthUser | null) => void }) {
  const [loggingIn, setLoggingIn] = useState(false);

  // Inline sign-in, matching the Pack Create / My Packs pattern. Previously this
  // pointed the user to Settings, which has no sign-in control — a dead end. The
  // uploader is the most common first entry point, so sign in right here.
  const handleLogin = async () => {
    setLoggingIn(true);
    try {
      const user = await invokeOrThrow<AuthUser>("auth_login");
      onAuthChange(user);
      toast.success(`Signed in as ${user.userName}`);
      warnIfSessionNotPersisted(user);
    } catch (e) {
      toast.error(`Sign in failed: ${getTauriErrorMessage(e)}`);
    } finally {
      setLoggingIn(false);
    }
  };

  return (
    // A contained "gateway" card so signing in reads as a distinct first step,
    // not floating text. Sky-accented (an account/connection action) to set it
    // apart from the gold Upload climax that comes later.
    <div className="mt-4">
      <GlassPanel
        variant="primary"
        className="flex flex-col items-center gap-4 border-accent-sky/15 bg-gradient-to-b from-accent-sky/[0.05] to-white/[0.01] px-6 py-8 text-center"
      >
        <div className="flex size-14 items-center justify-center rounded-2xl border border-accent-sky/20 bg-accent-sky/[0.1] text-accent-sky shadow-[0_0_28px_-8px_color-mix(in_oklab,var(--accent-sky)_50%,transparent)]">
          <LogIn className="size-7" aria-hidden />
        </div>
        <div>
          <div className="font-heading text-lg font-semibold text-foreground/95">
            Connect your ESO Logs account
          </div>
          <p className="mx-auto mt-1.5 max-w-sm text-sm text-muted-foreground">
            Sign in to upload your combat logs and get your reports. It's the same account Kalpa
            uses for Pack Hub — no extra password needed.
          </p>
        </div>
        {/* Outline (not gold): a connect action, deliberately distinct from the
            gold "Upload to ESO Logs" primary action that appears once signed in. */}
        <Button
          variant="outline"
          size="lg"
          onClick={handleLogin}
          disabled={loggingIn}
          className="border-accent-sky/30 bg-accent-sky/[0.06] text-accent-sky hover:border-accent-sky/50 hover:bg-accent-sky/[0.12]"
        >
          <LogIn className="size-4" />
          {loggingIn ? "Opening sign-in…" : "Sign in to ESO Logs"}
        </Button>
      </GlassPanel>
    </div>
  );
}

// Promoted "Direct upload (recommended)" section. Folds the old standalone
// native-session sign-in into one place that also drives discovery of the
// faster in-app path, and shows three states:
//   • opt-in OFF → a benefit-led promo with an inline "Enable" that opens the
//     same honest disclosure as Settings (2 clicks, no detour to Settings);
//   • opt-in ON, no session → the in-app esologs sign-in (relabelled so it's
//     clearly the SAME account, and clearly optional);
//   • opt-in ON, signed in → a calm "Ready" state with a quiet Sign out.
// `onChanged` re-reads the lifted opt-in/session state in the parent so the
// upload action's transport hint stays in sync. The setting key is the exact
// one Settings writes, so the two stay consistent; the backend coverage gate
// remains the final authority over which transport actually runs per log.
function DirectUploadSection({
  optIn,
  hasSession,
  onChanged,
}: {
  optIn: boolean;
  hasSession: boolean;
  onChanged: () => void | Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [disclosureOpen, setDisclosureOpen] = useState(false);

  const handleEnable = async () => {
    // Clear BOTH opt-OUT keys (default is native now); this state only shows when the
    // user previously turned direct upload OFF. The Settings toggle writes both manual
    // AND live keys as one unified "use the official uploader" control, so the inline
    // Enable must clear both too — otherwise enabling here re-enables manual direct
    // upload while LIVE silently keeps handing off to the official uploader.
    await Promise.all([
      setSetting("manualUseOfficialUploader", false),
      setSetting("liveUseOfficialUploader", false),
    ]);
    setDisclosureOpen(false);
    toast.success("Direct upload enabled.");
    await onChanged();
  };

  const handleSignIn = async () => {
    setBusy(true);
    try {
      const result = await invokeOrThrow<{ sessionPersisted?: boolean }>("uploader_login_esologs");
      toast.success("Direct upload ready — your logs now go straight from Kalpa.");
      warnIfSessionNotPersisted(result);
      await onChanged();
    } catch (e) {
      toast.error(`Direct-upload sign-in failed: ${getTauriErrorMessage(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const handleSignOut = async () => {
    try {
      await invokeOrThrow("uploader_logout_esologs");
      await onChanged();
    } catch (e) {
      toast.error(`Sign out failed: ${getTauriErrorMessage(e)}`);
    }
  };

  // State 1 — not opted in: promote the faster path. Sky accent (interactive),
  // never red — this is reversible and the official uploader is the safe default.
  if (!optIn) {
    return (
      <>
        <GlassPanel
          variant="subtle"
          className="flex items-center justify-between gap-3 border-accent-sky/20 bg-accent-sky/[0.03] p-3"
        >
          <div className="flex min-w-0 items-start gap-2.5">
            <Zap className="mt-0.5 size-4 shrink-0 text-accent-sky" aria-hidden />
            <div className="min-w-0">
              <p className="text-sm font-medium text-white/90">Upload faster, in-app</p>
              <p className="text-xs text-muted-foreground">
                Send logs straight from Kalpa and see the report here — no second window. Unofficial
                method; falls back to the official uploader automatically.
              </p>
            </div>
          </div>
          <Button size="sm" className="shrink-0" onClick={() => setDisclosureOpen(true)}>
            Enable
          </Button>
        </GlassPanel>
        <DirectUploadDisclosure
          open={disclosureOpen}
          onOpenChange={setDisclosureOpen}
          onAccept={handleEnable}
        />
      </>
    );
  }

  // State 3 — opted in and signed in: ready. Slimmed to a single confirmation
  // line (the header route readout already shows "Direct from Kalpa"), so it
  // doesn't add a second heavy panel above the action.
  if (hasSession) {
    return (
      <div className="flex items-center justify-between gap-3 rounded-lg border border-emerald-400/20 bg-gradient-to-b from-emerald-400/[0.06] to-emerald-400/[0.02] px-3 py-1.5 shadow-[inset_0_1px_0_rgba(255,255,255,0.04)]">
        <div className="flex min-w-0 items-center gap-2 text-xs">
          <span className="flex size-4 shrink-0 items-center justify-center rounded-full bg-emerald-400/15">
            <Check className="size-2.5 text-emerald-400" aria-hidden />
          </span>
          <span className="text-foreground/80">Direct upload ready</span>
          <span className="truncate text-muted-foreground">— reports appear here</span>
        </div>
        <Button variant="ghost" size="sm" className="shrink-0" onClick={handleSignOut}>
          Sign out
        </Button>
      </div>
    );
  }

  // State 2 — opted in, needs the in-app esologs session.
  return (
    <GlassPanel variant="subtle" className="flex items-center justify-between gap-3 p-3">
      <div className="min-w-0">
        <p className="text-sm font-medium text-white/90">Finish enabling direct upload</p>
        <p className="text-xs text-muted-foreground">
          Sign in to ESO Logs once inside Kalpa — same account as above. This is optional; it just
          enables the faster in-app path.
        </p>
      </div>
      <Button
        variant="outline"
        size="sm"
        className="shrink-0"
        onClick={handleSignIn}
        disabled={busy}
      >
        <LogIn className="size-3.5" />
        {busy ? "Opening…" : "Sign in"}
      </Button>
    </GlassPanel>
  );
}

// Inline, honest disclosure shown before enabling direct (native) upload — the
// same plain-language framing as the Settings disclosure, lifted here so the
// user can opt in from the uploader without a detour. Reversible, so it uses a
// neutral tone and the official uploader stays the always-available fallback.
function DirectUploadDisclosure({
  open,
  onOpenChange,
  onAccept,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAccept: () => void | Promise<void>;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Enable direct upload?</DialogTitle>
        </DialogHeader>
        <div className="space-y-3 text-sm text-muted-foreground">
          <p>
            Direct upload sends your logs to ESO Logs straight from Kalpa instead of opening the
            official ESO Logs uploader. It's faster and keeps everything in one window — the report
            link appears right here.
          </p>
          <p>
            It works by talking to ESO Logs' uploader endpoints directly — an{" "}
            <span className="font-medium text-white/90">unofficial method</span>. The ESO Logs
            operator has said this is fine, but it isn't an officially supported integration, so it
            could stop working if ESO Logs changes how their uploader works.
          </p>
          <p>
            Kalpa only uses direct upload for logs it can encode with full accuracy; anything else
            falls back to the official uploader automatically, so a report is never uploaded
            incorrectly. You can turn this off any time in Settings.
          </p>
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={() => void onAccept()}>Enable direct upload</Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function ModeTab({
  active,
  onClick,
  Icon,
  title,
  hint,
  buttonRef,
}: {
  active: boolean;
  onClick: () => void;
  Icon: typeof Upload;
  title: string;
  hint: string;
  buttonRef?: Ref<HTMLButtonElement>;
}) {
  return (
    <button
      ref={buttonRef}
      type="button"
      onClick={onClick}
      aria-pressed={active}
      aria-label={`${title} mode — ${hint}`}
      className={cn(
        "group relative overflow-hidden rounded-xl p-3 text-left transition-all duration-200",
        "focus-visible:ring-2 focus-visible:ring-accent-sky/40 focus-visible:outline-none",
        active
          ? // RAISED out of the well. The lift is built from layered NEUTRAL
            // shadows (matching the design system's dark-ambient idiom), not a
            // single saturated blue drop — a tight contact shadow + a soft
            // ambient shadow give real depth, a hairline ring defines the raised
            // edge against the dark track, the sky tint is a faint accent glow
            // (not the lift), and an inset top highlight catches the light. The
            // sky accents are tokenized (accent-sky) so the active tab follows
            // the theme.
            "bg-gradient-to-b from-accent-sky/[0.14] to-accent-sky/[0.05] ring-1 ring-inset ring-accent-sky/25 shadow-[0_1px_2px_rgba(0,0,0,0.5),0_8px_20px_-8px_rgba(0,0,0,0.55),0_0_22px_-12px_color-mix(in_oklab,var(--accent-sky)_55%,transparent),inset_0_1px_0_rgba(255,255,255,0.14)]"
          : // FLAT in the well: no fill, no border — just sits in the recess.
            "text-muted-foreground hover:bg-white/[0.04]"
      )}
    >
      <div
        className={cn(
          "flex items-center gap-2 text-sm font-semibold",
          active ? "text-accent-sky" : "text-foreground/70"
        )}
      >
        <Icon
          className={cn("size-4 shrink-0", active ? "text-accent-sky" : "text-muted-foreground")}
          aria-hidden
        />
        {title}
      </div>
      <div
        className={cn("mt-1 text-xs", active ? "text-accent-sky/60" : "text-muted-foreground/70")}
      >
        {hint}
      </div>
    </button>
  );
}

type LogFilter = "all" | "active" | "archives";
type LogSort = "newest" | "largest";

function LogPicker({
  detection,
  logsDir,
  logs,
  listError,
  selectedLog,
  scanning,
  dragOver,
  importing,
  onSelect,
  onRefresh,
  onPickFolder,
  onOpenFolder,
  onReveal,
  onCopyPath,
  onRequestDelete,
}: {
  detection: LogPathDetection | null;
  logsDir: string | null;
  logs: LogFileInfo[];
  listError: string | null;
  selectedLog: string | null;
  scanning: boolean;
  dragOver: boolean;
  importing: boolean;
  onSelect: (path: string) => void;
  onRefresh: () => void;
  onPickFolder: () => void;
  onOpenFolder: () => void;
  onReveal: (path: string) => void;
  onCopyPath: (path: string) => void;
  onRequestDelete: (log: LogFileInfo) => void;
}) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<LogFilter>("all");
  const [sort, setSort] = useState<LogSort>("newest");

  // Only show the controls once the folder has enough logs to be worth filtering.
  const showControls = logs.length > 1;
  const totalBytes = logs.reduce((sum, l) => sum + l.sizeBytes, 0);

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    let out = logs.filter((l) => {
      if (q && !l.fileName.toLowerCase().includes(q)) return false;
      if (filter === "active") return l.isActive;
      if (filter === "archives") return /^archive/i.test(l.fileName);
      return true;
    });
    out = [...out].sort((a, b) =>
      sort === "largest" ? b.sizeBytes - a.sizeBytes : b.modifiedAtMs - a.modifiedAtMs
    );
    return out;
  }, [logs, query, filter, sort]);

  return (
    // The picker is the primary work surface, so it RISES off the dark canvas:
    // a lighter fill, a luminous top edge (inset highlight), and a real outer
    // shadow. This is the one place the eye should land first.
    <div
      className={cn(
        "relative rounded-2xl border border-white/[0.1] bg-gradient-to-b from-white/[0.07] to-white/[0.025] p-3.5 transition-colors duration-150",
        "shadow-[0_12px_40px_-16px_rgba(0,0,0,0.7),inset_0_1px_0_rgba(255,255,255,0.08)]",
        dragOver && "border-accent-sky/60 from-accent-sky/[0.1] to-accent-sky/[0.03]"
      )}
    >
      {/* Drag-over overlay: a clear drop target appears while a file is dragged
          over the window. The actual import (copy-into-Logs) runs on drop. */}
      {dragOver && (
        <div className="pointer-events-none absolute inset-1 z-10 flex flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed border-accent-sky/50 bg-surface-overlay text-center">
          <CloudUpload className="size-7 text-accent-sky" aria-hidden />
          <span className="text-sm font-medium text-accent-sky">Drop your .log to add it</span>
        </div>
      )}
      {importing && (
        <div className="pointer-events-none absolute inset-1 z-10 flex flex-col items-center justify-center gap-2 rounded-lg bg-surface-overlay text-center">
          <span className="size-5 animate-spin rounded-full border-2 border-white/[0.1] border-t-accent-sky" />
          <span className="text-sm text-muted-foreground">Adding log to your folder…</span>
        </div>
      )}
      {/* Folder identity row: a sky folder-icon chip + the source path make it
          unmistakable that this is your on-disk Logs folder, not a generic
          panel. The path reads as a path; the count anchors "what's in here". */}
      <div className="mb-2.5 flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2.5">
          <span className="flex size-9 shrink-0 items-center justify-center rounded-lg border border-accent-sky/20 bg-accent-sky/[0.08] text-accent-sky">
            <FolderOpen className="size-4" aria-hidden />
          </span>
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold text-foreground/90">Logs folder</span>
              {logsDir && (
                <span className="rounded-md bg-white/[0.06] px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground tabular-nums">
                  {logs.length} {logs.length === 1 ? "file" : "files"}
                  {totalBytes > 0 && ` · ${compactBytes(totalBytes)}`}
                </span>
              )}
              {totalBytes > 5 * 1024 * 1024 * 1024 && (
                <InfoPill color="amber" className="text-[10px]">
                  Folder large — delete old archives
                </InfoPill>
              )}
            </div>
            {logsDir ? (
              <div className="truncate font-mono text-[11px] text-muted-foreground" title={logsDir}>
                {logsDir}
              </div>
            ) : (
              <div className="text-[11px] text-amber-400/90">{detection?.message}</div>
            )}
          </div>
        </div>
        <div className="flex shrink-0 gap-1">
          <SimpleTooltip content="Refresh logs" side="bottom">
            <Button variant="ghost" size="icon-sm" onClick={onRefresh} aria-label="Refresh logs">
              <RefreshCw className="size-3.5" />
            </Button>
          </SimpleTooltip>
          <SimpleTooltip content="Open Logs folder in File Explorer" side="bottom">
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={onOpenFolder}
              aria-label="Open Logs folder"
            >
              <FolderOpen className="size-3.5" />
            </Button>
          </SimpleTooltip>
          <SimpleTooltip content="Choose a different Logs folder" side="bottom">
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={onPickFolder}
              aria-label="Choose folder"
            >
              <FolderSearch className="size-3.5" />
            </Button>
          </SimpleTooltip>
        </div>
      </div>

      {/* Search + filter + sort, shown only when the folder is busy enough. */}
      {showControls && !listError && (
        <div className="mb-2 space-y-2">
          <div className="relative">
            <Search
              className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground/60"
              aria-hidden
            />
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search logs…"
              aria-label="Search logs"
              className="h-8 pl-8 text-xs"
            />
          </div>
          <div className="flex items-center justify-between gap-2">
            <div className="flex gap-1" role="group" aria-label="Filter logs">
              {(
                [
                  { id: "all", label: "All" },
                  { id: "active", label: "Active" },
                  { id: "archives", label: "Archives" },
                ] as { id: LogFilter; label: string }[]
              ).map((f) => (
                <button
                  key={f.id}
                  type="button"
                  aria-pressed={filter === f.id}
                  onClick={() => setFilter(f.id)}
                  className={cn(
                    "rounded-md border px-2 py-0.5 text-[11px] font-medium transition-colors",
                    "focus-visible:ring-2 focus-visible:ring-accent-sky/30 focus-visible:outline-none",
                    filter === f.id
                      ? "border-accent-sky/40 bg-accent-sky/[0.06] text-accent-sky"
                      : "border-white/[0.08] bg-white/[0.02] text-muted-foreground hover:text-foreground/80"
                  )}
                >
                  {f.label}
                </button>
              ))}
            </div>
            <button
              type="button"
              onClick={() => setSort((s) => (s === "newest" ? "largest" : "newest"))}
              className="inline-flex items-center gap-1 rounded-md border border-white/[0.08] bg-white/[0.02] px-2 py-0.5 text-[11px] font-medium text-muted-foreground transition-colors hover:text-foreground/80 focus-visible:ring-2 focus-visible:ring-accent-sky/30 focus-visible:outline-none"
              aria-label={`Sorted by ${sort === "newest" ? "newest" : "largest"} first — tap to sort by ${sort === "newest" ? "largest" : "newest"}`}
            >
              <ArrowDownUp className="size-3" aria-hidden />
              {sort === "newest" ? "Newest" : "Largest"}
            </button>
          </div>
        </div>
      )}

      {listError ? (
        // On-brand error card: 3px red left-accent, icon + headline + raw detail,
        // so a folder-access failure reads as an intentional state, not a glitch.
        <div className="rounded-lg border border-red-500/15 border-l-[3px] border-l-red-500 bg-red-500/[0.04] p-3">
          <div className="flex items-center gap-2 text-sm font-medium text-red-300/90">
            <AlertTriangle className="size-4 shrink-0" aria-hidden />
            Couldn't read this folder
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            Check it's accessible and try Refresh.
          </p>
          <p className="mt-1 text-xs break-words text-muted-foreground/70">{listError}</p>
        </div>
      ) : logs.length === 0 ? (
        // Unified empty state matching the FightList dashed pattern.
        <div className="flex flex-col items-center gap-2 rounded-lg border border-dashed border-white/[0.08] p-5 text-center">
          <FileText className="size-6 text-muted-foreground/40" aria-hidden />
          <p className="text-sm text-muted-foreground">
            {detection && !detection.encounterLogExists
              ? "No Encounter.log yet. Type /encounterlog in chat (or use a logging addon) to start recording."
              : "No log files found in this folder."}
          </p>
        </div>
      ) : (
        <ul
          className="max-h-52 space-y-1 overflow-y-auto rounded-xl border border-black/40 bg-black/25 p-1.5 shadow-[inset_0_2px_8px_-2px_rgba(0,0,0,0.6)]"
          aria-label="Log files"
          // Lightweight roving navigation: Up/Down/Home/End move focus between
          // log rows so a long folder isn't N Tab presses. Tab still works as a
          // fallback; we deliberately keep the aria-pressed button model (not a
          // listbox) for consistency with the rest of the uploader's selectors.
          onKeyDown={(e) => {
            const keys = ["ArrowDown", "ArrowUp", "Home", "End"];
            if (!keys.includes(e.key)) return;
            const buttons = Array.from(
              e.currentTarget.querySelectorAll<HTMLButtonElement>("button")
            );
            if (buttons.length === 0) return;
            const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
            e.preventDefault();
            let next: number;
            if (e.key === "Home") next = 0;
            else if (e.key === "End") next = buttons.length - 1;
            else if (e.key === "ArrowDown") next = current < 0 ? 0 : (current + 1) % buttons.length;
            else next = current <= 0 ? buttons.length - 1 : current - 1;
            buttons[next]?.focus();
          }}
        >
          {visible.length === 0 ? (
            <li className="rounded-lg border border-dashed border-white/[0.08] px-3 py-4 text-center text-xs text-muted-foreground">
              No logs match — clear the search or filter.
            </li>
          ) : null}
          {visible.map((log) => {
            const isSelected = selectedLog === log.path;
            return (
              // The row is a group container so the per-file actions can sit as
              // SIBLINGS of the select button (buttons can't nest in buttons) and
              // reveal on hover / keyboard focus-within.
              <li key={log.path} className="group/row relative">
                <button
                  type="button"
                  data-log-path={log.path}
                  onClick={() => onSelect(log.path)}
                  className={cn(
                    "flex w-full items-center justify-between gap-3 rounded-lg border py-2 pr-24 pl-3 text-left transition-all duration-150",
                    "focus-visible:border-accent-sky/40 focus-visible:ring-2 focus-visible:ring-accent-sky/30 focus-visible:outline-none",
                    isSelected
                      ? // Selected row pops OFF the recessed list: lit sky fill, a
                        // left accent bar, and a glow so the current choice is loud.
                        "border-accent-sky/50 border-l-[3px] border-l-accent-sky bg-accent-sky/[0.12] shadow-[0_2px_12px_-2px_color-mix(in_oklab,var(--accent-sky)_35%,transparent)]"
                      : "border-transparent bg-white/[0.03] hover:bg-white/[0.06]"
                  )}
                  aria-pressed={isSelected}
                >
                  <div className="flex min-w-0 items-center gap-2">
                    <FileText
                      className={cn(
                        "size-4 shrink-0",
                        isSelected ? "text-accent-sky" : "text-muted-foreground"
                      )}
                      aria-hidden
                    />
                    <div className="min-w-0">
                      <div className="truncate text-sm text-foreground/90">{log.fileName}</div>
                      <div className="text-xs text-muted-foreground">
                        {compactBytes(log.sizeBytes)} · {relativeFromMs(log.modifiedAtMs)}
                      </div>
                    </div>
                  </div>
                  {/* Scanning / active status, co-located with its row. Hidden
                      when the action cluster is showing so they don't overlap. */}
                  {isSelected && scanning ? (
                    <InfoPill color="sky" className="shrink-0 gap-1">
                      <span className="size-2.5 animate-spin rounded-full border-2 border-accent-sky/30 border-t-accent-sky" />
                      Scanning
                    </InfoPill>
                  ) : (
                    log.isActive && (
                      <InfoPill
                        color="sky"
                        className="shrink-0 gap-1 transition-opacity group-hover/row:opacity-0 group-focus-within/row:opacity-0"
                      >
                        <Radio className="size-3 animate-pulse" aria-hidden /> Active
                      </InfoPill>
                    )
                  )}
                </button>

                {/* Per-file actions — reveal, copy path, delete. Sit over the row's
                    right edge; appear on hover/focus, always present for keyboard.
                    stopPropagation so they never trigger row selection. */}
                <div className="absolute top-1/2 right-2 flex -translate-y-1/2 items-center gap-0.5 opacity-0 transition-opacity group-hover/row:opacity-100 group-focus-within/row:opacity-100">
                  <SimpleTooltip content="Reveal in Explorer" side="top">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="size-7 text-muted-foreground/70 hover:text-foreground"
                      onClick={(e) => {
                        e.stopPropagation();
                        onReveal(log.path);
                      }}
                      aria-label={`Reveal ${log.fileName} in Explorer`}
                    >
                      <FolderInput className="size-3.5" />
                    </Button>
                  </SimpleTooltip>
                  <SimpleTooltip content="Copy file path" side="top">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="size-7 text-muted-foreground/70 hover:text-foreground"
                      onClick={(e) => {
                        e.stopPropagation();
                        onCopyPath(log.path);
                      }}
                      aria-label={`Copy path of ${log.fileName}`}
                    >
                      <ClipboardCopy className="size-3.5" />
                    </Button>
                  </SimpleTooltip>
                  <SimpleTooltip
                    content={
                      log.isActive ? "Can't delete — this log is still being written" : "Delete log"
                    }
                    side="top"
                  >
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="size-7 text-muted-foreground/70 hover:text-red-400"
                      disabled={log.isActive}
                      onClick={(e) => {
                        e.stopPropagation();
                        if (!log.isActive) onRequestDelete(log);
                      }}
                      aria-label={
                        log.isActive
                          ? `${log.fileName} is active and can't be deleted`
                          : `Delete ${log.fileName}`
                      }
                    >
                      <Trash2 className="size-3.5" />
                    </Button>
                  </SimpleTooltip>
                </div>
              </li>
            );
          })}
        </ul>
      )}

      {/* Discoverability for drag-drop — a quiet hint that you can drop a log
          from anywhere; the backend copies it into this folder first. */}
      {!listError && (
        <p className="mt-2 text-center text-[11px] text-muted-foreground/60">
          or drop a <code className="text-muted-foreground/80">.log</code> file here from anywhere
        </p>
      )}
    </div>
  );
}

function Preflight({
  preflight,
  scanning,
  scanningSizeBytes,
  onSplit,
}: {
  preflight: LogPreflight | null;
  scanning: boolean;
  scanningSizeBytes: number | null;
  onSplit: () => void;
}) {
  if (scanning && !preflight) {
    // Surface the known file size so a long scan of a multi-GB log reads as
    // expected work, not a hang, and show skeleton pills shaped like the real
    // result so the layout doesn't jump when it resolves.
    const sizeHint = scanningSizeBytes ? ` (${compactBytes(scanningSizeBytes)})` : "";
    const big = (scanningSizeBytes ?? 0) > 256 * 1024 * 1024;
    return (
      <div className="space-y-2 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2">
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <span className="size-3.5 animate-spin rounded-full border-2 border-white/[0.1] border-t-primary" />
          Scanning the log{sizeHint}…{big ? " this may take a moment." : ""}
        </div>
        <div className="flex gap-2" aria-hidden>
          <span className="h-5 w-16 animate-pulse rounded-lg bg-white/[0.05]" />
          <span className="h-5 w-20 animate-pulse rounded-lg bg-white/[0.05]" />
          <span className="h-5 w-20 animate-pulse rounded-lg bg-white/[0.05]" />
        </div>
      </div>
    );
  }
  if (!preflight) return null;

  // The summary card carries the size/fights/sessions counts now; this row is the
  // split-affordance + large-log nudge. Render nothing when there's neither a
  // reason to split nor multiple sessions to carve.
  // Split is always available for a selected log (the workbench handles the
  // single-session case fine). The label/nudge adapts to the log.
  const sessionCount = preflight.sessions.length;
  return (
    <div className="flex flex-wrap items-center gap-2 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2 text-sm">
      <span
        className={cn(
          "text-xs",
          preflight.recommendSplit ? "text-amber-400/90" : "text-muted-foreground"
        )}
      >
        {preflight.recommendSplit
          ? "This log is large — splitting by session helps it upload."
          : sessionCount > 1
            ? `${sessionCount} logging sessions in this file.`
            : "Split this log into per-session files to upload them separately."}
      </span>
      <Button
        variant={preflight.recommendSplit ? "outline" : "ghost"}
        size="sm"
        onClick={onSplit}
        className={cn("ml-auto", !preflight.recommendSplit && "text-foreground/70")}
      >
        <Scissors className="size-3.5" />
        {preflight.recommendSplit ? "Split by session…" : "Split…"}
      </Button>
    </div>
  );
}

function LiveToggles({
  options,
  onChange,
  disabled,
}: {
  options: UploadOptions;
  onChange: (next: UploadOptions) => void;
  disabled?: boolean;
}) {
  // Note: live mode is definitionally real-time (the official uploader tails the
  // running file), so there is no real-time toggle — it would be a no-op.
  return (
    <div className="mt-4 space-y-2 border-t border-white/[0.06] pt-4">
      <SectionHeader>Live Options</SectionHeader>
      <Toggle
        checked={options.includeEntireFile}
        disabled={disabled}
        onChange={(v) => onChange({ ...options, includeEntireFile: v })}
        label="Include earlier fights"
        hint="Also upload fights already in the log, not just new ones."
      />
    </div>
  );
}

function Toggle({
  checked,
  onChange,
  label,
  hint,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  hint: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={`${label} — ${hint}`}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className="flex w-full items-center justify-between gap-3 rounded-lg px-1 py-1.5 text-left transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-accent-sky/40 focus-visible:outline-none disabled:opacity-50"
    >
      <span>
        <span className="block text-sm text-foreground/90">{label}</span>
        <span className="block text-xs text-muted-foreground">{hint}</span>
      </span>
      <span
        className={cn(
          "relative h-5 w-9 shrink-0 rounded-full transition-colors duration-200",
          checked ? "bg-accent-sky/70" : "bg-white/[0.1]"
        )}
      >
        <span
          className={cn(
            "absolute top-0.5 size-4 rounded-full bg-white transition-transform duration-200",
            checked ? "translate-x-4" : "translate-x-0.5"
          )}
        />
      </span>
    </button>
  );
}

function ManualActions({
  canUpload,
  uploading,
  transport,
  willUseNative,
  onUpload,
}: {
  canUpload: boolean;
  uploading: boolean;
  transport: TransportInfo | null;
  // The *intended* transport (opt-in + a captured session). The backend coverage
  // gate still decides for real per log, so this is an honest hint, not a promise
  // — the fallback caption keeps a fallback from feeling like a bug.
  willUseNative: boolean;
  onUpload: () => void;
}) {
  const installed = transport?.officialUploaderInstalled ?? false;
  const label = uploading
    ? "Preparing…"
    : willUseNative
      ? "Upload directly"
      : installed
        ? "Upload to ESO Logs"
        : "Open the ESO Logs uploader";

  return (
    // The climax — the MOST raised surface, and the only WARM (gold) one, so it
    // reads as the destination of the whole flow against the cool-blue inputs
    // above it. Strong outer shadow + gold top highlight lift it off the canvas.
    <div className="relative flex flex-col items-center gap-3 overflow-hidden rounded-2xl border border-primary/25 bg-gradient-to-b from-primary/[0.1] to-primary/[0.02] p-5 shadow-[0_16px_44px_-16px_rgba(0,0,0,0.75),0_0_40px_-20px_color-mix(in_oklab,var(--primary)_40%,transparent),inset_0_1px_0_rgba(255,255,255,0.08)]">
      <span
        className="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-primary/0 via-primary/60 to-primary/0"
        aria-hidden
      />
      <Button
        onClick={onUpload}
        disabled={!canUpload}
        size="lg"
        className="w-full sm:w-auto"
        aria-label="Upload your log to ESO Logs"
      >
        <CloudUpload className="size-4" />
        {label}
      </Button>

      <p className="max-w-md text-center text-xs text-muted-foreground">
        {willUseNative
          ? "Your report appears here when it's done. If a log has an event type Kalpa can't upload directly, it falls back to the official uploader automatically."
          : installed
            ? "Uploads run through the official ESO Logs uploader installed on your PC."
            : "We'll open the official ESO Logs uploader (or its download page) with your prepared log."}
      </p>
    </div>
  );
}

/** A monospace command chip with a one-click copy — for the in-game slash commands
 *  the live waiting state asks the user to type (`/reloadui`, `/encounterlog on`). */
function CopyChip({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(text);
          setCopied(true);
          setTimeout(() => setCopied(false), 1500);
        } catch {
          /* clipboard may be blocked; the visible text is still copyable manually */
        }
      }}
      className="inline-flex items-center gap-1.5 rounded-md border border-white/[0.08] bg-black/30 px-2 py-1 font-mono text-xs text-foreground/85 transition-colors hover:border-white/15 hover:bg-black/40"
      aria-label={`Copy ${text}`}
    >
      {copied ? (
        <Check className="size-3 text-emerald-400" />
      ) : (
        <Copy className="size-3 opacity-70" />
      )}
      {text}
    </button>
  );
}

function LiveDashboard({
  running,
  starting,
  canStart,
  startMs,
  liveFights,
  liveFightCount,
  liveReport,
  priorFightCount,
  handedOff,
  sessionAnchored,
  readiness,
  onStart,
  onStop,
  onCopyLink,
  onForceHandoff,
}: {
  running: boolean;
  starting: boolean;
  canStart: boolean;
  startMs: number | null;
  liveFights: LiveFight[];
  liveFightCount: number;
  liveReport: ReportRef | null;
  priorFightCount: number;
  handedOff: boolean;
  sessionAnchored: boolean;
  readiness: LiveReadiness | null;
  onStart: () => void;
  onStop: () => void;
  onCopyLink: (url: string) => void | Promise<void>;
  onForceHandoff: () => void;
}) {
  const detecting = running && liveFightCount > 0;
  // The native path has a WAITING phase: armed, but the encoder needs a BEGIN_LOG to
  // anchor a session, so nothing streams until one arrives. `sessionAnchored` (the
  // SessionAnchored event = first BEGIN_LOG) is the ground truth that flips
  // waiting→streaming — instant, no timeout. The handoff path has no waiting phase (the
  // official uploader picks up mid-session), so it's never "waiting" here.
  const isNative = running && !handedOff;
  const waiting = isNative && !sessionAnchored;
  // While waiting, the readiness probe says whether logging is already running (needs
  // /reloadui) or not yet (turn on /encounterlog). A confident "already running" verdict
  // also offers the official-uploader escape hatch.
  const alreadyLogging = readiness?.verdict === "activeNoHeader";
  return (
    <GlassPanel variant="primary" className="space-y-3 p-4">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2.5">
          <SectionHeader>Live Session</SectionHeader>
          {running && (
            <>
              {/* The glanceable honesty cue: AMBER "ARMED" while a native session waits
                  for its first BEGIN_LOG (nothing's streaming yet), EMERALD "LIVE" once
                  anchored (or for the handoff path, which is live immediately). Not red
                  — red is reserved for real errors. */}
              {waiting ? (
                <InfoPill color="amber" className="gap-1.5">
                  <span className="relative flex size-2">
                    <span className="absolute inline-flex size-full animate-ping rounded-full bg-amber-400/70" />
                    <span className="relative inline-flex size-2 rounded-full bg-amber-400" />
                  </span>
                  ARMED
                </InfoPill>
              ) : (
                <InfoPill color="emerald" className="gap-1.5">
                  <span className="relative flex size-2">
                    <span className="absolute inline-flex size-full animate-ping rounded-full bg-emerald-400/70" />
                    <span className="relative inline-flex size-2 rounded-full bg-emerald-400" />
                  </span>
                  LIVE
                </InfoPill>
              )}
              {startMs !== null && <SessionTimer startMs={startMs} />}
            </>
          )}
        </div>
        {running ? (
          <Button variant="outline" size="sm" onClick={onStop}>
            {/* Handoff: Kalpa only "tracks", so "Stop tracking" is honest. Native:
                Stop genuinely ends the upload, so don't call it mere "tracking". */}
            {handedOff ? "Stop tracking" : "Stop upload"}
          </Button>
        ) : (
          <Button size="sm" onClick={onStart} disabled={!canStart || starting}>
            <Radio className="size-3.5" />
            {starting ? "Starting…" : "Start live logging"}
          </Button>
        )}
      </div>

      {/* Report-ready, promoted to the top so it's seen the moment it lands. */}
      {liveReport && (
        <div className="flex items-center justify-between gap-3 rounded-lg border border-emerald-400/25 bg-emerald-400/[0.06] px-3 py-2.5">
          <div className="min-w-0">
            <div className="text-xs font-medium uppercase tracking-wide text-emerald-400/90">
              Report ready
            </div>
            <div className="truncate text-base text-foreground/90">{liveReport.code}</div>
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => void onCopyLink(liveReport.url)}
              aria-label="Copy report link"
            >
              <Copy className="size-3.5" />
            </Button>
            <Button variant="ghost" size="sm" onClick={() => void openReportUrl(liveReport.url)}>
              <ExternalLink className="size-3.5" />
              ESO Logs
            </Button>
            {/* The richer analysis lives in the ESO Log Aggregator (esotk) — make it
                the primary action. While a native session is still streaming, deep-link
                to esotk's LiveLog view (30s repoll follows the newest fight). */}
            <Button
              size="sm"
              className="bg-emerald-500/15 text-emerald-300 hover:bg-emerald-500/25"
              onClick={() =>
                void openReportUrl(esotkReportUrl(liveReport.code, { live: isNative }))
              }
            >
              <Zap className="size-3.5" />
              {isNative ? "Watch live" : "View analysis"}
            </Button>
          </div>
        </div>
      )}

      {/* Scannable "what Stop does" callout — the single most important thing to
          understand in live mode. PATH-AWARE: on the handoff path a separate uploader
          app does the actual upload (Stop here only ends Kalpa's timeline); on the
          native path Kalpa IS the uploader (Stop ends the upload + closes the report).
          Showing the handoff text for a native session (or vice-versa) is actively
          misleading, so it branches on the path the running session actually took. */}
      {running &&
        (handedOff ? (
          <div className="rounded-lg border border-amber-500/15 border-l-[3px] border-l-amber-500 bg-amber-500/[0.04] p-3">
            <div className="flex items-center gap-2 text-xs font-medium text-amber-300/90">
              <AlertCircle className="size-3.5 shrink-0" aria-hidden />
              Kalpa tracks; the ESO Logs uploader uploads
            </div>
            <ul className="mt-1.5 space-y-1 pl-5 text-xs text-muted-foreground">
              <li className="list-disc">
                <span className="text-amber-400/90">Stop tracking</span> ends this timeline in
                Kalpa.
              </li>
              <li className="list-disc">
                The ESO Logs uploader keeps streaming in its own window.
              </li>
              <li className="list-disc">
                To end uploading: stop it there and turn off in-game logging.
              </li>
            </ul>
          </div>
        ) : (
          <div className="rounded-lg border border-emerald-500/15 border-l-[3px] border-l-emerald-500 bg-emerald-500/[0.04] p-3">
            <div className="flex items-center gap-2 text-xs font-medium text-emerald-300/90">
              <Radio className="size-3.5 shrink-0" aria-hidden />
              Kalpa is uploading directly to ESO Logs
            </div>
            <ul className="mt-1.5 space-y-1 pl-5 text-xs text-muted-foreground">
              <li className="list-disc">
                Fights stream straight from Kalpa — no separate uploader window.
              </li>
              <li className="list-disc">
                <span className="text-emerald-400/90">Stop</span> ends the upload and closes the
                report on esologs.com.
              </li>
              <li className="list-disc">Keep Kalpa open until you stop or finish the raid.</li>
            </ul>
          </div>
        ))}

      {/* NATIVE WAITING (armed, not yet anchored): the encoder needs a BEGIN_LOG, so
          guide the user to produce one — immediately, no timeout. Which guidance shows
          first comes from the readiness probe; SessionAnchored then flips this to the
          streaming state below the instant a session header lands. */}
      {waiting &&
        (alreadyLogging ? (
          // Logging is ALREADY running → Kalpa joins the in-progress session (mid-session
          // warm-up replays the current session from disk to seed the encoder), so NO
          // /reloadui is needed to start. Reassure while warm-up runs; /reloadui is only a
          // fallback (it also forces ESO's disk-buffer flush outside raids), and the
          // official uploader is the explicit escape hatch.
          <div className="rounded-lg border border-accent-sky/20 border-l-[3px] border-l-accent-sky bg-accent-sky/[0.05] p-3">
            <div className="flex items-center gap-2 text-xs font-medium text-accent-sky/90">
              <Radio className="size-3.5 shrink-0" aria-hidden />
              Joining your in-progress session…
            </div>
            <p className="mt-1.5 text-xs text-muted-foreground">
              Kalpa is reading the session you’re already logging and will stream fights from here —
              no reload needed. For a long session this can take a few seconds.
              {readiness?.fightInProgress ? " (A fight is being logged right now.)" : ""}
            </p>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <span className="text-[11px] text-muted-foreground/80">Not starting?</span>
              <CopyChip text="/reloadui" />
              <Button variant="ghost" size="sm" onClick={onForceHandoff}>
                Use the official uploader instead
              </Button>
            </div>
            <p className="mt-1.5 text-[11px] text-muted-foreground/80">
              Only fights from now on are in this report — use “Upload a Log” for earlier ones.
            </p>
          </div>
        ) : (
          // Not logging yet (or uncertain) → turning on /encounterlog writes the header.
          <div className="rounded-lg border border-accent-sky/20 bg-accent-sky/[0.05] p-3">
            <div className="flex items-center gap-2 text-xs font-medium text-accent-sky/90">
              <Radio className="size-3.5 shrink-0" aria-hidden />
              Armed — waiting for a logging session
            </div>
            <p className="mt-1.5 text-xs text-muted-foreground">
              Kalpa uploads the moment ESO starts a logging session — no fights have been sent yet
              (an empty report is reserved on ESO Logs and fills in as you fight). Turn on combat
              logging: type <code className="text-foreground/80">/encounterlog on</code> in ESO (if
              it’s already on, <code className="text-foreground/80">/reloadui</code> starts a fresh
              session). Fights stream here as they finish.
            </p>
            <div className="mt-2 flex flex-wrap gap-2">
              <CopyChip text="/encounterlog on" />
              <CopyChip text="/reloadui" />
            </div>
          </div>
        ))}

      {/* STREAMING (handoff path always; native once anchored). */}
      {running && !waiting && (
        <div
          className={cn(
            "flex items-center gap-2 text-sm",
            detecting ? "text-foreground/80" : "text-muted-foreground"
          )}
          role="status"
          aria-live="polite"
        >
          {detecting && <InfoPill color="emerald">Detecting fights</InfoPill>}
          <span>
            {liveFightCount === 0
              ? isNative
                ? "Logging session started — streaming fights to ESO Logs as they finish."
                : "Watching for combat… start a fight in-game and it'll appear here."
              : `${liveFightCount} fight${liveFightCount === 1 ? "" : "s"} this session.` +
                (liveFightCount > liveFights.length
                  ? ` Showing the latest ${liveFights.length} — your full history is saved on esologs.com.`
                  : "")}
          </span>
        </div>
      )}

      {/* Pre-start expectation: live streams only NEW fights (the tail starts at the
          current end of the log), so earlier fights already in this Encounter.log are
          not part of this live report. Set the expectation; no destructive action —
          the report is already clean, and ESO holds the file open so Kalpa can't
          rotate it anyway. */}
      {!running && priorFightCount > 0 && (
        <div className="flex items-start gap-2 rounded-lg border border-white/[0.06] bg-white/[0.02] p-3 text-xs text-muted-foreground">
          <AlertCircle className="mt-0.5 size-3.5 shrink-0 text-foreground/50" aria-hidden />
          <span>
            Live streams only fights from when you start. The {priorFightCount} earlier fight
            {priorFightCount === 1 ? "" : "s"} already in this log{" "}
            {priorFightCount === 1 ? "isn't" : "aren't"} part of this live report — upload{" "}
            {priorFightCount === 1 ? "it" : "them"} separately with{" "}
            <span className="text-foreground/70">Upload a Log</span> if you want.
          </span>
        </div>
      )}

      <FightList
        fights={rowsFromLive(liveFights)}
        newestFirst
        emptyHint={running ? "No fights yet this session." : "Start live logging to begin."}
      />
    </GlassPanel>
  );
}

/** A scannable label for an upload history row. ESO archive logs carry long,
 *  machine-generated names (`Archive-2025-06-20__03_46_03-Encounter-session02-
 *  1750394097660.log`); strip that noise to the part a human recognizes, while
 *  leaving ordinary names (and user-named splits) intact. */
function tidyLogLabel(fileName: string): string {
  const base = fileName.replace(/\.log$/i, "");
  // Archive pattern with a session number → keep the readable "session NN".
  const sess = base.match(/-session(\d+)/i);
  if (/^Archive-/i.test(base) && sess) {
    const datePart = base.match(/Archive-(\d{4}-\d{2}-\d{2})/);
    return datePart
      ? `Archive ${datePart[1]} · session ${Number(sess[1])}`
      : `Session ${Number(sess[1])}`;
  }
  // ISO-stamped archive (Archive-20260614T190354Z-Encounter) → "Archive Jun 14".
  const iso = base.match(/^Archive-(\d{4})(\d{2})(\d{2})T\d+Z?/i);
  if (iso) {
    const [, y, m, d] = iso;
    const dt = new Date(Number(y), Number(m) - 1, Number(d));
    return `Archive · ${dt.toLocaleDateString(undefined, { month: "short", day: "numeric" })}`;
  }
  // Drop a trailing epoch-ms id some names carry.
  return base.replace(/-\d{13,}$/, "");
}

function HistoryPanel({
  history,
  onCopyLink,
  onRefresh,
  onAttachReport,
  onDelete,
}: {
  history: UploadRecord[];
  onCopyLink: (url: string) => void | Promise<void>;
  onRefresh: () => void;
  onAttachReport: (id: string, url: string) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
}) {
  const [attachingId, setAttachingId] = useState<string | null>(null);
  const [linkDraft, setLinkDraft] = useState("");
  // Inline two-step confirm: clicking trash arms the row; a second click on the
  // revealed "Remove" confirms. Removing a history record never touches the file.
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  if (history.length === 0) return null;

  const submitLink = async (id: string) => {
    const raw = linkDraft.trim();
    if (!raw) return;
    // The backend only accepts a full esologs.com/reports/<code> URL. We tell
    // users they can paste just the code, so normalize a bare alphanumeric code
    // into the canonical URL here before sending.
    const url = /^[a-zA-Z0-9]+$/.test(raw) ? `https://www.esologs.com/reports/${raw}` : raw;
    await onAttachReport(id, url);
    setAttachingId(null);
    setLinkDraft("");
  };

  return (
    <div className={cn(WORK_PANEL, "p-3.5")}>
      <div className="mb-2 flex items-center justify-between">
        <SectionHeader>Recent Uploads</SectionHeader>
        <SimpleTooltip content="Refresh history" side="bottom">
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => void onRefresh()}
            aria-label="Refresh history"
          >
            <RefreshCw className="size-3.5" />
          </Button>
        </SimpleTooltip>
      </div>
      <ul className="space-y-1.5">
        {history.slice(0, 8).map((r) => (
          <li
            key={r.id}
            className="rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2 transition-colors hover:bg-white/[0.04]"
          >
            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="truncate text-sm text-foreground/90">
                  {tidyLogLabel(r.fileName)}
                </div>
                <div className="mt-0.5 flex items-center gap-1.5 text-xs text-muted-foreground">
                  <span className="text-foreground/60">{relativeFromMs(r.createdAtMs)}</span>
                  <span className="text-muted-foreground/40">·</span>
                  <span>
                    {r.fightCount} fight{r.fightCount === 1 ? "" : "s"}
                  </span>
                  <span className="text-muted-foreground/40">·</span>
                  <span className="capitalize">{r.visibility}</span>
                </div>
              </div>
              <div className="flex shrink-0 items-center gap-1.5">
                <StatusBadge status={r.status} />
                {r.report ? (
                  <>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      onClick={() => void onCopyLink(r.report!.url)}
                      aria-label="Copy report link"
                    >
                      <Copy className="size-3.5" />
                    </Button>
                    <SimpleTooltip content="Open the raw report on ESO Logs" side="top">
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={() => void openReportUrl(r.report!.url)}
                        aria-label="Open report on ESO Logs"
                      >
                        <ExternalLink className="size-3.5" />
                      </Button>
                    </SimpleTooltip>
                    {/* The richer analysis (fight detection, rotation, scribing, replay)
                        lives in the ESO Log Aggregator — the primary "view" action. */}
                    <Button
                      variant="ghost"
                      size="sm"
                      className="text-emerald-300/90 hover:bg-emerald-500/15 hover:text-emerald-200"
                      onClick={() => void openReportUrl(esotkReportUrl(r.report!.code))}
                      aria-label="Open analysis in ESO Log Aggregator"
                    >
                      <Zap className="size-3.5" />
                      Analysis
                    </Button>
                  </>
                ) : (
                  // Handed-off uploads finish in the official uploader, so we
                  // can't observe the report code — let the user paste it in.
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      setAttachingId(attachingId === r.id ? null : r.id);
                      setLinkDraft("");
                    }}
                  >
                    <LinkIcon className="size-3.5" />
                    Add link
                  </Button>
                )}
                <SimpleTooltip
                  content="Remove from history (your log file stays on disk)"
                  side="top"
                >
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="text-muted-foreground/70 hover:text-red-400"
                    onClick={() => setConfirmDeleteId(confirmDeleteId === r.id ? null : r.id)}
                    aria-label="Remove this upload from history"
                  >
                    <Trash2 className="size-3.5" />
                  </Button>
                </SimpleTooltip>
              </div>
            </div>
            {confirmDeleteId === r.id && (
              <div className="mt-2 flex items-center justify-between gap-2 rounded-lg border border-red-500/20 bg-red-500/[0.05] px-3 py-2">
                <span className="text-xs text-red-200/90">
                  Remove this record? Your log file stays on disk.
                </span>
                <div className="flex shrink-0 gap-1.5">
                  <Button variant="ghost" size="sm" onClick={() => setConfirmDeleteId(null)}>
                    Cancel
                  </Button>
                  <Button
                    variant="destructive"
                    size="sm"
                    onClick={() => {
                      setConfirmDeleteId(null);
                      void onDelete(r.id);
                    }}
                  >
                    Remove
                  </Button>
                </div>
              </div>
            )}
            {attachingId === r.id && (
              <div className="mt-2 flex items-center gap-2">
                <Input
                  value={linkDraft}
                  onChange={(e) => setLinkDraft(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void submitLink(r.id);
                    if (e.key === "Escape") setAttachingId(null);
                  }}
                  placeholder="Report code or full esologs.com link"
                  aria-label="ESO Logs report link or code"
                  autoFocus
                  className="h-8 text-xs"
                />
                <Button
                  size="sm"
                  onClick={() => void submitLink(r.id)}
                  disabled={!linkDraft.trim()}
                >
                  Save
                </Button>
              </div>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}

function StatusBadge({ status }: { status: UploadRecord["status"] }) {
  switch (status) {
    case "completed":
      return <InfoPill color="emerald">Done</InfoPill>;
    case "uploading":
    case "queued":
      return <InfoPill color="sky">Uploading</InfoPill>;
    case "live":
      return <InfoPill color="red">Live</InfoPill>;
    case "handedOff":
      // The official uploader may still be streaming this one — neutral, not a
      // green "Done" that would imply the upload finished. The external-link cue
      // signals ownership transferred to the separate uploader.
      return (
        <InfoPill
          color="amber"
          className="gap-1"
          title="Finished in the official ESO Logs uploader — paste the report link to track it here."
        >
          <ExternalLink className="size-2.5" aria-hidden /> Handed off
        </InfoPill>
      );
    case "failed":
      return <InfoPill color="red">Failed</InfoPill>;
    case "cancelled":
      return <InfoPill color="muted">Cancelled</InfoPill>;
    default:
      return <InfoPill color="muted">{status}</InfoPill>;
  }
}
