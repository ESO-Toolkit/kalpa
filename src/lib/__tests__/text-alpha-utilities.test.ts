import { readdirSync, readFileSync } from "node:fs";
import { extname, join, relative } from "node:path";
import { describe, expect, it } from "vitest";

type AllowlistReason = "icon" | "aria-hidden" | "glyph" | "icon-button";

type Finding = {
  file: string;
  line: number;
  utility: string;
};

const SOURCE_ROOT = join(process.cwd(), "src");
const SOURCE_EXTENSIONS = new Set([".ts", ".tsx", ".css"]);
const TEXT_ALPHA_UTILITY_PATTERN =
  /(^|[^A-Za-z0-9_:/\-[\]])((?:(?:[A-Za-z0-9_/-]+|data-\[[^\]\s]+\]):)*text-(?:muted-foreground|foreground|primary|accent|white)\/\d+)(?=$|[^A-Za-z0-9_:/\-[\]])/g;

const TRANSIENT_VARIANT_PATTERNS = [
  /^hover$/,
  /^focus$/,
  /^active$/,
  /^disabled$/,
  /^group-hover(?:\/.+)?$/,
  /^group-focus(?:\/.+)?$/,
  /^peer-.+$/,
  /^data-\[.+\]$/,
];

const ALLOWLIST: Record<string, AllowlistReason> = {
  "src/components/addon-detail.tsx|text-muted-foreground/30": "icon",
  "src/components/addon-file-browser.tsx|text-muted-foreground/30": "glyph",
  "src/components/addon-file-browser.tsx|text-muted-foreground/40": "icon",
  "src/components/addon-file-browser.tsx|text-muted-foreground/50": "icon",
  "src/components/addon-list.tsx|text-muted-foreground/30": "icon",
  "src/components/app-header.tsx|text-muted-foreground/60": "icon-button",
  "src/components/backups.tsx|text-muted-foreground/60": "icon",
  "src/components/changelog-dialog.tsx|text-muted-foreground/60": "icon",
  "src/components/client-health.tsx|text-muted-foreground/70": "aria-hidden",
  "src/components/diff-viewer.tsx|text-primary/40": "glyph",
  "src/components/discover-detail.tsx|text-muted-foreground/20": "glyph",
  "src/components/discover-detail.tsx|text-muted-foreground/30": "icon",
  "src/components/discover-panel.tsx|text-muted-foreground/20": "icon",
  "src/components/esoui-overview.tsx|text-muted-foreground/30": "icon",
  "src/components/esoui-tab.tsx|text-muted-foreground/30": "icon",
  "src/components/pack-browse.tsx|text-muted-foreground/20": "glyph",
  "src/components/pack-browse.tsx|text-muted-foreground/40": "icon",
  "src/components/pack-browse.tsx|text-primary/60": "icon",
  "src/components/pack-create.tsx|text-muted-foreground/20": "glyph",
  "src/components/pack-create.tsx|text-muted-foreground/40": "icon",
  "src/components/pack-create.tsx|text-primary/50": "icon",
  "src/components/pack-detail.tsx|text-muted-foreground/30": "icon-button",
  "src/components/pack-detail.tsx|text-primary/70": "icon",
  "src/components/pack-import.tsx|text-muted-foreground/30": "icon",
  "src/components/pack-import.tsx|text-muted-foreground/50": "icon",
  "src/components/pack-my-packs.tsx|text-muted-foreground/20": "glyph",
  "src/components/pack-my-packs.tsx|text-muted-foreground/40": "icon-button",
  "src/components/pack-my-packs.tsx|text-primary/60": "icon",
  "src/components/saved-variables.tsx|text-muted-foreground/30": "icon",
  "src/components/saved-variables.tsx|text-muted-foreground/40": "icon",
  "src/components/saved-variables.tsx|text-muted-foreground/50": "icon",
  "src/components/saved-variables.tsx|text-primary/60": "icon",
  "src/components/settings.tsx|text-muted-foreground/40": "icon",
  "src/components/sv-controls.tsx|text-muted-foreground/60": "icon",
  "src/components/ui/combobox.tsx|text-muted-foreground/60": "icon",
  "src/components/ui/dialog.tsx|text-muted-foreground/60": "icon-button",
  "src/components/ui/select.tsx|text-muted-foreground/60": "icon",
  "src/components/uploader/fight-list.tsx|text-primary/70": "aria-hidden",
  "src/components/uploader/history-panel.tsx|text-muted-foreground/30": "aria-hidden",
  "src/components/uploader/history-panel.tsx|text-muted-foreground/40": "glyph",
  "src/components/uploader/history-panel.tsx|text-muted-foreground/70": "icon-button",
  "src/components/uploader/log-picker.tsx|text-muted-foreground/40": "aria-hidden",
  "src/components/uploader/log-picker.tsx|text-muted-foreground/60": "icon",
  "src/components/uploader/log-picker.tsx|text-muted-foreground/70": "icon-button",
  "src/components/uploader/upload-progress.tsx|text-primary/40": "aria-hidden",
  "src/components/uploader/uploader-shared.tsx|text-primary/80": "aria-hidden",
  "src/components/uploader/uploader-workspace.tsx|text-foreground/50": "aria-hidden",
  "src/components/uploader/uploader-workspace.tsx|text-muted-foreground/30": "aria-hidden",
  "src/components/uploader/uploader-workspace.tsx|text-muted-foreground/40": "aria-hidden",
  "src/components/uploader/uploader-workspace.tsx|text-muted-foreground/50": "aria-hidden",
  "src/components/uploader/uploader-workspace.tsx|text-muted-foreground/60": "icon",
};

