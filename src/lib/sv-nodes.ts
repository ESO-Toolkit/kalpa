import type { SvTreeNode, NodeContext } from "../types";

/**
 * Build a stable path ID for a node in the SV tree.
 *
 * Format: `addonName/scope/keyPath`
 *
 * Examples:
 *   LibHistoire/$AccountWide/guilds/1/lastScannedId
 *   Srendarr/@username/CharName^NA/profiles/Default/bars/bar1/enabled
 */
export function buildNodeId(addonName: string, pathSegments: string[]): string {
  return [addonName, ...pathSegments].join("/");
}

/**
 * Classify a second-level key into a context category.
 *
 * - `$AccountWide` (literal ESO key) → account-wide
 * - Keys containing `@` (e.g. `@username`) → account-wide (account-level context)
 * - Keys matching a known character name → per-character
 * - Everything else → setting
 */
export function classifyContext(
  key: string,
  depth: number,
  knownCharacters: Set<string>
): NodeContext {
  // Context classification only applies at the second level (depth 1)
  if (depth !== 1) return "setting";

  if (key === "$AccountWide") return "account-wide";
  if (key.includes("@")) return "account-wide";
  if (knownCharacters.has(key)) return "per-character";

  return "setting";
}

/**
 * Humanize a raw Lua key into a readable label.
 *
 * - camelCase / PascalCase → separate words
 * - snake_case → separate words
 * - Capitalize first letter
 */
export function humanizeKey(key: string): string {
  return (
    key
      // Insert space before uppercase letters in camelCase/PascalCase
      .replace(/([a-z])([A-Z])/g, "$1 $2")
      // Replace underscores/hyphens with spaces
      .replace(/[_-]/g, " ")
      // Capitalize first letter
      .replace(/^\w/, (c) => c.toUpperCase())
      // Collapse multiple spaces
      .replace(/\s+/g, " ")
      .trim()
  );
}

/**
 * Count leaf (non-table) children of a node recursively.
 */
export function countEntries(node: SvTreeNode): number {
  if (!node.children) return 0;
  return node.children.reduce((acc, child) => {
    if (child.valueType === "table" && child.children) {
      return acc + countEntries(child);
    }
    return acc + 1;
  }, 0);
}

/**
 * Collect all navigable (table) children of a node — used for the nav tree.
 */
export function getTableChildren(node: SvTreeNode): SvTreeNode[] {
  return (node.children ?? []).filter(
    (c) => c.valueType === "table" && c.children && c.children.length > 0
  );
}

/**
 * Collect all leaf (non-table) children of a node — used for the form panel.
 */
export function getLeafChildren(node: SvTreeNode): SvTreeNode[] {
  return (node.children ?? []).filter(
    (c) => c.valueType !== "table" || !c.children || c.children.length === 0
  );
}
