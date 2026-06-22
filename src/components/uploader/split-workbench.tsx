// The Split Workbench — a focused modal for carving an oversized Encounter.log
// into per-session files, in depth. Each logging session becomes a card the user
// can include/exclude, name (with a smart suggestion + the fight breakdown that
// justifies the name), and preview before writing. The backend
// (`uploader_split_to_disk_named`) re-sanitizes every name, so this is purely the
// authoring surface; it never controls the destination.

import { useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import {
  Scissors,
  ChevronDown,
  Swords,
  Sparkles,
  FolderOpen,
  CheckCheck,
  Filter,
  X,
} from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { InfoPill } from "@/components/ui/info-pill";
import { cn } from "@/lib/utils";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import type { FightSummary, LogPreflight, LogSession, SplitSelection } from "@/types/uploader";
import { compactBytes, formatDuration, relativeFromMs } from "./uploader-shared";
import { RUN_TAGS, suggestSplitName, withTag } from "./naming";

// Remember the last prefix the user applied to a batch of splits (e.g. a guild
// or character tag), so a regular raid-night split keeps the same convention.
const SPLIT_PREFIX_KEY = "kalpa.uploader.splitPrefix";

/** The fights that fall inside a session's byte range, in file order. */
function fightsInSession(session: LogSession, fights: FightSummary[]): FightSummary[] {
  return fights.filter(
    (f) => f.startOffset >= session.startOffset && f.startOffset < session.endOffset
  );
}

/** Per-session editable state in the workbench. */
interface SessionDraft {
  include: boolean;
  name: string;
}

export function SplitWorkbench({
  open,
  onOpenChange,
  filePath,
  fileName,
  preflight,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  filePath: string;
  fileName: string;
  preflight: LogPreflight | null;
}) {
  const sessions = preflight?.sessions ?? [];
  const fights = preflight?.fights ?? [];

  // Build initial drafts: include sessions that have fights by default (the
  // useful ones), pre-fill each with a smart name suggestion.
  const initialDrafts = useMemo<Record<number, SessionDraft>>(() => {
    const out: Record<number, SessionDraft> = {};
    for (const s of sessions) {
      out[s.index] = {
        include: s.fightCount > 0,
        name: suggestSplitName(s, fightsInSession(s, fights)),
      };
    }
    return out;
    // Recompute only when the session set changes (by count + first index).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessions.length, sessions[0]?.index, fights.length]);

  const [drafts, setDrafts] = useState<Record<number, SessionDraft>>(initialDrafts);
  const [expanded, setExpanded] = useState<number | null>(null);
  const [splitting, setSplitting] = useState(false);
  // A common prefix applied to every included split name (e.g. a guild tag),
  // remembered across sessions so a recurring raid keeps its naming convention.
  const [prefix, setPrefix] = useState<string>(() => {
    try {
      return localStorage.getItem(SPLIT_PREFIX_KEY) ?? "";
    } catch {
      return "";
    }
  });
  // The prefix currently baked into the draft names (kebab form). Starts empty:
  // the initial suggested names carry no prefix even if one is remembered in the
  // input. Lets applyPrefix strip the prior prefix idempotently per keystroke.
  const appliedPrefixRef = useRef("");

  // Keep drafts in sync if the preflight changes underneath (new selection).
  const draftsKey = Object.keys(drafts).length;
  if (draftsKey === 0 && sessions.length > 0) {
    setDrafts(initialDrafts);
  }

  const draftFor = (prev: Record<number, SessionDraft>, index: number): SessionDraft =>
    prev[index] ?? { include: false, name: "" };

  const setDraft = (index: number, patch: Partial<SessionDraft>) =>
    setDrafts((prev) => ({ ...prev, [index]: { ...draftFor(prev, index), ...patch } }));

  const selected = sessions.filter((s) => drafts[s.index]?.include);
  const selectedBytes = selected.reduce((sum, s) => sum + s.sizeBytes, 0);

  const selectAll = () =>
    setDrafts((prev) => {
      const next = { ...prev };
      for (const s of sessions) next[s.index] = { ...draftFor(prev, s.index), include: true };
      return next;
    });
  const clearAll = () =>
    setDrafts((prev) => {
      const next = { ...prev };
      for (const s of sessions) next[s.index] = { ...draftFor(prev, s.index), include: false };
      return next;
    });
  const keepWithFights = () =>
    setDrafts((prev) => {
      const next = { ...prev };
      for (const s of sessions)
        next[s.index] = { ...draftFor(prev, s.index), include: s.fightCount > 0 };
      return next;
    });

  // Append a run-tag (prog/core/pug/…) to every INCLUDED split's name at once,
  // reusing the same idempotent withTag() the report-name field uses.
  const tagAllIncluded = (tag: (typeof RUN_TAGS)[number]["id"]) =>
    setDrafts((prev) => {
      const next = { ...prev };
      for (const s of sessions) {
        const d = draftFor(prev, s.index);
        if (d.include) next[s.index] = { ...d, name: withTag(d.name, tag) };
      }
      return next;
    });

  const kebab = (raw: string) =>
    raw
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "");

  // Apply (and remember) a common prefix on every included split, live as the
  // user types. Idempotent: each change STRIPS the previously-applied prefix
  // before adding the new one, so typing "core" yields "core-name" — not the
  // compounding "co-c-name" bug. `appliedPrefixRef` tracks the last applied value.
  const applyPrefix = (raw: string) => {
    const next = kebab(raw);
    const prev = appliedPrefixRef.current;
    setPrefix(raw);
    try {
      localStorage.setItem(SPLIT_PREFIX_KEY, raw);
    } catch {
      /* ignore */
    }
    if (next === prev) return;
    setDrafts((drafts) => {
      const out = { ...drafts };
      for (const s of sessions) {
        const d = draftFor(drafts, s.index);
        if (!d.include) continue;
        // Strip the prior prefix (if present), then add the new one.
        let stem = d.name;
        if (prev && stem.startsWith(`${prev}-`)) stem = stem.slice(prev.length + 1);
        out[s.index] = { ...d, name: next ? `${next}-${stem}` : stem };
      }
      return out;
    });
    appliedPrefixRef.current = next;
  };

  const handleSplit = async () => {
    if (selected.length === 0) return;
    setSplitting(true);
    try {
      const selections: SplitSelection[] = selected.map((s) => ({
        index: s.index,
        name: drafts[s.index]?.name?.trim() || null,
        // Pin the session identity so the backend can detect a rescan that shifted
        // indices (log truncated/rotated since preflight) and refuse to mislabel.
        startTimeMs: s.startTimeMs,
      }));
      const written = await invokeOrThrow<string[]>("uploader_split_to_disk_named", {
        filePath,
        sessions: preflight?.sessions ?? null,
        selections,
      });
      toast.success(`Split into ${written.length} file${written.length === 1 ? "" : "s"}.`, {
        duration: 6000,
      });
      try {
        const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
        if (written[0]) await revealItemInDir(written[0]);
      } catch {
        /* reveal is best-effort */
      }
      onOpenChange(false);
    } catch (e) {
      toast.error(`Split failed: ${getTauriErrorMessage(e)}`);
    } finally {
      setSplitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[88vh] flex-col gap-0 overflow-hidden sm:max-w-2xl">
        <DialogHeader className="shrink-0">
          <DialogTitle className="flex items-center gap-2">
            <Scissors className="size-5 text-primary" aria-hidden />
            Split workbench
          </DialogTitle>
          <DialogDescription>
            Carve <code className="text-foreground/80">{fileName}</code> into one file per logging
            session. Pick which to keep, name each, then split.
          </DialogDescription>
        </DialogHeader>

        {/* Preset toolbar */}
        <div className="mt-4 flex shrink-0 flex-wrap items-center gap-1.5">
          <Button variant="outline" size="sm" onClick={selectAll}>
            <CheckCheck className="size-3.5" />
            Select all
          </Button>
          <Button variant="outline" size="sm" onClick={keepWithFights}>
            <Filter className="size-3.5" />
            Only sessions with fights
          </Button>
          <Button variant="ghost" size="sm" onClick={clearAll}>
            <X className="size-3.5" />
            Clear
          </Button>
        </div>

        {/* Batch naming: a remembered prefix + one-tap run-tags applied to every
            included split, so a recurring raid keeps one convention. */}
        {selected.length > 0 && (
          <div className="mt-2 flex shrink-0 flex-wrap items-center gap-1.5 rounded-lg border border-white/[0.06] bg-white/[0.02] px-2 py-1.5">
            <span className="text-[11px] font-medium text-muted-foreground">Name all:</span>
            <Input
              value={prefix}
              onChange={(e) => applyPrefix(e.target.value)}
              placeholder="prefix (e.g. guild)"
              aria-label="Prefix for all split names"
              className="h-7 w-32 text-[11px]"
            />
            <span className="text-[11px] text-muted-foreground/50">+ tag:</span>
            {RUN_TAGS.map((t) => (
              <button
                key={t.id}
                type="button"
                title={`Add "${t.label}" to every selected split`}
                onClick={() => tagAllIncluded(t.id)}
                className="rounded-md border border-white/[0.08] bg-white/[0.02] px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground transition-colors hover:border-accent-sky/30 hover:text-accent-sky"
              >
                {t.label}
              </button>
            ))}
          </div>
        )}

        {/* Session cards. flex-1 + min-h-0 are load-bearing: without min-h-0 a
            flex child won't shrink below its content, so overflow-y-auto never
            engages and the footer scrolls off the capped-height dialog. */}
        <div className="-mr-2 mt-3 min-h-0 flex-1 space-y-2 overflow-y-auto pr-2">
          {sessions.length === 0 ? (
            <div className="rounded-lg border border-dashed border-white/[0.08] p-6 text-center text-sm text-muted-foreground">
              No logging sessions found in this file.
            </div>
          ) : (
            sessions.map((s) => {
              const d = drafts[s.index] ?? { include: false, name: "" };
              const inFights = fightsInSession(s, fights);
              const isOpen = expanded === s.index;
              return (
                <SessionCard
                  key={s.index}
                  session={s}
                  draft={d}
                  fights={inFights}
                  expanded={isOpen}
                  onToggleInclude={() => setDraft(s.index, { include: !d.include })}
                  onRename={(name) => setDraft(s.index, { name })}
                  onSuggest={() => setDraft(s.index, { name: suggestSplitName(s, inFights) })}
                  onToggleExpand={() => setExpanded(isOpen ? null : s.index)}
                />
              );
            })
          )}
        </div>

        {/* Live preview + action footer */}
        <div className="-mx-5 -mb-5 mt-4 flex shrink-0 items-center justify-between gap-3 border-t border-white/[0.06] bg-gradient-to-b from-white/[0.02] to-transparent p-4">
          <div className="text-sm">
            {selected.length === 0 ? (
              <span className="text-muted-foreground">Select at least one session to split.</span>
            ) : (
              <span className="text-foreground/80">
                <span className="font-semibold text-primary">{selected.length}</span> file
                {selected.length === 1 ? "" : "s"}{" "}
                <span className="text-muted-foreground">· ~{compactBytes(selectedBytes)}</span>
              </span>
            )}
          </div>
          <div className="flex items-center gap-2">
            <Button variant="ghost" size="sm" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button size="sm" onClick={handleSplit} disabled={selected.length === 0 || splitting}>
              <FolderOpen className="size-3.5" />
              {splitting ? "Splitting…" : "Split & reveal"}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function SessionCard({
  session,
  draft,
  fights,
  expanded,
  onToggleInclude,
  onRename,
  onSuggest,
  onToggleExpand,
}: {
  session: LogSession;
  draft: SessionDraft;
  fights: FightSummary[];
  expanded: boolean;
  onToggleInclude: () => void;
  onRename: (name: string) => void;
  onSuggest: () => void;
  onToggleExpand: () => void;
}) {
  const realm = session.realm?.replace(/megaserver/i, "").trim() || null;
  const suggestion = suggestSplitName(session, fights);
  const showSuggest = draft.name.trim() !== suggestion;
  const empty = session.fightCount === 0;

  return (
    <div
      className={cn(
        "rounded-xl border transition-colors duration-150",
        draft.include
          ? "border-accent-sky/30 bg-accent-sky/[0.04]"
          : "border-white/[0.06] bg-white/[0.02]"
      )}
    >
      <div className="flex items-start gap-3 p-3">
        {/* Include checkbox */}
        <button
          type="button"
          role="checkbox"
          aria-checked={draft.include}
          aria-label={`Include session ${session.index + 1}`}
          onClick={onToggleInclude}
          className={cn(
            "mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-md border transition-colors duration-150",
            "focus-visible:ring-2 focus-visible:ring-accent-sky/40 focus-visible:outline-none",
            draft.include
              ? "border-accent-sky/60 bg-accent-sky/80 text-primary-foreground"
              : "border-white/[0.15] bg-white/[0.03] hover:border-white/[0.3]"
          )}
        >
          {draft.include && <CheckCheck className="size-3" aria-hidden />}
        </button>

        <div className="min-w-0 flex-1">
          {/* Meta row */}
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
            <span className="font-semibold text-foreground/90">Session {session.index + 1}</span>
            {realm && <InfoPill color="muted">{realm}</InfoPill>}
            <span className="text-xs text-muted-foreground">
              {relativeFromMs(session.startTimeMs)} · {compactBytes(session.sizeBytes)}
            </span>
            {empty ? (
              <InfoPill color="muted">no fights</InfoPill>
            ) : (
              <InfoPill color="gold">
                {session.fightCount} fight{session.fightCount === 1 ? "" : "s"}
              </InfoPill>
            )}
          </div>

          {/* Name input (only meaningful when included) */}
          <div
            className={cn(
              "mt-2 flex items-center gap-2 transition-opacity",
              draft.include ? "opacity-100" : "opacity-50"
            )}
          >
            <Input
              value={draft.name}
              disabled={!draft.include}
              onChange={(e) => onRename(e.target.value)}
              placeholder={suggestion}
              aria-label={`Name for session ${session.index + 1}`}
              className="h-8 text-xs"
            />
            <span className="text-[11px] text-muted-foreground/70">.log</span>
            {draft.include && showSuggest && (
              <button
                type="button"
                onClick={onSuggest}
                title="Use the suggested name"
                className="inline-flex shrink-0 items-center gap-1 rounded-md border border-accent-sky/25 bg-accent-sky/[0.06] px-1.5 py-1 text-[11px] font-medium text-accent-sky transition-colors hover:bg-accent-sky/[0.12]"
              >
                <Sparkles className="size-3" aria-hidden />
              </button>
            )}
          </div>

          {/* Fight breakdown toggle */}
          {fights.length > 0 && (
            <button
              type="button"
              onClick={onToggleExpand}
              aria-expanded={expanded}
              className="mt-2 inline-flex items-center gap-1 text-[11px] font-medium text-muted-foreground transition-colors hover:text-foreground/80"
            >
              <ChevronDown
                className={cn("size-3 transition-transform duration-200", expanded && "rotate-180")}
                aria-hidden
              />
              {expanded ? "Hide" : "Show"} {fights.length} fight{fights.length === 1 ? "" : "s"}
            </button>
          )}

          {/* Fight breakdown list */}
          {expanded && fights.length > 0 && (
            <ul className="mt-2 max-h-44 space-y-1 overflow-y-auto border-t border-white/[0.06] pt-2">
              {fights.map((f) => (
                <li
                  key={f.index}
                  className="flex items-center justify-between gap-3 rounded-md bg-white/[0.02] px-2 py-1"
                >
                  <span className="flex min-w-0 items-center gap-1.5">
                    <Swords className="size-3 shrink-0 text-primary/70" aria-hidden />
                    <span className="truncate text-xs text-foreground/85">
                      {f.bossName || f.zoneName || `Fight ${f.index + 1}`}
                    </span>
                  </span>
                  <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground">
                    {formatDuration(f.endMs - f.startMs)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}
