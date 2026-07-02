// The "Recent Uploads" history panel: the most recent upload records with a
// content-recognizable lead, quiet provenance, a status badge, report link
// actions, an inline "paste report link" affordance for handed-off uploads, and a
// two-step remove-from-history confirm. Memoized. Extracted from
// uploader-workspace.tsx unchanged; `tidyLogLabel` and `sourceLocation` are
// exported for unit tests.

import { memo, useMemo, useState } from "react";
import {
  Copy,
  ExternalLink,
  FileText,
  Link as LinkIcon,
  RefreshCw,
  Trash2,
  Zap,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { InfoPill } from "@/components/ui/info-pill";
import { SectionHeader } from "@/components/ui/section-header";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { UploadRecord } from "@/types/uploader";
import { parseReportCode, primaryReportUrl, relativeFromMs } from "./uploader-shared";
import { shortDate } from "./naming";
import { WORK_PANEL, openReportUrl } from "./uploader-actions";

/** A scannable label for an upload history row. ESO archive logs carry long,
 *  machine-generated names (`Archive-2025-06-20__03_46_03-Encounter-session02-
 *  1750394097660.log`); strip that noise to the part a human recognizes, while
 *  leaving ordinary names (and user-named splits) intact. */
export function tidyLogLabel(fileName: string): string {
  const base = fileName.replace(/\.log$/i, "");
  // Archive pattern with a session number → keep the readable "session NN".
  const sess = base.match(/-session(\d+)/i);
  if (/^Archive-/i.test(base) && sess) {
    const datePart = base.match(/Archive-(\d{4}-\d{2}-\d{2})/);
    return datePart
      ? `Archive ${datePart[1]} · session ${Number(sess[1])}`
      : `Session ${Number(sess[1])}`;
  }
  // ISO-stamped archive (Archive-20260614T190354Z-Encounter) → "Archive Jun 14".
  const iso = base.match(/^Archive-(\d{4})(\d{2})(\d{2})T\d+Z?/i);
  if (iso) {
    const [, y, m, d] = iso;
    const dt = new Date(Number(y), Number(m) - 1, Number(d));
    return `Archive · ${dt.toLocaleDateString(undefined, { month: "short", day: "numeric" })}`;
  }
  // Drop a trailing epoch-ms id some names carry.
  return base.replace(/-\d{13,}$/, "");
}

/** Split a stored source path into its immediate parent folder (for the row's
 *  provenance line) and the full directory (for the tooltip). Handles Windows
 *  (`\`) and POSIX (`/`) separators so the same record reads correctly on both. */
export function sourceLocation(sourcePath: string): { folder: string; dir: string } {
  const sepIdx = Math.max(sourcePath.lastIndexOf("/"), sourcePath.lastIndexOf("\\"));
  const dir = sepIdx > 0 ? sourcePath.slice(0, sepIdx) : sourcePath;
  const dirParts = dir.split(/[/\\]/).filter(Boolean);
  const folder = dirParts[dirParts.length - 1] ?? dir;
  return { folder, dir };
}

/** The 3px status-accent left-border color per upload status (the app's signature
 *  card idiom). Color encodes status REDUNDANTLY with the badge text — never color
 *  alone. Emerald for `live` (a healthy in-progress session); red is reserved for
 *  real failures only. */
const STATUS_ACCENT: Record<UploadRecord["status"], string> = {
  completed: "before:bg-emerald-400/70",
  queued: "before:bg-accent-sky/70",
  uploading: "before:bg-accent-sky/70",
  live: "before:bg-emerald-400/70",
  paused: "before:bg-amber-400/70",
  handedOff: "before:bg-amber-400/70",
  failed: "before:bg-red-400/80",
  cancelled: "before:bg-white/15",
};

export const HistoryPanel = memo(function HistoryPanel({
  history,
  onCopyLink,
  onRefresh,
  onAttachReport,
  onDelete,
}: {
  history: UploadRecord[];
  onCopyLink: (url: string) => void | Promise<void>;
  onRefresh: () => void;
  onAttachReport: (id: string, url: string) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
}) {
  const [attachingId, setAttachingId] = useState<string | null>(null);
  const [linkDraft, setLinkDraft] = useState("");
  // Inline two-step confirm: clicking trash arms the row; a second click on the
  // revealed "Remove" confirms. Removing a history record never touches the file.
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  // Only the first 8 records are rendered.
  const visibleHistory = useMemo(() => history.slice(0, 8), [history]);

  const submitLink = async (id: string) => {
    const code = parseReportCode(linkDraft);
    if (!code) {
      toast.error("That doesn't look like an ESO Logs report link or code.");
      return;
    }
    // Always rebuild the canonical URL from the parsed code, so neither a full URL
    // nor a bare code can forward anything but a well-formed report link.
    await onAttachReport(id, `https://www.esologs.com/reports/${code}`);
    setAttachingId(null);
    setLinkDraft("");
  };

  return (
    <div className={cn(WORK_PANEL, "p-3.5")}>
      <div className="mb-2.5 flex items-center justify-between">
        <SectionHeader>Recent Uploads</SectionHeader>
        <SimpleTooltip content="Refresh history" side="bottom">
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => void onRefresh()}
            aria-label="Refresh history"
          >
            <RefreshCw className="size-3.5" />
          </Button>
        </SimpleTooltip>
      </div>

      {history.length === 0 ? (
        <div className="px-1 py-6 text-center">
          <FileText className="mx-auto mb-2 size-5 text-muted-foreground/30" aria-hidden />
          <p className="text-xs text-muted-foreground/70">
            No uploads yet. Reports you create will appear here.
          </p>
        </div>
      ) : (
        <ul className="space-y-1.5">
          {visibleHistory.map((r) => {
            const loc = sourceLocation(r.sourcePath);
            // Lead with a content name the raider recognizes: the derived zone +
            // date, falling back to the report title, then a tidied file label.
            const date = shortDate(r.createdAtMs);
            const content = r.zone?.trim() ? `${r.zone.trim()}${date ? ` · ${date}` : ""}` : null;
            const title = r.title?.trim() || null;
            const lead = content ?? title ?? tidyLogLabel(r.fileName);
            // Show the report title as a secondary line only when it actually adds
            // something. Normalize separators + case so the COMMON suggested name
            // ("Zone — date") isn't echoed as a near-duplicate of the lead
            // ("Zone · date"), and a description that just repeats the zone is hidden.
            const norm = (s: string) =>
              s
                .toLowerCase()
                .replace(/[·—–-]+/g, " ")
                .replace(/\s+/g, " ")
                .trim();
            const showTitle =
              title !== null &&
              norm(title) !== norm(lead) &&
              (r.zone ? norm(title) !== norm(r.zone) : true);
            const handedOffNeedsLink = r.status === "handedOff" && !r.report;
            return (
              <li
                key={r.id}
                className={cn(
                  "relative overflow-hidden rounded-lg border border-white/[0.06] bg-white/[0.02] py-2 pr-3 pl-3.5 transition-colors hover:bg-white/[0.04]",
                  "before:absolute before:top-2 before:bottom-2 before:left-0 before:w-[3px] before:rounded-full before:content-['']",
                  STATUS_ACCENT[r.status]
                )}
              >
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0">
                    {/* Lead: the content name (zone · date), or the report title, or
                        a tidied file label — what the raider recognizes at a glance. */}
                    <div className="truncate text-sm font-semibold text-foreground/90">{lead}</div>
                    {/* The ESO Logs report title, only when distinct from the lead. */}
                    {showTitle && (
                      <div
                        className="truncate text-xs text-foreground/65"
                        title={title ?? undefined}
                      >
                        {title}
                      </div>
                    )}
                    {/* Quiet provenance: the exact file name (mono, so two
                        "Encounter.log" uploads stay distinguishable) + hard facts.
                        The full source folder lives on the file name's tooltip. */}
                    <div className="mt-0.5 flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
                      <SimpleTooltip content={loc.dir} side="top">
                        <span
                          className="truncate font-mono text-[11px] text-muted-foreground/70"
                          title={loc.dir}
                        >
                          {r.fileName}
                        </span>
                      </SimpleTooltip>
                      {/* The source folder is now visual-tooltip-only; expose it to
                          screen readers so two same-named logs stay distinguishable. */}
                      <span className="sr-only"> in {loc.dir}</span>
                      <span className="text-muted-foreground/40">·</span>
                      <span>
                        {r.fightCount} fight{r.fightCount === 1 ? "" : "s"}
                      </span>
                      <span className="text-muted-foreground/40">·</span>
                      <span className="capitalize">{r.visibility}</span>
                      <span className="text-muted-foreground/40">·</span>
                      <span>{relativeFromMs(r.createdAtMs)}</span>
                    </div>
                  </div>
                  <div className="flex shrink-0 items-center gap-1.5">
                    <StatusBadge status={r.status} hasReport={!!r.report} />
                    {r.report && (
                      <>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          onClick={() => void onCopyLink(r.report!.url)}
                          aria-label="Copy report link"
                        >
                          <Copy className="size-3.5" />
                        </Button>
                        <SimpleTooltip content="Open the raw report on ESO Logs" side="top">
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={() => void openReportUrl(r.report!.url)}
                            aria-label="Open report on ESO Logs"
                          >
                            <ExternalLink className="size-3.5" />
                          </Button>
                        </SimpleTooltip>
                        {/* The richer analysis (fight detection, rotation, scribing,
                            replay) lives in the ESO Log Aggregator — the primary view. */}
                        <Button
                          variant="ghost"
                          size="sm"
                          className="text-emerald-300/90 hover:bg-emerald-500/15 hover:text-emerald-200"
                          onClick={() =>
                            void openReportUrl(primaryReportUrl(r.report!, r.visibility))
                          }
                          aria-label={
                            r.visibility === "private"
                              ? "Open private report on ESO Logs"
                              : "Open analysis in ESO Log Aggregator"
                          }
                        >
                          {r.visibility === "private" ? (
                            <ExternalLink className="size-3.5" />
                          ) : (
                            <Zap className="size-3.5" />
                          )}
                          {r.visibility === "private" ? "ESO Logs" : "Analysis"}
                        </Button>
                      </>
                    )}
                    <SimpleTooltip
                      content="Remove from history (your log file stays on disk)"
                      side="top"
                    >
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        className="text-muted-foreground/70 hover:text-red-400"
                        onClick={() => setConfirmDeleteId(confirmDeleteId === r.id ? null : r.id)}
                        aria-label="Remove this upload from history"
                      >
                        <Trash2 className="size-3.5" />
                      </Button>
                    </SimpleTooltip>
                  </div>
                </div>

                {/* Handed-off explainer + paste affordance, replacing the old
                    context-free "Add link". ALWAYS visible (no hover) so the state
                    is self-explanatory; clicking reveals the inline input in-place,
                    with the prose still showing so the "why" never disappears. */}
                {handedOffNeedsLink && (
                  <div className="mt-2 rounded-lg border border-amber-400/20 bg-amber-400/[0.05] px-3 py-2">
                    <div className="flex items-start gap-2">
                      <ExternalLink
                        className="mt-0.5 size-3.5 shrink-0 text-amber-400/80"
                        aria-hidden
                      />
                      <p className="text-xs leading-relaxed text-amber-100/80">
                        Finished in the official ESO Logs uploader, so Kalpa doesn't have the report
                        link yet. Paste it to open the analysis.
                      </p>
                    </div>
                    {attachingId === r.id ? (
                      <div className="mt-2 flex items-center gap-2">
                        <Input
                          value={linkDraft}
                          onChange={(e) => setLinkDraft(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") void submitLink(r.id);
                            if (e.key === "Escape") setAttachingId(null);
                          }}
                          placeholder="Paste esologs.com link or report code"
                          aria-label="ESO Logs report link or code"
                          autoFocus
                          className="h-8 flex-1 text-xs"
                        />
                        <Button
                          size="sm"
                          onClick={() => void submitLink(r.id)}
                          disabled={!linkDraft.trim()}
                        >
                          Attach
                        </Button>
                        <Button variant="ghost" size="sm" onClick={() => setAttachingId(null)}>
                          Cancel
                        </Button>
                      </div>
                    ) : (
                      <Button
                        variant="ghost"
                        size="sm"
                        className="mt-1.5 -ml-1.5 gap-1.5 text-amber-200/90 hover:bg-amber-400/10 hover:text-amber-100"
                        onClick={() => {
                          setAttachingId(r.id);
                          setLinkDraft("");
                        }}
                      >
                        <LinkIcon className="size-3.5" />
                        Paste report link
                      </Button>
                    )}
                  </div>
                )}

                {confirmDeleteId === r.id && (
                  <div className="mt-2 flex items-center justify-between gap-2 rounded-lg border border-red-500/20 bg-red-500/[0.05] px-3 py-2">
                    <span className="text-xs text-red-200/90">
                      Remove this record? Your log file stays on disk.
                    </span>
                    <div className="flex shrink-0 gap-1.5">
                      <Button variant="ghost" size="sm" onClick={() => setConfirmDeleteId(null)}>
                        Cancel
                      </Button>
                      <Button
                        variant="destructive"
                        size="sm"
                        onClick={() => {
                          setConfirmDeleteId(null);
                          void onDelete(r.id);
                        }}
                      >
                        Remove
                      </Button>
                    </div>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
});

function StatusBadge({
  status,
  hasReport,
}: {
  status: UploadRecord["status"];
  hasReport: boolean;
}) {
  switch (status) {
    case "completed":
      return <InfoPill color="emerald">Done</InfoPill>;
    case "uploading":
    case "queued":
      return <InfoPill color="sky">Uploading</InfoPill>;
    case "live":
      // Emerald + pulse — a healthy in-progress live session (red is reserved for
      // real errors). Matches the header / LiveDashboard live treatment.
      return (
        <InfoPill color="emerald" className="gap-1.5">
          <span className="relative flex size-2" aria-hidden>
            <span className="absolute inline-flex size-full animate-ping rounded-full bg-emerald-400/70" />
            <span className="relative inline-flex size-2 rounded-full bg-emerald-400" />
          </span>
          Live
        </InfoPill>
      );
    case "paused":
      return <InfoPill color="amber">Paused</InfoPill>;
    case "handedOff":
      // Once a link is attached, the report is observable → "Done". Until then it's
      // "Link needed" (amber), paired with the row's always-visible explainer strip
      // so the badge is never jargon standing alone.
      return hasReport ? (
        <InfoPill color="emerald">Done</InfoPill>
      ) : (
        <InfoPill color="amber" className="gap-1">
          <LinkIcon className="size-2.5" aria-hidden /> Link needed
        </InfoPill>
      );
    case "failed":
      return <InfoPill color="red">Failed</InfoPill>;
    case "cancelled":
      return <InfoPill color="muted">Cancelled</InfoPill>;
    default:
      return <InfoPill color="muted">{status}</InfoPill>;
  }
}
