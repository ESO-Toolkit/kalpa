import { useMemo, useState } from "react";
import { ChevronRight } from "lucide-react";
import {
  parseChangelog,
  matchInstalledEntry,
  buildVersionDateIndex,
  dateForEntry,
  type ChangelogEntry,
} from "@/lib/changelog";
import type { ArchivedVersion } from "@/types";
import { RichText } from "@/components/ui/rich-description";
import { Collapsible, CollapsibleTrigger, CollapsiblePanel } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";

/**
 * Split an author's verbatim header into the version token and whatever
 * annotation trails it (a date, a maintainer's name, a platform note).
 *
 * The parser deliberately keeps headers byte-exact so its partition stays
 * lossless, which means the author's decoration — `## 3.16.12`, `version 1.7.8:`,
 * `• Version 1.0.18 (2021/03/08)` — arrives here intact. Prettifying is a render
 * concern, so it happens here and only here.
 */
function splitHeader(header: string): { version: string; annotation: string | null } {
  const stripped = header.replace(/^[\s#=*_~>•\-–—]+/, "").trim();
  if (!stripped) return { version: header.trim(), annotation: null };

  // Only split on an EXPLICIT delimiter. Authors legitimately write one header
  // covering several versions ("v104, v105", "v100, 101, 103"), so anything
  // that greedily takes the first token as "the version" shreds those into a
  // nonsense annotation like ", 103".
  const paren = /^(.*?)\s*[([]([^)\]]+)[)\]]\s*$/.exec(stripped);
  if (paren && (paren[1] ?? "").trim()) {
    return { version: (paren[1] ?? "").trim(), annotation: (paren[2] ?? "").trim() || null };
  }

  const delimited = /^(.*?)\s*(?::|\s-\s|\s–\s|\s—\s)\s*(.+)$/.exec(stripped);
  if (delimited && (delimited[1] ?? "").trim()) {
    return {
      version: (delimited[1] ?? "").trim(),
      annotation: (delimited[2] ?? "").trim() || null,
    };
  }

  // No delimiter: the whole line is the version label. Trailing punctuation the
  // author used purely as a separator still comes off.
  return { version: stripped.replace(/[:\s]+$/, ""), annotation: null };
}

/**
 * ESOUI archive dates arrive as `04/23/26 01:16 PM`. The time is noise in a
 * dense version list, so only the date is shown.
 */
function dateOnly(value: string): string {
  return value.split(" ")[0] ?? value;
}

/** A micro-label in the app's uppercase caption style. */
function Tag({ children, tone }: { children: string; tone: "primary" | "muted" }) {
  return (
    <span
      className={cn(
        "font-heading text-[10px] font-medium tracking-[0.05em] uppercase",
        tone === "primary" ? "text-primary" : "text-muted-foreground"
      )}
    >
      {children}
    </span>
  );
}

interface EntryRowProps {
  entry: ChangelogEntry;
  defaultOpen: boolean;
  isLatest: boolean;
  isInstalled: boolean;
  /** Release date, when ESOUI knows one. Blank rather than guessed. */
  date?: string;
}

function EntryRow({ entry, defaultOpen, isLatest, isInstalled, date }: EntryRowProps) {
  const { version, annotation } = useMemo(() => splitHeader(entry.header), [entry.header]);
  const hasBody = entry.body.trim().length > 0;

  return (
    <Collapsible
      defaultOpen={defaultOpen}
      // The installed version gets a quiet left edge rather than a badge —
      // it echoes the addon list's status-border language without shouting.
      className={cn("group/entry", isInstalled && "border-l-2 border-l-primary/40 -ml-px pl-2")}
    >
      <CollapsibleTrigger
        disabled={!hasBody}
        className={cn(
          "flex w-full items-center gap-2 px-2 py-2 text-left",
          hasBody && "hover:bg-structure-02",
          !hasBody && "cursor-default"
        )}
      >
        <ChevronRight
          aria-hidden="true"
          className={cn(
            "size-3.5 shrink-0 text-muted-foreground transition-transform duration-150 motion-reduce:transition-none",
            "group-data-open/entry:rotate-90",
            !hasBody && "opacity-0"
          )}
        />
        <span className="font-heading text-[13px] font-semibold tabular-nums text-foreground">
          {version}
        </span>
        {isLatest && <Tag tone="primary">Latest</Tag>}
        {isInstalled && <Tag tone="muted">Installed</Tag>}
        {(annotation ?? date) && (
          <span className="ml-auto truncate pl-3 text-[11px] tabular-nums text-muted-foreground">
            {annotation ?? (date ? dateOnly(date) : null)}
          </span>
        )}
      </CollapsibleTrigger>
      {hasBody && (
        <CollapsiblePanel>
          <div className="px-2 pt-0.5 pb-3 pl-[1.625rem]">
            <RichText text={entry.body} className="space-y-2 text-sm" />
          </div>
        </CollapsiblePanel>
      )}
    </Collapsible>
  );
}

/** The shared panel chrome, matching RichDescription's paper gradient. */
function Panel({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        "rounded-xl border border-structure-06 bg-gradient-to-b from-structure-03 to-structure-01 text-sm text-foreground shadow-[inset_0_1px_0_var(--structure-04)]",
        className
      )}
    >
      {children}
    </div>
  );
}

