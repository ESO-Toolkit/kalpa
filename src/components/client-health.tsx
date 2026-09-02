import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  AlertCircleIcon,
  AlertTriangleIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  ExternalLinkIcon,
  HardDriveIcon,
  ScrollTextIcon,
  PackageCheckIcon,
  RefreshCwIcon,
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
/* Data contract — install discovery                                         */
/* -------------------------------------------------------------------------- */
/* These mirror the Rust structs behind `detect_eso_clients` and                */
/* `validate_eso_client`.                                                      */

export type ClientSource = "steam" | "zos_registry" | "proton" | "manual";

export interface EsoClientLocation {
  client_dir: string;
  exe_path: string;
  source: ClientSource;
}

export type HealthLevel = "ok" | "info" | "warning" | "danger";

export interface HealthFinding {
  id: string;
  level: HealthLevel;
  title: string;
  detail: string;
  guide_url: string | null;
}

export interface ClientHealthPanelProps {
  open: boolean;
  onClose: () => void;
}

/* -------------------------------------------------------------------------- */
/* Data contract — the stack                                                 */
/* -------------------------------------------------------------------------- */
/* These mirror `src-tauri/src/client_stack.rs`. A DLSS 5 Neural Rendering     */
/* setup is a pipeline of layers, not three DLLs — see that file's module      */
/* doc for why the cross-layer findings below exist.                          */

export type StackRole =
  | "injector"
  | "neural_rendering"
  | "super_sampling"
  | "frame_generation"
  | "shader_compiler"
  | "addon"
  | "companion";

export interface StackItem {
  role: StackRole;
  file_name: string;
  display_name: string | null;
  version: string | null;
  company: string | null;
  description: string | null;
  size_bytes: number;
}

export interface PreservedOriginal {
  file_name: string;
  backs_up: string | null;
  version: string | null;
  size_bytes: number;
}

export interface Technique {
  name: string;
  source: string;
  source_present: boolean;
}

export interface PresetInfo {
  path: string;
  exists: boolean;
  techniques: Technique[];
  available: string[];
}

export interface TuningValue {
  key: string;
  value: string;
}

export interface ShaderTree {
  present: boolean;
  effect_count: number;
  texture_count: number;
  effect_search_paths: string | null;
}

export interface ClientStack {
  client_dir: string;
  items: StackItem[];
  preserved_originals: PreservedOriginal[];
  shaders: ShaderTree;
  preset: PresetInfo | null;
  tuning: TuningValue[];
  disabled_addons: string[];
  is_empty: boolean;
  findings: HealthFinding[];
}

/* -------------------------------------------------------------------------- */
/* Data contract — adoption                                                  */
/* -------------------------------------------------------------------------- */
/* These mirror `src-tauri/src/client_adopt.rs`.                               */

export interface AdoptionEntry {
  relative_path: string;
  kind: ManagedKind;
  role: StackRole;
  display_name: string | null;
  version: string | null;
  size_bytes: number;
  displaced_in_place: string | null;
  copyable: boolean;
}

export interface AdoptionPlan {
  client_dir: string;
  entries: AdoptionEntry[];
  copy_bytes: number;
  already_managed: boolean;
  is_empty: boolean;
}

export interface AdoptionOutcome {
  recorded: string[];
  copied: string[];
  skipped: string[];
}

/* -------------------------------------------------------------------------- */
/* Data contract — Kalpa's own records                                       */
/* -------------------------------------------------------------------------- */
/* These mirror `list_managed_client_files`, `uninstall_managed_client_files`  */
/* and `emergency_remove_injector` in `src-tauri/src/client_uninstall.rs`.     */

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
    line: string;
  }
> = {
  ok: {
    label: "OK",
    Icon: ShieldCheckIcon,
    text: "text-status-success",
    border: "border-status-success/20 border-l-status-success",
    tint: "bg-status-success/[0.04]",
    line: "border-status-success",
  },
  info: {
    label: "Info",
    Icon: AlertCircleIcon,
    text: "text-status-info",
    border: "border-status-info/20 border-l-status-info",
    tint: "bg-status-info/[0.04]",
    line: "border-status-info",
  },
  warning: {
    label: "Warning",
    Icon: AlertTriangleIcon,
    text: "text-status-warning",
    border: "border-status-warning/20 border-l-status-warning",
    tint: "bg-status-warning/[0.04]",
    line: "border-status-warning",
  },
  danger: {
    label: "Problem",
    Icon: ShieldAlertIcon,
    text: "text-status-danger",
    border: "border-status-danger/20 border-l-status-danger",
    tint: "bg-status-danger/[0.04]",
    line: "border-status-danger",
  },
};

/** Worst-first ordering for picking a representative level across findings. */
const LEVEL_ORDER: Record<HealthLevel, number> = { danger: 0, warning: 1, info: 2, ok: 3 };

function worstLevel(levels: HealthLevel[]): HealthLevel {
  return levels.reduce((worst, level) => (LEVEL_ORDER[level] < LEVEL_ORDER[worst] ? level : worst));
}

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

/* -------------------------------------------------------------------------- */
/* The pipeline                                                               */
/* -------------------------------------------------------------------------- */
/* Fixed load order — never reorder these by severity, the pipeline shape is   */
/* the information. Roles that are not their own named layer (companion,       */
/* frame_generation) fold into the nearest layer they physically ship next to. */

type Stage = "injector" | "nr" | "sr" | "addons" | "shaders" | "compiler" | "preset" | "tuning";

const STAGE_ORDER: Stage[] = [
  "injector",
  "nr",
  "sr",
  "addons",
  "shaders",
  "compiler",
  "preset",
  "tuning",
];

const STAGE_LABEL: Record<Stage, string> = {
  injector: "Injector",
  nr: "Neural Rendering runtime",
  sr: "Super Resolution runtime",
  addons: "ReShade add-ons",
  shaders: "Shaders",
  compiler: "Shader compiler",
  preset: "Preset",
  tuning: "Tuning",
};

