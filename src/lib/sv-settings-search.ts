import type { SvTreeNode } from "../types";
import { humanizeKey } from "./sv-nodes";

/**
 * Cap on collected results so a broad query can't render hundreds of rows;
 * the total match count is still reported via `SvSettingsSearchOutcome.total`.
 */
export const SETTINGS_SEARCH_LIMIT = 50;

export interface SvSettingsSearchResult {
  node: SvTreeNode;
  /**
   * Full path from the tree root, root itself excluded — `path[0]` is the
   * top-level addon table key and the last segment is the matched node's key.
   */
  path: string[];
  /** True when the match is a color-table group rather than a leaf setting. */
  isColor: boolean;
}

export interface SvSettingsSearchOutcome {
  results: SvSettingsSearchResult[];
  total: number;
}

function isTable(node: SvTreeNode): boolean {
  return node.valueType === "table" && !!node.children && node.children.length > 0;
}

function isColorTable(node: SvTreeNode): boolean {
  if (!node.children || node.children.length < 3 || node.children.length > 4) return false;
  const keys = new Set(node.children.map((c) => c.key));
  return keys.has("r") && keys.has("g") && keys.has("b");
}

function matchesQuery(node: SvTreeNode, needle: string): boolean {
  return (
    node.key.toLowerCase().includes(needle) || humanizeKey(node.key).toLowerCase().includes(needle)
  );
}

/**
 * Depth-first search over the whole SavedVariables tree for settings whose
 * name matches `query`, anywhere in the tree.
 *
 * - Color-table groups (3-4 children including r/g/b) are matched as a
 *   single unit and never descended into — their channel leaves are never
 *   returned.
 * - Leaves (non-table nodes) are matched individually.
 * - Other table nodes are never returned themselves (group navigation is the
 *   nav tree's job) but are always descended into.
 *
 * Pure: does not mutate `tree`. `total` counts every match in the tree even
 * past `limit`; `results` holds only the first `limit` in walk order.
 */
export function searchSvSettings(
  tree: SvTreeNode,
  query: string,
  limit: number = SETTINGS_SEARCH_LIMIT
): SvSettingsSearchOutcome {
  const needle = query.trim().toLowerCase();
  if (needle === "") return { results: [], total: 0 };

  const results: SvSettingsSearchResult[] = [];
  let total = 0;

  const walk = (node: SvTreeNode, path: string[]): void => {
    if (isTable(node)) {
      if (isColorTable(node)) {
        if (matchesQuery(node, needle)) {
          total++;
          if (results.length < limit) {
            results.push({ node, path, isColor: true });
          }
        }
        // Never descend into a color table.
        return;
      }

      // Other table nodes are never emitted themselves, always descend.
      for (const child of node.children ?? []) {
        walk(child, [...path, child.key]);
      }
      return;
    }

    // Leaf node.
    if (matchesQuery(node, needle)) {
      total++;
      if (results.length < limit) {
        results.push({ node, path, isColor: false });
      }
    }
  };

  for (const child of tree.children ?? []) {
    walk(child, [child.key]);
  }

  return { results, total };
}
