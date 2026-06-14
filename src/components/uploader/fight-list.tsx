// A per-fight segmented timeline. Each detected fight shows its own status chip
// (queued / uploading / uploaded / failed), beating the incumbents' single
// opaque progress bar. Used by both the manual preflight and the live dashboard.

import { CheckCircle2, Loader2, Swords, XCircle, Clock } from "lucide-react";
import { InfoPill } from "@/components/ui/info-pill";
import { cn } from "@/lib/utils";
import { formatDuration } from "./uploader-shared";
import type { FightSummary, LiveFight } from "@/types/uploader";

type FightRow = {
  index: number;
  title: string;
  subtitle: string;
  status?: LiveFight["status"];
  error?: string;
};

function statusChip(status: LiveFight["status"]) {
  switch (status) {
    case "uploaded":
      return (
        <InfoPill color="emerald" className="gap-1">
          <CheckCircle2 className="size-3" aria-hidden /> Uploaded
        </InfoPill>
      );
    case "uploading":
      return (
        <InfoPill color="sky" className="gap-1">
          <Loader2 className="size-3 animate-spin" aria-hidden /> Uploading
        </InfoPill>
      );
    case "failed":
      return (
        <InfoPill color="red" className="gap-1">
          <XCircle className="size-3" aria-hidden /> Failed
        </InfoPill>
      );
    default:
      return (
        <InfoPill color="muted" className="gap-1">
          <Clock className="size-3" aria-hidden /> Queued
        </InfoPill>
      );
  }
}

function fightTitle(zone: string | null, boss: string | null, index: number): string {
  if (boss) return boss;
  if (zone) return zone;
  return `Fight ${index + 1}`;
}

export function FightList({ fights, emptyHint }: { fights: FightRow[]; emptyHint?: string }) {
  if (fights.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-white/[0.08] p-6 text-center text-sm text-muted-foreground">
        {emptyHint ?? "No fights detected yet."}
      </div>
    );
  }

  return (
    <ul className="space-y-1.5" aria-label="Detected fights">
      {fights.map((f) => (
        <li
          key={f.index}
          className={cn(
            "flex items-center justify-between gap-3 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2",
            f.status === "failed" && "border-red-400/20"
          )}
        >
          <div className="flex min-w-0 items-center gap-2.5">
            <Swords className="size-4 shrink-0 text-[#c4a44a]/70" aria-hidden />
            <div className="min-w-0">
              <div className="truncate text-sm text-foreground/90">{f.title}</div>
              <div className="truncate text-xs text-muted-foreground">
                {f.subtitle}
                {f.error ? ` — ${f.error}` : ""}
              </div>
            </div>
          </div>
          {f.status && <div className="shrink-0">{statusChip(f.status)}</div>}
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

/** Build display rows from live-tracked fights. */
export function rowsFromLive(fights: LiveFight[]): FightRow[] {
  return fights.map((f) => ({
    index: f.index,
    title: fightTitle(f.zoneName, f.bossName, f.index),
    subtitle: `${formatDuration(f.durationMs)}${f.zoneName && f.bossName ? ` · ${f.zoneName}` : ""}`,
    status: f.status,
    error: f.error,
  }));
}
