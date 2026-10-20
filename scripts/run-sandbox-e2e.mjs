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

async function runMinion4Regression() {
  const profile = createIsolatedProfile();
  let child;
  try {
    const fixtures = path.join(profile.root, "fixtures");
    const minion = path.join(fixtures, "minion");
    mkdirSync(fixtures);
    mkdirSync(minion);
    mkdirSync(path.join(minion, "config"));
    const folder = "KalpaMinionFixture";
    const title = "Kalpa Minion Fixture";
    const addon = path.join(profile.addons, folder);
    mkdirSync(addon);
    writeFileSync(
      path.join(addon, `${folder}.txt`),
      `## Title: ${title}\n## APIVersion: 101048\n## Version: manifest-2\n## Author: Kalpa E2E\n`
    );
    // Minion 4 leaves its imported Minion 3 database behind. Both are
    // intentionally present so a fallback to the stale XML fails this gate.
    writeFileSync(
      path.join(minion, "minion.xml"),
      `<minion>\n<game addon-path="${Buffer.from(profile.addons).toString("base64")}">\n<addon uid="123" ui-version="1.0">\n<dir>${folder}</dir>\n</addon>\n</game>\n</minion>\n`
    );
    // Sanitized Minion 4 ESOPluginData/ESOLocalAddOnBundleData schema.
    writeFileSync(
      path.join(minion, "config", "eso.json"),
      JSON.stringify({
        installedGames: [
          {
            installPath: path.dirname(profile.addons),
            installedBundles: {
              "esoui:123": {
                version: "2.0",
                addOns: { [folder]: { path: folder, version: "manifest-2" } },
              },
            },
          },
        ],
      })
    );
    // ApiFileEntry: local catalogue evidence, never a production download.
    writeFileSync(
      path.join(fixtures, "filelist.json"),
      JSON.stringify([
        {
          id: 123,
          categoryId: 1,
          version: "2.0",
          lastUpdate: 2000,
          title,
          author: "Kalpa E2E",
          fileInfoUri: "https://www.esoui.com/downloads/info123.html",
          addons: [{ path: folder }],
        },
      ])
    );
    child = launchKalpaDetached(binaryPath, {
      tag: TAG,
      env: { ...profile.env, KALPA_E2E_FIXTURES: fixtures },
    });
    await proveOwnedLaunch(child);
    await waitForCdp(child, 120_000);
    await run(
      process.execPath,
      [playwrightCli, "test", "--grep", "@minion4"],
      "playwright Minion 4 regression",
      {
        cwd: repoRoot,
        env: {
          ...profile.testEnv,
          KALPA_CDP_ENDPOINT: CDP_ENDPOINT,
          KALPA_E2E_FIXTURES: fixtures,
        },
        tag: TAG,
      }
    );
  } finally {
    if (child?.pid) await killProcessTree(child.pid, TAG);
    await profile.cleanup();
  }
}

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

  await runMinion4Regression();
  await assertNoExistingCdp();
  await assertNoExistingKalpaProcess();

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
      [playwrightCli, "test", "--grep", "@sandbox", "--grep-invert", "@minion4"],
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
