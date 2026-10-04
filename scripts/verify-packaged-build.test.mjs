/**
 * Covers the CDP-port preflight in verify-packaged-build.mjs.
 *
 * `npm run test:packaged` is a required pre-tag gate, and the thing that most
 * often stops it is not the app at all -- it is another worktree's `tauri dev`
 * still holding 127.0.0.1:9222. The old message said only "Stop tauri dev or
 * any existing debug Kalpa process first", which during the beta.23 release
 * left a maintainer hunting the right window for hours. This test pins the two
 * properties that fix cost: the gate still refuses to run (it must never drive
 * an app it did not launch), and the refusal names the process holding the
 * port.
 *
 * It drives the real script rather than an exported helper because the
 * preflight's whole job is to run before anything else does -- including the
 * ~20-minute `tauri build` -- and that ordering is only observable from
 * outside. Cheap for the same reason: the gate exits before it builds.
 */

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import http from "node:http";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const gateScript = path.join(repoRoot, "scripts", "verify-packaged-build.mjs");
const CDP_PORT = 9222;

/**
 * A stand-in for the foreign listener, answering /json/version the way
 * WebView2 does. The gate's probe only needs valid JSON on that route, so this
 * reproduces the collision without needing a second Kalpa build.
 */
function startFakeCdpListener() {
  const server = http.createServer((request, response) => {
    response.setHeader("content-type", "application/json");
    response.end(JSON.stringify({ Browser: "FakeCdpListener/1.0" }));
  });
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(CDP_PORT, "127.0.0.1", () => resolve(server));
  });
}

function runGate() {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [gateScript], { cwd: repoRoot, windowsHide: true });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => (stdout += chunk));
    child.stderr.on("data", (chunk) => (stderr += chunk));
    child.on("error", reject);
    child.on("close", (code) => resolve({ code, stdout, stderr }));
  });
}

test(
  "packaged gate refuses a foreign CDP listener and names the process holding the port",
  { skip: process.platform === "win32" ? false : "the packaged gate is Windows-only" },
  async () => {
    let server;
    try {
      server = await startFakeCdpListener();
    } catch (error) {
      assert.fail(
        `Could not bind 127.0.0.1:${CDP_PORT} for this test (${error.code ?? error.message}). ` +
          "Close any tauri dev or packaged run and try again."
      );
    }

    try {
      const { code, stdout, stderr } = await runGate();

      // Fail closed: never attach to a listener the gate did not launch.
      assert.equal(code, 1, `gate should exit 1; stdout was: ${stdout}`);
      // And fail BEFORE the expensive part, or the diagnostic arrives 20
      // minutes late.
      assert.ok(
        !stdout.includes("tauri build"),
        `preflight must reject before the build starts; stdout was: ${stdout}`
      );

      // Name the owner. This test process is the one holding the port, so the
      // message has to point straight at it. The PID is always available and
      // is what makes the message actionable, so it is asserted unconditionally.
      assert.ok(
        stderr.includes(`PID ${process.pid} `),
        `message should name PID ${process.pid}; got: ${stderr}`
      );

      // The image and path come from a Windows process lookup that is not
      // always permitted -- a hosted runner can refuse it, and the gate then
      // says "(image and path unavailable)" rather than guessing. That is
      // deliberate behaviour, so accept it here; asserting otherwise makes
      // this test fail for a reason that has nothing to do with the gate.
      // When the lookup DOES work, hold it to naming both: the full path is
      // the part that tells two worktrees apart.
      if (!stderr.includes("(image and path unavailable)")) {
        assert.ok(
          stderr.includes(path.basename(process.execPath)),
          `message should name the image ${path.basename(process.execPath)}; got: ${stderr}`
        );
        assert.ok(
          stderr.includes(process.execPath),
          `message should name the executable path ${process.execPath}; got: ${stderr}`
        );
      }

      // Say plainly what has to be closed.
      assert.match(stderr, /tauri dev/, `message should name tauri dev; got: ${stderr}`);
      assert.match(stderr, /test:packaged/, `message should name test:packaged; got: ${stderr}`);
    } finally {
      await new Promise((resolve) => server.close(resolve));
    }
  }
);
