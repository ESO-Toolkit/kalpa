import { describe, it, expect } from "vitest";
import type { SvTreeNode } from "../../types";

// These functions are module-private in saved-variables.tsx, so we replicate them here
// for testing. This tests the logic correctness, and if the functions were ever extracted
// to a shared module, these tests would import from there directly.

const SYSTEM_SV_NAMES = new Set([
  "ZO_Ingame",
  "ZO_InternalIngame",
  "ZO_Pregame",
  "AccountSettings",
  "GuildHistoryCache",
]);

function classifyFile(
  f: { addonName: string },
  installedFolders: Set<string>
): "installed" | "system" | "orphaned" {
  if (SYSTEM_SV_NAMES.has(f.addonName)) return "system";
  if (installedFolders.has(f.addonName)) return "installed";
  for (const folder of installedFolders) {
    if (
      folder.length >= 4 &&
      f.addonName.startsWith(folder) &&
      f.addonName.length > folder.length
    ) {
      const boundaryChar = f.addonName[folder.length];
      if (!boundaryChar || /[A-Z_-]/.test(boundaryChar)) {
        return "installed";
      }
    }
  }
  return "orphaned";
}

type SizeCategory = "small" | "medium" | "large";
function sizeCategory(bytes: number): SizeCategory {
  if (bytes >= 5 * 1024 * 1024) return "large";
  if (bytes >= 1024 * 1024) return "medium";
  return "small";
}

function updateTreeNode(
  tree: SvTreeNode,
  path: string[],
  value: string | number | boolean | null,
  depth = 0
): SvTreeNode {
  if (depth >= path.length || !tree.children) return tree;
  const targetKey = path[depth];
  const isLeaf = depth === path.length - 1;
  return {
    ...tree,
    children: tree.children.map((child) => {
      if (child.key !== targetKey) return child;
      if (isLeaf) {
        return { ...child, value: value };
      }
      return updateTreeNode(child, path, value, depth + 1);
    }),
  };
}

// ── classifyFile ──

describe("classifyFile", () => {
  const installed = new Set(["CombatMetrics", "HarvestMap", "LibAddonMenu-2.0", "Azurah"]);

  it("classifies ESO system files", () => {
    expect(classifyFile({ addonName: "ZO_Ingame" }, installed)).toBe("system");
    expect(classifyFile({ addonName: "ZO_InternalIngame" }, installed)).toBe("system");
    expect(classifyFile({ addonName: "ZO_Pregame" }, installed)).toBe("system");
    expect(classifyFile({ addonName: "AccountSettings" }, installed)).toBe("system");
    expect(classifyFile({ addonName: "GuildHistoryCache" }, installed)).toBe("system");
  });

  it("classifies exact addon matches", () => {
    expect(classifyFile({ addonName: "CombatMetrics" }, installed)).toBe("installed");
    expect(classifyFile({ addonName: "LibAddonMenu-2.0" }, installed)).toBe("installed");
    expect(classifyFile({ addonName: "Azurah" }, installed)).toBe("installed");
  });

  it("classifies prefix matches with uppercase boundary", () => {
    expect(classifyFile({ addonName: "CombatMetricsFightData" }, installed)).toBe("installed");
    expect(classifyFile({ addonName: "HarvestMapAD" }, installed)).toBe("installed");
  });

  it("classifies prefix matches with underscore boundary", () => {
    expect(classifyFile({ addonName: "CombatMetrics_Data" }, installed)).toBe("installed");
    expect(classifyFile({ addonName: "HarvestMap_Nodes" }, installed)).toBe("installed");
  });

  it("classifies prefix matches with hyphen boundary", () => {
    expect(classifyFile({ addonName: "HarvestMap-Extra" }, installed)).toBe("installed");
  });

  it("rejects prefix matches with lowercase boundary", () => {
    expect(classifyFile({ addonName: "HarvestMapdata" }, installed)).toBe("orphaned");
    expect(classifyFile({ addonName: "CombatMetricsextra" }, installed)).toBe("orphaned");
  });

  it("rejects short folder prefix matches (< 4 chars)", () => {
    const shortFolders = new Set(["Lib", "UI"]);
    expect(classifyFile({ addonName: "Liberty" }, shortFolders)).toBe("orphaned");
    expect(classifyFile({ addonName: "UIData" }, shortFolders)).toBe("orphaned");
  });

  it("classifies unknown addons as orphaned", () => {
    expect(classifyFile({ addonName: "RandomUnknownAddon" }, installed)).toBe("orphaned");
    expect(classifyFile({ addonName: "TotallyNotInstalled" }, installed)).toBe("orphaned");
  });

  it("handles empty installed set", () => {
    const empty = new Set<string>();
    expect(classifyFile({ addonName: "ZO_Ingame" }, empty)).toBe("system");
    expect(classifyFile({ addonName: "SomeAddon" }, empty)).toBe("orphaned");
  });

  it("system classification takes priority over installed match", () => {
    const withSystem = new Set(["ZO_Ingame"]);
    expect(classifyFile({ addonName: "ZO_Ingame" }, withSystem)).toBe("system");
  });
});

