import { describe, it, expect } from "vitest";
import { buildDropdownItems } from "../../components/sv-controls";
import type { EffectiveField } from "../../types";

function field(overrides: Partial<EffectiveField>): EffectiveField {
  return {
    nodeId: "TestAddon\0key",
    path: ["TestAddon", "key"],
    key: "key",
    label: "Key",
    widget: "dropdown",
    confidence: "inferred",
    context: "setting",
    props: {},
    hidden: false,
    readOnly: false,
    value: null,
    ...overrides,
  };
}

describe("buildDropdownItems", () => {
  it("maps plain options to identity items", () => {
    const items = buildDropdownItems(field({ props: { options: ["a", "b", "c"] }, value: "a" }));
    expect(items).toEqual([
      { label: "a", value: "a" },
      { label: "b", value: "b" },
      { label: "c", value: "c" },
    ]);
  });

  it("does not prepend when the current value is already an option", () => {
    const items = buildDropdownItems(field({ props: { options: ["a", "b"] }, value: "b" }));
    expect(items).toEqual([
      { label: "a", value: "a" },
      { label: "b", value: "b" },
    ]);
  });

  it("prepends the current value when it is absent from options", () => {
    const items = buildDropdownItems(field({ props: { options: ["a", "b"] }, value: "z" }));
    expect(items[0]).toEqual({ label: "z", value: "z" });
    expect(items).toHaveLength(3);
  });

  it("matches a numeric current value against optionItems via String()", () => {
    const optionItems = [
      { label: "Small", value: 1 },
      { label: "Large", value: 2 },
    ];
    const items = buildDropdownItems(field({ props: { optionItems }, value: 1 }));
    // No prepend: current numeric value 1 matches String(item.value) "1".
    expect(items).toEqual(optionItems);
  });

  it("prepends a numeric current value not present in optionItems, using itself as label", () => {
    const optionItems = [
      { label: "Small", value: 1 },
      { label: "Large", value: 2 },
    ];
    const items = buildDropdownItems(field({ props: { optionItems }, value: 5 }));
    expect(items[0]).toEqual({ label: "5", value: 5 });
    expect(items).toHaveLength(3);
  });

  it("prefers optionItems over options when both are present", () => {
    const optionItems = [{ label: "Dark", value: "dark" }];
    const items = buildDropdownItems(
      field({ props: { optionItems, options: ["a", "b"] }, value: "dark" })
    );
    expect(items).toEqual(optionItems);
  });

  it("dedupes by stringified value, first occurrence wins", () => {
    const optionItems = [
      { label: "One", value: 1 },
      { label: "One (dup)", value: 1 },
      { label: "Two", value: 2 },
    ];
    const items = buildDropdownItems(field({ props: { optionItems }, value: 1 }));
    expect(items).toEqual([
      { label: "One", value: 1 },
      { label: "Two", value: 2 },
    ]);
  });

  it("handles a null field value by prepending an empty-string item", () => {
    const items = buildDropdownItems(field({ props: { options: ["a"] }, value: null }));
    expect(items[0]).toEqual({ label: "", value: "" });
    expect(items).toContainEqual({ label: "a", value: "a" });
  });

  it("handles empty options and null value", () => {
    const items = buildDropdownItems(field({ props: {}, value: null }));
    expect(items).toEqual([{ label: "", value: "" }]);
  });
});
