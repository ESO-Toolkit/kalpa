import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
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
 *
 * SCOPE IS THE WHOLE REPOSITORY, deliberately. The first version of this guard
 * walked `src/` for `.ts`/`.tsx`/`.css`, which is narrower than the problem: the
 * same byte in a Rust module, the worker, a runner script, an e2e spec or a
 * workflow file hides the file just as completely, and this test stayed green.
 * The file set comes from `git ls-files` because tracked files are precisely
 * what review sees.
 *
 * The scan is split into three assertions on purpose, because the two ways a
 * guard like this goes quiet are both invisible from its result: it can stop
 * detecting, and it can stop looking. So one test proves the detector fires,
 * one proves the file list reaches past the frontend, and one runs the scan.
 */
const REPO_ROOT = path.resolve(__dirname, "..", "..");

/** Tab, LF and CR are the only control characters legitimate in source. */
const ALLOWED = new Set([0x09, 0x0a, 0x0d]);

/** Index of the first byte that would make Git call this file binary, or -1. */
function firstOffendingByte(bytes: Buffer): number {
  for (let i = 0; i < bytes.length; i++) {
    const byte = bytes[i]!;
    if (byte < 0x20 && !ALLOWED.has(byte)) return i;
  }
  return -1;
}

/**
 * Skipped by extension — these are *supposed* to contain arbitrary bytes.
 *
 * A denylist, not an allowlist, and that direction is the point: an allowlist
 * fails open, which is how the first version of this test missed every Rust and
 * worker file in the repo. A new source extension is scanned by default here,
 * and only a type declared binary below escapes the check.
 */
const BINARY_EXTENSIONS = new Set([
  "png",
  "webp",
  "jpg",
  "jpeg",
  "gif",
  "ico",
  "icns",
  "ttf",
  "otf",
  "woff",
  "woff2",
  "zip",
  "wasm",
  "pdf",
  "mp4",
  "dll",
  "exe",
]);

function trackedTextFiles(): string[] {
  // `-C` rather than the process cwd: `git ls-files` run from a subdirectory
  // lists only that subdirectory, which would quietly reduce this back to a
  // src/-only check.
  const listing = execFileSync("git", ["-C", REPO_ROOT, "ls-files", "-z"], {
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  });
  return listing
    .split("\0")
    .filter(Boolean)
    .filter((file) => {
      const dot = file.lastIndexOf(".");
      const slash = Math.max(file.lastIndexOf("/"), file.lastIndexOf("\\"));
      if (dot <= slash + 1) return true; // no extension (LICENSE) or a dotfile
      return !BINARY_EXTENSIONS.has(file.slice(dot + 1).toLowerCase());
    });
}

describe("source hygiene", () => {
  it("detects the byte it exists to detect", () => {
    // Proves the scan below is capable of failing. The alternative — writing a
    // NUL into a real source file and watching the suite go red — is the same
    // proof with a corrupted working tree as a side effect.
    //
    // Byte arrays rather than string escapes, because this file is itself
    // inside the scan: the bytes under test must not appear literally here.
    expect(firstOffendingByte(Buffer.from("const a = 1;\n"))).toBe(-1);
    expect(firstOffendingByte(Buffer.from("tabs\tand\r\nnewlines are fine\n"))).toBe(-1);
    expect(firstOffendingByte(Buffer.from("caractère unicode é ok\n"))).toBe(-1);
    expect(firstOffendingByte(Buffer.from([0x61, 0x00, 0x62]))).toBe(1);
    expect(firstOffendingByte(Buffer.from([0x78, 0x1b, 0x79]))).toBe(1);
  });

  it("looks past the frontend", () => {
    // Without this the scan below passes vacuously whenever the file list comes
    // back empty or collapses to `src/` — the two ways this guard has already
    // been weaker than it read. Naming real files from four different roots
    // makes that failure loud instead of silent.
    const files = new Set(trackedTextFiles());

    expect(files).toContain("src/lib/removal-queue.ts");
    expect(files).toContain("src-tauri/src/commands.rs");
    expect(files).toContain("backend/eso-packs-worker/src/index.ts");
    expect(files).toContain("scripts/check-versions.js");
    expect(files.size).toBeGreaterThan(100);
  });

  it("has no raw control bytes that would make a file read as binary", () => {
    const offenders: string[] = [];

    for (const file of trackedTextFiles()) {
      let bytes: Buffer;
      try {
        bytes = readFileSync(path.join(REPO_ROOT, file));
      } catch (error) {
        // A staged deletion is listed but absent from the working tree, and a
        // file that is not on disk cannot hide anything from a reviewer. Any
        // other read failure is a real problem and must not be swallowed.
        if ((error as NodeJS.ErrnoException).code === "ENOENT") continue;
        throw error;
      }

      const at = firstOffendingByte(bytes);
      if (at === -1) continue;

      const line = bytes.subarray(0, at).toString("utf8").split("\n").length;
      offenders.push(
        `${file}:${line} contains raw 0x${bytes[at]!.toString(16).padStart(2, "0")} — ` +
          `use an escape such as \\u0000 instead`
      );
    }

    expect(offenders, offenders.join("\n")).toEqual([]);
  });
});
