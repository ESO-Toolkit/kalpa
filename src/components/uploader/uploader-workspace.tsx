// The ESO Logs uploader workspace. Full-screen dialog with a glanceable status
// pill, Manual / Live mode tabs, an auto-detected log picker with preflight, a
// per-fight timeline, split-to-disk for oversized logs, upload history, and a
// first-run wizard. Uploads are handed to the official ESO Logs uploader (Kalpa
// never speaks the private upload protocol itself).

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import type { AuthUser } from "@/types";
import {
  REGION_OPTIONS,
  type FightSummary,
  type LiveEvent,
  type LiveFight,
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
import { StatusPill, WhatGetsUploaded, compactBytes, relativeFromMs } from "./uploader-shared";
import { UploadOptionsControl } from "./upload-options";
import { FightList, rowsFromLive, rowsFromSummaries } from "./fight-list";

interface UploaderWorkspaceProps {
  authUser: AuthUser | null;
  onClose: () => void;
  onOpenSettings: () => void;
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

export function UploaderWorkspace({ authUser, onClose, onOpenSettings }: UploaderWorkspaceProps) {
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
  const [splitting, setSplitting] = useState(false);

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
  const [liveReport, setLiveReport] = useState<ReportRef | null>(null);
  const [liveStatus, setLiveStatus] = useState<UploaderStatus>("idle");
  const [starting, setStarting] = useState(false);
  // Synchronous re-entry guard for start-live (state updates lag a frame).
  const startingRef = useRef(false);
  // Holds the in-flight live session id from before the start await resolves, so
  // unmounting mid-await still stops the backend watcher (state hasn't landed).
  const liveSessionIdRef = useRef<string | null>(null);
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

  // Stop any live session when the workspace unmounts. Reads the ref (set
  // before the start await) so a session started but not yet reflected in state
  // is still torn down. Empty deps: this must run only on final unmount.
  useEffect(() => {
    return () => {
      liveActiveRef.current = false; // drop any late channel events
      const id = liveSessionIdRef.current;
      if (id) {
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
    } catch (e) {
      if (selectTokenRef.current !== token) return;
      toast.error(`Couldn't read that log: ${getTauriErrorMessage(e)}`);
    } finally {
      if (selectTokenRef.current === token) setScanning(false);
    }
  }, []);

  const handleManualUpload = async () => {
    if (!selectedLog) return;
    setUploading(true);
    try {
      const dispatch = await invokeOrThrow<UploadDispatch>("uploader_upload_log", {
        filePath: selectedLog,
        options,
        preferCli: transport?.officialUploaderInstalled ?? false,
        // Reuse the preflight's count so the backend doesn't re-scan a multi-GB
        // log just to fill the history record.
        fightCount: preflight?.totalFights ?? null,
      });
      if (dispatch.report) {
        toast.success("Upload complete — report ready.");
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

  const handleSplit = async () => {
    if (!selectedLog) return;
    setSplitting(true);
    try {
      // Split files go to an app-owned folder (the destination isn't
      // caller-controlled); we reveal the result so the user can find them.
      // Pass the preflight's sessions so the backend doesn't re-scan the whole
      // multi-GB file (re-scanning only as a fallback if we have none).
      const written = await invokeOrThrow<string[]>("uploader_split_to_disk", {
        filePath: selectedLog,
        sessions: preflight?.sessions ?? null,
      });
      toast.success(
        `Split into ${written.length} session file${written.length === 1 ? "" : "s"}.`,
        { duration: 7000 }
      );
      try {
        const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
        if (written[0]) await revealItemInDir(written[0]);
      } catch {
        /* reveal is best-effort */
      }
    } catch (e) {
      toast.error(`Split failed: ${getTauriErrorMessage(e)}`);
    } finally {
      setSplitting(false);
    }
  };

  const handleStartLive = async () => {
    if (!selectedLog) {
      toast.error("Pick the active Encounter.log first.");
      return;
    }
    // Guard re-entry SYNCHRONOUSLY via a ref: `starting`/`liveSessionId` state
    // doesn't update until the next render, so two clicks in one frame would
    // both pass a state-only check and start two backend watchers (orphaning
    // one). The ref flips immediately.
    if (startingRef.current || liveSessionId) return;
    startingRef.current = true;
    setStarting(true);
    const sessionId = `live-${Date.now()}`;
    // Record the id before the await so unmount cleanup can stop the backend
    // watcher even if the dialog closes before the await resolves.
    liveSessionIdRef.current = sessionId;
    const channel = new Channel<LiveEvent>();
    setLiveFights([]);
    setLiveFightCount(0);
    liveFightCountRef.current = 0;
    setLiveReport(null);
    setLiveStatus("watching");
    liveActiveRef.current = true;

    // The watcher emits UI-only fight-detection events; the actual upload is the
    // single whole-file handoff performed by uploader_start_live below.
    channel.onmessage = (ev) => {
      // Drop events that arrive after the session was stopped or the dialog
      // closed (the backend keeps emitting for up to one poll interval).
      if (!liveActiveRef.current) return;
      switch (ev.type) {
        case "started":
          setLiveStatus("watching");
          break;
        case "fightDetected": {
          const detected = ev;
          setLiveFights((prev) => {
            if (prev.some((f) => f.index === detected.index)) return prev;
            // Bump the truthful total only when this is a genuinely new fight
            // (not a re-delivered duplicate within the window).
            setLiveFightCount((c) => {
              liveFightCountRef.current = c + 1;
              return c + 1;
            });
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
          break;
        case "fightSkipped":
          // A genuinely oversized fight; surface once. The full log still uploads.
          toast.info(ev.reason);
          break;
        case "warning":
          // Transient (e.g. a read retry) — log but don't toast, as these recur.
          console.warn("[uploader] live watcher:", ev.message);
          break;
        case "stopped": {
          // A `stopped` event while we still consider the session active means
          // the watcher thread died on its own (lost folder access, couldn't
          // keep reading the log, etc.) — a user-initiated stop already cleared
          // liveActiveRef before this could run. Beyond tearing down the UI, the
          // backend still holds the now-dead `Running` slot and the history
          // record is still `Live`, and nothing else settles them until the
          // next-launch reconcile. Drive the existing stop path so the slot is
          // evicted and the record settled immediately. Capture the id + count
          // FIRST, before we null the ref, or the invoke would have nothing to
          // settle (the ref is set before the start await, so it holds the id
          // even if `liveSessionId` state hasn't landed yet).
          const stoppedId = liveSessionIdRef.current;
          const stoppedFightCount = liveFightCountRef.current;
          liveActiveRef.current = false;
          liveSessionIdRef.current = null;
          setLiveSessionId(null);
          setLiveStatus("attention");
          if (ev.reason && !/stopped\.?$/i.test(ev.reason)) toast.error(ev.reason);
          if (stoppedId) {
            // Best-effort: evicts the dead `Running` slot (stop_slot_in_map) and
            // settles the `Live` record to `Completed` (settle_live). Both are
            // idempotent, so this is safe even if the record was already settled.
            void invokeOrThrow("uploader_stop_live", {
              sessionId: stoppedId,
              fightCount: stoppedFightCount,
            }).catch(() => {});
          }
          break;
        }
      }
    };

    try {
      const dispatch = await invokeOrThrow<UploadDispatch>("uploader_start_live", {
        sessionId,
        filePath: selectedLog,
        options,
        channel,
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
      setLiveSessionId(sessionId);
      if (dispatch?.report) setLiveReport(dispatch.report);
      toast.success(
        dispatch?.handedOff
          ? "Live logging started in the ESO Logs Uploader."
          : "Live logging started."
      );
    } catch (e) {
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
    try {
      await invokeOrThrow("uploader_stop_live", {
        sessionId: id,
        fightCount: liveFightCountRef.current,
      });
    } catch {
      /* best-effort */
    }
    liveActiveRef.current = false; // stop processing any trailing events
    liveSessionIdRef.current = null;
    setLiveSessionId(null);
    setLiveStatus(liveFightCount > 0 ? "upToDate" : "idle");
    await refreshHistory();
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

  // The headline status pill reflects live state if a session is running,
  // otherwise manual upload state.
  const headlineStatus: UploaderStatus = useMemo(() => {
    if (liveSessionId) return liveStatus;
    if (uploading) return "uploading";
    return "idle";
  }, [liveSessionId, liveStatus, uploading]);

  const isLoggedIn = authUser !== null;

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      {/* Cap height to the viewport and lay the dialog out as a flex column so the
          header stays pinned while the body scrolls. Without this the shared
          DialogContent (overflow-hidden, no max-height, vertically centered)
          lets tall content spill off the top and bottom of the screen with no
          way to reach it. */}
      <DialogContent className="flex max-h-[90vh] flex-col gap-0 overflow-hidden sm:max-w-2xl">
        <DialogHeader className="shrink-0">
          <div className="flex items-center justify-between gap-3">
            <DialogTitle className="flex items-center gap-2">
              <CloudUpload className="size-5 text-[#c4a44a]" aria-hidden />
              Upload to ESO Logs
            </DialogTitle>
            <StatusPill status={headlineStatus} />
          </div>
          <DialogDescription>
            Send your combat logs to esologs.com — analyze your raids, compare parses, and share
            reports with your group.
          </DialogDescription>
        </DialogHeader>

        {!isLoggedIn ? (
          <LoggedOut onOpenSettings={onOpenSettings} />
        ) : (
          // -mr-2 pr-2 keeps the scrollbar off the content's right edge; pt-4
          // restores the gap the header used to provide via the grid.
          <div className="-mr-2 space-y-4 overflow-y-auto pr-2 pt-4">
            <WhatGetsUploaded />

            {/* Mode tabs */}
            <div className="grid grid-cols-2 gap-2">
              <ModeTab
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
                onClick={() => setMode("live")}
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
              onSelect={handleSelectLog}
              onRefresh={() => logsDir && loadLogs(logsDir)}
              onPickFolder={handlePickFolder}
            />

            {/* Preflight + fights */}
            {selectedLog && (
              <Preflight
                preflight={preflight}
                scanning={scanning}
                scanningSizeBytes={logs.find((l) => l.path === selectedLog)?.sizeBytes ?? null}
                onSplit={handleSplit}
                splitting={splitting}
              />
            )}

            {selectedLog &&
              (mode === "live" ? null : (
                <GlassPanel variant="subtle" className="p-3">
                  <SectionHeader className="mb-2">Fights</SectionHeader>
                  <FightList
                    fights={rowsFromSummaries(fights)}
                    emptyHint={scanning ? "Scanning the log…" : "No fights found in this log yet."}
                  />
                </GlassPanel>
              ))}

            {/* Upload options */}
            {selectedLog && (
              <GlassPanel variant="subtle" className="p-4">
                <UploadOptionsControl
                  options={options}
                  onChange={setOptions}
                  disabled={uploading || liveSessionId !== null}
                />
                {mode === "live" && (
                  <LiveToggles
                    options={options}
                    onChange={setOptions}
                    disabled={liveSessionId !== null}
                  />
                )}
              </GlassPanel>
            )}

            {/* Action area */}
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
                onUpload={handleManualUpload}
              />
            ) : (
              <LiveDashboard
                running={liveSessionId !== null}
                starting={starting}
                canStart={!!selectedLog}
                liveFights={liveFights}
                liveFightCount={liveFightCount}
                liveReport={liveReport}
                onStart={handleStartLive}
                onStop={handleStopLive}
                onCopyLink={copyLink}
              />
            )}

            {/* History */}
            <HistoryPanel
              history={history}
              onCopyLink={copyLink}
              onRefresh={refreshHistory}
              onAttachReport={handleAttachReport}
            />
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

// ── Sub-components ───────────────────────────────────────────────────────────

function LoggedOut({ onOpenSettings }: { onOpenSettings: () => void }) {
  return (
    <div className="flex flex-col items-center gap-4 py-8 text-center">
      <div className="flex size-14 items-center justify-center rounded-full bg-[#c4a44a]/10">
        <CloudUpload className="size-7 text-[#c4a44a]" aria-hidden />
      </div>
      <div>
        <div className="text-base font-medium">Sign in to ESO Logs</div>
        <p className="mx-auto mt-1 max-w-sm text-sm text-muted-foreground">
          Connect your ESO Logs account to upload your logs. It's the same sign-in Kalpa uses for
          Pack Hub — no extra password needed.
        </p>
      </div>
      <Button onClick={onOpenSettings}>Open Settings to sign in</Button>
    </div>
  );
}

function ModeTab({
  active,
  onClick,
  Icon,
  title,
  hint,
}: {
  active: boolean;
  onClick: () => void;
  Icon: typeof Upload;
  title: string;
  hint: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "rounded-xl border p-3 text-left transition-colors duration-150",
        active
          ? "border-sky-400/40 bg-sky-400/[0.06]"
          : "border-white/[0.06] bg-white/[0.02] hover:border-white/[0.12]"
      )}
    >
      <div
        className={cn(
          "flex items-center gap-1.5 text-sm font-medium",
          active ? "text-sky-400" : "text-foreground/80"
        )}
      >
        <Icon className="size-4" aria-hidden />
        {title}
      </div>
      <div className="mt-0.5 text-xs text-muted-foreground">{hint}</div>
    </button>
  );
}

function LogPicker({
  detection,
  logsDir,
  logs,
  listError,
  selectedLog,
  onSelect,
  onRefresh,
  onPickFolder,
}: {
  detection: LogPathDetection | null;
  logsDir: string | null;
  logs: LogFileInfo[];
  listError: string | null;
  selectedLog: string | null;
  onSelect: (path: string) => void;
  onRefresh: () => void;
  onPickFolder: () => void;
}) {
  return (
    <GlassPanel variant="subtle" className="p-3">
      <div className="mb-2 flex items-center justify-between gap-2">
        <SectionHeader>Logs Folder</SectionHeader>
        <div className="flex gap-1">
          <Button variant="ghost" size="icon-sm" onClick={onRefresh} aria-label="Refresh logs">
            <RefreshCw className="size-3.5" />
          </Button>
          <Button variant="ghost" size="icon-sm" onClick={onPickFolder} aria-label="Choose folder">
            <FolderSearch className="size-3.5" />
          </Button>
        </div>
      </div>

      {logsDir ? (
        <div className="mb-2 truncate text-xs text-muted-foreground" title={logsDir}>
          {logsDir}
        </div>
      ) : (
        <div className="mb-2 text-xs text-amber-400/90">{detection?.message}</div>
      )}

      {listError ? (
        <div className="rounded-lg border border-dashed border-red-400/30 bg-red-400/[0.04] p-4 text-center text-sm text-red-300/90">
          Couldn't read this folder — check it's accessible and try Refresh.
          <div className="mt-1 text-xs text-muted-foreground">{listError}</div>
        </div>
      ) : logs.length === 0 ? (
        <div className="rounded-lg border border-dashed border-white/[0.08] p-4 text-center text-sm text-muted-foreground">
          {detection && !detection.encounterLogExists
            ? "No Encounter.log yet. Type /encounterlog in chat (or use a logging addon) to start recording."
            : "No log files found in this folder."}
        </div>
      ) : (
        <ul className="max-h-44 space-y-1 overflow-y-auto" aria-label="Log files">
          {logs.map((log) => (
            <li key={log.path}>
              <button
                type="button"
                onClick={() => onSelect(log.path)}
                className={cn(
                  "flex w-full items-center justify-between gap-3 rounded-lg border px-3 py-2 text-left transition-colors duration-150",
                  selectedLog === log.path
                    ? "border-sky-400/40 bg-sky-400/[0.06]"
                    : "border-white/[0.06] bg-white/[0.02] hover:border-white/[0.12]"
                )}
                aria-pressed={selectedLog === log.path}
              >
                <div className="flex min-w-0 items-center gap-2">
                  <FileText className="size-4 shrink-0 text-muted-foreground" aria-hidden />
                  <div className="min-w-0">
                    <div className="truncate text-sm text-foreground/90">{log.fileName}</div>
                    <div className="text-xs text-muted-foreground">
                      {compactBytes(log.sizeBytes)} · {relativeFromMs(log.modifiedAtMs)}
                    </div>
                  </div>
                </div>
                {log.isActive && (
                  <InfoPill color="sky" className="shrink-0 gap-1">
                    <Radio className="size-3 animate-pulse" aria-hidden /> Active
                  </InfoPill>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
    </GlassPanel>
  );
}

function Preflight({
  preflight,
  scanning,
  scanningSizeBytes,
  onSplit,
  splitting,
}: {
  preflight: LogPreflight | null;
  scanning: boolean;
  scanningSizeBytes: number | null;
  onSplit: () => void;
  splitting: boolean;
}) {
  if (scanning && !preflight) {
    // Surface the known file size so a long scan of a multi-GB log reads as
    // expected work, not a hang.
    const sizeHint = scanningSizeBytes ? ` (${compactBytes(scanningSizeBytes)})` : "";
    const big = (scanningSizeBytes ?? 0) > 256 * 1024 * 1024;
    return (
      <div className="flex items-center gap-2 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2 text-sm text-muted-foreground">
        <span className="size-3.5 animate-spin rounded-full border-2 border-white/[0.1] border-t-[#c4a44a]" />
        Scanning the log{sizeHint}…{big ? " this may take a moment." : ""}
      </div>
    );
  }
  if (!preflight) return null;

  return (
    <div className="flex flex-wrap items-center gap-2 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2 text-sm">
      <InfoPill color="muted">{compactBytes(preflight.sizeBytes)}</InfoPill>
      <InfoPill color="gold">
        {preflight.totalFights} fight{preflight.totalFights === 1 ? "" : "s"}
      </InfoPill>
      <InfoPill color="muted">
        {preflight.sessions.length} session{preflight.sessions.length === 1 ? "" : "s"}
      </InfoPill>
      {preflight.recommendSplit && (
        <div className="ml-auto flex items-center gap-2">
          <span className="text-xs text-amber-400/90">This log is large — splitting helps.</span>
          <Button variant="outline" size="sm" onClick={onSplit} disabled={splitting}>
            <Scissors className="size-3.5" />
            {splitting ? "Splitting…" : "Split by session"}
          </Button>
        </div>
      )}
      {!preflight.recommendSplit && (
        <Button
          variant="ghost"
          size="sm"
          onClick={onSplit}
          disabled={splitting}
          className="ml-auto text-muted-foreground"
        >
          <Scissors className="size-3.5" />
          Split to disk
        </Button>
      )}
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
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className="flex w-full items-center justify-between gap-3 rounded-lg px-1 py-1.5 text-left disabled:opacity-50"
    >
      <span>
        <span className="block text-sm text-foreground/90">{label}</span>
        <span className="block text-xs text-muted-foreground">{hint}</span>
      </span>
      <span
        className={cn(
          "relative h-5 w-9 shrink-0 rounded-full transition-colors duration-200",
          checked ? "bg-sky-400/70" : "bg-white/[0.1]"
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
  onUpload,
}: {
  canUpload: boolean;
  uploading: boolean;
  transport: TransportInfo | null;
  onUpload: () => void;
}) {
  const installed = transport?.officialUploaderInstalled ?? false;
  return (
    <div className="flex items-center justify-between gap-3">
      <p className="text-xs text-muted-foreground">
        {installed
          ? "Uploads run through the official ESO Logs Uploader installed on your PC."
          : "We'll open the ESO Logs Uploader (or its download page) with your prepared log."}
      </p>
      <Button onClick={onUpload} disabled={!canUpload} className="shrink-0">
        <CloudUpload className="size-4" />
        {uploading ? "Preparing…" : installed ? "Upload to ESO Logs" : "Open in ESO Logs Uploader"}
      </Button>
    </div>
  );
}

function LiveDashboard({
  running,
  starting,
  canStart,
  liveFights,
  liveFightCount,
  liveReport,
  onStart,
  onStop,
  onCopyLink,
}: {
  running: boolean;
  starting: boolean;
  canStart: boolean;
  liveFights: LiveFight[];
  liveFightCount: number;
  liveReport: ReportRef | null;
  onStart: () => void;
  onStop: () => void;
  onCopyLink: (url: string) => void | Promise<void>;
}) {
  return (
    <GlassPanel variant="primary" className="space-y-3 p-4">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <SectionHeader>Live Session</SectionHeader>
          {running && (
            <InfoPill color="red" className="gap-1">
              <Radio className="size-3 animate-pulse" aria-hidden /> LIVE
            </InfoPill>
          )}
        </div>
        {running ? (
          <Button variant="outline" size="sm" onClick={onStop}>
            Stop
          </Button>
        ) : (
          <Button size="sm" onClick={onStart} disabled={!canStart || starting}>
            <Radio className="size-3.5" />
            {starting ? "Starting…" : "Start live logging"}
          </Button>
        )}
      </div>

      {running && (
        <p className="text-xs text-muted-foreground">
          The ESO Logs Uploader is streaming this log in real time. Fights appear below as they
          finish; leave it running for the rest of your session.
        </p>
      )}

      {liveReport && (
        <div className="flex items-center justify-between gap-2 rounded-lg border border-emerald-400/20 bg-emerald-400/[0.04] px-3 py-2">
          <span className="truncate text-sm text-emerald-300/90">
            Report ready: <span className="text-foreground/80">{liveReport.code}</span>
          </span>
          <div className="flex shrink-0 gap-1">
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => void onCopyLink(liveReport.url)}
              aria-label="Copy link"
            >
              <Copy className="size-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => void openReportUrl(liveReport.url)}
              aria-label="Open report"
            >
              <ExternalLink className="size-3.5" />
            </Button>
          </div>
        </div>
      )}

      {running && (
        <div className="text-sm text-muted-foreground" role="status" aria-live="polite">
          {liveFightCount === 0
            ? "Watching for combat… start a fight in-game and it'll appear here."
            : `${liveFightCount} fight${liveFightCount === 1 ? "" : "s"} detected this session.` +
              (liveFightCount > liveFights.length
                ? ` Showing the latest ${liveFights.length}.`
                : "")}
        </div>
      )}

      <FightList
        fights={rowsFromLive(liveFights)}
        emptyHint={running ? "No fights yet this session." : "Start live logging to begin."}
      />
    </GlassPanel>
  );
}

function HistoryPanel({
  history,
  onCopyLink,
  onRefresh,
  onAttachReport,
}: {
  history: UploadRecord[];
  onCopyLink: (url: string) => void | Promise<void>;
  onRefresh: () => void;
  onAttachReport: (id: string, url: string) => Promise<void>;
}) {
  const [attachingId, setAttachingId] = useState<string | null>(null);
  const [linkDraft, setLinkDraft] = useState("");

  if (history.length === 0) return null;

  const submitLink = async (id: string) => {
    const url = linkDraft.trim();
    if (!url) return;
    await onAttachReport(id, url);
    setAttachingId(null);
    setLinkDraft("");
  };

  return (
    <GlassPanel variant="subtle" className="p-3">
      <div className="mb-2 flex items-center justify-between">
        <SectionHeader>Recent Uploads</SectionHeader>
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={() => void onRefresh()}
          aria-label="Refresh history"
        >
          <RefreshCw className="size-3.5" />
        </Button>
      </div>
      <ul className="space-y-1">
        {history.slice(0, 8).map((r) => (
          <li
            key={r.id}
            className="rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2"
          >
            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="truncate text-sm text-foreground/90">{r.fileName}</div>
                <div className="text-xs text-muted-foreground">
                  {relativeFromMs(r.createdAtMs)} · {r.fightCount} fight
                  {r.fightCount === 1 ? "" : "s"} · {r.visibility}
                </div>
              </div>
              <div className="flex shrink-0 items-center gap-1.5">
                <StatusBadge status={r.status} />
                {r.report ? (
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => void onCopyLink(r.report!.url)}
                    aria-label="Copy report link"
                  >
                    <Copy className="size-3.5" />
                  </Button>
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
              </div>
            </div>
            {attachingId === r.id && (
              <div className="mt-2 flex items-center gap-2">
                <Input
                  value={linkDraft}
                  onChange={(e) => setLinkDraft(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void submitLink(r.id);
                    if (e.key === "Escape") setAttachingId(null);
                  }}
                  placeholder="https://www.esologs.com/reports/…"
                  aria-label="ESO Logs report link"
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
    </GlassPanel>
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
    case "failed":
      return <InfoPill color="red">Failed</InfoPill>;
    default:
      return <InfoPill color="muted">{status}</InfoPill>;
  }
}
