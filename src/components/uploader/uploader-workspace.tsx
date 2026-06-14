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
  const [liveFights, setLiveFights] = useState<LiveFight[]>([]);
  const [liveReport, setLiveReport] = useState<ReportRef | null>(null);
  const [liveStatus, setLiveStatus] = useState<UploaderStatus>("idle");

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

  const loadLogs = useCallback(async (dir: string) => {
    try {
      const files = await invokeOrThrow<LogFileInfo[]>("uploader_list_logs", { logsDir: dir });
      setLogs(files);
    } catch (e) {
      toast.error(`Couldn't list logs: ${getTauriErrorMessage(e)}`);
    }
  }, []);

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
      await refreshHistory();
    })();
    return () => {
      cancelled = true;
    };
  }, [loadLogs, refreshHistory]);

  // Stop any live session if the workspace unmounts.
  useEffect(() => {
    return () => {
      if (liveSessionId) {
        void invokeOrThrow("uploader_stop_live", { sessionId: liveSessionId }).catch(() => {});
      }
    };
  }, [liveSessionId]);

  const handlePickFolder = async () => {
    const picked = await openDialog({ directory: true, title: "Select your ESO Logs folder" });
    if (typeof picked === "string") {
      setLogsDir(picked);
      void loadLogs(picked);
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
      const written = await invokeOrThrow<string[]>("uploader_split_to_disk", {
        filePath: selectedLog,
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
    const sessionId = `live-${Date.now()}`;
    const channel = new Channel<LiveEvent>();
    setLiveFights([]);
    setLiveReport(null);
    setLiveStatus("watching");

    // The watcher emits UI-only fight-detection events; the actual upload is the
    // single whole-file handoff performed by uploader_start_live below.
    channel.onmessage = (ev) => {
      switch (ev.type) {
        case "started":
          setLiveStatus("watching");
          break;
        case "fightDetected": {
          const detected = ev;
          setLiveFights((prev) => {
            if (prev.some((f) => f.index === detected.index)) return prev;
            return [
              ...prev,
              {
                index: detected.index,
                zoneName: detected.zoneName,
                bossName: detected.bossName,
                durationMs: detected.durationMs,
              },
            ];
          });
          break;
        }
        case "sessionReset":
          toast.info("A new logging session started — continuing to watch.");
          setLiveFights([]);
          break;
        case "warning":
          // Non-fatal (e.g. transient read retry); surface quietly.
          break;
        case "stopped":
          setLiveStatus("idle");
          break;
      }
    };

    try {
      const dispatch = await invokeOrThrow<UploadDispatch>("uploader_start_live", {
        sessionId,
        filePath: selectedLog,
        options,
        preferCli: transport?.officialUploaderInstalled ?? false,
        channel,
      });
      setLiveSessionId(sessionId);
      if (dispatch?.report) setLiveReport(dispatch.report);
      toast.success(
        dispatch?.handedOff
          ? "Live logging started in the ESO Logs Uploader."
          : "Live logging started."
      );
    } catch (e) {
      setLiveStatus("attention");
      toast.error(`Couldn't start live logging: ${getTauriErrorMessage(e)}`);
    }
  };

  const handleStopLive = async () => {
    if (!liveSessionId) return;
    try {
      await invokeOrThrow("uploader_stop_live", { sessionId: liveSessionId });
    } catch {
      /* best-effort */
    }
    setLiveSessionId(null);
    setLiveStatus(liveFights.length > 0 ? "upToDate" : "idle");
    await refreshHistory();
  };

  const copyLink = (url: string) => {
    void navigator.clipboard.writeText(url);
    toast.success("Report link copied.");
  };

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
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
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
          <div className="space-y-4">
            <WhatGetsUploaded />

            {/* Mode tabs */}
            <div className="grid grid-cols-2 gap-2">
              <ModeTab
                active={mode === "manual"}
                onClick={() => setMode("manual")}
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
                canUpload={!!selectedLog && !uploading}
                uploading={uploading}
                transport={transport}
                onUpload={handleManualUpload}
              />
            ) : (
              <LiveDashboard
                running={liveSessionId !== null}
                canStart={!!selectedLog}
                liveFights={liveFights}
                liveReport={liveReport}
                onStart={handleStartLive}
                onStop={handleStopLive}
                onCopyLink={copyLink}
              />
            )}

            {/* History */}
            <HistoryPanel history={history} onCopyLink={copyLink} onRefresh={refreshHistory} />
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
  selectedLog,
  onSelect,
  onRefresh,
  onPickFolder,
}: {
  detection: LogPathDetection | null;
  logsDir: string | null;
  logs: LogFileInfo[];
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

      {logs.length === 0 ? (
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
  onSplit,
  splitting,
}: {
  preflight: LogPreflight | null;
  scanning: boolean;
  onSplit: () => void;
  splitting: boolean;
}) {
  if (scanning && !preflight) {
    return (
      <div className="flex items-center gap-2 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2 text-sm text-muted-foreground">
        <span className="size-3.5 animate-spin rounded-full border-2 border-white/[0.1] border-t-[#c4a44a]" />
        Scanning the log…
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
  return (
    <div className="mt-4 space-y-2 border-t border-white/[0.06] pt-4">
      <SectionHeader>Live Options</SectionHeader>
      <Toggle
        checked={options.includeEntireFile}
        disabled={disabled}
        onChange={(v) => onChange({ ...options, includeEntireFile: v })}
        label="Include earlier fights"
        hint="Upload fights already in the log, not just new ones."
      />
      <Toggle
        checked={options.realTime}
        disabled={disabled}
        onChange={(v) => onChange({ ...options, realTime: v })}
        label="Real-time streaming"
        hint="Stream events as they happen so spectators see fights live (uses more bandwidth)."
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
  canStart,
  liveFights,
  liveReport,
  onStart,
  onStop,
  onCopyLink,
}: {
  running: boolean;
  canStart: boolean;
  liveFights: LiveFight[];
  liveReport: ReportRef | null;
  onStart: () => void;
  onStop: () => void;
  onCopyLink: (url: string) => void;
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
          <Button size="sm" onClick={onStart} disabled={!canStart}>
            <Radio className="size-3.5" />
            Start live logging
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
              onClick={() => onCopyLink(liveReport.url)}
              aria-label="Copy link"
            >
              <Copy className="size-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => {
                void import("@tauri-apps/plugin-opener").then((m) => m.openUrl(liveReport.url));
              }}
              aria-label="Open report"
            >
              <ExternalLink className="size-3.5" />
            </Button>
          </div>
        </div>
      )}

      {running && (
        <div className="text-sm text-muted-foreground" role="status" aria-live="polite">
          {liveFights.length === 0
            ? "Watching for combat… start a fight in-game and it'll appear here."
            : `${liveFights.length} fight${liveFights.length === 1 ? "" : "s"} detected this session.`}
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
}: {
  history: UploadRecord[];
  onCopyLink: (url: string) => void;
  onRefresh: () => void;
}) {
  if (history.length === 0) return null;
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
            className="flex items-center justify-between gap-3 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2"
          >
            <div className="min-w-0">
              <div className="truncate text-sm text-foreground/90">{r.fileName}</div>
              <div className="text-xs text-muted-foreground">
                {relativeFromMs(r.createdAtMs)} · {r.fightCount} fight
                {r.fightCount === 1 ? "" : "s"} · {r.visibility}
              </div>
            </div>
            <div className="flex shrink-0 items-center gap-1.5">
              <StatusBadge status={r.status} />
              {r.report && (
                <Button
                  variant="ghost"
                  size="icon-sm"
                  onClick={() => onCopyLink(r.report!.url)}
                  aria-label="Copy report link"
                >
                  <Copy className="size-3.5" />
                </Button>
              )}
            </div>
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
