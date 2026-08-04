import { describe, it, expect } from "vitest";
import { BUILTIN_THEMES, DEFAULT_THEME, FORCED_DEFAULT_VERSION } from "../theme-presets";
import { statusVarsForTheme, themeColorsToVars } from "../theme-apply";
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
    // public/ is not in .prettierignore, so a repo-wide `prettier --write .`
    // adds a trailing comma here (trailingComma: "es5"). Tolerate it — this
    // test guards the VALUES against drift, not the boot script's formatting.
    const baked = JSON.parse(literal.replace(/,(\s*})$/, "$1"));
    expect(baked).toEqual(themeColorsToVars(DEFAULT_THEME.colors));
  });

  it("FORCED_VERSION stays in sync with FORCED_DEFAULT_VERSION", () => {
    const raw = bootSrc.match(/var FORCED_VERSION = (\d+);/)?.[1];
    if (!raw) throw new Error("FORCED_VERSION literal not found in theme-boot.js");
    expect(Number(raw)).toBe(FORCED_DEFAULT_VERSION);
  });
});

/** Read a `var NAME = { "--k": "v", … };` map out of the boot script's source. */
function bootTable(name: string): Record<string, string> {
  const literal = bootSrc.match(new RegExp(`var ${name} = \\{([\\s\\S]*?)\\};`))?.[1];
  if (!literal) throw new Error(`${name} literal not found in theme-boot.js`);
  const table: Record<string, string> = {};
  for (const entry of literal.matchAll(/"(--[a-z0-9-]+)":\s*"([^"]*)"/g)) {
    table[entry[1]!] = entry[2]!;
  }
  return table;
}

function themeById(id: string) {
  const theme = BUILTIN_THEMES.find((t) => t.id === id);
  if (!theme) throw new Error(`preset ${id} not found`);
  return theme;
}

/** Vars statusVarsForTheme derives from the theme's own seeds rather than a table. */
const SEED_DERIVED = ["--brand-gold-readable", "--brand-cyan-readable", "--addon-disabled"];
/** Vars that only exist as aliases of the danger pair, never as table entries. */
const ALIASES = ["--status-error", "--status-error-strong"];

describe("theme-boot.js status tables", () => {
  it("DARK_STATUS_VARS matches theme-apply's dark status output", () => {
    const expected = statusVarsForTheme(themeById("nordic-runestone").colors);
    const table = bootTable("DARK_STATUS_VARS");
    expect(Object.keys(table).sort()).toEqual(
      Object.keys(expected)
        .filter((k) => !ALIASES.includes(k))
        .sort()
    );
    for (const [key, value] of Object.entries(table)) expect(value, key).toBe(expected[key]);
  });

  it("LIGHT_STATUS_VARS matches theme-apply's light status output", () => {
    const expected = statusVarsForTheme(themeById("a11y-paper-white").colors);
    const table = bootTable("LIGHT_STATUS_VARS");
    expect(Object.keys(table).sort()).toEqual(
      Object.keys(expected)
        .filter((k) => !ALIASES.includes(k) && !SEED_DERIVED.includes(k))
        .sort()
    );
    for (const [key, value] of Object.entries(table)) expect(value, key).toBe(expected[key]);
  });
});
