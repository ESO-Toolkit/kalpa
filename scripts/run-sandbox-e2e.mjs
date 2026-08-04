/**
 * Run the destructive e2e specs against a throwaway AddOns folder.
 *
 * The rest of the e2e suite connects to whatever `npm run tauri dev` is already
 * running, which means it drives the developer's REAL ESO install. That is why
 * no spec has ever touched install, update, remove, restore, migrate or
 * profile-apply: asserting on those would mutate the machine running the tests.
 *
 * This runner owns the app instead of attaching to one. It builds the debug
 * binary, creates an empty sandbox AddOns folder plus fixture archives, launches
 * Kalpa with `KALPA_ADDONS_DIR` pointed at the sandbox (a debug-only override —
 * see `debug_addons_dir_override` in commands.rs), runs the `@sandbox` specs
 * against it, then kills the app and deletes the folder.
 *
 * Windows-only, for the same reason the packaged gate is: the CDP endpoint comes
 * from `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`.
 *
 * Note the sandbox covers the AddOns folder and its `.kalpa-metadata`, not the
 * app's settings.json — the specs here deliberately stay off flows that write
 * app-level settings.
 */

import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  CDP_ENDPOINT,
  assertNoExistingCdp,
  assertNoExistingKalpaProcess,
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

/** The addon the lifecycle spec installs, removes and restores. */
const FIXTURE_FOLDER = "KalpaE2EFixture";

async function main() {
  if (process.platform !== "win32") {
    throw new Error("The sandboxed e2e gate is Windows-only because it drives WebView2 CDP.");
  }

  await assertNoExistingCdp();
  await assertNoExistingKalpaProcess();

  // Build by default. Reusing whatever binary happens to be lying around tests
  // the wrong code — a stale one predating the debug override would fail these
  // specs for a reason that has nothing to do with the change under test.
  // `--no-build` is for iterating on the specs themselves.
  if (process.argv.includes("--no-build")) {
    console.log(`[${TAG}] --no-build: reusing ${path.relative(repoRoot, binaryPath)}`);
  } else {
    await run(process.execPath, [tauriCli, "build", "--debug", "--no-bundle"], "tauri build", {
      cwd: repoRoot,
      tag: TAG,
    });
  }
  if (!existsSync(binaryPath)) {
    throw new Error(`Expected debug binary at ${binaryPath}`);
  }

  const root = path.join(os.tmpdir(), `kalpa-e2e-${process.pid}`);
  // validate_addons_path requires the folder to be named exactly "AddOns".
  const sandboxDir = path.join(root, "AddOns");
  const fixturesDir = path.join(root, "fixtures");
  const fixtureZip = path.join(fixturesDir, `${FIXTURE_FOLDER}.zip`);

  mkdirSync(sandboxDir, { recursive: true });
  mkdirSync(fixturesDir, { recursive: true });
  writeFileSync(fixtureZip, makeAddonZip(FIXTURE_FOLDER, { title: "Kalpa E2E Fixture" }));
  console.log(`[${TAG}] sandbox ${sandboxDir}`);

  let child;
  try {
    child = launchKalpaDetached(binaryPath, {
      tag: TAG,
      env: { KALPA_ADDONS_DIR: sandboxDir },
    });
    await proveOwnedLaunch(child);
    await waitForCdp(child);
    await run(
      process.execPath,
      [playwrightCli, "test", "--grep", "@sandbox"],
      "playwright sandbox tests",
      {
        cwd: repoRoot,
        env: {
          KALPA_CDP_ENDPOINT: CDP_ENDPOINT,
          KALPA_E2E_SANDBOX_DIR: sandboxDir,
          KALPA_E2E_FIXTURE_ZIP: fixtureZip,
          KALPA_E2E_FIXTURE_FOLDER: FIXTURE_FOLDER,
        },
        tag: TAG,
      }
    );
  } finally {
    if (child?.pid) {
      await killProcessTree(child.pid, TAG);
    }
    // Only after the app is gone: it holds the metadata file open.
    try {
      rmSync(root, { recursive: true, force: true });
    } catch (error) {
      console.warn(`[${TAG}] could not remove ${root}: ${error.message}`);
    }
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
