import { relativeLuminance } from "./theme-color";
import type { ThemeColors, ThemeSkin } from "./theme-types";

/**
 * Applies a theme by writing the 12 BASE CSS variables plus derived structural/scrim ink to the document root.
 *
 * Every other token (card-alt, muted, secondary, glass tints, scrollbar, sidebar,
 * primary-hover, glow, …) is derived from these 12 in `index.css` using
 * `color-mix()` — a Baseline CSS feature in evergreen Chromium / WebView2. Because
 * `var()` is live, overriding a base variable here makes all derived tokens
 * recompute automatically. The default theme clears these overrides so the
 * authored `:root` values win exactly (no visual regression).
 */

/** Map of seed color -> the CSS custom property it drives. */
const VAR_MAP: Record<keyof ThemeColors, string> = {
  bgBase: "--bg-base",
  background: "--background",
  surface: "--card",
  foreground: "--foreground",
  mutedForeground: "--muted-foreground",
  primary: "--primary",
  primaryForeground: "--primary-foreground",
  accent: "--accent-sky",
  border: "--border",
  orb1: "--orb-1",
  orb2: "--orb-2",
  orb3: "--orb-3",
};

const STRUCTURE_RGB_VAR = "--structure-rgb";
const SCRIM_RGB_VAR = "--scrim-rgb";

const DARK_STATUS_VARS = {
  "--status-success": "#34d399",
  "--status-success-soft": "#6ee7b7",
  "--status-success-muted": "#a7f3d0",
  "--status-success-faint": "#d1fae5",
  "--status-success-strong": "#10b981",
  "--status-warning": "#fbbf24",
  "--status-warning-soft": "#fcd34d",
  "--status-warning-muted": "#fde68a",
  "--status-warning-faint": "#fef3c7",
  "--status-warning-strong": "#f59e0b",
  "--status-danger": "#f87171",
  "--status-danger-soft": "#fca5a5",
  "--status-danger-muted": "#fecaca",
  "--status-danger-faint": "#fee2e2",
  "--status-danger-strong": "#ef4444",
  "--status-library": "#a78bfa",
  "--status-library-strong": "#8b5cf6",
  "--status-info": "#38bdf8",
  "--status-info-soft": "#7dd3fc",
  "--status-info-strong": "#0ea5e9",
} as const;

const LIGHT_STATUS_VARS = {
  "--status-success": "#022c22",
  "--status-success-soft": "#022c22",
  "--status-success-muted": "#022c22",
  "--status-success-faint": "#022c22",
  "--status-success-strong": "#022c22",
  "--status-warning": "#451a03",
  "--status-warning-soft": "#451a03",
  "--status-warning-muted": "#451a03",
  "--status-warning-faint": "#451a03",
  "--status-warning-strong": "#451a03",
  "--status-danger": "#450a0a",
  "--status-danger-soft": "#450a0a",
  "--status-danger-muted": "#450a0a",
  "--status-danger-faint": "#450a0a",
  "--status-danger-strong": "#450a0a",
  "--status-library": "#6d28d9",
  "--status-library-strong": "#6d28d9",
  "--status-info": "#0369a1",
  "--status-info-soft": "#0369a1",
  "--status-info-strong": "#075985",
} as const;

const STATUS_ALIAS_VARS = ["--status-error", "--status-error-strong"] as const;

const STATUS_VARS = [...Object.keys(DARK_STATUS_VARS), ...STATUS_ALIAS_VARS] as `--${string}`[];

const MANAGED_VARS = [...Object.values(VAR_MAP), STRUCTURE_RGB_VAR, SCRIM_RGB_VAR, ...STATUS_VARS];

/** Background luminance at/above this point reads as a light theme, so translucent
 * structural affordances (hairlines, fill plates, dividers) should use black ink.
 * Below it, keep the original white ink exactly. */
export const LIGHT_THEME_BACKGROUND_LUMINANCE = 0.45;

export function structureInkForTheme(colors: ThemeColors): "0 0 0" | "255 255 255" {
  return relativeLuminance(colors.background) >= LIGHT_THEME_BACKGROUND_LUMINANCE
    ? "0 0 0"
    : "255 255 255";
}

/** Scrims are the inverse polarity of structural ink: dark themes keep the old
 * black depth washes exactly, while light themes use a white wash instead. */
export function scrimInkForTheme(colors: ThemeColors): "0 0 0" | "255 255 255" {
  return relativeLuminance(colors.background) >= LIGHT_THEME_BACKGROUND_LUMINANCE
    ? "255 255 255"
    : "0 0 0";
}

export function statusVarsForTheme(colors: ThemeColors): Record<string, string> {
  const vars =
    relativeLuminance(colors.background) >= LIGHT_THEME_BACKGROUND_LUMINANCE
      ? LIGHT_STATUS_VARS
      : DARK_STATUS_VARS;

  return {
    ...vars,
    "--status-error": vars["--status-danger"],
    "--status-error-strong": vars["--status-danger-strong"],
  };
}

/** Resolve a theme's seed colors into the `{ "--css-var": value }` map applied to
 * the root. Used both to apply at runtime and to mirror to localStorage for the
 * synchronous pre-paint boot script (see index.html). */
export function themeColorsToVars(colors: ThemeColors): Record<string, string> {
  const vars: Record<string, string> = {};
  for (const key of Object.keys(VAR_MAP) as (keyof ThemeColors)[]) {
    vars[VAR_MAP[key]] = colors[key];
  }
  vars[STRUCTURE_RGB_VAR] = structureInkForTheme(colors);
  vars[SCRIM_RGB_VAR] = scrimInkForTheme(colors);
  Object.assign(vars, statusVarsForTheme(colors));
  return vars;
}

export function applyThemeColors(colors: ThemeColors) {
  const root = document.documentElement;
  for (const [name, value] of Object.entries(themeColorsToVars(colors))) {
    root.style.setProperty(name, value);
  }
}

export function clearThemeColors() {
  const root = document.documentElement;
  for (const name of MANAGED_VARS) {
    root.style.removeProperty(name);
  }
}

// ---------------------------------------------------------------------------
// Skins (textures, patterns, radius) — optional material identity for art themes
// ---------------------------------------------------------------------------

/** All CSS vars a skin can drive. Cleared when a theme has no skin. */
const SKIN_VARS = [
  "--radius",
  "--app-texture",
  "--app-texture-size",
  "--app-pattern",
  "--app-pattern-size",
  "--app-pattern-opacity",
] as const;

/** Resolve a skin into a `{ "--css-var": value }` map (empty if no skin). */
export function themeSkinToVars(skin?: ThemeSkin): Record<string, string> {
  const v: Record<string, string> = {};
  if (!skin) return v;
  if (skin.radius) v["--radius"] = skin.radius;
  if (skin.texture) v["--app-texture"] = skin.texture;
  if (skin.textureSize) v["--app-texture-size"] = skin.textureSize;
  if (skin.pattern) v["--app-pattern"] = skin.pattern;
  if (skin.patternSize) v["--app-pattern-size"] = skin.patternSize;
  if (skin.patternOpacity != null) v["--app-pattern-opacity"] = String(skin.patternOpacity);
  return v;
}

/** Apply a skin (or clear all skin vars when undefined → falls back to defaults). */
export function applySkin(skin?: ThemeSkin) {
  const root = document.documentElement;
  for (const name of SKIN_VARS) root.style.removeProperty(name);
  for (const [name, value] of Object.entries(themeSkinToVars(skin))) {
    root.style.setProperty(name, value);
  }
  if (skin?.texture || skin?.pattern) root.dataset.textured = "true";
  else delete root.dataset.textured;
}
