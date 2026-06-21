import type { ThemeColors, ThemeSkin } from "./theme-types";

/**
 * Applies a theme by writing only the 12 BASE CSS variables to the document root.
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

const MANAGED_VARS = Object.values(VAR_MAP);

/** Resolve a theme's seed colors into the `{ "--css-var": value }` map applied to
 * the root. Used both to apply at runtime and to mirror to localStorage for the
 * synchronous pre-paint boot script (see index.html). */
export function themeColorsToVars(colors: ThemeColors): Record<string, string> {
  const vars: Record<string, string> = {};
  for (const key of Object.keys(VAR_MAP) as (keyof ThemeColors)[]) {
    vars[VAR_MAP[key]] = colors[key];
  }
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
