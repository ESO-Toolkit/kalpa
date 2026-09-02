import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  AlertCircleIcon,
  AlertTriangleIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  ExternalLinkIcon,
  HardDriveIcon,
  RefreshCwIcon,
  ScrollTextIcon,
  SearchIcon,
  ShieldAlertIcon,
  ShieldCheckIcon,
  Trash2Icon,
} from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { GlassPanel } from "@/components/ui/glass-panel";
import { SectionHeader } from "@/components/ui/section-header";
import { InfoPill } from "@/components/ui/info-pill";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { cn } from "@/lib/utils";

/* -------------------------------------------------------------------------- */
/* Data contract                                                              */
/* -------------------------------------------------------------------------- */
/* These mirror the Rust structs behind `detect_eso_clients`,                  */
/* `validate_eso_client` and `inspect_eso_client`. They are exported so they   */
/* can be re-homed into `types.ts` during integration without touching the     */
/* consuming code.                                                            */

export type ClientSource = "steam" | "zos_registry" | "proton" | "manual";

export interface EsoClientLocation {
  client_dir: string;
  exe_path: string;
  source: ClientSource;
}

export interface DllInfo {
  name: string;
  version: string | null;
}

export type HealthLevel = "ok" | "info" | "warning" | "danger";

export interface HealthFinding {
  id: string;
  level: HealthLevel;
  title: string;
  detail: string;
  guide_url: string | null;
}

export interface LogExcerpt {
  file: string;
  rule: string;
  line: string;
}

export interface ClientHealthReport {
  location: EsoClientLocation;
  injector: DllInfo | null;
  dlss: DllInfo | null;
  d3dcompiler: DllInfo | null;
  reshade_preset: string | null;
  findings: HealthFinding[];
  log_excerpts: LogExcerpt[];
}

export interface ClientHealthPanelProps {
  open: boolean;
  onClose: () => void;
}

/* -------------------------------------------------------------------------- */
/* Managed-files data contract                                               */
/* -------------------------------------------------------------------------- */
/* These mirror the Rust structs behind `list_managed_client_files`,           */
/* `uninstall_managed_client_files` and `emergency_remove_injector` in         */
/* `src-tauri/src/client_uninstall.rs`.                                        */

export type ManagedFileState = "present" | "modified" | "missing";

export type ManagedKind =
  | "re_shade_core"
  | "re_shade_config"
  | "shader"
  | "preset"
  | "addon"
  | "nvidia_runtime"
  | "shader_compiler";

export interface ManagedFileStatus {
  relative_path: string;
  kind: ManagedKind;
  placed_at: string;
  state: ManagedFileState;
  restores_backup: boolean;
}

export interface OrphanInjector {
  file_name: string;
  product_name: string;
  version: string | null;
}

export interface ManagedInventory {
  client_dir: string;
  files: ManagedFileStatus[];
  orphan_injectors: OrphanInjector[];
}

export interface UninstallOutcome {
  removed: string[];
  skipped: string[];
}

export interface EmergencyRemoval {
  file_name: string;
  quarantine_path: string;
}

/* -------------------------------------------------------------------------- */
/* Presentation tables                                                        */
/* -------------------------------------------------------------------------- */

const SOURCE_PILL: Record<
  ClientSource,
  { color: "sky" | "violet" | "amber" | "muted"; label: string }
> = {
  steam: { color: "sky", label: "Steam" },
  zos_registry: { color: "violet", label: "ZOS registry" },
  proton: { color: "amber", label: "Proton" },
  manual: { color: "muted", label: "Manual" },
};

/** Every level pairs its color with an icon *and* a word, so color is never the
 *  only signal (five built-in themes are light or high-contrast). */
const LEVEL_META: Record<
  HealthLevel,
  {
    label: string;
    Icon: typeof ShieldCheckIcon;
    text: string;
    border: string;
    tint: string;
  }
> = {
  ok: {
    label: "OK",
    Icon: ShieldCheckIcon,
    text: "text-status-success",
    border: "border-status-success/20 border-l-status-success",
    tint: "bg-status-success/[0.04]",
  },
  info: {
    label: "Info",
    Icon: AlertCircleIcon,
    text: "text-status-info",
    border: "border-status-info/20 border-l-status-info",
    tint: "bg-status-info/[0.04]",
  },
  warning: {
    label: "Warning",
    Icon: AlertTriangleIcon,
    text: "text-status-warning",
    border: "border-status-warning/20 border-l-status-warning",
    tint: "bg-status-warning/[0.04]",
  },
  danger: {
    label: "Problem",
    Icon: ShieldAlertIcon,
    text: "text-status-danger",
    border: "border-status-danger/20 border-l-status-danger",
    tint: "bg-status-danger/[0.04]",
  },
};

/** Order findings worst-first so the thing that needs attention is on top. */
const LEVEL_ORDER: Record<HealthLevel, number> = { danger: 0, warning: 1, info: 2, ok: 3 };

const KIND_LABEL: Record<ManagedKind, string> = {
  re_shade_core: "ReShade core",
  re_shade_config: "ReShade config",
  shader: "Shader",
  preset: "Preset",
  addon: "ReShade add-on",
  nvidia_runtime: "NVIDIA runtime",
  shader_compiler: "Shader compiler",
};

