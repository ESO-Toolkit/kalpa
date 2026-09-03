import { readdirSync, readFileSync } from "node:fs";
import { extname, join, relative } from "node:path";
import { describe, expect, it } from "vitest";

type Finding = {
  file: string;
  line: number;
  expression: string;
};

const SOURCE_ROOT = join(process.cwd(), "src");
const SOURCE_EXTENSIONS = new Set([".ts", ".tsx"]);

// Matches `nativeButton={...}` and captures the expression. A bare `nativeButton`
// (implicit `true`) is not matched, and neither is `nativeButton={true}` /
// `{false}` - those are the only three forms that are safe to write, because the
// component author picked the value against the element they actually render.
const NATIVE_BUTTON_BINDING_PATTERN = /\bnativeButton=\{([^}]*)\}/g;
const LITERAL_BINDING = /^\s*(?:true|false)\s*$/;

// Deliberately empty. If a wrapper ever becomes genuinely polymorphic - it
// forwards BOTH `render` and `nativeButton`, so the caller chooses the element
// and the flag together - add it here with a comment saying so.
const ALLOWLIST: Record<string, string> = {};

// Scope note: this catches one direction of the mismatch - a fixed render
// element with a caller-supplied `nativeButton`. It cannot catch the reverse
// (a non-<button> `render` with `nativeButton` left at a wrong literal), which
// needs the rendered element and so only shows up as a Base UI dev warning in
// the console. Base UI emits both warnings; keep the console clean.

describe("nativeButton prop ratchet", () => {
  it("never forwards nativeButton from a variable", () => {
    const findings = findNativeButtonBindings();
    const seenAllowlistKeys = new Set<string>();
    const unallowlisted = findings.filter((finding) => {
      const key = allowlistKey(finding);
      if (key in ALLOWLIST) {
        seenAllowlistKeys.add(key);
        return false;
      }
      return true;
    });
    const staleAllowlistKeys = Object.keys(ALLOWLIST).filter((key) => !seenAllowlistKeys.has(key));

    expect(formatFailures(unallowlisted, staleAllowlistKeys)).toBe("");
  });
});

function findNativeButtonBindings(): Finding[] {
  return walkSourceFiles(SOURCE_ROOT).flatMap((filePath) => {
    const file = relative(process.cwd(), filePath).replace(/\\/g, "/");
    const lines = readFileSync(filePath, "utf8").split(/\r?\n/);

    return lines.flatMap((line, index) =>
      Array.from(line.matchAll(NATIVE_BUTTON_BINDING_PATTERN)).flatMap((match) => {
        const expression = match[1] ?? "";
        if (LITERAL_BINDING.test(expression)) return [];
        return [{ file, line: index + 1, expression: expression.trim() }];
      })
    );
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
      `${file}:${line} forwards \`nativeButton={${expression}}\`. Base UI uses this flag to decide whether to layer non-native button semantics (\`role="button"\`, \`aria-disabled\`, its own key handlers, and a relocated \`id\`) onto the element. A wrapper that hard-codes its \`render\` element knows which value is correct, so it must pass the literal; forwarding lets a caller - or an omitted prop defaulting to \`undefined\` - silently pick the wrong one and double up the semantics on a real <button>. Pass \`nativeButton\` or \`nativeButton={false}\` to match the element you render, and omit the prop from your public props type.`
  );
  const staleMessages = staleAllowlistKeys.map(
    (key) => `${key} is allowlisted but no longer exists. Remove the stale ALLOWLIST entry.`
  );

  return [...unallowlistedMessages, ...staleMessages].join("\n");
}
