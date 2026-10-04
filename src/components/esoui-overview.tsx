import { useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import type { EsouiAddonDetail } from "@/types";
import { SectionHeader } from "@/components/ui/section-header";
import { RichDescription } from "@/components/ui/rich-description";
import { ChangelogView, changelogVersionCount } from "@/components/changelog-view";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import {
  Download,
  Calendar,
  Star,
  FileDown,
  Clock,
  ChevronLeft,
  ChevronRight,
  Check,
  Copy,
} from "lucide-react";

interface EsouiOverviewProps {
  detail: EsouiAddonDetail;
  /** Hide the ESOUI description when the host pane already shows one. */
  showDescription?: boolean;
  /** Installed version, when the host pane has one — marks a matching
   * changelog entry. Discover omits it: nothing is installed there. */
  installedVersion?: string;
}

/**
 * The read-only ESOUI presentation of an addon: stats, metadata, screenshots,
 * description and changelog. Contains no actions, so it can be dropped into
 * either the Discover pane or the installed-addon pane.
 */
export function EsouiOverview({
  detail,
  showDescription = true,
  installedVersion,
}: EsouiOverviewProps) {
  const [screenshotIdx, setScreenshotIdx] = useState(0);
  const [md5Copied, setMd5Copied] = useState(false);
  const md5TimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const versionCount = useMemo(() => changelogVersionCount(detail.changeLog), [detail.changeLog]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setScreenshotIdx(0);
    setMd5Copied(false);
    if (md5TimerRef.current) clearTimeout(md5TimerRef.current);
  }, [detail.id]);

  // Keyboard navigation for screenshots — only when this pane holds focus
  useEffect(() => {
    if (detail.screenshots.length <= 1) return;

    const handler = (e: KeyboardEvent) => {
      const target = e.target as Element;
      if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) return;
      if (target.closest('[role="listbox"], [role="combobox"], [role="option"], select')) return;
      // Only handle arrow keys when focus is within the detail panel
      const panel = containerRef.current?.parentElement ?? containerRef.current;
      if (!panel?.contains(document.activeElement ?? document.body)) return;
      if (e.key === "ArrowLeft") {
        setScreenshotIdx((prev) => (prev > 0 ? prev - 1 : detail.screenshots.length - 1));
      } else if (e.key === "ArrowRight") {
        setScreenshotIdx((prev) => (prev < detail.screenshots.length - 1 ? prev + 1 : 0));
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [detail]);

  useEffect(() => {
    return () => {
      if (md5TimerRef.current) clearTimeout(md5TimerRef.current);
    };
  }, []);

  const safeIdx = Math.max(0, Math.min(screenshotIdx, detail.screenshots.length - 1));

  return (
    <div ref={containerRef} className="space-y-5">
      {/* Quick Stats Bar */}
      <div className="grid grid-cols-4 gap-2">
        <StatCard
          icon={<Download className="size-3.5 text-accent-sky" />}
          label="Downloads"
          value={detail.totalDownloads}
          accent="sky"
        />
        <StatCard
          icon={<FileDown className="size-3.5 text-status-success" />}
          label="Monthly"
          value={detail.monthlyDownloads}
          accent="emerald"
        />
        <StatCard
          icon={<Star className="size-3.5 text-primary" />}
          label="Favorites"
          value={detail.favorites}
          accent="gold"
        />
        <StatCard
          icon={<Clock className="size-3.5 text-status-library" />}
          label="Updated"
          value={detail.updated}
          accent="violet"
        />
      </div>

      {/* Secondary metadata */}
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        {detail.created && (
          <SimpleTooltip content={detail.created}>
            <span className="flex items-center gap-1">
              <Calendar className="size-3" />
              Created {detail.created}
            </span>
          </SimpleTooltip>
        )}
        {detail.md5 && (
          <>
            <span>&middot;</span>
            <SimpleTooltip content={detail.md5}>
              <button
                className="group/md5 flex items-center gap-1 hover:text-accent-sky transition-colors duration-150"
                aria-label={`Copy MD5: ${detail.md5}`}
                onClick={async () => {
                  try {
                    await navigator.clipboard.writeText(detail.md5);
                    if (md5TimerRef.current) clearTimeout(md5TimerRef.current);
                    setMd5Copied(true);
                    md5TimerRef.current = setTimeout(() => setMd5Copied(false), 2000);
                  } catch {
                    toast.error("Failed to copy to clipboard");
                  }
                }}
              >
                <span className="font-mono">{detail.md5.slice(0, 8)}&hellip;</span>
                <span
                  className={cn(
                    "transition-all duration-150",
                    md5Copied
                      ? "text-status-success"
                      : "text-muted-foreground/30 group-hover/md5:text-accent-sky"
                  )}
                >
                  {md5Copied ? <Check className="size-2.5" /> : <Copy className="size-2.5" />}
                </span>
              </button>
            </SimpleTooltip>
          </>
        )}
      </div>

      {/* Screenshots */}
      {detail.screenshots.length > 0 && (
        <div>
          <SectionHeader className="mb-2">Screenshots ({detail.screenshots.length})</SectionHeader>
          <div className="relative overflow-hidden rounded-xl border border-structure-06 bg-structure-02 group/screenshot">
            <img
              src={detail.screenshots[safeIdx]}
              alt={`Screenshot ${safeIdx + 1}`}
              className="w-full max-h-[300px] object-contain"
              loading="lazy"
              decoding="async"
            />
            {detail.screenshots.length > 1 && (
              <>
                {/* Navigation arrows */}
                <button
                  className="absolute left-2 top-1/2 -translate-y-1/2 size-8 rounded-full bg-scrim-50 backdrop-blur-sm flex items-center justify-center opacity-40 group-hover/screenshot:opacity-100 transition-opacity hover:bg-scrim-70"
                  onClick={() =>
                    setScreenshotIdx((prev) =>
                      prev > 0 ? prev - 1 : detail.screenshots.length - 1
                    )
                  }
                  aria-label="Previous screenshot"
                >
                  <ChevronLeft className="size-4" />
                </button>
                <button
                  className="absolute right-2 top-1/2 -translate-y-1/2 size-8 rounded-full bg-scrim-50 backdrop-blur-sm flex items-center justify-center opacity-40 group-hover/screenshot:opacity-100 transition-opacity hover:bg-scrim-70"
                  onClick={() =>
                    setScreenshotIdx((prev) =>
                      prev < detail.screenshots.length - 1 ? prev + 1 : 0
                    )
                  }
                  aria-label="Next screenshot"
                >
                  <ChevronRight className="size-4" />
                </button>

                {/* Dot indicators */}
                <div className="absolute bottom-2 left-1/2 -translate-x-1/2 flex gap-1.5 bg-scrim-40 backdrop-blur-sm rounded-full px-2 py-1">
                  {detail.screenshots.map((_, i) => (
                    <button
                      key={i}
                      className={cn(
                        "size-2 rounded-full transition-all duration-200",
                        i === safeIdx
                          ? "bg-primary scale-110"
                          : "bg-structure-30 hover:bg-structure-50"
                      )}
                      onClick={() => setScreenshotIdx(i)}
                      aria-label={`Screenshot ${i + 1}`}
                    />
                  ))}
                </div>

                {/* Counter */}
                <div className="absolute top-2 right-2 bg-scrim-50 backdrop-blur-sm rounded-md px-2 py-0.5 text-[11px] text-foreground">
                  {safeIdx + 1} / {detail.screenshots.length}
                </div>
              </>
            )}
          </div>

          {/* Thumbnail strip */}
          {detail.screenshots.length > 1 && (
            <div className="mt-2 flex gap-2 overflow-x-auto pb-1 [scrollbar-width:thin]">
              {detail.screenshots.map((src, i) => (
                <button
                  key={src}
                  onClick={() => setScreenshotIdx(i)}
                  className={cn(
                    "shrink-0 overflow-hidden rounded-lg border-2 transition-all duration-200",
                    i === safeIdx
                      ? "border-primary shadow-[0_0_8px_color-mix(in_oklab,var(--primary)_30%,transparent)]"
                      : "border-structure-06 hover:border-structure-15 opacity-60 hover:opacity-100"
                  )}
                >
                  <img
                    src={src}
                    alt={`Thumb ${i + 1}`}
                    className="h-14 w-24 object-cover"
                    loading="lazy"
                    decoding="async"
                  />
                </button>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Description */}
      {showDescription && detail.description && (
        <div>
          <SectionHeader className="mb-2">Description</SectionHeader>
          <RichDescription text={detail.description} />
        </div>
      )}

      {/* Changelog */}
      {detail.changeLog !== "" && (
        <div>
          <SectionHeader className="mb-2">
            {versionCount > 0 ? `Changelog (${versionCount})` : "Changelog"}
          </SectionHeader>
          <ChangelogView
            changeLog={detail.changeLog}
            variant="inline"
            installedVersion={installedVersion}
            archivedVersions={detail.archivedVersions}
            latestDate={detail.updated}
          />
        </div>
      )}
    </div>
  );
}

function StatCard({
  icon,
  label,
  value,
  accent,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  accent: "sky" | "emerald" | "gold" | "violet";
}) {
  const borderColors = {
    sky: "border-accent-sky/10 hover:border-accent-sky/20",
    emerald: "border-status-success/10 hover:border-status-success/20",
    gold: "border-primary/10 hover:border-primary/20",
    violet: "border-status-library/10 hover:border-status-library/20",
  };

  return (
    <SimpleTooltip content={value || ""}>
      <div
        className={cn(
          "rounded-xl border bg-structure-02 p-2.5 transition-colors",
          borderColors[accent]
        )}
      >
        <div className="flex items-center gap-1.5 mb-1">
          {icon}
          <span className="text-[11px] font-heading font-bold uppercase tracking-[0.05em] text-muted-foreground">
            {label}
          </span>
        </div>
        <div className="text-sm font-semibold truncate">{value || "\u2014"}</div>
      </div>
    </SimpleTooltip>
  );
}
