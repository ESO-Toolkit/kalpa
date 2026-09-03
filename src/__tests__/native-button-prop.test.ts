import { readdirSync, readFileSync } from "node:fs";
import { extname, join, relative } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Base UI uses `nativeButton` to decide whether to layer non-native button
 * semantics onto whatever element a component renders. Get it wrong and you get
 * `aria-disabled` plus `tabIndex=-1` instead of a real `disabled` attribute,
 * Base UI's own Space/Enter handlers duplicating the browser's, and — on
 * Checkbox specifically — the `id` relocated onto the hidden input.
 *
 * That is exactly what shipped: `primitives/base/checkbox.tsx` hard-codes
 * `render={<motion.button>}` but forwarded `nativeButton` from its own props, so
 * with no caller supplying it Base UI received `undefined` and fell back to its
 * default. Checkbox is the one Base UI component in this tree whose default is
 * `false` (its default element is a `<span>`), so the mismatch was silent apart
 * from a dev warning.
 *
 * A wrapper that fixes its `render` element knows which value is correct, so it
 * must pass a literal. Forwarding hands that choice to a caller who cannot see
 * the element — or, worse, to an omitted prop.
 *
 * SCOPE: this only sees *explicit* `nativeButton={expr}` JSX bindings. It does
 * not see props forwarded by spread, which is how the dialog and popover
 * wrappers pass theirs through — those are genuinely polymorphic (they do not
 * fix `render`), so the caller choosing the value is correct there. Nor can it
 * catch the reverse mismatch (a non-`<button>` `render` with `nativeButton`
 * true), which needs the rendered element and only surfaces as a Base UI dev
 * warning in the console.
 *
 * Split into three assertions for the same reason as `source-hygiene.test.ts`:
 * a guard like this can stop detecting and it can stop looking, and neither
 * failure is visible from a green result.
 */

type Finding = {
  file: string;
  line: number;
  expression: string;
};

const SOURCE_ROOT = join(process.cwd(), "src");
const SOURCE_EXTENSIONS = new Set([".ts", ".tsx"]);

/**
 * Matches `nativeButton={...}` and captures the expression. Runs against whole
 * file text rather than per line, because Prettier wraps long attributes onto
 * their own lines and a line-based scan would miss those.
 */
const NATIVE_BUTTON_BINDING_PATTERN = /\bnativeButton=\{([^}]*)\}/g;
const LITERAL_BINDING = /^\s*(?:true|false)\s*$/;

/**
 * Deliberately empty. If a wrapper ever needs to forward `nativeButton` through
 * an explicit binding — it also forwards `render`, so the caller picks the
 * element and the flag together — add it here with a comment saying so.
 */
const ALLOWLIST: Record<string, string> = {};

describe("nativeButton prop ratchet", () => {
  it("detects a forwarded binding", () => {
    const findings = findNativeButtonBindings(
      "src/example.tsx",
      ["<Root nativeButton={nativeButton} />", "<Root nativeButton={cond ? a : b} />"].join("\n")
    );

    expect(findings.map((finding) => finding.expression)).toEqual(["nativeButton", "cond ? a : b"]);
  });

  it("allows the literal and bare forms, across line breaks", () => {
    const source = [
      "<Root nativeButton />",
      "<Root nativeButton={true} />",
      "<Root nativeButton={false} />",
      "<Root",
      "  nativeButton={",
      "    someExpression",
      "  }",
      "/>",
    ].join("\n");

    // The three safe forms produce nothing; the wrapped binding is still caught.
    expect(findNativeButtonBindings("src/example.tsx", source).map((f) => f.expression)).toEqual([
      "someExpression",
    ]);
  });

  it("scans a non-empty set of source files", () => {
    expect(walkSourceFiles(SOURCE_ROOT).length).toBeGreaterThan(100);
  });

  it("never forwards nativeButton through an explicit JSX binding", () => {
    const findings = scanRepository();
    const seenAllowlistKeys = new Set<string>();
    const unallowlisted = findings.filter((finding) => {
      const key = allowlistKey(finding);
      if (Object.prototype.hasOwnProperty.call(ALLOWLIST, key)) {
        seenAllowlistKeys.add(key);
        return false;
      }
      return true;
    });
    const staleAllowlistKeys = Object.keys(ALLOWLIST).filter((key) => !seenAllowlistKeys.has(key));

    expect(formatFailures(unallowlisted, staleAllowlistKeys)).toBe("");
  });
});

function scanRepository(): Finding[] {
  return walkSourceFiles(SOURCE_ROOT).flatMap((filePath) => {
    const file = relative(process.cwd(), filePath).replace(/\\/g, "/");
    return findNativeButtonBindings(file, readFileSync(filePath, "utf8"));
  });
}

function findNativeButtonBindings(file: string, source: string): Finding[] {
  return Array.from(source.matchAll(NATIVE_BUTTON_BINDING_PATTERN)).flatMap((match) => {
    const expression = match[1] ?? "";
    if (LITERAL_BINDING.test(expression)) return [];
    return [
      {
        file,
        line: source.slice(0, match.index).split("\n").length,
        expression: expression.trim().replace(/\s+/g, " "),
      },
    ];
  });
}

function walkSourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const fullPath = join(dir, entry.name);

    if (entry.isDirectory()) {
      return entry.name === "__tests__" ? [] : walkSourceFiles(fullPath);
    }

    return SOURCE_EXTENSIONS.has(extname(entry.name)) ? [fullPath] : [];
  });
}

function allowlistKey({ file, expression }: Pick<Finding, "file" | "expression">): string {
  return `${file}|${expression}`;
}

function formatFailures(unallowlisted: Finding[], staleAllowlistKeys: string[]): string {
  const unallowlistedMessages = unallowlisted.map(
    ({ file, line, expression }) =>
      `${file}:${line} forwards \`nativeButton={${expression}}\`. Base UI uses this flag to decide whether to layer non-native button semantics onto the element — \`aria-disabled\` and \`tabIndex=-1\` in place of a real \`disabled\` attribute, its own Space/Enter handlers, and a relocated \`id\`. A wrapper that hard-codes its \`render\` element knows which value is correct, so it must pass the literal; forwarding lets a caller — or an omitted prop defaulting to \`undefined\` — silently pick the wrong one. Pass \`nativeButton\` or \`nativeButton={false}\` to match the element you render, and omit the prop from your public props type.`
  );
  const staleMessages = staleAllowlistKeys.map(
    (key) => `${key} is allowlisted but no longer exists. Remove the stale ALLOWLIST entry.`
  );

  return [...unallowlistedMessages, ...staleMessages].join("\n");
}