const ROLE_TO_STAGE: Record<StackRole, Stage> = {
  injector: "injector",
  neural_rendering: "nr",
  super_sampling: "sr",
  frame_generation: "sr",
  shader_compiler: "compiler",
  addon: "addons",
  companion: "addons",
};

const ROLE_LABEL: Record<StackRole, string> = {
  injector: "Injector",
  neural_rendering: "Neural Rendering runtime",
  super_sampling: "Super Resolution runtime",
  frame_generation: "Frame Generation runtime",
  shader_compiler: "Shader compiler",
  addon: "ReShade add-on",
  companion: "Companion process",
};

/** Findings internal to one layer, rendered inside that layer's row. */
const FINDING_STAGE: Record<string, Stage> = {
  "stack-feed-host-missing": "addons",
  "stack-addon-disabled": "addons",
  "stack-preset-missing": "preset",
  "stack-technique-order": "preset",
  "stack-feed-technique-off": "preset",
  "stack-search-path-mismatch": "shaders",
  "stack-dlss-reverted": "sr",
};

/** Findings that are a relationship *between* two layers, rendered on the
 *  connector segment following the given stage. */
const CONNECTOR_FINDING_AFTER: Record<string, Stage> = {
  "stack-no-injector": "injector",
  "stack-nr-runtime-missing": "nr",
  "stack-technique-source-missing": "shaders",
};

/** Stages where a real feature (tuning controls, disable/re-enable, runtime
 *  swap, preset reordering) is deliberately not built yet. */
const STAGE_COMING_NEXT = new Set<Stage>(["nr", "sr", "addons", "preset", "tuning"]);

function stagePresent(stage: Stage, stack: ClientStack): boolean {
  switch (stage) {
    case "shaders":
      return stack.shaders.present;
    case "preset":
      return stack.preset?.exists ?? false;
    case "tuning":
      return stack.tuning.length > 0;
    default:
      return stack.items.some((item) => ROLE_TO_STAGE[item.role] === stage);
  }
}

function stageLevel(stage: Stage, stack: ClientStack): HealthLevel {
  const findings = stack.findings.filter((f) => FINDING_STAGE[f.id] === stage);
  if (findings.length > 0) return worstLevel(findings.map((f) => f.level));
  return stagePresent(stage, stack) ? "ok" : "info";
}

function firstNonEmptyStage(stack: ClientStack): Stage | null {
  return STAGE_ORDER.find((stage) => stagePresent(stage, stack)) ?? null;
}

/** The stage whose row should be auto-selected so its explanation is already
 *  on screen. Internal findings (which render inside a row) take priority;
 *  connector-only problems fall back to the stage the dashed segment follows. */
function firstIssueStage(stack: ClientStack): Stage | null {
  const internal = new Set(
    stack.findings.map((f) => FINDING_STAGE[f.id]).filter((s): s is Stage => Boolean(s))
  );
  for (const stage of STAGE_ORDER) if (internal.has(stage)) return stage;

  const connector = new Set(
    stack.findings.map((f) => CONNECTOR_FINDING_AFTER[f.id]).filter((s): s is Stage => Boolean(s))
  );
  for (const stage of STAGE_ORDER) if (connector.has(stage)) return stage;

  return null;
}

/** A line from ReShade.log or dlss5-feed.log that matched a known failure
 *  signature. These are where a DLSS 5 stack actually announces breakage —
 *  a shader that would not compile, an unsupported parameter — so they stay
 *  in the panel even though they are not a layer of the pipeline. */
interface LogExcerpt {
  file: string;
  rule: string;
  line: string;
}

type SelectionKey = Stage | "adoption" | "records" | "logs";

function computeDefaultSelection(
  stack: ClientStack | null,
  plan: AdoptionPlan | null,
  adoptionDismissed: boolean
): SelectionKey | null {
  if (!stack || stack.is_empty) return null;
  if (plan && !plan.already_managed && !adoptionDismissed) return "adoption";
  if (plan?.already_managed) return firstIssueStage(stack) ?? "tuning";
  return firstNonEmptyStage(stack) ?? "injector";
}

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

