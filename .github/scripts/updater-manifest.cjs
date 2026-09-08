// Assembles the Tauri updater's latest.json from the .sig files the three
// build jobs produced, so the publish job can write the whole manifest in one
// deterministic step instead of letting each platform's tauri-action merge its
// own entry into a shared release asset (a read-merge-delete-reupload cycle
// that raced when the build jobs ran in parallel, which is why release.yml
// used to serialize them).
//
// The output matches, key for key, the manifest tauri-action v1.0.0 wrote for
// v0.1.0-beta.22: the legacy `{os}-{arch}` keys the shipped updater reads,
// plus the `{os}-{arch}-{installer}` keys newer updaters prefer. Every
// installed copy of Kalpa resolves its update from this file, so a missing or
// renamed key here breaks auto-update for that platform; `REQUIRED_PLATFORMS`
// is the fail-closed list.
//
// URLs are the public `releases/download/<tag>/<asset>` form rather than the
// `api.github.com/.../releases/assets/<id>` form tauri-action emitted. Both
// serve the same bytes to the updater; the public form is what the Tauri
// updater docs show and, crucially, it is a pure function of the tag and the
// asset name, so the manifest can be written before anything is uploaded and
// re-generated identically on a re-run.
const fs = require("node:fs");
const path = require("node:path");

// Order is what a maintainer scrolling the release page expects; JSON key
// order carries no meaning to the updater.
const PLATFORM_RULES = [
  {
    // NSIS is the only Windows bundle target (tauri.conf.json bundle.targets),
    // so there is no msi to prefer over it.
    pattern: /_x64-setup\.exe$/,
    keys: ["windows-x86_64", "windows-x86_64-nsis"],
  },
  {
    // One universal .app.tar.gz serves both Mac arches; tauri-action fills
    // both native keys from it (plus the universal ones) when no native build
    // exists, and neither does here.
    pattern: /_universal\.app\.tar\.gz$/,
    keys: [
      "darwin-aarch64",
      "darwin-x86_64",
      "darwin-universal",
      "darwin-aarch64-app",
      "darwin-x86_64-app",
      "darwin-universal-app",
    ],
  },
  {
    // The AppImage is the Linux updater artifact; .deb/.rpm installs do not
    // self-update (see the release body) but keep their installer-specific
    // keys so the manifest stays shape-identical to earlier releases.
    pattern: /_amd64\.AppImage$/,
    keys: ["linux-x86_64", "linux-x86_64-appimage"],
  },
  { pattern: /_amd64\.deb$/, keys: ["linux-x86_64-deb"] },
  { pattern: /\.x86_64\.rpm$/, keys: ["linux-x86_64-rpm"] },
];

const REQUIRED_PLATFORMS = ["windows-x86_64", "darwin-x86_64", "darwin-aarch64", "linux-x86_64"];

// GitHub rewrites spaces and a few other characters in asset names on upload,
// which would silently desynchronise the URL written here from the asset that
// actually exists. Refuse anything that could be rewritten.
const SAFE_ASSET_NAME = /^[A-Za-z0-9._-]+$/;

function platformKeysFor(assetName) {
  const rule = PLATFORM_RULES.find(({ pattern }) => pattern.test(assetName));
  if (!rule) {
    throw new Error(
      `Signed asset ${assetName} matches no known updater artifact; refusing to guess its platform`
    );
  }
  return [...rule.keys];
}

function versionFromTag(tag) {
  if (typeof tag !== "string" || !/^v\d[0-9A-Za-z.-]*$/.test(tag)) {
    throw new Error(`Invalid release tag: ${tag}`);
  }
  return tag.slice(1);
}

