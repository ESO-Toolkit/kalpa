const KALPA_ICON_URL =
  "https://raw.githubusercontent.com/ESO-Toolkit/kalpa/main/src-tauri/icons/128x128.png";
const DISCORD_GOLD = 12887114;

// Alpha.2 through alpha.7 shipped with generic GitHub release bodies and the
// current changelog consolidates several of their ranges. Preserve their real
// history here, reconstructed from the commits between each published tag, so
// a backfill does not reduce them to "a new release is available."
const CURATED_HISTORICAL_HIGHLIGHTS = {
  "v0.1.0-alpha.1": [
    "**Kalpa's first public alpha**",
    "- Scan, install, search, and update ESO addons from a desktop app.",
    "- Manage profiles, backups, characters, API compatibility, and Minion migration.",
    "- Browse community collections through Pack Hub and share setups with codes or `.esopack` files.",
    "- Edit SavedVariables, work offline gracefully, and receive signed automatic updates.",
  ].join("\n"),
  "v0.1.0-alpha.2": [
    "**Pack Hub sync and desktop polish**",
    "- Pack changes now stay synchronized with the ESO Toolkit website.",
    "- Double-clicking Kalpa's header now maximizes or restores the window.",
    "- Refreshed the updater and core dependencies for a more reliable foundation.",
  ].join("\n"),
  "v0.1.0-alpha.3": [
    "**A smoother, clearer Kalpa**",
    "- Added polished transitions and interaction feedback throughout the app, with accessibility improvements alongside them.",
    "- ESOUI descriptions now display encoded characters correctly, and Discover's MD5 value is compact with click-to-copy.",
    "- Fixed the automatic-update endpoint and made pending batch removals persist when the window closes.",
    "- Updated audited dependencies to resolve security warnings.",
  ].join("\n"),
  "v0.1.0-alpha.4": [
    "**Cleaner ESOUI search results**",
    "- ESOUI's summary row is no longer mistaken for an addon.",
    "- Duplicate search results are filtered out, making browsing and installation less confusing.",
  ].join("\n"),
  "v0.1.0-alpha.5": [
    "**Protected edits arrive**",
    "- Kalpa now detects addon files you changed before an update can overwrite them.",
    "- Review file-level differences and choose whether to keep your copy or accept the update.",
    "- Edited files are backed up, restorable, and visible through the built-in file browser and editor.",
    "- Batch updates gained a conflict flow and a reusable conflict-policy setting.",
  ].join("\n"),
  "v0.1.0-alpha.6": [
    "**Safer backups with a simpler restore flow**",
    "- Backup and Restore was redesigned around clearer protection status and plain-language actions.",
    "- Safety snapshots now protect the current setup before a restore begins.",
    "- A restore stops instead of proceeding when its safety snapshot cannot be created.",
    "- Protected-edit backup and conflict detection received a focused reliability pass.",
  ].join("\n"),
  "v0.1.0-alpha.7": [
    "**Smarter dependencies and portable addon settings**",
    "- Addon updates can install newly required dependencies automatically, including transitive and subfolder libraries.",
    "- Version requirements are checked against installed libraries, with skipped dependencies explained after installation.",
    "- `.esopack` v2 introduced per-addon SavedVariables export and import with security hardening.",
    "- Outdated dependencies now expose the correct update action.",
  ].join("\n"),
  "v0.1.0-alpha.8": [
    "**Reliability and test coverage before beta**",
    "- Added frontend unit tests, real-app end-to-end coverage, and Pack Hub Worker tests.",
    "- Worker tests now run before deployment, and shared helpers ensure tests exercise production behavior.",
    "- Fixed outdated checks for bundled libraries and dependency updates tracked through metadata.",
    "- Pinned the Rust toolchain and dependency-audit tooling for repeatable builds.",
  ].join("\n"),
};

const ALPHA_COMPARE_BASES = {
  "v0.1.0-alpha.2": "v0.1.0-alpha.1",
  "v0.1.0-alpha.3": "v0.1.0-alpha.2",
  "v0.1.0-alpha.4": "v0.1.0-alpha.3",
  "v0.1.0-alpha.5": "v0.1.0-alpha.4",
  "v0.1.0-alpha.6": "v0.1.0-alpha.5",
  "v0.1.0-alpha.7": "v0.1.0-alpha.6",
  "v0.1.0-alpha.8": "v0.1.0-alpha.7",
};

function truncateMarkdown(text, limit = 3000) {
  if (text.length <= limit) return text;

  const suffix = "\n\n*… truncated — read the full changelog below*";
  const available = limit - suffix.length;
  const candidate = text.slice(0, available);
  const lastLineBreak = candidate.lastIndexOf("\n");
  const cleanCut = lastLineBreak > available * 0.7 ? lastLineBreak : available;
  return `${candidate.slice(0, cleanCut).trimEnd()}${suffix}`;
}

