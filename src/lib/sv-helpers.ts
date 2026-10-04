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

function valueTypeOf(value: string | number | boolean | null): SvTreeNode["valueType"] {
  if (value === null) return "nil";
  if (typeof value === "boolean") return "boolean";
  if (typeof value === "number") return "number";
  return "string";
}

// Paths remain strings for persisted schema compatibility. A reserved prefix
// distinguishes numeric Lua keys; literal keys with that prefix are escaped.
export function treePathSegment(node: SvTreeNode): string {
  if (node.keyType === "number") return "\0number:" + node.key;
  return node.key.startsWith("\0") ? "\0string:" + node.key : node.key;
}

export function treePathKey(segment: string): string {
  if (segment.startsWith("\0number:")) return segment.slice(8);
  if (segment.startsWith("\0string:")) return segment.slice(8);
  return segment;
}

export function treePathId(path: string[]): string {
  // Preserve existing overlay IDs for ordinary paths. JSON encoding avoids
  // collisions between reserved segments and literal backslash/NUL keys.
  return path.some((segment) => segment === "" || /[\\\0]/.test(segment))
    ? "\0path:" + JSON.stringify(path)
    : path.join("\0");
}

export function findTreeChild(node: SvTreeNode | null, segment: string): SvTreeNode | null {
  return node?.children?.find((child) => treePathSegment(child) === segment) ?? null;
}

export function updateTreeNode(
  tree: SvTreeNode,
  path: string[],
  value: string | number | boolean | null,
  depth = 0
): SvTreeNode {
  if (depth >= path.length || !tree.children) return tree;

  const targetIndex = tree.children.findIndex((child) => treePathSegment(child) === path[depth]);
  if (targetIndex < 0) return tree;
  const isLeaf = depth === path.length - 1;

  return {
    ...tree,
    children: tree.children.map((child, index) => {
      if (index !== targetIndex) return child;
      if (isLeaf) {
        // The user replaced the value, so re-derive the leaf's valueType from
        // the new value and drop any rawLuaValue (which would otherwise take
        // precedence in the Rust serializer and silently discard the edit).
        return {
          ...child,
          value: value,
          valueType: valueTypeOf(value),
          rawLuaValue: undefined,
        };
      }
      return updateTreeNode(child, path, value, depth + 1);
    }),
  };
}
