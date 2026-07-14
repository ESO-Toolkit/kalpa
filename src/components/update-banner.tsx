import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useReducedMotion } from "motion/react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { SimpleTooltip } from "@/components/ui/tooltip";
import {
  ArrowRightIcon,
  CheckIcon,
  ChevronDownIcon,
  DownloadIcon,
  ListChecksIcon,
  PackageIcon,
  SearchIcon,
  XIcon,
} from "lucide-react";
import { CountingNumber } from "@/components/animate-ui/primitives/texts/counting-number";
import { Slide } from "@/components/animate-ui/primitives/effects/slide";
import { AutoHeight } from "@/components/animate-ui/primitives/effects/auto-height";
import { cn } from "@/lib/utils";

type AddonPhase = "downloading" | "scanning" | "extracting" | "completed" | "failed";

/** A single available update, enriched with a display title for the chooser. */
export interface BannerUpdate {
  folderName: string;
  title: string;
  currentVersion: string;
  remoteVersion: string;
}

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
  /** Available updates, sorted for display — drives the "Choose" checklist. */
  updates: BannerUpdate[];
  onUpdateAll: () => void;
  /** Update only the chosen subset; routes through the same streaming batch path. */
  onUpdateSelected: (folderNames: string[]) => void;
  isOffline?: boolean;
}