/** Same color+icon+word discipline as `LEVEL_META`, for the managed-file
 *  state instead of the health-finding level. */
const FILE_STATE_META: Record<
  ManagedFileState,
  { label: string; Icon: typeof ShieldCheckIcon; text: string; hint: string }
> = {
  present: {
    label: "Present",
    Icon: ShieldCheckIcon,
    text: "text-status-success",
    hint: "Unchanged since Kalpa wrote it. Safe to remove.",
  },
  modified: {
    label: "Modified",
    Icon: AlertTriangleIcon,
    text: "text-status-warning",
    hint: "Changed since Kalpa wrote it. Kalpa will not delete this — that protects your edits.",
  },
  missing: {
    label: "Missing",
    Icon: AlertCircleIcon,
    text: "text-status-info",
    hint: "Already gone. Removing just tidies the record and restores anything it displaced.",
  },
};

const Spinner = ({ className }: { className?: string }) => (
  <span
    aria-hidden
    className={cn(
      "inline-block size-4 animate-spin rounded-full border-2 border-structure-10 border-t-primary",
      className
    )}
  />
);

const Divider = () => <div className="border-t border-structure-06" />;

function shortDirName(dir: string): string {
  const parts = dir.split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1]! : dir;
}

/** Open a guide URL through the Tauri opener plugin — never `window.open` or a
 *  bare anchor, both of which would navigate the webview itself. */
async function openGuide(url: string): Promise<void> {
  try {
    const m = await import("@tauri-apps/plugin-opener");
    await m.openUrl(url);
  } catch {
    // An opener-scope rejection must not take down the panel; the URL is still
    // rendered as text next to the button so it can be copied by hand.
  }
}

/* -------------------------------------------------------------------------- */
/* Panel                                                                      */
/* -------------------------------------------------------------------------- */

