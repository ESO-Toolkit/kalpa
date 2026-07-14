// The auto-detected ESO Logs folder picker: a folder-identity header, search /
// filter / sort controls (shown once the folder is busy enough), the scrollable
// log list with per-file actions (reveal, copy path, delete), and drag-drop
// affordances. Memoized so typing the report name elsewhere in the workspace
// doesn't re-render it. Extracted from uploader-workspace.tsx unchanged.

import { memo, useMemo, useState } from "react";
import {
  AlertTriangle,
  ArrowDownUp,
  ClipboardCopy,
  CloudUpload,
  FileText,
  FolderInput,
  FolderOpen,
  FolderSearch,
  Radio,
  RefreshCw,
  RotateCcw,
  Search,
  Trash2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { InfoPill } from "@/components/ui/info-pill";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { LogFileInfo, LogPathDetection } from "@/types/uploader";
import { compactBytes, relativeFromMs } from "./uploader-shared";

type LogFilter = "all" | "active" | "archives";
type LogSort = "newest" | "largest";

export const LogPicker = memo(function LogPicker({
  detection,
  logsDir,
  logs,
  listError,
  selectedLog,
  scanning,
  dragOver,
  importing,
  onSelect,
  onRefresh,
  onPickFolder,
  onResetFolder,
  onOpenFolder,
  onReveal,
  onCopyPath,
  onRequestDelete,
}: {
  detection: LogPathDetection | null;
  logsDir: string | null;
  logs: LogFileInfo[];
  listError: string | null;
  selectedLog: string | null;
  scanning: boolean;
  dragOver: boolean;
  importing: boolean;
  onSelect: (path: string) => void;
  onRefresh: () => void;
  onPickFolder: () => void;
  onResetFolder: () => void;
  onOpenFolder: () => void;
  onReveal: (path: string) => void;
  onCopyPath: (path: string) => void;
  onRequestDelete: (log: LogFileInfo) => void;
}) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<LogFilter>("all");
  const [sort, setSort] = useState<LogSort>("newest");

  // Only show the controls once the folder has enough logs to be worth filtering.
  const showControls = logs.length > 1;
  const totalBytes = logs.reduce((sum, l) => sum + l.sizeBytes, 0);

  // The user has navigated away from the auto-detected folder iff a detected path
  // exists AND differs from the current one. Gates BOTH the line-1 "Custom" tag
  // and the line-2 "Back to detected" reset so they appear/disappear as a pair.
  const detectedPath = detection?.path ?? null;
  const isCustomFolder = detectedPath != null && logsDir != null && logsDir !== detectedPath;

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    let out = logs.filter((l) => {
      if (q && !l.fileName.toLowerCase().includes(q)) return false;
      if (filter === "active") return l.isActive;
      if (filter === "archives") return /^archive/i.test(l.fileName);
      return true;
    });
    out = [...out].sort((a, b) =>
      sort === "largest" ? b.sizeBytes - a.sizeBytes : b.modifiedAtMs - a.modifiedAtMs
    );
    return out;
  }, [logs, query, filter, sort]);

  return (
    // The picker is the primary work surface, so it RISES off the dark canvas:
    // a lighter fill, a luminous top edge (inset highlight), and a real outer
    // shadow. This is the one place the eye should land first.
    <div
      className={cn(
        "relative rounded-2xl border border-white/[0.1] bg-gradient-to-b from-white/[0.07] to-white/[0.025] p-3.5 transition-colors duration-150",
        "shadow-[0_12px_40px_-16px_rgba(0,0,0,0.7),inset_0_1px_0_rgba(255,255,255,0.08)]",
        dragOver && "border-accent-sky/60 from-accent-sky/[0.1] to-accent-sky/[0.03]"
      )}
    >
      {/* Drag-over overlay: a clear drop target appears while a file is dragged
          over the window. The actual import (copy-into-Logs) runs on drop. */}
      {dragOver && (
        <div className="pointer-events-none absolute inset-1 z-10 flex flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed border-accent-sky/50 bg-surface-overlay text-center">
          <CloudUpload className="size-7 text-accent-sky" aria-hidden />
          <span className="text-sm font-medium text-accent-sky">Drop your .log to add it</span>
        </div>
      )}
      {importing && (
        <div className="pointer-events-none absolute inset-1 z-10 flex flex-col items-center justify-center gap-2 rounded-lg bg-surface-overlay text-center">
          <span className="size-5 animate-spin rounded-full border-2 border-white/[0.1] border-t-accent-sky" />
          <span className="text-sm text-muted-foreground">Adding log to your folder…</span>
        </div>
      )}
      {/* Folder identity row: a sky folder-icon chip + the source path make it
          unmistakable that this is your on-disk Logs folder, not a generic
          panel. The path reads as a path; the count anchors "what's in here". */}
      <div className="mb-2.5 flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2.5">
          <span className="flex size-9 shrink-0 items-center justify-center rounded-lg border border-accent-sky/20 bg-accent-sky/[0.08] text-accent-sky">
            <FolderOpen className="size-4" aria-hidden />
          </span>
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold text-foreground/90">Logs folder</span>
              {/* A neutral state tag (NOT sky — it's informational, not a control)
                  marking that this isn't the auto-detected folder. The default
                  state stays badge-free, so absence reads as "the folder Kalpa
                  found". */}
              {isCustomFolder && (
                <span className="inline-flex items-center gap-1 rounded-md bg-white/[0.06] px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                  <FolderSearch className="size-2.5" aria-hidden />
                  Custom
                </span>
              )}
              {logsDir && (
                <span className="rounded-md bg-white/[0.06] px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground tabular-nums">
                  {logs.length} {logs.length === 1 ? "file" : "files"}
                  {totalBytes > 0 && ` · ${compactBytes(totalBytes)}`}
                </span>
              )}
              {totalBytes > 5 * 1024 * 1024 * 1024 && (
                <InfoPill color="amber" className="text-[10px]">
                  Folder large — delete old archives
                </InfoPill>
              )}
            </div>
            {logsDir ? (
              <div className="flex items-center gap-2">
                <span
                  className="min-w-0 truncate font-mono text-[11px] text-muted-foreground"
                  title={logsDir}
                >
                  {logsDir}
                </span>
                {/* One-tap, non-destructive return to the detected folder — sky
                    (interactive recovery), gated on the same isCustomFolder flag. */}
                {isCustomFolder && (
                  <SimpleTooltip content={`Detected: ${detectedPath}`} side="bottom">
                    <button
                      type="button"
                      onClick={onResetFolder}
                      className="inline-flex shrink-0 items-center gap-1 rounded-md border border-accent-sky/30 bg-accent-sky/[0.06] px-1.5 py-0.5 text-[11px] font-medium text-accent-sky transition-colors duration-150 hover:bg-accent-sky/[0.12] focus-visible:ring-2 focus-visible:ring-accent-sky/30 focus-visible:outline-none animate-[fade-in_0.2s_ease-out]"
                      aria-label="Switch back to the auto-detected Logs folder"
                    >
                      <RotateCcw className="size-3" aria-hidden />
                      Back to detected
                    </button>
                  </SimpleTooltip>
                )}
              </div>
            ) : (
              <div className="text-[11px] text-amber-400/90">{detection?.message}</div>
            )}
          </div>
        </div>
        <div className="flex shrink-0 gap-1">
          <SimpleTooltip content="Refresh logs" side="bottom">
            <Button variant="ghost" size="icon-sm" onClick={onRefresh} aria-label="Refresh logs">
              <RefreshCw className="size-3.5" />
            </Button>
          </SimpleTooltip>
          <SimpleTooltip content="Open Logs folder in File Explorer" side="bottom">
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={onOpenFolder}
              aria-label="Open Logs folder"
            >
              <FolderOpen className="size-3.5" />
            </Button>
          </SimpleTooltip>
          <SimpleTooltip content="Choose a different Logs folder" side="bottom">
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={onPickFolder}
              aria-label="Choose folder"
            >
              <FolderSearch className="size-3.5" />
            </Button>
          </SimpleTooltip>
        </div>
      </div>

      {/* Search + filter + sort, shown only when the folder is busy enough. */}
      {showControls && !listError && (
        <div className="mb-2 space-y-2">
          <div className="relative">
            <Search
              className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground/60"
              aria-hidden
            />
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search logs…"
              aria-label="Search logs"
              className="h-8 pl-8 text-xs"
            />
          </div>
          <div className="flex items-center justify-between gap-2">
            <div className="flex gap-1" role="group" aria-label="Filter logs">
              {(
                [
                  { id: "all", label: "All" },
                  { id: "active", label: "Active" },
                  { id: "archives", label: "Archives" },
                ] as { id: LogFilter; label: string }[]
              ).map((f) => (
                <button
                  key={f.id}
                  type="button"
                  aria-pressed={filter === f.id}
                  onClick={() => setFilter(f.id)}
                  className={cn(
                    "rounded-md border px-2 py-0.5 text-[11px] font-medium transition-colors",
                    "focus-visible:ring-2 focus-visible:ring-accent-sky/30 focus-visible:outline-none",
                    filter === f.id
                      ? "border-accent-sky/40 bg-accent-sky/[0.06] text-accent-sky"
                      : "border-white/[0.08] bg-white/[0.02] text-muted-foreground hover:text-foreground/80"
                  )}
                >
                  {f.label}
                </button>
              ))}
            </div>
            <button
              type="button"
              onClick={() => setSort((s) => (s === "newest" ? "largest" : "newest"))}
              className="inline-flex items-center gap-1 rounded-md border border-white/[0.08] bg-white/[0.02] px-2 py-0.5 text-[11px] font-medium text-muted-foreground transition-colors hover:text-foreground/80 focus-visible:ring-2 focus-visible:ring-accent-sky/30 focus-visible:outline-none"
              aria-label={`Sorted by ${sort === "newest" ? "newest" : "largest"} first — tap to sort by ${sort === "newest" ? "largest" : "newest"}`}
            >
              <ArrowDownUp className="size-3" aria-hidden />
              {sort === "newest" ? "Newest" : "Largest"}
            </button>
          </div>
        </div>
      )}

      {listError ? (
        // On-brand error card: 3px red left-accent, icon + headline + raw detail,
        // so a folder-access failure reads as an intentional state, not a glitch.
        <div className="rounded-lg border border-red-500/15 border-l-[3px] border-l-red-500 bg-red-500/[0.04] p-3">
          <div className="flex items-center gap-2 text-sm font-medium text-red-300/90">
            <AlertTriangle className="size-4 shrink-0" aria-hidden />
            Couldn't read this folder
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            Check it's accessible and try Refresh.
          </p>
          <p className="mt-1 text-xs break-words text-muted-foreground/70">{listError}</p>
        </div>
      ) : logs.length === 0 ? (
        // Unified empty state matching the FightList dashed pattern.
        <div className="flex flex-col items-center gap-2 rounded-lg border border-dashed border-white/[0.08] p-5 text-center">
          <FileText className="size-6 text-muted-foreground/40" aria-hidden />
          <p className="text-sm text-muted-foreground">
            {detection && !detection.encounterLogExists
              ? "No Encounter.log yet. Type /encounterlog in chat (or use a logging addon) to start recording."
              : "No log files found in this folder."}
          </p>
        </div>
      ) : (
        <ul
          className="max-h-52 space-y-1 overflow-y-auto rounded-xl border border-black/40 bg-black/25 p-1.5 shadow-[inset_0_2px_8px_-2px_rgba(0,0,0,0.6)]"
          aria-label="Log files"
          // Lightweight roving navigation: Up/Down/Home/End move focus between
          // log rows so a long folder isn't N Tab presses. Tab still works as a
          // fallback; we deliberately keep the aria-pressed button model (not a
          // listbox) for consistency with the rest of the uploader's selectors.
          onKeyDown={(e) => {
            const keys = ["ArrowDown", "ArrowUp", "Home", "End"];
            if (!keys.includes(e.key)) return;
            const buttons = Array.from(
              e.currentTarget.querySelectorAll<HTMLButtonElement>("button")
            );
            if (buttons.length === 0) return;
            const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
            e.preventDefault();
            let next: number;
            if (e.key === "Home") next = 0;
            else if (e.key === "End") next = buttons.length - 1;
            else if (e.key === "ArrowDown") next = current < 0 ? 0 : (current + 1) % buttons.length;
            else next = current <= 0 ? buttons.length - 1 : current - 1;
            buttons[next]?.focus();
          }}
        >
          {visible.length === 0 ? (
            <li className="rounded-lg border border-dashed border-white/[0.08] px-3 py-4 text-center text-xs text-muted-foreground">
              No logs match — clear the search or filter.
            </li>
          ) : null}
          {visible.map((log) => {
            const isSelected = selectedLog === log.path;
            return (
              // The row is a group container so the per-file actions can sit as
              // SIBLINGS of the select button (buttons can't nest in buttons) and
              // reveal on hover / keyboard focus-within.
              <li key={log.path} className="group/row relative">
                <button
                  type="button"
                  data-log-path={log.path}
                  onClick={() => onSelect(log.path)}
                  className={cn(
                    "flex w-full items-center justify-between gap-3 rounded-lg border py-2 pr-24 pl-3 text-left transition-all duration-150",
                    "focus-visible:border-accent-sky/40 focus-visible:ring-2 focus-visible:ring-accent-sky/30 focus-visible:outline-none",
                    isSelected
                      ? // Selected row pops OFF the recessed list: lit sky fill, a
                        // left accent bar, and a glow so the current choice is loud.
                        "border-accent-sky/50 border-l-[3px] border-l-accent-sky bg-accent-sky/[0.12] shadow-[0_2px_12px_-2px_color-mix(in_oklab,var(--accent-sky)_35%,transparent)]"
                      : "border-transparent bg-white/[0.03] hover:bg-white/[0.06]"
                  )}
                  aria-pressed={isSelected}
                >
                  <div className="flex min-w-0 items-center gap-2">
                    <FileText
                      className={cn(
                        "size-4 shrink-0",
                        isSelected ? "text-accent-sky" : "text-muted-foreground"
                      )}
                      aria-hidden
                    />
                    <div className="min-w-0">
                      <div className="truncate text-sm text-foreground/90">{log.fileName}</div>
                      <div className="text-xs text-muted-foreground">
                        {compactBytes(log.sizeBytes)} · {relativeFromMs(log.modifiedAtMs)}
                      </div>
                    </div>
                  </div>
                  {/* Scanning / active status, co-located with its row. Hidden
                      when the action cluster is showing so they don't overlap. */}
                  {isSelected && scanning ? (
                    <InfoPill color="sky" className="shrink-0 gap-1">
                      <span className="size-2.5 animate-spin rounded-full border-2 border-accent-sky/30 border-t-accent-sky" />
                      Scanning
                    </InfoPill>
                  ) : (
                    log.isActive && (
                      <InfoPill
                        color="sky"
                        className="shrink-0 gap-1 transition-opacity group-hover/row:opacity-0 group-focus-within/row:opacity-0"
                      >
                        <Radio className="size-3 animate-pulse" aria-hidden /> Active
                      </InfoPill>
                    )
                  )}
                </button>

                {/* Per-file actions — reveal, copy path, delete. Sit over the row's
                    right edge; appear on hover/focus, always present for keyboard.
                    stopPropagation so they never trigger row selection. */}
                <div className="absolute top-1/2 right-2 flex -translate-y-1/2 items-center gap-0.5 opacity-0 transition-opacity group-hover/row:opacity-100 group-focus-within/row:opacity-100">
                  <SimpleTooltip content="Reveal in Explorer" side="top">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="size-7 text-muted-foreground/70 hover:text-foreground"
                      onClick={(e) => {
                        e.stopPropagation();
                        onReveal(log.path);
                      }}
                      aria-label={`Reveal ${log.fileName} in Explorer`}
                    >
                      <FolderInput className="size-3.5" />
                    </Button>
                  </SimpleTooltip>
                  <SimpleTooltip content="Copy file path" side="top">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="size-7 text-muted-foreground/70 hover:text-foreground"
                      onClick={(e) => {
                        e.stopPropagation();
                        onCopyPath(log.path);
                      }}
                      aria-label={`Copy path of ${log.fileName}`}
                    >
                      <ClipboardCopy className="size-3.5" />
                    </Button>
                  </SimpleTooltip>
                  <SimpleTooltip
                    content={
                      log.isActive ? "Can't delete — this log is still being written" : "Delete log"
                    }
                    side="top"
                  >
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="size-7 text-muted-foreground/70 hover:text-red-400"
                      disabled={log.isActive}
                      onClick={(e) => {
                        e.stopPropagation();
                        if (!log.isActive) onRequestDelete(log);
                      }}
                      aria-label={
                        log.isActive
                          ? `${log.fileName} is active and can't be deleted`
                          : `Delete ${log.fileName}`
                      }
                    >
                      <Trash2 className="size-3.5" />
                    </Button>
                  </SimpleTooltip>
                </div>
              </li>
            );
          })}
        </ul>
      )}

      {/* Discoverability for drag-drop — a quiet hint that you can drop a log
          from anywhere; the backend copies it into this folder first. */}
      {!listError && (
        <p className="mt-2 text-center text-[11px] text-muted-foreground/60">
          or drop a <code className="text-muted-foreground/80">.log</code> file here from anywhere
        </p>
      )}
    </div>
  );
});
