import { describe, it, expect } from "vitest";
import { resolveEffectiveField } from "../sv-widgets";
import type { SvTreeNode, SvSchemaOverlay } from "../../types";

const noOverlay: SvSchemaOverlay = {};
const noChars = new Set<string>();

function resolve(
  node: SvTreeNode,
  opts?: { path?: string[]; overlay?: SvSchemaOverlay; addon?: string }
) {
  return resolveEffectiveField(
    node,
    opts?.path ?? ["TestAddon", node.key],
    "setting",
    opts?.overlay ?? noOverlay,
    opts?.addon ?? "TestAddon",
    noChars
  );
}

describe("resolveEffectiveField — type inference", () => {
  it("infers nil as readonly (certain)", () => {
    const field = resolve({ key: "empty", valueType: "nil", value: null });
    expect(field.widget).toBe("readonly");
    expect(field.confidence).toBe("certain");
  });

  it("infers boolean as toggle (certain)", () => {
    const field = resolve({ key: "enabled", valueType: "boolean", value: true });
    expect(field.widget).toBe("toggle");
    expect(field.confidence).toBe("certain");
  });

  it("infers number as number (inferred)", () => {
    const field = resolve({ key: "count", valueType: "number", value: 42 });
    expect(field.widget).toBe("number");
    expect(field.confidence).toBe("inferred");
  });

  it("infers string as text (inferred)", () => {
    const field = resolve({ key: "name", valueType: "string", value: "hello" });
    expect(field.widget).toBe("text");
    expect(field.confidence).toBe("inferred");
    expect(field.props.multiline).toBe(false);
  });

  it("infers long string as multiline text", () => {
    const longStr = "a".repeat(81);
    const field = resolve({ key: "desc", valueType: "string", value: longStr });
    expect(field.widget).toBe("text");
    expect(field.props.multiline).toBe(true);
  });

  it("infers table as group (certain)", () => {
    const field = resolve({
      key: "settings",
      valueType: "table",
      children: [{ key: "a", valueType: "string", value: "val" }],
    });
    expect(field.widget).toBe("group");
    expect(field.confidence).toBe("certain");
    expect(field.children).toHaveLength(1);
  });

  it("falls back to raw for unknown value types", () => {
    const node = { key: "weird", valueType: "function" as SvTreeNode["valueType"] };
    const field = resolve(node);
    expect(field.widget).toBe("raw");
    expect(field.confidence).toBe("ambiguous");
  });
});

describe("resolveEffectiveField — color detection", () => {
  it("detects RGB color table", () => {
    const field = resolve({
      key: "color",
      valueType: "table",
      children: [
        { key: "r", valueType: "number", value: 0.5 },
        { key: "g", valueType: "number", value: 0.3 },
        { key: "b", valueType: "number", value: 0.8 },
      ],
    });
    expect(field.widget).toBe("color");
    expect(field.confidence).toBe("certain");
    expect(field.children).toBeUndefined();
  });

  it("detects RGBA color table", () => {
    const field = resolve({
      key: "color",
      valueType: "table",
      children: [
        { key: "r", valueType: "number", value: 1 },
        { key: "g", valueType: "number", value: 0 },
        { key: "b", valueType: "number", value: 0 },
        { key: "a", valueType: "number", value: 0.5 },
      ],
    });
    expect(field.widget).toBe("color");
  });

  it("rejects color table with out-of-range values", () => {
    const field = resolve({
      key: "notColor",
      valueType: "table",
      children: [
        { key: "r", valueType: "number", value: 255 },
        { key: "g", valueType: "number", value: 0 },
        { key: "b", valueType: "number", value: 0 },
      ],
    });
    expect(field.widget).toBe("group");
  });

  it("rejects color table with wrong keys", () => {
    const field = resolve({
      key: "notColor",
      valueType: "table",
      children: [
        { key: "x", valueType: "number", value: 0.5 },
        { key: "y", valueType: "number", value: 0.5 },
        { key: "z", valueType: "number", value: 0.5 },
      ],
    });
    expect(field.widget).toBe("group");
  });

  it("rejects color table with non-number children", () => {
    const field = resolve({
      key: "notColor",
      valueType: "table",
      children: [
        { key: "r", valueType: "string", value: "red" },
        { key: "g", valueType: "number", value: 0.5 },
        { key: "b", valueType: "number", value: 0.5 },
      ],
    });
    expect(field.widget).toBe("group");
  });

  it("rejects table with too few children for color", () => {
    const field = resolve({
      key: "notColor",
      valueType: "table",
      children: [
        { key: "r", valueType: "number", value: 0.5 },
        { key: "g", valueType: "number", value: 0.5 },
      ],
    });
    expect(field.widget).toBe("group");
  });
});

