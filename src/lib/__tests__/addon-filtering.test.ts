import { describe, it, expect } from "vitest";
import type { AddonManifest, FilterMode, SortMode } from "../../types";
import { filterAddons, isFilterMode, computeFilterCounts } from "../addon-helpers";

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
    esouiId: 1234,
    tags: [],
    esouiLastUpdate: 0,
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
