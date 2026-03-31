#!/usr/bin/env node

/**
 * Preflight check for Kalpa development environment.
 * Validates that all prerequisites for building a Tauri v2 app are present.
 *
 * Usage: node scripts/check-env.js
 *        npm run check:env
 */

import { execSync } from "node:child_process";
import { platform } from "node:os";

const isWindows = platform() === "win32";
let passed = 0;
let failed = 0;

function run(cmd) {
  try {
    return execSync(cmd, { encoding: "utf-8", stdio: ["pipe", "pipe", "pipe"] }).trim();
  } catch {
    return null;
  }
}

function check(label, fn) {
  const result = fn();
  if (result.ok) {
    console.log(`  ✓ ${label}: ${result.detail}`);
    passed++;
  } else {
    console.error(`  ✗ ${label}: ${result.detail}`);
    failed++;
  }
}

console.log("\nKalpa — environment check\n");

// Node.js version
check("Node.js", () => {
  const raw = run("node --version");
  if (!raw) return { ok: false, detail: "not found — install from https://nodejs.org/" };
  const major = parseInt(raw.replace("v", ""), 10);
  if (major < 18) return { ok: false, detail: `${raw} found, but 18+ is required` };
  return { ok: true, detail: raw };
});

// npm
check("npm", () => {
  const raw = run("npm --version");
  if (!raw) return { ok: false, detail: "not found" };
  return { ok: true, detail: `v${raw}` };
});

// Rust toolchain
check("Rust (rustc)", () => {
  const raw = run("rustc --version");
  if (!raw) return { ok: false, detail: "not found — install from https://rustup.rs/" };
  return { ok: true, detail: raw };
});

check("Cargo", () => {
  const raw = run("cargo --version");
  if (!raw) return { ok: false, detail: "not found" };
  return { ok: true, detail: raw };
});

// Tauri CLI
check("Tauri CLI", () => {
  // Check if @tauri-apps/cli is installed locally in node_modules
  const localVersion = run(
    'node -e "process.stdout.write(JSON.parse(require(\'fs\').readFileSync(\'node_modules/@tauri-apps/cli/package.json\',\'utf-8\')).version)"'
  );
  if (localVersion) return { ok: true, detail: `v${localVersion} (local)` };

  // Check for global cargo-tauri
  const cargoVersion = run("cargo tauri --version");
  if (cargoVersion) return { ok: true, detail: cargoVersion };

  // Fallback: npx with --no (never auto-install) to avoid interactive prompts
  const npxVersion = run("npx --no tauri --version");
  if (npxVersion) return { ok: true, detail: `${npxVersion} (npx)` };

  return { ok: false, detail: "not found — run: npm install" };
});

// Windows-specific checks
if (isWindows) {
  check("MSVC toolchain", () => {
    const target = run("rustup show active-toolchain");
    if (!target) return { ok: false, detail: "could not determine active toolchain" };
    if (!target.includes("msvc"))
      return {
        ok: false,
        detail: `active toolchain is "${target}" — MSVC toolchain required on Windows`,
      };
    return { ok: true, detail: target };
  });

  check("WebView2", () => {
    // Check via registry — WebView2 Evergreen or Fixed installs write here
    const regKeys = [
      // x64 / WoW64
      'HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
      // ARM64 and native x64
      'HKLM\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
      // User-level install
      'HKCU\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
    ];
    for (const key of regKeys) {
      const reg = run(`reg query "${key}" /v pv 2>nul`);
      if (reg) {
        const match = reg.match(/pv\s+REG_SZ\s+(.+)/);
        return { ok: true, detail: match ? `v${match[1].trim()}` : "installed" };
      }
    }
    return {
      ok: false,
      detail:
        "not detected — required at runtime. Get it from https://developer.microsoft.com/en-us/microsoft-edge/webview2/",
    };
  });
} else {
  console.log(
    "  ⓘ Non-Windows OS detected. Kalpa targets Windows only; cross-compilation may require extra setup."
  );
}

// Summary
console.log("");
if (failed > 0) {
  console.error(`${failed} check(s) failed. Fix the issues above before building.\n`);
  process.exit(1);
} else {
  console.log(`All ${passed} checks passed. You're ready to build Kalpa!\n`);
}