function formatMB(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  return mb >= 100 ? `${Math.round(mb)} MB` : `${mb.toFixed(1)} MB`;
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

  const [stack, setStack] = useState<ClientStack | null>(null);
  const [stackLoading, setStackLoading] = useState(false);
  const [stackError, setStackError] = useState<string | null>(null);

  const [plan, setPlan] = useState<AdoptionPlan | null>(null);
  const [planLoading, setPlanLoading] = useState(false);
  const [planError, setPlanError] = useState<string | null>(null);

  const [detecting, setDetecting] = useState(false);
  const [browsing, setBrowsing] = useState(false);

  const [detectError, setDetectError] = useState<string | null>(null);
  const [browseError, setBrowseError] = useState<string | null>(null);

  // Managed-by-Kalpa inventory for the selected install (the pre-existing
  // model: files Kalpa itself wrote, independent of adoption).
  const [managedInventory, setManagedInventory] = useState<ManagedInventory | null>(null);
  const [managedLoading, setManagedLoading] = useState(false);
  const [managedError, setManagedError] = useState<string | null>(null);

  // Rail navigation. `selection` is the user's explicit override; when null the
  // default is derived live from stack/plan (see `computeDefaultSelection`) so
  // there is never a stale setState racing the data it depends on.
  const [selection, setSelection] = useState<SelectionKey | null>(null);
  const [railManualExpand, setRailManualExpand] = useState(false);
  const [adoptionDismissed, setAdoptionDismissed] = useState(false);

  // Adoption flow. Reset per install alongside everything else below.
  const [keepCopies, setKeepCopies] = useState(true);
  const [adopting, setAdopting] = useState(false);
  const [adoptError, setAdoptError] = useState<string | null>(null);
  const [adoptOutcome, setAdoptOutcome] = useState<AdoptionOutcome | null>(null);
  const [forgetConfirming, setForgetConfirming] = useState(false);
  const [forgetting, setForgetting] = useState(false);
  const [forgetError, setForgetError] = useState<string | null>(null);
  const [logExcerpts, setLogExcerpts] = useState<LogExcerpt[]>([]);

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

  // Monotonic token: a slow load that resolves after the user has already
  // refreshed or picked a different install must not overwrite newer state.
  // Bumped by every load entry point.
  const runToken = useRef(0);

  /** Clears every per-install control latch: rail selection/expand state,
   *  adoption dismissal and its checkbox, and the managed-file confirm/
   *  emergency-removal machinery. Called at every point the selected install
   *  changes or is reloaded, so none of it can survive onto a different
   *  folder or a stale content swap. */
  const resetInstallUiState = useCallback(() => {
    setSelection(null);
    setRailManualExpand(false);
    setAdoptionDismissed(false);
    setKeepCopies(true);
    setAdopting(false);
    setAdoptError(null);
    setAdoptOutcome(null);
    setForgetConfirming(false);
    setForgetting(false);
    setForgetError(null);
    setLogExcerpts([]);
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

  const loadPlan = useCallback(async (clientDir: string, token: number) => {
    setPlanLoading(true);
    setPlanError(null);
    try {
      const next = await invokeOrThrow<AdoptionPlan>("plan_adoption", { clientDir });
      if (runToken.current !== token) return;
      setPlan(next);
    } catch (e) {
      if (runToken.current !== token) return;
      setPlan(null);
      setPlanError(getTauriErrorMessage(e));
    } finally {
      if (runToken.current === token) setPlanLoading(false);
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

  /** Inspects the stack, then — only when there is something to manage —
   *  loads the adoption plan and Kalpa's own records alongside it. A stock
   *  client never triggers those calls: nothing to plan, nothing recorded. */
  /** Known-failure lines out of ReShade.log and dlss5-feed.log.
   *
   *  A best-effort extra: a log Kalpa cannot read is not worth an error banner
   *  when the rest of the panel is fine, so this swallows its own failure and
   *  simply shows nothing. */
  const loadLogs = useCallback(async (clientDir: string, token: number) => {
    try {
      const report = await invokeOrThrow<{ log_excerpts: LogExcerpt[] }>("inspect_eso_client", {
        clientDir,
      });
      if (runToken.current !== token) return;
      setLogExcerpts(report.log_excerpts ?? []);
    } catch {
      if (runToken.current !== token) return;
      setLogExcerpts([]);
    }
  }, []);

  const loadInstall = useCallback(
    async (clientDir: string, token: number) => {
      setStackLoading(true);
      setStackError(null);
      setPlan(null);
      setPlanError(null);
      setManagedInventory(null);
      setManagedError(null);
      try {
        const next = await invokeOrThrow<ClientStack>("inspect_client_stack", { clientDir });
        if (runToken.current !== token) return;
        setStack(next);
        setStackLoading(false);
        if (!next.is_empty) {
          await Promise.all([
            loadPlan(clientDir, token),
            loadManaged(clientDir, token),
            loadLogs(clientDir, token),
          ]);
        }
      } catch (e) {
        if (runToken.current !== token) return;
        setStack(null);
        setStackError(getTauriErrorMessage(e));
        setStackLoading(false);
      }
    },
    [loadLogs, loadManaged, loadPlan]
  );

  const detect = useCallback(async () => {
    const token = ++runToken.current;
    setDetecting(true);
    setDetectError(null);
    setBrowseError(null);
    setStack(null);
    setStackError(null);
    resetInstallUiState();
    try {
      const found = await invokeOrThrow<EsoClientLocation[]>("detect_eso_clients");
      if (runToken.current !== token) return;
      setClients(found);
      const first = found[0];
      setSelectedDir(first ? first.client_dir : null);
      setDetecting(false);
      if (first) await loadInstall(first.client_dir, token);
    } catch (e) {
      if (runToken.current !== token) return;
      setClients([]);
      setSelectedDir(null);
      setDetectError(getTauriErrorMessage(e));
      setDetecting(false);
    }
  }, [loadInstall, resetInstallUiState]);

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
      resetInstallUiState();
      void loadInstall(clientDir, token);
    },
    [loadInstall, resetInstallUiState, selectedDir]
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
      setDetectError(null);
      resetInstallUiState();
      await loadInstall(validated.client_dir, token);
    } catch (e) {
      setBrowseError(getTauriErrorMessage(e));
    } finally {
      setBrowsing(false);
    }
  }, [loadInstall, resetInstallUiState]);

  const handleAdopt = useCallback(async () => {
    if (!selectedDir) return;
    const token = runToken.current;
    setAdopting(true);
    setAdoptError(null);
    try {
      const outcome = await invokeOrThrow<AdoptionOutcome>("adopt_stack", {
        clientDir: selectedDir,
        keepCopies,
      });
      if (runToken.current !== token) return;
      setAdoptOutcome(outcome);
      setSelection(null);
      await Promise.all([loadPlan(selectedDir, token), loadManaged(selectedDir, token)]);
    } catch (e) {
      if (runToken.current !== token) return;
      setAdoptError(getTauriErrorMessage(e));
    } finally {
      if (runToken.current === token) setAdopting(false);
    }
  }, [keepCopies, loadManaged, loadPlan, selectedDir]);

  const handleDismissAdoption = useCallback(() => {
    setAdoptionDismissed(true);
    setSelection(null);
  }, []);

  /** Give up Kalpa's records for this install.
   *
   *  This is the exit from "Manage this stack", and it has to exist for that
   *  button to be a safe click: without it, agreeing to be managed is a
   *  one-way door. It deletes no file in the game folder — only the records —
   *  so the stack keeps working exactly as it did. */
  const handleForget = useCallback(async () => {
    if (!selectedDir) return;
    const token = runToken.current;
    setForgetting(true);
    setForgetError(null);
    try {
      await invokeOrThrow<string[]>("forget_stack", { clientDir: selectedDir });
      if (runToken.current !== token) return;
      setForgetConfirming(false);
      setAdoptOutcome(null);
      // Re-listing is what flips the panel back to the unmanaged view, so the
      // adoption card must be offerable again rather than staying dismissed.
      setAdoptionDismissed(false);
      await Promise.all([loadPlan(selectedDir, token), loadManaged(selectedDir, token)]);
    } catch (e) {
      if (runToken.current !== token) return;
      setForgetError(getTauriErrorMessage(e));
    } finally {
      if (runToken.current === token) setForgetting(false);
    }
  }, [loadManaged, loadPlan, selectedDir]);

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

  const effectiveSelection = useMemo(
    () => selection ?? computeDefaultSelection(stack, plan, adoptionDismissed),
    [selection, stack, plan, adoptionDismissed]
  );

  const isHealthyManaged = useMemo(
    () => Boolean(stack && !stack.is_empty && plan?.already_managed && stack.findings.length === 0),
    [stack, plan]
  );

  const railExpanded = isHealthyManaged ? railManualExpand : true;

  // The injector this stack loads is the one that would otherwise be flagged
  // as an "unmanaged orphan" — offering to quarantine the ReShade the user
  // just asked Kalpa to manage was the old panel's worst moment.
  const hideEmergency = useMemo(
    () => Boolean(plan?.already_managed && stack?.items.some((item) => item.role === "injector")),
    [plan, stack]
  );

  const busy = detecting || stackLoading;

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="sm:max-w-4xl h-[85vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>Client Health</DialogTitle>
          <DialogDescription>
            The graphics-mod stack in your ESO client folder. Kalpa downloads nothing here — it only
            records what is already there.
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

              {stackLoading && (
                <div className="flex items-center gap-3 py-10 text-sm text-muted-foreground">
                  <Spinner />
                  <span>Inspecting the client folder...</span>
                </div>
              )}

              {!stackLoading && stackError && (
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
                    <p className="mt-0.5 text-xs text-muted-foreground">{stackError}</p>
                  </div>
                </GlassPanel>
              )}

              {!stackLoading && !stackError && stack && stack.is_empty && <StockClientCard />}

              {!stackLoading && !stackError && stack && !stack.is_empty && (
                <StackBody
                  stack={stack}
                  plan={plan}
                  planLoading={planLoading}
                  planError={planError}
                  effectiveSelection={effectiveSelection}
                  onSelect={(key) => setSelection(key)}
                  railExpanded={railExpanded}
                  isHealthyManaged={isHealthyManaged}
                  onToggleRailExpand={() => setRailManualExpand((v) => !v)}
                  keepCopies={keepCopies}
                  onToggleKeepCopies={() => setKeepCopies((v) => !v)}
                  adopting={adopting}
                  adoptError={adoptError}
                  adoptOutcome={adoptOutcome}
                  onAdopt={() => void handleAdopt()}
                  onDismissAdoption={handleDismissAdoption}
                  managedLoading={managedLoading}
                  managedError={managedError}
                  managedInventory={managedInventory}
                  logExcerpts={logExcerpts}
                  hideEmergency={hideEmergency}
                  isManaged={Boolean(plan?.already_managed)}
                  forgetConfirming={forgetConfirming}
                  forgetting={forgetting}
                  forgetError={forgetError}
                  onRequestForget={() => setForgetConfirming(true)}
                  onCancelForget={() => setForgetConfirming(false)}
                  onConfirmForget={() => void handleForget()}
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
/* Sub-views — install picker (unchanged shape)                              */
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

/* -------------------------------------------------------------------------- */
/* Stock client — the common case, kept deliberately quiet                   */
/* -------------------------------------------------------------------------- */

function StockClientCard() {
  return (
    <GlassPanel
      variant="subtle"
      className="flex flex-col items-center gap-2 px-4 py-10 text-center"
    >
      <HardDriveIcon aria-hidden className="size-6 text-muted-foreground/70" />
      <p className="font-heading text-sm font-semibold">Stock ESO client</p>
      <p className="max-w-sm text-xs leading-relaxed text-muted-foreground">
        No injector, no runtime swaps. Nothing to manage.
      </p>
    </GlassPanel>
  );
}

/* -------------------------------------------------------------------------- */
/* Finding row (shared by in-row findings and the connector gutter)          */
/* -------------------------------------------------------------------------- */

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
/* Master-detail body                                                        */
/* -------------------------------------------------------------------------- */

interface StackBodyProps {
  stack: ClientStack;
  plan: AdoptionPlan | null;
  planLoading: boolean;
  planError: string | null;
  effectiveSelection: SelectionKey | null;
  onSelect: (key: SelectionKey) => void;
  railExpanded: boolean;
  isHealthyManaged: boolean;
  onToggleRailExpand: () => void;
  keepCopies: boolean;
  onToggleKeepCopies: () => void;
  adopting: boolean;
  adoptError: string | null;
  adoptOutcome: AdoptionOutcome | null;
  onAdopt: () => void;
  onDismissAdoption: () => void;
  managedLoading: boolean;
  managedError: string | null;
  managedInventory: ManagedInventory | null;
  logExcerpts: LogExcerpt[];
  hideEmergency: boolean;
  isManaged: boolean;
  forgetConfirming: boolean;
  forgetting: boolean;
  forgetError: string | null;
  onRequestForget: () => void;
  onCancelForget: () => void;
  onConfirmForget: () => void;
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
}

function StackBody(props: StackBodyProps) {
  const { stack, plan, effectiveSelection, onSelect, railExpanded, isHealthyManaged } = props;
  const railRef = useRef<HTMLDivElement>(null);

  const otherKept = useMemo(() => stack.preserved_originals.filter((o) => !o.backs_up), [stack]);

  const handleRailKeyDown = useCallback((e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    const container = railRef.current;
    if (!container) return;
    const items = Array.from(container.querySelectorAll<HTMLElement>('[role="option"]'));
    if (items.length === 0) return;
    e.preventDefault();
    const idx = items.indexOf(document.activeElement as HTMLElement);
    const nextIdx =
      idx === -1
        ? 0
        : e.key === "ArrowDown"
          ? Math.min(idx + 1, items.length - 1)
          : Math.max(idx - 1, 0);
    items[nextIdx]?.focus();
  }, []);

  return (
    <div className="flex gap-4">
      <div
        ref={railRef}
        role="listbox"
        aria-label="Stack layers"
        onKeyDown={handleRailKeyDown}
        className="w-[300px] shrink-0 space-y-2"
      >
        {plan && !plan.already_managed && (
          <button
            type="button"
            role="option"
            aria-selected={effectiveSelection === "adoption"}
            onClick={() => onSelect("adoption")}
            className={cn(
              "flex w-full flex-col gap-1 rounded-xl border p-3 text-left transition-colors duration-150",
              "focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-accent-sky/20",
              effectiveSelection === "adoption"
                ? "border-primary/30 border-l-[3px] border-l-primary bg-primary/[0.04]"
                : "border-structure-06 bg-structure-02 hover:border-structure-10"
            )}
          >
            <InfoPill color="gold" className="self-start">
              Not managed yet
            </InfoPill>
            <span className="text-xs text-muted-foreground">Review and manage this stack</span>
          </button>
        )}

        {isHealthyManaged && !railExpanded ? (
          <GlassPanel variant="subtle" className="flex items-center justify-between gap-2 p-3">
            <span className="flex min-w-0 items-center gap-2 text-[13px]">
              <ShieldCheckIcon aria-hidden className="size-4 shrink-0 text-status-success" />
              <span className="truncate">
                <span className="text-status-success">●</span> Managed &middot; DLSS 5 Neural
                Rendering &middot; {STAGE_ORDER.length} layers &middot; all consistent
              </span>
            </span>
            <Button variant="ghost" size="xs" onClick={props.onToggleRailExpand}>
              Show layers
              <ChevronDownIcon />
            </Button>
          </GlassPanel>
        ) : (
          <>
            {isHealthyManaged && (
              <div className="flex justify-end">
                <Button variant="ghost" size="xs" onClick={props.onToggleRailExpand}>
                  Hide layers
                  <ChevronRightIcon />
                </Button>
              </div>
            )}
            {STAGE_ORDER.map((stage, i) => (
              <div key={stage}>
                <StageRow
                  stage={stage}
                  stack={stack}
                  selected={effectiveSelection === stage}
                  onSelect={() => onSelect(stage)}
                />
                {i < STAGE_ORDER.length - 1 && (
                  <Connector
                    findings={stack.findings.filter((f) => CONNECTOR_FINDING_AFTER[f.id] === stage)}
                  />
                )}
              </div>
            ))}
          </>
        )}

        {otherKept.length > 0 && (
          <GlassPanel variant="subtle" className="space-y-1 p-3">
            <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
              Other kept files
            </p>
            {otherKept.map((o) => (
              <p key={o.file_name} className="truncate font-mono text-[11px] text-muted-foreground">
                {o.file_name}
              </p>
            ))}
          </GlassPanel>
        )}

        <Divider />

        <button
          type="button"
          role="option"
          aria-selected={effectiveSelection === "records"}
          onClick={() => onSelect("records")}
          className={cn(
            "flex w-full items-center gap-2 rounded-xl border p-3 text-left transition-colors duration-150",
            "focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-accent-sky/20",
            effectiveSelection === "records"
              ? "border-primary/30 border-l-[3px] border-l-primary bg-primary/[0.04]"
              : "border-structure-06 bg-structure-02 hover:border-structure-10"
          )}
        >
          <HardDriveIcon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1">
            <span className="block font-heading text-[13px] font-semibold">
              Kalpa&apos;s records
            </span>
            <span className="block text-xs text-muted-foreground">
              {props.managedInventory?.files.length ?? 0} tracked file
              {props.managedInventory?.files.length === 1 ? "" : "s"}
            </span>
          </span>
        </button>

        {props.logExcerpts.length > 0 && (
          <button
            type="button"
            role="option"
            aria-selected={effectiveSelection === "logs"}
            onClick={() => onSelect("logs")}
            className={cn(
              "flex w-full items-center gap-2 rounded-xl border p-3 text-left transition-colors duration-150",
              "focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-accent-sky/20",
              effectiveSelection === "logs"
                ? "border-primary/30 border-l-[3px] border-l-primary bg-primary/[0.04]"
                : "border-status-warning/20 border-l-[3px] border-l-status-warning bg-structure-02 hover:border-structure-10"
            )}
          >
            <ScrollTextIcon aria-hidden className="size-4 shrink-0 text-status-warning" />
            <span className="min-w-0 flex-1">
              <span className="block font-heading text-[13px] font-semibold">Log signals</span>
              <span className="block text-xs text-muted-foreground">
                {props.logExcerpts.length} known problem line
                {props.logExcerpts.length === 1 ? "" : "s"}
              </span>
            </span>
          </button>
        )}
      </div>

      <div className="min-w-0 flex-1 space-y-3">
        <DetailPane {...props} />
      </div>
    </div>
  );
}

function StageRow({
  stage,
  stack,
  selected,
  onSelect,
}: {
  stage: Stage;
  stack: ClientStack;
  selected: boolean;
  onSelect: () => void;
}) {
  const level = stageLevel(stage, stack);
  const meta = LEVEL_META[level];
  const { Icon } = meta;
  const items = stack.items.filter((item) => ROLE_TO_STAGE[item.role] === stage);

  let summary: string;
  let fileName: string | null = null;
  if (stage === "shaders") {
    summary = stack.shaders.present
      ? `${stack.shaders.effect_count} effects, ${stack.shaders.texture_count} textures`
      : "Not present";
  } else if (stage === "preset") {
    summary = stack.preset
      ? stack.preset.exists
        ? `${stack.preset.techniques.length} technique${stack.preset.techniques.length === 1 ? "" : "s"} enabled`
        : "Missing file"
      : "Not configured";
  } else if (stage === "tuning") {
    summary = stack.tuning.length > 0 ? `${stack.tuning.length} values` : "Not present";
  } else if (items.length === 0) {
    summary = "Not present";
  } else if (items.length === 1) {
    summary = items[0]!.display_name ?? "no product name";
    fileName = items[0]!.file_name;
  } else {
    summary = `${items.length} files`;
  }

  return (
    <button
      type="button"
      role="option"
      aria-selected={selected}
      onClick={onSelect}
      className={cn(
        "flex w-full flex-col gap-1 rounded-xl border border-l-[3px] p-3 text-left transition-colors duration-150",
        "focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-accent-sky/20",
        selected
          ? "border-primary/30 border-l-primary bg-primary/[0.04]"
          : cn(meta.border, meta.tint)
      )}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="font-heading text-[13px] font-semibold">{STAGE_LABEL[stage]}</span>
        <span className="flex items-center gap-1">
          <Icon aria-hidden className={cn("size-3.5 shrink-0", meta.text)} />
          <span className={cn("text-[11px] font-semibold uppercase tracking-wide", meta.text)}>
            {meta.label}
          </span>
        </span>
      </div>
      <span className="truncate text-xs text-muted-foreground">{summary}</span>
      {fileName && (
        <span className="truncate font-mono text-[11px] text-muted-foreground">{fileName}</span>
      )}
    </button>
  );
}

function Connector({ findings }: { findings: HealthFinding[] }) {
  if (findings.length === 0) {
    return <div aria-hidden className="ml-[13px] h-3 w-px bg-structure-10" />;
  }
  const level = worstLevel(findings.map((f) => f.level));
  return (
    <div className="flex gap-2 py-1">
      <div
        aria-hidden
        className={cn("ml-[9px] w-0 shrink-0 border-l-2 border-dashed", LEVEL_META[level].line)}
      />
      <ul className="flex-1 space-y-2">
        {findings.map((f) => (
          <FindingRow key={f.id} finding={f} />
        ))}
      </ul>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Detail pane                                                               */
/* -------------------------------------------------------------------------- */

function DetailPane(props: StackBodyProps) {
  const { stack, plan, planLoading, planError, effectiveSelection } = props;

  if (effectiveSelection === "adoption") {
    if (planLoading) {
      return (
        <div className="flex items-center gap-3 py-10 text-sm text-muted-foreground" role="status">
          <Spinner />
          <span>Working out what adopting this stack would record...</span>
        </div>
      );
    }
    if (planError) {
      return (
        <GlassPanel variant="subtle" className="flex items-start gap-2 p-3 text-sm" role="alert">
          <ShieldAlertIcon aria-hidden className="mt-0.5 size-4 shrink-0 text-status-danger" />
          <div>
            <p className="font-heading text-[13px] font-semibold text-status-danger">
              Could not plan adoption
            </p>
            <p className="mt-0.5 text-xs text-muted-foreground">{planError}</p>
          </div>
        </GlassPanel>
      );
    }
    if (!plan) return null;
    return (
      <AdoptionCard
        plan={plan}
        keepCopies={props.keepCopies}
        onToggleKeepCopies={props.onToggleKeepCopies}
        adopting={props.adopting}
        adoptError={props.adoptError}
        onAdopt={props.onAdopt}
        onDismiss={props.onDismissAdoption}
      />
    );
  }

  if (effectiveSelection === "logs") {
    return <LogSignals excerpts={props.logExcerpts} />;
  }

  if (effectiveSelection === "records") {
    return (
      <ManagedSection
        loading={props.managedLoading}
        error={props.managedError}
        inventory={props.managedInventory}
        hideEmergency={props.hideEmergency}
        isManaged={props.isManaged}
        forgetConfirming={props.forgetConfirming}
        forgetting={props.forgetting}
        forgetError={props.forgetError}
        onRequestForget={props.onRequestForget}
        onCancelForget={props.onCancelForget}
        onConfirmForget={props.onConfirmForget}
        selectedPaths={props.selectedPaths}
        onToggleSelect={props.onToggleSelect}
        removeMode={props.removeMode}
        removing={props.removing}
        removeOutcome={props.removeOutcome}
        removeError={props.removeError}
        onRequestRemove={props.onRequestRemove}
        onCancelRemove={props.onCancelRemove}
        onConfirmRemove={props.onConfirmRemove}
        emergencyOpen={props.emergencyOpen}
        onToggleEmergencyOpen={props.onToggleEmergencyOpen}
        emergencyTarget={props.emergencyTarget}
        onSetEmergencyTarget={props.onSetEmergencyTarget}
        emergencyConfirmInput={props.emergencyConfirmInput}
        onEmergencyConfirmInputChange={props.onEmergencyConfirmInputChange}
        emergencyBusy={props.emergencyBusy}
        emergencyError={props.emergencyError}
        emergencyResult={props.emergencyResult}
        onEmergencyRemove={props.onEmergencyRemove}
      />
    );
  }

  if (!effectiveSelection) {
    return <p className="text-xs text-muted-foreground">Select a layer to see its detail.</p>;
  }

  const stage = effectiveSelection;
  if (stage === "shaders") return <ShadersDetail stack={stack} />;
  if (stage === "preset") return <PresetDetail stack={stack} />;
  if (stage === "tuning") return <TuningDetail stack={stack} />;
  return <ItemsStageDetail stage={stage} stack={stack} />;
}

function ComingNextNote() {
  return <p className="text-xs text-muted-foreground">Editing this from Kalpa is coming next.</p>;
}

function StageFindingsList({ stage, stack }: { stage: Stage; stack: ClientStack }) {
  const findings = stack.findings.filter((f) => FINDING_STAGE[f.id] === stage);
  if (findings.length === 0) return null;
  return (
    <ul className="space-y-2">
      {findings.map((f) => (
        <FindingRow key={f.id} finding={f} />
      ))}
    </ul>
  );
}

function ItemCard({ item, preserved }: { item: StackItem; preserved: PreservedOriginal | null }) {
  const identifiedByHash =
    !item.display_name && !item.version && !item.company && !item.description;
  return (
    <GlassPanel
      variant="subtle"
      className="space-y-1 rounded-xl border-l-[3px] border-l-structure-10 p-3"
    >
      <div className="flex flex-wrap items-center gap-2">
        <h4 className="font-heading text-[13px] font-semibold">{ROLE_LABEL[item.role]}</h4>
        {identifiedByHash && <InfoPill color="muted">identified by hash</InfoPill>}
      </div>
      <p className="text-xs text-muted-foreground">
        {item.display_name ?? "no product name"} &middot; {item.version ?? "no version info"}
      </p>
      <p className="font-mono text-[11px] text-muted-foreground">{item.file_name}</p>
      {preserved && (
        <p className="text-xs text-status-info">
          Original &middot; {preserved.version ?? "no version info"} &middot; kept by you
        </p>
      )}
    </GlassPanel>
  );
}

function ItemsStageDetail({ stage, stack }: { stage: Stage; stack: ClientStack }) {
  const items = stack.items.filter((item) => ROLE_TO_STAGE[item.role] === stage);
  const findByFileName = (name: string) =>
    stack.preserved_originals.find((o) => o.backs_up?.toLowerCase() === name.toLowerCase()) ?? null;

  return (
    <div className="space-y-3">
      <SectionHeader>{STAGE_LABEL[stage]}</SectionHeader>
      {items.length === 0 ? (
        <GlassPanel variant="subtle" className="p-3 text-xs text-muted-foreground">
          Not present in this install.
        </GlassPanel>
      ) : (
        <div className="space-y-2">
          {items.map((item) => (
            <ItemCard key={item.file_name} item={item} preserved={findByFileName(item.file_name)} />
          ))}
        </div>
      )}
      <StageFindingsList stage={stage} stack={stack} />
      {STAGE_COMING_NEXT.has(stage) && <ComingNextNote />}
    </div>
  );
}

function ShadersDetail({ stack }: { stack: ClientStack }) {
  const shaders = stack.shaders;
  return (
    <div className="space-y-3">
      <SectionHeader>Shaders</SectionHeader>
      <GlassPanel variant="subtle" className="space-y-1 p-3 text-xs">
        <p>
          <span className="text-muted-foreground">Present:</span> {shaders.present ? "Yes" : "No"}
        </p>
        <p>
          <span className="text-muted-foreground">Effects:</span> {shaders.effect_count}
        </p>
        <p>
          <span className="text-muted-foreground">Textures:</span> {shaders.texture_count}
        </p>
        <p className="break-words">
          <span className="text-muted-foreground">Search paths:</span>{" "}
          <span className="font-mono">{shaders.effect_search_paths ?? "not configured"}</span>
        </p>
      </GlassPanel>
      <StageFindingsList stage="shaders" stack={stack} />
    </div>
  );
}

function PresetDetail({ stack }: { stack: ClientStack }) {
  const preset = stack.preset;
  return (
    <div className="space-y-3">
      <SectionHeader>Preset</SectionHeader>
      {!preset ? (
        <GlassPanel variant="subtle" className="p-3 text-xs text-muted-foreground">
          No preset configured.
        </GlassPanel>
      ) : (
        <GlassPanel variant="subtle" className="space-y-2 p-3 text-xs">
          <p className="break-words">
            <span className="text-muted-foreground">Path:</span>{" "}
            <span className="font-mono">{preset.path}</span>
          </p>
          <p>
            <span className="text-muted-foreground">Exists:</span> {preset.exists ? "Yes" : "No"}
          </p>
          <div>
            <p className="mb-1 text-muted-foreground">Enabled techniques, in run order:</p>
            {preset.techniques.length === 0 ? (
              <p className="text-muted-foreground">None enabled.</p>
            ) : (
              <ol className="list-decimal space-y-1 pl-4">
                {preset.techniques.map((t) => (
                  <li key={t.name} className="flex flex-wrap items-center gap-2">
                    <span className="font-mono">{t.name}</span>
                    {!t.source_present && <InfoPill color="red">missing {t.source}</InfoPill>}
                  </li>
                ))}
              </ol>
            )}
          </div>
          {preset.available.length > 0 && (
            <p className="break-words">
              <span className="text-muted-foreground">All known techniques:</span>{" "}
              <span className="font-mono">{preset.available.join(", ")}</span>
            </p>
          )}
        </GlassPanel>
      )}
      <StageFindingsList stage="preset" stack={stack} />
      <ComingNextNote />
    </div>
  );
}

function TuningDetail({ stack }: { stack: ClientStack }) {
  return (
    <div className="space-y-3">
      <SectionHeader>Tuning</SectionHeader>
      {stack.tuning.length === 0 ? (
        <GlassPanel variant="subtle" className="p-3 text-xs text-muted-foreground">
          No [RenoDX.DLSS5] tuning block found.
        </GlassPanel>
      ) : (
        <GlassPanel variant="subtle" className="p-0">
          {stack.tuning.map((t) => (
            <div
              key={t.key}
              className="flex items-center justify-between gap-3 border-b border-structure-06 px-3 py-2 last:border-b-0"
            >
              <span className="font-mono text-[12px]">{t.key}</span>
              <span className="font-mono text-[12px] text-muted-foreground">{t.value}</span>
            </div>
          ))}
        </GlassPanel>
      )}
      <StageFindingsList stage="tuning" stack={stack} />
      <ComingNextNote />
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Adoption                                                                   */
/* -------------------------------------------------------------------------- */

function AdoptionCard({
  plan,
  keepCopies,
  onToggleKeepCopies,
  adopting,
  adoptError,
  onAdopt,
  onDismiss,
}: {
  plan: AdoptionPlan;
  keepCopies: boolean;
  onToggleKeepCopies: () => void;
  adopting: boolean;
  adoptError: string | null;
  onAdopt: () => void;
  onDismiss: () => void;
}) {
  return (
    <GlassPanel variant="default" className="space-y-3 p-4">
      <InfoPill color="gold" className="self-start">
        Not managed yet
      </InfoPill>
      <h3 className="font-heading text-base font-semibold">
        This stack works, and Kalpa didn&apos;t build it.
      </h3>
      <p className="text-sm leading-relaxed text-muted-foreground">
        Managing it only means Kalpa records what is here, so it can tell you if a game update
        knocks something out of place. Nothing is changed, downloaded, or moved. Your own backups —
        files ending in <span className="font-mono">.disabled-bak</span> or{" "}
        <span className="font-mono">.eso-orig-bak</span> — are treated as your originals and left
        exactly where they are.
      </p>
      <details className="text-xs">
        <summary className="cursor-pointer list-none font-semibold text-foreground [&::-webkit-details-marker]:hidden">
          ▸ What gets recorded ({plan.entries.length} files)
        </summary>
        <ul className="mt-2 space-y-1.5">
          {plan.entries.map((entry) => (
            <li key={entry.relative_path} className="flex items-center justify-between gap-2">
              <span className="min-w-0 flex-1 truncate font-mono">{entry.relative_path}</span>
              <InfoPill color="muted">{KIND_LABEL[entry.kind]}</InfoPill>
            </li>
          ))}
        </ul>
      </details>
      <label className="flex items-start gap-2 text-xs">
        <Checkbox
          checked={keepCopies}
          onCheckedChange={() => onToggleKeepCopies()}
          className="mt-0.5"
        />
        <span>
          Keep a copy of the swapped runtimes ({formatMB(plan.copy_bytes)}) so Kalpa can put them
          back after a game update.
        </span>
      </label>
      {adoptError && (
        <p className="text-xs text-status-danger" role="alert">
          {adoptError}
        </p>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" disabled={adopting} onClick={onAdopt}>
          {adopting ? <Spinner className="size-3.5" /> : <PackageCheckIcon />}
          {adopting ? "Managing..." : "Manage this stack"}
        </Button>
        <Button size="sm" variant="outline" disabled={adopting} onClick={onDismiss}>
          Not now
        </Button>
      </div>
    </GlassPanel>
  );
}

/* -------------------------------------------------------------------------- */
/* Log signals                                                                */
/* -------------------------------------------------------------------------- */

/** Lines ReShade and the DLSS 5 feed wrote that match a known failure
 *  signature. Deliberately presented as evidence rather than as findings:
 *  Kalpa matched a string in a log, which is a much weaker claim than the
 *  cross-layer checks, and a stale line from a problem already fixed will
 *  still be sitting in the file. */
function LogSignals({ excerpts }: { excerpts: LogExcerpt[] }) {
  const byFile = useMemo(() => {
    const groups = new Map<string, LogExcerpt[]>();
    for (const excerpt of excerpts) {
      const list = groups.get(excerpt.file) ?? [];
      list.push(excerpt);
      groups.set(excerpt.file, list);
    }
    return Array.from(groups.entries());
  }, [excerpts]);

  return (
    <section aria-labelledby="client-health-logs" className="space-y-3">
      <SectionHeader id="client-health-logs">Log signals</SectionHeader>
      <p className="text-xs leading-relaxed text-muted-foreground">
        Lines in your ReShade and DLSS 5 logs that match a known failure. Logs are append-only, so a
        line here may be from a problem you have already fixed — check the timestamps in the file
        before chasing it.
      </p>
      {byFile.map(([file, lines]) => (
        <GlassPanel key={file} variant="subtle" className="space-y-2 p-3">
          <p className="font-mono text-[11px] text-muted-foreground">{file}</p>
          <ul className="space-y-1.5">
            {lines.map((excerpt, index) => (
              <li key={`${excerpt.rule}-${index}`} className="space-y-0.5">
                <InfoPill color="amber">{excerpt.rule}</InfoPill>
                <p className="break-words font-mono text-[11px] text-muted-foreground">
                  {excerpt.line}
                </p>
              </li>
            ))}
          </ul>
        </GlassPanel>
      ))}
    </section>
  );
}

/* -------------------------------------------------------------------------- */
/* Kalpa's records (formerly "Managed by Kalpa")                             */
/* -------------------------------------------------------------------------- */

function ManagedSection({
  loading,
  error,
  inventory,
  hideEmergency,
  isManaged,
  forgetConfirming,
  forgetting,
  forgetError,
  onRequestForget,
  onCancelForget,
  onConfirmForget,
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
  hideEmergency: boolean;
  isManaged: boolean;
  forgetConfirming: boolean;
  forgetting: boolean;
  forgetError: string | null;
  onRequestForget: () => void;
  onCancelForget: () => void;
  onConfirmForget: () => void;
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
  const orphans = hideEmergency ? [] : (inventory?.orphan_injectors ?? []);

  const pendingPaths = useMemo(() => {
    if (removeMode === "all") return files.map((f) => f.relative_path);
    if (removeMode === "selected") return Array.from(selectedPaths);
    return [];
  }, [files, removeMode, selectedPaths]);

  return (
    <section aria-labelledby="client-health-managed">
      <SectionHeader id="client-health-managed" className="mb-2">
        Kalpa&apos;s records
      </SectionHeader>

      {isManaged && (
        <GlassPanel variant="subtle" className="mb-3 space-y-2 p-3">
          <p className="text-xs leading-relaxed text-muted-foreground">
            Kalpa is managing this stack: it has a record of what is here, so it can tell you if a
            game update changes something. It has not modified any of it.
          </p>
          {!forgetConfirming && (
            <Button variant="outline" size="sm" onClick={onRequestForget}>
              Stop managing
            </Button>
          )}
          {forgetConfirming && (
            <div className="space-y-2">
              <p className="text-xs leading-relaxed text-muted-foreground">
                Kalpa will delete its records for this folder and nothing else. Every file stays
                exactly where it is, your stack keeps working, and you can ask Kalpa to manage it
                again at any time.
              </p>
              <div className="flex items-center gap-2">
                <Button variant="outline" size="sm" disabled={forgetting} onClick={onConfirmForget}>
                  {forgetting ? <Spinner className="size-3.5" /> : null}
                  {forgetting ? "Forgetting..." : "Stop managing"}
                </Button>
                <Button variant="ghost" size="sm" disabled={forgetting} onClick={onCancelForget}>
                  Cancel
                </Button>
              </div>
            </div>
          )}
          {forgetError && (
            <p className="text-xs text-status-danger" role="alert">
              {forgetError}
            </p>
          )}
        </GlassPanel>
      )}

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
