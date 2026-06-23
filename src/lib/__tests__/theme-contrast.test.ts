import { describe, it, expect } from "vitest";
import { BUILTIN_THEMES } from "../theme-presets";
import { evaluateContrast } from "../theme-contrast";
import { relativeLuminance, contrastRatio, hexToRgb, mix } from "../theme-color";
import { THEME_COLOR_KEYS } from "../theme-types";

describe("built-in themes", () => {
  it("have unique ids", () => {
    const ids = BUILTIN_THEMES.map((t) => t.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("define all 12 seed colors as valid 6-digit hex", () => {
    for (const theme of BUILTIN_THEMES) {
      for (const key of THEME_COLOR_KEYS) {
        expect(theme.colors[key], `${theme.id}.${key}`).toMatch(/^#[0-9a-fA-F]{6}$/);
      }
    }
  });

  it("are genuinely dark (the component layer assumes dark surfaces)", () => {
    for (const theme of BUILTIN_THEMES) {
      // bgBase / background / surface must be low-luminance.
      expect(relativeLuminance(theme.colors.bgBase), `${theme.id} bgBase`).toBeLessThan(0.12);
      expect(relativeLuminance(theme.colors.background), `${theme.id} background`).toBeLessThan(
        0.12
      );
      expect(relativeLuminance(theme.colors.surface), `${theme.id} surface`).toBeLessThan(0.16);
    }
  });

  it("meet WCAG 2 contrast minimums on every checked pair", () => {
    for (const theme of BUILTIN_THEMES) {
      for (const check of evaluateContrast(theme.colors)) {
        expect(
          check.level,
          `${theme.id} — ${check.label} = ${check.ratio}:1 (needs ${check.min}:1)`
        ).not.toBe("fail");
      }
    }
  });
});

describe("theme-color utilities", () => {
  it("parses 3- and 6-digit hex", () => {
    expect(hexToRgb("#fff")).toEqual({ r: 255, g: 255, b: 255 });
    expect(hexToRgb("#000000")).toEqual({ r: 0, g: 0, b: 0 });
    expect(hexToRgb("38bdf8")).toEqual({ r: 56, g: 189, b: 248 });
  });

  it("computes a 21:1 ratio for black vs white", () => {
    expect(Math.round(contrastRatio("#000000", "#ffffff"))).toBe(21);
  });

  it("mixes endpoints correctly", () => {
    expect(mix("#000000", "#ffffff", 0)).toBe("#000000");
    expect(mix("#000000", "#ffffff", 1)).toBe("#ffffff");
    expect(mix("#000000", "#ffffff", 0.5)).toBe("#808080");
  });
});
