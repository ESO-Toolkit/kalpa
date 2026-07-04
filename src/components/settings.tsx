import { useState, useEffect } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { getSetting, setSetting, setSettings } from "@/lib/store";
import { getTauriErrorMessage, invokeOrThrow, invokeResult } from "@/lib/tauri";
import type { AuthUser, GameInstance, ImportResult } from "../types";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Alert } from "@/components/ui/alert";
import { SectionHeader } from "@/components/ui/section-header";
import { GlassPanel } from "@/components/ui/glass-panel";
import { Logo } from "@/components/ui/logo";
import { Fade } from "@/components/animate-ui/primitives/effects/fade";
import { motion, AnimatePresence } from "motion/react";
import {
  FolderOpen,
  Wrench,
  Database,
  FolderSearch,
  RefreshCw,
  Archive,
  Users,
  ShieldCheck,
  ArrowDownToLine,
  ClipboardCopy,
  ClipboardPaste,
  ChevronRight,
  Monitor,
  Gauge,
  Shield,
  Sparkles,
  Trash2,
  Palette,
} from "lucide-react";
import { AppearanceSettings } from "./appearance-settings";

type SettingsTab = "general" | "appearance" | "tools" | "data";
type PerformanceMode = "webview" | "native-slint";

interface SettingsProps {
  addonsPath: string;
  authUser: AuthUser | null;
  knownInstances: GameInstance[];
  onAuthChange: (user: AuthUser | null) => void;
  onPathChange: (path: string) => void;
  onClose: () => void;
  onRefresh: () => void;
  onShowBackups: () => void;
  onShowApiCompat: () => void;
  onShowCharacters: () => void;
  onShowMigrationWizard: () => void;
  onShowSafetyCenter: () => void;
  onCheckForAppUpdate: () => void;
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
  knownInstances,
  onAuthChange,
  onPathChange,
  onClose,
  onRefresh,
  onShowBackups,
  onShowApiCompat,
  onShowCharacters,
  onShowMigrationWizard,
  onShowSafetyCenter,
  onCheckForAppUpdate,
}: SettingsProps) {
  const [activeTab, setActiveTab] = useState<SettingsTab>("general");
  const [path, setPath] = useState(addonsPath);
  const [importing, setImporting] = useState(false);
  const [importResult, setImportResult] = useState<ImportResult | null>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const [exportStatus, setExportStatus] = useState<string | null>(null);
  const [autoUpdate, setAutoUpdate] = useState(false);
  const [warnEsoRunning, setWarnEsoRunning] = useState(true);
  const [performanceMode, setPerformanceMode] = useState<PerformanceMode>("webview");
  const [switchingPerformanceMode, setSwitchingPerformanceMode] = useState(false);
  const [minionDetected, setMinionDetected] = useState(false);
  const [redetecting, setRedetecting] = useState(false);
  const [redetectedInstances, setRedetectedInstances] = useState<GameInstance[] | null>(null);
  const [conflictPolicy, setConflictPolicy] = useState<"ask" | "keep_mine" | "take_update">("ask");
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false);
  const [deletingAccount, setDeletingAccount] = useState(false);
  // Opt-OUT of direct upload (native is the default for manual + live). Mirrors the
  // `manualUseOfficialUploader` key the uploader workspace reads; the toggle writes
  // both manual + live opt-out keys.
  const [useOfficialUploader, setUseOfficialUploader] = useState(false);
  const [autoOpenAnalysis, setAutoOpenAnalysis] = useState(false);

  useEffect(() => {
    void getSetting<boolean>("autoUpdate", false).then(setAutoUpdate);
    void getSetting<boolean>("suppressEsoRunningWarning", false).then((s) => setWarnEsoRunning(!s));
    void getSetting<string>("performanceMode", "webview").then((mode) =>
      setPerformanceMode(mode === "native-slint" ? "native-slint" : "webview")
    );
    // The toggle WRITES both opt-out keys, so its checked state must REFLECT both: a
    // pre-existing user who opted out of LIVE direct upload (liveUseOfficialUploader)
    // before this unified control existed must see it as on, or the toggle would claim
    // "direct upload" while live still hands off (a read/write split-brain).
    void Promise.all([
      getSetting<boolean>("manualUseOfficialUploader", false),
      getSetting<boolean>("liveUseOfficialUploader", false),
    ]).then(([manual, live]) => setUseOfficialUploader(manual || live));
    void getSetting<boolean>("autoOpenAnalysis", false).then(setAutoOpenAnalysis);
    void getSetting<"ask" | "keep_mine" | "take_update">("conflictPolicy", "ask").then(
      setConflictPolicy
    );
    void invokeResult<boolean>("detect_minion").then((result) => {
      if (result.ok) {
        setMinionDetected(result.data);
      }
    });
  }, []);

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

  const handlePerformanceModeChange = async (checked: boolean) => {
    if (switchingPerformanceMode) return;

    const next: PerformanceMode = checked ? "native-slint" : "webview";
    const previous = performanceMode;
    setPerformanceMode(next);
    setSwitchingPerformanceMode(true);

    const saved = await setSetting("performanceMode", next);
    if (!saved) {
      setPerformanceMode(previous);
      setSwitchingPerformanceMode(false);
      toast.error("Couldn't save performance mode.");
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
      setPerformanceMode(previous);
      void setSetting("performanceMode", previous);
      toast.error(`Native performance mode is not available: ${getTauriErrorMessage(e)}`);
      setSwitchingPerformanceMode(false);
    }
  };

  const pathDirty = path.trim() !== addonsPath;

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
          <div className="relative flex gap-1 rounded-lg bg-white/[0.03] border border-white/[0.04] p-1">
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
                      ? "text-white"
                      : "text-muted-foreground hover:text-white/70 hover:bg-white/[0.03]"
                  }`}
                >
                  {active && (
                    <motion.span
                      layoutId="settings-tab-indicator"
                      className="absolute inset-0 rounded-md bg-white/[0.08] shadow-[0_1px_3px_rgba(0,0,0,0.2),inset_0_1px_0_rgba(255,255,255,0.04)]"
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
                  {/* Path configuration */}
                  <GlassPanel variant="subtle" className="p-3 space-y-3">
                    <SectionHeader>AddOns Folder</SectionHeader>
                    <Input
                      id="addons-path"
                      value={path}
                      onChange={(e) => setPath(e.target.value)}
                      placeholder="C:\Users\...\Elder Scrolls Online\live\AddOns"
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
                              className="flex w-full items-center gap-2 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2 text-left text-xs text-white/80 transition-all duration-150 hover:border-white/[0.12] hover:bg-white/[0.04]"
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

                    {/* Quick-switch between already-known instances */}
                    {knownInstances.length > 1 && !redetectedInstances && (
                      <div className="space-y-1.5">
                        <p className="text-xs text-muted-foreground">Switch instance:</p>
                        {knownInstances.map((inst) => {
                          const isActive = inst.addonsPath === addonsPath;
                          return (
                            <button
                              key={inst.id}
                              type="button"
                              className={`flex w-full items-center gap-2 rounded-lg border px-3 py-2 text-left text-xs transition-all duration-150 ${
                                isActive
                                  ? "border-sky-400/30 bg-sky-400/[0.06] text-sky-300"
                                  : "border-white/[0.06] bg-white/[0.02] text-white/80 hover:border-white/[0.12] hover:bg-white/[0.04]"
                              }`}
                              onClick={() => {
                                if (!isActive) {
                                  setPath(inst.addonsPath);
                                }
                              }}
                            >
                              <Monitor className="size-3.5 shrink-0 text-muted-foreground" />
                              <span className="font-medium">{inst.displayLabel}</span>
                              <span className="text-muted-foreground">
                                {inst.addonCount} addon{inst.addonCount !== 1 ? "s" : ""}
                              </span>
                              {isActive && (
                                <span className="ml-auto text-[10px] font-semibold uppercase tracking-wider text-sky-400">
                                  active
                                </span>
                              )}
                            </button>
                          );
                        })}
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
                          setAutoUpdate(value);
                          setSetting("autoUpdate", value);
                        }}
                      />
                      <div>
                        <p className="text-sm font-medium text-white/90">Auto-update on launch</p>
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
                      <Gauge className="size-4 shrink-0 text-[#c4a44a]" />
                      <div>
                        <p className="text-sm font-medium text-white/90">Native performance UI</p>
                        <p className="text-xs text-muted-foreground">
                          Switch Kalpa to the smooth Slint shell and close the WebView process.
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
                          setWarnEsoRunning(value);
                          setSetting("suppressEsoRunningWarning", !value);
                        }}
                      />
                      <div>
                        <p className="text-sm font-medium text-white/90">
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
                          setUseOfficialUploader(value);
                          // Mirror live's opt-out model for manual too. Write both keys
                          // ATOMICALLY (one flush, all-or-nothing) so a failed/crashed
                          // write can't leave one mode opted out and the other native —
                          // the exact split-brain this unified toggle exists to prevent.
                          // On failure, revert the optimistic UI and surface it.
                          void setSettings({
                            manualUseOfficialUploader: value,
                            liveUseOfficialUploader: value,
                          }).then((ok) => {
                            if (!ok) {
                              setUseOfficialUploader(!value);
                              toast.error("Couldn't save that setting — try again.");
                            }
                          });
                        }}
                      />
                      <div>
                        <p className="text-sm font-medium text-white/90">
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
                          setAutoOpenAnalysis(value);
                          void setSetting("autoOpenAnalysis", value);
                        }}
                      />
                      <div>
                        <p className="text-sm font-medium text-white/90">
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
                          setConflictPolicy(value);
                          void setSetting("conflictPolicy", value);
                        }}
                      >
                        <span
                          className={`flex h-4 w-4 shrink-0 items-center justify-center rounded-full border transition-colors ${
                            conflictPolicy === value
                              ? "border-[#c4a44a] bg-[#c4a44a]/20"
                              : "border-white/20 bg-white/[0.03]"
                          }`}
                        >
                          {conflictPolicy === value && (
                            <span className="h-2 w-2 rounded-full bg-[#c4a44a]" />
                          )}
                        </span>
                        <span className="text-sm text-white/80">{label}</span>
                      </button>
                    ))}
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
                  <AppearanceSettings />
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
                  <ToolItem
                    icon={Archive}
                    label="Backup & Restore"
                    description="Save and recover your addon settings"
                    onClick={onShowBackups}
                  />
                  <ToolItem
                    icon={Users}
                    label="Characters"
                    description="View and manage your ESO characters"
                    onClick={onShowCharacters}
                  />
                  <ToolItem
                    icon={ShieldCheck}
                    label="API Compatibility"
                    description="Check addons against current API version"
                    onClick={onShowApiCompat}
                  />
                  <ToolItem
                    icon={ArrowDownToLine}
                    label="Check for App Updates"
                    description="See if a newer version of Kalpa is available"
                    onClick={onCheckForAppUpdate}
                  />
                  {minionDetected && (
                    <ToolItem
                      icon={Sparkles}
                      label="Minion Migration"
                      description="Import tracking data from Minion with backup and preview"
                      onClick={onShowMigrationWizard}
                      accent="gold"
                    />
                  )}
                  <ToolItem
                    icon={Shield}
                    label="Safety Center"
                    description="Snapshots, integrity checks, and operation log"
                    onClick={onShowSafetyCenter}
                  />
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
                    {exportStatus && <p className="text-xs text-emerald-400">{exportStatus}</p>}
                    {importError && (
                      <Alert variant="destructive" className="mt-1">
                        {importError}
                      </Alert>
                    )}
                    {importResult && (
                      <div className="space-y-2">
                        {importResult.installed.length > 0 && (
                          <div className="rounded-lg border border-emerald-400/20 bg-emerald-400/[0.04] p-2 text-xs text-emerald-400">
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
                          className="border-red-500/30 text-red-400 hover:bg-red-500/10 hover:border-red-500/50"
                          onClick={() => setDeleteConfirmOpen(true)}
                        >
                          <Trash2 className="size-3.5" />
                          Delete My Pack Hub Data
                        </Button>
                      ) : (
                        <div className="space-y-2 rounded-lg border border-red-500/20 bg-red-500/[0.04] p-3">
                          <p className="text-xs font-medium text-red-400">
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
          ? "border-[#c4a44a]/20 bg-[#c4a44a]/[0.04] hover:border-[#c4a44a]/30 hover:bg-[#c4a44a]/[0.06]"
          : "border-white/[0.04] bg-white/[0.02] hover:border-white/[0.08] hover:bg-white/[0.04]"
      }`}
    >
      <div
        className={`flex size-8 shrink-0 items-center justify-center rounded-lg ${
          accent === "gold"
            ? "bg-[#c4a44a]/10 text-[#c4a44a]"
            : "bg-white/[0.04] text-muted-foreground group-hover:text-white/70"
        } transition-colors duration-150`}
      >
        <Icon className="size-4" />
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium text-white/90">{label}</p>
        <p className="text-xs text-muted-foreground">{description}</p>
      </div>
      <ChevronRight className="size-4 text-muted-foreground/40 group-hover:text-muted-foreground transition-colors duration-150" />
    </button>
  );
}
