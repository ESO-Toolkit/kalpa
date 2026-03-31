import { useState, useEffect, useMemo, useCallback, useRef } from "react";
import { toast } from "sonner";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { GlassPanel } from "@/components/ui/glass-panel";
import { SectionHeader } from "@/components/ui/section-header";
import { InfoPill } from "@/components/ui/info-pill";
import { getTauriErrorMessage, invokeOrThrow, invokeResult } from "@/lib/tauri";
import { cn, decodeHtml } from "@/lib/utils";
import {
  DownloadIcon,
  Loader2Icon,
  CheckIcon,
  AlertCircleIcon,
  PackageIcon,
  XIcon,
  RefreshCwIcon,
} from "lucide-react";
import type {
  RosterPack,
  PackAddonEntry,
  EsouiAddonInfo,
  InstallResult,
  AddonManifest,
} from "../types";

interface RosterPackInstallProps {
  packId: string;
  addonsPath: string;
  installedAddons: AddonManifest[];
  onClose: () => void;
  onRefresh: () => void;
}

type AddonStatus = "pending" | "installing" | "installed" | "failed";

interface AddonInstallState {
  addon: PackAddonEntry;
  status: AddonStatus;
  selected: boolean;
}

export function RosterPackInstall({
  packId,
  addonsPath,
  installedAddons,
  onClose,
  onRefresh,
}: RosterPackInstallProps) {
  const [pack, setPack] = useState<RosterPack | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [addonStates, setAddonStates] = useState<AddonInstallState[]>([]);
  const [installing, setInstalling] = useState(false);
  const [installProgress, setInstallProgress] = useState<{
    completed: number;
    failed: number;
    total: number;
  } | null>(null);

  const cancelledRef = useRef(false);
  const fetchSeqRef = useRef(0);

  const installedEsouiIds = useMemo(
    () => new Set(installedAddons.filter((a) => a.esouiId).map((a) => a.esouiId!)),
    [installedAddons]
  );

  const fetchPack = useCallback(async () => {
    const seq = ++fetchSeqRef.current;
    cancelledRef.current = true; // abort any in-flight install loop
    setLoading(true);
    setError(null);
    setInstalling(false);
    setInstallProgress(null);
    try {
      const result = await invokeOrThrow<RosterPack>("fetch_roster_pack", {
        packId,
      });
      if (seq !== fetchSeqRef.current) return;
      setPack(result);
      setAddonStates(
        result.addons.map((addon) => ({
          addon,
          status: installedEsouiIds.has(addon.esouiId) ? "installed" : "pending",
          selected: addon.required || !installedEsouiIds.has(addon.esouiId),
        }))
      );
    } catch (err) {
      if (seq !== fetchSeqRef.current) return;
      setError(getTauriErrorMessage(err));
    } finally {
      if (seq === fetchSeqRef.current) setLoading(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [packId]);

  useEffect(() => {
    void fetchPack();
  }, [fetchPack]);

  const toggleAddon = useCallback(
    (esouiId: number) => {
      if (installing) return;
      setAddonStates((prev) =>
        prev.map((s) => {
          if (s.addon.esouiId !== esouiId) return s;
          if (s.status === "installed") return s;
          if (s.addon.required) return s;
          return { ...s, selected: !s.selected };
        })
      );
    },
    [installing]
  );

  const addonsToInstall = useMemo(
    () => addonStates.filter((s) => s.selected && s.status === "pending"),
    [addonStates]
  );

  const handleInstall = useCallback(async () => {
    if (addonsToInstall.length === 0) {
      toast.info("All selected addons are already installed.");
      return;
    }

    cancelledRef.current = false;
    setInstalling(true);
    setInstallProgress({ completed: 0, failed: 0, total: addonsToInstall.length });

    let completed = 0;
    let failed = 0;

    for (const item of addonsToInstall) {
      if (cancelledRef.current) break;

      setAddonStates((prev) =>
        prev.map((s) =>
          s.addon.esouiId === item.addon.esouiId ? { ...s, status: "installing" } : s
        )
      );

      const info = await invokeResult<EsouiAddonInfo>("resolve_esoui_addon", {
        input: String(item.addon.esouiId),
      });

      if (cancelledRef.current) break;

      if (!info.ok) {
        failed++;
        setAddonStates((prev) =>
          prev.map((s) => (s.addon.esouiId === item.addon.esouiId ? { ...s, status: "failed" } : s))
        );
        setInstallProgress({ completed, failed, total: addonsToInstall.length });
        continue;
      }

      const result = await invokeResult<InstallResult>("install_addon", {
        addonsPath,
        downloadUrl: info.data.downloadUrl,
        esouiId: item.addon.esouiId,
        esouiTitle: info.data.title,
        esouiVersion: info.data.version,
      });

      if (cancelledRef.current) break;

      if (result.ok) {
        completed++;
        setAddonStates((prev) =>
          prev.map((s) =>
            s.addon.esouiId === item.addon.esouiId ? { ...s, status: "installed" } : s
          )
        );
      } else {
        failed++;
        setAddonStates((prev) =>
          prev.map((s) => (s.addon.esouiId === item.addon.esouiId ? { ...s, status: "failed" } : s))
        );
      }

      setInstallProgress({ completed, failed, total: addonsToInstall.length });
    }

    if (cancelledRef.current) return;

    setInstalling(false);
    setInstallProgress(null);

    if (completed > 0) {
      onRefresh();
      toast.success(`Installed ${completed} addon${completed !== 1 ? "s" : ""}`);
    }
    if (failed > 0) {
      toast.error(`${failed} addon${failed !== 1 ? "s" : ""} failed to install`);
    }
  }, [addonsToInstall, addonsPath, onRefresh]);

  useEffect(() => {
    return () => {
      cancelledRef.current = true;
    };
  }, []);

  const allInstalled = addonStates.length > 0 && addonStates.every((s) => s.status === "installed");

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !installing) onClose();
      }}
    >
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <PackageIcon className="h-5 w-5 text-[#c4a44a]" />
            {loading ? "Loading Pack..." : pack ? decodeHtml(pack.title) : "Roster Pack"}
          </DialogTitle>
        </DialogHeader>

        <div className="flex max-h-[60vh] flex-col gap-3 overflow-y-auto pr-1">
          {loading && (
            <div className="flex items-center justify-center py-8">
              <Loader2Icon className="h-6 w-6 animate-spin text-[#c4a44a]" />
            </div>
          )}

          {error && (
            <GlassPanel variant="subtle" className="flex flex-col gap-2 p-3">
              <div className="flex items-center gap-2 text-red-400">
                <AlertCircleIcon className="h-4 w-4 shrink-0" />
                <span className="text-sm">{error}</span>
              </div>
              <Button
                variant="outline"
                size="sm"
                className="self-start"
                onClick={() => void fetchPack()}
              >
                <RefreshCwIcon className="mr-1.5 h-3.5 w-3.5" />
                Retry
              </Button>
            </GlassPanel>
          )}

          {pack && !loading && addonStates.length === 0 && (
            <p className="py-4 text-center text-sm text-white/40">This pack has no addons.</p>
          )}

          {pack && !loading && addonStates.length > 0 && (
            <>
              <SectionHeader>Addons ({addonStates.length})</SectionHeader>

              <div className="flex flex-col gap-1">
                {addonStates.map(({ addon, status, selected }) => (
                  <GlassPanel
                    key={addon.esouiId}
                    variant="subtle"
                    className={cn(
                      "flex items-center gap-3 px-3 py-2 transition-colors duration-150",
                      status === "installed" && "opacity-60"
                    )}
                  >
                    {/* Selection checkbox */}
                    <button
                      type="button"
                      disabled={installing || status === "installed" || addon.required}
                      onClick={() => toggleAddon(addon.esouiId)}
                      className={cn(
                        "flex h-4 w-4 shrink-0 items-center justify-center rounded border transition-colors",
                        selected || status === "installed"
                          ? "border-sky-400 bg-sky-400/20"
                          : "border-white/20 bg-white/[0.03]",
                        (installing || status === "installed" || addon.required) &&
                          "cursor-not-allowed opacity-50"
                      )}
                    >
                      {(selected || status === "installed") && (
                        <CheckIcon className="h-3 w-3 text-sky-400" />
                      )}
                    </button>

                    {/* Addon info */}
                    <div className="flex min-w-0 flex-1 flex-col">
                      <span className="truncate text-sm font-medium text-white/90">
                        {addon.name}
                      </span>
                      {addon.note && (
                        <span className="truncate text-xs text-white/40">{addon.note}</span>
                      )}
                    </div>

                    {/* Status indicators */}
                    <div className="flex shrink-0 items-center gap-1.5">
                      {addon.required && (
                        <InfoPill color="gold" className="text-[10px]">
                          Required
                        </InfoPill>
                      )}
                      {status === "installed" && <CheckIcon className="h-4 w-4 text-emerald-400" />}
                      {status === "installing" && (
                        <Loader2Icon className="h-4 w-4 animate-spin text-sky-400" />
                      )}
                      {status === "failed" && <XIcon className="h-4 w-4 text-red-400" />}
                    </div>
                  </GlassPanel>
                ))}
              </div>

              {installProgress && (
                <div className="mt-1">
                  <div className="h-1 overflow-hidden rounded-full bg-white/[0.06]">
                    <div
                      className="h-full rounded-full bg-sky-400 transition-all duration-300"
                      style={{
                        width: `${(installProgress.completed / installProgress.total) * 100}%`,
                      }}
                    />
                  </div>
                  <p className="mt-1 text-center text-xs text-white/40">
                    {installProgress.completed + installProgress.failed} / {installProgress.total}
                    {installProgress.failed > 0 && (
                      <span className="text-red-400"> ({installProgress.failed} failed)</span>
                    )}
                  </p>
                </div>
              )}
            </>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={installing}>
            {allInstalled ? "Done" : "Cancel"}
          </Button>
          {pack && !allInstalled && (
            <Button
              onClick={() => void handleInstall()}
              disabled={installing || addonsToInstall.length === 0}
            >
              {installing ? (
                <>
                  <Loader2Icon className="mr-2 h-4 w-4 animate-spin" />
                  Installing...
                </>
              ) : (
                <>
                  <DownloadIcon className="mr-2 h-4 w-4" />
                  Install {addonsToInstall.length} Addon{addonsToInstall.length !== 1 ? "s" : ""}
                </>
              )}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
