import { execFile, spawn } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const cdpEndpoint = "http://localhost:9222";
const cdpVersionUrl = `${cdpEndpoint}/json/version`;
const cdpPagesUrl = `${cdpEndpoint}/json/list`;
const binaryPath = path.join(repoRoot, "src-tauri", "target", "debug", "kalpa.exe");
const tauriCli = path.join(repoRoot, "node_modules", "@tauri-apps", "cli", "tauri.js");
const playwrightCli = path.join(repoRoot, "node_modules", "@playwright", "test", "cli.js");

async function main() {
  if (process.platform !== "win32") {
    throw new Error("The packaged build gate is Windows-only because it verifies WebView2 CDP.");
  }

  await assertNoExistingCdp();
  await assertNoExistingKalpaProcess();
  await run(process.execPath, [tauriCli, "build", "--debug", "--no-bundle"], "tauri build");
  assertBuildArtifacts();

  let child;
  try {
    child = launchKalpaDetached();
    await proveOwnedLaunch(child);
    await waitForCdp(child);
    await assertPackagedOrigin();
    await run(
      process.execPath,
      [playwrightCli, "test", "--grep", "@packaged"],
      "playwright packaged tests",
      { KALPA_CDP_ENDPOINT: cdpEndpoint }
    );
  } finally {
    if (child?.pid) {
      await killProcessTree(child.pid);
    }
  }
}

async function run(command, args, label, env = {}) {
  console.log(`[packaged] ${label}`);
  await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: repoRoot,
      env: { ...process.env, ...env },
      stdio: "inherit",
      windowsHide: true,
    });
    child.on("error", reject);
    child.on("exit", (code, signal) => {
      if (code === 0) resolve();
      else reject(new Error(`${label} failed with ${signal ?? `exit code ${code}`}`));
    });
  });
}

function launchKalpaDetached() {
  console.log(`[packaged] launch ${path.relative(repoRoot, binaryPath)}`);
  const child = spawn(binaryPath, [], {
    cwd: path.dirname(binaryPath),
    detached: true,
    env: { ...process.env, KALPA_FORCE_WEBVIEW: "1" },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });

  child.stdout?.on("data", (chunk) => process.stdout.write(`[kalpa] ${chunk}`));
  child.stderr?.on("data", (chunk) => process.stderr.write(`[kalpa] ${chunk}`));
  child.on("error", (error) => {
    throw error;
  });

  if (!child.pid) {
    throw new Error("Kalpa process did not expose a PID.");
  }
  return child;
}

async function proveOwnedLaunch(child) {
  await delay(750);
  if (child.exitCode !== null) {
    throw new Error(
      `Kalpa exited immediately with code ${child.exitCode}; a running single-instance app may have absorbed the launch.`
    );
  }

  const pids = await findKalpaPids();
  if (!pids.includes(child.pid)) {
    throw new Error(
      `Owned Kalpa PID ${child.pid} is not running; active kalpa.exe PIDs: ${pids.join(", ") || "none"}.`
    );
  }
  const foreign = pids.filter((pid) => pid !== child.pid);
  if (foreign.length > 0) {
    throw new Error(
      `Found pre-existing kalpa.exe process(es) after launch: ${foreign.join(", ")}. The gate must own the only Kalpa process.`
    );
  }
}

async function waitForCdp(child) {
  const deadline = Date.now() + 30_000;
  let lastError = "CDP did not respond";
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`Kalpa exited before CDP became available with code ${child.exitCode}.`);
    }
    try {
      await httpJson(cdpVersionUrl, 2_000);
      await httpJson(cdpPagesUrl, 2_000);
      return;
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error);
      await delay(500);
    }
  }
  throw new Error(`Timed out waiting for ${cdpEndpoint}: ${lastError}`);
}

async function assertPackagedOrigin() {
  const pages = await httpJson(cdpPagesUrl, 2_000);
  const urls = Array.isArray(pages)
    ? pages.map((page) => (typeof page?.url === "string" ? page.url : ""))
    : [];
  if (!urls.some((url) => url === "http://tauri.localhost/")) {
    throw new Error(
      `Packaged origin was not present in CDP pages. Saw: ${urls.join(", ") || "none"}. A dev-server run must fail here.`
    );
  }
}

async function assertNoExistingCdp() {
  try {
    await httpJson(cdpVersionUrl, 1_000);
  } catch {
    return;
  }
  throw new Error(
    `${cdpEndpoint} is already responding before the gate launched Kalpa. Stop tauri dev or any existing debug Kalpa process first.`
  );
}

async function assertNoExistingKalpaProcess() {
  const pids = await findKalpaPids();
  if (pids.length > 0) {
    throw new Error(
      `kalpa.exe is already running (${pids.join(", ")}). The packaged gate must launch and own the process it tests.`
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

async function findKalpaPids() {
  const { stdout } = await execFileAsync(
    "tasklist",
    ["/FI", "IMAGENAME eq kalpa.exe", "/FO", "CSV", "/NH"],
    {
      windowsHide: true,
    }
  );
  return stdout
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.includes("No tasks"))
    .map((line) => parseCsvLine(line)[1])
    .map((pid) => Number(pid))
    .filter((pid) => Number.isInteger(pid) && pid > 0);
}

function parseCsvLine(line) {
  const values = [];
  let current = "";
  let quoted = false;
  for (let i = 0; i < line.length; i++) {
    const char = line[i];
    if (char === '"' && line[i + 1] === '"') {
      current += '"';
      i += 1;
    } else if (char === '"') {
      quoted = !quoted;
    } else if (char === "," && !quoted) {
      values.push(current);
      current = "";
    } else {
      current += char;
    }
  }
  values.push(current);
  return values;
}

async function killProcessTree(pid) {
  console.log(`[packaged] teardown PID ${pid}`);
  try {
    await execFileAsync("taskkill", ["/PID", String(pid), "/T", "/F"], { windowsHide: true });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (!message.includes("not found")) {
      console.warn(`[packaged] taskkill warning: ${message}`);
    }
  }
}

function httpJson(url, timeoutMs) {
  return new Promise((resolve, reject) => {
    const request = http.get(url, { timeout: timeoutMs }, (response) => {
      let body = "";
      response.setEncoding("utf8");
      response.on("data", (chunk) => {
        body += chunk;
      });
      response.on("end", () => {
        if ((response.statusCode ?? 0) < 200 || (response.statusCode ?? 0) >= 300) {
          reject(new Error(`${url} returned HTTP ${response.statusCode}`));
          return;
        }
        try {
          resolve(JSON.parse(body));
        } catch (error) {
          reject(error);
        }
      });
    });
    request.on("timeout", () => {
      request.destroy(new Error(`${url} timed out`));
    });
    request.on("error", reject);
  });
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
