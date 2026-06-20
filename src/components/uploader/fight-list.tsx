// A per-fight timeline. Each detected fight is its own row with name + duration
// — far more glanceable than the incumbents' single opaque progress bar. Used by
// both the manual preflight (static fight list) and the live dashboard (fights
// appear as the official uploader streams them).

import { Radio, Swords } from "lucide-react";
import { InfoPill } from "@/components/ui/info-pill";
import { formatDuration } from "./uploader-shared";
import type { FightSummary, LiveFight } from "@/types/uploader";

type FightRow = {
  index: number;
  title: string;
  subtitle: string;
  /** When true, show a subtle "streaming" badge (live timeline). */
  live?: boolean;
};

function fightTitle(zone: string | null, boss: string | null, index: number): string {
  if (boss) return boss;
  if (zone) return zone;
  return `Fight ${index + 1}`;
}

export function FightList({
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
      {ordered.map((f) => (
        <li
          key={f.index}
          className="flex animate-[fade-in_0.2s_ease-out] items-center justify-between gap-3 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2"
        >
          <div className="flex min-w-0 items-center gap-2.5">
            <Swords className="size-4 shrink-0 text-[#c4a44a]/70" aria-hidden />
            <div className="min-w-0">
              <div className="truncate text-sm text-foreground/90">{f.title}</div>
              <div className="truncate text-xs text-muted-foreground">{f.subtitle}</div>
            </div>
          </div>
          {f.live && (
            <InfoPill color="sky" className="shrink-0 gap-1">
              <Radio className="size-3 animate-pulse" aria-hidden /> Streaming
            </InfoPill>
          )}
        </li>
      ))}
    </ul>
  );
}

/** Build display rows from static preflight fight summaries. */
export function rowsFromSummaries(fights: FightSummary[]): FightRow[] {
  return fights.map((f) => ({
    index: f.index,
    title: fightTitle(f.zoneName, f.bossName, f.index),
    subtitle: `${formatDuration(f.endMs - f.startMs)}${f.zoneName && f.bossName ? ` · ${f.zoneName}` : ""}`,
  }));
}

/** Build display rows from fights detected during a live session. */
export function rowsFromLive(fights: LiveFight[]): FightRow[] {
  return fights.map((f) => ({
    index: f.index,
    title: fightTitle(f.zoneName, f.bossName, f.index),
    subtitle: `${formatDuration(f.durationMs)}${f.zoneName && f.bossName ? ` · ${f.zoneName}` : ""}`,
    live: true,
  }));
}
