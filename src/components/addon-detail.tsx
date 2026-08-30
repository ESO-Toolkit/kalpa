import { memo, useState, useMemo, useRef, useEffect, lazy, Suspense } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import type {
  AddonManifest,
  BatchConflictAddon,
  UpdateCheckResult,
  InstallResult,
  ConflictReport,
  FileDecision,
} from "../types";
import { PRESET_TAGS } from "../types";
import { Button } from "@/components/ui/button";
import { Alert } from "@/components/ui/alert";
import { GlassPanel } from "@/components/ui/glass-panel";
import { SectionHeader } from "@/components/ui/section-header";
import { InfoPill } from "@/components/ui/info-pill";
import { Tabs, TabsList, TabsTrigger, TabsContent, TabsIndicator } from "@/components/ui/tabs";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { getSetting } from "@/lib/store";
import { getDependencyPolicy } from "@/lib/dependency-policy";
import { useResolvePendingDeps } from "@/lib/dependency-prompt-context";
import { useEnsureEsoNotBlocking } from "@/lib/eso-running-context";
import { cn } from "@/lib/utils";
import { RichDescription } from "@/components/ui/rich-description";
import { SimpleTooltip } from "@/components/ui/tooltip";
import {
  ExternalLink,
  Trash2,
  Check,
  Power,
  Files,
  FileText,
  Globe,
  ScrollText,
} from "lucide-react";
import { Fade } from "@/components/animate-ui/primitives/effects/fade";
import { AnimatedCheckmark } from "@/components/ui/animated-checkmark";

// The Files tab and conflict resolver pull in heavier editor/diff surfaces.
// Keep them out of AddonDetail's first paint and let the packaged gate prove
// their emitted chunks load from the bundled app origin.
const AddonFileBrowser = lazy(() =>
  import("@/components/addon-file-browser").then((m) => ({ default: m.AddonFileBrowser }))
);

const UpdateConflictPanel = lazy(() =>
  import("@/components/update-conflict-panel").then((m) => ({ default: m.UpdateConflictPanel }))
);

// The ESOUI tab pulls in the screenshot carousel and description renderer, and
// the changelog dialog is only ever opened deliberately — neither belongs in
// AddonDetail's first paint.
const EsouiTab = lazy(() =>
  import("@/components/esoui-tab").then((m) => ({ default: m.EsouiTab }))
);

const ChangelogDialog = lazy(() =>
  import("@/components/changelog-dialog").then((m) => ({ default: m.ChangelogDialog }))
);

function relativeDate(ts: number): string {
  const diff = Date.now() - ts;
  if (diff < 0) return "Today"; // future timestamp (clock skew)
  const days = Math.floor(diff / 86400000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 30) return `${days} days ago`;
  return new Date(ts).toLocaleDateString();
}

interface AddonDetailProps {
  addon: AddonManifest | null;
  installedAddons: AddonManifest[];
  addonsPath: string;
  /** Rescan the addon list WITHOUT touching the selection. Dependency
   * install/remove changes other addons, so this pane must stay on screen —
   * App's scan deliberately re-resolves the selected addon across a rescan. */
  onRefresh: () => void;
  onRemoveAddon: (folderName: string) => void;
  onToggleDisable: (folderName: string, currentlyDisabled: boolean) => void;
  updateResult: UpdateCheckResult | null;
  onAddonUpdated: (esouiId: number) => void;
  onTagsChange: (folderName: string, tags: string[]) => void;
  isOffline?: boolean;
  pendingConflict?: BatchConflictAddon;
  onConflictResolved?: (folderName: string) => void;
}