describe("resolveEffectiveField — overlay overrides", () => {
  it("applies widget override from overlay", () => {
    const overlay: SvSchemaOverlay = {
      TestAddon: {
        "TestAddon\0count": { widget: "slider", props: { min: 0, max: 100 } },
      },
    };
    const field = resolve({ key: "count", valueType: "number", value: 50 }, { overlay });
    expect(field.widget).toBe("slider");
    expect(field.confidence).toBe("certain");
    expect(field.props.min).toBe(0);
    expect(field.props.max).toBe(100);
  });

  it("downgrades slider to number when missing min/max", () => {
    const overlay: SvSchemaOverlay = {
      TestAddon: {
        "TestAddon\0count": { widget: "slider" },
      },
    };
    const field = resolve({ key: "count", valueType: "number", value: 50 }, { overlay });
    expect(field.widget).toBe("number");
  });

  it("applies hidden override", () => {
    const overlay: SvSchemaOverlay = {
      TestAddon: {
        "TestAddon\0secret": { hidden: true },
      },
    };
    const field = resolve({ key: "secret", valueType: "string", value: "hidden" }, { overlay });
    expect(field.hidden).toBe(true);
  });

  it("applies readOnly override", () => {
    const overlay: SvSchemaOverlay = {
      TestAddon: {
        "TestAddon\0locked": { readOnly: true },
      },
    };
    const field = resolve({ key: "locked", valueType: "string", value: "locked" }, { overlay });
    expect(field.readOnly).toBe(true);
  });

  it("applies label override", () => {
    const overlay: SvSchemaOverlay = {
      TestAddon: {
        "TestAddon\0uglyKey": { label: "Pretty Label" },
      },
    };
    const field = resolve({ key: "uglyKey", valueType: "string", value: "val" }, { overlay });
    expect(field.label).toBe("Pretty Label");
  });
});

describe("resolveEffectiveField — metadata", () => {
  it("generates correct nodeId from path segments", () => {
    const field = resolve(
      { key: "setting", valueType: "string", value: "val" },
      { path: ["MyAddon", "section", "setting"] }
    );
    expect(field.nodeId).toBe("MyAddon\0section\0setting");
  });

  it("humanizes key for label when no overlay", () => {
    const field = resolve({ key: "showTooltip", valueType: "boolean", value: true });
    expect(field.label).toBe("Show Tooltip");
  });

  it("preserves value in output", () => {
    const field = resolve({ key: "count", valueType: "number", value: 42 });
    expect(field.value).toBe(42);
  });

  it("defaults hidden and readOnly to false", () => {
    const field = resolve({ key: "x", valueType: "string", value: "y" });
    expect(field.hidden).toBe(false);
    expect(field.readOnly).toBe(false);
  });
});

describe("resolveEffectiveField — recursive children", () => {
  it("recursively resolves table children", () => {
    const node: SvTreeNode = {
      key: "root",
      valueType: "table",
      children: [
        { key: "enabled", valueType: "boolean", value: true },
        { key: "name", valueType: "string", value: "test" },
        {
          key: "nested",
          valueType: "table",
          children: [{ key: "deep", valueType: "number", value: 1 }],
        },
      ],
    };
    const field = resolve(node, { path: ["TestAddon", "root"] });
    expect(field.children).toHaveLength(3);
    expect(field.children![0]!.widget).toBe("toggle");
    expect(field.children![1]!.widget).toBe("text");
    expect(field.children![2]!.widget).toBe("group");
    expect(field.children![2]!.children).toHaveLength(1);
    expect(field.children![2]!.children![0]!.widget).toBe("number");
  });
});
