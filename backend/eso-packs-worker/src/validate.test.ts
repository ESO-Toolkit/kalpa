import { describe, expect, it } from "vitest";
import { validatePack } from "./validate";

function validPack(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    title: "Healer Pack",
    description: "A solid healer setup",
    pack_type: "addon-pack",
    tags: ["healer", "pve"],
    addons: [{ esouiId: 1, name: "Foo" }],
    ...overrides,
  };
}

describe("validatePack — top-level shape", () => {
  it.each([
    ["null", null],
    ["undefined", undefined],
    ["string", "not an object"],
    ["number", 42],
    ["true", true],
  ])("rejects non-object payload (%s)", (_label, value) => {
    const errors = validatePack(value);
    expect(errors).toEqual([{ field: "pack", message: "Pack must be a JSON object" }]);
  });

  it("accepts a fully valid payload", () => {
    expect(validatePack(validPack())).toEqual([]);
  });

  it("an array does not short-circuit (typeof []==='object'); it cascades to field errors", () => {
    const errors = validatePack([]);
    expect(errors.length).toBeGreaterThan(0);
    expect(errors.find((e) => e.field === "pack")).toBeUndefined();
  });
});

describe("validatePack — id (optional on create)", () => {
  it("allows id to be absent", () => {
    expect(validatePack(validPack())).toEqual([]);
  });

  it.each(["", "UPPER", "has spaces", "has_underscores", "has.dots", "a".repeat(101)])(
    "rejects invalid id %j",
    (id) => {
      const errors = validatePack(validPack({ id }));
      expect(errors).toContainEqual(
        expect.objectContaining({ field: "id" })
      );
    }
  );

  it.each(["a", "abc", "abc-123", "a-b-c-d", "a".repeat(100)])(
    "accepts valid id %j",
    (id) => {
      const errors = validatePack(validPack({ id }));
      expect(errors.find((e) => e.field === "id")).toBeUndefined();
    }
  );

  it("rejects non-string id", () => {
    expect(validatePack(validPack({ id: 42 }))).toContainEqual(
      expect.objectContaining({ field: "id" })
    );
  });
});

describe("validatePack — title", () => {
  it("requires title", () => {
    const errors = validatePack(validPack({ title: undefined }));
    expect(errors).toContainEqual(expect.objectContaining({ field: "title" }));
  });

  it("rejects empty title", () => {
    expect(validatePack(validPack({ title: "" }))).toContainEqual(
      expect.objectContaining({ field: "title" })
    );
  });

  it("rejects non-string title", () => {
    expect(validatePack(validPack({ title: 42 }))).toContainEqual(
      expect.objectContaining({ field: "title" })
    );
  });

  it("rejects title over 100 chars", () => {
    expect(validatePack(validPack({ title: "x".repeat(101) }))).toContainEqual(
      expect.objectContaining({ field: "title" })
    );
  });

  it("accepts title at the 100-char boundary", () => {
    const errors = validatePack(validPack({ title: "x".repeat(100) }));
    expect(errors.find((e) => e.field === "title")).toBeUndefined();
  });
});

describe("validatePack — description", () => {
  it("requires description to be a string", () => {
    expect(validatePack(validPack({ description: undefined }))).toContainEqual(
      expect.objectContaining({ field: "description" })
    );
  });

  it("accepts empty description", () => {
    const errors = validatePack(validPack({ description: "" }));
    expect(errors.find((e) => e.field === "description")).toBeUndefined();
  });

  it("rejects description over 1000 chars", () => {
    expect(validatePack(validPack({ description: "x".repeat(1001) }))).toContainEqual(
      expect.objectContaining({ field: "description" })
    );
  });

  it("accepts description at 1000-char boundary", () => {
    const errors = validatePack(validPack({ description: "x".repeat(1000) }));
    expect(errors.find((e) => e.field === "description")).toBeUndefined();
  });
});

describe("validatePack — pack_type", () => {
  it.each(["addon-pack", "build-pack", "roster-pack"])("accepts %j", (t) => {
    const errors = validatePack(validPack({ pack_type: t }));
    expect(errors.find((e) => e.field === "pack_type")).toBeUndefined();
  });

  it.each(["unknown", "", "Addon-Pack", undefined, 42, null])("rejects %j", (t) => {
    expect(validatePack(validPack({ pack_type: t }))).toContainEqual(
      expect.objectContaining({ field: "pack_type" })
    );
  });
});

describe("validatePack — status (optional)", () => {
  it("allows status to be absent", () => {
    const errors = validatePack(validPack());
    expect(errors.find((e) => e.field === "status")).toBeUndefined();
  });

  it.each(["draft", "published"])("accepts %j", (s) => {
    const errors = validatePack(validPack({ status: s }));
    expect(errors.find((e) => e.field === "status")).toBeUndefined();
  });

  it.each(["archived", "", "Draft", 42])("rejects invalid status %j", (s) => {
    expect(validatePack(validPack({ status: s }))).toContainEqual(
      expect.objectContaining({ field: "status" })
    );
  });
});

