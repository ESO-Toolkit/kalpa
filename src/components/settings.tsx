import { useState, useEffect } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { getSetting, setSetting } from "@/lib/store";
import { getTauriErrorMessage, invokeOrThrow, invokeResult } from "@/lib/tauri";
import type { GameInstance, ImportResult } from "../types";
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
  Shield,
  Sparkles,
} from "lucide-react";

type SettingsTab = "general" | "tools" | "data";

interface SettingsProps {
  addonsPath: string;
  knownInstances: GameInstance[];
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
  { id: "tools", label: "Tools", icon: Wrench },
  { id: "data", label: "Data", icon: Database },
];

export function Settings({
  addonsPath,
  knownInstances,
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
  const [minionDetected, setMinionDetected] = useState(false);
  const [redetecting, setRedetecting] = useState(false);
  const [redetectedInstances, setRedetectedInstances] = useState<GameInstance[] | null>(null);

  useEffect(() => {
    void getSetting<boolean>("autoUpdate", false).then(setAutoUpdate);
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
      toast.error(`Failed to open folder picker: ${e}`);
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
        const detected = instances[0].addonsPath;
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

  const pathDirty = path.trim() !== addonsPath;

  return (
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
          <AnimatePresence>
            {activeTab === "general" && (
              <motion.div
                key={activeTab}
                initial={{ opacity: 0, y: 3 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0 }}
                transition={{ type: "spring", stiffness: 600, damping: 35 }}
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
              </motion.div>
            )}

            {activeTab === "tools" && (
              <motion.div
                key={activeTab}
                initial={{ opacity: 0, y: 3 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0 }}
                transition={{ type: "spring", stiffness: 600, damping: 35 }}
                className="space-y-2"
              >
                <ToolItem
                  icon={Archive}
                  label="SavedVariables Backup"
                  description="Back up and restore your addon settings"
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
                initial={{ opacity: 0, y: 3 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0 }}
                transition={{ type: "spring", stiffness: 600, damping: 35 }}
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
                    <Button variant="outline" size="sm" onClick={handleImport} disabled={importing}>
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
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      </DialogContent>
    </Dialog>
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
