#!/usr/bin/env node

/**
 * Verifies every file that carries Kalpa's version agrees with the others.
 *
 * A version bump has to land in five files — package-lock.json stores the
 * version twice — and nothing enforced that: nine consecutive tags (alpha.8
 * through beta.9) shipped with package-lock.json still pinned to an older
 * version. tauri.conf.json is the reference because it is the value the bundler
 * stamps into the installer and the one latest.json reports to the updater,
 * so it is already the version the release publish gate compares to the tag.
 *
 * Usage: node scripts/check-versions.js
 *        node scripts/check-versions.js --expect v0.1.0-beta.14
 *        npm run check:versions
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const read = (relPath) => readFileSync(join(root, relPath), "utf-8");
const readJson = (relPath) => JSON.parse(read(relPath));

/**
 * Splits a TOML document at every table header, so a key can be looked up
 * inside one specific table instead of anywhere in the file. Both files this
 * reads are machine-generated and contain no multi-line strings, so a line
 * that looks like a header is a header.
 */
function tomlTables(text) {
  const tables = [];
  let current = { header: "", lines: [] };
  for (const line of text.split(/\r?\n/)) {
    const header = /^\s*(\[.+\])\s*$/.exec(line);
    if (header) {
      tables.push(current);
      current = { header: header[1], lines: [] };
    } else {
      current.lines.push(line);
    }
  }
  tables.push(current);
  return tables;
}

/** Reads a bare `key = "value"` out of one table's lines. */
function tomlString(table, key) {
  for (const line of table.lines) {
    const entry = /^\s*([A-Za-z0-9_-]+)\s*=\s*"([^"]*)"\s*$/.exec(line);
    if (entry && entry[1] === key) return entry[2];
  }
  return null;
}

// The crate version lives under [package]. Scoping to that table matters:
// [dependencies] entries carry their own `version = "…"` keys, and a bare
// regex over the file would happily return tauri's "2".
function cargoTomlVersion() {
  const table = tomlTables(read("src-tauri/Cargo.toml")).find((t) => t.header === "[package]");
  if (!table) throw new Error("src-tauri/Cargo.toml has no [package] table");
  return tomlString(table, "version");
}

// Cargo.lock describes ~666 crates, each with its own `version = "…"`, plus a
// bare `version = 4` lockfile-format key at the top. Only the [[package]]
// block whose name is exactly "kalpa" is this app — matching on a prefix would
// also catch the real sibling crate "kalpa-slint-prototype", and matching by
// regex over the whole file returns whichever version happens to come first
// (that is `adler2`'s "2.0.1" — plausible-looking, which is the dangerous part).
function cargoLockVersion() {
  const blocks = tomlTables(read("src-tauri/Cargo.lock")).filter(
    (t) => t.header === "[[package]]" && tomlString(t, "name") === "kalpa"
  );
  if (blocks.length !== 1) {
    throw new Error(
      `src-tauri/Cargo.lock: expected exactly one [[package]] named "kalpa", found ${blocks.length}`
    );
  }
  return tomlString(blocks[0], "version");
}

const tauriConf = readJson("src-tauri/tauri.conf.json");
const packageJson = readJson("package.json");
const packageLock = readJson("package-lock.json");

const found = [
  // tauri.conf.json leads: every other entry is compared against it.
  ["src-tauri/tauri.conf.json", "version", tauriConf.version],
  ["package.json", "version", packageJson.version],
  // npm writes the root version into BOTH the top level and packages[""].
  // `npm version` keeps them in step; a hand-edit of one does not.
  ["package-lock.json", "version", packageLock.version],
  ["package-lock.json", 'packages[""].version', packageLock.packages?.[""]?.version],
  ["src-tauri/Cargo.toml", "[package] version", cargoTomlVersion()],
  ["src-tauri/Cargo.lock", '[[package]] "kalpa" version', cargoLockVersion()],
];

const expected = tauriConf.version;
const fileWidth = Math.max(...found.map(([file]) => file.length));
const fieldWidth = Math.max(...found.map(([, field]) => field.length));

console.log("\nKalpa — version consistency\n");

const mismatched = [];
for (const [file, field, version] of found) {
  const row = `${file.padEnd(fileWidth)}  ${field.padEnd(fieldWidth)}  ${version ?? "(missing)"}`;
  if (version === expected) {
    console.log(`  ✓ ${row}`);
  } else {
    console.error(`  ✗ ${row}`);
    mismatched.push([file, field, version]);
  }
}

// Also pin the agreed version to a release tag, so a tag cut before the bump
// landed aborts in seconds instead of being caught an hour later by the
// publish job's latest.json check — after three platform builds have run.
const expectIndex = process.argv.indexOf("--expect");
const expectedTag = expectIndex === -1 ? null : process.argv[expectIndex + 1];
if (expectIndex !== -1 && !expectedTag) {
  console.error("\n--expect requires a tag, e.g. --expect v0.1.0-beta.14\n");
  process.exit(2);
}
const tagVersion = expectedTag?.replace(/^v/, "");

console.log("");
if (mismatched.length > 0) {
  const summary = `${mismatched.length} file(s) disagree with src-tauri/tauri.conf.json (${expected}): ${mismatched
    .map(([file, field, version]) => `${file} ${field} = ${version ?? "(missing)"}`)
    .join("; ")}`;
  if (process.env.GITHUB_ACTIONS) console.error(`::error::${summary}`);
  console.error(`${summary}\nBump every file above to the same version.\n`);
  process.exit(1);
}
if (tagVersion && tagVersion !== expected) {
  const summary = `All files agree on ${expected}, but the release tag is ${expectedTag} — the version bump never landed.`;
  if (process.env.GITHUB_ACTIONS) console.error(`::error::${summary}`);
  console.error(`${summary}\n`);
  process.exit(1);
}
console.log(`All ${found.length} version fields agree on ${expected}.\n`);
