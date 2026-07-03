// The ESO Logs uploader workspace. Full-screen dialog with a glanceable status
// pill, Manual / Live mode tabs, an auto-detected log picker with preflight, a
// per-fight timeline, split-to-disk for oversized logs, upload history, and a
// first-run wizard. Uploads use the official ESO Logs uploader by default, with
// an opt-in native direct route when the backend proves the log/session is safe.

import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type Ref,
} from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import {
  CloudUpload,
  FileText,
  Radio,
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
  CheckCircle2,
  LogIn,
  Trash2,
  UserRound,
  CircleDashed,
  Loader2,
} from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { GlassPanel } from "@/components/ui/glass-panel";
import { SectionHeader } from "@/components/ui/section-header";
import { InfoPill } from "@/components/ui/info-pill";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { getTauriErrorMessage, invokeOrThrow, warnIfSessionNotPersisted } from "@/lib/tauri";
import { getSettingChecked, setSettings } from "@/lib/store";
import { cn } from "@/lib/utils";
import type { AuthUser } from "@/types";
import {
  REGION_OPTIONS,
  type FightSummary,
  type LiveFight,
  type LiveReadiness,
  type LogFileInfo,
  type LogPathDetection,
  type LogPreflight,
  type ReportRef,
  type TransportInfo,
  type UploadDispatch,
  type UploadOptions,
  type UploadRecord,
  type Visibility,
} from "@/types/uploader";
import {
  SessionTimer,
  WhatGetsUploaded,
  attachRaceSafe,
  compactBytes,
  deriveNativeState,
  fightLabel,
  formatDuration,
  liveExitConfirmCopy,
  primaryReportUrl,
  relativeFromMs,
  shouldConfirmLiveExit,
} from "./uploader-shared";
import { UploadOptionsControl } from "./upload-options";
import { FightList, rowsFromLive, rowsFromSummaries } from "./fight-list";
import { SplitWorkbench } from "./split-workbench";
import { dominantZone } from "./naming";
import {
  WORK_PANEL,
  maybeAutoOpenAnalysis,
  openReportUrl,
  usesOfficialUploader,
} from "./uploader-actions";
import { LogPicker } from "./log-picker";
import { HistoryPanel } from "./history-panel";
import { useLiveSession } from "./use-live-session";

interface UploaderWorkspaceProps {
  authUser: AuthUser | null;
  onAuthChange: (user: AuthUser | null) => void;
  onClose: () => void;
}

type Mode = "manual" | "live";

/** The phase the pinned header's single adaptive status pill reflects. Priority
 *  order (highest first): a running live session (armed→live), an in-flight manual
 *  upload, an in-progress scan, a scanned-ready selection, else idle. */
type HeaderPhase =
  | "idle"
  | "scanning"
  | "ready"
  | "uploading"
  | "armed"
  | "live"
  | "attention"
  | "signedOut";

const DEFAULT_OPTIONS: UploadOptions = {
  region: 1,
  guildId: null,
  visibility: "unlisted",
  description: null,
  realTime: false,
  includeEntireFile: false,
};

