import { describe, it, expect } from "vitest";
import { classifyContext, humanizeKey, getTableChildren, getLeafChildren } from "../sv-nodes";
import type { SvTreeNode } from "../../types";

describe("classifyContext", () => {
  const knownChars = new Set(["Gandalf", "Aragorn"]);

  it("returns 'setting' for non-depth-1 keys", () => {
    expect(classifyContext("$AccountWide", 0, knownChars)).toBe("setting");
    expect(classifyContext("$AccountWide", 2, knownChars)).toBe("setting");
  });

  it("returns 'account-wide' for $AccountWide at depth 1", () => {
    expect(classifyContext("$AccountWide", 1, knownChars)).toBe("account-wide");
  });

  it("returns 'account-wide' for keys containing @", () => {
    expect(classifyContext("@username", 1, knownChars)).toBe("account-wide");
    expect(classifyContext("data@server", 1, knownChars)).toBe("account-wide");
  });

  it("returns 'per-character' for known character names", () => {
    expect(classifyContext("Gandalf", 1, knownChars)).toBe("per-character");
    expect(classifyContext("Aragorn", 1, knownChars)).toBe("per-character");
  });

  it("returns 'setting' for unknown keys at depth 1", () => {
    expect(classifyContext("unknownKey", 1, knownChars)).toBe("setting");
  });
});

describe("humanizeKey", () => {
  it("splits camelCase", () => {
    expect(humanizeKey("camelCase")).toBe("Camel Case");
  });

  it("splits PascalCase", () => {
    expect(humanizeKey("PascalCase")).toBe("Pascal Case");
  });

  it("splits snake_case", () => {
    expect(humanizeKey("snake_case")).toBe("Snake case");
  });

  it("splits kebab-case", () => {
    expect(humanizeKey("kebab-case")).toBe("Kebab case");
  });

  it("capitalizes first letter", () => {
    expect(humanizeKey("test")).toBe("Test");
  });

  it("handles mixed camelCase and underscores", () => {
    expect(humanizeKey("myFavorite_setting")).toBe("My Favorite setting");
  });

  it("collapses multiple spaces", () => {
    expect(humanizeKey("some__double__underscored")).toBe("Some double underscored");
  });

  it("trims whitespace", () => {
    // trim happens after capitalize, so leading space means ^\w doesn't match
    expect(humanizeKey("  padded  ")).toBe("padded");
  });
});

describe("getTableChildren", () => {
  it("returns only table children with non-empty children", () => {
    const node: SvTreeNode = {
      key: "root",
      valueType: "table",
      children: [
        {
          key: "subtable",
          valueType: "table",
          children: [{ key: "leaf", valueType: "string", value: "hello" }],
        },
        { key: "scalar", valueType: "string", value: "world" },
        { key: "emptyTable", valueType: "table", children: [] },
      ],
    };
    const result = getTableChildren(node);
    expect(result).toHaveLength(1);
    expect(result[0]!.key).toBe("subtable");
  });

  it("returns empty array for node without children", () => {
    const node: SvTreeNode = { key: "root", valueType: "table" };
    expect(getTableChildren(node)).toEqual([]);
  });
});

describe("getLeafChildren", () => {
  it("returns non-table children and empty tables", () => {
    const node: SvTreeNode = {
      key: "root",
      valueType: "table",
      children: [
        {
          key: "subtable",
          valueType: "table",
          children: [{ key: "leaf", valueType: "string", value: "hello" }],
        },
        { key: "str", valueType: "string", value: "world" },
        { key: "num", valueType: "number", value: 42 },
        { key: "emptyTable", valueType: "table", children: [] },
      ],
    };
    const result = getLeafChildren(node);
    expect(result).toHaveLength(3);
    expect(result.map((c) => c.key)).toEqual(["str", "num", "emptyTable"]);
  });

  it("returns empty array for node without children", () => {
    const node: SvTreeNode = { key: "root", valueType: "table" };
    expect(getLeafChildren(node)).toEqual([]);
  });
});
