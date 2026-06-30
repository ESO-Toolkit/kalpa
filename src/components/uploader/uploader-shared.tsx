// Shared presentational pieces for the uploader workspace: the glanceable
// status pill, the "what gets uploaded" privacy summary, and small helpers.

import { useEffect, useState } from "react";
import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  CircleDashed,
  Loader2,
  RefreshCw,
  ShieldQuestion,
  Swords,
  Lock,
  Zap,
} from "lucide-react";
import { InfoPill } from "@/components/ui/info-pill";
import { cn } from "@/lib/utils";
import type { ReportRef, UploaderStatus, Visibility } from "@/types/uploader";

/** Map the uploader status to its pill color, label, and icon. */
export function StatusPill({ status }: { status: UploaderStatus }) {
  const map: Record<
    UploaderStatus,
    {
      color: "muted" | "sky" | "emerald" | "amber" | "red";
      label: string;
      Icon: typeof Activity;
      spin?: boolean;
    }
  > = {
    idle: { color: "muted", label: "Idle", Icon: CircleDashed },
    watching: { color: "sky", label: "Watching", Icon: Activity },
    uploading: { color: "sky", label: "Uploading", Icon: Loader2, spin: true },
    upToDate: { color: "emerald", label: "Up to date", Icon: CheckCircle2 },
    retrying: { color: "amber", label: "Retrying", Icon: RefreshCw, spin: true },
    attention: { color: "red", label: "Needs attention", Icon: AlertTriangle },
  };
  const { color, label, Icon, spin } = map[status];
  return (
    // role=status so screen readers announce status changes (text + icon, not
    // color alone — the app is always dark).
    <InfoPill color={color} role="status" aria-live="polite" className="gap-1.5 px-2.5 py-1">
      <Icon className={cn("size-3.5", spin && "animate-spin")} aria-hidden />
      {label}
    </InfoPill>
  );
}

/** Format a byte-count compactly (kept local to avoid import churn). */
export function compactBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 10 || i === 0 ? 0 : 1)} ${units[i]}`;
}

/** Deep-link to the ESO Log Aggregator (esotk.com) analysis for a report `code`.
 *  esotk reads the ESO Logs report by code via the public GraphQL API and renders
 *  the owner's richer analysis (fight detection + HM, rotation/cast lists, scribing,
 *  insights, 3D replay) — strictly better than the raw esologs.com report view, and
 *  the reason Kalpa hands off viewing rather than rebuilding it.
 *
 *  `live: true` targets esotk's LiveLog view, which 30s-repolls the newest in-progress
 *  fight — the right link to open while a raid is still streaming. The code is always
 *  server-issued alphanumeric, but encode it defensively so a malformed value can't
 *  break out of the fragment path. */
export function esotkReportUrl(code: string, opts?: { live?: boolean }): string {
  const base = `https://esotk.com/#/report/${encodeURIComponent(code)}`;
  return opts?.live ? `${base}/live` : base;
}

/** Raw ESO Logs report URL. During live streaming, `fight=last` follows the newest
 *  fight and avoids esotk live-data lag where the report can have players on ESO
 *  Logs while esotk's `playerDetails` resolver still returns an empty array. */
export function esoLogsReportUrl(report: ReportRef, opts?: { live?: boolean }): string {
  if (!opts?.live) return report.url;
  try {
    const url = new URL(report.url);
    url.searchParams.set("fight", "last");
    return url.toString();
  } catch {
    const separator = report.url.includes("?") ? "&" : "?";
    return `${report.url}${separator}fight=last`;
  }
}

/** Primary report destination for buttons/auto-open. Public and unlisted completed
 *  reports can be read by esotk by code; private reports and active live reports
 *  open directly on ESO Logs, where players/fight=last are reliable mid-stream. */
export function primaryReportUrl(
  report: ReportRef,
  visibility: Visibility,
  opts?: { live?: boolean }
): string {
  if (opts?.live || visibility === "private") return esoLogsReportUrl(report, opts);
  return esotkReportUrl(report.code);
}

/** Extract an ESO Logs report code from a pasted URL or bare code, or null if the
 *  input doesn't look like one. Accepts both link shapes Kalpa hands users: the raw
 *  `esologs.com/reports/<code>` path and the `esotk.com/#/report/<code>` analysis
 *  deep-link, plus a bare mixed-alphanumeric report token. */
export function parseReportCode(raw: string): string | null {
  const s = raw.trim();
  const fromUrl = s.match(/reports?\/([a-zA-Z0-9]+)/);
  if (fromUrl?.[1]) return fromUrl[1];
  if (/^[a-zA-Z0-9]{12,}$/.test(s) && /[a-zA-Z]/.test(s) && /[0-9]/.test(s)) return s;
  return null;
}

