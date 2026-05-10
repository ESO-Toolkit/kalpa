import type { SvTreeNode } from "../types";

export const SYSTEM_SV_NAMES = new Set([
  "ZO_Ingame",
  "ZO_InternalIngame",
  "ZO_Pregame",
  "AccountSettings",
  "GuildHistoryCache",
]);

export function classifyFile(
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

export type SizeCategory = "small" | "medium" | "large";

export function sizeCategory(bytes: number): SizeCategory {
  if (bytes >= 5 * 1024 * 1024) return "large";
  if (bytes >= 1024 * 1024) return "medium";
  return "small";
}

export function updateTreeNode(
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
