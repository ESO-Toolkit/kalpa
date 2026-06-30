// Smart naming helpers for upload report names and split file names.
//
// ESO Logs reports are conventionally named with a run-type tag + content +
// date — e.g. "core prog — Lucent Citadel", "pug vRG farm". We derive a
// content+date base from the log's own parsed fights (zone/boss names) and offer
// one-tap run-type tags. All output is plain, lowercase-friendly, and safe to
// drop into a file stem (the backend re-sanitizes split names regardless).

import type { FightSummary, LogSession } from "@/types/uploader";

/** One-tap run-type tags from ESO raid vernacular. Order = display order. */
export const RUN_TAGS = [
  { id: "prog", label: "prog", hint: "Progression — learning the content" },
  { id: "core", label: "core", hint: "Your regular/core team" },
  { id: "pug", label: "pug", hint: "Pick-up group" },
  { id: "hm", label: "hm", hint: "Hard mode" },
  { id: "farm", label: "farm", hint: "Farming clears" },
  { id: "clear", label: "clear", hint: "A full clear" },
] as const;

export type RunTagId = (typeof RUN_TAGS)[number]["id"];

/** Title-case-ish a raw zone/boss string into a tidy token, keeping it short. */
function tidy(name: string): string {
  return name.trim().replace(/\s+/g, " ").slice(0, 48);
}

/** The most representative zone for a set of fights — the zone with the most
 *  fights (a raid night is usually one trial), falling back to the first seen. */
export function dominantZone(fights: FightSummary[]): string | null {
  const counts = new Map<string, number>();
  for (const f of fights) {
    if (f.zoneName) counts.set(f.zoneName, (counts.get(f.zoneName) ?? 0) + 1);
  }
  let best: string | null = null;
  let bestN = 0;
  for (const [zone, n] of counts) {
    if (n > bestN) {
      best = zone;
      bestN = n;
    }
  }
  return best ? tidy(best) : null;
}

/** A short, sortable date stamp (e.g. "Jun 18") from epoch ms. */
export function shortDate(ms: number | null | undefined): string {
  if (!ms) return "";
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** Build a human report-name suggestion from the log's content + a date.
 *  Examples: "Lucent Citadel — Jun 18", "Combat log — Jun 18". */
export function suggestReportName(fights: FightSummary[], whenMs: number | null): string {
  const zone = dominantZone(fights);
  const date = shortDate(whenMs);
  const base = zone ?? "Combat log";
  return date ? `${base} — ${date}` : base;
}

/** Build a file-stem suggestion for a single session's split — kebab-cased,
 *  content + date, e.g. "lucent-citadel-jun18". Falls back to a session label. */
export function suggestSplitName(session: LogSession, fights: FightSummary[]): string {
  // Fights are file-global; we can't cheaply map them to a session here, so the
  // caller passes the fights it considers in-session. Use the dominant zone if
  // any, else a stable session label.
  const zone = dominantZone(fights);
  const date = session.startTimeMs
    ? new Date(session.startTimeMs)
        .toLocaleDateString(undefined, { month: "short", day: "numeric" })
        .toLowerCase()
        .replace(/\s+/g, "")
    : "";
  const base = zone
    ? zone
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/^-|-$/g, "")
    : null;
  if (base) return date ? `${base}-${date}` : base;
  return `session-${session.index + 1}`;
}

/** Build a file-stem suggestion for a SINGLE fight's split — kebab-cased, from the
 *  fight's boss (preferred) or zone name, disambiguated by its 1-based ordinal
 *  within the session so repeated pulls of the same boss don't collide. Falls back
 *  to `fight-NN`. Example: "yandir-the-butcher-02", "lucent-citadel-01". */
export function suggestFightName(fight: FightSummary, ordinalInSession: number): string {
  const raw = fight.bossName || fight.zoneName || "";
  const base = raw
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
  const n = String(ordinalInSession).padStart(2, "0");
  return base ? `${base}-${n}` : `fight-${n}`;
}

/** Apply or toggle a run-tag onto an existing name. Tags are appended as a
 *  trailing " · tag" segment for report names, idempotently. */
export function withTag(name: string, tag: RunTagId): string {
  const trimmed = name.trim();
  // If the tag word is already present (whole word), leave it.
  const re = new RegExp(`(^|\\W)${tag}(\\W|$)`, "i");
  if (re.test(trimmed)) return trimmed;
  return trimmed ? `${trimmed} ${tag}` : tag;
}
