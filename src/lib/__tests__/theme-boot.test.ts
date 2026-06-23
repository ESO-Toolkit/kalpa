import { describe, it, expect } from "vitest";
import { DEFAULT_THEME } from "../theme-presets";
import { themeColorsToVars } from "../theme-apply";
// Vite `?raw` import — the boot script's source as a string (no Node APIs, so the
// DOM-only app tsconfig still typechecks).
import bootSrc from "../../../public/theme-boot.js?raw";

/**
 * The pre-paint boot script (public/theme-boot.js) hardcodes the factory
 * default's resolved color vars so a fresh install paints the right theme before
 * the JS bundle loads. It cannot import from the bundle (it's a dependency-free
 * classic script), so this test guards that copy against drift from
 * DEFAULT_THEME.
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
});
