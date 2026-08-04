import { readdirSync, readFileSync } from "node:fs";
import { extname, join, relative } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Guards the frontend <-> Rust IPC command-name contract.
 *
 * Command names are opaque strings on both sides: `tsc` cannot see them, the
 * shared `invoke` mock resolves any name, and the e2e suite exercises only a
 * handful of read-only flows. A renamed or removed Rust command therefore fails
 * nowhere but a production build. This test reads both sides as text and asserts
 * every name the frontend invokes is registered in `generate_handler![...]`.
 *
 * The reverse direction (registered but never invoked) is deliberately not
 * asserted — dead handlers are a cleanup task, not a runtime break.
 */

const REPO_ROOT = process.cwd();
const SOURCE_ROOT = join(REPO_ROOT, "src");
const LIB_RS = join(REPO_ROOT, "src-tauri", "src", "lib.rs");
const SOURCE_EXTENSIONS = new Set([".ts", ".tsx"]);
const SKIPPED_DIRECTORIES = new Set(["__tests__", "__mocks__"]);

/**
 * Commands dispatched through a variable instead of a literal, which the scanner
 * below cannot see. Keep this list in sync by hand.
 *
 * - `enable_addon` / `disable_addon`: App.tsx `handleToggleDisable` picks one at
 *   call time and passes it as a variable.
 */
const DYNAMIC_COMMANDS = ["enable_addon", "disable_addon"];

/**
 * `invoke` / `invokeResult` / `invokeOrThrow` followed by a quoted command name.
 * The optional type argument can span lines and contain `;` and braces, so it is
 * matched lazily up to the `(` that must follow it.
 */
function invokePattern(): RegExp {
  return /\binvoke(?:Result|OrThrow)?\s*(?:<[\s\S]*?>)?\s*\(\s*(["'])([a-z][a-z0-9_]*)\1/g;
}

/** Drop comments so a command name quoted in prose is not counted as a call. */
function stripComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/[^\n]*/gm, "$1");
}

function collectSourceFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (!SKIPPED_DIRECTORIES.has(entry.name)) collectSourceFiles(full, out);
    } else if (SOURCE_EXTENSIONS.has(extname(entry.name)) && !/\.test\.tsx?$/.test(entry.name)) {
      out.push(full);
    }
  }
  return out;
}

/** Every invoked command name, mapped to the first file that invokes it. */
function collectInvokedCommands(): Map<string, string> {
  const commands = new Map<string, string>();
  for (const file of collectSourceFiles(SOURCE_ROOT)) {
    const source = stripComments(readFileSync(file, "utf8"));
    for (const match of source.matchAll(invokePattern())) {
      const name = match[2];
      if (name && !commands.has(name)) commands.set(name, relative(REPO_ROOT, file));
    }
  }
  return commands;
}

/** Every command name registered in lib.rs's `tauri::generate_handler![...]`. */
function collectRegisteredCommands(): Set<string> {
  const source = readFileSync(LIB_RS, "utf8");
  const macro = source.indexOf("tauri::generate_handler![");
  if (macro === -1) throw new Error("tauri::generate_handler![ not found in src-tauri/src/lib.rs");

  const open = source.indexOf("[", macro);
  let depth = 0;
  let close = -1;
  for (let i = open; i < source.length; i++) {
    if (source[i] === "[") depth++;
    else if (source[i] === "]" && --depth === 0) {
      close = i;
      break;
    }
  }
  if (close === -1) throw new Error("generate_handler![ was never closed in src-tauri/src/lib.rs");

  // Entries look like `commands::name,` or `uploader::commands::name,`, some
  // preceded by a `#[cfg(...)]` attribute — take the identifier before each comma.
  const names = source
    .slice(open + 1, close)
    .split(",")
    .map((entry) => /([A-Za-z_][A-Za-z0-9_]*)\s*$/.exec(entry.trim())?.[1])
    .filter((name): name is string => Boolean(name));

  return new Set(names);
}

const invoked = collectInvokedCommands();
const registered = collectRegisteredCommands();

describe("IPC command-name contract", () => {
  it("finds the frontend's invoke() call sites", () => {
    // A scanner that silently matches nothing would make the contract check
    // below vacuously pass, so assert it saw a realistic surface.
    expect(invoked.size).toBeGreaterThan(80);
    expect([...invoked.keys()]).toEqual(
      expect.arrayContaining(["scan_installed_addons", "update_addon", "uploader_upload_log"])
    );
  });

  it("finds the Rust invoke_handler registrations", () => {
    expect(registered.size).toBeGreaterThan(80);
    expect([...registered]).toEqual(
      expect.arrayContaining(["scan_installed_addons", "update_addon", "uploader_upload_log"])
    );
  });

  it("registers every command the frontend invokes", () => {
    const unregistered = [...invoked.entries()]
      .filter(([name]) => !registered.has(name))
      .map(([name, file]) => `${name} (invoked from ${file})`);

    expect(unregistered).toEqual([]);
  });

  it("registers the commands dispatched through a variable", () => {
    for (const name of DYNAMIC_COMMANDS) {
      expect(registered.has(name), `${name} is not registered in lib.rs`).toBe(true);
    }
  });
});
