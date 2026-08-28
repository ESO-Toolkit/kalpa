import { useEffect, useRef, useState } from "react";
import { WifiOff } from "lucide-react";
import type { EsouiAddonDetail } from "../types";
import { Button } from "@/components/ui/button";
import { EsouiTabSkeleton } from "@/components/ui/skeletons";
import { EsouiOverview } from "@/components/esoui-overview";
import { useEsouiDetail } from "@/hooks/use-esoui-detail";

interface EsouiTabProps {
  esouiId: number;
  isOffline?: boolean;
}

/**
 * The ESOUI tab of an installed addon's detail pane: the same rich remote view
 * Discover shows, fetched lazily the first time the tab is opened.
 *
 * Offline handling mirrors the rest of the app — the fetch is disabled entirely
 * while offline (the hook never fires without a network), but any detail that
 * was already fetched this session stays on screen with a muted note, and the
 * hook is re-run as soon as connectivity returns.
 */
export function EsouiTab({ esouiId, isOffline }: EsouiTabProps) {
  const { detail, loading, error, refetch } = useEsouiDetail(esouiId, !isOffline);

  // Last successfully-loaded detail. Keeps the rich view on screen if the user
  // drops offline after it loaded, instead of flashing the "requires an
  // internet connection" empty state at them. Derived during render (React's
  // "adjust state when a prop changes" pattern) rather than in an effect, so
  // there is no frame where the fallback lags the hook's value.
  const [cache, setCache] = useState<{ id: number; detail: EsouiAddonDetail } | null>(null);
  if (detail && (cache?.detail !== detail || cache.id !== esouiId)) {
    setCache({ id: esouiId, detail });
  }

  // Re-fetch on the offline → online transition only. The hook is cached and
  // deduped, so this is cheap, and gating on the transition means coming back
  // online doesn't compete with the hook's own enable-driven fetch on mount.
  const wasOffline = useRef(isOffline);
  useEffect(() => {
    if (wasOffline.current && !isOffline) refetch();
    wasOffline.current = isOffline;
  }, [isOffline, refetch]);

  const shown = detail ?? (cache?.id === esouiId ? cache.detail : null);

  if (isOffline && !shown) {
    return (
      <div className="relative flex flex-col items-center justify-center gap-4 px-8 py-12 text-muted-foreground">
        <div className="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 h-[200px] w-[200px] rounded-full bg-primary/[0.04] blur-[60px]" />
        <div className="relative rounded-2xl bg-structure-03 border border-structure-06 p-5 shadow-[0_0_30px_color-mix(in_oklab,var(--primary)_3%,transparent)]">
          <WifiOff
            aria-hidden="true"
            className="size-10 text-muted-foreground/30"
            strokeWidth={1.2}
          />
        </div>
        <div className="relative text-center">
          <p className="font-heading text-sm font-medium text-foreground">You&rsquo;re offline</p>
          <p className="mt-1 text-xs text-muted-foreground">
            ESOUI details require an internet connection
          </p>
        </div>
      </div>
    );
  }

  if (loading && !shown) return <EsouiTabSkeleton />;

  if (error && !shown) {
    return (
      <div className="flex flex-col items-center gap-3 px-8 py-12">
        <div className="rounded-xl border border-status-danger/20 bg-status-danger/[0.04] p-4 text-sm text-status-danger">
          {error}
        </div>
        <Button size="sm" variant="outline" onClick={() => refetch()}>
          Retry
        </Button>
      </div>
    );
  }

  if (!shown) return null;

  return (
    <div className="space-y-3">
      {isOffline && (
        <p className="text-xs text-muted-foreground">
          You&rsquo;re offline — showing cached details.
        </p>
      )}
      <EsouiOverview detail={shown} />
    </div>
  );
}
