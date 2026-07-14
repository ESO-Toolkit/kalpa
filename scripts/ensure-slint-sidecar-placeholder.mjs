// tauri.windows.conf.json declares the kalpa-slint sidecar as an externalBin,
// which makes tauri-build (build.rs) require the file to EXIST for every
// Windows compile of src-tauri — including `tauri dev`, clippy, and tests.
// This creates a zero-byte placeholder so those builds work without first
// compiling the (heavy) Slint prototype. Release builds never see it: the
// Windows beforeBuildCommand (`npm run build:release-assets`) overwrites the
// placeholder with the real sidecar binary before bundling.
//
// Dev note: with only the placeholder present, the in-app "native UI" handoff
// spawns an empty exe and fails; run `npm run build:native-slint` once to get
// a real sidecar for local end-to-end testing.
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

if (process.platform !== "win32") process.exit(0);

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const triple =
  process.env.KALPA_NATIVE_TARGET_TRIPLE ||
  execFileSync("rustc", ["--print", "host-tuple"], { encoding: "utf8" }).trim();
const dir = path.join(repoRoot, "src-tauri", "binaries");
const file = path.join(dir, `kalpa-slint-${triple}.exe`);

if (!fs.existsSync(file)) {
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(file, "");
  console.log(`Created Slint sidecar placeholder: ${path.relative(repoRoot, file)}`);
}
