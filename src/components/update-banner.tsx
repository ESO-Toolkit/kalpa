import { useEffect, useRef } from "react";
import { Button } from "@/components/ui/button";
import { CheckIcon, XIcon, DownloadIcon, PackageIcon } from "lucide-react";

type AddonPhase = "downloading" | "extracting" | "completed" | "failed";

interface UpdateBannerProps {
  availableCount: number;
  updatingAll: boolean;
  updateProgress: {
    completed: number;
    failed: number;
    total: number;
    currentAddon?: string;
  } | null;
  addonStatuses: Map<string, AddonPhase>;
  onUpdateAll: () => void;
  isOffline?: boolean;
}

function PhaseIcon({ phase }: { phase: AddonPhase }) {
  switch (phase) {
    case "downloading":
      return <DownloadIcon className="h-3 w-3 animate-pulse text-[#38bdf8]" />;
    case "extracting":
      return <PackageIcon className="h-3 w-3 animate-pulse text-[#c4a44a]" />;
    case "completed":
      return (
        <div className="flex h-3.5 w-3.5 items-center justify-center rounded-full bg-emerald-500/20">
          <CheckIcon className="h-2.5 w-2.5 text-emerald-400" strokeWidth={3} />
        </div>
      );
    case "failed":
      return (
        <div className="flex h-3.5 w-3.5 items-center justify-center rounded-full bg-red-500/20">
          <XIcon className="h-2.5 w-2.5 text-red-400" strokeWidth={3} />
        </div>
      );
  }
}

function AddonStatusPill({ name, phase }: { name: string; phase: AddonPhase }) {
  const bgColor =
    phase === "completed"
      ? "bg-emerald-500/[0.06] border-emerald-500/15"
      : phase === "failed"
        ? "bg-red-500/[0.06] border-red-500/15"
        : phase === "extracting"
          ? "bg-[#c4a44a]/[0.06] border-[#c4a44a]/15"
          : "bg-[#38bdf8]/[0.06] border-[#38bdf8]/15";

  return (
    <div
      className={`inline-flex animate-[fade-in_0.3s_ease-out] items-center gap-1.5 rounded-md border px-2 py-0.5 transition-colors duration-300 ease-out ${bgColor}`}
    >
      <PhaseIcon phase={phase} />
      <span className="max-w-[120px] truncate text-[11px] font-medium text-white/70">{name}</span>
    </div>
  );
}

export function UpdateBanner({
  availableCount,
  updatingAll,
  updateProgress,
  addonStatuses,
  onUpdateAll,
  isOffline,
}: UpdateBannerProps) {
  const scrollRef = useRef<HTMLDivElement>(null);

  // Auto-scroll pill container to the right as new pills appear
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollLeft = scrollRef.current.scrollWidth;
    }
  }, [addonStatuses]);

  const total = updateProgress?.total ?? 0;
  const doneCount = (updateProgress?.completed ?? 0) + (updateProgress?.failed ?? 0);
  const allDone = updatingAll && total > 0 && doneCount === total;

  if (availableCount === 0 && !updatingAll) return null;

  // Sort statuses: in-progress first, then completed/failed
  const sortedEntries = [...addonStatuses.entries()].sort((a, b) => {
    const order: Record<AddonPhase, number> = {
      downloading: 0,
      extracting: 1,
      failed: 2,
      completed: 3,
    };
    return order[a[1]] - order[b[1]];
  });

  const progressPct = total > 0 ? ((doneCount / total) * 100).toFixed(0) : "0";

  return (
    <div className="animate-[slide-down_0.3s_ease-out] border-b border-[#c4a44a]/15 bg-gradient-to-r from-[#c4a44a]/[0.06] via-[#c4a44a]/[0.03] to-transparent backdrop-blur-sm">
      {/* Header row */}
      <div className="flex items-center justify-between px-5 py-2">
        {updatingAll && updateProgress ? (
          <div className="flex items-center gap-3 min-w-0">
            {/* Animated counter */}
            <div className="flex items-center gap-2">
              <div className="relative h-5 w-5">
                {/* Spinning ring */}
                <svg className="h-5 w-5 -rotate-90" viewBox="0 0 20 20">
                  <circle
                    cx="10"
                    cy="10"
                    r="8"
                    fill="none"
                    stroke="rgba(255,255,255,0.06)"
                    strokeWidth="2"
                  />
                  <circle
                    cx="10"
                    cy="10"
                    r="8"
                    fill="none"
                    stroke="#c4a44a"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeDasharray={`${(doneCount / Math.max(total, 1)) * 50.27} 50.27`}
                    className="transition-all duration-500 ease-out"
                  />
                </svg>
              </div>
              <span className="text-sm font-medium tabular-nums text-[#c4a44a]">
                {doneCount}/{total}
              </span>
            </div>

            {/* Phase summary */}
            <span className="text-xs text-white/40">
              {allDone ? (
                <span className="text-emerald-400 animate-[fade-in_0.3s_ease-out]">All done</span>
              ) : updateProgress.failed > 0 ? (
                <span className="text-red-400/70">{updateProgress.failed} failed</span>
              ) : (
                "Updating addons..."
              )}
            </span>
          </div>
        ) : (
          <span className="text-sm font-medium text-[#c4a44a]">
            {availableCount} update{availableCount > 1 ? "s" : ""} available
          </span>
        )}
        <Button
          onClick={onUpdateAll}
          size="sm"
          disabled={updatingAll || isOffline}
          title={isOffline ? "Updates require an internet connection" : undefined}
        >
          {updatingAll ? "Updating..." : "Update All"}
        </Button>
      </div>

      {/* Per-addon streaming pills */}
      {updatingAll && sortedEntries.length > 0 && (
        <div
          ref={scrollRef}
          className="flex gap-1.5 overflow-x-auto px-5 pb-2 [&::-webkit-scrollbar]:hidden [-ms-overflow-style:none] [scrollbar-width:none]"
        >
          {sortedEntries.map(([name, phase]) => (
            <AddonStatusPill key={name} name={name} phase={phase} />
          ))}
        </div>
      )}

      {/* Segmented progress bar */}
      {updatingAll && total > 0 && (
        <div className="relative h-[3px] bg-white/[0.04]">
          {/* Completed fill */}
          <div
            className="absolute inset-y-0 left-0 bg-gradient-to-r from-[#c4a44a] to-[#d4b45a] transition-all duration-500 ease-out"
            style={{ width: `${progressPct}%` }}
          />
          {/* Shimmer overlay on active bar */}
          {!allDone && Number(progressPct) > 0 && (
            <div
              className="absolute inset-y-0 left-0 overflow-hidden transition-all duration-500 ease-out"
              style={{ width: `${progressPct}%` }}
            >
              <div className="h-full w-full animate-[shimmer_1.5s_ease-in-out_infinite] bg-gradient-to-r from-transparent via-white/20 to-transparent" />
            </div>
          )}
        </div>
      )}
    </div>
  );
}