describe("validatePack — tags", () => {
  it("requires array", () => {
    expect(validatePack(validPack({ tags: "not-an-array" }))).toContainEqual(
      expect.objectContaining({ field: "tags" })
    );
  });

  it("rejects more than 10 tags", () => {
    const tags = Array.from({ length: 11 }, (_, i) => `tag-${i}`);
    expect(validatePack(validPack({ tags }))).toContainEqual(
      expect.objectContaining({ field: "tags" })
    );
  });

  it("accepts exactly 10 tags", () => {
    const tags = Array.from({ length: 10 }, (_, i) => `tag-${i}`);
    const errors = validatePack(validPack({ tags }));
    expect(errors.find((e) => e.field === "tags")).toBeUndefined();
  });

  it("accepts empty tag array", () => {
    expect(validatePack(validPack({ tags: [] }))).toEqual([]);
  });

  it("rejects non-string tag entry", () => {
    expect(validatePack(validPack({ tags: ["ok", 42] }))).toContainEqual(
      expect.objectContaining({ field: "tags[1]" })
    );
  });

  it("rejects empty tag string", () => {
    expect(validatePack(validPack({ tags: [""] }))).toContainEqual(
      expect.objectContaining({ field: "tags[0]" })
    );
  });

  it("rejects tag over 50 chars", () => {
    expect(validatePack(validPack({ tags: ["x".repeat(51)] }))).toContainEqual(
      expect.objectContaining({ field: "tags[0]" })
    );
  });

  it("only reports the first invalid tag (short-circuits)", () => {
    const errors = validatePack(validPack({ tags: ["", "", ""] }));
    const tagErrors = errors.filter((e) => e.field.startsWith("tags["));
    expect(tagErrors).toHaveLength(1);
  });
});

describe("validatePack — addons", () => {
  it("requires array", () => {
    expect(validatePack(validPack({ addons: "nope" }))).toContainEqual(
      expect.objectContaining({ field: "addons" })
    );
  });

  it("accepts empty addon array (validatePack allows it; share validation does not)", () => {
    const errors = validatePack(validPack({ addons: [] }));
    expect(errors.find((e) => e.field === "addons")).toBeUndefined();
  });

  it("rejects more than 200 addons", () => {
    const addons = Array.from({ length: 201 }, (_, i) => ({
      esouiId: i + 1,
      name: `a${i}`,
    }));
    expect(validatePack(validPack({ addons }))).toContainEqual(
      expect.objectContaining({ field: "addons" })
    );
  });

  it("accepts exactly 200 addons", () => {
    const addons = Array.from({ length: 200 }, (_, i) => ({
      esouiId: i + 1,
      name: `a${i}`,
    }));
    const errors = validatePack(validPack({ addons }));
    expect(errors.find((e) => e.field === "addons")).toBeUndefined();
  });

  it.each([
    ["zero", 0],
    ["negative", -1],
    ["float", 1.5],
    ["string", "1"],
    ["NaN", NaN],
    ["missing", undefined],
  ])("rejects esouiId that is %s", (_label, esouiId) => {
    expect(
      validatePack(validPack({ addons: [{ esouiId, name: "ok" }] }))
    ).toContainEqual(expect.objectContaining({ field: "addons[0].esouiId" }));
  });

  it("rejects empty addon name", () => {
    expect(
      validatePack(validPack({ addons: [{ esouiId: 1, name: "" }] }))
    ).toContainEqual(expect.objectContaining({ field: "addons[0].name" }));
  });

  it("rejects missing addon name", () => {
    expect(
      validatePack(validPack({ addons: [{ esouiId: 1 }] }))
    ).toContainEqual(expect.objectContaining({ field: "addons[0].name" }));
  });

  it("reports errors for multiple addons independently", () => {
    const errors = validatePack(
      validPack({
        addons: [
          { esouiId: 1, name: "ok" },
          { esouiId: -1, name: "" },
        ],
      })
    );
    expect(errors).toContainEqual(
      expect.objectContaining({ field: "addons[1].esouiId" })
    );
    expect(errors).toContainEqual(
      expect.objectContaining({ field: "addons[1].name" })
    );
    expect(errors.find((e) => e.field === "addons[0].esouiId")).toBeUndefined();
  });
});

describe("validatePack — multiple errors are accumulated", () => {
  it("reports every problem in one pass (not just the first)", () => {
    const errors = validatePack({
      title: "",
      description: 42,
      pack_type: "bogus",
      tags: "not-array",
      addons: "not-array",
    });
    const fields = errors.map((e) => e.field).sort();
    expect(fields).toEqual(
      ["addons", "description", "pack_type", "tags", "title"].sort()
    );
  });
});
