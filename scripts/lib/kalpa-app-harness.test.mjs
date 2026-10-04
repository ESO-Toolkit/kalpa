import assert from "node:assert/strict";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { createIsolatedProfile } from "./kalpa-app-harness.mjs";

function assertOwnedPath(root, target) {
  assert.ok(path.isAbsolute(target), `cleanup target must be absolute: ${target}`);
  const relative = path.relative(root, target);
  assert.ok(
    !path.isAbsolute(relative) && relative !== ".." && !relative.startsWith(`..${path.sep}`),
    `cleanup target must stay within the owned temporary root: ${target}`
  );
}

test(
  "isolated profiles keep production settings and each other's files intact",
  { skip: process.platform === "win32" ? false : "profile creation is Windows-only" },
  async () => {
    const root = path.resolve(mkdtempSync(path.join(os.tmpdir(), "kalpa-profile-test-")));
    const savedEnv = Object.fromEntries(
      ["APPDATA", "LOCALAPPDATA", "TEMP", "TMP"].map((key) => [key, process.env[key]])
    );
    const roaming = path.join(root, "Roaming");
    const local = path.join(root, "Local");
    const temp = path.join(root, "Temp");
    const profiles = [];
    try {
      for (const directory of [roaming, local, temp]) mkdirSync(directory);
      Object.assign(process.env, { APPDATA: roaming, LOCALAPPDATA: local, TEMP: temp, TMP: temp });
      assert.equal(path.resolve(os.tmpdir()), temp);

      const production = path.join(roaming, "com.kalpa.desktop");
      mkdirSync(production);
      const sentinel = path.join(production, "settings.json");
      const settings = '{"sentinel":"production settings must survive"}';
      writeFileSync(sentinel, settings);

      for (let index = 0; index < 2; index++) {
        const profile = createIsolatedProfile();
        profiles.push(profile);
        const token = profile.env.KALPA_E2E_TOKEN;
        const identifier = `com.kalpa.desktop.e2e.${token}`;
        const localData = path.join(local, identifier);
        for (const target of [profile.root, profile.addons, profile.appData, localData]) {
          assertOwnedPath(root, target);
          assert.ok(existsSync(target));
        }
        assert.equal(profile.root, path.join(temp, `kalpa-e2e-${token}`));
        assert.equal(profile.addons, path.join(profile.root, "AddOns"));
        assert.equal(profile.appData, path.join(roaming, identifier));
        assert.equal(profile.env.KALPA_ADDONS_DIR, profile.addons);
        assert.equal(profile.testEnv.KALPA_E2E_SANDBOX_DIR, profile.addons);
        assert.equal(profile.testEnv.KALPA_E2E_APP_DATA, profile.appData);
        assert.equal(readFileSync(path.join(profile.addons, ".kalpa-e2e-sandbox"), "utf8"), token);
        assert.deepEqual(readdirSync(profile.appData), []);
        assert.deepEqual(readdirSync(localData), []);
      }

      const [first, second] = profiles;
      assert.notEqual(first.env.KALPA_E2E_TOKEN, second.env.KALPA_E2E_TOKEN);
      assert.notEqual(first.root, second.root);
      assert.notEqual(first.appData, second.appData);
      const secondSettings = path.join(second.appData, "settings.json");
      writeFileSync(secondSettings, "second profile must survive first cleanup");

      await first.cleanup();
      for (const target of [
        first.root,
        first.appData,
        path.join(local, `com.kalpa.desktop.e2e.${first.env.KALPA_E2E_TOKEN}`),
      ])
        assert.equal(existsSync(target), false);
      assert.ok(existsSync(second.root));
      assert.ok(
        existsSync(path.join(local, `com.kalpa.desktop.e2e.${second.env.KALPA_E2E_TOKEN}`))
      );
      assert.equal(
        readFileSync(secondSettings, "utf8"),
        "second profile must survive first cleanup"
      );
      assert.equal(readFileSync(sentinel, "utf8"), settings);

      await second.cleanup();
      assert.deepEqual(readdirSync(temp), []);
      assert.deepEqual(readdirSync(local), []);
      assert.deepEqual(readdirSync(roaming), ["com.kalpa.desktop"]);
      assert.equal(readFileSync(sentinel, "utf8"), settings);
    } finally {
      try {
        for (const profile of profiles) {
          for (const target of [
            profile.root,
            profile.appData,
            path.join(local, `com.kalpa.desktop.e2e.${profile.env.KALPA_E2E_TOKEN}`),
          ])
            assertOwnedPath(root, target);
          await profile.cleanup();
        }
      } finally {
        for (const [key, value] of Object.entries(savedEnv)) {
          if (value === undefined) delete process.env[key];
          else process.env[key] = value;
        }
        assertOwnedPath(root, root);
        rmSync(root, { recursive: true, force: true });
      }
    }
  }
);
