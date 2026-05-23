import { describe, it, expect } from "vitest";
import { validatePack } from "../src/validate";

function validPack(overrides: Record<string, unknown> = {}) {
  return {
    title: "My Pack",
    description: "A test pack",
    pack_type: "addon-pack",
    tags: ["pvp"],
    addons: [{ esouiId: 1, name: "AddonA", required: true }],
    ...overrides,
  };
}

describe("validatePack", () => {
  it("accepts a valid pack without id", () => {
    expect(validatePack(validPack())).toEqual([]);
  });

  it("accepts a valid pack with id", () => {
    expect(validatePack(validPack({ id: "my-pack-1" }))).toEqual([]);
  });

  it("rejects non-object input", () => {
    expect(validatePack(null)).toEqual([
      { field: "pack", message: "Pack must be a JSON object" },
    ]);
    expect(validatePack("string")).toEqual([
      { field: "pack", message: "Pack must be a JSON object" },
    ]);
    expect(validatePack(42)).toEqual([
      { field: "pack", message: "Pack must be a JSON object" },
    ]);
  });

  describe("id", () => {
    it("allows omitted id", () => {
      const errors = validatePack(validPack());
      expect(errors.find((e) => e.field === "id")).toBeUndefined();
    });

    it("rejects empty id", () => {
      const errors = validatePack(validPack({ id: "" }));
      expect(errors).toContainEqual(
        expect.objectContaining({ field: "id" }),
      );
    });

    it("rejects id with uppercase", () => {
      const errors = validatePack(validPack({ id: "MyPack" }));
      expect(errors).toContainEqual(
        expect.objectContaining({ field: "id" }),
      );
    });

    it("rejects id over 100 chars", () => {
      const errors = validatePack(validPack({ id: "a".repeat(101) }));
      expect(errors).toContainEqual(
        expect.objectContaining({ field: "id" }),
      );
    });

    it("accepts max-length id", () => {
      const errors = validatePack(validPack({ id: "a".repeat(100) }));
      expect(errors.find((e) => e.field === "id")).toBeUndefined();
    });
  });

  describe("title", () => {
    it("rejects missing title", () => {
      const pack = validPack();
      delete (pack as Record<string, unknown>).title;
      expect(validatePack(pack)).toContainEqual(
        expect.objectContaining({ field: "title" }),
      );
    });

    it("rejects empty title", () => {
      expect(validatePack(validPack({ title: "" }))).toContainEqual(
        expect.objectContaining({ field: "title" }),
      );
    });

    it("rejects title over 100 chars", () => {
      expect(
        validatePack(validPack({ title: "x".repeat(101) })),
      ).toContainEqual(expect.objectContaining({ field: "title" }));
    });

    it("accepts 100-char title", () => {
      const errors = validatePack(validPack({ title: "x".repeat(100) }));
      expect(errors.find((e) => e.field === "title")).toBeUndefined();
    });
  });

  describe("description", () => {
    it("rejects non-string description", () => {
      expect(
        validatePack(validPack({ description: 42 })),
      ).toContainEqual(
        expect.objectContaining({ field: "description" }),
      );
    });

    it("rejects description over 1000 chars", () => {
      expect(
        validatePack(validPack({ description: "x".repeat(1001) })),
      ).toContainEqual(
        expect.objectContaining({ field: "description" }),
      );
    });

    it("accepts empty description", () => {
      const errors = validatePack(validPack({ description: "" }));
      expect(errors.find((e) => e.field === "description")).toBeUndefined();
    });
  });

  describe("pack_type", () => {
    it.each(["addon-pack", "build-pack", "roster-pack"])(
      "accepts %s",
      (type) => {
        const errors = validatePack(validPack({ pack_type: type }));
        expect(errors.find((e) => e.field === "pack_type")).toBeUndefined();
      },
    );

    it("rejects invalid type", () => {
      expect(
        validatePack(validPack({ pack_type: "invalid" })),
      ).toContainEqual(
        expect.objectContaining({ field: "pack_type" }),
      );
    });
  });

  describe("status", () => {
    it("allows omitted status", () => {
      const errors = validatePack(validPack());
      expect(errors.find((e) => e.field === "status")).toBeUndefined();
    });

    it.each(["draft", "published"])("accepts %s", (status) => {
      const errors = validatePack(validPack({ status }));
      expect(errors.find((e) => e.field === "status")).toBeUndefined();
    });

    it("rejects invalid status", () => {
      expect(
        validatePack(validPack({ status: "archived" })),
      ).toContainEqual(
        expect.objectContaining({ field: "status" }),
      );
    });
  });

  describe("tags", () => {
    it("rejects non-array tags", () => {
      expect(validatePack(validPack({ tags: "pvp" }))).toContainEqual(
        expect.objectContaining({ field: "tags" }),
      );
    });

    it("accepts empty tags array", () => {
      const errors = validatePack(validPack({ tags: [] }));
      expect(errors.find((e) => e.field === "tags")).toBeUndefined();
    });

    it("rejects more than 10 tags", () => {
      const tags = Array.from({ length: 11 }, (_, i) => `tag${i}`);
      expect(validatePack(validPack({ tags }))).toContainEqual(
        expect.objectContaining({ field: "tags" }),
      );
    });

    it("accepts exactly 10 tags", () => {
      const tags = Array.from({ length: 10 }, (_, i) => `tag${i}`);
      const errors = validatePack(validPack({ tags }));
      expect(errors.find((e) => e.field === "tags")).toBeUndefined();
    });

    it("rejects empty string tag", () => {
      expect(validatePack(validPack({ tags: [""] }))).toContainEqual(
        expect.objectContaining({ field: "tags[0]" }),
      );
    });

    it("rejects tag over 50 chars", () => {
      expect(
        validatePack(validPack({ tags: ["x".repeat(51)] })),
      ).toContainEqual(
        expect.objectContaining({ field: "tags[0]" }),
      );
    });
  });

  describe("addons", () => {
    it("rejects non-array addons", () => {
      expect(validatePack(validPack({ addons: "not-array" }))).toContainEqual(
        expect.objectContaining({ field: "addons" }),
      );
    });

    it("rejects empty addons array", () => {
      expect(validatePack(validPack({ addons: [] }))).toContainEqual(
        expect.objectContaining({ field: "addons" }),
      );
    });

    it("rejects more than 200 addons", () => {
      const addons = Array.from({ length: 201 }, (_, i) => ({
        esouiId: i + 1,
        name: `Addon${i}`,
        required: true,
      }));
      expect(validatePack(validPack({ addons }))).toContainEqual(
        expect.objectContaining({ field: "addons" }),
      );
    });

    it("rejects addon with non-integer esouiId", () => {
      expect(
        validatePack(
          validPack({ addons: [{ esouiId: 1.5, name: "A", required: true }] }),
        ),
      ).toContainEqual(
        expect.objectContaining({ field: "addons[0].esouiId" }),
      );
    });

    it("rejects addon with zero esouiId", () => {
      expect(
        validatePack(
          validPack({ addons: [{ esouiId: 0, name: "A", required: true }] }),
        ),
      ).toContainEqual(
        expect.objectContaining({ field: "addons[0].esouiId" }),
      );
    });

    it("rejects addon with negative esouiId", () => {
      expect(
        validatePack(
          validPack({
            addons: [{ esouiId: -1, name: "A", required: true }],
          }),
        ),
      ).toContainEqual(
        expect.objectContaining({ field: "addons[0].esouiId" }),
      );
    });

    it("rejects addon with empty name", () => {
      expect(
        validatePack(
          validPack({ addons: [{ esouiId: 1, name: "", required: true }] }),
        ),
      ).toContainEqual(
        expect.objectContaining({ field: "addons[0].name" }),
      );
    });

    it("validates multiple addons", () => {
      const errors = validatePack(
        validPack({
          addons: [
            { esouiId: 1, name: "Good", required: true },
            { esouiId: -1, name: "", required: true },
          ],
        }),
      );
      expect(errors).toContainEqual(
        expect.objectContaining({ field: "addons[1].esouiId" }),
      );
      expect(errors).toContainEqual(
        expect.objectContaining({ field: "addons[1].name" }),
      );
    });
  });

  it("collects multiple errors at once", () => {
    const errors = validatePack({
      title: "",
      description: 42,
      pack_type: "invalid",
      tags: "not-array",
      addons: "not-array",
    });
    expect(errors.length).toBeGreaterThanOrEqual(5);
  });
});
