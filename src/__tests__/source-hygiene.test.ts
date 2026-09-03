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
 * A byte-order mark at offset 0. PowerShell's `Set-Content -Encoding utf8` and
 * `Out-File` write one silently; several tools then see a different first line
 * than the reviewer does. Every hit found while adding this detector was
 * tool-introduced, never intentional — so it fails hard with no allowlist.
 */
function hasByteOrderMark(bytes: Buffer): boolean {
  if (bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) return true;
  if (bytes.length >= 2) {
    if (bytes[0] === 0xff && bytes[1] === 0xfe) return true;
    if (bytes[0] === 0xfe && bytes[1] === 0xff) return true;
  }
  return false;
}

/**
 * U+FFFD is what a decoder emits when bytes were already destroyed — by the
 * time it is in a source file the original character is unrecoverable, so its
 * presence always marks damage (a banner comment in commands.rs shipped with
 * three of them). Two files legitimately contain it as a FIXTURE and are
 * allowlisted by exact path below.
 */
const REPLACEMENT_CHAR = Buffer.from([0xef, 0xbf, 0xbd]);
const REPLACEMENT_CHAR_FIXTURES = new Set([
  // Asserts that a lone invalid Lua byte decodes to the replacement char; the
  // FFFD is the expected value, not damage.
  "src/lib/__tests__/saved-variables-logic.test.ts",
]);

/**
 * Mojibake: UTF-8 bytes decoded as latin-1 and re-encoded, the shape produced
 * by writing files through a legacy codepage (PowerShell 5.1's default, the
 * codex sandbox). Matched on the DECODED string, not the raw bytes — a
 * correctly-encoded "é" is C3 A9 on disk but ONE character decoded, while
 * mojibake decodes to the two-character telltale. C3/C2 lead-ins cover mangled
 * latin-1; the E2 family covers mangled punctuation and box-drawing.
 */
// Two codepage families exist in the wild and BOTH have been written into
// this repo by tooling: latin-1 (0x80-0x9F become C1 controls) and
// Windows-1252 (0x80-0x9F become printable punctuation \u2014 an em dash arrives
// as U+00E2 U+20AC U+201D). The class is the union of both continuation
// ranges. Escape sequences throughout, for the same reason the control-byte
// test uses byte arrays: this file is itself inside the scan.
const MOJI_CONT =
  "\\u0080-\\u00bf" +
  "\\u20ac\\u201a\\u0192\\u201e\\u2026\\u2020\\u2021\\u02c6\\u2030\\u0160\\u2039\\u0152\\u017d" +
  "\\u2018\\u2019\\u201c\\u201d\\u2022\\u2013\\u2014\\u02dc\\u2122\\u0161\\u203a\\u0153\\u017e\\u0178";
const MOJIBAKE = new RegExp(
  `\\u00c3[${MOJI_CONT}]|\\u00e2[${MOJI_CONT}][${MOJI_CONT}]|\\u00c2[\\u00a1-\\u00bf]`
);
const MOJIBAKE_FIXTURES = new Set([
  // An anti-mojibake regression test whose comment spells out the exact shape
  // it guards against; the literal is the documentation.
  "src-tauri/src/saved_variables/serializer.rs",
]);

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

/**
 * Read every tracked text file, returning what was actually READ alongside what
 * offended.
 *
 * The distinction matters: an earlier version asserted coverage against the
 * `git ls-files` listing while the scan itself skipped unreadable entries, so a
 * tracked file missing from the working tree could satisfy "we look at
 * src-tauri" without a single byte of it being examined. Coverage is asserted
 * against `scanned` for that reason.
 */
