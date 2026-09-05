import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  ActivityIcon,
  AlertCircleIcon,
  AlertTriangleIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  CircleHelpIcon,
  HardDriveIcon,
  PackageCheckIcon,
  PauseIcon,
  PowerOffIcon,
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
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { GlassPanel } from "@/components/ui/glass-panel";
import { SectionHeader } from "@/components/ui/section-header";
import { InfoPill } from "@/components/ui/info-pill";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { StackPowerCard } from "@/components/client-stack/power-card";
import { SlotPane } from "@/components/client-stack/slot-pane";
import { SlotRail } from "@/components/client-stack/slot-rail";
import type { StackView } from "@/components/client-stack/slot-rail";
import {
  LEVEL_ORDER,
  SLOT_ORDER,
  findingsForSlot,
  slotFilled,
  slotLevel,
  slotNeed,
} from "@/components/client-stack/slots";
import type { Slot } from "@/components/client-stack/slots";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import type {
  StackMutationCoordinator,
  StackMutationResult,
} from "@/components/client-stack/panel-props";

import type {
  AdoptionOutcome,
  AdoptionPlan,
  ClientHealthPanelProps,
  ClientHealthReport,
  ClientSource,
  ClientStack,
  EmergencyRemoval,
  EsoClientLocation,
  ForgetOutcome,
  LogExcerpt,
  ManagedFileState,
  ManagedFileStatus,
  ManagedInventory,
  ManagedKind,
  NeuralRenderingState,
  OrphanInjector,
  UninstallOutcome,
} from "@/components/client-stack/types";
export * from "@/components/client-stack/types";

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
  parked: {
    label: "Switched off",
    Icon: PowerOffIcon,
    text: "text-muted-foreground",
    hint: "Moved aside so ESO does not load it. Switch the stack back on to put it back.",
  },
};

/* -------------------------------------------------------------------------- */
/* Selection                                                                  */
/* -------------------------------------------------------------------------- */
/* The slot model itself lives in `client-stack/slots.ts`. What stays here is  */
/* only the panel's own navigation: which slot, or which of the three views    */
/* that are not slots, is currently open.                                      */

/**
 * What the logs said, kept together because the three fields only mean
 * anything read as one answer.
 *
 * This used to be a bare `LogExcerpt[]` declared locally, which quietly encoded
 * the assumption the rest of this file was built on: that a log has nothing to
 * say unless it matched a failure. `log_excerpts` is now **fatal-only** — the
 * six ERROR/WARN lines a *working* RenoDX DLSS-NR setup writes on every launch
 * are recognised as benign and counted into `log_benign_suppressed` instead —
 * and `neural_rendering` carries the positive evidence that the stack ran at
 * all. An empty excerpt list is therefore the absence of known failures, never
 * proof of health, and the two are not allowed to be confused here again.
 *
 * The shape is the read fields of `ClientHealthReport`; `inspect_eso_client`
 * returns the whole report and this is the part the panel uses.
 */
type LogEvidence = Pick<
  ClientHealthReport,
  "log_excerpts" | "neural_rendering" | "log_benign_suppressed"
>;

/**
 * What a log Kalpa could not read looks like.
 *
 * Deliberately `unknown` rather than an empty-but-healthy shape: failing to
 * read the log is exactly the case where the panel must not claim anything,
 * and the fallback that says "no findings, so fine" is the bug being fixed.
 */
const NO_LOG_EVIDENCE: LogEvidence = {
  log_excerpts: [],
  neural_rendering: {
    state: "unknown",
    samples: 0,
    first_evaluation: null,
    last_evaluation: null,
  },
  log_benign_suppressed: 0,
};

/**
 * The header's one-glance verdict on the whole install.
 *
 * Exported and pure so the gating can be tested without a DOM, because the
 * gating *is* the feature. The old badge read
 * `attentionCount > 0 ? "n need attention" : "Everything agrees"`, so
 * "Everything agrees" was rendered by an empty array — the absence of findings,
 * never positive evidence that anything ran. That is how it claimed agreement
 * over a stack that could not work.
 *
 * Three gates now, in order, and each one blocks the claim on its own:
 *
 * 1. **Findings above `info`.** Unchanged, and still first: a cross-layer
 *    finding is a diagnosis, which outranks evidence.
 * 2. **Fatal log lines.** Easy to miss and the reason this is a separate gate:
 *    no `HealthFinding` is emitted from a log rule at all, by design, so a
 *    findings-only check sails straight past the `LoadFromDllMain` line — the
 *    single highest-value signature Kalpa knows.
 * 3. **Neural Rendering evidence.** Only `running` — the `EvaluateFeature`
 *    counter found *and climbing* — earns "Everything agrees". `stalled` and
 *    `unknown` get their own copy and must not be collapsed into each other or
 *    into failure: `unknown` means the log is absent, truncated by the 400-line
 *    tail window, or older than the add-on, and rendering that as broken just
 *    re-creates the same bug pointing the other way.
 */
