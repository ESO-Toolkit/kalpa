const crypto = require("node:crypto");
const fs = require("node:fs");

function normalizeReleaseName(releaseRef) {
  if (releaseRef === "Unreleased") return releaseRef;
  if (typeof releaseRef !== "string" || !/^v?\d[0-9A-Za-z.-]*$/.test(releaseRef)) {
    throw new Error(`Invalid release reference: ${releaseRef}`);
  }
  return releaseRef.replace(/^v/, "");
}

function extractReleaseSection(changelog, releaseRef) {
  const releaseName = normalizeReleaseName(releaseRef);
  const lines = changelog.split(/\r?\n/);
  const exactHeading =
    releaseName === "Unreleased"
      ? "## [Unreleased]"
      : new RegExp(`^## \\[${escapeRegExp(releaseName)}\\] — \\d{4}-\\d{2}-\\d{2}$`);
  const targetPrefix = `## [${releaseName}]`;
  const matchingIndexes = [];
  const malformedHeadings = [];

  for (const [index, line] of lines.entries()) {
    if (!line.startsWith(targetPrefix)) continue;
    if (typeof exactHeading === "string" ? line === exactHeading : exactHeading.test(line)) {
      matchingIndexes.push(index);
    } else {
      malformedHeadings.push(line);
    }
  }

  if (malformedHeadings.length > 0) {
    throw new Error(`Malformed ${releaseName} changelog heading: ${malformedHeadings[0]}`);
  }
  if (matchingIndexes.length === 0) {
    throw new Error(`Missing changelog section for ${releaseName}`);
  }
  if (matchingIndexes.length > 1) {
    throw new Error(`Duplicate changelog sections for ${releaseName}`);
  }

  const startIndex = matchingIndexes[0];
  const nextBoundaryOffset = lines
    .slice(startIndex + 1)
    .findIndex((line) => /^## \[/.test(line) || /^\[[^\]]+\]:\s+\S/.test(line));
  const endIndex = nextBoundaryOffset === -1 ? lines.length : startIndex + 1 + nextBoundaryOffset;
  const section = lines
    .slice(startIndex + 1, endIndex)
    .join("\n")
    .replace(/<!--[^]*?-->/g, "")
    .trim();

  if (!section || /^_Nothing yet\._$/i.test(section)) {
    throw new Error(`Empty changelog section for ${releaseName}`);
  }
  return section;
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function buildReleaseBody(changelog, releaseRef) {
  const changed = extractReleaseSection(changelog, releaseRef);
  const tag = releaseRef === "Unreleased" ? "main" : `v${normalizeReleaseName(releaseRef)}`;

  return `See [CHANGELOG.md](https://github.com/ESO-Toolkit/kalpa/blob/${tag}/CHANGELOG.md) for full details.

## Install

**Windows** (stable): download the \`.exe\` installer and run it. Requires Windows 10 (1803+) or Windows 11; WebView2 is pre-installed on Windows 11 and bootstrapped automatically on Windows 10.

**macOS** (beta): download the \`.dmg\` (universal — Intel & Apple Silicon, macOS 10.15+). These builds are not yet notarized with Apple, so on first launch **right-click the app → Open → Open**. If macOS still reports the app as damaged, clear the quarantine flag: \`xattr -dr com.apple.quarantine /Applications/Kalpa.app\`

**Linux** (beta): the \`.AppImage\` is recommended (\`chmod +x\` and run — it self-updates like the Windows build). \`.deb\` and \`.rpm\` packages are also provided but do **not** self-update; install new versions from this page. ESO under Steam Proton is detected automatically, including Flatpak Steam and secondary Steam libraries.

Kalpa auto-updates when future releases are published (all platforms except \`.deb\`/\`.rpm\` installs).

## Changed:
${changed}

## Verify your download
Each release ships an installer per platform, a \`.sig\` (auto-updater signature) for every auto-updatable artifact, one shared \`latest.json\`, and a \`SHA256SUMS.txt\` listing the SHA-256 of every other file here. See [Verify your download](https://github.com/ESO-Toolkit/kalpa/blob/${tag}/docs/verify-download.md) for how to check the integrity of the file you downloaded.

## Known issues
None known — please report anything you hit.

---

🐛 **Found a bug or have feedback?** Use the [Beta Feedback template](https://github.com/ESO-Toolkit/kalpa/issues/new?template=beta_feedback.md) so it's tagged \`beta\` for triage. Security issues: see [SECURITY.md](https://github.com/ESO-Toolkit/kalpa/blob/${tag}/SECURITY.md).`;
}

function writeGitHubOutput(outputPath, body) {
  const delimiter = `KALPA_RELEASE_BODY_${crypto.randomBytes(12).toString("hex")}`;
  fs.appendFileSync(outputPath, `body<<${delimiter}\n${body}\n${delimiter}\n`, "utf8");
}

function parseArguments(argv) {
  const options = { changelog: "CHANGELOG.md" };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--version" || argument === "--changelog") {
      const value = argv[index + 1];
      if (!value) throw new Error(`${argument} requires a value`);
      options[argument.slice(2)] = value;
      index += 1;
    } else if (argument === "--github-output") {
      options.githubOutput = true;
    } else {
      throw new Error(`Unknown argument: ${argument}`);
    }
  }
  if (!options.version) throw new Error("--version is required");
  return options;
}

function main() {
  const options = parseArguments(process.argv.slice(2));
  const changelog = fs.readFileSync(options.changelog, "utf8");
  const body = buildReleaseBody(changelog, options.version);
  if (options.githubOutput) {
    if (!process.env.GITHUB_OUTPUT) throw new Error("GITHUB_OUTPUT is not set");
    writeGitHubOutput(process.env.GITHUB_OUTPUT, body);
  } else {
    process.stdout.write(`${body}\n`);
  }
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(`Release body generation failed: ${error.message}`);
    process.exitCode = 1;
  }
}

module.exports = {
  buildReleaseBody,
  extractReleaseSection,
  writeGitHubOutput,
};