// ── sizeCategory ──

describe("sizeCategory", () => {
  it("classifies small files (< 1 MB)", () => {
    expect(sizeCategory(0)).toBe("small");
    expect(sizeCategory(100)).toBe("small");
    expect(sizeCategory(1024 * 1024 - 1)).toBe("small");
  });

  it("classifies medium files (1-5 MB)", () => {
    expect(sizeCategory(1024 * 1024)).toBe("medium");
    expect(sizeCategory(3 * 1024 * 1024)).toBe("medium");
    expect(sizeCategory(5 * 1024 * 1024 - 1)).toBe("medium");
  });

  it("classifies large files (>= 5 MB)", () => {
    expect(sizeCategory(5 * 1024 * 1024)).toBe("large");
    expect(sizeCategory(100 * 1024 * 1024)).toBe("large");
  });
});

// ── updateTreeNode ──

describe("updateTreeNode", () => {
  const tree: SvTreeNode = {
    key: "root",
    valueType: "table",
    children: [
      {
        key: "section",
        valueType: "table",
        children: [
          { key: "enabled", valueType: "boolean", value: true },
          { key: "count", valueType: "number", value: 10 },
          {
            key: "nested",
            valueType: "table",
            children: [{ key: "deep", valueType: "string", value: "original" }],
          },
        ],
      },
      { key: "other", valueType: "string", value: "untouched" },
    ],
  };

  it("updates a leaf node at depth 1", () => {
    const updated = updateTreeNode(tree, ["section", "enabled"], false);
    const section = updated.children![0];
    expect(section.children![0].value).toBe(false);
  });

  it("updates a leaf node at depth 2", () => {
    const updated = updateTreeNode(tree, ["section", "nested", "deep"], "changed");
    const deep = updated.children![0].children![2].children![0];
    expect(deep.value).toBe("changed");
  });

  it("does not mutate the original tree", () => {
    const updated = updateTreeNode(tree, ["section", "count"], 999);
    expect(tree.children![0].children![1].value).toBe(10);
    expect(updated.children![0].children![1].value).toBe(999);
  });

  it("leaves other branches untouched", () => {
    const updated = updateTreeNode(tree, ["section", "enabled"], false);
    expect(updated.children![1].value).toBe("untouched");
    expect(updated.children![0].children![1].value).toBe(10);
  });

  it("returns tree unchanged for empty path", () => {
    const result = updateTreeNode(tree, [], "nope");
    expect(result).toEqual(tree);
  });

  it("returns tree unchanged for non-existent path", () => {
    const result = updateTreeNode(tree, ["nonexistent", "path"], "nope");
    expect(result.children![0].children![0].value).toBe(true);
    expect(result.children![1].value).toBe("untouched");
  });

  it("returns tree unchanged for node without children", () => {
    const leaf: SvTreeNode = { key: "leaf", valueType: "string", value: "val" };
    const result = updateTreeNode(leaf, ["anything"], "new");
    expect(result).toEqual(leaf);
  });

  it("handles updating to null value", () => {
    const updated = updateTreeNode(tree, ["section", "enabled"], null);
    expect(updated.children![0].children![0].value).toBe(null);
  });

  it("handles updating to numeric value", () => {
    const updated = updateTreeNode(tree, ["section", "enabled"], 42);
    expect(updated.children![0].children![0].value).toBe(42);
  });
});
