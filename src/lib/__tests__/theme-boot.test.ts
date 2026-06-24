import { describe, it, expect } from "vitest";
import { DEFAULT_THEME, FORCED_DEFAULT_VERSION } from "../theme-presets";
import { themeColorsToVars } from "../theme-apply";
// Vite `?raw` import — the boot script's source as a string (no Node APIs, so the
// DOM-only app tsconfig still typechecks).
import bootSrc from "../../../public/theme-boot.js?raw";

/**
 * The pre-paint boot script (public/theme-boot.js) duplicates a few constants
 * from the theme bundle because it is a dependency-free classic script that runs
 * before any module loads. These tests guard those copies against drift.
 */
describe("theme-boot.js factory-default fallback", () => {
  it("embeds a DEFAULT_VARS literal", () => {
    expect(bootSrc).toMatch(/var DEFAULT_VARS = \{[\s\S]*?\};/);
  });

  it("matches DEFAULT_THEME's resolved color vars exactly", () => {
    const literal = bootSrc.match(/var DEFAULT_VARS = (\{[\s\S]*?\});/)?.[1];
    if (!literal) throw new Error("DEFAULT_VARS literal not found in theme-boot.js");
    const baked = JSON.parse(literal);
    expect(baked).toEqual(themeColorsToVars(DEFAULT_THEME.colors));
  });

  it("FORCED_VERSION stays in sync with FORCED_DEFAULT_VERSION", () => {
    const raw = bootSrc.match(/var FORCED_VERSION = (\d+);/)?.[1];
    if (!raw) throw new Error("FORCED_VERSION literal not found in theme-boot.js");
    expect(Number(raw)).toBe(FORCED_DEFAULT_VERSION);
  });
});
