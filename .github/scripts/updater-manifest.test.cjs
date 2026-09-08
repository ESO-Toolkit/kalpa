const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  REQUIRED_PLATFORMS,
  buildUpdaterManifest,
  collectSignedAssets,
  platformKeysFor,
  versionFromTag,
} = require("./updater-manifest.cjs");

const TAG = "v0.1.0-beta.22";
const REPO = "ESO-Toolkit/kalpa";

// The exact asset names tauri-action uploaded for v0.1.0-beta.22.
const SIGNED_ASSETS = [
  "Kalpa_0.1.0-beta.22_x64-setup.exe",
  "Kalpa_0.1.0-beta.22_universal.app.tar.gz",
  "Kalpa_0.1.0-beta.22_amd64.AppImage",
  "Kalpa_0.1.0-beta.22_amd64.deb",
  "Kalpa-0.1.0-beta.22-1.x86_64.rpm",
];

function assets(names = SIGNED_ASSETS) {
  return names.map((name) => ({ name, signature: `sig-of-${name}\n` }));
}

function manifest(overrides = {}) {
  return buildUpdaterManifest({
    tag: TAG,
    repo: REPO,
    assets: assets(),
    notes: "notes",
    now: new Date("2026-08-31T09:16:25.200Z"),
    ...overrides,
  });
}

test("reproduces every platform key tauri-action wrote for beta.22", () => {
  const { platforms } = manifest();
  assert.deepEqual(Object.keys(platforms).sort(), [
    "darwin-aarch64",
    "darwin-aarch64-app",
    "darwin-universal",
    "darwin-universal-app",
    "darwin-x86_64",
    "darwin-x86_64-app",
    "linux-x86_64",
    "linux-x86_64-appimage",
    "linux-x86_64-deb",
    "linux-x86_64-rpm",
    "windows-x86_64",
    "windows-x86_64-nsis",
  ]);
  for (const key of REQUIRED_PLATFORMS) assert.ok(platforms[key], key);
});

test("orders platforms Windows, macOS, Linux however the assets were listed", () => {
  const { platforms } = manifest({ assets: assets([...SIGNED_ASSETS].reverse()) });
  assert.deepEqual(Object.keys(platforms).slice(0, 3), [
    "windows-x86_64",
    "windows-x86_64-nsis",
    "darwin-aarch64",
  ]);
  assert.equal(Object.keys(platforms).at(-1), "linux-x86_64-rpm");
});

test("points each key at the public download URL of the asset that signs it", () => {
  const { platforms } = manifest();
  const base = `https://github.com/${REPO}/releases/download/${TAG}`;
  assert.deepEqual(platforms["windows-x86_64"], {
    signature: "sig-of-Kalpa_0.1.0-beta.22_x64-setup.exe\n",
    url: `${base}/Kalpa_0.1.0-beta.22_x64-setup.exe`,
  });
  // Both Mac arches and the universal keys resolve to the one universal bundle.
  for (const key of ["darwin-aarch64", "darwin-x86_64", "darwin-universal-app"]) {
    assert.equal(platforms[key].url, `${base}/Kalpa_0.1.0-beta.22_universal.app.tar.gz`);
  }
  assert.equal(platforms["linux-x86_64"].url, `${base}/Kalpa_0.1.0-beta.22_amd64.AppImage`);
  assert.equal(platforms["linux-x86_64-deb"].url, `${base}/Kalpa_0.1.0-beta.22_amd64.deb`);
  assert.equal(platforms["linux-x86_64-rpm"].url, `${base}/Kalpa-0.1.0-beta.22-1.x86_64.rpm`);
});

test("writes the version without the tag's v, the notes, and an RFC 3339 pub_date", () => {
  const result = manifest();
  assert.equal(result.version, "0.1.0-beta.22");
  assert.equal(result.notes, "notes");
  assert.equal(result.pub_date, "2026-08-31T09:16:25.200Z");
  assert.deepEqual(Object.keys(result), ["version", "notes", "pub_date", "platforms"]);
});

