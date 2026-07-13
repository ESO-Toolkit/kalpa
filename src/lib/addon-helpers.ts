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

export const VALID_SORT_MODES: readonly SortMode[] = ["name", "author", "updated", "installed"];

export function isSortMode(value: string): value is SortMode {
  return (VALID_SORT_MODES as readonly string[]).includes(value);
}

// Shared collator: `a.localeCompare(b)` constructs collation state per call,
// which multiplies across the O(n log n) comparisons of every list sort. One
// default-locale, default-options Collator is semantically identical.
const collator = new Intl.Collator();

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
    .map((addon) => ({
      // Decorate-sort-undecorate: lowercase once per element instead of once
      // per comparison (the sort would otherwise allocate two fresh strings
      // per comparison, ~2·n·log n allocations per keystroke/recompute).
      addon,
      titleKey: addon.title.toLowerCase(),
      authorKey: addon.author.toLowerCase(),
    }))
    .sort((left, right) => {
      const byName = () => collator.compare(left.titleKey, right.titleKey);
      switch (opts.sortMode) {
        case "author": {
          const byAuthor = collator.compare(left.authorKey, right.authorKey);
          return byAuthor !== 0 ? byAuthor : byName();
        }
        case "updated": {
          // Most recently updated upstream (ESOUI) first. Addons with no known
          // update time (0 — not on ESOUI or never update-checked) sort last,
          // then ties break by name so the order stays stable and legible.
          const lu = left.addon.esouiLastUpdate || 0;
          const ru = right.addon.esouiLastUpdate || 0;
          if (lu !== ru) {
            if (lu === 0) return 1;
            if (ru === 0) return -1;
            return ru - lu;
          }
          return byName();
        }
        case "installed": {
          // Most recently downloaded locally first (refreshed on each real
          // install/update). The timestamp is an ISO 8601 UTC string, so
          // lexicographic comparison is chronological. Addons Kalpa never
          // downloaded (empty string) sort last, ties break by name.
          const li = left.addon.installedAt || "";
          const ri = right.addon.installedAt || "";
          if (li !== ri) {
            if (!li) return 1;
            if (!ri) return -1;
            return ri.localeCompare(li);
          }
          return byName();
        }
        case "name":
        default:
          return byName();
      }
    })
    .map((entry) => entry.addon);
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