function scanRepo(): { scanned: string[]; offenders: string[] } {
  const scanned: string[] = [];
  const offenders: string[] = [];

  for (const file of trackedTextFiles()) {
    let bytes: Buffer;
    try {
      bytes = readFileSync(path.join(REPO_ROOT, file));
    } catch (error) {
      // An unstaged deletion is still listed by `git ls-files` but is not in
      // the tree under review, so it cannot hide anything from a reviewer.
      // Anything else is a real failure and must not be swallowed.
      if ((error as NodeJS.ErrnoException).code === "ENOENT") continue;
      throw error;
    }
    scanned.push(file);

    const at = firstOffendingByte(bytes);
    if (at !== -1) {
      const line = bytes.subarray(0, at).toString("utf8").split("\n").length;
      offenders.push(
        `${file}:${line} contains raw 0x${bytes[at]!.toString(16).padStart(2, "0")} — ` +
          `use an escape such as \\u0000 instead`
      );
    }

    if (hasByteOrderMark(bytes)) {
      offenders.push(`${file} starts with a byte-order mark — rewrite it without one`);
    }

    if (!REPLACEMENT_CHAR_FIXTURES.has(file) && bytes.includes(REPLACEMENT_CHAR)) {
      const idx = bytes.indexOf(REPLACEMENT_CHAR);
      const line = bytes.subarray(0, idx).toString("utf8").split("\n").length;
      offenders.push(
        `${file}:${line} contains U+FFFD — the original character was destroyed; restore it`
      );
    }

    if (!MOJIBAKE_FIXTURES.has(file)) {
      const text = bytes.toString("utf8");
      const hit = MOJIBAKE.exec(text);
      if (hit) {
        const line = text.slice(0, hit.index).split("\n").length;
        offenders.push(
          `${file}:${line} contains mojibake (double-encoded UTF-8) — ` +
            `rewrite the file with a UTF-8-clean tool (node fs, not Set-Content)`
        );
      }
    }
  }

  return { scanned, offenders };
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

  it("detects the encodings damage it exists to detect", () => {
    // Byte arrays again — none of these shapes may appear literally in this
    // file, or the scan flags its own detector.
    expect(hasByteOrderMark(Buffer.from([0xef, 0xbb, 0xbf, 0x61]))).toBe(true);
    expect(hasByteOrderMark(Buffer.from([0xff, 0xfe, 0x61]))).toBe(true);
    expect(hasByteOrderMark(Buffer.from([0x61, 0x62]))).toBe(false);

    expect(Buffer.from([0x61, 0xef, 0xbf, 0xbd, 0x62]).includes(REPLACEMENT_CHAR)).toBe(true);
    expect(Buffer.from("plain ascii").includes(REPLACEMENT_CHAR)).toBe(false);

    const cc = String.fromCharCode;
    // Windows-1252 family: an em dash written through cp1252 arrives as
    // U+00E2 U+20AC U+201D (this exact shape shipped in Cargo.toml).
    expect(MOJIBAKE.test(cc(0xe2, 0x20ac, 0x201d))).toBe(true);
    // latin-1 family: "é" double-encoded arrives as U+00C3 U+00A9.
    expect(MOJIBAKE.test(cc(0xc3, 0xa9))).toBe(true);
    // Correctly-encoded text must pass: single-char accents and punctuation.
    expect(MOJIBAKE.test("caract" + cc(0xe8) + "re " + cc(0xe9) + " " + cc(0x2014))).toBe(false);
    expect(MOJIBAKE.test("box " + cc(0x2500, 0x2502, 0x2514) + " drawing")).toBe(false);
  });

  it("actually reads files well past the frontend", () => {
    // Without this the scan below passes vacuously whenever the file list comes
    // back empty or collapses to `src/` — the two ways this guard has already
    // been weaker than it read. These four are asserted against the files whose
    // bytes were READ, so neither a shrunken listing nor a skipped read can
    // satisfy it.
    const read = new Set(scanRepo().scanned);

    expect(read).toContain("src/lib/removal-queue.ts");
    expect(read).toContain("src-tauri/src/commands.rs");
    expect(read).toContain("backend/eso-packs-worker/src/index.ts");
    expect(read).toContain("scripts/check-versions.js");
    expect(read).toContain("prototypes/slint-kalpa/src/main.rs");
    expect(read.size).toBeGreaterThan(100);
  });

  it("has no raw control bytes that would make a file read as binary", () => {
    const { offenders } = scanRepo();
    expect(offenders, offenders.join("\n")).toEqual([]);
  });
});