const OPTIONS_KEY = "kalpa.uploader.options";

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
  // Live mode defaults to native for everyone (the official handoff is an explicit
  // opt-out via `liveUseOfficialUploader`, default false); manual now mirrors this with
  // its own `manualUseOfficialUploader` opt-out. The readout must stay HONEST, though:
  // native also requires an upload session, and Go Live can fail/decline the sign-in
  // prompt and hand off. So gate the live readout on `hasNativeSession` — showing "ESO
  // Logs uploader" until a session is captured. This under-promises only in the narrow
  // "user will sign in at Go Live" case (the safe direction) and never claims "Direct
  // from Kalpa" for a session that handed off.
  const [liveUseOfficial, setLiveUseOfficial] = useState(false);
  // Unified opt-out: native for live iff NEITHER key is opted out (== the manual rule),
  // so the live readout matches live routing in any split persisted state.
  const liveWillUseNative = nativeOptIn && !liveUseOfficial && hasNativeSession;

  // Direct (native) upload is the *intended* MANUAL path when the user hasn't opted out
  // AND has a session. The opt-out is UNIFIED (the Settings toggle writes both keys and
  // the promo reflects both), so a MIGRATED user with only the live opt-out
  // (`manualUseOfficialUploader` unset + `liveUseOfficialUploader=true`) must read as
  // "official" for manual too — hence `!liveUseOfficial`. `nativeOptIn` alone is the raw
  // `!manualUseOfficialUploader`. The per-upload routing (handleManualUpload) reads BOTH
  // keys fresh the same way, so this readout and the actual route never disagree. Live
  // now routes native under the same gate, so the header readout reflects native in live
  // too (the manual-only consumers ManualActions/LogSummaryCard render in manual mode).
  const willUseNative = nativeOptIn && !liveUseOfficial && hasNativeSession;

  // The transport hint for the CURRENT mode. Several shared panels (the header
  // readout, LogSummaryCard's route chip, UploadOptionsControl's report-name field)
  // render in BOTH modes, so they must reflect the mode-correct flag — live uses a
  // different opt-out (`liveUseOfficialUploader`) than manual (`manualUseOfficialUploader`).
  // Passing the manual `willUseNative` into them while in live mode made the route
  // chip / report-name field contradict the (mode-aware) header.
  const activeWillUseNative = mode === "live" ? liveWillUseNative : willUseNative;

  // A pending confirm for an action that would end a RUNNING live session — closing
  // the dialog ("close") or leaving Live for Manual ("manual"). Gated so an accidental
  // Esc/backdrop/X/tab-switch can't silently stop a native upload and close its report.
  const [pendingLiveExit, setPendingLiveExit] = useState<"close" | "manual" | null>(null);

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

  // Persist options, DEBOUNCED (~300ms). Typing the report name updates `options`
  // per keystroke; without the debounce that hit localStorage (a sync main-thread
  // write) on every character. A ref holds the latest value so the unmount flush
  // below can't lose an edit made inside the debounce window.
  const optionsRef = useRef(options);
  useEffect(() => {
    optionsRef.current = options;
    const id = setTimeout(() => {
      try {
        localStorage.setItem(OPTIONS_KEY, JSON.stringify(options));
      } catch {
        /* ignore */
      }
    }, 300);
    return () => clearTimeout(id);
  }, [options]);

  // Flush the latest options on unmount so closing within the debounce window still
  // persists the final choice.
  useEffect(() => {
    return () => {
      try {
        localStorage.setItem(OPTIONS_KEY, JSON.stringify(optionsRef.current));
      } catch {
        /* ignore */
      }
    };
  }, []);

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

  const applyDetectedLogs = useCallback(
    async (det: LogPathDetection) => {
      if (!det.path) {
        setLogsDir(null);
        setLogs([]);
        setListError(null);
        clearSelection();
        return;
      }

      setLogsDir(det.path);
      if (det.logsDirExists) {
        await loadLogs(det.path);
      } else {
        setLogs([]);
        setListError(null);
        clearSelection();
      }
    },
    [clearSelection, loadLogs]
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
        await applyDetectedLogs(det);
      } catch (e) {
        if (!cancelled) toast.error(getTauriErrorMessage(e));
      }
      if (!cancelled) await refreshHistory();
    })();
    return () => {
      cancelled = true;
    };
  }, [applyDetectedLogs, refreshHistory]);

  // Re-read the direct-upload opt-in + session presence. Called on mount and
  // after the user enables/signs in/out inline, so the promoted section and the
  // transport hint stay in sync with Settings and the credential store.
  const refreshNativeState = useCallback(async () => {
    try {
      const [manual, session, live] = await Promise.all([
        // Manual now mirrors live: native is the DEFAULT, opt-OUT via
        // `manualUseOfficialUploader` (default false → native). Read FAIL-CLOSED so a
        // store error presents as opted-out, never claiming "direct" in the readout
        // against an opt-out it couldn't confirm (matches routing's usesOfficialUploader).
        getSettingChecked<boolean>("manualUseOfficialUploader", false),
        // Fail closed on a session-check error too: a rejection must NOT abort the
        // whole refresh (it used to throw here and skip every setState), it means
        // "no session".
        invokeOrThrow<boolean>("uploader_has_session").catch(() => false),
        getSettingChecked<boolean>("liveUseOfficialUploader", false),
      ]);
      // Treat a tainted (untrusted-empty) store as a read failure too.
      const tainted = await invokeOrThrow<boolean>("settings_tainted").catch(() => true);
      const next = deriveNativeState({ manual, live, session, tainted });
      setNativeOptIn(next.nativeOptIn);
      setHasNativeSession(next.hasNativeSession);
      setLiveUseOfficial(next.liveUseOfficial);
    } catch {
      /* best-effort — the upload path still reads the setting fresh per upload */
    }
  }, []);

  // Read the direct-upload opt-in + session on mount. Reuses refreshNativeState (no
  // drifted copy): it's a stable empty-deps callback, so this runs once. Wrapped in an
  // async IIFE so its (post-await) setStates aren't flagged as synchronous-in-effect.
  useEffect(() => {
    void (async () => {
      await refreshNativeState();
    })();
  }, [refreshNativeState]);

  // Stable (useCallback) so the memoized LogPicker doesn't re-render on unrelated
  // state changes (e.g. typing the report name).
  const handlePickFolder = useCallback(async () => {
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
  }, [logsDir, clearSelection, loadLogs]);

  // Switch back to the auto-detected Logs folder after the user manually picked a
  // different one. Mirrors handlePickFolder's folder-switch body (minus the OS
  // dialog): a one-tap, non-destructive undo — nothing is deleted, it just
  // re-points logsDir at the detected path and re-lists. clearSelection() bumps
  // selectTokenRef, orphaning any in-flight preflight scan on the custom folder.
  const handleResetFolder = useCallback(() => {
    const detected = detection?.path;
    if (!detected || detected === logsDir) return;
    clearSelection();
    void applyDetectedLogs(detection);
  }, [applyDetectedLogs, detection, logsDir, clearSelection]);

  const handleRefreshLogs = useCallback(async () => {
    if (!logsDir) {
      try {
        const det = await invokeOrThrow<LogPathDetection>("uploader_detect_path");
        setDetection(det);
        await applyDetectedLogs(det);
      } catch (e) {
        toast.error(getTauriErrorMessage(e));
      }
      return;
    }

    if (detection?.path === logsDir) {
      try {
        const det = await invokeOrThrow<LogPathDetection>("uploader_detect_path");
        setDetection(det);
        await applyDetectedLogs(det);
      } catch (e) {
        toast.error(getTauriErrorMessage(e));
      }
      return;
    }

    await loadLogs(logsDir);
  }, [applyDetectedLogs, detection?.path, loadLogs, logsDir]);

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

  // The live-session engine: all live-mode state + the ref-based session-ownership
  // protocol, the start/stop/force-handoff handlers, and the last-resort unmount
  // teardown. It reads the current selection/options context and drives the refresh
  // callbacks; the render below consumes its returned state/refs unchanged.
  const {
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
    liveSessionIdRef,
    startingRef,
    handleStartLive,
    handleStopLive,
    handleForceHandoffLive,
  } = useLiveSession({
    selectedLog,
    logs,
    options,
    fights,
    autoSelectActiveLog,
    refreshNativeState,
    refreshHistory,
  });

  // Native file drag-drop over the window. Tauri delivers real OS paths (unlike
  // HTML5 drag-drop in a webview), which the backend then copy-confines. We only
  // act on a single dropped .log; the drag-over state drives the picker visual.
  // attachRaceSafe tears the listener down even if this effect is cleaned up
  // (StrictMode double-mount / logsDir change) before onDragDropEvent resolves —
  // otherwise the listener would leak.
  useEffect(() => {
    return attachRaceSafe(async () => {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      return getCurrentWebview().onDragDropEvent((event) => {
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
    });
  }, [handleImportLog]);

  const handleManualUpload = async () => {
    if (!selectedLog) return;
    setUploading(true);
    try {
      // Resolve the effective (unified, fail-closed) opt-out AND session presence fresh
      // per upload. Native is the DEFAULT, but it still needs a captured esologs session
      // — without it the backend would route native and hard-fail "Not signed in", so
      // gate on the session (an opted-in user with no session still hands off).
      // `usesOfficialUploader` honours EITHER opt-out key and fails closed on a store
      // read error, so a migrated/degraded state never silently routes native against
      // the opt-out the UI shows.
      const [useOfficial, hasSession] = await Promise.all([
        usesOfficialUploader(),
        invokeOrThrow<boolean>("uploader_has_session").catch(() => false),
      ]);
      const nativeOptIn = !useOfficial && hasSession;
      const dispatch = await invokeOrThrow<UploadDispatch>("uploader_upload_log", {
        filePath: selectedLog,
        options,
        preferCli: transport?.officialUploaderInstalled ?? false,
        // Reuse the preflight's count so the backend doesn't re-scan a multi-GB
        // log just to fill the history record.
        fightCount: preflight?.totalFights ?? null,
        nativeOptIn,
        // The derived content label for the history row's headline.
        zone: dominantZone(fights),
      });
      if (dispatch.report) {
        toast.success("Upload complete — report ready.");
        // Native upload produced a report code; offer to jump straight to the richer
        // ESO Log Aggregator analysis if the user opted into auto-open.
        void maybeAutoOpenAnalysis(dispatch.report, options.visibility);
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
  // preflight to be loaded so the workbench has sessions to show. Stable (useCallback)
  // so the memoized Preflight doesn't re-render on unrelated state changes.
  const handleSplit = useCallback(() => {
    if (!selectedLog || !preflight) return;
    setWorkbenchOpen(true);
  }, [selectedLog, preflight]);

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

  const isLoggedIn = authUser !== null;

  // ── Pinned header derivations ───────────────────────────────────────────────
  // The single adaptive status pill in the header reflects, in priority order: a
  // running live session (ARMED until its first BEGIN_LOG anchors, then LIVE), an
  // in-flight manual upload, an in-progress scan, a scanned-and-ready selection,
  // else idle. Matches LiveDashboard's armed/live split so the pinned pill never
  // contradicts the body.
  const headerPhase: HeaderPhase = useMemo(() => {
    if (!isLoggedIn) return "signedOut";
    if (liveSessionId) {
      // A live session that needs the user's attention (e.g. the ESO Logs session
      // expired mid-stream and posting is paused) outranks the healthy live state.
      if (liveNeedsAttention) return "attention";
      return sessionAnchored || liveHandedOff ? "live" : "armed";
    }
    if (uploading) return "uploading";
    if (scanning) return "scanning";
    if (selectedLog && preflight) return "ready";
    return "idle";
  }, [
    isLoggedIn,
    liveSessionId,
    liveNeedsAttention,
    sessionAnchored,
    liveHandedOff,
    uploading,
    scanning,
    selectedLog,
    preflight,
  ]);

  // The route's middle (engine) chip must reflect the RUNNING session's real path
  // (native vs handed-off), not just the intended path — a session that handed off
  // should never still read "Direct from Kalpa".
  const routeDirect = liveSessionId !== null ? !liveHandedOff : activeWillUseNative;

  // The most recent upload time, derived order-agnostically (don't assume
  // history[0] is newest), for the idle pill's tooltip.
  const lastUploadMs = useMemo(
    () => (history.length ? Math.max(...history.map((h) => h.createdAtMs)) : null),
    [history]
  );

  // The selected log's dominant zone — the content name the Mission Control band
  // leads with when a log is scanned and ready.
  const headerZone = useMemo(() => dominantZone(fights), [fights]);

  // Preflight fight rows (up to 500), memoized so re-rendering the workspace for an
  // unrelated reason (e.g. typing the report name) doesn't re-map them — and so the
  // memoized FightList sees a stable `fights` reference and can skip its re-render.
  const fightRows = useMemo(() => rowsFromSummaries(fights), [fights]);

  // Stable refresh handler for the memoized LogPicker.
  const handleRefreshLogsClick = useCallback(() => void handleRefreshLogs(), [handleRefreshLogs]);

  // Initial dialog focus: land on the first meaningful control (the Manual mode
  // tab) rather than the close button, so keyboard users start in the flow. When
  // logged out this ref is null and the dialog falls back to default focus (the
  // sign-in button).
  const firstTabRef = useRef<HTMLButtonElement>(null);

  return (
    <Dialog
      open
      onOpenChange={(o) => {
        if (o) return;
        // A3: never let an accidental Esc/backdrop/X close a running (or still-
        // starting) live session out from under the user — on the native path that
        // stops the upload and closes its ESO Logs report. Confirm first; the unmount
        // cleanup (kept as-is) performs the actual teardown once we onClose().
        if (shouldConfirmLiveExit(liveSessionIdRef.current !== null)) {
          setPendingLiveExit("close");
          return;
        }
        onClose();
      }}
    >
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
            <HeaderStatusPill
              phase={headerPhase}
              totalFights={preflight?.totalFights ?? 0}
              logsCount={listError ? 0 : logs.length}
              lastUploadMs={lastUploadMs}
            />
          </div>
          {/* Keep an accessible description for the Dialog's aria-describedby but
              drop the marketing prose — the Mission Control band below + the body's
              "What gets uploaded" note carry the real, honest explanation. */}
          <DialogDescription className="sr-only">
            Turn your Encounter.log into a shareable report on esologs.com.
          </DialogDescription>
          {isLoggedIn ? (
            <MissionControlBand
              phase={headerPhase}
              mode={mode}
              routeDirect={routeDirect}
              officialInstalled={transport?.officialUploaderInstalled ?? false}
              userName={authUser?.userName ?? null}
              sessionPersisted={authUser?.sessionPersisted !== false}
              logsCount={listError ? 0 : logs.length}
              selectedFileName={
                selectedLog ? (selectedLog.split(/[/\\]/).pop() ?? selectedLog) : null
              }
              readyZone={headerZone}
              readyFights={preflight?.totalFights ?? 0}
              readySessions={preflight?.sessions.length ?? 0}
              readySizeBytes={preflight?.sizeBytes ?? 0}
              live={liveSessionId !== null}
              liveStartMs={liveStartMs}
              liveFights={liveFights}
              liveFightCount={liveFightCount}
              liveReport={liveReport}
              liveHandedOff={liveHandedOff}
              visibility={liveVisibility}
              sessionAnchored={sessionAnchored}
              onCopyLink={copyLink}
            />
          ) : (
            <p className="mt-2 text-[13px] leading-relaxed text-muted-foreground">
              Sign in to ESO Logs to upload your combat logs.
            </p>
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
                    // A3: leaving Live unmounts its only Stop control and stops the
                    // session — on the native path that closes its ESO Logs report —
                    // so confirm first rather than ending it on a stray click. Check
                    // the REF (not `liveSessionId` state): a session still starting has
                    // its id in the ref before state lands, and handleStopLive keys off
                    // the ref too, so the confirm's proceed path also cancels an
                    // in-flight start.
                    if (shouldConfirmLiveExit(liveSessionIdRef.current !== null)) {
                      setPendingLiveExit("manual");
                      return;
                    }
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
                onRefresh={handleRefreshLogsClick}
                onPickFolder={handlePickFolder}
                onResetFolder={handleResetFolder}
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
                      fights={fightRows}
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
                    officialInstalled={transport?.officialUploaderInstalled ?? false}
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
                    visibility={liveVisibility}
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

      {/* A3: confirm before an accidental close / Manual-switch ends a running live
          session (native stop closes the ESO Logs report). */}
      <LiveExitConfirm
        open={pendingLiveExit !== null}
        handedOff={liveHandedOff}
        onCancel={() => setPendingLiveExit(null)}
        onConfirm={() => {
          const action = pendingLiveExit;
          setPendingLiveExit(null);
          if (action === "close") {
            // The unmount cleanup stops the session + closes the report.
            onClose();
          } else if (action === "manual") {
            void handleStopLive();
            setMode("manual");
          }
        }}
      />
    </Dialog>
  );
}

// Confirmation before an action that would end a RUNNING live session — closing the
// dialog or leaving Live for Manual. On the native path Stop closes the report on ESO
// Logs, so this can't be a stray Esc/backdrop/click. Copy is path-aware (native vs
// handoff); mirrors DeleteLogConfirm's glass styling.
function LiveExitConfirm({
  open,
  handedOff,
  onCancel,
  onConfirm,
}: {
  open: boolean;
  handedOff: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const copy = liveExitConfirmCopy(handedOff);
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onCancel()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{copy.title}</DialogTitle>
          <DialogDescription>{copy.description}</DialogDescription>
        </DialogHeader>
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="ghost" onClick={onCancel}>
            Keep streaming
          </Button>
          <Button variant="destructive" onClick={onConfirm}>
            {copy.confirmLabel}
          </Button>
        </div>
      </DialogContent>
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

// The pinned header's single adaptive status pill: one glanceable phase word
// (Idle / Scanning / Ready / Uploading / Armed / Live / Signed out) with an
// aria-hidden companion stat (the live timer + fight count, the ready fight
// count, or the folder's log count). Only the phase word lives in the announced
// region, so a screen reader never hears the per-second timer tick.
function HeaderStatusPill({
  phase,
  totalFights,
  logsCount,
  lastUploadMs,
}: {
  phase: HeaderPhase;
  totalFights: number;
  logsCount: number;
  lastUploadMs: number | null;
}) {
  // Live/armed reuse the shared PulseDot so the pinned pill matches the live core
  // and body. Amber = armed (waiting to anchor), emerald = live (red is reserved
  // for real errors, never used here).
  let color: "muted" | "sky" | "emerald" | "amber";
  let icon: ReactNode;
  let label: string;
  switch (phase) {
    case "scanning":
      color = "sky";
      icon = <Loader2 className="size-3.5 animate-spin" aria-hidden />;
      label = "Scanning";
      break;
    case "uploading":
      color = "sky";
      icon = <Loader2 className="size-3.5 animate-spin" aria-hidden />;
      label = "Uploading";
      break;
    case "ready":
      color = "emerald";
      icon = <CheckCircle2 className="size-3.5" aria-hidden />;
      label = "Ready";
      break;
    case "armed":
      color = "amber";
      icon = <PulseDot tone="amber" />;
      label = "Armed";
      break;
    case "live":
      color = "emerald";
      icon = <PulseDot tone="emerald" />;
      label = "Live";
      break;
    case "attention":
      color = "amber";
      icon = <AlertTriangle className="size-3.5" aria-hidden />;
      label = "Attention";
      break;
    case "signedOut":
      color = "muted";
      icon = <CircleDashed className="size-3.5" aria-hidden />;
      label = "Signed out";
      break;
    default:
      color = "muted";
      icon = <CircleDashed className="size-3.5" aria-hidden />;
      label = "Idle";
      break;
  }

  // The aria-hidden companion stat sits OUTSIDE the announced region. In live /
  // armed / attention the new Mission Control band below carries the timer + fight
  // count (and far more), so the pill stays just the announced phase word there —
  // no duplicate readout. The ready / idle phases keep a one-glance companion.
  let companion: ReactNode = null;
  let companionTitle: string | undefined;
  if (phase === "ready" && totalFights > 0) {
    companion = (
      <span>
        · {totalFights} fight{totalFights === 1 ? "" : "s"}
      </span>
    );
  } else if (phase === "idle" && logsCount > 0) {
    companion = (
      <span>
        · {logsCount} log{logsCount === 1 ? "" : "s"}
      </span>
    );
    if (lastUploadMs) companionTitle = `Last upload ${relativeFromMs(lastUploadMs)}`;
  }

  return (
    <div className="flex shrink-0 items-center gap-1.5" title={companionTitle}>
      <InfoPill color={color} role="status" aria-live="polite" className="gap-1.5 px-2.5 py-1">
        {icon}
        {label}
      </InfoPill>
      {companion && (
        <span
          className="flex items-center gap-1.5 text-xs tabular-nums text-muted-foreground"
          aria-hidden
        >
          {companion}
        </span>
      )}
    </div>
  );
}

// A pulsing status dot (ping ring + solid core), shared by the header pill and the
// live core so the "live"/"armed" cue reads identically everywhere. Amber = armed
// (waiting to anchor), emerald = live/streaming. Never red — red is for errors.
function PulseDot({ tone }: { tone: "amber" | "emerald" }) {
  return (
    <span className="relative flex size-2" aria-hidden>
      <span
        className={cn(
          "absolute inline-flex size-full animate-ping rounded-full",
          tone === "amber" ? "bg-amber-400/70" : "bg-emerald-400/70"
        )}
      />
      <span
        className={cn(
          "relative inline-flex size-2 rounded-full",
          tone === "amber" ? "bg-amber-400" : "bg-emerald-400"
        )}
      />
    </span>
  );
}

// The signed-in ESO Logs account chip — the terminal of the route (the account
// that will own the report). Sky = identity; an amber warning rides along when the
// session won't persist past a restart.
function AccountChip({
  userName,
  sessionPersisted,
}: {
  userName: string;
  sessionPersisted: boolean;
}) {
  return (
    <SimpleTooltip content="Reports upload to this ESO Logs account" side="bottom">
      <span
        className="inline-flex shrink-0 items-center gap-1.5 rounded-md border border-accent-sky/20 bg-accent-sky/[0.05] px-2 py-1 font-medium text-accent-sky"
        // Fold the session-only caution into the label: the amber icon is aria-hidden
        // and the `title` lives on a non-focusable span, so without this a screen-reader
        // user would never hear that the sign-in won't persist.
        aria-label={
          sessionPersisted
            ? `Signed in to ESO Logs as ${userName}`
            : `Signed in to ESO Logs as ${userName}. Signed in for this session only — it won't persist after you restart Kalpa.`
        }
      >
        <UserRound className="size-3" aria-hidden />
        <span className="max-w-[140px] truncate">{userName}</span>
        {!sessionPersisted && (
          <span
            title="Signed in for this session only — it won't persist after you restart Kalpa."
            className="inline-flex"
          >
            <AlertTriangle className="size-3 text-amber-400" aria-hidden />
          </span>
        )}
      </span>
    </SimpleTooltip>
  );
}

// The route pipeline: your log → the active engine → esologs.com, then (as a
// sibling terminal) the account chip. The engine chip reflects the effective
// transport (direct = sky/Zap, official = muted). esologs.com stays NEUTRAL — gold
// is reserved exclusively for the Upload action.
function RouteFlow({
  routeDirect,
  officialInstalled,
  userName,
  sessionPersisted,
}: {
  routeDirect: boolean;
  officialInstalled: boolean;
  userName: string | null;
  sessionPersisted: boolean;
}) {
  return (
    <>
      <span
        className="inline-flex items-center gap-2"
        aria-label={`Upload route: your log, ${routeDirect ? "Direct from Kalpa" : "Official uploader"}, to esologs.com`}
      >
        <span className="inline-flex items-center gap-1.5 rounded-md border border-white/[0.08] bg-white/[0.03] px-2 py-1 font-medium text-foreground/80">
          <FileText className="size-3 text-muted-foreground" aria-hidden />
          Your log
        </span>
        <ChevronRight className="size-3 shrink-0 text-muted-foreground/50" aria-hidden />
        {routeDirect ? (
          <span className="inline-flex items-center gap-1.5 rounded-md border border-accent-sky/25 bg-accent-sky/[0.06] px-2 py-1 font-medium text-accent-sky">
            <Zap className="size-3" aria-hidden />
            Direct from Kalpa
          </span>
        ) : (
          <span className="inline-flex items-center gap-1.5 rounded-md border border-white/[0.08] bg-white/[0.03] px-2 py-1 font-medium text-muted-foreground">
            <CloudUpload className="size-3" aria-hidden />
            {officialInstalled ? "Official uploader" : "ESO Logs uploader"}
          </span>
        )}
        <ChevronRight className="size-3 shrink-0 text-muted-foreground/50" aria-hidden />
        <span className="inline-flex items-center gap-1.5 rounded-md border border-white/[0.1] bg-white/[0.04] px-2 py-1 font-medium text-foreground/80">
          esologs.com
        </span>
      </span>
      {userName && (
        <>
          <ChevronRight className="size-3 shrink-0 text-muted-foreground/50" aria-hidden />
          <AccountChip userName={userName} sessionPersisted={sessionPersisted} />
        </>
      )}
    </>
  );
}

// ── Mission Control header band ──────────────────────────────────────────────
// The pinned, adaptive header below the title. It stays a slim instrument row at
// rest (a contextual readout + the route → account flow) and, the moment a live
// session is running, expands into a real glance dashboard — a state core (orb +
// big timer + fight count), a fight ticker, and the report link the instant it
// lands — so the most relevant state is always visible while the body scrolls.
function MissionControlBand({
  phase,
  mode,
  routeDirect,
  officialInstalled,
  userName,
  sessionPersisted,
  logsCount,
  selectedFileName,
  readyZone,
  readyFights,
  readySessions,
  readySizeBytes,
  live,
  liveStartMs,
  liveFights,
  liveFightCount,
  liveReport,
  liveHandedOff,
  visibility,
  sessionAnchored,
  onCopyLink,
}: {
  phase: HeaderPhase;
  mode: Mode;
  routeDirect: boolean;
  officialInstalled: boolean;
  userName: string | null;
  sessionPersisted: boolean;
  logsCount: number;
  selectedFileName: string | null;
  readyZone: string | null;
  readyFights: number;
  readySessions: number;
  readySizeBytes: number;
  live: boolean;
  liveStartMs: number | null;
  liveFights: LiveFight[];
  liveFightCount: number;
  liveReport: ReportRef | null;
  liveHandedOff: boolean;
  visibility: Visibility;
  sessionAnchored: boolean;
  onCopyLink: (url: string) => void | Promise<void>;
}) {
  if (live) {
    return (
      <LiveDashboardBand
        phase={phase}
        routeDirect={routeDirect}
        officialInstalled={officialInstalled}
        userName={userName}
        sessionPersisted={sessionPersisted}
        startMs={liveStartMs}
        fights={liveFights}
        fightCount={liveFightCount}
        report={liveReport}
        handedOff={liveHandedOff}
        visibility={visibility}
        sessionAnchored={sessionAnchored}
        onCopyLink={onCopyLink}
      />
    );
  }

  // ── Slim instrument row (idle / scanning / ready / uploading) ──────────────
  // The phase-specific lead: what the header confirms at a glance right now.
  let lead: ReactNode;
  if (phase === "ready") {
    lead = (
      <span className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5">
        <Swords className="size-3 shrink-0 text-muted-foreground" aria-hidden />
        <span className="truncate font-medium text-foreground/90">{readyZone ?? "Combat log"}</span>
        <span className="text-muted-foreground/70">
          · {readyFights} fight{readyFights === 1 ? "" : "s"}
          {readySessions > 0 && ` · ${readySessions} session${readySessions === 1 ? "" : "s"}`}
          {readySizeBytes > 0 && ` · ${compactBytes(readySizeBytes)}`}
        </span>
      </span>
    );
  } else if (phase === "scanning") {
    lead = (
      <span className="flex min-w-0 items-center gap-1.5 text-muted-foreground">
        <Loader2 className="size-3 shrink-0 animate-spin" aria-hidden />
        <span className="truncate">Scanning {selectedFileName ?? "the log"}…</span>
      </span>
    );
  } else if (phase === "uploading") {
    lead = (
      <span className="flex min-w-0 items-center gap-1.5 text-muted-foreground">
        <Loader2 className="size-3 shrink-0 animate-spin" aria-hidden />
        <span className="truncate">Uploading {selectedFileName ?? "your log"}…</span>
      </span>
    );
  } else {
    // Idle — a quiet directive (the status pill already carries the log count).
    lead = (
      <span className="text-muted-foreground/80">
        {logsCount > 0 ? "Pick a log to upload" : "Turn on /encounterlog in ESO to start a log"}
      </span>
    );
  }

  return (
    <div className="mt-2.5 flex flex-wrap items-center gap-x-2.5 gap-y-1.5 text-[11px]">
      <span className="font-heading text-[10px] font-bold tracking-[0.08em] text-muted-foreground/50 uppercase">
        {mode === "live" ? "Live Log" : "Upload a Log"}
      </span>
      <span className="h-3 w-px bg-white/[0.08]" aria-hidden />
      {lead}
      <span className="min-w-2 flex-1" aria-hidden />
      <RouteFlow
        routeDirect={routeDirect}
        officialInstalled={officialInstalled}
        userName={userName}
        sessionPersisted={sessionPersisted}
      />
    </div>
  );
}

// The live glance dashboard the header expands into while a session runs. Three
// zones — state core, fight ticker, report CTA — over a route/account footer. Tone
// follows the phase: emerald when streaming, amber while armed (waiting to anchor)
// or needing attention (the ESO Logs session expired).
function LiveDashboardBand({
  phase,
  routeDirect,
  officialInstalled,
  userName,
  sessionPersisted,
  startMs,
  fights,
  fightCount,
  report,
  handedOff,
  visibility,
  sessionAnchored,
  onCopyLink,
}: {
  phase: HeaderPhase;
  routeDirect: boolean;
  officialInstalled: boolean;
  userName: string | null;
  sessionPersisted: boolean;
  startMs: number | null;
  fights: LiveFight[];
  fightCount: number;
  report: ReportRef | null;
  handedOff: boolean;
  visibility: Visibility;
  sessionAnchored: boolean;
  onCopyLink: (url: string) => void | Promise<void>;
}) {
  const tone = phase === "live" ? "emerald" : "amber";
  const routeLabel = routeDirect
    ? "Direct to esologs.com"
    : officialInstalled
      ? "Via the official ESO Logs uploader"
      : "Via the ESO Logs uploader";
  return (
    <div
      className={cn(
        "mt-2.5 rounded-xl border p-3",
        tone === "emerald"
          ? "border-emerald-400/20 bg-gradient-to-b from-emerald-400/[0.06] to-emerald-400/[0.01]"
          : "border-amber-400/20 bg-gradient-to-b from-amber-400/[0.06] to-amber-400/[0.01]"
      )}
    >
      <div className="flex flex-wrap items-stretch gap-3">
        <LiveCore phase={phase} startMs={startMs} fightCount={fightCount} />
        <FightTicker
          fights={fights}
          fightCount={fightCount}
          armed={phase === "armed"}
          attention={phase === "attention"}
          handedOff={handedOff}
        />
        <LiveReportCTA
          report={report}
          handedOff={handedOff}
          visibility={visibility}
          sessionAnchored={sessionAnchored}
          onCopyLink={onCopyLink}
        />
      </div>
      <div className="mt-2.5 flex flex-wrap items-center gap-2 border-t border-white/[0.06] pt-2 text-[11px]">
        <span className="inline-flex items-center gap-1.5 text-muted-foreground">
          {routeDirect ? (
            <Zap className="size-3 text-accent-sky" aria-hidden />
          ) : (
            <CloudUpload className="size-3" aria-hidden />
          )}
          {routeLabel}
        </span>
        <span className="min-w-2 flex-1" aria-hidden />
        {userName && <AccountChip userName={userName} sessionPersisted={sessionPersisted} />}
      </div>
    </div>
  );
}

// The state core: the orb + phase word, a big counting timer, and the fight count.
function LiveCore({
  phase,
  startMs,
  fightCount,
}: {
  phase: HeaderPhase;
  startMs: number | null;
  fightCount: number;
}) {
  const tone = phase === "live" ? "emerald" : "amber";
  const label = phase === "attention" ? "ATTENTION" : phase === "armed" ? "ARMED" : "LIVE";
  return (
    <div
      className={cn(
        "flex min-w-[116px] flex-col justify-center gap-0.5 rounded-lg border px-3 py-2",
        tone === "emerald"
          ? "border-emerald-400/20 bg-emerald-400/[0.05]"
          : "border-amber-400/20 bg-amber-400/[0.05]"
      )}
    >
      <div className="flex items-center gap-1.5">
        <PulseDot tone={tone} />
        <span
          className={cn(
            "font-heading text-[10px] font-bold tracking-[0.08em]",
            tone === "emerald" ? "text-emerald-300/90" : "text-amber-300/90"
          )}
        >
          {label}
        </span>
      </div>
      {startMs !== null && (
        <SessionTimer
          startMs={startMs}
          className="text-xl leading-tight font-semibold text-foreground/95"
        />
      )}
      <span className="text-[11px] text-muted-foreground">
        {fightCount} fight{fightCount === 1 ? "" : "s"}
      </span>
    </div>
  );
}

// The fight ticker: the few most-recent fights (newest first), with the most
// recent emphasized, plus an "+N earlier" tail. Empty copy is phase-aware.
function FightTicker({
  fights,
  fightCount,
  armed,
  attention,
  handedOff,
}: {
  fights: LiveFight[];
  fightCount: number;
  armed: boolean;
  attention: boolean;
  handedOff: boolean;
}) {
  const recent = fights.slice(-3).reverse();
  const empty = attention
    ? "Paused — sign in to ESO Logs to resume posting."
    : armed
      ? "Waiting for a logging session to start…"
      : handedOff
        ? "The ESO Logs uploader is streaming this session."
        : "Streaming fights to ESO Logs as they finish…";
  return (
    <div className="flex min-w-[150px] flex-1 flex-col justify-center">
      <div className="font-heading text-[10px] font-bold tracking-[0.08em] text-muted-foreground/55 uppercase">
        Fights this session
      </div>
      {recent.length === 0 ? (
        <p className="mt-1 text-xs text-muted-foreground">{empty}</p>
      ) : (
        <ul className="mt-1 space-y-0.5" aria-label="Most recent fights">
          {recent.map((f, i) => (
            <li key={f.index} className="flex items-center justify-between gap-2 text-xs">
              <span className="flex min-w-0 items-center gap-1.5">
                <span className="text-muted-foreground/40" aria-hidden>
                  ▸
                </span>
                <span
                  className={cn("truncate", i === 0 ? "text-foreground/90" : "text-foreground/60")}
                >
                  {fightLabel(f)}
                </span>
              </span>
              <span className="shrink-0 tabular-nums text-muted-foreground/70">
                {formatDuration(f.durationMs)}
              </span>
            </li>
          ))}
        </ul>
      )}
      {fightCount > recent.length && (
        <div className="mt-0.5 text-[10px] text-muted-foreground/50">
          +{fightCount - recent.length} earlier
        </div>
      )}
    </div>
  );
}

// The report CTA: once the report code lands, a one-tap "Watch live"/"Open" +
// copy; before then, an honest placeholder so the slot doesn't pop in late.
function LiveReportCTA({
  report,
  handedOff,
  visibility,
  sessionAnchored,
  onCopyLink,
}: {
  report: ReportRef | null;
  handedOff: boolean;
  visibility: Visibility;
  sessionAnchored: boolean;
  onCopyLink: (url: string) => void | Promise<void>;
}) {
  if (!report) {
    return (
      <div className="flex min-w-[112px] flex-col items-center justify-center gap-1 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2 text-center">
        <CircleDashed className="size-4 text-muted-foreground/50" aria-hidden />
        <span className="text-[10px] leading-tight text-muted-foreground/70">
          {handedOff
            ? "Report opens in the uploader"
            : sessionAnchored
              ? "Reserving report…"
              : "Report opens when logging starts"}
        </span>
      </div>
    );
  }
  // Native sessions open raw ESO Logs with fight=last; a handed-off report isn't
  // streaming through Kalpa, so it just opens.
  const isNative = !handedOff;
  return (
    <div className="flex min-w-[124px] flex-col justify-center gap-1.5 rounded-lg border border-emerald-400/20 bg-emerald-400/[0.05] px-3 py-2">
      <span className="font-heading text-[10px] font-bold tracking-[0.08em] text-emerald-300/90 uppercase">
        Report ready
      </span>
      <span className="truncate text-xs text-foreground/85" title={report.code}>
        {report.code}
      </span>
      <div className="flex items-center gap-1">
        <Button
          size="sm"
          className="h-7 flex-1 gap-1.5 bg-emerald-500/15 text-emerald-200 hover:bg-emerald-500/25"
          onClick={() =>
            void openReportUrl(primaryReportUrl(report, visibility, { live: isNative }))
          }
          aria-label={
            visibility === "private"
              ? "Open private report on ESO Logs"
              : isNative
                ? "Watch live report"
                : "Open report"
          }
        >
          {visibility === "private" ? (
            <ExternalLink className="size-3" aria-hidden />
          ) : (
            <Zap className="size-3" aria-hidden />
          )}
          {visibility === "private" ? "ESO Logs" : isNative ? "Watch" : "Open"}
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          className="size-7"
          onClick={() => void onCopyLink(report.url)}
          aria-label="Copy report link"
        >
          <Copy className="size-3" />
        </Button>
      </div>
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
    // user previously turned direct upload OFF. Write them ATOMICALLY (one flush,
    // all-or-nothing) via the unified store batch — the Settings toggle uses the same
    // write — so a partial/failed write can't re-enable manual direct upload while LIVE
    // silently keeps handing off (the split-brain). Surface a failed write.
    const ok = await setSettings({
      manualUseOfficialUploader: false,
      liveUseOfficialUploader: false,
    });
    if (!ok) {
      toast.error("Couldn't enable direct upload — try again.");
      return;
    }
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

const Preflight = memo(function Preflight({
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

  // A PROMINENT split entry point — the old thin row was easy to miss. A bordered
  // card with an icon chip, a clear heading, the session/fight counts, and an
  // obvious CTA. It stays sky (or amber when the log is large enough to need
  // splitting) — never gold, which is reserved for the Upload climax below it.
  const sessionCount = preflight.sessions.length;
  const fightCount = preflight.totalFights;
  const urgent = preflight.recommendSplit;
  // Per-fight split needs the parsed fight list, which the backend omits for very
  // large logs — so don't promise it in the card when the workbench can't offer it.
  const perFightAvailable = preflight.fights.length > 0;
  const counts =
    sessionCount > 0
      ? `${sessionCount} session${sessionCount === 1 ? "" : "s"}` +
        (fightCount > 0 ? ` · ${fightCount} fight${fightCount === 1 ? "" : "s"}` : "")
      : "";
  // A peek at what's in the log, right on the card, so the user sees the fights
  // before opening the workbench. Only when the per-fight list was scanned.
  const fightPreview = perFightAvailable ? preflight.fights.slice(0, 4) : [];
  const moreFights = preflight.fights.length - fightPreview.length;
  return (
    <div
      className={cn(
        "rounded-xl border border-l-[3px] p-3 transition-colors",
        urgent
          ? "border-amber-400/25 border-l-amber-400 bg-amber-400/[0.05]"
          : "border-accent-sky/20 border-l-accent-sky/70 bg-accent-sky/[0.04]"
      )}
    >
      <div className="flex items-center gap-3">
        <span
          className={cn(
            "flex size-9 shrink-0 items-center justify-center rounded-lg",
            urgent ? "bg-amber-400/12 text-amber-300" : "bg-accent-sky/12 text-accent-sky"
          )}
        >
          <Scissors className="size-4" aria-hidden />
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold text-foreground/90">
            {urgent ? "This log is large — split it to upload" : "Split this log"}
          </div>
          <div className="mt-0.5 text-xs text-muted-foreground">
            {perFightAvailable
              ? "Carve it into per-session or per-fight files — upload a single fight or a whole night on its own."
              : "Carve it into per-session files so each uploads cleanly (per-fight split is available on smaller logs)."}
            {counts && <span className="text-muted-foreground/70"> {counts}.</span>}
          </div>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={onSplit}
          className={cn(
            "shrink-0",
            urgent
              ? "border-amber-400/40 bg-amber-400/[0.08] text-amber-200 hover:bg-amber-400/[0.16]"
              : "border-accent-sky/30 bg-accent-sky/[0.06] text-accent-sky hover:bg-accent-sky/[0.12]"
          )}
        >
          <Scissors className="size-3.5" />
          Split log…
        </Button>
      </div>

      {/* Fight peek — the first few fights with their durations, so the content is
          visible before opening the workbench. */}
      {fightPreview.length > 0 && (
        <div className="mt-2.5 flex flex-wrap items-center gap-1.5 border-t border-white/[0.06] pt-2.5">
          {fightPreview.map((f) => (
            <span
              key={f.index}
              className="inline-flex items-center gap-1.5 rounded-md border border-white/[0.08] bg-white/[0.02] px-2 py-0.5 text-[11px]"
            >
              <Swords className="size-2.5 shrink-0 text-primary/60" aria-hidden />
              <span className="max-w-[150px] truncate text-foreground/75">{fightLabel(f)}</span>
              <span className="tabular-nums text-muted-foreground/70">
                {formatDuration(f.endMs - f.startMs)}
              </span>
            </span>
          ))}
          {moreFights > 0 && (
            <span className="text-[11px] text-muted-foreground/60">+{moreFights} more</span>
          )}
        </div>
      )}
    </div>
  );
});

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
  visibility,
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
  visibility: Visibility;
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
  // Map live fights to display rows once per liveFights change, so the memoized
  // FightList sees a stable `fights` reference and can skip re-rendering when the
  // dashboard re-renders for an unrelated reason (e.g. a ticking timer).
  const liveRows = useMemo(() => rowsFromLive(liveFights), [liveFights]);
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
            {/* Completed analysis lives in the ESO Log Aggregator. Active native live
                sessions open raw ESO Logs so players/fight=last stay current. */}
            <Button
              size="sm"
              className="bg-emerald-500/15 text-emerald-300 hover:bg-emerald-500/25"
              onClick={() =>
                void openReportUrl(primaryReportUrl(liveReport, visibility, { live: isNative }))
              }
            >
              {visibility === "private" ? (
                <ExternalLink className="size-3.5" />
              ) : (
                <Zap className="size-3.5" />
              )}
              {visibility === "private" ? "Open report" : isNative ? "Watch live" : "View analysis"}
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

      {/* Pre-start expectation: completed fights already in this Encounter.log are
          not part of this live report. Native live can preserve an already-open fight
          once attached, but old completed pulls are intentionally left for manual
          upload. Set the expectation without taking destructive action. */}
      {!running && priorFightCount > 0 && (
        <div className="flex items-start gap-2 rounded-lg border border-white/[0.06] bg-white/[0.02] p-3 text-xs text-muted-foreground">
          <AlertCircle className="mt-0.5 size-3.5 shrink-0 text-foreground/50" aria-hidden />
          <span>
            Live skips the {priorFightCount} completed fight{priorFightCount === 1 ? "" : "s"}{" "}
            already in this log. If combat is active when live attaches, Kalpa keeps that open
            fight's context and streams it once it ends. Upload older completed fights with{" "}
            <span className="text-foreground/70">Upload a Log</span> if you want.
          </span>
        </div>
      )}

      <FightList
        fights={liveRows}
        newestFirst
        emptyHint={running ? "No fights yet this session." : "Start live logging to begin."}
      />
    </GlassPanel>
  );
}
