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

  // Monotonic token: a slow detect/inspect that resolves after the user has
  // already refreshed or picked a different install must not overwrite newer
  // state. Bumped by every load entry point.
  const runToken = useRef(0);

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

  const detect = useCallback(async () => {
    const token = ++runToken.current;
    setDetecting(true);
    setDetectError(null);
    setBrowseError(null);
    setInspectError(null);
    setReport(null);
    try {
      const found = await invokeOrThrow<EsoClientLocation[]>("detect_eso_clients");
      if (runToken.current !== token) return;
      setClients(found);
      const first = found[0];
      setSelectedDir(first ? first.client_dir : null);
      setDetecting(false);
      if (first) await inspect(first.client_dir, token);
    } catch (e) {
      if (runToken.current !== token) return;
      setClients([]);
      setSelectedDir(null);
      setDetectError(getTauriErrorMessage(e));
      setDetecting(false);
    }
  }, [inspect]);

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
      void inspect(clientDir, token);
    },
    [inspect, selectedDir]
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
      await inspect(validated.client_dir, token);
    } catch (e) {
      setBrowseError(getTauriErrorMessage(e));
    } finally {
      setBrowsing(false);
    }
  }, [inspect]);

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

  const busy = detecting || inspecting;

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="sm:max-w-xl h-[74vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>Client Health</DialogTitle>
          <DialogDescription>
            A read-only report on your ESO client folder. Kalpa never installs, downloads or changes
            anything here — it only tells you what it found.
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

export default ClientHealthPanel;
export { ClientHealthPanel };
