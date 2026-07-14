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
const destinationDir = path.join(repoRoot, "src-tauri", "binaries");
const destination = path.join(destinationDir, `kalpa-slint-${targetTriple}${exeExt}`);

if (!fs.existsSync(source)) {
  throw new Error(`Slint sidecar build did not produce ${source}`);
}

fs.mkdirSync(destinationDir, { recursive: true });
fs.copyFileSync(source, destination);
fs.chmodSync(destination, 0o755);
console.log(`Prepared Slint sidecar: ${path.relative(repoRoot, destination)}`);