test("keeps signatures verbatim, trailing newline included", () => {
  const { platforms } = manifest({
    assets: assets().map((asset) => ({ ...asset, signature: "untrusted comment: x\nabc==\n" })),
  });
  assert.equal(platforms["linux-x86_64"].signature, "untrusted comment: x\nabc==\n");
});

test("fails closed when a required platform has no signed asset", () => {
  assert.throws(
    () => manifest({ assets: assets(SIGNED_ASSETS.filter((n) => !n.endsWith(".exe"))) }),
    /missing required platform\(s\): windows-x86_64/
  );
  assert.throws(
    () => manifest({ assets: assets(SIGNED_ASSETS.filter((n) => !n.includes("universal"))) }),
    /darwin-x86_64, darwin-aarch64/
  );
  assert.throws(
    () => manifest({ assets: assets(SIGNED_ASSETS.filter((n) => !n.endsWith(".AppImage"))) }),
    /linux-x86_64/
  );
});

test("refuses signed assets it cannot map to a platform rather than guessing", () => {
  assert.throws(() => platformKeysFor("Kalpa_0.1.0-beta.22_x64_en-US.msi"), /no known updater/);
  assert.throws(
    () => manifest({ assets: assets([...SIGNED_ASSETS, "Kalpa_0.1.0-beta.22_x64_en-US.msi"]) }),
    /no known updater/
  );
});

test("refuses two assets claiming the same platform", () => {
  assert.throws(
    () => manifest({ assets: assets([...SIGNED_ASSETS, "Other_0.1.0-beta.22_x64-setup.exe"]) }),
    /Both .* claim updater platform windows-x86_64/
  );
});

test("refuses assets from a different version, empty signatures, and unsafe names", () => {
  assert.throws(
    () => manifest({ tag: "v0.1.0-beta.23" }),
    /does not carry version 0\.1\.0-beta\.23/
  );
  assert.throws(
    () => manifest({ assets: assets().map((a) => ({ ...a, signature: " \n" })) }),
    /Signature for .* is empty/
  );
  assert.throws(
    () => manifest({ assets: [{ name: "Kalpa 0.1.0-beta.22_x64-setup.exe", signature: "s" }] }),
    /not safe to use in a release URL/
  );
});

test("validates the tag, repository, notes and asset list up front", () => {
  assert.equal(versionFromTag("v1.2.3"), "1.2.3");
  assert.throws(() => versionFromTag("1.2.3"), /Invalid release tag/);
  assert.throws(() => versionFromTag("v$(id)"), /Invalid release tag/);
  assert.throws(() => manifest({ repo: "not-a-repo" }), /Invalid repository/);
  assert.throws(() => manifest({ notes: undefined }), /notes must be a string/);
  assert.throws(() => manifest({ assets: [] }), /No signed updater artifacts/);
});

test("collects every .sig beside its asset and rejects an orphaned signature", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "kalpa-updater-manifest-"));
  try {
    for (const name of SIGNED_ASSETS) {
      fs.writeFileSync(path.join(directory, name), "bytes");
      fs.writeFileSync(path.join(directory, `${name}.sig`), `sig ${name}\n`);
    }
    fs.writeFileSync(path.join(directory, "Kalpa_0.1.0-beta.22_universal.dmg"), "dmg");
    // A directory (tauri-action lists the bare Kalpa.app) must be ignored.
    fs.mkdirSync(path.join(directory, "Kalpa.app"));

    const collected = collectSignedAssets(directory);
    assert.deepEqual(
      collected.map((a) => a.name),
      [...SIGNED_ASSETS].sort()
    );
    assert.equal(collected[0].signature, `sig ${[...SIGNED_ASSETS].sort()[0]}\n`);

    fs.writeFileSync(path.join(directory, "Kalpa_0.1.0-beta.22_arm64.AppImage.sig"), "sig");
    assert.throws(
      () => collectSignedAssets(directory),
      /has no Kalpa_0.1.0-beta.22_arm64.AppImage/
    );
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});
