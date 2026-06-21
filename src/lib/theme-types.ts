/**
 * Theme type definitions.
 *
 * A theme is stored as a compact 12-color "seed" (see {@link ThemeColors}).
 * `expandSeed` (in `theme-color`/`theme-presets` consumers) turns a seed into
 * the ~40 CSS custom properties the rest of the app reads. Keeping themes as
 * seeds means the custom-theme editor only has to expose 12 pickers, and every
 * theme stays internally coherent.
 */

/** The 12 colors that fully define a theme. All values are `#rrggbb` hex. */
export interface ThemeColors {
  /** Deepest backdrop; ambient blurred orbs glow on top of it. */
  bgBase: string;
  /** App background, just above `bgBase`. */
  background: string;
  /** Panels & cards. */
  surface: string;
  /** Primary text (light, high contrast on `background`). */
  foreground: string;
  /** Secondary / dim text. */
  mutedForeground: string;
  /** Brand accent — buttons, key highlights, active states. */
  primary: string;
  /** Text/icon color that sits on top of `primary`-filled surfaces. */
  primaryForeground: string;
  /** Interactive accent — links, focus rings, info. */
  accent: string;
  /** Base border / divider color. */
  border: string;
  /** Ambient background orb #1 (usually echoes `primary`). */
  orb1: string;
  /** Ambient background orb #2 (usually echoes `accent`). */
  orb2: string;
  /** Ambient background orb #3 (tertiary mood hue). */
  orb3: string;
}

/** The editable keys of a theme, in display order, for the custom editor. */
export const THEME_COLOR_KEYS: readonly (keyof ThemeColors)[] = [
  "bgBase",
  "background",
  "surface",
  "foreground",
  "mutedForeground",
  "primary",
  "primaryForeground",
  "accent",
  "border",
  "orb1",
  "orb2",
  "orb3",
] as const;

/** Human-friendly labels + hints for each seed color (used by the editor). */
export const THEME_COLOR_META: Record<keyof ThemeColors, { label: string; hint: string }> = {
  bgBase: { label: "Base", hint: "Deepest backdrop behind the glow" },
  background: { label: "Background", hint: "Main app background" },
  surface: { label: "Surface", hint: "Panels and cards" },
  foreground: { label: "Text", hint: "Primary text" },
  mutedForeground: { label: "Muted text", hint: "Secondary / dim text" },
  primary: { label: "Primary", hint: "Brand accent — buttons & highlights" },
  primaryForeground: { label: "On primary", hint: "Text on primary buttons" },
  accent: { label: "Accent", hint: "Links, focus, interactive" },
  border: { label: "Border", hint: "Dividers and outlines" },
  orb1: { label: "Glow 1", hint: "Ambient background orb" },
  orb2: { label: "Glow 2", hint: "Ambient background orb" },
  orb3: { label: "Glow 3", hint: "Ambient background orb" },
};

export type ThemeCategory =
  | "ESO"
  | "ESO Lore"
  | "Elder Scrolls"
  | "Editor Classics"
  | "Neon"
  | "Nature"
  | "Gemstone"
  | "Metal"
  | "Minimal"
  | "Custom";

/**
 * Optional "skin" for flagship art themes — goes beyond color to give a theme a
 * material identity. All fields are plain CSS strings applied as variables, so
 * they stay offline/CSP-safe (gradients + SVG data-URIs, no external images).
 * Themes without a skin render with default radius and no texture.
 */
export interface ThemeSkin {
  /** Overrides `--radius` (e.g. "0.2rem" angular, "0.85rem" soft). */
  radius?: string;
  /** Full-screen material layer behind the UI — a `background-image` CSS value. */
  texture?: string;
  /** `background-size` for the texture layer. */
  textureSize?: string;
  /** Tiling motif overlay — a `background-image` CSS value. */
  pattern?: string;
  /** `background-size` for the pattern tile. */
  patternSize?: string;
  /** Opacity (0–1) of the pattern overlay. */
  patternOpacity?: number;
}

/** A complete, named theme. */
export interface Theme {
  id: string;
  name: string;
  description: string;
  category: ThemeCategory;
  colors: ThemeColors;
  /** Optional material skin (textures, patterns, radius) for art themes. */
  skin?: ThemeSkin;
  /** True for user-created themes (persisted separately, editable/deletable). */
  custom?: boolean;
}

/** Persisted appearance settings shape (Tauri store + localStorage mirror). */
export interface AppearanceState {
  activeThemeId: string;
  customThemes: Theme[];
}
