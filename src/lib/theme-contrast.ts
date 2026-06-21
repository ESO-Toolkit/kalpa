import { contrastRatio } from "./theme-color";
import type { ThemeColors } from "./theme-types";

/**
 * WCAG 2 contrast checks surfaced live in the custom theme editor.
 *
 * WCAG 2 is the normative standard, so these are the hard gate. Each check has a
 * `min` (the AA-ish floor we warn below) and `good` (the comfortable target we
 * celebrate at/above). Text pairs target the usual 4.5:1; large/decorative and
 * non-text UI pairs target 3:1.
 */

export interface ContrastCheck {
  key: string;
  label: string;
  ratio: number;
  min: number;
  good: number;
  /** below `min` → fail, below `good` → ok, else → great */
  level: "fail" | "ok" | "great";
}

function check(
  key: string,
  label: string,
  a: string,
  b: string,
  min: number,
  good: number
): ContrastCheck {
  const ratio = Math.round(contrastRatio(a, b) * 100) / 100;
  const level: ContrastCheck["level"] = ratio < min ? "fail" : ratio < good ? "ok" : "great";
  return { key, label, ratio, min, good, level };
}

export function evaluateContrast(c: ThemeColors): ContrastCheck[] {
  return [
    check("fg-bg", "Text on background", c.foreground, c.background, 4.5, 7),
    check("muted-surface", "Muted text on panels", c.mutedForeground, c.surface, 4.5, 7),
    // `primary` is rendered as real text (text-primary) across the app, so it
    // needs the 4.5:1 text floor against the darkest surface it sits on (bg).
    check("primary-text", "Primary as text", c.primary, c.background, 4.5, 7),
    check("accent-surface", "Accent on panels", c.accent, c.surface, 3, 4.5),
    check("pfg-primary", "Label on primary button", c.primaryForeground, c.primary, 4.5, 7),
  ];
}

/** True if every check meets its minimum. */
export function passesAllContrast(c: ThemeColors): boolean {
  return evaluateContrast(c).every((r) => r.level !== "fail");
}
