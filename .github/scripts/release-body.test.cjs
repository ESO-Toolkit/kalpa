const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  buildReleaseBody,
  extractReleaseSection,
  writeGitHubOutput,
} = require("./release-body.cjs");

const changelog = `# Changelog

## [Unreleased]

### Bug Fixes

- Fix the next thing.

## [1.2.3] — 2026-08-26

A focused release.

### Features

- Add the useful thing.

## [1.2.2] — 2026-08-20

- Previous work.

[Unreleased]: https://example.test/compare/v1.2.3...HEAD
[1.2.3]: https://example.test/releases/v1.2.3
`;

test("extracts the exact tagged release section", () => {
  assert.equal(
    extractReleaseSection(changelog, "v1.2.3"),
    "A focused release.\n\n### Features\n\n- Add the useful thing."
  );
});

test("does not include trailing changelog link definitions", () => {
  assert.equal(
    extractReleaseSection(
      "## [1.0.0] — 2026-08-26\n\n- Final entry.\n\n[1.0.0]: https://example.test/v1.0.0",
      "v1.0.0"
    ),
    "- Final entry."
  );
});

test("does not truncate content after a section-local link definition", () => {
  assert.equal(
    extractReleaseSection(
      [
        "## [1.0.0] — 2026-08-26",
        "",
        "- First [linked item][details].",
        "",
        "[details]: https://example.test/details",
        "",
        "- A later item must still ship.",
        "",
        "## [0.9.0] — 2026-08-20",
        "",
        "- Previous work.",
      ].join("\n"),
      "v1.0.0"
    ),
    [
      "- First [linked item][details].",
      "",
      "[details]: https://example.test/details",
      "",
      "- A later item must still ship.",
    ].join("\n")
  );
});

test("retains a trailing link definition used by release copy", () => {
  assert.equal(
    extractReleaseSection(
      "## [1.0.0] — 2026-08-26\n\n- Read the [details][notes].\n\n[notes]: https://example.test/notes",
      "v1.0.0"
    ),
    "- Read the [details][notes].\n\n[notes]: https://example.test/notes"
  );
});

test("selects Unreleased only when explicitly requested", () => {
  assert.equal(
    extractReleaseSection(changelog, "Unreleased"),
    "### Bug Fixes\n\n- Fix the next thing."
  );
  assert.throws(() => extractReleaseSection(changelog, "vUnreleased"), /invalid release/i);
});

test("fails closed when the requested section is missing, empty, malformed, or duplicated", () => {
  assert.throws(() => extractReleaseSection(changelog, "v9.9.9"), /missing/i);
  assert.throws(
    () => extractReleaseSection("## [1.2.3] — 2026-08-26\n\n_Nothing yet._", "v1.2.3"),
    /empty/i
  );
  assert.throws(
    () => extractReleaseSection("## [1.2.3] 2026-08-26\n\n- Work", "v1.2.3"),
    /malformed/i
  );
  assert.throws(
    () =>
      extractReleaseSection(
        "## [1.2.3] — 2026-08-26\n\n- One\n\n## [1.2.3] — 2026-08-27\n\n- Two",
        "v1.2.3"
      ),
    /duplicate/i
  );
});

test("builds the established release body around changelog-authored Changed copy", () => {
  const body = buildReleaseBody(changelog, "v1.2.3");

  assert.match(body, /^See \[CHANGELOG\.md\]/);
  assert.match(body, /## Install\n/);
  assert.match(
    body,
    /## Changed:\nA focused release\.\n\n### Features\n\n- Add the useful thing\./
  );
  assert.match(body, /## Verify your download\n/);
  assert.match(body, /## Known issues\nNone known/);
  assert.doesNotMatch(body, /security and dependency refresh/i);
});

test("writes a multiline GitHub Actions output without altering markdown", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "kalpa-release-body-"));
  const outputPath = path.join(directory, "output.txt");

  try {
    writeGitHubOutput(outputPath, "line one\nline two");
    const output = fs.readFileSync(outputPath, "utf8");
    assert.match(output, /^body<<KALPA_RELEASE_BODY_[a-f0-9]+\n/);
    assert.match(output, /\nline one\nline two\nKALPA_RELEASE_BODY_[a-f0-9]+\n$/);
    const [opening, ...rest] = output.trimEnd().split("\n");
    const delimiter = opening.slice("body<<".length);
    assert.equal(rest.at(-1), delimiter);
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});
