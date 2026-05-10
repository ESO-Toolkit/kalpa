import { describe, it, expect } from "vitest";
import {
  TYPE_LABELS,
  TAG_COLORS,
  PACK_TYPE_ACCENT,
  PACK_TYPE_PILL_COLOR,
  PRESET_TAGS,
  PACK_TYPE_DESCRIPTIONS,
} from "../../components/pack-constants";

describe("TYPE_LABELS", () => {
  it("has labels for all pack types", () => {
    expect(TYPE_LABELS["addon-pack"]).toBe("Addon Pack");
    expect(TYPE_LABELS["build-pack"]).toBe("Build Pack");
    expect(TYPE_LABELS["roster-pack"]).toBe("Roster Pack");
  });

  it("returns undefined for unknown types", () => {
    expect(TYPE_LABELS["unknown-pack"]).toBeUndefined();
  });
});

describe("TAG_COLORS", () => {
  it("maps all preset tags to valid colors", () => {
    const validColors = new Set(["gold", "sky", "emerald", "amber", "red", "violet", "muted"]);
    for (const [tag, color] of Object.entries(TAG_COLORS)) {
      expect(validColors.has(color), `${tag} has invalid color: ${color}`).toBe(true);
    }
  });

  it("includes combat role tags", () => {
    expect(TAG_COLORS.healer).toBe("sky");
    expect(TAG_COLORS.dps).toBe("amber");
    expect(TAG_COLORS.tank).toBe("violet");
  });

  it("includes gameplay mode tags", () => {
    expect(TAG_COLORS.pve).toBe("emerald");
    expect(TAG_COLORS.pvp).toBe("red");
  });
});

describe("PACK_TYPE_ACCENT", () => {
  it("has accent styles for all pack types", () => {
    for (const type of ["addon-pack", "build-pack", "roster-pack"]) {
      const accent = PACK_TYPE_ACCENT[type];
      expect(accent).toBeDefined();
      expect(accent.border).toBeTruthy();
      expect(accent.bg).toBeTruthy();
      expect(accent.hoverBg).toBeTruthy();
      expect(accent.text).toBeTruthy();
      expect(accent.hoverGlow).toBeTruthy();
    }
  });

  it("uses ESO gold for addon-pack", () => {
    expect(PACK_TYPE_ACCENT["addon-pack"].text).toContain("#c4a44a");
  });

  it("uses sky for build-pack", () => {
    expect(PACK_TYPE_ACCENT["build-pack"].text).toContain("sky");
  });

  it("uses violet for roster-pack", () => {
    expect(PACK_TYPE_ACCENT["roster-pack"].text).toContain("violet");
  });
});

describe("PACK_TYPE_PILL_COLOR", () => {
  it("maps all types to pill colors", () => {
    expect(PACK_TYPE_PILL_COLOR["addon-pack"]).toBe("gold");
    expect(PACK_TYPE_PILL_COLOR["build-pack"]).toBe("sky");
    expect(PACK_TYPE_PILL_COLOR["roster-pack"]).toBe("violet");
  });
});

describe("PRESET_TAGS", () => {
  it("contains expected gameplay tags", () => {
    expect(PRESET_TAGS).toContain("trial");
    expect(PRESET_TAGS).toContain("pvp");
    expect(PRESET_TAGS).toContain("healer");
    expect(PRESET_TAGS).toContain("tank");
    expect(PRESET_TAGS).toContain("dps");
    expect(PRESET_TAGS).toContain("beginner");
    expect(PRESET_TAGS).toContain("utility");
  });

  it("has exactly 7 tags", () => {
    expect(PRESET_TAGS).toHaveLength(7);
  });

  it("does not include 'favorite' (handled separately)", () => {
    expect(PRESET_TAGS).not.toContain("favorite");
  });
});

describe("PACK_TYPE_DESCRIPTIONS", () => {
  it("has descriptions for all pack types", () => {
    expect(PACK_TYPE_DESCRIPTIONS["addon-pack"]).toBeTruthy();
    expect(PACK_TYPE_DESCRIPTIONS["build-pack"]).toBeTruthy();
    expect(PACK_TYPE_DESCRIPTIONS["roster-pack"]).toBeTruthy();
  });
});
