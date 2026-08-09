#!/usr/bin/env node

/**
 * Preflight check for Kalpa development environment.
 * Validates that all prerequisites for building a Tauri v2 app are present.
 *
 * Usage: node scripts/check-env.js
 *        npm run check:env
 */

import { execSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { platform } from "node:os";

// package.json's engines.node is the single source of truth for the supported
// range, so this reads it rather than restating it.
function readNodeRange() {
  try {
    const pkg = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf-8"));
    const range = pkg.engines?.node;
    return typeof range === "string" && range.trim() ? range : null;
  } catch {
    return null;
  }
}

const NODE_RANGE = readNodeRange();

// Only `^x.y.z` and `>=x.y.z` are understood. Anything else throws rather than
// being silently treated as a bare floor: `~22.22.2` or `>=22 <23` would
// otherwise evaluate as unbounded and quietly accept versions the range
// excludes, which is worse than refusing to answer.
const TERM = /^(\^|>=)(\d+)\.(\d+)\.(\d+)$/;

function parseVersion(text) {
  // Anchored, and it keeps the prerelease tag. `v22.22.2-rc.1` sorts BELOW
  // 22.22.2, so reading it as [22,22,2] would reintroduce exactly the kind of
  // false pass this check exists to remove — and that RC predates the 22.22.2
  // fix jsdom 30 needs.
  const match = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/.exec(text.trim());
  if (!match) return null;
  return {
    version: [Number(match[1]), Number(match[2]), Number(match[3])],
    prerelease: match[4] ?? null,
  };
}

function isAtLeast(version, floor) {
  for (let i = 0; i < 3; i++) {
    if (version[i] !== floor[i]) return version[i] > floor[i];
  }
  return true;
}

function satisfiesNodeRange(version) {
  return NODE_RANGE.split("||").some((clause) => {
    const term = clause.trim();
    const match = TERM.exec(term);
    if (!match) {
      throw new Error(`engines.node term "${term}" is not \`^x.y.z\` or \`>=x.y.z\``);
    }
    const floor = [Number(match[2]), Number(match[3]), Number(match[4])];
    if (!isAtLeast(version, floor)) return false;
    // `^x.y.z` also caps at the next major; `>=x.y.z` has no ceiling.
    return match[1] === "^" ? version[0] === floor[0] : true;
  });
}

const isWindows = platform() === "win32";
const isMac = platform() === "darwin";
const isLinux = platform() === "linux";
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
  if (!NODE_RANGE) return { ok: false, detail: "package.json declares no engines.node to check" };
  const parsed = parseVersion(raw);
  if (!parsed) return { ok: false, detail: `could not parse version from "${raw}"` };
  if (parsed.prerelease) {
    return { ok: false, detail: `${raw} is a prerelease; ${NODE_RANGE} is required` };
  }
  // A bare `major >= 22` gate passed on versions the dependency tree rejects:
  // jsdom 30 needs ^22.22.2 || ^24.15.0 || >=26.0.0, so 22.13 and 24.0 both
  // install "fine" and then fail somewhere inside the test run instead. Check
  // the real range so the failure lands here, with the version printed.
  try {
    if (!satisfiesNodeRange(parsed.version)) {
      return { ok: false, detail: `${raw} found, but ${NODE_RANGE} is required` };
    }
  } catch (error) {
    return { ok: false, detail: error.message };
  }
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
    "node -e \"process.stdout.write(JSON.parse(require('fs').readFileSync('node_modules/@tauri-apps/cli/package.json','utf-8')).version)\""
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
      "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
      // ARM64 and native x64
      "HKLM\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
      // User-level install
      "HKCU\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
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
} else if (isMac) {
  check("Xcode Command Line Tools", () => {
    const path = run("xcode-select -p");
    if (!path)
      return {
        ok: false,
        detail: "not found — run: xcode-select --install",
      };
    return { ok: true, detail: path };
  });
} else if (isLinux) {
  // Tauri v2 on Linux builds against WebKitGTK 4.1 + GTK3; the appindicator
  // library backs the tray icon and librsvg the bundler's icon pipeline.
  const pkgs = [
    ["webkit2gtk-4.1", "libwebkit2gtk-4.1-dev"],
    ["gtk+-3.0", "libgtk-3-dev"],
    ["ayatana-appindicator3-0.1", "libayatana-appindicator3-dev"],
    ["librsvg-2.0", "librsvg2-dev"],
  ];
  for (const [pcName, aptName] of pkgs) {
    check(pcName, () => {
      const version = run(`pkg-config --modversion ${pcName}`);
      if (!version)
        return {
          ok: false,
          detail: `not found — install ${aptName} (Debian/Ubuntu) or the equivalent for your distro`,
        };
      return { ok: true, detail: `v${version}` };
    });
  }
  check("patchelf", () => {
    const version = run("patchelf --version");
    if (!version)
      return { ok: false, detail: "not found — needed for AppImage bundling (apt: patchelf)" };
    return { ok: true, detail: version };
  });
}

// Summary
console.log("");
if (failed > 0) {
  console.error(`${failed} check(s) failed. Fix the issues above before building.\n`);
  process.exit(1);
} else {
  console.log(`All ${passed} checks passed. You're ready to build Kalpa!\n`);
}