// opacity-<n> is intentionally not scanned here. Kalpa uses opacity utilities heavily
// for non-text transitions, disabled affordances, skeletons, and reveal-on-hover layout;
// a lexical source scan would create excessive false positives without DOM semantics.

describe("text alpha utility ratchet", () => {
  it("keeps alpha text colors limited to explicitly allowlisted decorative uses", () => {
    const findings = findTextAlphaUtilities();
    const seenAllowlistKeys = new Set<string>();
    const unallowlisted = findings.filter((finding) => {
      const key = allowlistKey(finding);
      if (key in ALLOWLIST) {
        seenAllowlistKeys.add(key);
        return false;
      }
      return true;
    });
    const staleAllowlistKeys = Object.keys(ALLOWLIST).filter((key) => !seenAllowlistKeys.has(key));

    expect(formatFailures(unallowlisted, staleAllowlistKeys)).toBe("");
  });
});

function findTextAlphaUtilities(): Finding[] {
  return walkSourceFiles(SOURCE_ROOT).flatMap((filePath) => {
    const file = relative(process.cwd(), filePath).replace(/\\/g, "/");
    const lines = readFileSync(filePath, "utf8").split(/\r?\n/);

    return lines.flatMap((line, index) => findTextAlphaUtilitiesInLine(line, file, index + 1));
  });
}

function walkSourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const fullPath = join(dir, entry.name);

    if (entry.isDirectory()) {
      return entry.name === "__tests__" ? [] : walkSourceFiles(fullPath);
    }

    return SOURCE_EXTENSIONS.has(extname(entry.name)) ? [fullPath] : [];
  });
}

function findTextAlphaUtilitiesInLine(line: string, file: string, lineNumber: number): Finding[] {
  return Array.from(line.matchAll(TEXT_ALPHA_UTILITY_PATTERN)).flatMap((match) => {
    const utility = match[2];
    if (!utility || hasTransientVariant(utility)) return [];
    return [{ file, line: lineNumber, utility }];
  });
}

function hasTransientVariant(utility: string): boolean {
  const variants = utility.split(":").slice(0, -1);
  return variants.some((variant) =>
    TRANSIENT_VARIANT_PATTERNS.some((pattern) => pattern.test(variant))
  );
}

function allowlistKey({ file, utility }: Pick<Finding, "file" | "utility">): string {
  return `${file}|${utility}`;
}

function formatFailures(unallowlisted: Finding[], staleAllowlistKeys: string[]): string {
  const unallowlistedMessages = unallowlisted.map(
    ({ file, line, utility }) =>
      `${file}:${line} fades text with \`${utility}\`. The theme's colour already meets 4.5:1; multiplying it by alpha drops it below. Use a different token (muted-foreground / foreground) to express hierarchy, or - if this is decorative (icon, glyph, aria-hidden) - add it to ALLOWLIST in this file with a reason.`
  );
  const staleMessages = staleAllowlistKeys.map(
    (key) => `${key} is allowlisted but no longer exists. Remove the stale ALLOWLIST entry.`
  );

  return [...unallowlistedMessages, ...staleMessages].join("\n");
}
