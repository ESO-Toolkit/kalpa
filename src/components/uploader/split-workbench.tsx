// The Split Workbench — a focused modal for carving an oversized `Encounter.log`
// into smaller, individually-uploadable files, at two granularities:
//
//   • By session — one file per logging session (each session already starts with
//     its own `BEGIN_LOG`, the cleanest split point for a giant file).
//   • By fight — one file per individual fight (the session preamble + just that
//     fight's combat block), so a single boss kill can be uploaded on its own.
//
// Each unit becomes a card/row the user can include/exclude, name (with a smart
// suggestion), and preview before writing. The backend re-sanitizes every name, so
// this is purely the authoring surface; it never controls the destination.

import { useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { Scissors, Swords, Sparkles, FolderOpen, CheckCheck, Filter, X } from "lucide-react";
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
import type {
  FightSelection,
  FightSummary,
  LogPreflight,
  LogSession,
  SplitSelection,
} from "@/types/uploader";
import {
  compactBytes,
  fightDurationHint,
  fightLabel,
  formatDuration,
  relativeFromMs,
} from "./uploader-shared";
import { RUN_TAGS, suggestFightName, suggestSplitName, withTag } from "./naming";

// Remember the last prefix the user applied to a batch of splits (e.g. a guild
// or character tag), so a regular raid-night split keeps the same convention.
const SPLIT_PREFIX_KEY = "kalpa.uploader.splitPrefix";

/** The two split granularities the workbench offers. */
type Granularity = "session" | "fight";

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

/** Per-fight editable state in the workbench. */
interface FightDraft {
  include: boolean;
  name: string;
}

/** Lowercase + kebab a raw string into a safe-ish file stem fragment. */
function kebab(raw: string): string {
  return raw
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
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
  // Per-fight split needs the parsed fight list, which preflight omits for very
  // large logs (to bound the IPC payload). When it's empty we keep the "By fight"
  // tab disabled and steer the user to split by session first.
  const fightsAvailable = fights.length > 0;

  const [granularity, setGranularity] = useState<Granularity>("session");

  // ── Per-session drafts ──────────────────────────────────────────────────────
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

  // ── Per-fight drafts ────────────────────────────────────────────────────────
  // For each fight, its 1-based ordinal within its enclosing session (so repeated
  // pulls of the same boss get distinct suggested names).
  const fightOrdinals = useMemo(() => {
    const m = new Map<number, number>();
    for (const s of sessions) {
      fightsInSession(s, fights).forEach((f, i) => m.set(f.index, i + 1));
    }
    return m;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessions.length, sessions[0]?.index, fights.length]);

  const initialFightDrafts = useMemo<Record<number, FightDraft>>(() => {
    const out: Record<number, FightDraft> = {};
    for (const f of fights) {
      out[f.index] = {
        include: true,
        name: suggestFightName(f, fightOrdinals.get(f.index) ?? f.index + 1),
      };
    }
    return out;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fights.length, fightOrdinals]);

  const [fightDrafts, setFightDrafts] = useState<Record<number, FightDraft>>(initialFightDrafts);
  const [fightPrefix, setFightPrefix] = useState<string>(() => {
    try {
      return localStorage.getItem(SPLIT_PREFIX_KEY) ?? "";
    } catch {
      return "";
    }
  });
  const appliedFightPrefixRef = useRef("");

  // Re-seed fight drafts if the preflight's fight set changes underneath.
  const fightDraftsKey = Object.keys(fightDrafts).length;
  if (fightDraftsKey === 0 && fights.length > 0) {
    setFightDrafts(initialFightDrafts);
  }

  const fightDraftFor = (prev: Record<number, FightDraft>, index: number): FightDraft =>
    prev[index] ?? { include: false, name: "" };

  const setFightDraft = (index: number, patch: Partial<FightDraft>) =>
    setFightDrafts((prev) => ({ ...prev, [index]: { ...fightDraftFor(prev, index), ...patch } }));

  const selectedFights = fights.filter((f) => fightDrafts[f.index]?.include);

  const selectAllFights = () =>
    setFightDrafts((prev) => {
      const next = { ...prev };
      for (const f of fights) next[f.index] = { ...fightDraftFor(prev, f.index), include: true };
      return next;
    });
  const clearAllFights = () =>
    setFightDrafts((prev) => {
      const next = { ...prev };
      for (const f of fights) next[f.index] = { ...fightDraftFor(prev, f.index), include: false };
      return next;
    });
  const keepBossFights = () =>
    setFightDrafts((prev) => {
      const next = { ...prev };
      for (const f of fights)
        next[f.index] = { ...fightDraftFor(prev, f.index), include: !!f.bossName };
      return next;
    });

  // Mirror of applyPrefix for the per-fight name set.
  const applyFightPrefix = (raw: string) => {
    const next = kebab(raw);
    const prev = appliedFightPrefixRef.current;
    setFightPrefix(raw);
    try {
      localStorage.setItem(SPLIT_PREFIX_KEY, raw);
    } catch {
      /* ignore */
    }
    if (next === prev) return;
    setFightDrafts((drafts) => {
      const out = { ...drafts };
      for (const f of fights) {
        const d = fightDraftFor(drafts, f.index);
        if (!d.include) continue;
        let stem = d.name;
        if (prev && stem.startsWith(`${prev}-`)) stem = stem.slice(prev.length + 1);
        out[f.index] = { ...d, name: next ? `${next}-${stem}` : stem };
      }
      return out;
    });
    appliedFightPrefixRef.current = next;
  };

  // ── Split actions ───────────────────────────────────────────────────────────
  // Shared post-write step: toast, reveal the first file, close.
  const finishSplit = async (written: string[], noun: string) => {
    toast.success(`Split into ${written.length} ${noun}${written.length === 1 ? "" : "s"}.`, {
      duration: 6000,
    });
    try {
      const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
      if (written[0]) await revealItemInDir(written[0]);
    } catch {
      /* reveal is best-effort */
    }
    onOpenChange(false);
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
      await finishSplit(written, "file");
    } catch (e) {
      toast.error(`Split failed: ${getTauriErrorMessage(e)}`);
    } finally {
      setSplitting(false);
    }
  };

  const handleSplitFights = async () => {
    if (selectedFights.length === 0) return;
    setSplitting(true);
    try {
      const selections: FightSelection[] = selectedFights.map((f) => ({
        index: f.index,
        name: fightDrafts[f.index]?.name?.trim() || null,
        // Pin the fight identity so a rescan that shifted indices is caught.
        startMs: f.startMs,
      }));
      const written = await invokeOrThrow<string[]>("uploader_split_fights_to_disk", {
        filePath,
        sessions: preflight?.sessions ?? null,
        fights: preflight?.fights ?? null,
        selections,
      });
      await finishSplit(written, "fight file");
    } catch (e) {
      toast.error(`Split failed: ${getTauriErrorMessage(e)}`);
    } finally {
      setSplitting(false);
    }
  };

  const byFight = granularity === "fight";
  const sessionsWithFights = sessions.filter((s) => fightsInSession(s, fights).length > 0);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[88vh] flex-col gap-0 overflow-hidden sm:max-w-2xl">
        <DialogHeader className="shrink-0">
          <DialogTitle className="flex items-center gap-2">
            <Scissors className="size-5 text-primary" aria-hidden />
            Split workbench
          </DialogTitle>
          <DialogDescription>
            Carve <code className="text-foreground/80">{fileName}</code> into smaller logs. Choose a
            granularity, pick which to keep, name each, then split.
          </DialogDescription>
        </DialogHeader>

        {/* Granularity toggle — a segmented control in a recessed track, mirroring
            the workspace mode tabs. "By fight" is disabled when the fight list
            wasn't scanned (very large log). */}
        <div className="mt-4 grid shrink-0 grid-cols-2 gap-1 rounded-lg border border-black/40 bg-black/25 p-1 shadow-[inset_0_2px_8px_-2px_rgba(0,0,0,0.6)]">
          <GranTab
            active={granularity === "session"}
            onClick={() => setGranularity("session")}
            Icon={FolderOpen}
            label="By session"
            hint={`${sessions.length} session${sessions.length === 1 ? "" : "s"}`}
          />
          <GranTab
            active={byFight}
            disabled={!fightsAvailable}
            onClick={() => fightsAvailable && setGranularity("fight")}
            Icon={Swords}
            label="By fight"
            hint={
              fightsAvailable
                ? `${fights.length} fight${fights.length === 1 ? "" : "s"}`
                : "too large to scan"
            }
          />
        </div>

        {/* Per-fight unavailable: an ALWAYS-VISIBLE explanation (a tooltip on the
            disabled tab wouldn't open in WebView2). Honest per case: a multi-session
            log can be split by session first, but a single dense session can't —
            don't send the user in a circle. */}
        {!fightsAvailable && (
          <p className="mt-2 px-1 text-[11px] text-muted-foreground/70">
            {sessions.length > 1
              ? "This log is too large to list every fight here — split it by session first, then split a single session by fight."
              : "This log has too many fights in one session to list them individually here — upload it whole, or split it by size outside Kalpa."}
          </p>
        )}

        {/* Preset toolbar */}
        {byFight ? (
          <div className="mt-4 flex shrink-0 flex-wrap items-center gap-1.5">
            <Button variant="outline" size="sm" onClick={selectAllFights}>
              <CheckCheck className="size-3.5" />
              Select all
            </Button>
            <Button variant="outline" size="sm" onClick={keepBossFights}>
              <Filter className="size-3.5" />
              Only boss fights
            </Button>
            <Button variant="ghost" size="sm" onClick={clearAllFights}>
              <X className="size-3.5" />
              Clear
            </Button>
          </div>
        ) : (
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
        )}

        {/* Batch naming */}
        {byFight
          ? selectedFights.length > 0 && (
              <div className="mt-2 flex shrink-0 flex-wrap items-center gap-1.5 rounded-lg border border-white/[0.06] bg-white/[0.02] px-2 py-1.5">
                <span className="text-[11px] font-medium text-muted-foreground">Name all:</span>
                <Input
                  value={fightPrefix}
                  onChange={(e) => applyFightPrefix(e.target.value)}
                  placeholder="prefix (e.g. kynes-hm)"
                  aria-label="Prefix for all fight names"
                  className="h-7 w-40 text-[11px]"
                />
                <span className="text-[11px] text-muted-foreground/50">
                  added before each fight name
                </span>
              </div>
            )
          : selected.length > 0 && (
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

        {/* Cards body. flex-1 + min-h-0 are load-bearing: without min-h-0 a flex
            child won't shrink below its content, so overflow-y-auto never engages
            and the footer scrolls off the capped-height dialog. */}
        <div className="-mr-2 mt-3 min-h-0 flex-1 space-y-2 overflow-y-auto pr-2">
          {byFight ? (
            sessionsWithFights.length === 0 ? (
              <div className="rounded-lg border border-dashed border-white/[0.08] p-6 text-center text-sm text-muted-foreground">
                No fights found in this file.
              </div>
            ) : (
              sessionsWithFights.map((s) => {
                const inFights = fightsInSession(s, fights);
                const realm = s.realm?.replace(/megaserver/i, "").trim() || null;
                return (
                  <div key={s.index} className="space-y-1.5">
                    <div className="flex items-center gap-2 px-1 pt-1">
                      <span className="font-heading text-[11px] font-semibold tracking-[0.06em] text-muted-foreground/60 uppercase">
                        Session {s.index + 1}
                      </span>
                      {realm && <InfoPill color="muted">{realm}</InfoPill>}
                      <span className="text-[11px] text-muted-foreground/50">
                        {relativeFromMs(s.startTimeMs)} · {inFights.length} fight
                        {inFights.length === 1 ? "" : "s"}
                      </span>
                    </div>
                    {inFights.map((f, i) => {
                      const d = fightDrafts[f.index] ?? { include: false, name: "" };
                      const suggestion = suggestFightName(f, i + 1);
                      return (
                        <FightRow
                          key={f.index}
                          fight={f}
                          draft={d}
                          suggestion={suggestion}
                          onToggleInclude={() => setFightDraft(f.index, { include: !d.include })}
                          onRename={(name) => setFightDraft(f.index, { name })}
                          onSuggest={() => setFightDraft(f.index, { name: suggestion })}
                        />
                      );
                    })}
                  </div>
                );
              })
            )
          ) : sessions.length === 0 ? (
            <div className="rounded-lg border border-dashed border-white/[0.08] p-6 text-center text-sm text-muted-foreground">
              No logging sessions found in this file.
            </div>
          ) : (
            sessions.map((s) => {
              const d = drafts[s.index] ?? { include: false, name: "" };
              const inFights = fightsInSession(s, fights);
              return (
                <SessionCard
                  key={s.index}
                  session={s}
                  draft={d}
                  fights={inFights}
                  onToggleInclude={() => setDraft(s.index, { include: !d.include })}
                  onRename={(name) => setDraft(s.index, { name })}
                  onSuggest={() => setDraft(s.index, { name: suggestSplitName(s, inFights) })}
                />
              );
            })
          )}
        </div>

        {/* Live preview + action footer */}
        <div className="-mx-5 -mb-5 mt-4 flex shrink-0 items-center justify-between gap-3 border-t border-white/[0.06] bg-gradient-to-b from-white/[0.02] to-transparent p-4">
          <div className="text-sm">
            {byFight ? (
              selectedFights.length === 0 ? (
                <span className="text-muted-foreground">Select at least one fight to split.</span>
              ) : (
                <span className="text-foreground/80">
                  <span className="font-semibold text-primary">{selectedFights.length}</span> fight
                  file{selectedFights.length === 1 ? "" : "s"}{" "}
                  <span className="text-muted-foreground">· one log per fight</span>
                </span>
              )
            ) : selected.length === 0 ? (
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
            {byFight ? (
              <Button
                size="sm"
                onClick={handleSplitFights}
                disabled={selectedFights.length === 0 || splitting}
              >
                <FolderOpen className="size-3.5" />
                {splitting ? "Splitting…" : "Split & reveal"}
              </Button>
            ) : (
              <Button size="sm" onClick={handleSplit} disabled={selected.length === 0 || splitting}>
                <FolderOpen className="size-3.5" />
                {splitting ? "Splitting…" : "Split & reveal"}
              </Button>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** One tab of the granularity toggle. Mirrors the workspace mode-tab styling but
 *  compact. When disabled (the per-fight list wasn't scanned) the reason is shown
 *  as always-visible helper text beneath the toggle — a tooltip on a native
 *  disabled button never opens in WebView2, so we don't rely on hover. */
function GranTab({
  active,
  disabled,
  onClick,
  Icon,
  label,
  hint,
}: {
  active: boolean;
  disabled?: boolean;
  onClick: () => void;
  Icon: typeof Swords;
  label: string;
  hint: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-pressed={active}
      className={cn(
        "flex items-center justify-center gap-2 rounded-md px-3 py-2 text-left transition-all duration-150",
        "focus-visible:ring-2 focus-visible:ring-accent-sky/40 focus-visible:outline-none",
        "disabled:cursor-not-allowed disabled:opacity-40",
        active
          ? "bg-gradient-to-b from-accent-sky/[0.14] to-accent-sky/[0.05] text-accent-sky shadow-[0_1px_2px_rgba(0,0,0,0.5),inset_0_1px_0_rgba(255,255,255,0.12)] ring-1 ring-inset ring-accent-sky/25"
          : "text-muted-foreground hover:bg-white/[0.04]"
      )}
    >
      <Icon
        className={cn("size-4 shrink-0", active ? "text-accent-sky" : "text-muted-foreground")}
        aria-hidden
      />
      <span className="text-sm font-semibold">{label}</span>
      <span
        className={cn("text-[11px]", active ? "text-accent-sky/60" : "text-muted-foreground/60")}
      >
        {hint}
      </span>
    </button>
  );
}

/** A single selectable fight row in the by-fight body: include checkbox, label +
 *  duration, and (when included) a name input with a suggest reset. */
function FightRow({
  fight,
  draft,
  suggestion,
  onToggleInclude,
  onRename,
  onSuggest,
}: {
  fight: FightSummary;
  draft: FightDraft;
  suggestion: string;
  onToggleInclude: () => void;
  onRename: (name: string) => void;
  onSuggest: () => void;
}) {
  const label = fightLabel(fight);
  const duration = formatDuration(fight.endMs - fight.startMs);
  const showSuggest = draft.include && draft.name.trim() !== suggestion;
  return (
    <div
      className={cn(
        "rounded-lg border px-3 py-2 transition-colors duration-150",
        draft.include
          ? "border-accent-sky/30 bg-accent-sky/[0.04]"
          : "border-white/[0.06] bg-white/[0.02]"
      )}
    >
      <div className="flex items-center gap-3">
        <button
          type="button"
          role="checkbox"
          aria-checked={draft.include}
          aria-label={`Include ${label}`}
          onClick={onToggleInclude}
          className={cn(
            "flex size-5 shrink-0 items-center justify-center rounded-md border transition-colors duration-150",
            "focus-visible:ring-2 focus-visible:ring-accent-sky/40 focus-visible:outline-none",
            draft.include
              ? "border-accent-sky/60 bg-accent-sky/80 text-primary-foreground"
              : "border-white/[0.15] bg-white/[0.03] hover:border-white/[0.3]"
          )}
        >
          {draft.include && <CheckCheck className="size-3" aria-hidden />}
        </button>
        <Swords className="size-3.5 shrink-0 text-primary/70" aria-hidden />
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <span className="truncate text-sm text-foreground/90">{label}</span>
          <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground">
            {duration}
          </span>
        </div>
      </div>
      {draft.include && (
        <div className="mt-2 flex items-center gap-2 pl-8">
          <Input
            value={draft.name}
            onChange={(e) => onRename(e.target.value)}
            placeholder={suggestion}
            aria-label={`Name for ${label}`}
            className="h-8 text-xs"
          />
          <span className="text-[11px] text-muted-foreground/70">.log</span>
          {showSuggest && (
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
      )}
    </div>
  );
}

function SessionCard({
  session,
  draft,
  fights,
  onToggleInclude,
  onRename,
  onSuggest,
}: {
  session: LogSession;
  draft: SessionDraft;
  fights: FightSummary[];
  onToggleInclude: () => void;
  onRename: (name: string) => void;
  onSuggest: () => void;
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

          {/* Fights — shown inline by default (no expander) so the user sees
              exactly what's in this session before splitting. Each row carries a
              duration and an honest quick-reset / long-pull hint. Capped height +
              scroll keeps a dense session from dominating the modal. */}
          {fights.length > 0 && (
            <div className="mt-2.5 border-t border-white/[0.06] pt-2">
              <div className="mb-1.5 flex items-center justify-between px-0.5">
                <span className="font-heading text-[10px] font-bold tracking-[0.06em] text-muted-foreground/55 uppercase">
                  Fights
                </span>
                <span className="text-[10px] tabular-nums text-muted-foreground/50">
                  {fights.length}
                </span>
              </div>
              <ul className="max-h-56 space-y-1 overflow-y-auto pr-0.5">
                {fights.map((f) => {
                  const ms = f.endMs - f.startMs;
                  const hint = fightDurationHint(ms);
                  return (
                    <li
                      key={f.index}
                      className="flex items-center justify-between gap-3 rounded-md bg-white/[0.02] px-2 py-1.5"
                    >
                      <span className="flex min-w-0 items-center gap-1.5">
                        <Swords className="size-3 shrink-0 text-primary/70" aria-hidden />
                        <span className="truncate text-xs text-foreground/85">{fightLabel(f)}</span>
                      </span>
                      <span className="flex shrink-0 items-center gap-1.5">
                        {hint && (
                          <InfoPill color={hint.color} className="text-[10px]">
                            {hint.label}
                          </InfoPill>
                        )}
                        <span className="text-[11px] tabular-nums text-muted-foreground">
                          {formatDuration(ms)}
                        </span>
                      </span>
                    </li>
                  );
                })}
              </ul>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
