import type { DropdownOptionItem } from "../types";

/**
 * Dropdowns with more than this many options get a search input; smaller
 * ones stay plain selects.
 */
export const DROPDOWN_SEARCH_THRESHOLD = 10;

/**
 * Whether a dropdown with `itemCount` options should show a type-to-filter
 * search input.
 */
export function dropdownSearchEnabled(itemCount: number): boolean {
  return itemCount > DROPDOWN_SEARCH_THRESHOLD;
}

/**
 * Case-insensitive substring filter over dropdown items.
 *
 * An empty (after trimming) query returns `items` as-is. Otherwise an item
 * matches when the trimmed, lowercased query is a substring of its label or
 * of its stringified value. Pure: never mutates `items` or its elements.
 */
export function filterDropdownItems(
  items: DropdownOptionItem[],
  query: string
): DropdownOptionItem[] {
  const trimmed = query.trim();
  if (trimmed === "") return items;

  const needle = trimmed.toLowerCase();
  return items.filter(
    (item) =>
      item.label.toLowerCase().includes(needle) || String(item.value).toLowerCase().includes(needle)
  );
}
