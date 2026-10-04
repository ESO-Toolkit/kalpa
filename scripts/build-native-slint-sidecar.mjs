import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const isWindows = process.platform === "win32";
const exeExt = isWindows ? ".exe" : "";
const explicitTarget =
  process.env.KALPA_NATIVE_TARGET_TRIPLE || process.env.CARGO_BUILD_TARGET || "";
const targetTriple =
  explicitTarget || execFileSync("rustc", ["--print", "host-tuple"], { encoding: "utf8" }).trim();

if (!targetTriple) {
  throw new Error("Could not determine the Rust target triple for the Slint sidecar.");
}

const destinationDir = path.join(repoRoot, "src-tauri", "binaries");
const destination = path.join(destinationDir, `kalpa-slint-${targetTriple}${exeExt}`);

// The release workflow compiles the sidecar in its own job, in parallel with
// the app builds, and drops the binary into src-tauri/binaries before the
// Windows app job runs `npm run build:release-assets`. This env var tells that
// second invocation to trust the file it was handed instead of spending
// another ~14 minutes rebuilding it on the critical path. It is opt-in and
// fails loudly, so a missing download cannot degrade into either a silent
// rebuild or a bundle that ships the zero-byte dev placeholder
// (scripts/ensure-slint-sidecar-placeholder.mjs) as the real sidecar.
if (process.env.KALPA_SIDECAR_PREBUILT === "1") {
  if (!fs.existsSync(destination) || fs.statSync(destination).size === 0) {
    throw new Error(
      `KALPA_SIDECAR_PREBUILT=1 but ${path.relative(repoRoot, destination)} is missing or empty; ` +
        "the sidecar artifact was not downloaded before the app build."
    );
  }
  console.log(
    `Using prebuilt Slint sidecar: ${path.relative(repoRoot, destination)} ` +
      `(${fs.statSync(destination).size} bytes)`
  );
  process.exit(0);
}

const cargoArgs = [
  "build",
  "--manifest-path",
  path.join("prototypes", "slint-kalpa", "Cargo.toml"),
  "--release",
];

if (explicitTarget) {
  cargoArgs.push("--target", explicitTarget);
}

execFileSync("cargo", cargoArgs, {
  cwd: repoRoot,
  stdio: "inherit",
});

const targetDir = explicitTarget
  ? path.join("target", explicitTarget, "release")
  : path.join("target", "release");
const source = path.join(
  repoRoot,
  "prototypes",
  "slint-kalpa",
  targetDir,
  `kalpa-slint-prototype${exeExt}`
);

if (!fs.existsSync(source)) {
  throw new Error(`Slint sidecar build did not produce ${source}`);
}

fs.mkdirSync(destinationDir, { recursive: true });
fs.copyFileSync(source, destination);
fs.chmodSync(destination, 0o755);
console.log(`Prepared Slint sidecar: ${path.relative(repoRoot, destination)}`);