/**
 * An unparseable changelog: the plain dump, clamped so it cannot dominate the
 * pane, with the overflow faded out by a mask rather than a solid gradient
 * (a colour stop would have to guess the panel's gradient and would break on
 * the light and high-contrast themes).
 */
function UnparsedBody({ text, clampHeight }: { text: string; clampHeight: number }) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div>
      <div
        className="overflow-hidden px-4 pt-4"
        style={
          expanded
            ? undefined
            : {
                maxHeight: `${clampHeight}px`,
                maskImage: "linear-gradient(to bottom, black calc(100% - 48px), transparent)",
                WebkitMaskImage: "linear-gradient(to bottom, black calc(100% - 48px), transparent)",
              }
        }
      >
        <RichText text={text} />
      </div>
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        aria-expanded={expanded}
        className="flex w-full items-center justify-center gap-1.5 border-t border-structure-06 px-4 py-2.5 font-heading text-xs text-muted-foreground transition-colors duration-150 hover:bg-structure-02 hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-accent-sky/20 focus-visible:outline-none motion-reduce:transition-none"
      >
        {expanded ? "Show less" : "Show more"}
      </button>
    </div>
  );
}

export interface ChangelogViewProps {
  /** Cleaned changelog text from the ESOUI detail. */
  changeLog: string;
  variant: "inline" | "dialog";
  /** Installed version — highlights a matching entry. Omitted in Discover. */
  installedVersion?: string;
  /** Archived release dates from ESOUI, used to date past entries. */
  archivedVersions?: ArchivedVersion[];
  /** Date of the newest release (the API's lastUpdate); the archive omits it. */
  latestDate?: string;
  /** How many entries to list before the "show all" affordance. */
  initialVisible?: number;
  className?: string;
}

/**
 * Renders an ESOUI changelog as a scannable version list.
 *
 * Authors write changelogs freeform, so `parseChangelog` splits them on line
 * structure and returns a typed result; when it cannot find a reliable
 * structure this falls back to the clamped plain dump rather than risk
 * presenting a mis-split history as though it were authoritative.
 *
 * Entries are never hidden based on the installed version — only marked. See
 * the warning on `matchInstalledEntry`.
 */
export function ChangelogView({
  changeLog,
  variant,
  installedVersion,
  archivedVersions,
  latestDate,
  initialVisible,
  className,
}: ChangelogViewProps) {
  const parsed = useMemo(() => parseChangelog(changeLog), [changeLog]);
  const visibleCount = initialVisible ?? (variant === "dialog" ? 10 : 5);
  const [showAll, setShowAll] = useState(false);
  const dateIndex = useMemo(
    () => buildVersionDateIndex(archivedVersions ?? []),
    [archivedVersions]
  );

  const installedIdx = useMemo(
    () => (parsed.kind === "parsed" ? matchInstalledEntry(parsed.entries, installedVersion) : -1),
    [parsed, installedVersion]
  );

  if (parsed.kind === "empty") return null;

  if (parsed.kind === "unparsed") {
    return (
      <Panel className={className}>
        <UnparsedBody text={parsed.text} clampHeight={variant === "dialog" ? 420 : 280} />
      </Panel>
    );
  }

  const { entries, preamble } = parsed;
  const shown = showAll ? entries : entries.slice(0, visibleCount);
  const hidden = entries.length - shown.length;

  // The entries between the newest and the installed one are what an update
  // would bring. When that delta is small, open all of it: in the update flow
  // this is precisely the question the user is asking.
  const delta = installedIdx > 0 ? installedIdx : 0;
  const expandDelta = variant === "dialog" && delta > 0 && delta <= visibleCount;

  return (
    <Panel className={className}>
      <section aria-label="Changelog">
        {preamble && (
          <div className="border-b border-structure-06 px-4 py-3 text-[13px] text-muted-foreground">
            <RichText text={preamble} className="space-y-2" />
          </div>
        )}
        <div className="px-2 py-1">
          {shown.map((entry, idx) => (
            <div
              key={`${entry.header}-${idx}`}
              className={idx > 0 ? "border-t border-structure-06" : undefined}
            >
              <EntryRow
                entry={entry}
                defaultOpen={idx === 0 || (expandDelta && idx < delta)}
                isLatest={idx === 0}
                isInstalled={idx === installedIdx}
                // The newest release is never in the archive table, so it takes
                // the API's lastUpdate instead.
                date={
                  idx === 0
                    ? (dateForEntry(entry.header, dateIndex) ?? latestDate)
                    : dateForEntry(entry.header, dateIndex)
                }
              />
            </div>
          ))}
        </div>
        {hidden > 0 && (
          <button
            type="button"
            onClick={() => setShowAll(true)}
            className="flex w-full items-center justify-center gap-1.5 border-t border-structure-06 px-4 py-2.5 font-heading text-xs text-muted-foreground transition-colors duration-150 hover:bg-structure-02 hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-accent-sky/20 focus-visible:outline-none motion-reduce:transition-none"
          >
            Show all {entries.length} versions
          </button>
        )}
      </section>
    </Panel>
  );
}

/** The version count, for the section header. Returns 0 when unparsed/empty. */
export function changelogVersionCount(changeLog: string): number {
  const parsed = parseChangelog(changeLog);
  return parsed.kind === "parsed" ? parsed.entries.length : 0;
}

/** Whether a changelog has any content worth rendering a section for. */
export function hasChangelog(changeLog: string): boolean {
  return parseChangelog(changeLog).kind !== "empty";
}
