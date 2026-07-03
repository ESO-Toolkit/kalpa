import { describe, it, expect } from "vitest";
import {
  DROPDOWN_SEARCH_THRESHOLD,
  dropdownSearchEnabled,
  filterDropdownItems,
} from "../sv-dropdown-filter";
import type { DropdownOptionItem } from "../../types";

describe("filterDropdownItems", () => {
  it("returns the same array reference for an empty query", () => {
    const items: DropdownOptionItem[] = [{ label: "Arial", value: "arial" }];
    expect(filterDropdownItems(items, "")).toBe(items);
  });

  it("returns all items for a whitespace-only query", () => {
    const items: DropdownOptionItem[] = [
      { label: "Arial", value: "arial" },
      { label: "Times", value: "times" },
    ];
    expect(filterDropdownItems(items, "   ")).toEqual(items);
  });

  it("matches labels case-insensitively", () => {
    const items: DropdownOptionItem[] = [
      { label: "Arial Narrow", value: "arial-narrow" },
      { label: "Times New Roman", value: "times" },
    ];
    expect(filterDropdownItems(items, "ARIAL")).toEqual([
      { label: "Arial Narrow", value: "arial-narrow" },
    ]);
  });

  it("matches substrings, not just prefixes", () => {
    const items: DropdownOptionItem[] = [
      { label: "Arial Narrow", value: "arial-narrow" },
      { label: "Times New Roman", value: "times" },
    ];
    expect(filterDropdownItems(items, "nar")).toEqual([
      { label: "Arial Narrow", value: "arial-narrow" },
    ]);
  });

  it("matches against the stringified value when the label doesn't match", () => {
    const items: DropdownOptionItem[] = [
      { label: "Small", value: "size-sm" },
      { label: "Large", value: "size-lg" },
    ];
    expect(filterDropdownItems(items, "size")).toEqual(items);
  });

  it("matches numeric values via their stringified form", () => {
    const items: DropdownOptionItem[] = [
      { label: "Two", value: 2 },
      { label: "Three", value: 3 },
    ];
    expect(filterDropdownItems(items, "2")).toEqual([{ label: "Two", value: 2 }]);
  });

  it("matches boolean values via their stringified form", () => {
    const items: DropdownOptionItem[] = [
      { label: "On", value: true },
      { label: "Off", value: false },
    ];
    expect(filterDropdownItems(items, "tru")).toEqual([{ label: "On", value: true }]);
  });

  it("returns an empty array when nothing matches", () => {
    const items: DropdownOptionItem[] = [{ label: "Arial", value: "arial" }];
    expect(filterDropdownItems(items, "zzz")).toEqual([]);
  });

  it("does not mutate the input array", () => {
    const items: DropdownOptionItem[] = [
      { label: "Arial Narrow", value: "arial-narrow" },
      { label: "Times New Roman", value: "times" },
    ];
    const original = JSON.parse(JSON.stringify(items)) as DropdownOptionItem[];
    filterDropdownItems(items, "arial");
    expect(items).toEqual(original);
  });

  it("trims surrounding whitespace from the query", () => {
    const items: DropdownOptionItem[] = [
      { label: "Arial Narrow", value: "arial-narrow" },
      { label: "Times New Roman", value: "times" },
    ];
    expect(filterDropdownItems(items, "  arial  ")).toEqual([
      { label: "Arial Narrow", value: "arial-narrow" },
    ]);
  });
});

describe("dropdownSearchEnabled", () => {
  it("has a threshold of 10", () => {
    expect(DROPDOWN_SEARCH_THRESHOLD).toBe(10);
  });

  it("is disabled at exactly the threshold", () => {
    expect(dropdownSearchEnabled(10)).toBe(false);
  });

  it("is enabled just above the threshold", () => {
    expect(dropdownSearchEnabled(11)).toBe(true);
  });

  it("is disabled for zero items", () => {
    expect(dropdownSearchEnabled(0)).toBe(false);
  });
});