export function stackVerdict(
  attentionCount: number,
  evidence: LogEvidence
): { label: string; color: "amber" | "red" | "emerald" | "muted"; Icon: typeof ShieldCheckIcon } {
  if (attentionCount > 0) {
    return {
      label: `${attentionCount} need attention`,
      color: "amber",
      Icon: AlertTriangleIcon,
    };
  }
  const fatal = evidence.log_excerpts.length;
  if (fatal > 0) {
    return {
      label: `${fatal} log failure${fatal === 1 ? "" : "s"}`,
      color: "red",
      Icon: AlertCircleIcon,
    };
  }
  switch (evidence.neural_rendering.state) {
    case "running":
      return { label: "Everything agrees", color: "emerald", Icon: ShieldCheckIcon };
    case "stalled":
      return { label: "Ran, then stopped", color: "amber", Icon: PauseIcon };
    case "unknown":
      return { label: "No proof it ran", color: "muted", Icon: CircleHelpIcon };
  }
}

/**
 * What the pane is showing.
 *
 * Eight of these are slots. The other three are whole-stack views that are
 * deliberately *not* slots: adoption is about the folder rather than any one
 * choice in it, the log check is evidence rather than a diagnosis, and
 * "What Kalpa tracks" is the recovery path for everything else — which is why
 * it is reachable from the footer in every state rather than living in a row.
 */
type SelectionKey = Slot | "power" | "adoption" | "records" | "logs";

/** The first slot with a problem, worst first, so the pane opens on the thing
 *  most worth reading. */
function firstProblemSlot(stack: ClientStack): Slot | null {
  const withFindings = SLOT_ORDER.filter((slot) => findingsForSlot(slot, stack).length > 0);
  if (withFindings.length === 0) return null;
  return withFindings.reduce((worst, slot) =>
    LEVEL_ORDER[slotLevel(slot, stack)] < LEVEL_ORDER[slotLevel(worst, stack)] ? slot : worst
  );
}