function ClientHealthPanel({ open, onClose }: ClientHealthPanelProps) {
  const [clients, setClients] = useState<EsoClientLocation[] | null>(null);
  const [selectedDir, setSelectedDir] = useState<string | null>(null);
  const [report, setReport] = useState<ClientHealthReport | null>(null);

  const [detecting, setDetecting] = useState(false);
  const [inspecting, setInspecting] = useState(false);
  const [browsing, setBrowsing] = useState(false);

  const [detectError, setDetectError] = useState<string | null>(null);
  const [inspectError, setInspectError] = useState<string | null>(null);
  const [browseError, setBrowseError] = useState<string | null>(null);

  const [logsExpanded, setLogsExpanded] = useState(false);

  // Managed-by-Kalpa inventory for the selected install.
  const [managedInventory, setManagedInventory] = useState<ManagedInventory | null>(null);
  const [managedLoading, setManagedLoading] = useState(false);
  const [managedError, setManagedError] = useState<string | null>(null);

  // Selection + destructive-confirm state for managed uninstall. All of this
  // is per-install: it is reset every time the selected install changes or a
  // refresh runs, so a stale "are you sure" latch can never survive a content
  // swap onto a different folder.
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [removeMode, setRemoveMode] = useState<"selected" | "all" | null>(null);
  const [removing, setRemoving] = useState(false);
  const [removeOutcome, setRemoveOutcome] = useState<UninstallOutcome | null>(null);
  const [removeError, setRemoveError] = useState<string | null>(null);

  // Emergency removal: collapsed disclosure + typed-confirmation state, also
  // reset on every install switch/refresh.
  const [emergencyOpen, setEmergencyOpen] = useState(false);
  const [emergencyTarget, setEmergencyTarget] = useState<string | null>(null);
  const [emergencyConfirmInput, setEmergencyConfirmInput] = useState("");
  const [emergencyBusy, setEmergencyBusy] = useState(false);
  const [emergencyError, setEmergencyError] = useState<string | null>(null);
  const [emergencyResult, setEmergencyResult] = useState<EmergencyRemoval | null>(null);

  // Monotonic token: a slow detect/inspect that resolves after the user has
  // already refreshed or picked a different install must not overwrite newer
  // state. Bumped by every load entry point.
  const runToken = useRef(0);

  /** Clears every per-install control latch (selection, confirm state,
   *  emergency disclosure and its typed input, and the last outcome/error).
   *  Called at every point the selected install changes or is reloaded, so
   *  none of it can survive onto a different folder. */
  const resetManagedUiState = useCallback(() => {
    setSelectedPaths(new Set());
    setRemoveMode(null);
    setRemoving(false);
    setRemoveOutcome(null);
    setRemoveError(null);
    setEmergencyOpen(false);
    setEmergencyTarget(null);
    setEmergencyConfirmInput("");
    setEmergencyBusy(false);
    setEmergencyError(null);
    setEmergencyResult(null);
  }, []);

  const inspect = useCallback(async (clientDir: string, token: number) => {
    setInspecting(true);
    setInspectError(null);
    try {
      const next = await invokeOrThrow<ClientHealthReport>("inspect_eso_client", { clientDir });
      if (runToken.current !== token) return;
      setReport(next);
    } catch (e) {
      if (runToken.current !== token) return;
      setReport(null);
      setInspectError(getTauriErrorMessage(e));
    } finally {
      if (runToken.current === token) setInspecting(false);
    }
  }, []);

  const loadManaged = useCallback(async (clientDir: string, token: number) => {
    setManagedLoading(true);
    setManagedError(null);
    try {
      const next = await invokeOrThrow<ManagedInventory>("list_managed_client_files", {
        clientDir,
      });
      if (runToken.current !== token) return;
      setManagedInventory(next);
    } catch (e) {
      if (runToken.current !== token) return;
      setManagedInventory(null);
      setManagedError(getTauriErrorMessage(e));
    } finally {
      if (runToken.current === token) setManagedLoading(false);
    }
  }, []);

  const detect = useCallback(async () => {
    const token = ++runToken.current;
    setDetecting(true);
    setDetectError(null);
    setBrowseError(null);
    setInspectError(null);
    setReport(null);
    setManagedInventory(null);
    setManagedError(null);
    resetManagedUiState();
    try {
      const found = await invokeOrThrow<EsoClientLocation[]>("detect_eso_clients");
      if (runToken.current !== token) return;
      setClients(found);
      const first = found[0];
      setSelectedDir(first ? first.client_dir : null);
      setDetecting(false);
      if (first)
        await Promise.all([inspect(first.client_dir, token), loadManaged(first.client_dir, token)]);
    } catch (e) {
      if (runToken.current !== token) return;
      setClients([]);
      setSelectedDir(null);
      setDetectError(getTauriErrorMessage(e));
      setDetecting(false);
    }
  }, [inspect, loadManaged, resetManagedUiState]);

  // On-open fetch only. Project policy forbids background polling, so there is
  // no interval here — the Refresh button is the only other trigger.
  useEffect(() => {
    if (!open) return;
    // `detect` flips the loading flags before its first await, which the rule
    // reads as a synchronous setState. That is the intended behaviour for an
    // on-open fetch (the spinner has to appear on the same commit the panel
    // opens), and it matches the existing `ApiCompat` dialog.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void detect();
  }, [open, detect]);

  const handleSelect = useCallback(
    (clientDir: string) => {
      if (clientDir === selectedDir) return;
      const token = ++runToken.current;
      setSelectedDir(clientDir);
      setReport(null);
      setManagedInventory(null);
      setManagedError(null);
      resetManagedUiState();
      void inspect(clientDir, token);
      void loadManaged(clientDir, token);
    },
    [inspect, loadManaged, resetManagedUiState, selectedDir]
  );

  const handleBrowse = useCallback(async () => {
    setBrowsing(true);
    setBrowseError(null);
    try {
      const picked = await openFileDialog({
        directory: false,
        multiple: false,
        title: "Locate eso64.exe",
        filters: [{ name: "ESO client", extensions: ["exe"] }],
      });
      if (typeof picked !== "string") return;
      const token = ++runToken.current;
      const validated = await invokeOrThrow<EsoClientLocation>("validate_eso_client", {
        path: picked,
      });
      if (runToken.current !== token) return;
      setClients((prev) => {
        const rest = (prev ?? []).filter((c) => c.client_dir !== validated.client_dir);
        return [validated, ...rest];
      });
      setSelectedDir(validated.client_dir);
      setReport(null);
      setDetectError(null);
      setManagedInventory(null);
      setManagedError(null);
      resetManagedUiState();
      await Promise.all([
        inspect(validated.client_dir, token),
        loadManaged(validated.client_dir, token),
      ]);
    } catch (e) {
      setBrowseError(getTauriErrorMessage(e));
    } finally {
      setBrowsing(false);
    }
  }, [inspect, loadManaged, resetManagedUiState]);

  const sortedFindings = useMemo(() => {
    if (!report) return [];
    return [...report.findings].sort(
      (a, b) => LEVEL_ORDER[a.level] - LEVEL_ORDER[b.level] || a.title.localeCompare(b.title)
    );
  }, [report]);

  const dlls = useMemo(() => {
    if (!report) return [];
    return [
      { role: "Injector", info: report.injector },
      { role: "DLSS", info: report.dlss },
      { role: "D3D compiler", info: report.d3dcompiler },
    ];
  }, [report]);

  const handleToggleSelect = useCallback((relativePath: string) => {
    setSelectedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(relativePath)) next.delete(relativePath);
      else next.add(relativePath);
      return next;
    });
    // Disarm the confirm step. `pendingPaths` is derived live from the
    // selection, so leaving the confirm block open across a selection change
    // would let a user arm it for one set of files and confirm it for a
    // different one -- the prompt only ever states a count, so swapping one
    // file for another is invisible. Consent is for the set that was on
    // screen when it was given; change the set and it has to be given again.
    setRemoveMode(null);
  }, []);

  const handleRequestRemove = useCallback((mode: "selected" | "all") => {
    setRemoveError(null);
    setRemoveOutcome(null);
    setRemoveMode(mode);
  }, []);

  const handleCancelRemove = useCallback(() => {
    setRemoveMode(null);
  }, []);

  const handleConfirmRemove = useCallback(
    async (paths: string[]) => {
      if (!selectedDir || paths.length === 0) return;
      const token = runToken.current;
      setRemoving(true);
      setRemoveError(null);
      try {
        const outcome = await invokeOrThrow<UninstallOutcome>("uninstall_managed_client_files", {
          clientDir: selectedDir,
          relativePaths: paths,
        });
        if (runToken.current !== token) return;
        setRemoveOutcome(outcome);
        setRemoveMode(null);
        setSelectedPaths(new Set());
        await loadManaged(selectedDir, token);
      } catch (e) {
        if (runToken.current !== token) return;
        setRemoveError(getTauriErrorMessage(e));
      } finally {
        if (runToken.current === token) setRemoving(false);
      }
    },
    [loadManaged, selectedDir]
  );

  const handleEmergencyRemove = useCallback(
    async (fileName: string) => {
      if (!selectedDir) return;
      const token = runToken.current;
      setEmergencyBusy(true);
      setEmergencyError(null);
      try {
        const result = await invokeOrThrow<EmergencyRemoval>("emergency_remove_injector", {
          clientDir: selectedDir,
          fileName,
          confirmation: emergencyConfirmInput,
        });
        if (runToken.current !== token) return;
        setEmergencyResult(result);
        setEmergencyTarget(null);
        setEmergencyConfirmInput("");
        await loadManaged(selectedDir, token);
      } catch (e) {
        if (runToken.current !== token) return;
        setEmergencyError(getTauriErrorMessage(e));
      } finally {
        if (runToken.current === token) setEmergencyBusy(false);
      }
    },
    [emergencyConfirmInput, loadManaged, selectedDir]
  );

  const busy = detecting || inspecting;

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="sm:max-w-xl h-[74vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>Client Health</DialogTitle>
          <DialogDescription>
            A report on your ESO client folder. Kalpa downloads nothing here — it can only remove
            files it placed itself, restoring whatever they displaced.
          </DialogDescription>
        </DialogHeader>

        <div className="flex-1 min-h-0 space-y-4 overflow-y-auto pr-1">
          {detecting && (
            <div className="flex items-center gap-3 py-10 text-sm text-muted-foreground">
              <Spinner />
              <span>Looking for ESO installs...</span>
            </div>
          )}

          {!detecting && (clients?.length ?? 0) === 0 && (
            <EmptyState
              detectError={detectError}
              browseError={browseError}
              browsing={browsing}
              onBrowse={() => void handleBrowse()}
            />
          )}

          {!detecting && clients && clients.length > 0 && (
            <>
              <section aria-labelledby="client-health-install">
                <SectionHeader id="client-health-install" className="mb-2">
                  {clients.length > 1 ? `Installs (${clients.length})` : "Install"}
                </SectionHeader>
                <div className="space-y-2">
                  {clients.map((client) => (
                    <InstallRow
                      key={client.client_dir}
                      client={client}
                      selected={client.client_dir === selectedDir}
                      selectable={clients.length > 1}
                      onSelect={() => handleSelect(client.client_dir)}
                    />
                  ))}
                </div>
                <div className="mt-2 flex flex-wrap items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={browsing}
                    onClick={() => void handleBrowse()}
                  >
                    {browsing ? <Spinner className="size-3.5" /> : <SearchIcon />}
                    {browsing ? "Browsing..." : "Browse for eso64.exe"}
                  </Button>
                  {browseError && (
                    <p className="text-xs text-status-danger" role="alert">
                      {browseError}
                    </p>
                  )}
                </div>
              </section>

              <Divider />

              {inspecting && (
                <div className="flex items-center gap-3 py-10 text-sm text-muted-foreground">
                  <Spinner />
                  <span>Inspecting client folder...</span>
                </div>
              )}

              {!inspecting && inspectError && (
                <GlassPanel
                  variant="subtle"
                  className="flex items-start gap-2 p-3 text-sm"
                  role="alert"
                >
                  <ShieldAlertIcon
                    aria-hidden
                    className="mt-0.5 size-4 shrink-0 text-status-danger"
                  />
                  <div>
                    <p className="font-heading text-[13px] font-semibold text-status-danger">
                      Inspection failed
                    </p>
                    <p className="mt-0.5 text-xs text-muted-foreground">{inspectError}</p>
                  </div>
                </GlassPanel>
              )}

              {!inspecting && !inspectError && report && (
                <ReportBody
                  report={report}
                  dlls={dlls}
                  findings={sortedFindings}
                  logsExpanded={logsExpanded}
                  onToggleLogs={() => setLogsExpanded((v) => !v)}
                />
              )}

              {!inspecting && selectedDir && (
                <>
                  <Divider />
                  <ManagedSection
                    loading={managedLoading}
                    error={managedError}
                    inventory={managedInventory}
                    selectedPaths={selectedPaths}
                    onToggleSelect={handleToggleSelect}
                    removeMode={removeMode}
                    removing={removing}
                    removeOutcome={removeOutcome}
                    removeError={removeError}
                    onRequestRemove={handleRequestRemove}
                    onCancelRemove={handleCancelRemove}
                    onConfirmRemove={handleConfirmRemove}
                    emergencyOpen={emergencyOpen}
                    onToggleEmergencyOpen={() => setEmergencyOpen((v) => !v)}
                    emergencyTarget={emergencyTarget}
                    onSetEmergencyTarget={(name) => {
                      setEmergencyTarget(name);
                      setEmergencyConfirmInput("");
                      setEmergencyError(null);
                    }}
                    emergencyConfirmInput={emergencyConfirmInput}
                    onEmergencyConfirmInputChange={setEmergencyConfirmInput}
                    emergencyBusy={emergencyBusy}
                    emergencyError={emergencyError}
                    emergencyResult={emergencyResult}
                    onEmergencyRemove={handleEmergencyRemove}
                  />
                </>
              )}
            </>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Close
          </Button>
          <Button variant="outline" disabled={busy} onClick={() => void detect()}>
            {busy ? <Spinner className="size-3.5" /> : <RefreshCwIcon />}
            Refresh
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/* -------------------------------------------------------------------------- */
/* Sub-views                                                                  */
/* -------------------------------------------------------------------------- */

function EmptyState({
  detectError,
  browseError,
  browsing,
  onBrowse,
}: {
  detectError: string | null;
  browseError: string | null;
  browsing: boolean;
  onBrowse: () => void;
}) {
  return (
    <GlassPanel
      variant="subtle"
      className="flex flex-col items-center gap-3 px-4 py-10 text-center"
    >
      <HardDriveIcon aria-hidden className="size-6 text-muted-foreground/70" />
      <div className="space-y-1">
        <p className="font-heading text-sm font-semibold">No ESO install found</p>
        <p className="mx-auto max-w-sm text-xs leading-relaxed text-muted-foreground">
          Kalpa checked Steam, the ZOS registry entry and known Proton prefixes. Point it at your
          game executable and it will report on that folder instead.
        </p>
      </div>
      {detectError && (
        <p className="text-xs text-status-warning" role="status">
          Detection reported: {detectError}
        </p>
      )}
      <Button variant="outline" size="sm" disabled={browsing} onClick={onBrowse}>
        {browsing ? <Spinner className="size-3.5" /> : <SearchIcon />}
        {browsing ? "Browsing..." : "Browse for eso64.exe"}
      </Button>
      {browseError && (
        <p className="max-w-sm text-xs text-status-danger" role="alert">
          {browseError}
        </p>
      )}
    </GlassPanel>
  );
}

function InstallRow({
  client,
  selected,
  selectable,
  onSelect,
}: {
  client: EsoClientLocation;
  selected: boolean;
  selectable: boolean;
  onSelect: () => void;
}) {
  const pill = SOURCE_PILL[client.source];
  const body = (
    <>
      <div className="min-w-0 flex-1 text-left">
        <p className="truncate font-heading text-[13px] font-semibold">
          {shortDirName(client.client_dir)}
        </p>
        <p className="truncate text-xs text-muted-foreground" title={client.exe_path}>
          {client.exe_path}
        </p>
      </div>
      <InfoPill color={pill.color}>{pill.label}</InfoPill>
    </>
  );

  if (!selectable) {
    return (
      <GlassPanel variant="subtle" className="flex items-center gap-3 p-3">
        {body}
      </GlassPanel>
    );
  }

  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onSelect}
      className={cn(
        "flex w-full items-center gap-3 rounded-xl border p-3 text-left transition-colors duration-150",
        "focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-accent-sky/20",
        selected
          ? "border-primary/30 border-l-[3px] border-l-primary bg-primary/[0.04]"
          : "border-structure-06 bg-structure-02 hover:border-structure-10"
      )}
    >
      {body}
    </button>
  );
}

