import { existsSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  CDP_ENDPOINT,
  CDP_PAGES_URL,
  assertNoExistingCdp,
  assertNoExistingKalpaProcess,
  httpJson,
  killProcessTree,
  launchKalpaDetached,
  proveOwnedLaunch,
  run,
  waitForCdp,
} from "./lib/kalpa-app-harness.mjs";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const binaryPath = path.join(repoRoot, "src-tauri", "target", "debug", "kalpa.exe");
const tauriCli = path.join(repoRoot, "node_modules", "@tauri-apps", "cli", "tauri.js");
const playwrightCli = path.join(repoRoot, "node_modules", "@playwright", "test", "cli.js");
const TAG = "packaged";

async function main() {
  if (process.platform !== "win32") {
    throw new Error("The packaged build gate is Windows-only because it verifies WebView2 CDP.");
  }

  await assertNoExistingCdp();
  await assertNoExistingKalpaProcess();
  await run(process.execPath, [tauriCli, "build", "--debug", "--no-bundle"], "tauri build", {
    cwd: repoRoot,
    tag: TAG,
  });
  assertBuildArtifacts();

  let child;
  try {
    child = launchKalpaDetached(binaryPath, { tag: TAG });
    await proveOwnedLaunch(child);
    await waitForCdp(child);
    await assertPackagedOrigin();
    await run(
      process.execPath,
      [playwrightCli, "test", "--grep", "@packaged"],
      "playwright packaged tests",
      { cwd: repoRoot, env: { KALPA_CDP_ENDPOINT: CDP_ENDPOINT }, tag: TAG }
    );
  } finally {
    if (child?.pid) {
      await killProcessTree(child.pid, TAG);
    }
  }
}

/**
 * Wait for the app's own page, then insist it is the packaged origin.
 *
 * POLLED, and that is the whole point. `waitForCdp` only proves the debug port
 * answers `/json/version` and `/json/list`, which WebView2 does the moment it
 * opens — while the only target is still `about:blank`. Checking once here
 * turned that startup race into a hard failure that reads like a product bug:
 * "Packaged origin was not present in CDP pages. Saw: about:blank."
 *
 * The identical mistake was fixed in `connectToTauriAt` (e2e/helpers.ts), where
 * two single-shot pre-checks sat above the poll loop added to survive exactly
 * this. Fixing it there and not here left the same defect one file away.
 *
 * A dev-server run must still fail: the deadline expires having seen only
 * `http://127.0.0.1:1430/`, and the message says so.
 */
async function assertPackagedOrigin() {
  const deadline = Date.now() + 20_000;
  let seen = [];

  while (Date.now() < deadline) {
    const pages = await httpJson(CDP_PAGES_URL, 2_000).catch(() => null);
    seen = Array.isArray(pages)
      ? pages.map((page) => (typeof page?.url === "string" ? page.url : "")).filter(Boolean)
      : [];
    if (seen.some((url) => url === "http://tauri.localhost/")) return;
    // A dev-server origin is not a race — it is the thing this gate exists to
    // catch — so fail immediately rather than burning the full deadline.
    if (seen.some((url) => /^https?:\/\/(127\.0\.0\.1|localhost):\d+\//.test(url))) break;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }

  throw new Error(
    `Packaged origin never appeared in CDP pages within 20s. Saw: ${seen.join(", ") || "none"}. ` +
      `A dev-server run must fail here.`
  );
}

function assertBuildArtifacts() {
  if (!existsSync(binaryPath)) {
    throw new Error(`Expected debug binary at ${binaryPath}`);
  }
  const assetsDir = path.join(repoRoot, "dist", "assets");
  if (
    !existsSync(assetsDir) ||
    readdirSync(assetsDir).filter((name) => name.endsWith(".js")).length === 0
  ) {
    throw new Error("Expected built frontend JS assets under dist/assets.");
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