function PhaseIcon({ phase }: { phase: AddonPhase }) {
  switch (phase) {
    case "downloading":
      return <DownloadIcon className="h-3 w-3 animate-pulse text-accent-sky" />;
    case "scanning":
      return <SearchIcon className="h-3 w-3 animate-pulse text-violet-400" />;
    case "extracting":
      return <PackageIcon className="h-3 w-3 animate-pulse text-primary" />;
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
          ? "bg-primary/[0.06] border-primary/15"
          : phase === "scanning"
            ? "bg-violet-400/[0.06] border-violet-400/15"
            : "bg-accent-sky/[0.06] border-accent-sky/15";

  return (
    <div
      className={`inline-flex animate-[fade-in_0.3s_ease-out] items-center gap-1.5 rounded-md border px-2 py-0.5 transition-colors duration-300 ease-out ${bgColor}`}
    >
      <PhaseIcon phase={phase} />
      <span className="max-w-[120px] truncate text-[11px] font-medium text-white/70">{name}</span>
    </div>
  );
}

/** One selectable row in the chooser: title + current → new version delta. */
function ChooserRow({
  update,
  checked,
  onToggle,
}: {
  update: BannerUpdate;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <label
      className={cn(
        "flex cursor-pointer items-center gap-2.5 rounded-md px-2 py-1.5 transition-colors duration-150 ease-out hover:bg-white/[0.04]",
        checked && "bg-primary/[0.04]"
      )}
    >
      <Checkbox
        checked={checked}
        onCheckedChange={onToggle}
        tabIndex={0}
        aria-label={`${update.title}, update from ${update.currentVersion || "unknown"} to ${
          update.remoteVersion || "unknown"
        }`}
      />
      <span className="min-w-0 flex-1 truncate text-[13px] text-white/80" title={update.title}>
        {update.title}
      </span>
      <span className="flex shrink-0 items-center gap-1 text-[11px] tabular-nums">
        <span
          className="max-w-[88px] truncate text-white/30"
          title={update.currentVersion || undefined}
        >
          {update.currentVersion || "—"}
        </span>
        <ArrowRightIcon className="h-3 w-3 text-white/20" aria-hidden="true" />
        <span
          className="max-w-[88px] truncate font-medium text-primary/80"
          title={update.remoteVersion || undefined}
        >
          {update.remoteVersion || "—"}
        </span>
      </span>
    </label>
  );
}

function UpdateBannerBase({
  availableCount,
  updatingAll,
  updateProgress,
  addonStatuses,
  updates,
  onUpdateAll,
  onUpdateSelected,
  isOffline,
}: UpdateBannerProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const chooseButtonRef = useRef<HTMLButtonElement>(null);
  const reduceMotion = useReducedMotion();

  const [expanded, setExpanded] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());

  // Reset the picker whenever an update run begins from ANY source — a button
  // click here or a parent-driven auto-update — so the panel can't spring back
  // open with stale picks when the banner re-renders after the run finishes.
  // Adjusting state during render by comparing the previous prop is React's
  // sanctioned alternative to a syncing effect.
  const [prevUpdatingAll, setPrevUpdatingAll] = useState(updatingAll);
  if (updatingAll !== prevUpdatingAll) {
    setPrevUpdatingAll(updatingAll);
    if (updatingAll) {
      setExpanded(false);
      setSelected(new Set());
    }
  }

  const canChoose = !updatingAll && updates.length >= 2;

  // Auto-scroll pill container to the right as new pills appear
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollLeft = scrollRef.current.scrollWidth;
    }
  }, [addonStatuses]);

  // Selections filtered to addons that are still outdated, so a pick that
  // finished updating elsewhere never inflates the count or the submit payload.
  // Derived (not stored) to avoid syncing state inside an effect.
  const effectiveSelected = useMemo(() => {
    if (selected.size === 0) return selected;
    const valid = new Set(updates.map((u) => u.folderName));
    const next = new Set<string>();
    for (const folder of selected) if (valid.has(folder)) next.add(folder);
    return next.size === selected.size ? selected : next;
  }, [selected, updates]);

  const selectedCount = effectiveSelected.size;
  const allSelected = updates.length > 0 && selectedCount === updates.length;
  const someSelected = selectedCount > 0 && !allSelected;

  const toggleOne = (folderName: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(folderName)) next.delete(folderName);
      else next.add(folderName);
      return next;
    });

  const toggleAll = () =>
    setSelected(allSelected ? new Set() : new Set(updates.map((u) => u.folderName)));

  const submitSelected = () => {
    const folders = [...effectiveSelected];
    if (folders.length === 0) return;
    onUpdateSelected(folders);
    setSelected(new Set());
    setExpanded(false);
    // Collapsing unmounts the submit button it was triggered from; move focus to
    // the still-mounted "Choose" trigger so keyboard/SR focus isn't dropped to
    // <body> (WCAG 2.4.3).
    chooseButtonRef.current?.focus();
  };

  // Update All takes over the banner with live progress, so fold the picker away.
  const handleUpdateAllClick = () => {
    setExpanded(false);
    onUpdateAll();
  };

  const total = updateProgress?.total ?? 0;
  const doneCount = (updateProgress?.completed ?? 0) + (updateProgress?.failed ?? 0);
  const allDone = updatingAll && total > 0 && doneCount === total;

  // Sort statuses: in-progress first, then completed/failed
  const sortedEntries = useMemo(
    () =>
      [...addonStatuses.entries()].sort((a, b) => {
        const order: Record<AddonPhase, number> = {
          downloading: 0,
          scanning: 1,
          extracting: 2,
          failed: 3,
          completed: 4,
        };
        return order[a[1]] - order[b[1]];
      }),
    [addonStatuses]
  );

  if (availableCount === 0 && !updatingAll) return null;

  const progressPct = total > 0 ? ((doneCount / total) * 100).toFixed(0) : "0";

  return (
    <Slide
      direction="down"
      offset={20}
      transition={{ type: "spring", stiffness: 300, damping: 25 }}
    >
      <div className="border-b border-primary/15 bg-gradient-to-r from-primary/[0.06] via-primary/[0.03] to-transparent backdrop-blur-sm">
        {/* Header row */}
        <div className="flex items-center justify-between gap-3 px-5 py-2">
          {updatingAll && updateProgress ? (
            <div className="flex items-center gap-3 min-w-0">
              {/* Animated counter */}
              <div className="flex items-center gap-2">
                <div className="relative h-5 w-5">
                  {/* Spinning ring */}
                  <svg aria-hidden="true" className="h-5 w-5 -rotate-90" viewBox="0 0 20 20">
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
                      stroke="var(--primary)"
                      strokeWidth="2"
                      strokeLinecap="round"
                      strokeDasharray={`${(doneCount / Math.max(total, 1)) * 50.27} 50.27`}
                      className="transition-all duration-500 ease-out"
                    />
                  </svg>
                </div>
                <span className="text-sm font-medium tabular-nums text-primary">
                  <CountingNumber number={doneCount} transition={{ stiffness: 200, damping: 25 }} />
                  /
                  <CountingNumber
                    number={total}
                    initiallyStable
                    transition={{ stiffness: 200, damping: 25 }}
                  />
                  <span className="ml-1.5 text-xs text-white/30">{progressPct}%</span>
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
            <span className="text-sm font-medium text-primary">
              <CountingNumber
                number={availableCount}
                transition={{ stiffness: 200, damping: 25 }}
                initiallyStable
              />{" "}
              update{availableCount > 1 ? "s" : ""} available
            </span>
          )}
          <div className="flex shrink-0 items-center gap-2">
            {canChoose && (
              <Button
                ref={chooseButtonRef}
                onClick={() => setExpanded((v) => !v)}
                size="sm"
                variant="ghost"
                aria-expanded={expanded}
                aria-controls={expanded ? "update-chooser" : undefined}
              >
                <ListChecksIcon className="h-3.5 w-3.5" />
                Choose
                <ChevronDownIcon
                  className={cn(
                    "h-3.5 w-3.5 transition-transform duration-200 ease-out motion-reduce:transition-none",
                    expanded && "rotate-180"
                  )}
                />
              </Button>
            )}
            <SimpleTooltip content={isOffline ? "Updates require an internet connection" : ""}>
              <Button onClick={handleUpdateAllClick} size="sm" disabled={updatingAll || isOffline}>
                {updatingAll ? "Updating..." : "Update All"}
              </Button>
            </SimpleTooltip>
          </div>
        </div>

        {/* Expandable chooser — pick a subset to update in-context */}
        {canChoose && (
          <AutoHeight
            deps={[expanded, updates.length]}
            transition={reduceMotion ? { duration: 0 } : undefined}
          >
            {expanded ? (
              <div
                id="update-chooser"
                role="region"
                aria-label="Choose which addons to update"
                className="border-t border-white/[0.06] px-5 pt-2.5 pb-3"
              >
                {/* Toolbar: select-all + scoped update action */}
                <div className="flex items-center justify-between gap-3 pb-1.5">
                  <label className="flex cursor-pointer select-none items-center gap-2">
                    <Checkbox
                      checked={allSelected}
                      indeterminate={someSelected}
                      onCheckedChange={toggleAll}
                      aria-label="Select all updates"
                    />
                    <span className="text-[11px] font-medium uppercase tracking-wide text-white/40">
                      {selectedCount > 0 ? `${selectedCount} selected` : "Select all"}
                    </span>
                  </label>
                  <SimpleTooltip
                    content={isOffline ? "Updates require an internet connection" : ""}
                  >
                    <Button
                      onClick={submitSelected}
                      size="sm"
                      variant={selectedCount > 0 ? "default" : "secondary"}
                      disabled={selectedCount === 0 || isOffline}
                    >
                      <DownloadIcon className="h-3.5 w-3.5" />
                      Update{selectedCount > 0 ? ` ${selectedCount}` : ""} selected
                    </Button>
                  </SimpleTooltip>
                </div>

                {/* Scrollable checklist of available updates */}
                <div className="-mx-1 max-h-[240px] overflow-y-auto px-1">
                  {updates.map((update) => (
                    <ChooserRow
                      key={update.folderName}
                      update={update}
                      checked={effectiveSelected.has(update.folderName)}
                      onToggle={() => toggleOne(update.folderName)}
                    />
                  ))}
                </div>
              </div>
            ) : null}
          </AutoHeight>
        )}

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
          <div
            className="relative h-[3px] bg-white/[0.04]"
            role="progressbar"
            aria-valuenow={Number(progressPct)}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-label={`Update progress: ${progressPct}%`}
          >
            {/* Completed fill */}
            <div
              className="absolute inset-y-0 left-0 bg-gradient-to-r from-primary to-primary-hover transition-all duration-500 ease-out"
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
    </Slide>
  );
}

// Memoized: during Update All the banner is the only legitimate consumer of the
// per-event progress state; everything else in App bails, and the banner bails
// out of unrelated renders (keystrokes, dialogs) in turn.
export const UpdateBanner = memo(UpdateBannerBase);
