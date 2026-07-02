// A per-fight timeline. Each detected fight is its own row with name + duration
// — far more glanceable than the incumbents' single opaque progress bar. Used by
// both the manual preflight (static fight list) and the live dashboard (fights
// appear as the selected live path streams them).

import { memo } from "react";
import { Radio, Swords } from "lucide-react";
import { InfoPill } from "@/components/ui/info-pill";
import { cn } from "@/lib/utils";
import { fightDurationHint, fightLabel, formatDuration } from "./uploader-shared";
import type { FightSummary, LiveFight } from "@/types/uploader";

type FightRow = {
  index: number;
  title: string;
  subtitle: string;
  /** Fight length in ms, for the duration hint. */
  durationMs?: number;
  /** When true, show a subtle "streaming" badge (live timeline). */
  live?: boolean;
};

export const FightList = memo(function FightList({
  fights,
  emptyHint,
  // Live mode shows the most-recent fight on top so the current action never
  // scrolls off-screen. Display-only: the caller's array stays chronological
  // (the rolling-window + dedupe logic depends on append order), we only flip
  // the rendered order here.
  newestFirst = false,
}: {
  fights: FightRow[];
  emptyHint?: string;
  newestFirst?: boolean;
}) {
  if (fights.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-white/[0.08] p-6 text-center text-sm text-muted-foreground">
        {emptyHint ?? "No fights detected yet."}
      </div>
    );
  }

  const ordered = newestFirst ? [...fights].reverse() : fights;
  // Announce streamed fights to assistive tech only in live mode (polite, so it
  // doesn't interrupt). The static preflight list isn't a live region.
  const live = ordered.some((f) => f.live);

  return (
    // Cap the height and scroll: a dense progression night can produce hundreds
    // of fights, which would otherwise grow the dialog unbounded.
    <ul
      className="max-h-64 space-y-1.5 overflow-y-auto"
      aria-label="Detected fights"
      {...(live ? { role: "status", "aria-live": "polite", "aria-atomic": "false" } : {})}
    >
      {ordered.map((f, i) => {
        // In live newest-first mode the first rendered row is the most recent
        // fight; give it a one-shot accent so the eye catches the new arrival,
        // then it settles into the list.
        const isNewest = live && newestFirst && i === 0;
        const hint = fightDurationHint(f.durationMs);
        return (
          <li
            key={f.index}
            className={cn(
              "flex animate-[fade-in_0.2s_ease-out] items-center justify-between gap-3 rounded-lg border px-3 py-2 transition-colors duration-150",
              isNewest
                ? "border-accent-sky/30 bg-accent-sky/[0.05]"
                : "border-white/[0.06] bg-white/[0.02] hover:bg-white/[0.04]"
            )}
          >
            <div className="flex min-w-0 items-center gap-2.5">
              <Swords className="size-4 shrink-0 text-primary/70" aria-hidden />
              <div className="min-w-0">
                <div className="truncate text-sm text-foreground/90">{f.title}</div>
                <div className="truncate text-xs text-muted-foreground">{f.subtitle}</div>
              </div>
            </div>
            <div className="flex shrink-0 items-center gap-1.5">
              {hint && (
                <InfoPill color={hint.color} className="text-[11px]">
                  {hint.label}
                </InfoPill>
              )}
              {f.live && (
                <InfoPill color="sky" className="gap-1">
                  <Radio className="size-3 animate-pulse" aria-hidden /> Streaming
                </InfoPill>
              )}
            </div>
          </li>
        );
      })}
    </ul>
  );
});

/** Build display rows from static preflight fight summaries. */
export function rowsFromSummaries(fights: FightSummary[]): FightRow[] {
  return fights.map((f) => ({
    index: f.index,
    title: fightLabel(f),
    subtitle: `${formatDuration(f.endMs - f.startMs)}${f.zoneName && f.bossName ? ` · ${f.zoneName}` : ""}`,
    durationMs: f.endMs - f.startMs,
  }));
}

/** Build display rows from fights detected during a live session. */
export function rowsFromLive(fights: LiveFight[]): FightRow[] {
  return fights.map((f) => ({
    index: f.index,
    title: fightLabel(f),
    subtitle: `${formatDuration(f.durationMs)}${f.zoneName && f.bossName ? ` · ${f.zoneName}` : ""}`,
    durationMs: f.durationMs,
    live: true,
  }));
}