function ReportBody({
  report,
  dlls,
  findings,
  logsExpanded,
  onToggleLogs,
}: {
  report: ClientHealthReport;
  dlls: { role: string; info: DllInfo | null }[];
  findings: HealthFinding[];
  logsExpanded: boolean;
  onToggleLogs: () => void;
}) {
  return (
    <div className="space-y-4">
      <section aria-labelledby="client-health-dlls">
        <SectionHeader id="client-health-dlls" className="mb-2">
          Detected files
        </SectionHeader>
        <GlassPanel variant="subtle" className="p-0">
          {dlls.map(({ role, info }) => (
            <div
              key={role}
              className="flex items-center justify-between gap-3 border-b border-structure-06 px-3 py-2"
            >
              <span className="text-xs text-muted-foreground">{role}</span>
              {info ? (
                <span className="min-w-0 truncate text-right font-sans text-[13px]">
                  <span className="font-medium">{info.name}</span>
                  <span className="text-muted-foreground">
                    {" "}
                    &middot; {info.version ?? "unknown version"}
                  </span>
                </span>
              ) : (
                <span className="text-[13px] text-muted-foreground">Not present</span>
              )}
            </div>
          ))}
          <div className="flex items-center justify-between gap-3 px-3 py-2">
            <span className="text-xs text-muted-foreground">ReShade preset</span>
            <span
              className="min-w-0 truncate text-right text-[13px]"
              title={report.reshade_preset ?? undefined}
            >
              {report.reshade_preset ?? <span className="text-muted-foreground">Not present</span>}
            </span>
          </div>
        </GlassPanel>
      </section>

      <section aria-labelledby="client-health-findings">
        <SectionHeader id="client-health-findings" className="mb-2">
          Findings ({findings.length})
        </SectionHeader>
        {findings.length === 0 ? (
          <GlassPanel variant="subtle" className="p-3 text-xs text-muted-foreground">
            Nothing to report for this install.
          </GlassPanel>
        ) : (
          <ul className="space-y-2">
            {findings.map((finding) => (
              <FindingRow key={finding.id} finding={finding} />
            ))}
          </ul>
        )}
      </section>

      <section aria-labelledby="client-health-logs">
        <SectionHeader id="client-health-logs" className="mb-2">
          Log excerpts
        </SectionHeader>
        <GlassPanel variant="subtle" className="overflow-hidden p-0">
          <button
            type="button"
            aria-expanded={logsExpanded}
            aria-controls="client-health-log-list"
            onClick={onToggleLogs}
            className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors duration-150 hover:bg-structure-04 focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-accent-sky/20"
          >
            {logsExpanded ? (
              <ChevronDownIcon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
            ) : (
              <ChevronRightIcon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
            )}
            <ScrollTextIcon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
            <span className="text-[13px]">
              {report.log_excerpts.length === 0
                ? "No matching log lines"
                : `${report.log_excerpts.length} matching log ${
                    report.log_excerpts.length === 1 ? "line" : "lines"
                  }`}
            </span>
          </button>
          {logsExpanded && (
            <div id="client-health-log-list" className="border-t border-structure-06">
              {report.log_excerpts.length === 0 ? (
                <p className="px-3 py-2 text-xs text-muted-foreground">
                  Kalpa found nothing worth quoting in the client logs.
                </p>
              ) : (
                <ul>
                  {report.log_excerpts.map((excerpt, i) => (
                    <li
                      key={`${excerpt.file}-${excerpt.rule}-${i}`}
                      className="border-b border-structure-06 px-3 py-2 last:border-b-0"
                    >
                      <div className="flex flex-wrap items-center gap-2">
                        <InfoPill color="muted">{excerpt.rule}</InfoPill>
                        <span
                          className="truncate text-xs text-muted-foreground"
                          title={excerpt.file}
                        >
                          {excerpt.file}
                        </span>
                      </div>
                      <pre className="mt-1 overflow-x-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-muted-foreground">
                        {excerpt.line}
                      </pre>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </GlassPanel>
      </section>
    </div>
  );
}

function FindingRow({ finding }: { finding: HealthFinding }) {
  const meta = LEVEL_META[finding.level];
  const { Icon } = meta;
  const guideUrl = finding.guide_url;
  return (
    <li
      className={cn(
        "rounded-xl border border-l-[3px] p-3 transition-colors duration-150",
        meta.border,
        meta.tint
      )}
    >
      <div className="flex items-start gap-2">
        <Icon aria-hidden className={cn("mt-0.5 size-4 shrink-0", meta.text)} />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h4 className="font-heading text-[13px] font-semibold">{finding.title}</h4>
            {/* The level word is rendered as text, not encoded in the color
                alone — the icon and this label carry it on light and
                high-contrast themes too. */}
            <span className={cn("text-[11px] font-semibold uppercase tracking-wide", meta.text)}>
              {meta.label}
            </span>
          </div>
          <p className="mt-1 text-xs leading-relaxed text-muted-foreground">{finding.detail}</p>
          {guideUrl && (
            <Button
              variant="link"
              size="xs"
              className="mt-1 h-auto px-0"
              onClick={() => void openGuide(guideUrl)}
            >
              Read the guide
              <ExternalLinkIcon />
            </Button>
          )}
        </div>
      </div>
    </li>
  );
}

/* -------------------------------------------------------------------------- */
/* Managed by Kalpa                                                          */
/* -------------------------------------------------------------------------- */

function ManagedSection({
  loading,
  error,
  inventory,
  selectedPaths,
  onToggleSelect,
  removeMode,
  removing,
  removeOutcome,
  removeError,
  onRequestRemove,
  onCancelRemove,
  onConfirmRemove,
  emergencyOpen,
  onToggleEmergencyOpen,
  emergencyTarget,
  onSetEmergencyTarget,
  emergencyConfirmInput,
  onEmergencyConfirmInputChange,
  emergencyBusy,
  emergencyError,
  emergencyResult,
  onEmergencyRemove,
}: {
  loading: boolean;
  error: string | null;
  inventory: ManagedInventory | null;
  selectedPaths: Set<string>;
  onToggleSelect: (relativePath: string) => void;
  removeMode: "selected" | "all" | null;
  removing: boolean;
  removeOutcome: UninstallOutcome | null;
  removeError: string | null;
  onRequestRemove: (mode: "selected" | "all") => void;
  onCancelRemove: () => void;
  onConfirmRemove: (paths: string[]) => void | Promise<void>;
  emergencyOpen: boolean;
  onToggleEmergencyOpen: () => void;
  emergencyTarget: string | null;
  onSetEmergencyTarget: (fileName: string | null) => void;
  emergencyConfirmInput: string;
  onEmergencyConfirmInputChange: (value: string) => void;
  emergencyBusy: boolean;
  emergencyError: string | null;
  emergencyResult: EmergencyRemoval | null;
  onEmergencyRemove: (fileName: string) => void | Promise<void>;
}) {
  const files = useMemo(() => inventory?.files ?? [], [inventory]);
  const orphans = inventory?.orphan_injectors ?? [];

  const pendingPaths = useMemo(() => {
    if (removeMode === "all") return files.map((f) => f.relative_path);
    if (removeMode === "selected") return Array.from(selectedPaths);
    return [];
  }, [files, removeMode, selectedPaths]);

  return (
    <section aria-labelledby="client-health-managed">
      <SectionHeader id="client-health-managed" className="mb-2">
        Managed by Kalpa
      </SectionHeader>

      {loading && (
        <div className="flex items-center gap-3 py-6 text-sm text-muted-foreground" role="status">
          <Spinner />
          <span>Checking files Kalpa has placed here...</span>
        </div>
      )}

      {!loading && error && (
        <GlassPanel variant="subtle" className="flex items-start gap-2 p-3 text-sm" role="alert">
          <ShieldAlertIcon aria-hidden className="mt-0.5 size-4 shrink-0 text-status-danger" />
          <div>
            <p className="font-heading text-[13px] font-semibold text-status-danger">
              Could not check managed files
            </p>
            <p className="mt-0.5 text-xs text-muted-foreground">{error}</p>
          </div>
        </GlassPanel>
      )}

      {!loading && !error && inventory && files.length === 0 && orphans.length === 0 && (
        <GlassPanel variant="subtle" className="p-3 text-xs text-muted-foreground">
          Kalpa has not placed anything in this folder.
        </GlassPanel>
      )}

      {!loading && !error && inventory && files.length > 0 && (
        <div className="space-y-2">
          <ul className="space-y-2">
            {files.map((file) => (
              <ManagedFileRow
                key={file.relative_path}
                file={file}
                selected={selectedPaths.has(file.relative_path)}
                onToggle={() => onToggleSelect(file.relative_path)}
              />
            ))}
          </ul>

          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={selectedPaths.size === 0 || removing}
              onClick={() => onRequestRemove("selected")}
            >
              <Trash2Icon />
              Remove selected ({selectedPaths.size})
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={removing}
              onClick={() => onRequestRemove("all")}
            >
              <Trash2Icon />
              Remove all ({files.length})
            </Button>
          </div>

          {removeMode && (
            <GlassPanel
              variant="subtle"
              className="flex flex-col gap-2 border-status-danger/20 p-3 text-xs"
            >
              <p className="text-muted-foreground">
                Remove {pendingPaths.length} file{pendingPaths.length === 1 ? "" : "s"}? Kalpa will
                skip anything modified since it wrote it, and say why.
              </p>
              <div className="flex items-center gap-2">
                <Button
                  size="sm"
                  variant="destructive"
                  disabled={removing || pendingPaths.length === 0}
                  onClick={() => void onConfirmRemove(pendingPaths)}
                >
                  {removing ? <Spinner className="size-3.5" /> : <Trash2Icon />}
                  {removing ? "Removing..." : "Confirm remove"}
                </Button>
                <Button size="sm" variant="outline" disabled={removing} onClick={onCancelRemove}>
                  Cancel
                </Button>
              </div>
            </GlassPanel>
          )}

          {removeError && (
            <p className="text-xs text-status-danger" role="alert">
              {removeError}
            </p>
          )}

          {removeOutcome && (
            <GlassPanel variant="subtle" className="space-y-1 p-3 text-xs" role="status">
              <p className="text-status-success">
                Removed {removeOutcome.removed.length} file
                {removeOutcome.removed.length === 1 ? "" : "s"}.
              </p>
              {removeOutcome.skipped.length > 0 && (
                <p className="text-muted-foreground">
                  Skipped {removeOutcome.skipped.length} file
                  {removeOutcome.skipped.length === 1 ? "" : "s"}: modified since Kalpa wrote them,
                  or already accounted for. Left in place, not a failure.
                </p>
              )}
            </GlassPanel>
          )}
        </div>
      )}

      {!loading && !error && orphans.length > 0 && (
        <GlassPanel variant="subtle" className="mt-3 overflow-hidden p-0">
          <button
            type="button"
            aria-expanded={emergencyOpen}
            aria-controls="client-health-emergency-list"
            onClick={onToggleEmergencyOpen}
            className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors duration-150 hover:bg-structure-04 focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-accent-sky/20"
          >
            {emergencyOpen ? (
              <ChevronDownIcon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
            ) : (
              <ChevronRightIcon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
            )}
            <ShieldAlertIcon aria-hidden className="size-4 shrink-0 text-status-danger" />
            <span className="text-[13px]">
              Emergency removal ({orphans.length} unmanaged{" "}
              {orphans.length === 1 ? "file" : "files"})
            </span>
          </button>
          {emergencyOpen && (
            <div
              id="client-health-emergency-list"
              className="space-y-3 border-t border-structure-06 p-3"
            >
              <p className="text-xs leading-relaxed text-muted-foreground">
                Kalpa has no record of these — the manifest is gone, or they were placed by hand
                before Kalpa existed. Steam&apos;s Verify integrity and the ZOS launcher&apos;s
                Repair only check files they shipped, so neither removes a foreign injector. Kalpa
                only lists a file here when its version information positively identifies it as
                ReShade.
              </p>
              <ul className="space-y-2">
                {orphans.map((injector) => (
                  <OrphanInjectorRow
                    key={injector.file_name}
                    injector={injector}
                    active={emergencyTarget === injector.file_name}
                    confirmInput={emergencyConfirmInput}
                    onSetTarget={onSetEmergencyTarget}
                    onInputChange={onEmergencyConfirmInputChange}
                    busy={emergencyBusy}
                    error={emergencyError}
                    result={emergencyResult}
                    onRemove={onEmergencyRemove}
                  />
                ))}
              </ul>
            </div>
          )}
        </GlassPanel>
      )}
    </section>
  );
}

function ManagedFileRow({
  file,
  selected,
  onToggle,
}: {
  file: ManagedFileStatus;
  selected: boolean;
  onToggle: () => void;
}) {
  const meta = FILE_STATE_META[file.state];
  const { Icon } = meta;
  return (
    <li
      className={cn(
        "flex items-start gap-3 rounded-xl border p-3 transition-colors duration-150",
        selected ? "border-primary/30 bg-primary/[0.04]" : "border-structure-06 bg-structure-02"
      )}
    >
      <Checkbox
        checked={selected}
        onCheckedChange={() => onToggle()}
        aria-label={`Select ${file.relative_path}`}
        className="mt-0.5"
      />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="truncate font-mono text-[12px]" title={file.relative_path}>
            {file.relative_path}
          </span>
          <InfoPill color="muted">{KIND_LABEL[file.kind]}</InfoPill>
        </div>
        <div className="mt-1 flex items-center gap-1.5">
          <Icon aria-hidden className={cn("size-3.5 shrink-0", meta.text)} />
          <span className={cn("text-[11px] font-semibold uppercase tracking-wide", meta.text)}>
            {meta.label}
          </span>
        </div>
        <p className="mt-1 text-xs leading-relaxed text-muted-foreground">{meta.hint}</p>
        {file.restores_backup && (
          <p className="mt-1 text-xs text-status-info">Restores your original file when removed.</p>
        )}
      </div>
    </li>
  );
}

function OrphanInjectorRow({
  injector,
  active,
  confirmInput,
  onSetTarget,
  onInputChange,
  busy,
  error,
  result,
  onRemove,
}: {
  injector: OrphanInjector;
  active: boolean;
  confirmInput: string;
  onSetTarget: (fileName: string | null) => void;
  onInputChange: (value: string) => void;
  busy: boolean;
  error: string | null;
  result: EmergencyRemoval | null;
  onRemove: (fileName: string) => void | Promise<void>;
}) {
  const matches = confirmInput === injector.file_name;
  const succeeded = result?.file_name === injector.file_name;
  return (
    <li className="rounded-xl border border-l-[3px] border-status-danger/20 border-l-status-danger bg-status-danger/[0.04] p-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="font-heading text-[13px] font-semibold">{injector.file_name}</p>
          <p className="text-xs text-muted-foreground">
            Matched: {injector.product_name}
            {injector.version ? ` · v${injector.version}` : ""}
          </p>
        </div>
        {!active && !succeeded && (
          <Button size="sm" variant="destructive" onClick={() => onSetTarget(injector.file_name)}>
            <Trash2Icon />
            Remove
          </Button>
        )}
      </div>

      {active && (
        <div className="mt-2 space-y-2">
          <p className="text-xs leading-relaxed text-muted-foreground">
            This moves the file to quarantine — it is not deleted. Type{" "}
            <span className="font-mono text-foreground">{injector.file_name}</span> to confirm.
          </p>
          <div className="flex flex-wrap items-center gap-2">
            <Input
              value={confirmInput}
              onChange={(e) => onInputChange(e.target.value)}
              placeholder={injector.file_name}
              aria-label={`Type ${injector.file_name} to confirm removal`}
              className="max-w-[220px]"
            />
            <Button
              size="sm"
              variant="destructive"
              disabled={!matches || busy}
              onClick={() => void onRemove(injector.file_name)}
            >
              {busy ? <Spinner className="size-3.5" /> : <Trash2Icon />}
              {busy ? "Quarantining..." : "Quarantine"}
            </Button>
            <Button size="sm" variant="outline" disabled={busy} onClick={() => onSetTarget(null)}>
              Cancel
            </Button>
          </div>
          {error && (
            <p className="text-xs text-status-danger" role="alert">
              {error}
            </p>
          )}
        </div>
      )}

      {succeeded && result && (
        <p className="mt-2 text-xs text-status-success" role="status">
          Moved to quarantine: <span className="font-mono">{result.quarantine_path}</span>
        </p>
      )}
    </li>
  );
}

export default ClientHealthPanel;
export { ClientHealthPanel };
