import { describe, it, expect } from "vitest";
import { resolveEffectiveField } from "../sv-widgets";
import type { SvTreeNode, SvSchemaOverlay } from "../../types";

const noOverlay: SvSchemaOverlay = {};
const noChars = new Set<string>();

function resolve(
  node: SvTreeNode,
  opts?: { path?: string[]; overlay?: SvSchemaOverlay; addon?: string; chars?: Set<string> }
) {
  return resolveEffectiveField(
    node,
    opts?.path ?? ["TestAddon", node.key],
    "setting",
    opts?.overlay ?? noOverlay,
    opts?.addon ?? "TestAddon",
    opts?.chars ?? noChars
  );
}

describe("resolveEffectiveField — color edge cases", () => {
  it("rejects color table with 5+ children", () => {
    const field = resolve({
      key: "notColor",
      valueType: "table",
      children: [
        { key: "r", valueType: "number", value: 0.5 },
        { key: "g", valueType: "number", value: 0.5 },
        { key: "b", valueType: "number", value: 0.5 },
        { key: "a", valueType: "number", value: 1.0 },
        { key: "extra", valueType: "number", value: 0.0 },
      ],
    });
    expect(field.widget).toBe("group");
  });

  it("accepts exact boundary values (0 and 1) for color", () => {
    const field = resolve({
      key: "color",
      valueType: "table",
      children: [
        { key: "r", valueType: "number", value: 0 },
        { key: "g", valueType: "number", value: 1 },
        { key: "b", valueType: "number", value: 0 },
      ],
    });
    expect(field.widget).toBe("color");
  });

  it("rejects negative color values", () => {
    const field = resolve({
      key: "notColor",
      valueType: "table",
      children: [
        { key: "r", valueType: "number", value: -0.1 },
        { key: "g", valueType: "number", value: 0.5 },
        { key: "b", valueType: "number", value: 0.5 },
      ],
    });
    expect(field.widget).toBe("group");
  });

  it("rejects color value slightly above 1", () => {
    const field = resolve({
      key: "notColor",
      valueType: "table",
      children: [
        { key: "r", valueType: "number", value: 1.01 },
        { key: "g", valueType: "number", value: 0.5 },
        { key: "b", valueType: "number", value: 0.5 },
      ],
    });
    expect(field.widget).toBe("group");
  });

  it("does not create children for color nodes", () => {
    const field = resolve({
      key: "color",
      valueType: "table",
      children: [
        { key: "r", valueType: "number", value: 0.5 },
        { key: "g", valueType: "number", value: 0.5 },
        { key: "b", valueType: "number", value: 0.5 },
      ],
    });
    expect(field.children).toBeUndefined();
  });
});

describe("resolveEffectiveField — string edge cases", () => {
  it("treats string of exactly 80 chars as single-line", () => {
    const str80 = "a".repeat(80);
    const field = resolve({ key: "text", valueType: "string", value: str80 });
    expect(field.props.multiline).toBe(false);
  });

  it("treats string of exactly 81 chars as multiline", () => {
    const str81 = "a".repeat(81);
    const field = resolve({ key: "text", valueType: "string", value: str81 });
    expect(field.props.multiline).toBe(true);
  });

  it("handles empty string as single-line", () => {
    const field = resolve({ key: "text", valueType: "string", value: "" });
    expect(field.props.multiline).toBe(false);
  });
});

describe("resolveEffectiveField — overlay edge cases", () => {
  it("ignores overlay for wrong addon name", () => {
    const overlay: SvSchemaOverlay = {
      OtherAddon: {
        "OtherAddon\0key": { widget: "slider", props: { min: 0, max: 10 } },
      },
    };
    const field = resolve(
      { key: "key", valueType: "number", value: 5 },
      { overlay, addon: "TestAddon" }
    );
    expect(field.widget).toBe("number");
  });

  it("merges overlay props with inferred props", () => {
    const overlay: SvSchemaOverlay = {
      TestAddon: {
        "TestAddon\0key": { props: { step: 0.1 } },
      },
    };
    const field = resolve({ key: "key", valueType: "number", value: 5 }, { overlay });
    expect(field.widget).toBe("number");
    expect(field.props.step).toBe(0.1);
  });

  it("overlay with only hidden doesn't change widget", () => {
    const overlay: SvSchemaOverlay = {
      TestAddon: {
        "TestAddon\0key": { hidden: true },
      },
    };
    const field = resolve({ key: "key", valueType: "boolean", value: true }, { overlay });
    expect(field.widget).toBe("toggle");
    expect(field.hidden).toBe(true);
  });
});

describe("resolveEffectiveField — nodeId escaping", () => {
  it("escapes null bytes in path segments", () => {
    const field = resolve(
      { key: "key", valueType: "string", value: "val" },
      { path: ["TestAddon", "seg\0ment", "key"] }
    );
    expect(field.nodeId).toBe("TestAddon\0seg\\0ment\0key");
  });

  it("handles path with special characters", () => {
    const field = resolve(
      { key: "$AccountWide", valueType: "table", children: [] },
      { path: ["TestAddon", "$AccountWide"] }
    );
    expect(field.nodeId).toBe("TestAddon\0$AccountWide");
  });
});

describe("resolveEffectiveField — context classification in children", () => {
  it("classifies $AccountWide as account-wide at depth 1", () => {
    // classifyContext only activates at depth 1 (pathSegments.length = 1)
    const node: SvTreeNode = {
      key: "AddonData",
      valueType: "table",
      children: [
        {
          key: "$AccountWide",
          valueType: "table",
          children: [{ key: "setting", valueType: "boolean", value: true }],
        },
      ],
    };
    // path = ["TestAddon"] means children are resolved at depth = 1
    const field = resolve(node, { path: ["TestAddon"] });
    expect(field.children![0]!.context).toBe("account-wide");
  });

  it("classifies known character names as per-character at depth 1", () => {
    const chars = new Set(["Gandalf"]);
    const node: SvTreeNode = {
      key: "AddonData",
      valueType: "table",
      children: [
        {
          key: "Gandalf",
          valueType: "table",
          children: [{ key: "level", valueType: "number", value: 50 }],
        },
      ],
    };
    const field = resolve(node, { path: ["TestAddon"], chars });
    expect(field.children![0]!.context).toBe("per-character");
  });

  it("returns 'setting' context for keys at depth != 1", () => {
    const node: SvTreeNode = {
      key: "root",
      valueType: "table",
      children: [
        {
          key: "$AccountWide",
          valueType: "table",
          children: [{ key: "setting", valueType: "boolean", value: true }],
        },
      ],
    };
    // path = ["TestAddon", "root"] means children are at depth 2 → always "setting"
    const field = resolve(node, { path: ["TestAddon", "root"] });
    expect(field.children![0]!.context).toBe("setting");
  });
});

describe("resolveEffectiveField — table with no children", () => {
  it("treats empty table as group without recursion", () => {
    const field = resolve({ key: "empty", valueType: "table", children: [] });
    expect(field.widget).toBe("group");
    expect(field.children).toHaveLength(0);
  });

  it("treats table with undefined children as raw (no children array)", () => {
    // inferWidget checks `node.children` — undefined doesn't match the table branch
    const field = resolve({ key: "bare", valueType: "table" });
    expect(field.widget).toBe("raw");
    expect(field.confidence).toBe("ambiguous");
  });
});
