import { useSyncExternalStore } from "react";
import {
  deleteCustomTheme,
  getActiveTheme,
  getAllThemes,
  getState,
  setActiveTheme,
  subscribe,
  upsertCustomTheme,
} from "./theme-manager";
import { BUILTIN_THEMES } from "./theme-presets";
import type { Theme } from "./theme-types";

/**
 * React binding for the theme manager. Any component can call this to read the
 * active theme / custom themes and mutate them; updates are pushed via the
 * manager's external store.
 */
export function useTheme() {
  const state = useSyncExternalStore(subscribe, getState, getState);

  return {
    activeThemeId: state.activeThemeId,
    activeTheme: getActiveTheme(),
    customThemes: state.customThemes,
    builtinThemes: BUILTIN_THEMES,
    allThemes: getAllThemes(),
    setActiveTheme,
    upsertCustomTheme,
    deleteCustomTheme,
  };
}

export type { Theme };
