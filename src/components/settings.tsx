import { useState, useEffect } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { getSetting, setSetting, setSettings } from "@/lib/store";
import { getTauriErrorMessage, invokeOrThrow, invokeResult } from "@/lib/tauri";
import { exampleAddonsPath } from "@/lib/platform";
import { FEEDBACK_DISCORD_URL, FEEDBACK_ISSUES_URL, openFeedbackUrl } from "@/lib/feedback";
import {
  clearSkippedDependencies,
  getSkippedDependencies,
  getAskRequiredDependenciesOnly,
  setAskRequiredDependenciesOnly,
  DEFAULT_ASK_REQUIRED_ONLY,
  getDependencyPolicy,
  DEFAULT_DEPENDENCY_POLICY,
  // Aliased to keep "set" from reading like local state: this one writes to disk.
  setDependencyPolicy as saveDependencyPolicy,
  type DependencyPolicy,
} from "@/lib/dependency-policy";
import { useConfirmedSetting } from "@/hooks/use-confirmed-setting";
import { useOptimisticSetting } from "@/hooks/use-optimistic-setting";
import type { AuthUser, CopyAddonsResult, GameInstance, ImportResult } from "../types";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Alert } from "@/components/ui/alert";
import { SectionHeader } from "@/components/ui/section-header";
import { GlassPanel } from "@/components/ui/glass-panel";
import { InfoPill } from "@/components/ui/info-pill";
import { Logo } from "@/components/ui/logo";
import { Fade } from "@/components/animate-ui/primitives/effects/fade";
import { motion, AnimatePresence } from "motion/react";
import {
  FolderOpen,
  Wrench,
  Database,
  FolderSearch,
  RefreshCw,
  ArrowDownToLine,
  ClipboardCopy,
  ClipboardPaste,
  ChevronRight,
  Monitor,
  Gauge,
  Sparkles,
  Trash2,
  Palette,
  MessageSquareText,
  Bug,
  MessageCircle,
} from "lucide-react";
import { AccountSettings } from "./account-settings";
import { AppearanceSettings } from "./appearance-settings";
import { FEATURES, toolsMenuFeatures, type FeatureId, type ToolsGroup } from "@/lib/features";

type SettingsTab = "general" | "appearance" | "tools" | "data";
type PerformanceMode = "webview" | "native-slint";

interface SettingsProps {
  addonsPath: string;
  authUser: AuthUser | null;
  authVerifying: boolean;
  knownInstances: GameInstance[];
  onAuthChange: (user: AuthUser | null) => void;
  onInstancesDetected: (instances: GameInstance[]) => void;
  onPathChange: (path: string) => void;
  onClose: () => void;
  onRefresh: () => void;
  onOpenLogUpload: () => void;
  onOpenFeature: (id: FeatureId) => void;
  minionDetected: boolean;
  onShowShortcuts: () => void;
  onCheckForAppUpdate: () => void;
  toolbarHidden: FeatureId[];
  /** Takes an updater, not a value — see `handleToolbarHiddenChange` in App.tsx. */
  onToolbarHiddenChange: (update: (prev: FeatureId[]) => FeatureId[]) => void;
}

const tabs: { id: SettingsTab; label: string; icon: React.ElementType }[] = [
  { id: "general", label: "General", icon: FolderOpen },
  { id: "appearance", label: "Appearance", icon: Palette },
  { id: "tools", label: "Tools", icon: Wrench },
  { id: "data", label: "Data", icon: Database },
];

