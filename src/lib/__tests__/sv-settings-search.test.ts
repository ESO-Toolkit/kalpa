import { describe, it, expect } from "vitest";
import { SETTINGS_SEARCH_LIMIT, searchSvSettings } from "../sv-settings-search";
import type { SvTreeNode } from "../../types";

function leaf(
  key: string,
  valueType: SvTreeNode["valueType"] = "number",
  value: string | number | boolean | null = 0
): SvTreeNode {
  return { key, valueType, value };
}

function table(key: string, children: SvTreeNode[]): SvTreeNode {
  return { key, valueType: "table", children };
}

function root(children: SvTreeNode[]): SvTreeNode {
  return { key: "root", valueType: "table", children };
}

describe("searchSvSettings", () => {
  it("returns empty results and zero total for an empty query", () => {
    const tree = root([table("MyAddon", [leaf("fontSize")])]);
    expect(searchSvSettings(tree, "")).toEqual({ results: [], total: 0 });
  });

  it("returns empty results and zero total for a whitespace-only query", () => {
    const tree = root([table("MyAddon", [leaf("fontSize")])]);
    expect(searchSvSettings(tree, "   ")).toEqual({ results: [], total: 0 });
  });

  it("matches the raw key case-insensitively", () => {
    const fontSizeNode = leaf("fontSize");
    const tree = root([table("MyAddon", [fontSizeNode])]);

    const outcome = searchSvSettings(tree, "FONTS");

    expect(outcome.total).toBe(1);
    expect(outcome.results).toHaveLength(1);
    expect(outcome.results[0]?.node).toBe(fontSizeNode);
    expect(outcome.results[0]?.isColor).toBe(false);
  });

  it("matches the humanized label when the raw key doesn't match", () => {
    const fontSizeNode = leaf("fontSize");
    const showHudNode = leaf("show_hud", "boolean", true);
    const tree = root([table("MyAddon", [fontSizeNode, showHudNode])]);

    expect(searchSvSettings(tree, "font size").results[0]?.node).toBe(fontSizeNode);
    expect(searchSvSettings(tree, "show hud").results[0]?.node).toBe(showHudNode);
  });

  it("produces a root-excluded, fully-qualified path for a deeply nested leaf", () => {
    const volumeNode = leaf("volumeLevel");
    const tree = root([table("MyAddon", [table("audio", [table("sfx", [volumeNode])])])]);

    const outcome = searchSvSettings(tree, "volume");

    expect(outcome.results).toEqual([
      { node: volumeNode, path: ["MyAddon", "audio", "sfx", "volumeLevel"], isColor: false },
    ]);
  });

  it("matches a color-table group as a single unit and never descends into it", () => {
    const barColor = table("barColor", [leaf("r"), leaf("g"), leaf("b")]);
    const tree = root([table("MyAddon", [barColor])]);

    const colorOutcome = searchSvSettings(tree, "color");
    expect(colorOutcome.results).toEqual([
      { node: barColor, path: ["MyAddon", "barColor"], isColor: true },
    ]);

    // A query that would match a channel leaf ("g") must never surface it,
    // because color-table children are never visited.
    const channelOutcome = searchSvSettings(tree, "g");
    expect(channelOutcome.results).toEqual([]);
    expect(channelOutcome.total).toBe(0);
  });

  it("supports 4-channel (rgba) color tables", () => {
    const barColor = table("barColor", [leaf("r"), leaf("g"), leaf("b"), leaf("a")]);
    const tree = root([table("MyAddon", [barColor])]);

    const outcome = searchSvSettings(tree, "bar");
    expect(outcome.results).toEqual([
      { node: barColor, path: ["MyAddon", "barColor"], isColor: true },
    ]);
  });

  it("never returns non-color table groups, even when their key matches, but does return matching leaves inside them", () => {
    const colorModeLeaf = leaf("colorMode", "string", "dark");
    const opacityLeaf = leaf("opacity");
    const colorPanel = table("colorPanel", [colorModeLeaf, opacityLeaf]);
    const tree = root([table("MyAddon", [colorPanel])]);

    const outcome = searchSvSettings(tree, "color");

    expect(outcome.total).toBe(1);
    expect(outcome.results).toEqual([
      { node: colorModeLeaf, path: ["MyAddon", "colorPanel", "colorMode"], isColor: false },
    ]);
  });

  it("caps results at the default limit while still reporting the full total", () => {
    const leaves = Array.from({ length: 60 }, (_, i) => leaf(`setting${i}`));
    const tree = root([table("MyAddon", leaves)]);

    const outcome = searchSvSettings(tree, "setting");

    expect(outcome.results).toHaveLength(SETTINGS_SEARCH_LIMIT);
    expect(outcome.total).toBe(60);
  });

  it("honors a custom limit argument", () => {
    const leaves = Array.from({ length: 60 }, (_, i) => leaf(`setting${i}`));
    const tree = root([table("MyAddon", leaves)]);

    const outcome = searchSvSettings(tree, "setting", 5);

    expect(outcome.results).toHaveLength(5);
    expect(outcome.total).toBe(60);
  });

  it("has a default limit of 50", () => {
    expect(SETTINGS_SEARCH_LIMIT).toBe(50);
  });

  it("walks in document order, so the first-defined match appears first", () => {
    const first = leaf("aaa");
    const second = leaf("aab");
    const tree = root([table("MyAddon", [first, second])]);

    const outcome = searchSvSettings(tree, "aa");

    expect(outcome.results.map((r) => r.node)).toEqual([first, second]);
  });
});