function extractChangedSection(body) {
  if (!body) return null;

  const lines = body.split(/\r?\n/);
  const changedIndex = lines.findIndex((line) => /^##\s+Changed:/i.test(line));
  if (changedIndex === -1) return null;

  const nextHeadingOffset = lines.slice(changedIndex + 1).findIndex((line) => /^##\s+/.test(line));
  const endIndex = nextHeadingOffset === -1 ? lines.length : changedIndex + 1 + nextHeadingOffset;
  const heading = lines[changedIndex].replace(/^##\s+Changed:\s*/i, "").trim();
  const details = lines
    .slice(changedIndex + 1, endIndex)
    .join("\n")
    .trim();

  return `${heading ? `**${heading}**` : "**What changed**"}${details ? `\n${details}` : ""}`;
}

function extractChangelogSection(changelog, tagName) {
  if (!changelog) return null;

  const version = tagName.replace(/^v/, "");
  const lines = changelog.split(/\r?\n/);
  const releaseIndex = lines.findIndex((line) => line.startsWith(`## [${version}]`));
  if (releaseIndex === -1) return null;

  const nextReleaseOffset = lines.slice(releaseIndex + 1).findIndex((line) => /^## \[/.test(line));
  const endIndex = nextReleaseOffset === -1 ? lines.length : releaseIndex + 1 + nextReleaseOffset;

  return lines
    .slice(releaseIndex + 1, endIndex)
    .join("\n")
    .replace(/<!--[^]*?-->/g, "")
    .trim();
}

function releaseHighlights(release, changelog) {
  const curated = CURATED_HISTORICAL_HIGHLIGHTS[release.tag_name];
  if (curated) return curated;

  const changed = extractChangedSection(release.body);
  if (changed) return truncateMarkdown(changed);

  const changelogSection = extractChangelogSection(changelog, release.tag_name);
  if (changelogSection) return truncateMarkdown(changelogSection);

  return "A new Kalpa release is ready to download.";
}

function releasePlatforms(assets = []) {
  const names = assets.map((asset) => asset.name.toLowerCase());
  const platforms = [];
  if (names.some((name) => name.endsWith(".exe"))) platforms.push("Windows");
  if (names.some((name) => name.endsWith(".dmg"))) platforms.push("macOS");
  if (
    names.some(
      (name) => name.endsWith(".appimage") || name.endsWith(".deb") || name.endsWith(".rpm")
    )
  ) {
    platforms.push("Linux");
  }

  return platforms.length > 0 ? platforms.join(" · ") : "See release assets";
}

function buildReleasePayload({ release, repository, changelog, historical }) {
  const repositoryUrl = `https://github.com/${repository}`;
  const changelogUrl = `${repositoryUrl}/blob/${release.tag_name}/CHANGELOG.md`;
  const compareBase = ALPHA_COMPARE_BASES[release.tag_name];
  const detailsUrl = compareBase
    ? `${repositoryUrl}/compare/${compareBase}...${release.tag_name}`
    : changelogUrl;
  const detailsLabel = compareBase ? "View release changes" : "Read the changelog";
  const version = release.tag_name.replace(/^v/, "");
  const historicalLabel = historical ? "Historical release · " : "";

  return {
    username: "Kalpa Releases",
    avatar_url: KALPA_ICON_URL,
    allowed_mentions: { parse: [] },
    embeds: [
      {
        title: `Kalpa ${version} is available`,
        url: release.html_url,
        description: `${releaseHighlights(release, changelog)}\n\n⬇️ **[Download this release](${release.html_url})**`,
        color: DISCORD_GOLD,
        thumbnail: { url: KALPA_ICON_URL },
        fields: [
          {
            name: "Platforms",
            value: releasePlatforms(release.assets),
            inline: true,
          },
          {
            name: "Full details",
            value: `[${detailsLabel}](${detailsUrl})`,
            inline: true,
          },
        ],
        footer: {
          text: `${historicalLabel}Kalpa · Signed installers are attached to the GitHub Release`,
        },
        timestamp: release.published_at,
      },
    ],
  };
}

async function sendReleaseToDiscord({
  release,
  repository,
  changelog,
  historical = false,
  webhook,
  core,
  fetchImpl = fetch,
}) {
  const webhookUrl = new URL(webhook);
  webhookUrl.searchParams.set("wait", "true");
  const payload = buildReleasePayload({
    release,
    repository,
    changelog,
    historical,
  });

  for (let attempt = 1; attempt <= 3; attempt += 1) {
    const response = await fetchImpl(webhookUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });

    if (response.ok) return;

    const responseText = await response.text();
    const retryable = response.status === 429 || response.status >= 500;
    if (!retryable || attempt === 3) {
      throw new Error(`Discord webhook failed (${response.status}): ${responseText}`);
    }

    const delayMs = attempt * 2000;
    core.warning(`Discord returned ${response.status}; retrying in ${delayMs}ms.`);
    await new Promise((resolve) => setTimeout(resolve, delayMs));
  }
}

module.exports = {
  CURATED_HISTORICAL_HIGHLIGHTS,
  buildReleasePayload,
  extractChangelogSection,
  extractChangedSection,
  releasePlatforms,
  sendReleaseToDiscord,
};