export function Settings({
  addonsPath,
  authUser,
  authVerifying,
  knownInstances,
  onAuthChange,
  onInstancesDetected,
  onPathChange,
  onClose,
  onRefresh,
  onOpenLogUpload,
  onOpenFeature,
  minionDetected,
  onShowShortcuts,
  onCheckForAppUpdate,
  toolbarHidden,
  onToolbarHiddenChange,
}: SettingsProps) {
  const [activeTab, setActiveTab] = useState<SettingsTab>("general");
  const [path, setPath] = useState(addonsPath);
  const [importing, setImporting] = useState(false);
  const [importResult, setImportResult] = useState<ImportResult | null>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const [exportStatus, setExportStatus] = useState<string | null>(null);
  const {
    value: autoUpdate,
    commit: commitAutoUpdate,
    hydrate: hydrateAutoUpdate,
  } = useOptimisticSetting(false, (value) => setSetting("autoUpdate", value));
  const {
    value: warnEsoRunning,
    commit: commitWarnEsoRunning,
    hydrate: hydrateWarnEsoRunning,
  } = useOptimisticSetting(true, (value) => setSetting("suppressEsoRunningWarning", !value));
  const {
    value: performanceMode,
    commit: commitPerformanceMode,
    hydrate: hydratePerformanceMode,
  } = useOptimisticSetting<PerformanceMode>(
    "webview",
    (value) => setSetting("performanceMode", value),
    "Couldn't save performance mode."
  );
  const [switchingPerformanceMode, setSwitchingPerformanceMode] = useState(false);
  const [redetecting, setRedetecting] = useState(false);
  const [redetectedInstances, setRedetectedInstances] = useState<GameInstance[] | null>(null);
  const [copyTarget, setCopyTarget] = useState<GameInstance | null>(null);
  const [copying, setCopying] = useState(false);
  const {
    value: conflictPolicy,
    commit: commitConflictPolicy,
    hydrate: hydrateConflictPolicy,
  } = useOptimisticSetting<"ask" | "keep_mine" | "take_update">("ask", (value) =>
    setSetting("conflictPolicy", value)
  );
  const {
    value: dependencyPolicy,
    commit: commitDependencyPolicy,
    hydrate: hydrateDependencyPolicy,
  } = useConfirmedSetting<DependencyPolicy>(DEFAULT_DEPENDENCY_POLICY, saveDependencyPolicy);
  // Names the user answered "don't ask again" for. Kept as the full list (not just
  // a count) so the row can name them in a tooltip — otherwise "3 libraries" is an
  // opaque thing to be asked to clear.
  const [skippedDependencies, setSkippedDependencies] = useState<string[]>([]);
  const [clearingSkippedDependencies, setClearingSkippedDependencies] = useState(false);
  // Narrows the "ask" prompt to required libraries. Stored separately from the
  // policy rather than as a fourth radio: it answers "about what", where the
  // radios answer "do what", and only one of the three has anything to narrow.
  //
  // Both go through useConfirmedSetting: the install path reads the STORED
  // values, so neither control may show something settings.json does not have.
  // They lag a click by one local write and cannot show an unsaved value.
  const {
    value: askRequiredOnly,
    commit: commitAskRequiredOnly,
    hydrate: hydrateAskRequiredOnly,
  } = useConfirmedSetting<boolean>(DEFAULT_ASK_REQUIRED_ONLY, setAskRequiredDependenciesOnly);
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false);
  const [deletingAccount, setDeletingAccount] = useState(false);
  // Opt-OUT of direct upload (native is the default for manual + live). Mirrors the
  // `manualUseOfficialUploader` key the uploader workspace reads; the toggle writes
  // both manual + live opt-out keys.
  const {
    value: useOfficialUploader,
    commit: commitUseOfficialUploader,
    hydrate: hydrateUseOfficialUploader,
  } = useOptimisticSetting(false, (value) =>
    setSettings({
      manualUseOfficialUploader: value,
      liveUseOfficialUploader: value,
    })
  );
  const {
    value: autoOpenAnalysis,
    commit: commitAutoOpenAnalysis,
    hydrate: hydrateAutoOpenAnalysis,
  } = useOptimisticSetting(false, (value) => setSetting("autoOpenAnalysis", value));

  useEffect(() => {
    void getSetting<boolean>("autoUpdate", false).then(hydrateAutoUpdate);
    void getSetting<boolean>("suppressEsoRunningWarning", false).then((s) =>
      hydrateWarnEsoRunning(!s)
    );
    void getSetting<string>("performanceMode", "webview").then((mode) =>
      hydratePerformanceMode(mode === "native-slint" ? "native-slint" : "webview")
    );
    // The toggle WRITES both opt-out keys, so its checked state must REFLECT both: a
    // pre-existing user who opted out of LIVE direct upload (liveUseOfficialUploader)
    // before this unified control existed must see it as on, or the toggle would claim
    // "direct upload" while live still hands off (a read/write split-brain).
    void Promise.all([
      getSetting<boolean>("manualUseOfficialUploader", false),
      getSetting<boolean>("liveUseOfficialUploader", false),
    ]).then(([manual, live]) => hydrateUseOfficialUploader(manual || live));
    void getSetting<boolean>("autoOpenAnalysis", false).then(hydrateAutoOpenAnalysis);
    void getSetting<"ask" | "keep_mine" | "take_update">("conflictPolicy", "ask").then(
      hydrateConflictPolicy
    );
    // Via the module's reader, which narrows: settings.json is user-editable and
    // survives downgrades, so a bad value must fall back rather than leave every
    // radio unselected while the app quietly behaves as the default.
    void getDependencyPolicy().then(hydrateDependencyPolicy);
    void getSkippedDependencies().then(setSkippedDependencies);
    void getAskRequiredDependenciesOnly().then(hydrateAskRequiredOnly);
    // Hydrators are stable callbacks, so listing them keeps this a mount-only
    // load while documenting every setting seeded by the effect.
  }, [
    hydrateAskRequiredOnly,
    hydrateAutoOpenAnalysis,
    hydrateAutoUpdate,
    hydrateConflictPolicy,
    hydrateDependencyPolicy,
    hydratePerformanceMode,
    hydrateUseOfficialUploader,
    hydrateWarnEsoRunning,
  ]);

  // Silently refresh the detected-instance list every time Settings opens, so
  // a PTS install created after app startup shows up in the switcher without
  // requiring a manual Re-detect.
  useEffect(() => {
    invokeOrThrow<GameInstance[]>("detect_game_instances")
      .then(onInstancesDetected)
      .catch(console.error);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleCopyToInstance = async (target: GameInstance) => {
    setCopying(true);
    try {
      const result = await invokeOrThrow<CopyAddonsResult>("copy_addons_to_instance", {
        addonsPath,
        targetAddonsPath: target.addonsPath,
      });
      const parts: string[] = [];
      if (result.copied.length > 0) parts.push(`${result.copied.length} copied`);
      if (result.skipped.length > 0) parts.push(`${result.skipped.length} already there`);
      if (result.copied.length === 0 && result.skipped.length === 0) {
        parts.push("no enabled addons to copy");
      }
      toast.success(`Addons → ${target.displayLabel}: ${parts.join(", ")}`);
      if (result.failed.length > 0) {
        toast.error(`Failed to copy ${result.failed.length} item(s): ${result.failed.join(", ")}`);
      }
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setCopying(false);
      setCopyTarget(null);
    }
  };

  const handleSave = () => {
    if (path.trim()) {
      onPathChange(path.trim());
    }
    onClose();
  };

  const handleBrowse = async () => {
    try {
      const selected = await open({
        directory: true,
        title: "Select ESO AddOns Folder",
        defaultPath: path || undefined,
      });
      if (selected) {
        setPath(selected);
      }
    } catch (e) {
      toast.error(`Failed to open folder picker: ${getTauriErrorMessage(e)}`);
    }
  };

  const handleRedetect = async () => {
    setRedetecting(true);
    setRedetectedInstances(null);
    try {
      const instances = await invokeOrThrow<GameInstance[]>("detect_game_instances");
      if (instances.length === 0) {
        toast.info("No ESO AddOns folders detected.");
      } else if (instances.length === 1) {
        const detected = instances[0]!.addonsPath;
        if (detected !== addonsPath) {
          setPath(detected);
          toast.success("Found AddOns folder. Click Save to apply.");
        } else {
          toast.info("Current folder is already the best candidate.");
        }
      } else {
        setRedetectedInstances(instances);
      }
    } catch (e) {
      toast.error(`Re-detection failed: ${getTauriErrorMessage(e)}`);
    } finally {
      setRedetecting(false);
    }
  };

  const handleExport = async () => {
    setExportStatus(null);
    try {
      const json = await invokeOrThrow<string>("export_addon_list", {
        addonsPath,
      });
      await navigator.clipboard.writeText(json);
      setExportStatus("Addon list copied to clipboard!");
    } catch (e) {
      setExportStatus(`Export failed: ${getTauriErrorMessage(e)}`);
    }
  };

  const handleImport = async () => {
    setImportError(null);
    setImportResult(null);
    try {
      const text = await navigator.clipboard.readText();
      if (!text.trim()) {
        setImportError("Clipboard is empty. Copy an export JSON first.");
        return;
      }
      try {
        JSON.parse(text);
      } catch {
        setImportError("Clipboard does not contain valid JSON.");
        return;
      }
      setImporting(true);
      const result = await invokeOrThrow<ImportResult>("import_addon_list", {
        addonsPath,
        jsonData: text,
      });
      setImportResult(result);
      onRefresh();
    } catch (e) {
      setImportError(getTauriErrorMessage(e));
    } finally {
      setImporting(false);
    }
  };

  const handleDeleteAccount = async () => {
    setDeletingAccount(true);
    try {
      const result = await invokeOrThrow<{ packs: number; votes: number; shares: number }>(
        "delete_pack_hub_account"
      );
      onAuthChange(null);
      setDeleteConfirmOpen(false);
      toast.success(
        `Deleted ${result.packs} pack${result.packs !== 1 ? "s" : ""}, ${result.votes} vote${result.votes !== 1 ? "s" : ""}, and ${result.shares} share code${result.shares !== 1 ? "s" : ""}.`
      );
    } catch (e) {
      toast.error(`Failed to delete account data: ${getTauriErrorMessage(e)}`);
    } finally {
      setDeletingAccount(false);
    }
  };

  const handleClearSkippedDependencies = async () => {
    setClearingSkippedDependencies(true);
    try {
      // Mirrors setSetting's contract: false means the write failed, so the list
      // is still there — say so rather than falsely reporting success.
      const cleared = await clearSkippedDependencies();
      if (cleared === false) {
        toast.error("Couldn't clear that list — try again.");
        return;
      }
      setSkippedDependencies([]);
      toast.success("Kalpa will ask about those libraries again.");
    } finally {
      setClearingSkippedDependencies(false);
    }
  };

  const handlePerformanceModeChange = async (checked: boolean) => {
    if (switchingPerformanceMode) return;

    if (checked) {
      // Switching to the native shell exits this process. A live-logging
      // session streaming right now would be killed mid-report with no
      // recovery path from the native side — refuse instead of orphaning it.
      const liveActive = await invokeResult<boolean>("uploader_live_active");
      if (liveActive.ok && liveActive.data) {
        toast.error("A live log upload is running.", {
          description: "Stop live logging (or let the session finish) before switching UI modes.",
        });
        return;
      }
      // The consequence is drastic (this window closes immediately); make the
      // user say it twice.
      if (
        !window.confirm(
          "Switch to the native performance UI?\n\n" +
            "Kalpa will close this window and relaunch as the native app. " +
            "You can switch back anytime from the native app's Settings."
        )
      ) {
        return;
      }
    }

    const next: PerformanceMode = checked ? "native-slint" : "webview";
    const previous = performanceMode;
    setSwitchingPerformanceMode(true);

    const saved = await commitPerformanceMode(next);
    if (!saved) {
      setSwitchingPerformanceMode(false);
      return;
    }

    if (!checked) {
      setSwitchingPerformanceMode(false);
      return;
    }

    try {
      await invokeOrThrow<{ exePath: string }>("launch_native_performance_mode");
      toast.success("Switching to native performance mode...");
    } catch (e) {
      void commitPerformanceMode(previous);
      toast.error(`Native performance mode is not available: ${getTauriErrorMessage(e)}`);
      setSwitchingPerformanceMode(false);
    }
  };

  const pathDirty = path.trim() !== addonsPath;

  const toolsCtx = { minionDetected };
  const toolFeatures = (group: ToolsGroup) =>
    FEATURES.filter(
      (f) =>
        f.placement === "tools" && f.toolsGroup === group && (f.visibleWhen?.(toolsCtx) ?? true)
    );
  // Pinnable features the user has unpinned from the header toolbar. Without
  // this block they would appear in NEITHER surface — `toolFeatures` only ever
  // matches `placement: "tools"` — leaving an unpinned Pack Hub reachable only
  // by deep link. This tab is the catalog, so it lists them first.
  const unpinnedFeatures = toolsMenuFeatures(FEATURES, toolbarHidden, toolsCtx).filter(
    (f) => f.pinnableToToolbar
  );

  return (
    <>
      <Dialog open onOpenChange={(open) => !open && onClose()}>
        <DialogContent className="sm:max-w-xl h-[70vh] flex flex-col">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Logo size={18} className="text-[#4dc2e6]" />
              Settings
            </DialogTitle>
          </DialogHeader>

          {/* Tab bar */}
          <div className="relative flex gap-1 rounded-lg bg-structure-03 border border-structure-04 p-1">
            {tabs.map((tab) => {
              const Icon = tab.icon;
              const active = activeTab === tab.id;
              return (
                <button
                  key={tab.id}
                  type="button"
                  onClick={() => setActiveTab(tab.id)}
                  className={`relative z-10 flex flex-1 items-center justify-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium transition-colors duration-150 ${
                    active
                      ? "text-foreground"
                      : "text-muted-foreground hover:text-foreground hover:bg-structure-03"
                  }`}
                >
                  {active && (
                    <motion.span
                      layoutId="settings-tab-indicator"
                      className="absolute inset-0 rounded-md bg-structure-08 shadow-[0_1px_3px_var(--scrim-20),inset_0_1px_0_var(--structure-04)]"
                      transition={{ type: "spring", stiffness: 400, damping: 30 }}
                    />
                  )}
                  <span className="relative z-10 flex items-center gap-1.5">
                    <Icon className="size-3.5" />
                    {tab.label}
                  </span>
                </button>
              );
            })}
          </div>

          {/* Tab content */}
          <div className="flex-1 min-h-0 overflow-y-auto">
            <AnimatePresence mode="wait">
              {activeTab === "general" && (
                <motion.div
                  key={activeTab}
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  transition={{ duration: 0.08 }}
                  className="space-y-3"
                >
                  <AccountSettings
                    authUser={authUser}
                    authVerifying={authVerifying}
                    onAuthChange={onAuthChange}
                    onOpenLogUpload={onOpenLogUpload}
                  />

                  {/* Path configuration */}
                  <GlassPanel variant="subtle" className="p-3 space-y-3">
                    <SectionHeader>AddOns Folder</SectionHeader>
                    <Input
                      id="addons-path"
                      value={path}
                      onChange={(e) => setPath(e.target.value)}
                      placeholder={exampleAddonsPath()}
                      autoFocus
                      onKeyDown={(e) => {
                        if (e.key === "Enter") handleSave();
                      }}
                    />
                    <div className="flex gap-2">
                      <Button variant="outline" size="sm" onClick={handleBrowse}>
                        <FolderSearch className="size-3.5" />
                        Browse
                      </Button>
                      <Button
                        variant="outline"
                        size="sm"
                        disabled={redetecting}
                        onClick={handleRedetect}
                      >
                        <RefreshCw className={`size-3.5 ${redetecting ? "animate-spin" : ""}`} />
                        {redetecting ? "Detecting..." : "Re-detect"}
                      </Button>
                      {pathDirty && (
                        <Button size="sm" onClick={handleSave} className="ml-auto">
                          <Sparkles className="size-3.5" />
                          Apply
                        </Button>
                      )}
                    </div>

                    {/* Instance picker — shown after re-detect finds multiple folders */}
                    {redetectedInstances && redetectedInstances.length > 1 && (
                      <Fade>
                        <div className="space-y-1.5">
                          <p className="text-xs text-muted-foreground">Select an instance:</p>
                          {redetectedInstances.map((inst) => (
                            <button
                              key={inst.id}
                              type="button"
                              className="flex w-full items-center gap-2 rounded-lg border border-structure-06 bg-structure-02 px-3 py-2 text-left text-xs text-foreground transition-all duration-150 hover:border-structure-12 hover:bg-structure-04"
                              onClick={() => {
                                setPath(inst.addonsPath);
                                setRedetectedInstances(null);
                              }}
                            >
                              <Monitor className="size-3.5 text-muted-foreground shrink-0" />
                              <span className="font-medium">{inst.displayLabel}</span>
                              <span className="text-muted-foreground">
                                {inst.addonCount} addon{inst.addonCount !== 1 ? "s" : ""}
                              </span>
                            </button>
                          ))}
                        </div>
                      </Fade>
                    )}

                    {/* Quick-switch between already-known instances.
                        Clicking a row applies the switch immediately — no
                        separate Save step — and rescans that instance. */}
                    {knownInstances.length > 1 && !redetectedInstances && (
                      <div className="space-y-1.5">
                        <p className="text-xs text-muted-foreground">
                          Switch instance (applies immediately):
                        </p>
                        {knownInstances.map((inst) => {
                          const isActive = inst.addonsPath === addonsPath;
                          return (
                            <div key={inst.id} className="flex items-center gap-1.5">
                              <button
                                type="button"
                                className={`flex flex-1 items-center gap-2 rounded-lg border px-3 py-2 text-left text-xs transition-all duration-150 ${
                                  isActive
                                    ? "border-status-info/30 bg-status-info/[0.06] text-status-info-soft"
                                    : "border-structure-06 bg-structure-02 text-foreground hover:border-structure-12 hover:bg-structure-04"
                                }`}
                                onClick={() => {
                                  if (!isActive) {
                                    setPath(inst.addonsPath);
                                    onPathChange(inst.addonsPath);
                                    toast.success(`Switched to ${inst.displayLabel}`);
                                  }
                                }}
                              >
                                <Monitor className="size-3.5 shrink-0 text-muted-foreground" />
                                <span className="font-medium">{inst.displayLabel}</span>
                                <span className="text-muted-foreground">
                                  {inst.addonCount} addon{inst.addonCount !== 1 ? "s" : ""}
                                </span>
                                {isActive && (
                                  <span className="ml-auto text-[11px] font-semibold uppercase tracking-wider text-status-info">
                                    active
                                  </span>
                                )}
                              </button>
                              {!isActive && (
                                <Button
                                  variant="outline"
                                  size="sm"
                                  className="shrink-0 px-2"
                                  disabled={copying}
                                  title={`Copy this instance's missing addons from the active instance`}
                                  onClick={() => setCopyTarget(inst)}
                                >
                                  <ClipboardCopy className="size-3.5" />
                                </Button>
                              )}
                            </div>
                          );
                        })}

                        {/* Cross-instance copy confirm */}
                        {copyTarget && (
                          <Fade>
                            <div className="space-y-2 rounded-lg border border-primary/25 bg-primary/[0.04] px-3 py-2">
                              <p className="text-xs text-foreground">
                                Copy all enabled addons from the active instance into{" "}
                                <span className="font-medium text-primary">
                                  {copyTarget.displayLabel}
                                </span>
                                ? Addons that instance already has (enabled or disabled) are left
                                untouched.
                              </p>
                              <div className="flex justify-end gap-2">
                                <Button
                                  variant="outline"
                                  size="sm"
                                  disabled={copying}
                                  onClick={() => setCopyTarget(null)}
                                >
                                  Cancel
                                </Button>
                                <Button
                                  size="sm"
                                  disabled={copying}
                                  onClick={() => void handleCopyToInstance(copyTarget)}
                                >
                                  {copying ? "Copying..." : "Copy addons"}
                                </Button>
                              </div>
                            </div>
                          </Fade>
                        )}
                      </div>
                    )}
                  </GlassPanel>

                  {/* Auto-update */}
                  <GlassPanel variant="subtle" className="p-3">
                    <label className="flex items-center gap-3 cursor-pointer">
                      <Checkbox
                        checked={autoUpdate}
                        onCheckedChange={(checked) => {
                          const value = checked === true;
                          void commitAutoUpdate(value);
                        }}
                      />
                      <div>
                        <p className="text-sm font-medium text-foreground">Auto-update on launch</p>
                        <p className="text-xs text-muted-foreground">
                          Automatically update all addons when Kalpa starts
                        </p>
                      </div>
                    </label>
                  </GlassPanel>

                  <GlassPanel variant="subtle" className="p-3">
                    <label
                      className={`flex items-center gap-3 ${
                        switchingPerformanceMode ? "cursor-wait opacity-70" : "cursor-pointer"
                      }`}
                    >
                      <Checkbox
                        checked={performanceMode === "native-slint"}
                        disabled={switchingPerformanceMode}
                        onCheckedChange={(checked) => {
                          void handlePerformanceModeChange(checked === true);
                        }}
                      />
                      <Gauge className="size-4 shrink-0 text-brand-gold-readable" />
                      <div>
                        <div className="flex items-center gap-2">
                          <p className="text-sm font-medium text-foreground">
                            Native performance UI
                          </p>
                          <InfoPill color="amber" className="text-[10px]">
                            Beta
                          </InfoPill>
                        </div>
                        <p className="text-xs text-muted-foreground">
                          Relaunches Kalpa as a lightweight native app that uses less memory — this
                          window closes immediately, and future launches start the native UI. Switch
                          back anytime from the native app&apos;s Settings. Still experimental;
                          Windows only for now.
                        </p>
                      </div>
                    </label>
                  </GlassPanel>

                  {/* Warn when ESO is running */}
                  <GlassPanel variant="subtle" className="p-3">
                    <label className="flex items-center gap-3 cursor-pointer">
                      <Checkbox
                        checked={warnEsoRunning}
                        onCheckedChange={(checked) => {
                          const value = checked === true;
                          void commitWarnEsoRunning(value);
                        }}
                      />
                      <div>
                        <p className="text-sm font-medium text-foreground">
                          Warn when ESO is running
                        </p>
                        <p className="text-xs text-muted-foreground">
                          Remind me to /reloadui after changing addons while the game is open
                        </p>
                      </div>
                    </label>
                  </GlassPanel>

                  {/* Direct (native) upload is now the DEFAULT for both manual and
                    live (faster, report in-app). This is the opt-OUT: turning it on
                    forces the official ESO Logs uploader for both. One control writes
                    both the manual and live opt-out keys so they stay in sync. */}
                  <GlassPanel variant="subtle" className="p-3">
                    <label className="flex items-center gap-3 cursor-pointer">
                      <Checkbox
                        checked={useOfficialUploader}
                        onCheckedChange={(checked) => {
                          const value = checked === true;
                          // Mirror live's opt-out model for manual too. Write both keys
                          // ATOMICALLY (one flush, all-or-nothing) so a failed/crashed
                          // write can't leave one mode opted out and the other native —
                          // the exact split-brain this unified toggle exists to prevent.
                          // On failure, revert the optimistic UI and surface it.
                          void commitUseOfficialUploader(value);
                        }}
                      />
                      <div>
                        <p className="text-sm font-medium text-foreground">
                          Use the official ESO Logs uploader
                        </p>
                        <p className="text-xs text-muted-foreground">
                          Off by default — Kalpa uploads directly (faster, and the report appears
                          in-app). Direct upload is an unofficial method that falls back to the
                          official uploader automatically when a log can't be encoded with full
                          accuracy. Turn this on to always use the official uploader instead.
                        </p>
                      </div>
                    </label>
                  </GlassPanel>

                  {/* Auto-open the ESO Log Aggregator analysis after an upload. Off by
                    default so an upload never steals focus to the browser unasked. */}
                  <GlassPanel variant="subtle" className="p-3">
                    <label className="flex items-center gap-3 cursor-pointer">
                      <Checkbox
                        checked={autoOpenAnalysis}
                        onCheckedChange={(checked) => {
                          const value = checked === true;
                          void commitAutoOpenAnalysis(value);
                        }}
                      />
                      <div>
                        <p className="text-sm font-medium text-foreground">
                          Open analysis after upload
                        </p>
                        <p className="text-xs text-muted-foreground">
                          When an upload finishes, automatically open its report in the ESO Log
                          Aggregator (fight detection, rotations, scribing, replay). You can always
                          open it from a report's “View analysis” button instead.
                        </p>
                      </div>
                    </label>
                  </GlassPanel>

                  {/* Conflict policy */}
                  <GlassPanel variant="subtle" className="p-3 space-y-2">
                    <SectionHeader>When your edited files conflict with an update</SectionHeader>
                    {(
                      [
                        ["ask", "Ask me each time"],
                        ["keep_mine", "Always keep my version"],
                        ["take_update", "Always take the update (back up my files)"],
                      ] as const
                    ).map(([value, label]) => (
                      <button
                        key={value}
                        type="button"
                        className="flex items-center gap-3 cursor-pointer w-full text-left"
                        onClick={() => {
                          void commitConflictPolicy(value);
                        }}
                      >
                        <span
                          className={`flex h-4 w-4 shrink-0 items-center justify-center rounded-full border transition-colors ${
                            conflictPolicy === value
                              ? "border-[#c4a44a] bg-[#c4a44a]/20"
                              : "border-structure-20 bg-structure-03"
                          }`}
                        >
                          {conflictPolicy === value && (
                            <span className="h-2 w-2 rounded-full bg-[#c4a44a]" />
                          )}
                        </span>
                        <span className="text-sm text-foreground">{label}</span>
                      </button>
                    ))}
                  </GlassPanel>

                  {/* Dependency policy — deliberately shaped as the conflict policy's
                    sibling (same radio rows, same read/save path) because both answer
                    the same kind of question: what should an install do on your behalf. */}
                  <GlassPanel variant="subtle" className="p-3 space-y-2">
                    <SectionHeader>When an addon needs other libraries</SectionHeader>
                    {(
                      [
                        // "required ones" is not a softening of the label — it
                        // is what this mode does. Auto-resolution filters to
                        // required entries (`resolve_transitive_deps` in
                        // commands.rs); optional libraries are never installed
                        // without an explicit tick under any policy. The old
                        // "Install them automatically" read as "all of them",
                        // which drove users away from the mode they wanted.
                        ["auto", "Install required ones automatically"],
                        ["ask", "Ask me which ones to install (default)"],
                        ["skip", "Never install them"],
                      ] as const
                    ).map(([value, label]) => (
                      <button
                        key={value}
                        type="button"
                        className="flex items-center gap-3 cursor-pointer w-full text-left"
                        // Deliberately NOT optimistic: the selection moves only
                        // once the write confirms. The install path reads the
                        // stored policy, so a radio showing "ask" over a stored
                        // "skip" would silently never offer a missing required
                        // library — the outcome this whole feature exists to
                        // prevent. Confirming first makes that state
                        // unreachable rather than recoverable.
                        onClick={() => commitDependencyPolicy(value)}
                      >
                        <span
                          className={`flex h-4 w-4 shrink-0 items-center justify-center rounded-full border transition-colors ${
                            dependencyPolicy === value
                              ? "border-[#c4a44a] bg-[#c4a44a]/20"
                              : "border-structure-20 bg-structure-03"
                          }`}
                        >
                          {dependencyPolicy === value && (
                            <span className="h-2 w-2 rounded-full bg-[#c4a44a]" />
                          )}
                        </span>
                        <span className="text-sm text-foreground">{label}</span>
                      </button>
                    ))}

                    {dependencyPolicy === "skip" && (
                      <p className="text-xs text-muted-foreground">
                        Addons that depend on a missing library won&apos;t load until you install it
                        yourself.
                      </p>
                    )}

                    {/* Only under "ask": the other two policies never surface an
                      optional library, so there would be nothing to narrow. The
                      stored value is left alone when the row is hidden, so
                      passing through "auto" and back doesn't silently reset it. */}
                    {dependencyPolicy === "ask" && (
                      <label className="flex cursor-pointer items-start gap-3 border-t border-structure-06 pt-2">
                        <Checkbox
                          className="mt-0.5"
                          checked={askRequiredOnly}
                          onCheckedChange={(checked) => commitAskRequiredOnly(checked === true)}
                        />
                        <div>
                          <p className="text-sm text-foreground">Only ask about required ones</p>
                          <p className="text-xs text-muted-foreground">
                            Optional libraries stay listed under an addon&apos;s Details tab, each
                            with an Install button.
                          </p>
                        </div>
                      </label>
                    )}

                    {/* Escape hatch for “don’t ask again”: that choice is otherwise
                      permanent and invisible, so the prompt could never come back.
                      Hidden while the list is empty so it stays quiet. */}
                    {skippedDependencies.length > 0 && (
                      <div className="flex items-center justify-between gap-2 border-t border-structure-06 pt-2">
                        <p
                          className="text-xs text-muted-foreground"
                          title={skippedDependencies.join(", ")}
                        >
                          {skippedDependencies.length}{" "}
                          {skippedDependencies.length === 1 ? "library" : "libraries"} set to “don’t
                          ask again”
                        </p>
                        <Button
                          variant="ghost"
                          size="xs"
                          disabled={clearingSkippedDependencies}
                          onClick={() => void handleClearSkippedDependencies()}
                        >
                          {clearingSkippedDependencies ? "Clearing..." : "Clear"}
                        </Button>
                      </div>
                    )}
                  </GlassPanel>
                </motion.div>
              )}

              {activeTab === "appearance" && (
                <motion.div
                  key={activeTab}
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  transition={{ duration: 0.08 }}
                >
                  <AppearanceSettings
                    onShowShortcuts={onShowShortcuts}
                    toolbarHidden={toolbarHidden}
                    onToolbarHiddenChange={onToolbarHiddenChange}
                  />
                </motion.div>
              )}

              {activeTab === "tools" && (
                <motion.div
                  key={activeTab}
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  transition={{ duration: 0.08 }}
                  className="space-y-2"
                >
                  {unpinnedFeatures.map((f) => (
                    <ToolItem
                      key={f.id}
                      icon={f.icon}
                      label={f.label}
                      description={f.description}
                      accent={f.accent}
                      onClick={() => onOpenFeature(f.id)}
                    />
                  ))}
                  {unpinnedFeatures.length > 0 && <div className="border-t border-structure-06" />}
                  {toolFeatures("primary").map((f) => (
                    <ToolItem
                      key={f.id}
                      icon={f.icon}
                      label={f.label}
                      description={f.description}
                      accent={f.accent}
                      onClick={() => onOpenFeature(f.id)}
                    />
                  ))}
                  <ToolItem
                    icon={ArrowDownToLine}
                    label="Check for App Updates"
                    description="See if a newer version of Kalpa is available"
                    onClick={onCheckForAppUpdate}
                  />
                  <FeedbackToolGroup />
                  {toolFeatures("secondary").map((f) => (
                    <ToolItem
                      key={f.id}
                      icon={f.icon}
                      label={f.label}
                      description={f.description}
                      accent={f.accent}
                      onClick={() => onOpenFeature(f.id)}
                    />
                  ))}
                </motion.div>
              )}

              {activeTab === "data" && (
                <motion.div
                  key={activeTab}
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  transition={{ duration: 0.08 }}
                  className="space-y-3"
                >
                  <GlassPanel variant="subtle" className="p-3 space-y-3">
                    <SectionHeader>Addon List Backup</SectionHeader>
                    <p className="text-xs text-muted-foreground">
                      Export your tracked addon list to clipboard, or import from a previously
                      exported list to restore on a new machine.
                    </p>
                    <div className="flex gap-2">
                      <Button variant="outline" size="sm" onClick={handleExport}>
                        <ClipboardCopy className="size-3.5" />
                        Export to Clipboard
                      </Button>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={handleImport}
                        disabled={importing}
                      >
                        <ClipboardPaste className="size-3.5" />
                        {importing ? "Importing..." : "Import from Clipboard"}
                      </Button>
                    </div>
                    {exportStatus && <p className="text-xs text-status-success">{exportStatus}</p>}
                    {importError && (
                      <Alert variant="destructive" className="mt-1">
                        {importError}
                      </Alert>
                    )}
                    {importResult && (
                      <div className="space-y-2">
                        {importResult.installed.length > 0 && (
                          <div className="rounded-lg border border-status-success/20 bg-status-success/[0.04] p-2 text-xs text-status-success">
                            Installed: {importResult.installed.join(", ")}
                          </div>
                        )}
                        {importResult.skipped.length > 0 && (
                          <p className="text-xs text-muted-foreground">
                            Already installed: {importResult.skipped.join(", ")}
                          </p>
                        )}
                        {importResult.failed.length > 0 && (
                          <Alert variant="destructive">
                            Failed:{" "}
                            {importResult.failed
                              .map((f) =>
                                importResult.errors?.[f] ? `${f} (${importResult.errors[f]})` : f
                              )
                              .join(", ")}
                          </Alert>
                        )}
                      </div>
                    )}
                  </GlassPanel>

                  {authUser && (
                    <GlassPanel variant="subtle" className="p-3 space-y-3">
                      <SectionHeader>Pack Hub Data</SectionHeader>
                      <p className="text-xs text-muted-foreground">
                        Permanently delete all your data from the Pack Hub, including packs, votes,
                        and share codes. This cannot be undone.
                      </p>
                      {!deleteConfirmOpen ? (
                        <Button
                          variant="outline"
                          size="sm"
                          className="border-status-danger-strong/30 text-status-danger hover:bg-status-danger-strong/10 hover:border-status-danger-strong/50"
                          onClick={() => setDeleteConfirmOpen(true)}
                        >
                          <Trash2 className="size-3.5" />
                          Delete My Pack Hub Data
                        </Button>
                      ) : (
                        <div className="space-y-2 rounded-lg border border-status-danger-strong/20 bg-status-danger-strong/[0.04] p-3">
                          <p className="text-xs font-medium text-status-danger">
                            Are you sure? This will permanently delete all your packs, votes, and
                            share codes. You will also be signed out.
                          </p>
                          <div className="flex gap-2">
                            <Button
                              variant="destructive"
                              size="sm"
                              disabled={deletingAccount}
                              onClick={handleDeleteAccount}
                            >
                              {deletingAccount ? "Deleting..." : "Yes, delete everything"}
                            </Button>
                            <Button
                              variant="outline"
                              size="sm"
                              disabled={deletingAccount}
                              onClick={() => setDeleteConfirmOpen(false)}
                            >
                              Cancel
                            </Button>
                          </div>
                        </div>
                      )}
                    </GlassPanel>
                  )}
                </motion.div>
              )}
            </AnimatePresence>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}

function FeedbackToolGroup() {
  return (
    <GlassPanel variant="subtle" className="space-y-2 p-3">
      <div className="flex items-start gap-3">
        <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-structure-04 text-muted-foreground">
          <MessageSquareText className="size-4" />
        </div>
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-foreground">Feedback and Support</p>
          <p className="text-xs text-muted-foreground">
            Choose GitHub templates for tracked issues or Discord for lower-friction help.
          </p>
        </div>
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="justify-start"
          onClick={() => void openFeedbackUrl(FEEDBACK_ISSUES_URL)}
        >
          <Bug className="size-3.5" />
          GitHub Issues
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="justify-start"
          onClick={() => void openFeedbackUrl(FEEDBACK_DISCORD_URL)}
        >
          <MessageCircle className="size-3.5" />
          Discord
        </Button>
      </div>
    </GlassPanel>
  );
}

function ToolItem({
  icon: Icon,
  label,
  description,
  onClick,
  accent,
}: {
  icon: React.ElementType;
  label: string;
  description: string;
  onClick: () => void;
  accent?: "gold";
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`group flex w-full items-center gap-3 rounded-xl border p-3 text-left transition-all duration-150 hover:-translate-y-px ${
        accent === "gold"
          ? "border-brand-gold-readable/20 bg-brand-gold-readable/[0.04] hover:border-brand-gold-readable/30 hover:bg-brand-gold-readable/[0.06]"
          : "border-structure-04 bg-structure-02 hover:border-structure-08 hover:bg-structure-04"
      }`}
    >
      <div
        className={`flex size-8 shrink-0 items-center justify-center rounded-lg ${
          accent === "gold"
            ? "bg-brand-gold-readable/10 text-brand-gold-readable"
            : "bg-structure-04 text-muted-foreground group-hover:text-foreground"
        } transition-colors duration-150`}
      >
        <Icon className="size-4" />
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium text-foreground">{label}</p>
        <p className="text-xs text-muted-foreground">{description}</p>
      </div>
      <ChevronRight className="size-4 text-muted-foreground/40 group-hover:text-muted-foreground transition-colors duration-150" />
    </button>
  );
}
