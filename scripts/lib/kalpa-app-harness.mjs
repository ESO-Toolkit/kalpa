/**
 * Shared plumbing for the scripts that launch a debug Kalpa build and drive it
 * over CDP: the packaged-build release gate and the sandboxed e2e runner.
 *
 * Both need the same things — own the only Kalpa process, wait for WebView2's
 * debug port, and tear the process tree down afterwards — and both are
 * Windows-only for the same reason: the CDP endpoint comes from
 * `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, which WebKitGTK/WKWebView do not have.
 */

import { execFile, spawn } from "node:child_process";
import http from "node:http";
import path from "node:path";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

/**
 * 127.0.0.1, deliberately not `localhost`.
 *
 * WebView2's `--remote-debugging-port` server binds IPv4 loopback only. On a
 * host where `localhost` resolves to `::1` first — GitHub's Windows runners do —
 * every probe gets ECONNREFUSED against IPv6 while the port is wide open on
 * IPv4, so the app looks dead when it is perfectly healthy.
 */
export const CDP_ENDPOINT = "http://127.0.0.1:9222";
export const CDP_VERSION_URL = `${CDP_ENDPOINT}/json/version`;
export const CDP_PAGES_URL = `${CDP_ENDPOINT}/json/list`;

export function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Run a command to completion, inheriting stdio, rejecting on non-zero exit. */
export function run(command, args, label, { cwd, env = {}, tag = "kalpa" } = {}) {
  console.log(`[${tag}] ${label}`);
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd,
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

/**
 * Launch the debug binary detached, with CDP enabled.
 *
 * `KALPA_FORCE_WEBVIEW` keeps the native Slint sidecar out of the way: without
 * it a machine with performance mode enabled starts the sidecar instead, and
 * there is no webview to attach to.
 */
export function launchKalpaDetached(binaryPath, { env = {}, tag = "kalpa" } = {}) {
  console.log(`[${tag}] launch ${binaryPath}`);
  const child = spawn(binaryPath, [], {
    cwd: path.dirname(binaryPath),
    detached: true,
    env: { ...process.env, KALPA_FORCE_WEBVIEW: "1", ...env },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });

  child.stdout?.on("data", (chunk) => process.stdout.write(`[${tag}] ${chunk}`));
  child.stderr?.on("data", (chunk) => process.stderr.write(`[${tag}] ${chunk}`));
  child.on("error", (error) => {
    throw error;
  });

  if (!child.pid) {
    throw new Error("Kalpa process did not expose a PID.");
  }
  return child;
}

/**
 * Prove the launch produced OUR process and no other.
 *
 * Kalpa is single-instance: a stray running copy absorbs the launch and exits
 * the child immediately, which would otherwise leave the gate driving whatever
 * app was already open — against whatever AddOns folder IT was pointed at.
 */
export async function proveOwnedLaunch(child) {
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

export async function waitForCdp(child, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError = "CDP did not respond";
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`Kalpa exited before CDP became available with code ${child.exitCode}.`);
    }
    try {
      await httpJson(CDP_VERSION_URL, 2_000);
      await httpJson(CDP_PAGES_URL, 2_000);
      return;
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error);
      await delay(500);
    }
  }
  // Say WHICH failure this was. "Timed out" alone cannot distinguish a WebView2
  // that never started (no desktop, no runtime) from one that started but never
  // bound the debug port — and those want completely different fixes.
  const webviewCount = await countProcesses("msedgewebview2.exe");
  const listeners = await describePortListeners(9222);
  throw new Error(
    `Timed out waiting for ${CDP_ENDPOINT} after ${Math.round(timeoutMs / 1000)}s: ${lastError || "connection refused"}. ` +
      `Kalpa is ${child.exitCode === null ? "still running" : `gone (exit ${child.exitCode})`}; ` +
      `msedgewebview2.exe processes: ${webviewCount}; ` +
      `listeners on 9222: ${listeners || "none"}. ` +
      (webviewCount === 0
        ? "Zero webview processes means WebView2 never initialized — no usable runtime or desktop session."
        : listeners
          ? "Something IS listening, so this is an address/protocol mismatch, not a dead app."
          : "WebView2 is up but nothing bound the port — the debug argument never reached it.")
  );
}

/** `netstat` LISTENING lines for a port, so a bind-address mismatch is visible. */
async function describePortListeners(port) {
  try {
    const { stdout } = await execFileAsync("netstat", ["-ano", "-p", "TCP"], {
      windowsHide: true,
    });
    return stdout
      .split(/\r?\n/)
      .filter((line) => line.includes(`:${port}`) && /LISTENING/i.test(line))
      .map((line) => line.trim())
      .join(" | ");
  } catch {
    return "";
  }
}

/** How many processes with this image name are running (0 on any error). */
export async function countProcesses(imageName) {
  try {
    const { stdout } = await execFileAsync(
      "tasklist",
      ["/FI", `IMAGENAME eq ${imageName}`, "/FO", "CSV", "/NH"],
      { windowsHide: true }
    );
    return stdout
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line && !line.includes("No tasks")).length;
  } catch {
    return 0;
  }
}

export async function assertNoExistingCdp() {
  try {
    await httpJson(CDP_VERSION_URL, 1_000);
  } catch {
    return;
  }
  throw new Error(
    `${CDP_ENDPOINT} is already responding before the gate launched Kalpa. Stop tauri dev or any existing debug Kalpa process first.`
  );
}

export async function assertNoExistingKalpaProcess() {
  const pids = await findKalpaPids();
  if (pids.length > 0) {
    throw new Error(
      `kalpa.exe is already running (${pids.join(", ")}). The gate must launch and own the process it tests.`
    );
  }
}

export async function findKalpaPids() {
  const { stdout } = await execFileAsync(
    "tasklist",
    ["/FI", "IMAGENAME eq kalpa.exe", "/FO", "CSV", "/NH"],
    { windowsHide: true }
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

export async function killProcessTree(pid, tag = "kalpa") {
  console.log(`[${tag}] teardown PID ${pid}`);
  try {
    await execFileAsync("taskkill", ["/PID", String(pid), "/T", "/F"], { windowsHide: true });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (!message.includes("not found")) {
      console.warn(`[${tag}] taskkill warning: ${message}`);
    }
  }
}

export function httpJson(url, timeoutMs) {
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
