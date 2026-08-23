const assert = require("node:assert/strict");
const fs = require("node:fs");
const test = require("node:test");
const {
  CURATED_HISTORICAL_HIGHLIGHTS,
  buildReleasePayload,
  extractChangelogSection,
  extractChangedSection,
  releasePlatforms,
  sendReleaseToDiscord,
} = require("./discord-release.cjs");

const changelog = fs.readFileSync("CHANGELOG.md", "utf8");

function release(tag, body = "", assets = [{ name: "Kalpa.exe" }]) {
  return {
    tag_name: tag,
    body,
    html_url: `https://github.com/ESO-Toolkit/kalpa/releases/tag/${tag}`,
    published_at: "2026-05-03T00:00:00Z",
    assets,
  };
}

test("every alpha release has a substantial curated summary", () => {
  const alphaTags = Array.from({ length: 8 }, (_, index) => `v0.1.0-alpha.${index + 1}`);

  for (const tag of alphaTags) {
    assert(CURATED_HISTORICAL_HIGHLIGHTS[tag], `${tag} is not curated`);
    assert(CURATED_HISTORICAL_HIGHLIGHTS[tag].length >= 150, `${tag} summary is too thin`);
  }
});

test("extracts curated release and changelog sections", () => {
  assert.match(
    extractChangedSection(
      "## Install\nignore\n\n## Changed: Better updates\nPlayers see more.\n\n## Verify\nignore"
    ),
    /\*\*Better updates\*\*\nPlayers see more\./
  );
  assert.match(extractChangelogSection(changelog, "v0.1.0-beta.17"), /required libraries/i);
});

test("infers platforms from actual release assets", () => {
  assert.equal(
    releasePlatforms([{ name: "Kalpa.exe" }, { name: "Kalpa.dmg" }, { name: "Kalpa.AppImage" }]),
    "Windows · macOS · Linux"
  );
  assert.equal(releasePlatforms([{ name: "Kalpa.exe" }]), "Windows");
});

test("builds a safe historical alpha announcement", () => {
  const payload = buildReleasePayload({
    release: release("v0.1.0-alpha.5"),
    repository: "ESO-Toolkit/kalpa",
    changelog,
    historical: true,
  });
  const embed = payload.embeds[0];

  assert.match(embed.description, /Protected edits arrive/);
  assert.match(embed.fields[1].value, /compare\/v0\.1\.0-alpha\.4\.\.\.v0\.1\.0-alpha\.5/);
  assert.match(embed.footer.text, /Historical release/);
  assert.deepEqual(payload.allowed_mentions, { parse: [] });
  assert(embed.description.length < 4096);
});

test("prefers a future release's maintained Changed section", () => {
  const payload = buildReleasePayload({
    release: release(
      "v0.1.0-beta.17",
      "## Changed: Better dependency choices\nPlayers can choose required libraries."
    ),
    repository: "ESO-Toolkit/kalpa",
    changelog,
    historical: false,
  });

  assert.match(payload.embeds[0].description, /Better dependency choices/);
  assert.match(payload.embeds[0].fields[1].value, /CHANGELOG\.md/);
});

test("sends a mention-safe webhook payload with confirmation enabled", async () => {
  let attempts = 0;
  await sendReleaseToDiscord({
    release: release("v0.1.0-alpha.1"),
    repository: "ESO-Toolkit/kalpa",
    changelog,
    historical: true,
    webhook: "https://discord.com/api/webhooks/example/token",
    core: { warning() {} },
    fetchImpl: async (url, options) => {
      attempts += 1;
      assert.match(String(url), /wait=true/);
      assert.equal(JSON.parse(options.body).username, "Kalpa Releases");
      return { ok: true, status: 200, text: async () => "" };
    },
  });

  assert.equal(attempts, 1);
});

test("embedded GitHub workflow scripts are valid JavaScript", () => {
  const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
  const workflows = [
    ".github/workflows/release.yml",
    ".github/workflows/discord-release-backfill.yml",
    ".github/workflows/discord-pr-notify.yml",
  ];

  for (const workflow of workflows) {
    const lines = fs.readFileSync(workflow, "utf8").split(/\r?\n/);
    const scriptIndex = lines.findIndex((line) => line.trim() === "script: |");
    assert(scriptIndex >= 0, `${workflow} has no github-script block`);
    const script = lines
      .slice(scriptIndex + 1)
      .map((line) => (line.startsWith("            ") ? line.slice(12) : line))
      .join("\n");

    assert.doesNotThrow(
      () => new AsyncFunction("context", "github", "core", "fetch", "process", "require", script),
      `${workflow} contains invalid JavaScript`
    );
  }
});
