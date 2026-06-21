import { getSetting, setSetting } from "./store";
import {
  BUILTIN_THEMES,
  BUILTIN_THEME_IDS,
  DEFAULT_THEME,
  DEFAULT_THEME_ID,
} from "./theme-presets";
import {
  applyThemeColors,
  applySkin,
  clearThemeColors,
  themeColorsToVars,
  themeSkinToVars,
} from "./theme-apply";
import type { Theme, ThemeColors } from "./theme-types";

/** Minimal shape of the ViewTransition returned by document.startViewTransition. */
interface ViewTransitionLike {
  skipTransition: () => void;
  finished: Promise<unknown>;
}

/**
 * Theme manager — the single source of truth for the active theme and the user's
 * custom themes. It owns:
 *  - applying a theme's colors to the document (delegated to `theme-apply`),
 *  - persistence to the Tauri store (durable) mirrored to localStorage (so the
 *    choice can be applied synchronously before first paint — no flash),
 *  - a tiny external store so React components can subscribe via `useSyncExternalStore`.
 */

const STORE_KEY_ACTIVE = "appearance.activeThemeId";
const STORE_KEY_CUSTOM = "appearance.customThemes";
const LS_KEY_ACTIVE = "kalpa.appearance.activeThemeId";
const LS_KEY_CUSTOM = "kalpa.appearance.customThemes";
/** Resolved `{ "--var": value }` map of the active theme — read SYNCHRONOUSLY by
 * the inline boot script in index.html before first paint (see that file). */
export const LS_KEY_VARS = "kalpa.appearance.activeVars";

interface ManagerState {
  activeThemeId: string;
  customThemes: Theme[];
}

let state: ManagerState = { activeThemeId: DEFAULT_THEME_ID, customThemes: [] };
const listeners = new Set<() => void>();

