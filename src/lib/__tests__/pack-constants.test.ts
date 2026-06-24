import { describe, it, expect } from "vitest";
import {
  TYPE_LABELS,
  TAG_COLORS,
  PACK_TYPE_ACCENT,
  PACK_TYPE_PILL_COLOR,
  PRESET_TAGS,
  PACK_TYPE_DESCRIPTIONS,
  PACK_IDENTITY_VARS,
  packIdentity,
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
    for (const type of ["addon-pack", "build-pack", "roster-pack"] as const) {
      const accent = PACK_TYPE_ACCENT[type];
      expect(accent.border).toBeTruthy();
      expect(accent.bg).toBeTruthy();
      expect(accent.hoverBg).toBeTruthy();
      expect(accent.text).toBeTruthy();
      expect(accent.hoverGlow).toBeTruthy();
    }
  });

  it("uses the theme primary accent for addon-pack", () => {
    expect(PACK_TYPE_ACCENT["addon-pack"].text).toContain("text-primary");
  });

  it("uses the theme sky accent for build-pack", () => {
    expect(PACK_TYPE_ACCENT["build-pack"].text).toContain("accent-sky");
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

describe("packIdentity", () => {
  // Mirror of the (private) PACK_TYPE_IDENTITY_VAR map in pack-constants.ts.
  const TYPE_VAR: Record<string, string> = {
    "addon-pack": "--primary",
    "build-pack": "--accent-sky",
    "roster-pack": "--status-library",
  };

  it("is deterministic for a fixed id", () => {
    const a = packIdentity({ id: "abc123", title: "Spike's Utilities", packType: "addon-pack" });
    const b = packIdentity({ id: "abc123", title: "Spike's Utilities", packType: "addon-pack" });
    expect(a.accentVar).toBe(b.accentVar);
    expect(a.monogram).toBe(b.monogram);
    expect(a.tileStyle).toEqual(b.tileStyle);
  });

  it("always picks an on-brand accent CSS variable", () => {
    for (let i = 0; i < 200; i++) {
      const { accentVar } = packIdentity({ id: `pack-${i}`, title: `Pack ${i}` });
      expect(PACK_IDENTITY_VARS as readonly string[]).toContain(accentVar);
    }
  });

  it("never reuses the pack's own type accent (collision guard)", () => {
    for (const packType of ["addon-pack", "build-pack", "roster-pack"]) {
      for (let i = 0; i < 200; i++) {
        const { accentVar } = packIdentity({
          id: `seed-${packType}-${i}`,
          title: `T${i}`,
          packType,
        });
        expect(accentVar).not.toBe(TYPE_VAR[packType]);
      }
    }
  });

  it("exposes the --pk-glow custom property for the card hover glow", () => {
    const { glowVars } = packIdentity({ id: "x", title: "Trial Necessities" });
    expect((glowVars as unknown as Record<string, string>)["--pk-glow"]).toMatch(/^color-mix\(/);
  });

  it("derives a monogram from the title", () => {
    expect(packIdentity({ id: "1", title: "Spike's Utilities" }).monogram).toBe("SU");
    expect(packIdentity({ id: "2", title: "Spike's Trial Necessities" }).monogram).toBe("SN");
    expect(packIdentity({ id: "3", title: "Lighthouse" }).monogram).toBe("LI");
    expect(packIdentity({ id: "4", title: "  trial  " }).monogram).toBe("TR");
    expect(packIdentity({ id: "5", title: "A" }).monogram).toBe("A");
  });

  it("falls back to '?' when the title yields no letters", () => {
    expect(packIdentity({ id: "6", title: "" }).monogram).toBe("?");
    expect(packIdentity({ id: "7", title: "✨🔥" }).monogram).toBe("?");
  });

  it("distinguishes two same-type packs by accent or monogram", () => {
    const a = packIdentity({ id: "pack-aaa", title: "Spike's Utilities", packType: "addon-pack" });
    const b = packIdentity({
      id: "pack-bbb",
      title: "Spike's Trial Necessities",
      packType: "addon-pack",
    });
    // They must differ on at least one identity axis (color OR letters).
    expect(a.accentVar !== b.accentVar || a.monogram !== b.monogram).toBe(true);
  });
});
