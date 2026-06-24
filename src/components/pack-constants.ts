// Shared constants for Pack Hub components

import type { CSSProperties } from "react";
import type { PackType } from "../types";

export type PackTypeFilter = "all" | "addon-pack" | "build-pack" | "roster-pack";
export type SortOption = "votes" | "newest" | "updated";
export type TabMode = "browse" | "create" | "my-packs";
export type ShareMode = "private-link" | "export-file";
export type ImportMode = "enter-code" | "import-file";
export type MyPacksSubTab = "created" | "installed";

export const TYPE_LABELS: Record<string, string> = {
  "addon-pack": "Addon Pack",
  "build-pack": "Build Pack",
  "roster-pack": "Roster Pack",
};

export const TAG_COLORS: Record<
  string,
  "gold" | "sky" | "emerald" | "amber" | "red" | "violet" | "muted"
> = {
  essential: "gold",
  trial: "red",
  pve: "emerald",
  pvp: "red",
  healer: "sky",
  dps: "amber",
  tank: "violet",
  beginner: "emerald",
  utility: "muted",
};

export const PACK_TYPE_ACCENT: Record<
  PackType,
  { border: string; bg: string; hoverBg: string; text: string; hoverGlow: string }
> = {
  "addon-pack": {
    border: "border-l-primary/70",
    bg: "bg-primary/[0.03]",
    hoverBg: "hover:bg-primary/[0.08]",
    text: "text-primary",
    hoverGlow:
      "hover:shadow-[0_6px_24px_color-mix(in_oklab,var(--primary)_10%,transparent),inset_0_1px_0_rgba(255,255,255,0.06)]",
  },
  "build-pack": {
    border: "border-l-accent-sky/70",
    bg: "bg-accent-sky/[0.03]",
    hoverBg: "hover:bg-accent-sky/[0.08]",
    text: "text-accent-sky",
    hoverGlow:
      "hover:shadow-[0_6px_24px_color-mix(in_oklab,var(--accent-sky)_10%,transparent),inset_0_1px_0_rgba(255,255,255,0.06)]",
  },
  "roster-pack": {
    border: "border-l-violet-400/70",
    bg: "bg-violet-400/[0.03]",
    hoverBg: "hover:bg-violet-400/[0.08]",
    text: "text-violet-400",
    hoverGlow:
      "hover:shadow-[0_6px_24px_color-mix(in_oklab,var(--status-library)_10%,transparent),inset_0_1px_0_rgba(255,255,255,0.06)]",
  },
};

export const PACK_TYPE_PILL_COLOR: Record<string, "gold" | "sky" | "violet" | "muted"> = {
  "addon-pack": "gold",
  "build-pack": "sky",
  "roster-pack": "violet",
};

export const PRESET_TAGS = [
  "trial",
  "pvp",
  "beginner",
  "healer",
  "tank",
  "dps",
  "utility",
] as const;

export const PACK_TYPE_DESCRIPTIONS: Record<string, string> = {
  "addon-pack": "A collection of addons",
  "build-pack": "A skill build or loadout",
  "roster-pack": "A group or raid roster",
};

// ── Per-pack visual identity (monogram tile) ──────────────────────────────
//
// Problem: two packs of the SAME type + SAME author render identically today
// (same border / tint / pill). We fix this WITHOUT discarding type:
//   - TYPE stays the canonical PACK_TYPE_ACCENT left border + tint + pill text.
//   - Per-pack IDENTITY is added via a monogram tile whose LETTERS come from the
//     title (data-derived, never collide for differently-titled packs) and whose
//     COLOR is a deterministic FNV-1a hash of the pack id.
//
// All accent anchors are THEME CSS VARIABLES (emerald/amber/error/library/cyan
// all remap to color-mix(... var(--primary)) tokens in index.css), so identity
// tiles recompute on every theme switch and never go off-brand. Nothing here is
// keyed on specific pack names — everything derives from id/title.

/** On-brand accent anchors, each a themeable CSS var that recomputes per theme
 *  (see index.css --status-* / --accent-cyan tokens). Order is fixed so the
 *  hash -> accent mapping stays stable across releases. */
