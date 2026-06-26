import { describe, it, expect } from "vitest";
import type { AddonManifest, FilterMode, SortMode } from "../../types";
import { filterAddons, isFilterMode, isSortMode, computeFilterCounts } from "../addon-helpers";

// ── Test data factory ──

function makeAddon(overrides: Partial<AddonManifest> = {}): AddonManifest {
  return {
    folderName: "TestAddon",
    title: "Test Addon",
    author: "TestAuthor",
    version: "1.0.0",
    addonVersion: 1,
    apiVersion: [101042],
    description: "A test addon",
    isLibrary: false,
    dependsOn: [],
    optionalDependsOn: [],
    missingDependencies: [],
    outdatedDependencies: [],
    missingOptionalDependencies: [],
    esouiId: 1234,
    tags: [],
    esouiLastUpdate: 0,
    installedAt: "",
    disabled: false,
    modifiedFileCount: 0,
    ...overrides,
  };
}

const ADDONS: AddonManifest[] = [
  makeAddon({ folderName: "AddonA", title: "Alpha Addon", author: "Zeta" }),
  makeAddon({ folderName: "AddonB", title: "Beta Addon", author: "Alpha", isLibrary: true }),
  makeAddon({
    folderName: "AddonC",
    title: "Charlie Addon",
    author: "Beta",
    tags: ["favorite", "essential"],
  }),
  makeAddon({
    folderName: "AddonD",
    title: "Delta Addon",
    author: "Gamma",
    missingDependencies: ["LibStub"],
    disabled: true,
  }),
  makeAddon({
    folderName: "AddonE",
    title: "Echo Addon",
    author: "Delta",
    outdatedDependencies: ["LibOld"],
  }),
];

const UPDATES_SET = new Set(["AddonA", "AddonC"]);

// ── Tests ──

describe("isFilterMode", () => {
  it("accepts valid filter modes", () => {
    expect(isFilterMode("all")).toBe(true);
    expect(isFilterMode("addons")).toBe(true);
    expect(isFilterMode("libraries")).toBe(true);
    expect(isFilterMode("outdated")).toBe(true);
    expect(isFilterMode("missing-deps")).toBe(true);
    expect(isFilterMode("favorites")).toBe(true);
    expect(isFilterMode("disabled")).toBe(true);
  });

  it("rejects invalid filter modes", () => {
    expect(isFilterMode("invalid")).toBe(false);
    expect(isFilterMode("")).toBe(false);
    expect(isFilterMode("ALL")).toBe(false);
    expect(isFilterMode("outdated-deps")).toBe(false);
  });
});

describe("isSortMode", () => {
  it("accepts valid sort modes", () => {
    expect(isSortMode("name")).toBe(true);
    expect(isSortMode("author")).toBe(true);
    expect(isSortMode("updated")).toBe(true);
    expect(isSortMode("installed")).toBe(true);
  });

  it("rejects invalid sort modes", () => {
    expect(isSortMode("invalid")).toBe(false);
    expect(isSortMode("")).toBe(false);
    expect(isSortMode("Name")).toBe(false);
    expect(isSortMode("date")).toBe(false);
  });
});

describe("filterAddons — filter modes", () => {
  const baseOpts = {
    filterMode: "all" as FilterMode,
    sortMode: "name" as SortMode,
    updatesSet: UPDATES_SET,
  };

  it("returns all addons for 'all' filter", () => {
    const result = filterAddons(ADDONS, baseOpts);
    expect(result).toHaveLength(5);
  });

  it("filters addons (non-libraries)", () => {
    const result = filterAddons(ADDONS, { ...baseOpts, filterMode: "addons" });
    expect(result).toHaveLength(4);
    expect(result.every((a) => !a.isLibrary)).toBe(true);
  });

  it("filters libraries only", () => {
    const result = filterAddons(ADDONS, { ...baseOpts, filterMode: "libraries" });
    expect(result).toHaveLength(1);
    expect(result[0]!.folderName).toBe("AddonB");
  });

  it("filters outdated addons", () => {
    const result = filterAddons(ADDONS, { ...baseOpts, filterMode: "outdated" });
    expect(result).toHaveLength(2);
    expect(result.map((a) => a.folderName).sort()).toEqual(["AddonA", "AddonC"]);
  });

  it("filters missing deps", () => {
    const result = filterAddons(ADDONS, { ...baseOpts, filterMode: "missing-deps" });
    expect(result).toHaveLength(2);
    expect(result.map((a) => a.folderName).sort()).toEqual(["AddonD", "AddonE"]);
  });

  it("filters favorites", () => {
    const result = filterAddons(ADDONS, { ...baseOpts, filterMode: "favorites" });
    expect(result).toHaveLength(1);
    expect(result[0]!.folderName).toBe("AddonC");
  });

  it("filters disabled", () => {
    const result = filterAddons(ADDONS, { ...baseOpts, filterMode: "disabled" });
    expect(result).toHaveLength(1);
    expect(result[0]!.folderName).toBe("AddonD");
  });

  it("filters by custom tag", () => {
    const result = filterAddons(ADDONS, { ...baseOpts, effectiveTagFilter: "essential" });
    expect(result).toHaveLength(1);
    expect(result[0]!.folderName).toBe("AddonC");
  });
});

