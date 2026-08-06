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
 * ## The isolation is PARTIAL. Know exactly where the line is.
 *
 * Isolated (throwaway, deleted afterwards):
 *   - the AddOns folder and its `.kalpa-metadata`
 *   - the WebView2 user-data folder
 *
 * NOT isolated — these are the developer's real, live files:
 *   - `settings.json` (sortMode, filterMode, autoUpdate, addonsPath, …)
 *   - the manifest-cache SQLite DB, uploader history, saved auth tokens
 *   - anything else under the app-data dir
 *
 * Tauri resolves the app-data dir from the bundle identifier through the OS
 * known-folder API, so no environment variable redirects it the way
 * `KALPA_ADDONS_DIR` redirects the AddOns folder. Full isolation needs a fixture
 * profile or a dedicated test binary — tracked as follow-up, not solved here.
 *
 * Two consequences, both of which have already bitten:
 *   1. Persisted settings CHANGE test behaviour. A developer whose last session
 *      ended on the Libs filter booted the sandbox with that filter live and the
 *      row-count assertions read 0. Specs must normalise the state they depend
 *      on — see `showAllAddons` in the lifecycle spec.
 *   2. Specs can WRITE to those files. Keep `@sandbox` specs off flows that
 *      persist app-level settings beyond what they normalise.
 *
 * Treat this as a strong local smoke test for destructive flows, not as a
 * containment boundary.
 */

import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  CDP_ENDPOINT,
  assertNoExistingCdp,
  assertNoExistingKalpaProcess,
  delay,
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
  // Give WebView2 its own user-data folder rather than letting it default to
  // one beside the exe. A run that cannot create that folder brings the app up
  // without a webview — so no window, and no debug port to attach to.
  const webviewDataDir = path.join(root, "webview2");

  mkdirSync(sandboxDir, { recursive: true });
  mkdirSync(fixturesDir, { recursive: true });
  mkdirSync(webviewDataDir, { recursive: true });

  // The marker is what authorises the destructive debug commands — not
  // KALPA_ADDONS_DIR, which can name any folder including a live ESO install.
  // Must match SANDBOX_MARKER in src-tauri/src/commands.rs.
  writeFileSync(
    path.join(sandboxDir, ".kalpa-e2e-sandbox"),
    "Created by scripts/run-sandbox-e2e.mjs. This folder is deleted when the run ends.\n"
  );
  writeFileSync(fixtureZip, makeAddonZip(FIXTURE_FOLDER, { title: "Kalpa E2E Fixture" }));
  console.log(`[${TAG}] sandbox ${sandboxDir}`);

  let child;
  try {
    child = launchKalpaDetached(binaryPath, {
      tag: TAG,
      env: {
        KALPA_ADDONS_DIR: sandboxDir,
        WEBVIEW2_USER_DATA_FOLDER: webviewDataDir,
      },
    });
    await proveOwnedLaunch(child);
    // Generous: a cold WebView2 on a fresh CI runner has to build its user-data
    // folder before it serves anything, which is far slower than a warm dev box.
    await waitForCdp(child, 120_000);
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
    // Only after the app is gone: it holds the metadata file open, and
    // WebView2 keeps a handle on its user-data folder for a moment past
    // taskkill — so retry rather than warning on a race that resolves itself.
    await removeWithRetry(root);
  }
}

async function removeWithRetry(dir, attempts = 10) {
  for (let i = 0; i < attempts; i++) {
    try {
      rmSync(dir, { recursive: true, force: true });
      return;
    } catch (error) {
      if (i === attempts - 1) {
        console.warn(`[${TAG}] could not remove ${dir}: ${error.message}`);
        return;
      }
      await delay(300);
    }
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