function emit() {
  for (const l of listeners) l();
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getState(): ManagerState {
  return state;
}

/** All themes (built-in + custom) available to choose from. */
export function getAllThemes(): Theme[] {
  return [...BUILTIN_THEMES, ...state.customThemes];
}

export function resolveTheme(id: string): Theme | undefined {
  return getAllThemes().find((t) => t.id === id);
}

export function getActiveTheme(): Theme {
  return resolveTheme(state.activeThemeId) ?? DEFAULT_THEME;
}

// ---------------------------------------------------------------------------
// localStorage mirror (synchronous, used for flash-free startup)
// ---------------------------------------------------------------------------

function writeLocalMirror(next: ManagerState) {
  try {
    localStorage.setItem(LS_KEY_ACTIVE, next.activeThemeId);
    localStorage.setItem(LS_KEY_CUSTOM, JSON.stringify(next.customThemes));
    // Mirror the active theme's resolved CSS vars so the pre-paint boot script
    // can apply them with zero logic. The default theme stores nothing — the
    // authored :root values govern.
    const active = [...BUILTIN_THEMES, ...next.customThemes].find(
      (t) => t.id === next.activeThemeId
    );
    if (!active || active.id === DEFAULT_THEME_ID) {
      localStorage.removeItem(LS_KEY_VARS);
    } else {
      const vars = { ...themeColorsToVars(active.colors), ...themeSkinToVars(active.skin) };
      localStorage.setItem(LS_KEY_VARS, JSON.stringify(vars));
    }
  } catch {
    // localStorage may be unavailable; durability still comes from the Tauri store.
  }
}

// ---------------------------------------------------------------------------
// Applying
// ---------------------------------------------------------------------------

let activeVT: ViewTransitionLike | null = null;

/** Apply a resolved theme to the document. The default theme clears overrides so
 * the base `:root` values (authored in index.css) win exactly. */
function applyTheme(theme: Theme, animate: boolean) {
  const run = () => {
    if (theme.id === DEFAULT_THEME_ID) {
      clearThemeColors();
    } else {
      applyThemeColors(theme.colors);
    }
    applySkin(theme.id === DEFAULT_THEME_ID ? undefined : theme.skin);
    document.documentElement.dataset.themeId = theme.id;
  };

  const prefersReduced =
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

  const vt = (
    document as Document & {
      startViewTransition?: (cb: () => void) => ViewTransitionLike;
    }
  ).startViewTransition;

  if (animate && !prefersReduced && typeof vt === "function") {
    // Collapse any in-flight transition so rapid theme switching doesn't stack
    // snapshots (only one view transition runs at a time).
    activeVT?.skipTransition();
    const transition = vt.call(document, run);
    activeVT = transition;
    transition.finished.finally(() => {
      if (activeVT === transition) activeVT = null;
    });
  } else {
    run();
  }
}

// ---------------------------------------------------------------------------
// Public mutations
// ---------------------------------------------------------------------------

/** Switch the active theme (with a cross-fade when supported). */
export function setActiveTheme(id: string) {
  const theme = resolveTheme(id) ?? DEFAULT_THEME;
  state = { ...state, activeThemeId: theme.id };
  applyTheme(theme, true);
  writeLocalMirror(state);
  void setSetting(STORE_KEY_ACTIVE, theme.id);
  emit();
}

/** Create or update a custom theme; re-applies it if it's the active one. */
export function upsertCustomTheme(theme: Theme) {
  const custom: Theme = { ...theme, custom: true };
  const existing = state.customThemes.findIndex((t) => t.id === custom.id);
  const customThemes =
    existing >= 0
      ? state.customThemes.map((t, i) => (i === existing ? custom : t))
      : [...state.customThemes, custom];
  state = { ...state, customThemes };
  if (state.activeThemeId === custom.id) applyTheme(custom, false);
  writeLocalMirror(state);
  void setSetting(STORE_KEY_CUSTOM, customThemes);
  emit();
}

/** Delete a custom theme; falls back to the default if it was active. */
export function deleteCustomTheme(id: string) {
  const customThemes = state.customThemes.filter((t) => t.id !== id);
  let activeThemeId = state.activeThemeId;
  if (activeThemeId === id) {
    activeThemeId = DEFAULT_THEME_ID;
    applyTheme(DEFAULT_THEME, true);
  }
  state = { activeThemeId, customThemes };
  writeLocalMirror(state);
  void setSetting(STORE_KEY_CUSTOM, customThemes);
  void setSetting(STORE_KEY_ACTIVE, activeThemeId);
  emit();
}

/** Generate a unique custom theme id. */
export function newCustomThemeId(): string {
  const rand =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID().slice(0, 8)
      : Math.floor(Math.random() * 1e8).toString(16);
  return `custom-${rand}`;
}

export function isBuiltin(id: string): boolean {
  return BUILTIN_THEME_IDS.has(id);
}

// ---------------------------------------------------------------------------
// Live preview (used by the custom theme editor; does not persist)
// ---------------------------------------------------------------------------

/** Apply colors to the document without changing state or persisting. Custom
 * theme drafts have no skin, so any active skin (texture/pattern) is cleared. */
export function previewThemeColors(colors: ThemeColors) {
  applyThemeColors(colors);
  applySkin(undefined);
}

/** Restore the document to the current active theme after a preview. */
export function stopPreview() {
  applyTheme(getActiveTheme(), false);
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

/**
 * Hydrate from the durable Tauri store (source of truth) and apply. Call once on
 * mount; reconciles anything the synchronous pre-paint boot script missed (e.g.
 * a custom theme edited on another window). The pre-paint apply itself happens
 * in the inline boot script in index.html — NOT here — because this module is a
 * deferred ES module and would paint a frame late.
 */
export async function hydrateThemeFromStore() {
  const [activeThemeId, customThemes] = await Promise.all([
    getSetting<string>(STORE_KEY_ACTIVE, DEFAULT_THEME_ID),
    getSetting<Theme[]>(STORE_KEY_CUSTOM, []),
  ]);
  state = {
    activeThemeId,
    customThemes: Array.isArray(customThemes) ? customThemes : [],
  };
  applyTheme(getActiveTheme(), false);
  writeLocalMirror(state);
  emit();
}
