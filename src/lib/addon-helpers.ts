import type { AddonManifest, FilterMode, SortMode } from "../types";

export const VALID_FILTER_MODES: readonly FilterMode[] = [
  "all",
  "addons",
  "libraries",
  "outdated",
  "missing-deps",
  "favorites",
  "disabled",
];

export function isFilterMode(value: string): value is FilterMode {
  return (VALID_FILTER_MODES as readonly string[]).includes(value);
}

export function filterAddons(
  addons: AddonManifest[],
  opts: {
    searchQuery?: string;
    filterMode: FilterMode;
    sortMode: SortMode;
    updatesSet: Set<string>;
    effectiveTagFilter?: string | null;
  }
): AddonManifest[] {
  return addons
    .filter((addon) => {
      if (opts.searchQuery) {
        const query = opts.searchQuery.toLowerCase();
        const matchesSearch =
          addon.title.toLowerCase().includes(query) ||
          addon.folderName.toLowerCase().includes(query) ||
          addon.author.toLowerCase().includes(query) ||
          addon.tags.some((tag) => tag.toLowerCase().includes(query));
        if (!matchesSearch) return false;
      }

      switch (opts.filterMode) {
        case "addons":
          return !addon.isLibrary;
        case "libraries":
          return addon.isLibrary;
        case "outdated":
          return opts.updatesSet.has(addon.folderName);
        case "missing-deps":
          return addon.missingDependencies.length > 0 || addon.outdatedDependencies.length > 0;
        case "favorites":
          return addon.tags.includes("favorite");
        case "disabled":
          return addon.disabled;
        default:
          if (opts.effectiveTagFilter) return addon.tags.includes(opts.effectiveTagFilter);
          return true;
      }
    })
    .sort((left, right) => {
      switch (opts.sortMode) {
        case "author":
          return left.author.toLowerCase().localeCompare(right.author.toLowerCase());
        case "name":
        default:
          return left.title.toLowerCase().localeCompare(right.title.toLowerCase());
      }
    });
}

export function computeFilterCounts(
  addons: AddonManifest[],
  updatesMap: Map<string, unknown>
): Record<FilterMode, number> {
  return {
    all: addons.length,
    addons: addons.filter((a) => !a.isLibrary).length,
    libraries: addons.filter((a) => a.isLibrary).length,
    favorites: addons.filter((a) => a.tags.includes("favorite")).length,
    outdated: addons.filter((a) => updatesMap.has(a.folderName)).length,
    "missing-deps": addons.filter(
      (a) => a.missingDependencies.length > 0 || a.outdatedDependencies.length > 0
    ).length,
    disabled: addons.filter((a) => a.disabled).length,
  };
}