export const PACK_IDENTITY_VARS = [
  "--primary", // gold
  "--accent-sky", // sky
  "--accent-cyan", // cyan
  "--status-success", // emerald
  "--status-warning", // amber
  "--status-error", // rose/red
  "--status-library", // violet
] as const;

/** Canonical TYPE accent var — used as the collision reference so a pack's
 *  identity color always differs from its own type color (visible travel). */
const PACK_TYPE_IDENTITY_VAR: Record<string, string> = {
  "addon-pack": "--primary",
  "build-pack": "--accent-sky",
  "roster-pack": "--status-library",
};

/** FNV-1a (32-bit). Stable, well-distributed, dependency-free. id -> same look. */
function hashPackId(input: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < input.length; i++) {
    h ^= input.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** Title -> 1-2 uppercase initials. Single word -> first two letters; multi-word
 *  -> first + last word initial. Strips punctuation/emoji, falls back to "?".
 *  Every index access is guarded for noUncheckedIndexedAccess. */
function packMonogram(title: string): string {
  const words = title
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  if (words.length === 0) return "?";
  if (words.length === 1) {
    const w = words[0] ?? "";
    return [...w].slice(0, 2).join("").toUpperCase() || "?";
  }
  const first = words[0] ?? "";
  const last = words[words.length - 1] ?? "";
  const mono = ((first[0] ?? "") + (last[0] ?? "")).toUpperCase();
  return mono || "?";
}

export interface PackIdentity {
  /** 1-2 char monogram for the tile. */
  monogram: string;
  /** The chosen accent CSS var name, e.g. "--accent-cyan". */
  accentVar: string;
  /** Inline style for the monogram tile (per-pack gradient + glyph + ring). */
  tileStyle: CSSProperties;
  /** Inline style exposing --pk-glow for the card's per-pack hover glow. */
  glowVars: CSSProperties;
}

/** Deterministic identity for a pack. Pure function of id + title. Accepts a
 *  minimal shape so the browse card (Pack), the My Packs card (Pack), and the
 *  Installed card (InstalledPackRef, mapped to { id, title }) can all reuse it. */
export function packIdentity(pack: { id: string; title: string; packType?: string }): PackIdentity {
  const seed = pack.id || pack.title || "?";
  const hash = hashPackId(seed);

  // Pick an accent that is NOT the pack's own type color, so two same-type packs
  // always read as distinct hues (visible color travel from the type accent).
  const typeVar = pack.packType ? PACK_TYPE_IDENTITY_VAR[pack.packType] : undefined;
  let idx = hash % PACK_IDENTITY_VARS.length;
  if (typeVar && PACK_IDENTITY_VARS[idx] === typeVar) {
    idx = (idx + 1) % PACK_IDENTITY_VARS.length;
  }
  const accentVar = PACK_IDENTITY_VARS[idx] ?? "--accent-sky";
  const angle = 115 + (hash % 50); // 115-164deg: subtle per-pack gradient variance

  const tileStyle: CSSProperties = {
    backgroundImage: `linear-gradient(${angle}deg, color-mix(in oklab, var(${accentVar}) 30%, transparent), color-mix(in oklab, var(${accentVar}) 8%, transparent))`,
    // Glyph: bright accent on the dark translucent tile.
    color: `color-mix(in oklab, var(${accentVar}) 70%, white)`,
    // Top inner highlight + crisp 1px accent ring = modern app-tile depth.
    boxShadow:
      `inset 0 1px 0 color-mix(in oklab, white 16%, transparent), ` +
      `inset 0 0 0 1px color-mix(in oklab, var(${accentVar}) 26%, transparent)`,
  };

  // Per-pack hover glow color, consumed by the card via a bounded arbitrary class.
  const glowVars = {
    "--pk-glow": `color-mix(in oklab, var(${accentVar}) 16%, transparent)`,
  } as CSSProperties;

  return { monogram: packMonogram(pack.title), accentVar, tileStyle, glowVars };
}