// `assets` is the list of signed updater artifacts: { name, signature } where
// `name` is the release asset name (the .sig file name without its suffix)
// and `signature` is the .sig file's content, verbatim.
function buildUpdaterManifest({ tag, repo, assets, notes, now = new Date() }) {
  const version = versionFromTag(tag);
  if (typeof repo !== "string" || !/^[\w.-]+\/[\w.-]+$/.test(repo)) {
    throw new Error(`Invalid repository: ${repo}`);
  }
  if (typeof notes !== "string") throw new Error("notes must be a string");
  if (!Array.isArray(assets) || assets.length === 0) {
    throw new Error("No signed updater artifacts were provided");
  }

  const platforms = {};
  const sources = new Map();
  for (const { name, signature } of assets) {
    if (!SAFE_ASSET_NAME.test(name)) {
      throw new Error(`Asset name ${JSON.stringify(name)} is not safe to use in a release URL`);
    }
    if (!name.includes(version)) {
      throw new Error(`Asset ${name} does not carry version ${version}; stale artifact?`);
    }
    if (typeof signature !== "string" || signature.trim() === "") {
      throw new Error(`Signature for ${name} is empty`);
    }
    const entry = {
      signature,
      url: `https://github.com/${repo}/releases/download/${tag}/${name}`,
    };
    for (const key of platformKeysFor(name)) {
      if (sources.has(key)) {
        throw new Error(`Both ${sources.get(key)} and ${name} claim updater platform ${key}`);
      }
      sources.set(key, name);
      platforms[key] = entry;
    }
  }

  const missing = REQUIRED_PLATFORMS.filter((key) => !platforms[key]);
  if (missing.length > 0) {
    throw new Error(`latest.json would be missing required platform(s): ${missing.join(", ")}`);
  }

  // Emit in PLATFORM_RULES order regardless of the order assets arrived in.
  const ordered = {};
  for (const { keys } of PLATFORM_RULES) {
    for (const key of keys) ordered[key] = platforms[key];
  }
  return { version, notes, pub_date: now.toISOString(), platforms: ordered };
}

// Every `<asset>.sig` in the directory, paired with its content, after
// checking the asset it signs sits beside it — a signature for a file that
// never gets uploaded would produce a manifest whose URL 404s.
function collectSignedAssets(directory) {
  const entries = fs.readdirSync(directory, { withFileTypes: true });
  const files = new Set(entries.filter((entry) => entry.isFile()).map((entry) => entry.name));
  const assets = [];
  for (const sigName of [...files].filter((name) => name.endsWith(".sig")).sort()) {
    const name = sigName.slice(0, -".sig".length);
    if (!files.has(name)) {
      throw new Error(`${sigName} has no ${name} beside it in ${directory}`);
    }
    assets.push({ name, signature: fs.readFileSync(path.join(directory, sigName), "utf8") });
  }
  return assets;
}

function parseArguments(argv) {
  const options = {};
  const valued = new Set(["--tag", "--repo", "--assets-dir", "--notes-file", "--output"]);
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (!valued.has(argument)) throw new Error(`Unknown argument: ${argument}`);
    const value = argv[index + 1];
    if (!value) throw new Error(`${argument} requires a value`);
    options[argument.slice(2).replace(/-([a-z])/g, (_, c) => c.toUpperCase())] = value;
    index += 1;
  }
  for (const required of ["tag", "repo", "assetsDir", "notesFile", "output"]) {
    if (!options[required]) throw new Error(`--${required} is required`);
  }
  return options;
}

function main() {
  const options = parseArguments(process.argv.slice(2));
  const manifest = buildUpdaterManifest({
    tag: options.tag,
    repo: options.repo,
    assets: collectSignedAssets(options.assetsDir),
    notes: fs.readFileSync(options.notesFile, "utf8"),
  });
  fs.writeFileSync(options.output, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  console.log(`Wrote ${options.output} for ${manifest.version}:`);
  for (const [key, { url }] of Object.entries(manifest.platforms)) {
    console.log(`  ${key} -> ${url}`);
  }
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(`Updater manifest generation failed: ${error.message}`);
    process.exitCode = 1;
  }
}

module.exports = {
  REQUIRED_PLATFORMS,
  buildUpdaterManifest,
  collectSignedAssets,
  platformKeysFor,
  versionFromTag,
};
