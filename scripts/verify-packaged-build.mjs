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

async function assertPackagedOrigin() {
  const pages = await httpJson(CDP_PAGES_URL, 2_000);
  const urls = Array.isArray(pages)
    ? pages.map((page) => (typeof page?.url === "string" ? page.url : ""))
    : [];
  if (!urls.some((url) => url === "http://tauri.localhost/")) {
    throw new Error(
      `Packaged origin was not present in CDP pages. Saw: ${urls.join(", ") || "none"}. A dev-server run must fail here.`
    );
  }
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