function computeDefaultSelection(
  stack: ClientStack | null,
  plan: AdoptionPlan | null,
  adoptionDismissed: boolean
): SelectionKey | null {
  if (!stack || stack.is_empty) return null;
  if (plan && !plan.already_managed && !adoptionDismissed) return "adoption";
  // A healthy stack opens on ReShade: it is the top of the load order and the
  // one slot every other slot depends on, so it is the most useful thing to be
  // looking at when there is nothing wrong.
  //
  // "Filled **or** required", not filled alone. Presence stopped being the
  // whole story when the need axis arrived: on the direct path the motion,
  // preset and tuning slots are correctly empty, so a filled-only search walks
  // straight past them — but it also walks past a *required* slot that is empty,
  // which is the one the user most needs to land on. Requiring either means the
  // panel opens on something the live path actually cares about.
  return (
    firstProblemSlot(stack) ??
    SLOT_ORDER.find((slot) => slotFilled(slot, stack) || slotNeed(slot, stack) === "required") ??
    "reshade"
  );
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

  const [adoptionDismissed, setAdoptionDismissed] = useState(false);

  // Adoption flow. Reset per install alongside everything else below.
  const [keepCopies, setKeepCopies] = useState(true);
  const [adopting, setAdopting] = useState(false);
  const [adoptError, setAdoptError] = useState<string | null>(null);
  const [forgetConfirming, setForgetConfirming] = useState(false);
  const [forgetting, setForgetting] = useState(false);
  const [forgetError, setForgetError] = useState<string | null>(null);
  const [logEvidence, setLogEvidence] = useState<LogEvidence>(NO_LOG_EVIDENCE);

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
  const selectedDirRef = useRef<string | null>(selectedDir);
  const openRef = useRef(open);
  const mountedRef = useRef(true);
  const mutationId = useRef(0);
  const pendingMutationRef = useRef<{
    id: number;
    clientDir: string;
    generation: number;
    label: string;
  } | null>(null);
  const [pendingMutation, setPendingMutation] = useState<{
    id: number;
    clientDir: string;
    generation: number;
    label: string;
  } | null>(null);

  // Event handlers consult refs so two clicks in the same render cannot both
  // start, while state mirrors the lock for accessible disabled controls. A
  // layout effect keeps prop/state mirrors current before any promise
  // continuation from the committed UI can run.
  useLayoutEffect(() => {
    selectedDirRef.current = selectedDir;
    openRef.current = open;
  }, [open, selectedDir]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      runToken.current += 1;
    };
  }, []);

  useEffect(() => {
    if (!open) runToken.current += 1;
  }, [open]);

  /** Clears every per-install control latch: rail selection/expand state,
   *  adoption dismissal and its checkbox, and the managed-file confirm/
   *  emergency-removal machinery. Called at every point the selected install
   *  changes or is reloaded, so none of it can survive onto a different
   *  folder or a stale content swap. */
  const resetInstallUiState = useCallback(() => {
    setSelection(null);
    setAdoptionDismissed(false);
    setKeepCopies(true);
    setAdopting(false);
    setAdoptError(null);
    setForgetConfirming(false);
    setForgetting(false);
    setForgetError(null);
    setLogEvidence(NO_LOG_EVIDENCE);
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
  /** What ReShade.log and dlss5-feed.log say: fatal matches, how many benign
   *  lines were suppressed, and whether Neural Rendering can be shown to have
   *  actually run.
   *
   *  A best-effort extra: a log Kalpa cannot read is not worth an error banner
   *  when the rest of the panel is fine, so this swallows its own failure — but
   *  it falls back to `NO_LOG_EVIDENCE`, whose Neural Rendering state is
   *  `unknown`, not to something that reads as healthy. A failed read must
   *  never be able to earn "Everything agrees". */
  const loadLogs = useCallback(async (clientDir: string, token: number) => {
    try {
      const report = await invokeOrThrow<ClientHealthReport>("inspect_eso_client", {
        clientDir,
      });
      if (runToken.current !== token) return;
      setLogEvidence({
        log_excerpts: report.log_excerpts ?? [],
        neural_rendering: report.neural_rendering ?? NO_LOG_EVIDENCE.neural_rendering,
        log_benign_suppressed: report.log_benign_suppressed ?? 0,
      });
    } catch {
      if (runToken.current !== token) return;
      setLogEvidence(NO_LOG_EVIDENCE);
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

  const isCurrentMutation = useCallback(
    (pending: { id: number; clientDir: string; generation: number }) =>
      mountedRef.current &&
      openRef.current &&
      pendingMutationRef.current?.id === pending.id &&
      selectedDirRef.current === pending.clientDir &&
      runToken.current === pending.generation,
    []
  );

  /** Serialize all client-stack writes and bind their completion to one
   * installation generation. The synchronous ref is the lock; React state is
   * only its UI projection. A successful command is not exposed to a child
   * until the same installation has been re-inspected. */
  const runMutation = useCallback(
    async <T,>(
      label: string,
      clientDir: string,
      operation: () => Promise<T>
    ): Promise<StackMutationResult<T>> => {
      if (!mountedRef.current || !openRef.current || selectedDirRef.current !== clientDir) {
        return { status: "stale" };
      }
      if (pendingMutationRef.current) return { status: "busy" };

      const pending = {
        id: ++mutationId.current,
        clientDir,
        generation: runToken.current,
        label,
      };
      pendingMutationRef.current = pending;
      setPendingMutation(pending);

      try {
        const value = await operation();
        if (!isCurrentMutation(pending)) return { status: "stale" };

        // Invalidate every read begun before the write and make the reload the
        // authoritative generation for this completion.
        pending.generation = ++runToken.current;
        await loadInstall(clientDir, pending.generation);
        if (!isCurrentMutation(pending)) return { status: "stale" };
        return { status: "committed", value };
      } finally {
        if (pendingMutationRef.current?.id === pending.id) {
          pendingMutationRef.current = null;
          if (mountedRef.current) setPendingMutation(null);
        }
      }
    },
    [isCurrentMutation, loadInstall]
  );

  const mutation: StackMutationCoordinator = {
    pending: pendingMutation !== null,
    pendingLabel: pendingMutation?.label ?? null,
    run: runMutation,
  };
  const mutationPending = pendingMutation !== null;

  const detect = useCallback(async () => {
    if (pendingMutationRef.current) return;
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
      selectedDirRef.current = first ? first.client_dir : null;
      setSelectedDir(first ? first.client_dir : null);
      setDetecting(false);
      if (first) await loadInstall(first.client_dir, token);
    } catch (e) {
      if (runToken.current !== token) return;
      setClients([]);
      selectedDirRef.current = null;
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

  // Write approval is session-scoped state in the Rust layer, so without this
  // it would outlive the window that asked for it: close the panel and the
  // client folder stays writable for the rest of the app's run. Revoking on
  // close keeps the approved window as narrow as the user's actual intent.
  useEffect(() => {
    if (!open) return;
    return () => {
      void invokeOrThrow("clear_game_install_path").catch(() => {
        // Nothing useful to do or say — the panel is already gone, and the
        // approval expires with the process regardless.
      });
    };
  }, [open]);

  const handleSelect = useCallback(
    (clientDir: string) => {
      if (pendingMutationRef.current || clientDir === selectedDirRef.current) return;
      const token = ++runToken.current;
      selectedDirRef.current = clientDir;
      setSelectedDir(clientDir);
      resetInstallUiState();
      void loadInstall(clientDir, token);
    },
    [loadInstall, resetInstallUiState]
  );

  const handleBrowse = useCallback(async () => {
    if (pendingMutationRef.current) return;
    let token: number | null = null;
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
      if (pendingMutationRef.current) return;
      token = ++runToken.current;
      const validated = await invokeOrThrow<EsoClientLocation>("validate_eso_client", {
        path: picked,
      });
      if (runToken.current !== token || pendingMutationRef.current) return;
      setClients((prev) => {
        const rest = (prev ?? []).filter((c) => c.client_dir !== validated.client_dir);
        return [validated, ...rest];
      });
      selectedDirRef.current = validated.client_dir;
      setSelectedDir(validated.client_dir);
      setDetectError(null);
      resetInstallUiState();
      await loadInstall(validated.client_dir, token);
    } catch (e) {
      if (
        !mountedRef.current ||
        pendingMutationRef.current ||
        (token !== null && runToken.current !== token)
      )
        return;
      setBrowseError(getTauriErrorMessage(e));
    } finally {
      if (mountedRef.current) setBrowsing(false);
    }
  }, [loadInstall, resetInstallUiState]);

  /** Register `clientDir` as approved for writes, for this session.
   *
   *  `client_write::begin_write` refuses to touch a folder the user has not
   *  explicitly approved, and detecting or inspecting an install is not
   *  approval — the whole point of the gate is that reading a folder must not
   *  grant permission to write to it. So approval is minted here, inside the
   *  handlers that only run when the user has clicked a specific action on a
   *  specific folder, and nowhere else. Approval is revoked when the panel
   *  closes. */
  const approveForWrite = useCallback(async (clientDir: string) => {
    await invokeOrThrow<EsoClientLocation>("set_game_install_path", { path: clientDir });
  }, []);

  const handleAdopt = useCallback(async () => {
    if (!selectedDir) return;
    setAdopting(true);
    setAdoptError(null);
    try {
      const result = await runMutation("Managing this client stack", selectedDir, async () => {
        await approveForWrite(selectedDir);
        return invokeOrThrow<AdoptionOutcome>("adopt_stack", {
          clientDir: selectedDir,
          keepCopies,
        });
      });
      if (result.status !== "committed") return;
      setSelection(null);
    } catch (e) {
      if (!mountedRef.current || !openRef.current || selectedDirRef.current !== selectedDir) return;
      setAdoptError(getTauriErrorMessage(e));
    } finally {
      if (mountedRef.current && selectedDirRef.current === selectedDir) setAdopting(false);
    }
  }, [approveForWrite, keepCopies, runMutation, selectedDir]);

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
    setForgetting(true);
    setForgetError(null);
    try {
      const result = await runMutation("Forgetting this managed stack", selectedDir, () =>
        invokeOrThrow<ForgetOutcome>("forget_stack", { clientDir: selectedDir })
      );
      if (result.status !== "committed") return;
      setForgetConfirming(false);
      // Re-listing is what flips the panel back to the unmanaged view, so the
      // adoption card must be offerable again rather than staying dismissed.
      setAdoptionDismissed(false);
    } catch (e) {
      if (!mountedRef.current || !openRef.current || selectedDirRef.current !== selectedDir) return;
      setForgetError(getTauriErrorMessage(e));
    } finally {
      if (mountedRef.current && selectedDirRef.current === selectedDir) setForgetting(false);
    }
  }, [runMutation, selectedDir]);

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
      setRemoving(true);
      setRemoveError(null);
      try {
        const result = await runMutation("Removing managed client files", selectedDir, async () => {
          await approveForWrite(selectedDir);
          return invokeOrThrow<UninstallOutcome>("uninstall_managed_client_files", {
            clientDir: selectedDir,
            relativePaths: paths,
          });
        });
        if (result.status !== "committed") return;
        setRemoveOutcome(result.value);
        setRemoveMode(null);
        setSelectedPaths(new Set());
      } catch (e) {
        if (!mountedRef.current || !openRef.current || selectedDirRef.current !== selectedDir)
          return;
        setRemoveError(getTauriErrorMessage(e));
      } finally {
        if (mountedRef.current && selectedDirRef.current === selectedDir) setRemoving(false);
      }
    },
    [approveForWrite, runMutation, selectedDir]
  );

  const handleEmergencyRemove = useCallback(
    async (fileName: string) => {
      if (!selectedDir) return;
      setEmergencyBusy(true);
      setEmergencyError(null);
      try {
        const result = await runMutation(
          "Removing an unmanaged injector",
          selectedDir,
          async () => {
            await approveForWrite(selectedDir);
            return invokeOrThrow<EmergencyRemoval>("emergency_remove_injector", {
              clientDir: selectedDir,
              fileName,
              confirmation: emergencyConfirmInput,
            });
          }
        );
        if (result.status !== "committed") return;
        setEmergencyResult(result.value);
        setEmergencyTarget(null);
        setEmergencyConfirmInput("");
      } catch (e) {
        if (!mountedRef.current || !openRef.current || selectedDirRef.current !== selectedDir)
          return;
        setEmergencyError(getTauriErrorMessage(e));
      } finally {
        if (mountedRef.current && selectedDirRef.current === selectedDir) setEmergencyBusy(false);
      }
    },
    [approveForWrite, emergencyConfirmInput, runMutation, selectedDir]
  );

  const effectiveSelection = useMemo(
    () => selection ?? computeDefaultSelection(stack, plan, adoptionDismissed),
    [selection, stack, plan, adoptionDismissed]
  );

  // The injector this stack loads is the one that would otherwise be flagged
  // as an "unmanaged orphan" — offering to quarantine the ReShade the user
  // just asked Kalpa to manage was the old panel's worst moment.
  const hideEmergency = useMemo(
    () => Boolean(plan?.already_managed && stack?.items.some((item) => item.role === "injector")),
    [plan, stack]
  );

  const selectedClient = useMemo(
    () => clients?.find((client) => client.client_dir === selectedDir) ?? null,
    [clients, selectedDir]
  );

  /**
   * Findings above `info`, across every slot.
   *
   * No longer the header's one-glance answer on its own — it never should have
   * been. It is the *first* of three gates in `stackVerdict`, which is where
   * the header's answer is now decided: this count says nothing about the logs
   * (no finding is ever emitted from a log rule) and nothing about whether
   * Neural Rendering actually ran, and a zero here was being rendered as
   * "Everything agrees" over a stack that could not work.
   */
  const attentionCount = useMemo(
    () => stack?.findings.filter((f) => f.level !== "info" && f.level !== "ok").length ?? 0,
    [stack]
  );

  const verdict = useMemo(
    () => stackVerdict(attentionCount, logEvidence),
    [attentionCount, logEvidence]
  );

  const busy = detecting || stackLoading || browsing || mutationPending;
  const handleClose = useCallback(() => {
    if (!pendingMutationRef.current) onClose();
  }, [onClose]);

  return (
    <Dialog open={open} onOpenChange={(next) => !next && handleClose()}>
      <DialogContent
        className="flex h-[calc(100dvh-2rem)] max-h-[760px] flex-col gap-0 sm:max-w-5xl"
        showCloseButton={!mutationPending}
      >
        {/* One header row instead of three stacked ones.

            The panel used to spend 184px before any content appeared: a
            DialogHeader with a description restating the title, then an install
            row, then a status strip — in a dialog whose whole content region was
            267px. Title, which install, and how that install is doing are all
            one thought, so they are one line. The description stays for screen
            readers via `aria-describedby`; it said what the title says. */}
        <DialogHeader className="shrink-0 pt-4 pb-3">
          <div className="flex h-7 items-center gap-2">
            <DialogTitle className="shrink-0 tracking-[-0.01em]">Graphics stack</DialogTitle>

            {clients && clients.length > 0 && (
              <>
                <div className="h-4 shrink-0 border-l border-structure-10" />
                {clients.length > 1 ? (
                  // Several installs used to be a bounded scrolling list that
                  // could cost 128px on its own. A select costs 24.
                  <Select
                    value={selectedDir ?? undefined}
                    disabled={browsing || mutationPending}
                    onValueChange={(next) => next && handleSelect(next)}
                  >
                    <SelectTrigger size="sm" className="h-6 w-auto max-w-[220px] gap-1 text-xs">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {clients.map((client) => (
                        <SelectItem key={client.client_dir} value={client.client_dir}>
                          {shortDirName(client.client_dir)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                ) : (
                  <p
                    // Medium, not semibold. Which install this is sits one rank
                    // below the panel's own title, and at 13/600 next to a
                    // 16/600 title the two read as a single compound heading.
                    className="min-w-0 truncate font-heading text-[13px] font-medium"
                    title={clients[0]!.exe_path}
                  >
                    {shortDirName(clients[0]!.client_dir)}
                  </p>
                )}
                {selectedClient && (
                  <InfoPill color={SOURCE_PILL[selectedClient.source].color}>
                    {SOURCE_PILL[selectedClient.source].label}
                  </InfoPill>
                )}
                <Button
                  variant="ghost"
                  size="xs"
                  className="shrink-0"
                  disabled={browsing || mutationPending}
                  onClick={() => void handleBrowse()}
                >
                  {browsing ? <Spinner className="size-3.5" /> : <SearchIcon />}
                  Change
                </Button>
              </>
            )}

            <div className="ml-auto flex shrink-0 items-center gap-2">
              {/* Icon and word, never colour alone: `status-*` is reseeded per
                  theme and five shipped themes are light or high-contrast.
                  Clicking it opens the evidence rather than leaving the user to
                  work out what "No proof it ran" is based on. */}
              {stack && !stack.is_empty && (
                <button
                  type="button"
                  className="shrink-0"
                  onClick={() => setSelection("logs")}
                  aria-label={`${verdict.label} — show the log evidence`}
                >
                  <InfoPill color={verdict.color}>
                    <verdict.Icon aria-hidden className="size-3" />
                    {verdict.label}
                  </InfoPill>
                </button>
              )}
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Refresh"
                disabled={busy}
                onClick={() => void detect()}
              >
                {busy ? <Spinner className="size-3.5" /> : <RefreshCwIcon />}
              </Button>
            </div>
          </div>
          <DialogDescription className="sr-only">
            The ReShade and DLSS 5 setup in your ESO game folder — what is in each slot, and what
            you can put there instead.
          </DialogDescription>
        </DialogHeader>

        {/* Does NOT scroll. A pane can only be given a height if every ancestor
            between it and the fixed-height dialog is a flex box with `min-h-0`;
            scrolling here would put the rail and the pane back in one
            scrollport, which is the bug this replaced. */}
        <div className="flex min-h-0 flex-1 flex-col gap-3 pt-3">
          {mutationPending && (
            <div
              className="flex shrink-0 items-center gap-2 rounded-lg border border-status-info/20 bg-status-info/[0.04] px-3 py-2 text-xs text-muted-foreground"
              role="status"
            >
              <Spinner className="size-3.5" />
              <span>{pendingMutation?.label}… Client controls are temporarily locked.</span>
            </div>
          )}
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

          {browseError && clients && clients.length > 0 && (
            <p className="shrink-0 text-xs text-status-danger" role="alert">
              {browseError}
            </p>
          )}

          {!detecting && clients && clients.length > 0 && (
            <>
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

              {!stackLoading && !stackError && stack && stack.is_empty && (
                <div className="min-h-0 flex-1 overflow-y-auto pr-1">
                  <StockClientCard />
                </div>
              )}

              {!stackLoading && !stackError && stack && !stack.is_empty && (
                <fieldset disabled={mutationPending || browsing} className="contents">
                  <StackBody
                    stack={stack}
                    plan={plan}
                    planLoading={planLoading}
                    planError={planError}
                    effectiveSelection={effectiveSelection}
                    onSelect={(key) => setSelection(key)}
                    keepCopies={keepCopies}
                    onToggleKeepCopies={() => setKeepCopies((v) => !v)}
                    adopting={adopting}
                    adoptError={adoptError}
                    onAdopt={() => void handleAdopt()}
                    onDismissAdoption={handleDismissAdoption}
                    managedLoading={managedLoading}
                    managedError={managedError}
                    managedInventory={managedInventory}
                    logEvidence={logEvidence}
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
                    mutation={mutation}
                  />
                </fieldset>
              )}
            </>
          )}
        </div>
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
/* The body                                                                  */
/* -------------------------------------------------------------------------- */

interface StackBodyProps {
  stack: ClientStack;
  plan: AdoptionPlan | null;
  planLoading: boolean;
  planError: string | null;
  effectiveSelection: SelectionKey | null;
  onSelect: (key: SelectionKey) => void;
  keepCopies: boolean;
  onToggleKeepCopies: () => void;
  adopting: boolean;
  adoptError: string | null;
  onAdopt: () => void;
  onDismissAdoption: () => void;
  managedLoading: boolean;
  managedError: string | null;
  managedInventory: ManagedInventory | null;
  logEvidence: LogEvidence;
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
  mutation: StackMutationCoordinator;
}

/**
 * Strip, rail, pane — one screen, one scroller.
 *
 * The Overview/Details tab split is gone. It existed because the rail was a
 * pipeline diagram with no actions on it, so a second, friendlier surface had
 * to be built in front. Once each row became a slot that owns its findings and
 * its controls, the second surface had nothing left to say — and the tab strip
 * was 36px of a 532px dialog spent on a choice the user should not have to
 * make.
 *
 * The height chain is the fragile part and it is written out deliberately:
 * `DialogContent` is the only fixed height, and **every** wrapper between it
 * and the pane's scroller is a flex box with `min-h-0`. Break one link and the
 * pane stops being bounded — that is how 1870px of content once rendered
 * inside a 532px dialog. The rail is `shrink-0` and never scrolls: it and the
 * pane were siblings in one scrollport once, and because the rail is the
 * taller of the two, scrolling to a low row carried the pane 1372px above the
 * viewport.
 */
export function StackBody(props: StackBodyProps) {
  const { stack, effectiveSelection, onSelect } = props;
  const paneScrollRef = useRef<HTMLDivElement>(null);

  // The pane would otherwise open at whatever offset the previous selection
  // left behind, which for a long slot reads as a blank pane.
  useEffect(() => {
    if (paneScrollRef.current) {
      paneScrollRef.current.scrollTop = 0;
    }
  }, [effectiveSelection]);

  // The rail owns the whole-stack views too now, so the only thing between the
  // header and the two columns is this row. There is no strip and no footer:
  // both were chrome spent restating what a 32px rail row says, in a dialog
  // whose content region was half its own height.
  return (
    <div className="flex min-h-0 flex-1 gap-4">
      <SlotRail
        stack={stack}
        selected={
          effectiveSelection === "adoption"
            ? "records"
            : (effectiveSelection as Slot | StackView | null)
        }
        onSelect={onSelect}
        isManaged={props.isManaged}
        trackedCount={props.managedInventory?.files.length ?? null}
        logCount={props.logEvidence.log_excerpts.length}
        nrState={props.logEvidence.neural_rendering.state}
      />
      <div ref={paneScrollRef} className="min-h-0 min-w-0 flex-1 overflow-y-auto pr-1">
        <DetailPane {...props} />
      </div>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Detail pane                                                               */
/* -------------------------------------------------------------------------- */

/**
 * Kalpa's own records for this folder, plus uninstall and the
 * emergency removal path.
 *
 * Its own component because both tabs render it: the Details rail as a
 * layer, and the Overview inside a collapsed "Remove or stop managing"
 * section. It is the recovery path for everything the panel can do, so it
 * has to stay reachable from wherever the user actually is.
 */
function RecordsSection(props: StackBodyProps) {
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

/**
 * What the pane shows for the current selection.
 *
 * Eight of the branches are slots and go through one component; the other
 * three are the whole-stack views that were never slots. There is no
 * per-slot special-casing left here — that all moved into `SlotPane`, which
 * is why this reads as navigation rather than as a second layout.
 */
function DetailPane(props: StackBodyProps) {
  const { stack, plan, planLoading, planError, effectiveSelection } = props;

  if (effectiveSelection === "power") {
    return <StackPowerCard clientDir={stack.client_dir} stack={stack} mutation={props.mutation} />;
  }

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

  if (effectiveSelection === "logs") return <LogSignals evidence={props.logEvidence} />;
  if (effectiveSelection === "records") return <RecordsSection {...props} />;
  if (!effectiveSelection) {
    return <p className="text-xs text-muted-foreground">Pick a slot to see what is in it.</p>;
  }

  return (
    <SlotPane
      slot={effectiveSelection}
      stack={stack}
      mutation={props.mutation}
      onOpenGuide={(url) => void openGuide(url)}
    />
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
      {plan.stack_switched_off && (
        <p className="text-xs text-status-warning" role="status">
          This stack is switched off, so what is in the folder right now is the game&apos;s own
          files rather than yours. Switch it back on before managing it.
        </p>
      )}
      {adoptError && (
        <p className="text-xs text-status-danger" role="alert">
          {adoptError}
        </p>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" disabled={adopting || plan.stack_switched_off} onClick={onAdopt}>
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

/**
 * How the three Neural Rendering evidence states are worded.
 *
 * They are three genuinely different answers and collapsing any two of them
 * re-creates the bug this view exists to fix. `running` is the only one that is
 * proof: `EvaluateFeature succeeded: evaluation=N` advances once per rendered
 * frame, so a climbing counter cannot be produced by an add-on that merely
 * loaded. `stalled` is suspicious and nothing more — a single occurrence lands
 * there. `unknown` is neither working nor failing, and it is by far the most
 * common: the log may be missing, from a session before the add-on was
 * installed, or simply longer than the 400-line tail window Kalpa reads.
 */
const NR_STATE_COPY: Record<
  NeuralRenderingState,
  {
    title: string;
    color: "emerald" | "amber" | "muted";
    Icon: typeof ShieldCheckIcon;
    body: string;
  }
> = {
  running: {
    title: "Neural Rendering ran",
    color: "emerald",
    Icon: ActivityIcon,
    body: "ReShade logged its per-frame evaluation counter climbing. That advances once per rendered frame, so it is real proof rather than an add-on that merely loaded.",
  },
  stalled: {
    title: "Neural Rendering started, then stopped",
    color: "amber",
    Icon: PauseIcon,
    body: "The evaluation counter appears but never advances. That is suspicious and it is not proof of anything on its own — one line from the moment the add-on initialised looks exactly like this.",
  },
  unknown: {
    title: "No evidence either way",
    color: "muted",
    Icon: CircleHelpIcon,
    body: "Kalpa found no evaluation line at all. That is not the same as broken: the log may be missing, may be from a session before the add-on was installed, or the line may have scrolled out of the 400 lines Kalpa reads from the end. Launch the game, then refresh.",
  },
};

/**
 * What the logs actually say — positive evidence first, then fatal matches.
 *
 * The order is the correction. This view used to be failure matches and nothing
 * else, so an empty file rendered as an empty page and the rail beside it said
 * "Nothing matched", which reads as an all-clear. It is not one: no
 * `HealthFinding` is emitted from a log rule at all, so the log is the *only*
 * place the `LoadFromDllMain` misconfiguration is visible, and "nothing matched"
 * over a log Kalpa never managed to read says the same thing as "nothing
 * matched" over a clean one.
 *
 * Benign lines are counted rather than discarded. Six distinct ERROR and WARN
 * lines appear on a *working* RenoDX DLSS-NR setup — the D3D11 proxy declining,
 * the absent Streamline interposer, four NVNGX vtable hooks — and surfacing
 * them is how a healthy stack got triaged as a broken one. Saying how many were
 * ignored keeps them findable by anyone who opens the raw log and sees them.
 */
function LogSignals({ evidence }: { evidence: LogEvidence }) {
  const byFile = useMemo(() => {
    const groups = new Map<string, LogExcerpt[]>();
    for (const excerpt of evidence.log_excerpts) {
      const list = groups.get(excerpt.file) ?? [];
      list.push(excerpt);
      groups.set(excerpt.file, list);
    }
    return Array.from(groups.entries());
  }, [evidence.log_excerpts]);

  const nr = evidence.neural_rendering;
  const copy = NR_STATE_COPY[nr.state];
  const benign = evidence.log_benign_suppressed;

  return (
    <section aria-labelledby="client-health-logs" className="space-y-3">
      <SectionHeader id="client-health-logs">Log signals</SectionHeader>

      <GlassPanel variant="subtle" className="space-y-2 p-3">
        <div className="flex flex-wrap items-center gap-2">
          <InfoPill color={copy.color}>
            <copy.Icon aria-hidden className="size-3" />
            {copy.title}
          </InfoPill>
          {nr.samples > 0 && (
            <span className="font-mono text-[11px] tabular-nums text-muted-foreground">
              {nr.samples} evaluation line{nr.samples === 1 ? "" : "s"}
              {nr.first_evaluation !== null &&
                nr.last_evaluation !== null &&
                `, counter ${nr.first_evaluation} → ${nr.last_evaluation}`}
            </span>
          )}
        </div>
        <p className="max-w-[72ch] text-xs leading-relaxed text-muted-foreground">{copy.body}</p>
      </GlassPanel>

      {byFile.length === 0 ? (
        // Never "Nothing matched" on its own. The honest statement is what was
        // checked and what was deliberately ignored, not silence.
        <p className="max-w-[72ch] text-xs leading-relaxed text-muted-foreground">
          No known failure signatures in your ReShade and DLSS 5 logs
          {benign > 0
            ? `; ${benign} line${benign === 1 ? "" : "s"} that a working setup also writes were ignored.`
            : "."}{" "}
          Kalpa only knows the signatures it has been taught, so this is the absence of a known
          failure rather than a clean bill of health.
        </p>
      ) : (
        <>
          <p className="max-w-[72ch] text-xs leading-relaxed text-muted-foreground">
            Lines matching a known failure. Logs are append-only, so a line here may be from a
            problem you have already fixed — check the timestamps in the file before chasing it.
            {benign > 0 &&
              ` ${benign} further line${benign === 1 ? "" : "s"} that a working setup also writes were ignored.`}
          </p>
          {byFile.map(([file, lines]) => (
            <GlassPanel key={file} variant="subtle" className="space-y-2 p-3">
              <p className="font-mono text-[11px] text-muted-foreground">{file}</p>
              <ul className="space-y-1.5">
                {lines.map((excerpt, index) => (
                  <li key={`${excerpt.rule}-${index}`} className="space-y-0.5">
                    <InfoPill color="red">
                      <AlertCircleIcon aria-hidden className="size-3" />
                      {excerpt.rule}
                    </InfoPill>
                    <p className="break-words font-mono text-[11px] text-muted-foreground">
                      {excerpt.line}
                    </p>
                  </li>
                ))}
              </ul>
            </GlassPanel>
          ))}
        </>
      )}
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

  /** Both derived from the inventory rather than passed down: a parked file is
   *  one the backend already reports as `parked`, and a kept copy is exactly an
   *  entry that has a displaced backup behind it. */
  const switchedOff = files.some((file) => file.state === "parked");
  const keptCopies = files.filter((file) => file.restores_backup).length;

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
            game update changes something.{" "}
            {switchedOff
              ? "You have switched it off, so some of these files are currently parked."
              : "It has not modified any of it."}
          </p>
          {!forgetConfirming &&
            (switchedOff ? (
              /* The records being dropped are the ones that say which parked
                 file belongs to which original. Kalpa can still put them back
                 from the folder alone, but telling the user "your stack keeps
                 working" while it is switched off would be a lie. */
              <p className="text-xs leading-relaxed text-status-warning">
                Switch the stack back on before asking Kalpa to stop managing it — right now some of
                its files are parked, and these records are what describe them.
              </p>
            ) : (
              <Button variant="outline" size="sm" onClick={onRequestForget}>
                Stop managing
              </Button>
            ))}
          {forgetConfirming && (
            <div className="space-y-2">
              <p className="text-xs leading-relaxed text-muted-foreground">
                Kalpa will delete its records for this folder. Every file stays exactly where it is,
                your stack keeps working, and you can ask Kalpa to manage it again at any time.
              </p>
              {keptCopies > 0 && (
                <p className="text-xs leading-relaxed text-status-warning">
                  One caveat: the {keptCopies} kept {keptCopies === 1 ? "copy" : "copies"} of your
                  swapped runtimes stop being protected from Kalpa&apos;s own cleanup, so a later
                  install may reclaim that space. Your files in the game folder are untouched either
                  way.
                </p>
              )}
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
