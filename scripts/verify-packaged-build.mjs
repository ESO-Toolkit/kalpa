import { execFile } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import {
  CDP_ENDPOINT,
  CDP_PAGES_URL,
  CDP_VERSION_URL,
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
const execFileAsync = promisify(execFile);
// Derived from CDP_ENDPOINT rather than written out again: netstat matches on
// the bare number, and a second literal 9222 here would silently stop matching
// the day the harness moves the port.
const CDP_PORT = Number(new URL(CDP_ENDPOINT).port);

async function main() {
  if (process.platform !== "win32") {
    throw new Error("The packaged build gate is Windows-only because it verifies WebView2 CDP.");
  }

  await assertNoForeignCdpListener();
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
 * Refuse to start when something already owns the CDP port, and say WHO owns it.
 *
 * Detection is unchanged from the harness's assertNoExistingCdp() -- one probe
 * of /json/version, and any answer at all is disqualifying. Only the message
 * is different, and that is the whole point: "Stop tauri dev or any existing
 * debug Kalpa process first" names no process, and during the beta.23 release
 * that cost hours. A sibling worktree's dev instance held 9222 and the gate
 * gave the maintainer nothing to look for on a machine where several checkouts
 * each run their own app.
 *
 * Still fail-closed, deliberately. Attaching to a foreign listener would drive
 * whatever app is on the other end -- someone else's build, pointed at
 * whatever AddOns folder THEIR worktree configured -- and report green for
 * code this gate never loaded. A blocked release is cheaper than a gate that
 * lies about one.
 *
 * The owner is reported as a chain because the process holding the port is
 * almost never the one anyone recognises: WebView2 binds it from
 * msedgewebview2.exe, and the thing that has to be closed is its parent.
 */
async function assertNoForeignCdpListener() {
  let version = null;
  try {
    version = await httpJson(CDP_VERSION_URL, 1_000);
  } catch {
    return;
  }

  const owners = await describeCdpPortOwners();
  const browser = typeof version?.Browser === "string" ? version.Browser : "";

  throw new Error(
    [
      `${CDP_ENDPOINT} is already answering, so port ${CDP_PORT} was taken before this gate launched anything.`,
      owners.length
        ? `Holding it: ${owners.join("; ")}.`
        : `Could not identify the owner: netstat reported no LISTENING line for port ${CDP_PORT}, or the process belongs to another user. Check by hand with: netstat -ano | findstr :${CDP_PORT}`,
      browser ? `The listener identifies itself as ${browser}.` : "",
      "Close it and re-run. It is a `npm run tauri dev`, another `npm run test:packaged`, or a leftover debug Kalpa -- most often from a different worktree on this machine.",
      "This gate will not attach to a listener it did not launch, because a pass against someone else's app would say nothing about this build.",
    ]
      .filter(Boolean)
      .join(" ")
  );
}

/**
 * PID, image name and executable path for whatever is LISTENING on the CDP
 * port, plus the process that spawned it.
 *
 * Every lookup degrades to a shorter answer instead of throwing. This only
 * ever runs while assembling an error message, and letting a secondary failure
 * ("powershell is not recognized") replace the real one is how a diagnostic
 * turns back into the terse message it was meant to replace.
 *
 * The parent is a hint, not proof: Windows recycles PIDs, so a listener
 * whose real parent has already exited can be attributed to whatever now
 * holds that number. The listener line above it is the one to trust.
 */
async function describeCdpPortOwners() {
  const described = [];
  for (const pid of await findPortListenerPids(CDP_PORT)) {
    const listener = await describeProcess(pid);
    const parent = listener?.parentPid ? await describeProcess(listener.parentPid) : null;
    described.push(
      parent
        ? `${formatProcess(pid, listener)}, launched by ${formatProcess(parent.pid, parent)}`
        : formatProcess(pid, listener)
    );
  }
  return described;
}

function formatProcess(pid, info) {
  if (!info) return `PID ${pid} (image and path unavailable)`;
  const location = info.path || "path unavailable -- elevated or protected process";
  return `PID ${pid} ${info.name} (${location})`;
}

/** PIDs LISTENING on a TCP port, or [] if netstat is unavailable. */
async function findPortListenerPids(port) {
  let stdout = "";
  try {
    ({ stdout } = await execFileAsync("netstat", ["-ano", "-p", "TCP"], { windowsHide: true }));
  } catch {
    return [];
  }
  const pids = new Set();
  for (const line of stdout.split(/\r?\n/)) {
    // Columns are proto, local address, foreign address, state, PID. Match the
    // LOCAL address only: an outbound connection to some other host's :9222
    // also contains ":9222" and would otherwise be blamed for holding ours.
    const parts = line.trim().split(/\s+/);
    if (parts.length < 5 || parts[3].toUpperCase() !== "LISTENING") continue;
    if (!parts[1].endsWith(`:${port}`)) continue;
    const pid = Number(parts[4]);
    if (Number.isInteger(pid) && pid > 0) pids.add(pid);
  }
  return [...pids];
}

/**
 * Win32_Process rather than tasklist: tasklist reports the image name but
 * never the full path, and the path is the only thing that distinguishes one
 * worktree's kalpa.exe from another's -- which is exactly the question that
 * went unanswered during beta.23.
 */
async function describeProcess(pid) {
  // Guarded before interpolation: this integer is spliced into a PowerShell
  // filter string, and it arrives from parsed netstat output.
  if (!Number.isInteger(pid) || pid <= 0) return null;
  const script =
    `Get-CimInstance Win32_Process -Filter "ProcessId=${pid}" | ` +
    "Select-Object -First 1 -Property Name,ExecutablePath,ParentProcessId | ConvertTo-Json -Compress";
  try {
    const { stdout } = await execFileAsync(
      "powershell",
      ["-NoProfile", "-NonInteractive", "-Command", script],
      { windowsHide: true, timeout: 10_000 }
    );
    const parsed = JSON.parse(stdout.trim());
    return {
      pid,
      name: typeof parsed?.Name === "string" ? parsed.Name : "unknown image",
      path: typeof parsed?.ExecutablePath === "string" ? parsed.ExecutablePath : "",
      // 0 reads as "no parent to chase", which is also what a dead or recycled
      // parent should produce.
      parentPid: Number.isInteger(parsed?.ParentProcessId) ? parsed.ParentProcessId : 0,
    };
  } catch {
    return null;
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
