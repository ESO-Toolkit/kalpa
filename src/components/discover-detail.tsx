import { memo, useState, useEffect } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import type { EsouiSearchResult, InstallResult } from "../types";
import { Button } from "@/components/ui/button";
import { InfoPill } from "@/components/ui/info-pill";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { getDependencyPolicy } from "@/lib/dependency-policy";
import { useResolvePendingDeps } from "@/lib/dependency-prompt-context";
import { useEnsureEsoNotBlocking } from "@/lib/eso-running-context";
import { EsouiOverview } from "@/components/esoui-overview";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { Fade } from "@/components/animate-ui/primitives/effects/fade";
import { DiscoverDetailSkeleton } from "@/components/ui/skeletons";
import { useEsouiDetail } from "@/hooks/use-esoui-detail";
import { Download, ExternalLink, Check, Trash2, Search } from "lucide-react";

interface DiscoverDetailProps {
  result: EsouiSearchResult | null;
  addonsPath: string;
  onInstalled: () => void;
  onRemoveByEsouiId?: (esouiId: number) => void;
  installedEsouiIds: Set<number>;
  isOffline?: boolean;
}

function DiscoverDetailBase({
  result,
  addonsPath,
  onInstalled,
  onRemoveByEsouiId,
  installedEsouiIds,
  isOffline,
}: DiscoverDetailProps) {
  const ensureEsoNotBlocking = useEnsureEsoNotBlocking();
  const resolvePendingDeps = useResolvePendingDeps();
  const { detail, loading, error } = useEsouiDetail(result?.id, true);
  const [installingId, setInstallingId] = useState<number | null>(null);
  const [installSuccess, setInstallSuccess] = useState<InstallResult | null>(null);

  // Per-selection state resets when a different search result is opened.
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setInstallSuccess(null);
  }, [result]);

  if (!result) {
    return (
      <div className="relative flex flex-1 flex-col items-center justify-center gap-4 text-muted-foreground px-8">
        <div className="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 h-[200px] w-[200px] rounded-full bg-primary/[0.04] blur-[60px]" />
        <div className="relative rounded-2xl bg-structure-03 border border-structure-06 p-5 shadow-[0_0_30px_color-mix(in_oklab,var(--primary)_3%,transparent)]">
          <Search
            aria-hidden="true"
            className="size-10 text-muted-foreground/30"
            strokeWidth={1.2}
          />
        </div>
        <div className="relative text-center">
          <p className="font-heading text-sm font-medium text-foreground">Discover Addons</p>
          <p className="mt-1 text-xs text-muted-foreground">
            Search or browse to see addon details here
          </p>
        </div>
      </div>
    );
  }

  const handleInstall = async (downloadUrl?: string) => {
    if (!result) return;
    if (installingId === result.id) return;
    setInstallingId(result.id);
    if (!(await ensureEsoNotBlocking())) {
      setInstallingId(null);
      return;
    }
    setInstallSuccess(null);
    try {
      let url = downloadUrl;
      let title = detail?.title ?? result.title;
      let version = detail?.version ?? "";
      if (!url) {
        const info = await invokeOrThrow<{
          id: number;
          title: string;
          version: string;
          downloadUrl: string;
        }>("resolve_esoui_addon", { input: String(result.id) });
        url = info.downloadUrl;
        title = info.title;
        version = info.version;
      }
      const res = await invokeOrThrow<InstallResult>("install_addon", {
        addonsPath,
        downloadUrl: url,
        esouiId: result.id,
        esouiTitle: title,
        esouiVersion: version,
        dependencyPolicy: await getDependencyPolicy(),
      });
      setInstallSuccess(res);
      toast.success(`Installed ${res.installedFolders.join(", ")}`);
      onInstalled();
      // Empty unless the policy is "ask"; the app-level picker owns the rest.
      // Same `addonsPath` this install was started with, so a folder switch
      // while it ran can't redirect the deps to another instance.
      void resolvePendingDeps(res.pendingDeps, addonsPath);
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setInstallingId(null);
    }
  };

  if (loading) {
    return (
      <Fade className="flex-1 overflow-hidden">
        <DiscoverDetailSkeleton />
      </Fade>
    );
  }

  if (error) {
    return (
      <Fade className="flex-1">
        <div className="flex flex-1 items-center justify-center px-8 h-full">
          <div className="rounded-xl border border-status-danger/20 bg-status-danger/[0.04] p-4 text-sm text-status-danger">
            {error}
          </div>
        </div>
      </Fade>
    );
  }

  if (!detail) return null;

  return (
    <Fade className="flex-1 overflow-hidden">
      <div className="h-full overflow-y-auto p-6 space-y-5" data-discover-detail>
        {/* Header */}
        <div className="flex items-start justify-between gap-4">
          <div className="flex-1 min-w-0">
            <h2 className="font-heading text-xl font-semibold bg-gradient-to-r from-primary to-primary-hover bg-clip-text text-transparent">
              {detail.title}
            </h2>
            {detail.version && (
              <span className="mt-1 inline-block text-xs font-mono text-muted-foreground bg-structure-04 px-1.5 py-0.5 rounded">
                v{detail.version}
              </span>
            )}
            <div className="mt-1 flex items-center gap-2 text-sm text-muted-foreground">
              <span>by {detail.author}</span>
              {result.category && (
                <>
                  <span className="text-muted-foreground/20">&middot;</span>
                  <InfoPill color="muted">{result.category}</InfoPill>
                </>
              )}
            </div>
            {detail.compatibility && (
              <div className="mt-1.5 text-xs text-muted-foreground">
                Compatible with {detail.compatibility}
              </div>
            )}
          </div>
          <div className="flex flex-col gap-1.5 items-end shrink-0">
            <div className="flex items-center gap-1.5">
              {(installSuccess || installedEsouiIds.has(result.id)) && onRemoveByEsouiId && (
                <Button
                  variant="outline"
                  onClick={() => {
                    onRemoveByEsouiId(result.id);
                    setInstallSuccess(null);
                  }}
                  className="border-status-danger/20 text-status-danger hover:bg-status-danger/[0.08] hover:text-status-danger"
                >
                  <Trash2 className="size-3.5" />
                  Uninstall
                </Button>
              )}
              <SimpleTooltip content={isOffline ? "Installs require an internet connection" : ""}>
                <Button
                  onClick={() => handleInstall(detail.downloadUrl)}
                  disabled={installingId !== null || isOffline}
                  className="min-w-[100px]"
                >
                  {installingId !== null ? (
                    <span className="flex items-center gap-2">
                      <span className="inline-block size-3 animate-spin rounded-full border-2 border-[var(--primary-foreground)]/20 border-t-[var(--primary-foreground)]" />
                      Installing
                    </span>
                  ) : installSuccess || installedEsouiIds.has(result.id) ? (
                    <span className="flex items-center gap-2">
                      <Check className="size-3.5" />
                      Reinstall
                    </span>
                  ) : (
                    <>
                      <Download className="size-3.5" />
                      Install
                    </>
                  )}
                </Button>
              </SimpleTooltip>
            </div>
            <button
              onClick={() => openUrl(`https://www.esoui.com/downloads/info${detail.id}.html`)}
              className="text-xs text-muted-foreground hover:text-foreground transition-colors flex items-center gap-1 cursor-pointer"
            >
              <ExternalLink className="size-3" />
              View on ESOUI
            </button>
          </div>
        </div>

        {/* Install success */}
        {installSuccess && (
          <div className="rounded-xl border border-status-success/20 bg-status-success/[0.04] p-3 text-sm text-status-success flex items-center gap-2">
            <Check className="size-4 shrink-0" />
            <span className="flex-1">
              Installed: {installSuccess.installedFolders.join(", ")}
              {installSuccess.installedDeps.length > 0 &&
                ` + deps: ${installSuccess.installedDeps.join(", ")}`}
              {installSuccess.skippedDeps.length > 0 &&
                ` (skipped: ${installSuccess.skippedDeps.join(", ")})`}
            </span>
            {onRemoveByEsouiId && result && (
              <button
                onClick={() => {
                  onRemoveByEsouiId(result.id);
                  setInstallSuccess(null);
                }}
                className="shrink-0 rounded-lg border border-status-danger/20 bg-status-danger/[0.06] px-2.5 py-1 text-xs font-medium text-status-danger hover:bg-status-danger/[0.12] transition-colors"
              >
                <Trash2 className="size-3 inline -mt-px mr-1" />
                Uninstall
              </button>
            )}
          </div>
        )}

        <EsouiOverview detail={detail} />
      </div>
    </Fade>
  );
}

// Memoized: bails out of App re-renders it doesn't consume (search keystrokes,
// update-progress events, dialog toggles).
export const DiscoverDetail = memo(DiscoverDetailBase);