/** Format a millisecond duration as `m:ss` or `s.s s`. */
export function formatDuration(ms: number): string {
  if (ms <= 0) return "0s";
  const totalSec = ms / 1000;
  if (totalSec < 60) return `${totalSec.toFixed(totalSec < 10 ? 1 : 0)}s`;
  const m = Math.floor(totalSec / 60);
  const s = Math.round(totalSec % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/** A short label for a fight: boss > zone > 1-based ordinal fallback. Shared so the
 *  timeline, live ticker, split workbench, and preflight peek all read the same. */
export function fightLabel(fight: {
  bossName: string | null;
  zoneName: string | null;
  index: number;
}): string {
  return fight.bossName || fight.zoneName || `Fight ${fight.index + 1}`;
}

/** A duration-derived hint for a fight (honest, not a kill/wipe claim): a very
 *  short fight is usually a quick reset/pull, a long one a sustained attempt.
 *  Null = no strong signal, so callers show nothing rather than guess. Shared by
 *  the fight timeline and the split workbench so both read the same. */
export function fightDurationHint(
  ms: number | undefined
): { label: string; color: "muted" | "amber" } | null {
  if (!ms || ms <= 0) return null;
  if (ms < 12_000) return { label: "quick reset", color: "amber" };
  if (ms >= 90_000) return { label: "long pull", color: "muted" };
  return null;
}

/** Format an elapsed duration as a session clock: `M:SS`, then `H:MM:SS` past an
 *  hour. Used by the live session timer (counts up; raids have no deadline). */
export function formatElapsed(ms: number): string {
  const totalSec = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  const s = totalSec % 60;
  if (h > 0) {
    return `${h}:${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
  }
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/** A live session stopwatch. Ticks once a second while mounted, computing
 *  elapsed from a fixed start timestamp (drift-free; survives dropped ticks over
 *  a multi-hour raid). Mounted only while a session runs, so the interval is torn
 *  down with the component — no setState-after-unmount risk. */
export function SessionTimer({ startMs, className }: { startMs: number; className?: string }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);
  // Fold the value INTO the label: the timer is now rendered in a non-aria-hidden
  // place (the live core), so a bare "Session elapsed time" label would mask the
  // visible value from screen readers. Safe because this is outside any aria-live
  // region — it's read only on navigation, never announced on each tick.
  const elapsed = formatElapsed(now - startMs);
  return (
    <span
      className={cn("font-heading text-xs tabular-nums text-muted-foreground", className)}
      aria-label={`Session elapsed time: ${elapsed}`}
    >
      {elapsed}
    </span>
  );
}

/** Relative time from an epoch-ms timestamp. */
export function relativeFromMs(ms: number): string {
  if (!ms) return "unknown";
  const diff = Date.now() - ms;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) {
    const m = Math.floor(diff / 60_000);
    return `${m} min${m === 1 ? "" : "s"} ago`;
  }
  if (diff < 86_400_000) {
    const h = Math.floor(diff / 3_600_000);
    return `${h} hour${h === 1 ? "" : "s"} ago`;
  }
  return new Date(ms).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

/**
 * A layered-notice privacy summary: a scannable headline that expands to the
 * specifics. No existing uploader surfaces this — it's a deliberate trust
 * differentiator. Frames the upload honestly as "this is your log".
 */
export function WhatGetsUploaded() {
  const [open, setOpen] = useState(false);
  return (
    // Read-once trust note — it RECEDES into the canvas (no panel, no border) so
    // it doesn't compete with the real work surfaces. Expands to the detail.
    <div className="overflow-hidden rounded-lg">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center justify-between gap-3 rounded-lg px-1 py-1.5 text-left transition-colors hover:bg-white/[0.03]"
        aria-expanded={open}
      >
        <span className="flex items-center gap-2 text-xs text-muted-foreground/80">
          <Lock className="size-3.5 text-emerald-400/70" aria-hidden />
          This report is built from your <code className="text-foreground/70">
            Encounter.log
          </code>{" "}
          and is owned by your ESO Logs account.
        </span>
        <ChevronDown
          className={cn(
            "size-4 shrink-0 text-muted-foreground transition-transform duration-200",
            open && "rotate-180"
          )}
          aria-hidden
        />
      </button>
      {open && (
        <div className="mt-1 animate-[fade-in_0.2s_ease-out] space-y-2 rounded-lg bg-black/20 p-3 text-xs text-muted-foreground">
          <div className="flex items-start gap-2">
            <Swords className="mt-0.5 size-3.5 shrink-0 text-accent-sky/80" aria-hidden />
            <span>
              <span className="text-foreground/80">What's uploaded:</span> combat events, character
              and ability data, and timestamps from your session log.
            </span>
          </div>
          <div className="flex items-start gap-2">
            <ShieldQuestion className="mt-0.5 size-3.5 shrink-0 text-emerald-400/80" aria-hidden />
            <span>
              <span className="text-foreground/80">What's never uploaded:</span> your account
              password, chat, or anything outside the combat log.
            </span>
          </div>
          <div className="flex items-start gap-2">
            <Lock className="mt-0.5 size-3.5 shrink-0 text-violet-400/80" aria-hidden />
            <span>
              You control visibility (public, unlisted, or private) and can delete a report from ESO
              Logs at any time.
            </span>
          </div>
          <div className="flex items-start gap-2">
            <Zap className="mt-0.5 size-3.5 shrink-0 text-primary/80" aria-hidden />
            <span>
              <span className="text-foreground/80">How it uploads:</span> Kalpa uploads directly to
              ESO Logs when you enable it (faster, an unofficial but operator-approved method);
              anything it can't encode exactly falls back to the official uploader automatically.
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