function AddonDetailBase({
  addon,
  installedAddons,
  addonsPath,
  onRefresh,
  onRemoveAddon,
  onToggleDisable,
  updateResult,
  onAddonUpdated,
  onTagsChange,
  isOffline,
  pendingConflict,
  onConflictResolved,
}: AddonDetailProps) {
  const ensureEsoNotBlocking = useEnsureEsoNotBlocking();
  const resolvePendingDeps = useResolvePendingDeps();
  const [updating, setUpdating] = useState(false);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [updateSuccess, setUpdateSuccess] = useState(false);
  const [installingDep, setInstallingDep] = useState<string | null>(null);
  const [justInstalledDeps, setJustInstalledDeps] = useState<Set<string>>(new Set());
  const [removingDep, setRemovingDep] = useState<string | null>(null);
  const [customTagInput, setCustomTagInput] = useState("");
  const customTagRef = useRef<HTMLInputElement>(null);
  const [conflictReport, setConflictReport] = useState<ConflictReport | null>(null);
  // The ESOUI tab fetches on mount, so it must not mount until the user actually
  // opens that tab. Base UI unmounts inactive panels by default, but this flag
  // makes the "no request on addon select" guarantee independent of that.
  const [esouiTabOpened, setEsouiTabOpened] = useState(false);
  const [changelogOpen, setChangelogOpen] = useState(false);
  const [pendingConflictDismissed, setPendingConflictDismissed] = useState(false);
  // Per-file extraction progress for THIS addon's in-flight update, correlated
  // by operation id. Drives the "Extracting N of M" label and the Stop button.
  const [extractProgress, setExtractProgress] = useState<{ done: number; total: number } | null>(
    null
  );
  const [canStopUpdate, setCanStopUpdate] = useState(false);
  // The operation id lives in this component instance. App.tsx mounts AddonDetail
  // with key={folderName}, so selecting a different addon mid-update remounts and
  // drops this ref: the backend update keeps running (it's detached in
  // spawn_blocking) but its Stop control and progress are lost for the rest of
  // that update. Accepted known limitation — like the batch-flow and
  // download-phase Stop gaps — kept small here because the hashing fix shrinks the
  // motivating multi-minute window to seconds; lifting operation tracking into
  // App.tsx would be the fix if it becomes a real annoyance.
  const operationIdRef = useRef<string | null>(null);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    // Extraction events can burst far faster than the label can usefully
    // change. Coalesce to at most one setState per animation frame: keep only
    // the latest payload and flush it from a single scheduled rAF, so a large
    // addon's event flood can't queue up hundreds of renders. The flush
    // re-checks the operation id: a rAF can fire after `endOperation` already
    // reset the UI (completion resolving in the same frame, or the rAF frozen
    // while the window was hidden) and must not resurrect stale progress.
    let pendingProgress: { done: number; total: number; opId: string } | null = null;
    let rafId: number | null = null;
    const flush = () => {
      rafId = null;
      if (pendingProgress !== null && pendingProgress.opId === operationIdRef.current) {
        setCanStopUpdate(true);
        setExtractProgress({ done: pendingProgress.done, total: pendingProgress.total });
      }
      pendingProgress = null;
    };
    void listen<{ operationId: string; fileIndex: number; fileTotal: number }>(
      "update-progress",
      (event) => {
        if (event.payload.operationId && event.payload.operationId === operationIdRef.current) {
          pendingProgress = {
            done: event.payload.fileIndex,
            total: event.payload.fileTotal,
            opId: event.payload.operationId,
          };
          rafId ??= requestAnimationFrame(flush);
        }
      }
    )
      .then((un) => {
        if (disposed) un();
        else unlisten = un;
      })
      .catch((e) => console.error("[tauri:update-progress]", e));
    return () => {
      disposed = true;
      if (rafId !== null) cancelAnimationFrame(rafId);
      unlisten?.();
    };
  }, []);

  // Start a cancellable update: mint an operation id (correlates progress events
  // and lets Stop signal the backend) and reset progress. `endOperation` clears
  // both on completion. `handleStopUpdate` asks the backend to abort.
  const beginOperation = (): string => {
    const id = crypto.randomUUID();
    operationIdRef.current = id;
    setCanStopUpdate(false);
    setExtractProgress(null);
    return id;
  };
  const endOperation = () => {
    operationIdRef.current = null;
    setCanStopUpdate(false);
    setExtractProgress(null);
  };
  const handleStopUpdate = () => {
    const id = operationIdRef.current;
    if (id) void invokeOrThrow("cancel_update", { operationId: id }).catch(() => {});
  };
  const isCancellation = (e: unknown) => getTauriErrorMessage(e).includes("Update cancelled");

  // Every update this pane can start goes through here, so the dependency policy
  // is applied — and any dependencies the backend held back are surfaced — the
  // same way for auto-resolved conflicts, clean updates and both conflict panels.
  const applyUpdate = async (sessionId: string, decisions: FileDecision[]) => {
    const dependencyPolicy = await getDependencyPolicy();
    const result = await invokeOrThrow<InstallResult>("update_addon_with_decisions", {
      addonsPath,
      sessionId,
      decisions,
      operationId: beginOperation(),
      dependencyPolicy,
    });
    // Empty unless the policy is "ask". The picker is app-level and owns the
    // install + refresh from here, so this update's busy state can clear. Hand it
    // the same `addonsPath` this update ran against (closure-captured, so it stays
    // this instance's folder even if the user switches instance mid-update).
    void resolvePendingDeps(result.pendingDeps, addonsPath);
  };

  // Map of lowercased top-level folder name → its real on-disk spelling. ESO
  // resolves addon names case-insensitively, so membership tests must too (a
  // `LUIMedia` folder still satisfies a `LuiMedia` dependency). Keeping the real
  // spelling lets the Remove button delete the actual folder rather than the
  // dependency token's casing. Only holds top-level folders, so it answers "is
  // this a removable top-level addon" — required/optional satisfaction comes
  // from the backend (subfolder-aware) fields.
  const installedByLower = useMemo(
    () => new Map(installedAddons.map((a) => [a.folderName.toLowerCase(), a.folderName])),
    [installedAddons]
  );

  const dependents = useMemo(
    () =>
      addon
        ? installedAddons.filter((a) =>
            a.dependsOn.some((dep) => dep.name.toLowerCase() === addon.folderName.toLowerCase())
          )
        : [],
    [installedAddons, addon]
  );

  // Auto-dismiss update success banner after 5 seconds
  useEffect(() => {
    if (!updateSuccess) return;
    const timer = setTimeout(() => setUpdateSuccess(false), 5000);
    return () => clearTimeout(timer);
  }, [updateSuccess]);

  if (!addon) {
    return (
      <Fade
        transition={{ type: "spring", stiffness: 120, damping: 20 }}
        className="relative flex flex-1 flex-col items-center justify-center gap-4 text-muted-foreground px-8"
      >
        {/* Ambient glow behind icon */}
        <div className="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 h-[200px] w-[200px] rounded-full bg-primary/[0.04] blur-[60px]" />
        <div className="relative rounded-2xl bg-structure-03 border border-structure-06 p-5 shadow-[0_0_30px_color-mix(in_oklab,var(--primary)_3%,transparent)]">
          <FileText
            aria-hidden="true"
            className="size-10 text-muted-foreground/30"
            strokeWidth={1.2}
          />
        </div>
        <div className="relative text-center">
          <p className="font-heading text-sm font-medium text-foreground">No addon selected</p>
          <p className="mt-1 text-xs text-muted-foreground">
            Select an addon from the list to view details
          </p>
        </div>
      </Fade>
    );
  }

  const handleUpdate = async () => {
    if (!updateResult || !addon.esouiId) return;
    if (updating) return;
    // Set the busy guard before the async ESO check so a fast double-click can't
    // enter twice; clear it if the user cancels the warning.
    setUpdating(true);
    if (!(await ensureEsoNotBlocking())) {
      setUpdating(false);
      return;
    }
    setUpdateError(null);
    setConflictReport(null);
    try {
      const report = await invokeOrThrow<ConflictReport>("scan_update_conflicts", {
        addonsPath,
        folderName: addon.folderName,
        esouiId: addon.esouiId,
      });

      if (report.conflicts.length > 0) {
        const policy = await getSetting<"ask" | "keep_mine" | "take_update">(
          "conflictPolicy",
          "ask"
        );

        if (policy !== "ask") {
          const autoDecisions: FileDecision[] = [
            ...report.autoKeptFiles.map((p) => ({
              relativePath: p,
              action: "keep_mine" as const,
            })),
            ...report.conflicts.map((c) => ({
              relativePath: c.relativePath,
              action: policy,
            })),
          ];
          await applyUpdate(report.sessionId, autoDecisions);
          setUpdateSuccess(true);
          toast.success(`Updated ${addon.title}`);
          onAddonUpdated(updateResult.esouiId);
          return;
        }

        setConflictReport(report);
        setUpdating(false);
        return;
      }

      // No conflicts — proceed directly (preserve auto-kept files)
      const autoKeptDecisions: FileDecision[] = report.autoKeptFiles.map((p) => ({
        relativePath: p,
        action: "keep_mine" as const,
      }));
      await applyUpdate(report.sessionId, autoKeptDecisions);
      setUpdateSuccess(true);
      toast.success(`Updated ${addon.title}`);
      onAddonUpdated(updateResult.esouiId);
    } catch (e) {
      if (isCancellation(e)) {
        setConflictReport(null);
        toast.info(`Stopped updating ${addon.title}`, {
          description: "It may be partially updated — run the update again to finish.",
        });
        // Re-scan so the row reflects the on-disk truth (the update didn't finish).
        onAddonUpdated(updateResult.esouiId);
      } else {
        setUpdateError(getTauriErrorMessage(e));
      }
    } finally {
      endOperation();
      setUpdating(false);
    }
  };

  const handleConflictResolve = async (decisions: FileDecision[]) => {
    if (!conflictReport || !updateResult) return;
    // Busy guard: the panel's Apply button stays enabled during the update, so a
    // double-click would re-submit the same session and fail "Session not found"
    // once the first apply removes it. Mirrors handleUpdate.
    if (updating) return;
    setUpdating(true);
    // Re-check here too: ESO may have launched after the initial scan while the
    // conflict panel was open, so the earlier handleUpdate gate can be stale.
    if (!(await ensureEsoNotBlocking())) {
      setUpdating(false);
      return;
    }
    setUpdateError(null);
    try {
      await applyUpdate(conflictReport.sessionId, decisions);
      setConflictReport(null);
      setUpdateSuccess(true);
      toast.success(`Updated ${addon.title}`);
      onAddonUpdated(updateResult.esouiId);
    } catch (e) {
      if (isCancellation(e)) {
        // The backend deletes the pending session on cancel, so this panel's
        // sessionId is now dead — clear it (like the other cancel paths) so a
        // retry goes through the main Update button and a fresh scan rather than
        // re-applying a stale session and hitting "Session not found".
        setConflictReport(null);
        toast.info(`Stopped updating ${addon.title}`, {
          description: "It may be partially updated — run the update again to finish.",
        });
        onAddonUpdated(updateResult.esouiId);
      } else {
        setUpdateError(getTauriErrorMessage(e));
      }
    } finally {
      endOperation();
      setUpdating(false);
    }
  };

  const handleConflictSkip = () => {
    if (conflictReport) {
      void invokeOrThrow("cancel_pending_update", { sessionId: conflictReport.sessionId });
    }
    setConflictReport(null);
  };

  const handleInstallDep = async (depName: string) => {
    if (installingDep) return;
    setInstallingDep(depName);
    // Installing/updating a dependency also writes to the AddOns folder, so it needs
    // the same ESO-running gate — the game won't load it until /reloadui either way.
    if (!(await ensureEsoNotBlocking())) {
      setInstallingDep(null);
      return;
    }
    try {
      const result = await invokeOrThrow<InstallResult>("install_dependency", {
        addonsPath,
        depName,
      });
      const depCount = result.installedDeps.length;
      if (depCount > 0) {
        toast.success(
          `Installed ${depName} + ${depCount} ${depCount === 1 ? "dependency" : "dependencies"}`
        );
      } else {
        toast.success(`Installed ${depName}`);
      }
      setJustInstalledDeps((prev) => new Set(prev).add(depName));
      onRefresh(); // refresh addon list, keeping this addon selected
    } catch (e) {
      toast.error(`Failed to install ${depName}: ${getTauriErrorMessage(e)}`);
    } finally {
      setInstallingDep(null);
    }
  };

  const handleRemoveDep = async (depName: string) => {
    setRemovingDep(depName);
    try {
      await invokeOrThrow("remove_addon", {
        addonsPath,
        folderName: depName,
      });
      toast.success(`Removed ${depName}`);
      onRefresh(); // refresh addon list, keeping this addon selected
    } catch (e) {
      toast.error(`Failed to remove ${depName}: ${getTauriErrorMessage(e)}`);
    } finally {
      setRemovingDep(null);
    }
  };

  const submitCustomTag = () => {
    const tag = customTagInput.trim().toLowerCase();
    if (!tag) return;
    if (addon.tags.includes(tag)) {
      toast.info("Tag already added");
      setCustomTagInput("");
      return;
    }
    onTagsChange(addon.folderName, [...addon.tags, tag]);
    setCustomTagInput("");
  };

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <h2 className="font-heading text-xl font-semibold bg-gradient-to-r from-primary to-primary-hover bg-clip-text text-transparent">
        {addon.title}
      </h2>
      <div className="mt-1 mb-4 flex items-center gap-2 flex-wrap">
        <InfoPill color="muted">{addon.folderName}/</InfoPill>
        {addon.esouiId && <InfoPill color="sky">ESOUI #{addon.esouiId}</InfoPill>}
      </div>

      {updateSuccess ? (
        <GlassPanel
          variant="subtle"
          className="mb-4 flex items-center gap-2 border-status-success-strong/20! bg-status-success-strong/[0.04]! p-3"
        >
          <AnimatedCheckmark size={18} />
          <span className="text-sm text-status-success">Updated successfully</span>
        </GlassPanel>
      ) : updateResult?.hasUpdate ? (
        <GlassPanel
          variant="subtle"
          className="mb-4 flex items-center justify-between gap-3 border-status-warning-strong/20! bg-status-warning-strong/[0.04]! p-3"
        >
          <span className="text-sm text-status-warning">
            Update available: {updateResult.currentVersion} &rarr; {updateResult.remoteVersion}
          </span>
          {updating ? (
            <div className="flex items-center gap-2">
              <span className="text-xs tabular-nums text-muted-foreground">
                {extractProgress && extractProgress.total > 0
                  ? `Extracting ${extractProgress.done.toLocaleString()} / ${extractProgress.total.toLocaleString()}`
                  : "Updating…"}
              </span>
              <Button
                onClick={handleStopUpdate}
                disabled={!canStopUpdate}
                size="sm"
                variant="outline"
              >
                Stop
              </Button>
            </div>
          ) : (
            <div className="flex items-center gap-2">
              {addon.esouiId && (
                <SimpleTooltip
                  content={isOffline ? "Changelogs require an internet connection" : ""}
                >
                  <Button
                    onClick={() => setChangelogOpen(true)}
                    disabled={isOffline}
                    size="sm"
                    variant="outline"
                  >
                    <ScrollText className="size-4 mr-1.5" />
                    What&rsquo;s new
                  </Button>
                </SimpleTooltip>
              )}
              <SimpleTooltip content={isOffline ? "Updates require an internet connection" : ""}>
                <Button onClick={handleUpdate} disabled={isOffline} size="sm">
                  Update
                </Button>
              </SimpleTooltip>
            </div>
          )}
        </GlassPanel>
      ) : null}

      {changelogOpen && addon.esouiId && (
        <Suspense fallback={null}>
          <ChangelogDialog
            esouiId={addon.esouiId}
            title={addon.title}
            currentVersion={updateResult?.currentVersion}
            remoteVersion={updateResult?.remoteVersion}
            open={changelogOpen}
            onOpenChange={setChangelogOpen}
          />
        </Suspense>
      )}

      {updateError && (
        <Alert variant="destructive" className="mb-4">
          {updateError}
        </Alert>
      )}

      {conflictReport && (
        <div className="mb-4">
          <Suspense fallback={null}>
            <UpdateConflictPanel
              folderName={conflictReport.folderName}
              currentVersion={updateResult?.currentVersion ?? ""}
              updateVersion={conflictReport.updateVersion}
              conflicts={conflictReport.conflicts}
              autoKeptFiles={conflictReport.autoKeptFiles}
              safeFileCount={conflictReport.safeFiles.length}
              sessionId={conflictReport.sessionId}
              addonsPath={addonsPath}
              onResolve={handleConflictResolve}
              onSkip={handleConflictSkip}
            />
          </Suspense>
        </div>
      )}

      {!conflictReport && pendingConflict && !pendingConflictDismissed && (
        <div className="mb-4">
          <Suspense fallback={null}>
            <UpdateConflictPanel
              folderName={pendingConflict.folderName}
              currentVersion={updateResult?.currentVersion ?? addon.version}
              updateVersion={pendingConflict.updateVersion}
              conflicts={pendingConflict.conflicts}
              autoKeptFiles={pendingConflict.autoKeptFiles}
              safeFileCount={0}
              sessionId={pendingConflict.sessionId}
              addonsPath={addonsPath}
              onResolve={async (decisions) => {
                if (updating) return;
                setUpdating(true);
                setUpdateError(null);
                if (!(await ensureEsoNotBlocking())) {
                  setUpdating(false);
                  return;
                }
                try {
                  await applyUpdate(pendingConflict.sessionId, decisions);
                  toast.success(`Updated ${addon.title}`);
                  onConflictResolved?.(addon.folderName);
                  if (updateResult) onAddonUpdated(updateResult.esouiId);
                } catch (e) {
                  if (isCancellation(e)) {
                    setPendingConflictDismissed(true);
                    if (onConflictResolved) {
                      onConflictResolved(addon.folderName);
                    } else if (updateResult) {
                      onAddonUpdated(updateResult.esouiId);
                    }
                    toast.info(`Stopped updating ${addon.title}`, {
                      description: "It may be partially updated — run the update again to finish.",
                    });
                  } else {
                    setUpdateError(getTauriErrorMessage(e));
                  }
                } finally {
                  endOperation();
                  setUpdating(false);
                }
              }}
              onSkip={() => setPendingConflictDismissed(true)}
            />
          </Suspense>
        </div>
      )}

      <Tabs
        defaultValue="details"
        onValueChange={(value) => {
          if (value === "esoui") setEsouiTabOpened(true);
        }}
      >
        <TabsList>
          <TabsIndicator />
          <TabsTrigger value="details">Details</TabsTrigger>
          <TabsTrigger value="files">
            <Files className="h-3.5 w-3.5" />
            Files
          </TabsTrigger>
          {/* Side-loaded addons have no ESOUI page, so they get no tab at all. */}
          {addon.esouiId && (
            <TabsTrigger value="esoui">
              <Globe className="h-3.5 w-3.5" />
              ESOUI
            </TabsTrigger>
          )}
        </TabsList>

        <TabsContent value="details" className="pt-4">
          <GlassPanel variant="subtle" className="mb-6 p-3">
            <dl className="grid grid-cols-[120px_1fr] gap-x-4 gap-y-2 text-sm">
              {addon.author && (
                <>
                  <dt className="text-muted-foreground font-heading text-xs uppercase tracking-wider">
                    Author
                  </dt>
                  <dd>{addon.author}</dd>
                </>
              )}
              <dt className="text-muted-foreground font-heading text-xs uppercase tracking-wider">
                Version
              </dt>
              <dd>{addon.version || addon.addonVersion || "Unknown"}</dd>
              {addon.apiVersion.length > 0 && (
                <>
                  <dt className="text-muted-foreground font-heading text-xs uppercase tracking-wider">
                    API Version
                  </dt>
                  <dd>{addon.apiVersion.join(", ")}</dd>
                </>
              )}
              <dt className="text-muted-foreground font-heading text-xs uppercase tracking-wider">
                Type
              </dt>
              <dd>
                {addon.isLibrary ? (
                  <InfoPill color="emerald">Library</InfoPill>
                ) : (
                  <InfoPill color="gold">Addon</InfoPill>
                )}
              </dd>
              {addon.esouiLastUpdate > 0 && (
                <>
                  <dt className="text-muted-foreground font-heading text-xs uppercase tracking-wider">
                    Last Updated
                  </dt>
                  <dd>{relativeDate(addon.esouiLastUpdate)}</dd>
                </>
              )}
              {addon.installedAt && !Number.isNaN(Date.parse(addon.installedAt)) && (
                <>
                  <dt className="text-muted-foreground font-heading text-xs uppercase tracking-wider">
                    Downloaded
                  </dt>
                  <dd>{relativeDate(Date.parse(addon.installedAt))}</dd>
                </>
              )}
            </dl>
            {addon.esouiId && (
              <div className="mt-3 pt-3 border-t border-structure-06">
                <button
                  onClick={() => openUrl(`https://www.esoui.com/downloads/info${addon.esouiId}`)}
                  className="inline-flex items-center gap-1.5 rounded-md bg-accent-sky/10 px-3 py-1.5 text-xs font-medium text-accent-sky hover:bg-accent-sky/20 transition-colors"
                >
                  <ExternalLink className="size-3" />
                  View on ESOUI
                </button>
              </div>
            )}
          </GlassPanel>

          {/* Tags */}
          <div className="mb-5">
            <SectionHeader className="mb-2">Tags</SectionHeader>
            <div className="flex flex-wrap gap-1.5">
              {PRESET_TAGS.map((tag) => {
                const active = addon.tags.includes(tag);
                return (
                  <button
                    key={tag}
                    aria-label={`${active ? "Remove" : "Add"} tag: ${tag}`}
                    aria-pressed={active}
                    onClick={() => {
                      const next = active
                        ? addon.tags.filter((t) => t !== tag)
                        : [...addon.tags, tag];
                      onTagsChange(addon.folderName, next);
                    }}
                    className={cn(
                      "cursor-pointer rounded-md px-2.5 py-1 text-xs font-medium transition-all duration-150 border",
                      active
                        ? tag === "favorite"
                          ? "bg-primary/15 text-primary border-primary/25"
                          : tag === "broken"
                            ? "bg-status-danger-strong/15 text-status-danger border-status-danger-strong/25"
                            : tag === "testing"
                              ? "bg-status-warning-strong/15 text-status-warning border-status-warning-strong/25"
                              : tag === "essential"
                                ? "bg-status-success-strong/15 text-status-success border-status-success-strong/25"
                                : "bg-status-library-strong/15 text-status-library border-status-library-strong/25"
                        : "bg-structure-03 text-muted-foreground border-structure-06 hover:bg-structure-06 hover:text-foreground"
                    )}
                  >
                    {tag === "favorite" && (active ? "\u2605 " : "\u2606 ")}
                    {tag}
                  </button>
                );
              })}
              {/* Custom tags */}
              {addon.tags
                .filter((t) => !(PRESET_TAGS as readonly string[]).includes(t))
                .map((tag) => (
                  <span
                    key={tag}
                    className="inline-flex items-center gap-1 rounded-md bg-accent-sky/15 text-accent-sky border border-accent-sky/25 px-2.5 py-1 text-xs font-medium"
                  >
                    {tag}
                    <button
                      onClick={() =>
                        onTagsChange(
                          addon.folderName,
                          addon.tags.filter((t) => t !== tag)
                        )
                      }
                      className="cursor-pointer ml-0.5 text-accent-sky/60 hover:text-accent-sky transition-colors"
                      aria-label={`Remove tag ${tag}`}
                    >
                      &times;
                    </button>
                  </span>
                ))}
              {/* Add custom tag */}
              <form
                className="inline-flex items-center gap-1"
                onSubmit={(e) => {
                  e.preventDefault();
                  submitCustomTag();
                }}
              >
                <input
                  ref={customTagRef}
                  type="text"
                  value={customTagInput}
                  onChange={(e) => setCustomTagInput(e.target.value)}
                  placeholder="+ tag"
                  className="w-16 focus:w-24 transition-all duration-150 rounded-md bg-structure-03 border border-structure-06 px-2 py-1 text-xs text-foreground placeholder:text-muted-foreground outline-none focus:border-accent-sky/30 focus:bg-structure-05"
                />
                {customTagInput.trim() && (
                  <button
                    type="submit"
                    className="flex items-center justify-center size-6 rounded-md bg-accent-sky/10 text-accent-sky hover:bg-accent-sky/20 transition-colors text-xs font-bold"
                    aria-label="Add tag"
                  >
                    +
                  </button>
                )}
              </form>
            </div>
          </div>

          {addon.description && (
            <div className="mb-5">
              <SectionHeader className="mb-2">Description</SectionHeader>
              <RichDescription text={addon.description} />
            </div>
          )}

          {addon.dependsOn.length > 0 && (
            <div className="mb-5">
              <SectionHeader className="mb-2">Required Dependencies</SectionHeader>
              <div className="space-y-0.5">
                {addon.dependsOn.map((dep) => {
                  // satisfied = backend truth (accounts for bundled sub-modules in subfolders)
                  // removeTarget = real top-level folder spelling, if removable
                  const satisfied =
                    !addon.missingDependencies.includes(dep.name) ||
                    justInstalledDeps.has(dep.name);
                  const removeTarget = installedByLower.get(dep.name.toLowerCase());
                  const outdated = addon.outdatedDependencies.includes(dep.name);
                  const justInstalled = justInstalledDeps.has(dep.name);
                  return (
                    <div
                      key={dep.name}
                      className="flex items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-structure-03 transition-colors"
                    >
                      <span
                        className={cn(
                          "flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-[10px] font-bold",
                          outdated
                            ? "bg-status-warning-strong/15 text-status-warning"
                            : satisfied
                              ? "bg-status-success-strong/15 text-status-success"
                              : "bg-status-danger-strong/15 text-status-danger"
                        )}
                      >
                        {outdated ? "!" : satisfied ? "\u2713" : "!"}
                      </span>
                      <div className="flex-1 min-w-0">
                        <span className="truncate block">{dep.name}</span>
                        {dep.min_version !== null && (
                          <span
                            className={cn(
                              "text-[11px]",
                              outdated ? "text-status-warning" : "text-muted-foreground"
                            )}
                          >
                            v{dep.min_version}+{outdated ? " (outdated)" : ""}
                          </span>
                        )}
                      </div>
                      {satisfied ? (
                        <div className="flex items-center gap-1">
                          {outdated && (
                            <SimpleTooltip
                              content={
                                isOffline
                                  ? "Updates require an internet connection"
                                  : `Update ${dep.name}`
                              }
                            >
                              <button
                                className="shrink-0 cursor-pointer rounded bg-status-warning-strong/10 px-2 py-1 text-xs font-medium text-status-warning hover:bg-status-warning-strong/20 transition-colors disabled:opacity-50"
                                onClick={() => handleInstallDep(dep.name)}
                                disabled={installingDep === dep.name || isOffline}
                              >
                                {installingDep === dep.name ? (
                                  <span className="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-structure-10 border-t-status-warning" />
                                ) : justInstalled ? (
                                  <span className="flex items-center gap-1 text-status-success">
                                    <Check className="size-3" />
                                    Updated
                                  </span>
                                ) : (
                                  "Update"
                                )}
                              </button>
                            </SimpleTooltip>
                          )}
                          {removeTarget && (
                            <SimpleTooltip content={`Remove ${removeTarget}`}>
                              <button
                                className="shrink-0 cursor-pointer rounded p-1 text-muted-foreground/30 hover:bg-status-danger-strong/10 hover:text-status-danger transition-colors disabled:opacity-50"
                                onClick={() => handleRemoveDep(removeTarget)}
                                disabled={removingDep === removeTarget}
                              >
                                {removingDep === removeTarget ? (
                                  <span className="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-structure-10 border-t-status-danger" />
                                ) : (
                                  <Trash2 className="size-3.5" />
                                )}
                              </button>
                            </SimpleTooltip>
                          )}
                        </div>
                      ) : (
                        <SimpleTooltip
                          content={
                            isOffline
                              ? "Installs require an internet connection"
                              : `Install ${dep.name}`
                          }
                        >
                          <button
                            className="shrink-0 cursor-pointer rounded bg-accent-sky/10 px-2 py-1 text-xs font-medium text-accent-sky hover:bg-accent-sky/20 transition-colors disabled:opacity-50"
                            onClick={() => handleInstallDep(dep.name)}
                            disabled={installingDep === dep.name || isOffline}
                          >
                            {installingDep === dep.name ? (
                              <span className="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-structure-10 border-t-accent-sky" />
                            ) : justInstalled ? (
                              <span className="flex items-center gap-1 text-status-success">
                                <Check className="size-3" />
                                Installed
                              </span>
                            ) : (
                              "Install"
                            )}
                          </button>
                        </SimpleTooltip>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          {addon.optionalDependsOn.length > 0 && (
            <div className="mb-5">
              <SectionHeader className="mb-2">Optional Dependencies</SectionHeader>
              <div className="space-y-0.5">
                {addon.optionalDependsOn.map((dep) => {
                  // present = backend truth (subfolder-aware, case-insensitive).
                  // removeTarget = real top-level folder spelling, if removable.
                  const present =
                    !addon.missingOptionalDependencies.includes(dep.name) ||
                    justInstalledDeps.has(dep.name);
                  const removeTarget = installedByLower.get(dep.name.toLowerCase());
                  const justInstalled = justInstalledDeps.has(dep.name);
                  return (
                    <div
                      key={dep.name}
                      className="flex items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-structure-03 transition-colors"
                    >
                      <span
                        className={cn(
                          "flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-[10px]",
                          present
                            ? "bg-status-success-strong/15 text-status-success font-bold"
                            : "bg-structure-04 text-muted-foreground"
                        )}
                      >
                        {present ? "\u2713" : "\u2013"}
                      </span>
                      <div className={cn("flex-1 min-w-0", !present && "text-muted-foreground")}>
                        <span className="truncate block">{dep.name}</span>
                        {dep.min_version !== null && (
                          <span className="text-xs text-muted-foreground">v{dep.min_version}+</span>
                        )}
                      </div>
                      {present ? (
                        removeTarget ? (
                          <SimpleTooltip content={`Remove ${removeTarget}`}>
                            <button
                              className="shrink-0 cursor-pointer rounded p-1 text-muted-foreground/30 hover:bg-status-danger-strong/10 hover:text-status-danger transition-colors disabled:opacity-50"
                              onClick={() => handleRemoveDep(removeTarget)}
                              disabled={removingDep === removeTarget}
                            >
                              {removingDep === removeTarget ? (
                                <span className="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-structure-10 border-t-status-danger" />
                              ) : (
                                <Trash2 className="size-3.5" />
                              )}
                            </button>
                          </SimpleTooltip>
                        ) : null
                      ) : (
                        <SimpleTooltip
                          content={
                            isOffline
                              ? "Installs require an internet connection"
                              : `Install ${dep.name}`
                          }
                        >
                          <button
                            className="shrink-0 cursor-pointer rounded bg-accent-sky/10 px-2 py-1 text-xs font-medium text-accent-sky hover:bg-accent-sky/20 transition-colors disabled:opacity-50"
                            onClick={() => handleInstallDep(dep.name)}
                            disabled={installingDep === dep.name || isOffline}
                          >
                            {installingDep === dep.name ? (
                              <span className="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-structure-10 border-t-accent-sky" />
                            ) : justInstalled ? (
                              <span className="flex items-center gap-1 text-status-success">
                                <Check className="size-3" />
                                Installed
                              </span>
                            ) : (
                              "Install"
                            )}
                          </button>
                        </SimpleTooltip>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          )}
        </TabsContent>

        <TabsContent value="files" className="pt-4">
          <Suspense
            fallback={
              <div className="flex items-center justify-center py-12 text-sm text-muted-foreground">
                <div className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-structure-10 border-t-primary" />
                Loading files...
              </div>
            }
          >
            <AddonFileBrowser addonsPath={addonsPath} folderName={addon.folderName} />
          </Suspense>
        </TabsContent>

        {addon.esouiId && (
          <TabsContent value="esoui" className="pt-4">
            {esouiTabOpened && (
              <Suspense
                fallback={
                  <div className="flex items-center justify-center py-12 text-sm text-muted-foreground">
                    <div className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-structure-10 border-t-primary" />
                    Loading details...
                  </div>
                }
              >
                <EsouiTab
                  esouiId={addon.esouiId}
                  isOffline={isOffline}
                  installedVersion={updateResult?.currentVersion ?? addon.version}
                />
              </Suspense>
            )}
          </TabsContent>
        )}
      </Tabs>

      <div className="mt-6 border-t border-structure-06 pt-4 space-y-3">
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            onClick={() => onToggleDisable(addon.folderName, addon.disabled)}
            className={cn(
              addon.disabled
                ? "border-status-success-strong/25 text-status-success hover:bg-status-success-strong/10"
                : "border-status-warning-strong/25 text-status-warning hover:bg-status-warning-strong/10"
            )}
          >
            <Power className="size-4 mr-1.5" />
            {addon.disabled ? "Enable Addon" : "Disable Addon"}
          </Button>
        </div>
        {addon.disabled && dependents.length > 0 && (
          <p className="text-xs text-status-warning">
            {dependents.map((d) => d.title).join(", ")}{" "}
            {dependents.length === 1 ? "depends" : "depend"} on this addon and may not work.
          </p>
        )}

        {!addon.disabled && dependents.length > 0 && (
          <p className="text-xs text-status-warning">
            {dependents.map((d) => d.title).join(", ")}{" "}
            {dependents.length === 1 ? "depends" : "depend"} on this addon.
          </p>
        )}
        <Button variant="destructive" onClick={() => onRemoveAddon(addon.folderName)}>
          Remove Addon
        </Button>
      </div>
    </div>
  );
}

// Memoized: bails out of App re-renders it doesn't consume (search keystrokes,
// update-progress events, dialog toggles).
export const AddonDetail = memo(AddonDetailBase);