describe("filterAddons — search", () => {
  const baseOpts = {
    filterMode: "all" as FilterMode,
    sortMode: "name" as SortMode,
    updatesSet: UPDATES_SET,
  };

  it("searches by title (case-insensitive)", () => {
    const result = filterAddons(ADDONS, { ...baseOpts, searchQuery: "charlie" });
    expect(result).toHaveLength(1);
    expect(result[0]!.title).toBe("Charlie Addon");
  });

  it("searches by folder name", () => {
    const result = filterAddons(ADDONS, { ...baseOpts, searchQuery: "AddonB" });
    expect(result).toHaveLength(1);
    expect(result[0]!.folderName).toBe("AddonB");
  });

  it("searches by author", () => {
    const result = filterAddons(ADDONS, { ...baseOpts, searchQuery: "gamma" });
    expect(result).toHaveLength(1);
    expect(result[0]!.author).toBe("Gamma");
  });

  it("searches by tag", () => {
    const result = filterAddons(ADDONS, { ...baseOpts, searchQuery: "essential" });
    expect(result).toHaveLength(1);
    expect(result[0]!.folderName).toBe("AddonC");
  });

  it("returns empty for no match", () => {
    const result = filterAddons(ADDONS, { ...baseOpts, searchQuery: "zzzznonexistent" });
    expect(result).toHaveLength(0);
  });

  it("search combines with filter mode", () => {
    const result = filterAddons(ADDONS, {
      ...baseOpts,
      searchQuery: "addon",
      filterMode: "libraries",
    });
    expect(result).toHaveLength(1);
    expect(result[0]!.isLibrary).toBe(true);
  });

  it("empty search returns all", () => {
    const result = filterAddons(ADDONS, { ...baseOpts, searchQuery: "" });
    expect(result).toHaveLength(5);
  });
});

describe("filterAddons — sorting", () => {
  const baseOpts = {
    filterMode: "all" as FilterMode,
    sortMode: "name" as SortMode,
    updatesSet: UPDATES_SET,
  };

  it("sorts by name (alphabetical, case-insensitive)", () => {
    const result = filterAddons(ADDONS, baseOpts);
    expect(result.map((a) => a.title)).toEqual([
      "Alpha Addon",
      "Beta Addon",
      "Charlie Addon",
      "Delta Addon",
      "Echo Addon",
    ]);
  });

  it("sorts by author (alphabetical, case-insensitive)", () => {
    const result = filterAddons(ADDONS, { ...baseOpts, sortMode: "author" });
    expect(result.map((a) => a.author)).toEqual(["Alpha", "Beta", "Delta", "Gamma", "Zeta"]);
  });

  it("sorting is stable with duplicate values", () => {
    const dupes = [
      makeAddon({ folderName: "A", title: "Same", author: "Same" }),
      makeAddon({ folderName: "B", title: "Same", author: "Same" }),
    ];
    const result = filterAddons(dupes, baseOpts);
    expect(result.map((a) => a.folderName)).toEqual(["A", "B"]);
  });

  it("sorts by recently updated (newest ESOUI update first)", () => {
    const addons = [
      makeAddon({ folderName: "Old", title: "Old", esouiLastUpdate: 1_000 }),
      makeAddon({ folderName: "New", title: "New", esouiLastUpdate: 9_000 }),
      makeAddon({ folderName: "Mid", title: "Mid", esouiLastUpdate: 5_000 }),
    ];
    const result = filterAddons(addons, { ...baseOpts, sortMode: "updated" });
    expect(result.map((a) => a.folderName)).toEqual(["New", "Mid", "Old"]);
  });

  it("sorts addons with no update time (0) last, tie-broken by name", () => {
    const addons = [
      makeAddon({ folderName: "Unknown2", title: "Zebra", esouiLastUpdate: 0 }),
      makeAddon({ folderName: "Known", title: "Known", esouiLastUpdate: 5_000 }),
      makeAddon({ folderName: "Unknown1", title: "Apple", esouiLastUpdate: 0 }),
    ];
    const result = filterAddons(addons, { ...baseOpts, sortMode: "updated" });
    // Known (has a date) first; the two unknowns follow ordered by title.
    expect(result.map((a) => a.folderName)).toEqual(["Known", "Unknown1", "Unknown2"]);
  });

  it("sorts by recently downloaded (newest download first)", () => {
    const addons = [
      makeAddon({ folderName: "First", title: "First", installedAt: "2024-01-01T00:00:00Z" }),
      makeAddon({ folderName: "Third", title: "Third", installedAt: "2024-06-15T00:00:00Z" }),
      makeAddon({ folderName: "Second", title: "Second", installedAt: "2024-03-10T00:00:00Z" }),
    ];
    const result = filterAddons(addons, { ...baseOpts, sortMode: "installed" });
    expect(result.map((a) => a.folderName)).toEqual(["Third", "Second", "First"]);
  });

  it("sorts never-downloaded addons (no installedAt) last, tie-broken by name", () => {
    const addons = [
      makeAddon({ folderName: "Untracked2", title: "Zebra", installedAt: "" }),
      makeAddon({ folderName: "Tracked", title: "Tracked", installedAt: "2024-05-01T00:00:00Z" }),
      makeAddon({ folderName: "Untracked1", title: "Apple", installedAt: "" }),
    ];
    const result = filterAddons(addons, { ...baseOpts, sortMode: "installed" });
    expect(result.map((a) => a.folderName)).toEqual(["Tracked", "Untracked1", "Untracked2"]);
  });
});

describe("computeFilterCounts", () => {
  const updatesMap = new Map([
    ["AddonA", {}],
    ["AddonC", {}],
  ]);

  it("computes correct counts for all modes", () => {
    const counts = computeFilterCounts(ADDONS, updatesMap);
    expect(counts.all).toBe(5);
    expect(counts.addons).toBe(4);
    expect(counts.libraries).toBe(1);
    expect(counts.favorites).toBe(1);
    expect(counts.outdated).toBe(2);
    expect(counts["missing-deps"]).toBe(2);
    expect(counts.disabled).toBe(1);
  });

  it("returns zero counts for empty addon list", () => {
    const counts = computeFilterCounts([], new Map());
    expect(Object.values(counts).every((c) => c === 0)).toBe(true);
  });
});
