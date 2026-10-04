/**
 * Run destructive specs against an owned Windows debug process. Its AddOns,
 * app data, credentials and WebView2 profiles use a unique per-run namespace.
 * This is a fixture smoke test, not proof of production download/install or
 * authenticated upload behavior. The runner force-kills the app at teardown;
 * shutdown semantics need separate coverage.
 */

import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  CDP_ENDPOINT,
  assertNoExistingCdp,
  assertNoExistingKalpaProcess,
  createIsolatedProfile,
  killProcessTree,
  launchKalpaDetached,
  proveOwnedLaunch,
  run,
  waitForCdp,
} from "./lib/kalpa-app-harness.mjs";
import { makeAddonZip } from "./lib/make-fixture-zip.mjs";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const binaryPath = path.join(repoRoot, "src-tauri", "target", "debug", "kalpa.exe");
const tauriCli = path.join(repoRoot, "node_modules", "@tauri-apps", "cli", "tauri.js");
const playwrightCli = path.join(repoRoot, "node_modules", "@playwright", "test", "cli.js");
const TAG = "sandbox";
const FIXTURE_FOLDER = "KalpaE2EFixture";

async function main() {
  if (process.platform !== "win32") {
    throw new Error("The sandboxed e2e gate is Windows-only because it drives WebView2 CDP.");
  }
  await assertNoExistingCdp();
  await assertNoExistingKalpaProcess();

  if (process.argv.includes("--no-build")) {
    throw new Error(
      "--no-build is unsafe: an older binary may write to the real app profile before IPC can verify isolation."
    );
  }
  await run(process.execPath, [tauriCli, "build", "--debug", "--no-bundle"], "tauri build", {
    cwd: repoRoot,
    tag: TAG,
  });
  if (!existsSync(binaryPath)) throw new Error(`Expected debug binary at ${binaryPath}`);

  const profile = createIsolatedProfile();
  let child;
  try {
    const fixturesDir = path.join(profile.root, "fixtures");
    mkdirSync(fixturesDir);
    const fixtureZip = path.join(fixturesDir, `${FIXTURE_FOLDER}.zip`);
    writeFileSync(fixtureZip, makeAddonZip(FIXTURE_FOLDER, { title: "Kalpa E2E Fixture" }));
    console.log(`[${TAG}] sandbox ${profile.addons}`);

    child = launchKalpaDetached(binaryPath, { tag: TAG, env: profile.env });
    await proveOwnedLaunch(child);
    await waitForCdp(child, 120_000);
    await run(
      process.execPath,
      [playwrightCli, "test", "--grep", "@sandbox"],
      "playwright sandbox tests",
      {
        cwd: repoRoot,
        env: {
          ...profile.testEnv,
          KALPA_CDP_ENDPOINT: CDP_ENDPOINT,
          KALPA_E2E_FIXTURE_ZIP: fixtureZip,
          KALPA_E2E_FIXTURE_FOLDER: FIXTURE_FOLDER,
        },
        tag: TAG,
      }
    );
  } finally {
    if (child?.pid) await killProcessTree(child.pid, TAG);
    await profile.cleanup();
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
