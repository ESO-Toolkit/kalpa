import { readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

/**
 * A single raw control byte in a source file makes Git and ripgrep classify the
 * whole file as binary. Nothing else notices: TypeScript compiles it, ESLint and
 * Prettier pass, and the tests run — so it reaches review invisibly, and review
 * is exactly what it breaks. `git diff` renders `Bin 0 -> 11729 bytes` instead of
 * the code, and `rg` reports "Binary file matches" instead of the line.
 *
 * That happened here: `maskKey` in removal-queue.ts was written with a literal
 * U+0000 delimiter rather than an escape sequence, hiding a 278-line
 * removal/undo state machine — one of the highest-risk files in the app — from
 * every diff and grep-based audit on the branch.
 *
 * Escapes are fine; this only rejects the raw bytes.
 */
const SOURCE_ROOT = path.resolve(__dirname, "..", "..");

/** Tab, LF and CR are the only control characters legitimate in source. */
const ALLOWED = new Set([0x09, 0x0a, 0x0d]);

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const full = path.join(dir, entry);
    if (statSync(full).isDirectory()) return sourceFiles(full);
    return /\.(ts|tsx|css)$/.test(entry) ? [full] : [];
  });
}

describe("source hygiene", () => {
  it("has no raw control bytes that would make a file read as binary", () => {
    const offenders: string[] = [];

    for (const file of sourceFiles(SOURCE_ROOT)) {
      const bytes = readFileSync(file);
      for (let i = 0; i < bytes.length; i++) {
        const byte = bytes[i]!;
        if (byte < 0x20 && !ALLOWED.has(byte)) {
          const line = bytes.subarray(0, i).toString("utf8").split("\n").length;
          offenders.push(
            `${path.relative(SOURCE_ROOT, file)}:${line} contains raw 0x${byte
              .toString(16)
              .padStart(2, "0")} — use an escape such as \\u0000 instead`
          );
          break;
        }
      }
    }

    expect(offenders, offenders.join("\n")).toEqual([]);
  });
});
