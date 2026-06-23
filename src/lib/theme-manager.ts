import { getSetting, setSetting, setSettings } from "./store";
import {
  BUILTIN_THEMES,
  BUILTIN_THEME_IDS,
  DEFAULT_THEME,
  DEFAULT_THEME_ID,
  ROOT_THEME_ID,
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
/** Durable record of the last forced-default migration this user has been moved
 * through (source of truth; the Tauri store). */
const STORE_KEY_FORCED_DEFAULT = "appearance.forcedDefaultVersion";
/** Bump this to forcibly reset EVERY user's active theme to the current
 * {@link DEFAULT_THEME_ID} once on next launch — overriding any theme they had
 * previously chosen. After the reset they keep full control of the picker; the
 * stored version marker stops it from re-applying on later launches.
 * v1 — move everyone onto Nordic Runestone.
 * KEEP IN SYNC with FORCED_VERSION in public/theme-boot.js (guarded by
 * theme-boot.test.ts) so the pre-paint path agrees with hydration. */
export const FORCED_DEFAULT_VERSION = 1;
/** Resolved `{ "--var": value }` map of the active theme — read SYNCHRONOUSLY by
 * the boot script in index.html before first paint (see that file). The Tauri
 * store is the durable source of truth. */
export const LS_KEY_VARS = "kalpa.appearance.activeVars";
/** Synchronous mirror of {@link STORE_KEY_FORCED_DEFAULT} for the pre-paint boot
 * script: lets it tell a not-yet-migrated install (paint the factory default)
 * from a migrated one (trust the per-user mirror), avoiding a stale-theme flash
 * on the migration launch. */
export const LS_KEY_FORCED = "kalpa.appearance.forcedDefaultVersion";

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
    // Mirror the active theme's resolved CSS vars so the pre-paint boot script
    // can apply them with zero logic. Written for EVERY theme — including the
    // ESO Gold base — so an absent mirror unambiguously means "fresh install,"
    // letting the boot script paint the factory default instead of the authored
    // :root (which is ESO Gold, no longer the default). The base theme's vars
    // equal :root, so applying them pre-paint is visually a no-op.
    const active = [...BUILTIN_THEMES, ...next.customThemes].find(
      (t) => t.id === next.activeThemeId
    );
    if (!active) {
      localStorage.removeItem(LS_KEY_VARS);
      return;
    }
    const vars = { ...themeColorsToVars(active.colors), ...themeSkinToVars(active.skin) };
    localStorage.setItem(LS_KEY_VARS, JSON.stringify(vars));
  } catch {
    // localStorage may be unavailable; durability still comes from the Tauri store.
  }
}

// ---------------------------------------------------------------------------
// Applying
// ---------------------------------------------------------------------------

let activeVT: ViewTransitionLike | null = null;

/** Apply a resolved theme to the document. The ESO Gold base theme clears
 * overrides so the authored `:root` values (in index.css) win exactly. */
function applyTheme(theme: Theme, animate: boolean) {
  const run = () => {
    if (theme.id === ROOT_THEME_ID) {
      clearThemeColors();
    } else {
      applyThemeColors(theme.colors);
    }
    applySkin(theme.id === ROOT_THEME_ID ? undefined : theme.skin);
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

/** Create or update a custom theme; re-applies it if it's the active one.
 * State/apply/mirror are synchronous; resolves to the DURABLE-store outcome so
 * callers can report success/failure honestly. */
export async function upsertCustomTheme(theme: Theme): Promise<boolean> {
  const custom: Theme = { ...theme, custom: true };
  const existing = state.customThemes.findIndex((t) => t.id === custom.id);
  const customThemes =
    existing >= 0
      ? state.customThemes.map((t, i) => (i === existing ? custom : t))
      : [...state.customThemes, custom];
  state = { ...state, customThemes };
  if (state.activeThemeId === custom.id) applyTheme(custom, false);
  writeLocalMirror(state);
  emit();
  return setSetting(STORE_KEY_CUSTOM, customThemes);
}

/** Delete a custom theme; falls back to the default if it was active. */
export async function deleteCustomTheme(id: string): Promise<boolean> {
  const customThemes = state.customThemes.filter((t) => t.id !== id);
  let activeThemeId = state.activeThemeId;
  if (activeThemeId === id) {
    activeThemeId = DEFAULT_THEME_ID;
    applyTheme(DEFAULT_THEME, true);
    void setSetting(STORE_KEY_ACTIVE, activeThemeId);
  }
  state = { activeThemeId, customThemes };
  writeLocalMirror(state);
  emit();
  return setSetting(STORE_KEY_CUSTOM, customThemes);
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
  const [storedActiveId, storedCustom, storedForced] = await Promise.all([
    getSetting<string>(STORE_KEY_ACTIVE, DEFAULT_THEME_ID),
    getSetting<Theme[]>(STORE_KEY_CUSTOM, []),
    getSetting<number>(STORE_KEY_FORCED_DEFAULT, 0),
  ]);
  const customThemes = Array.isArray(storedCustom) ? storedCustom : [];
  const known = new Set([...BUILTIN_THEME_IDS, ...customThemes.map((t) => t.id)]);
  const storedActive = known.has(storedActiveId) ? storedActiveId : DEFAULT_THEME_ID;
  const forcedVersion = storedForced ?? 0;

  let activeThemeId = storedActive;
  let effectiveForced = forcedVersion;

  if (forcedVersion < FORCED_DEFAULT_VERSION) {
    // One-time forced migration: move every user onto the current factory default
    // once, overriding whatever they had chosen. Record the override and its
    // version marker as one unit and only adopt the reset if it actually persisted
    // — otherwise a partial write could override a choice without durably marking
    // the migration done, re-firing the reset on every later launch.
    // Marker first: if a debounced autosave ever flushed mid-batch (before the
    // single save()), a partial state should be "marked migrated, still old theme"
    // (benign) — never "reset without marker," which would re-fire and clobber a
    // later choice.
    const recorded = await setSettings({
      [STORE_KEY_FORCED_DEFAULT]: FORCED_DEFAULT_VERSION,
      [STORE_KEY_ACTIVE]: DEFAULT_THEME_ID,
    });
    if (recorded) {
      activeThemeId = DEFAULT_THEME_ID;
      effectiveForced = FORCED_DEFAULT_VERSION;
    }
    // If it did not persist, honor the stored choice and retry next launch.
  } else if (storedActive !== storedActiveId) {
    // Already migrated: just heal a dangling active id (e.g. a custom theme
    // deleted on another window).
    void setSetting(STORE_KEY_ACTIVE, activeThemeId);
  }

  state = { activeThemeId, customThemes };
  applyTheme(getActiveTheme(), false);
  writeLocalMirror(state);
  // Mirror the (now durable) migration version for the pre-paint boot script.
  try {
    localStorage.setItem(LS_KEY_FORCED, String(effectiveForced));
  } catch {
    // localStorage may be unavailable; the Tauri store remains the source of truth.
  }
  emit();
}
