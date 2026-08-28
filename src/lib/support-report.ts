import type { AddonManifest, UpdateCheckResult } from "@/types";

export const SUPPORT_ISSUES = [
  {
    id: "addon-status",
    label: "Addon status looks wrong",
    description: "Kalpa still says an addon is outdated, missing, or disabled.",
    placeholder:
      "For example: I updated several addons in Minion, but Kalpa still says they need an update.",
    diagnosticNote:
      "Kalpa included the addon versions, dependency warnings, and modified-file state it currently sees.",
  },
  {
    id: "install-update",
    label: "Install or update failed",
    description: "A download, install, or update did not finish correctly.",
    placeholder: "What were you installing or updating, and what happened after you clicked it?",
    diagnosticNote:
      "Kalpa included the addon versions and local file state it currently sees. Please describe the failed step above.",
  },
  {
    id: "addon-folder",
    label: "Wrong game or addon folder",
    description: "Kalpa may be looking at the wrong live, EU, or PTS folder.",
    placeholder:
      "Which ESO installation should Kalpa be using, and what folder or addons are missing?",
    diagnosticNote:
      "Kalpa included the detected ESO instance. Your full folder path remains hidden unless you opt in below.",
  },
  {
    id: "backups-data",
    label: "Backups, profiles, or saved data",
    description: "Something is unexpected with a backup, profile, or SavedVariables tool.",
    placeholder:
      "Which backup, profile, or saved-data action were you trying, and what went wrong?",
    diagnosticNote:
      "Backup contents and SavedVariables are deliberately not collected. Please describe the affected item and action above.",
  },
  {
    id: "log-upload",
    label: "ESO Logs upload",
    description: "A combat log could not be found, prepared, or uploaded.",
    placeholder: "Which upload step failed, and what message or unexpected result did you see?",
    diagnosticNote:
      "Combat-log contents and account credentials are deliberately not collected. Please describe the failed upload step above.",
  },
  {
    id: "other",
    label: "Something else",
    description: "The problem does not fit one of the choices above.",
    placeholder: "What were you trying to do, what did you expect, and what happened instead?",
    diagnosticNote:
      "Kalpa included only general app and addon state. Please describe what you were doing above.",
  },
] as const;

export type SupportIssueId = (typeof SUPPORT_ISSUES)[number]["id"];

export interface SupportReportInput {
  issueId: SupportIssueId;
  description: string;
  appVersion: string;
  platform: string;
  generatedAt: Date;
  isOffline: boolean;
  checkingUpdates: boolean;
  addonsPath: string;
  instanceLabel: string | null;
  includeAddonsPath: boolean;
  addons: AddonManifest[];
  updateResults: UpdateCheckResult[];
  lastError: string | null;
}

export const SUPPORT_REPORT_MAX_LENGTH = 1950;

function neutralizeMentions(value: string): string {
  return value.replace(/@(everyone|here)/gi, "＠$1").replace(/<(@[!&]?\d+|#\d+)>/g, "<\u200b$1>");
}

function cleanSingleLine(value: string, maxLength: number): string {
  const clean = neutralizeMentions(value)
    .replace(/\s*[\r\n]+\s*/g, " ")
    .trim();
  return clean.length <= maxLength ? clean : `${clean.slice(0, maxLength - 1)}…`;
}

function cleanMultiline(value: string, maxLength: number): string {
  const clean = neutralizeMentions(value).replace(/\r\n?/g, "\n").trim();
  return clean.length <= maxLength ? clean : `${clean.slice(0, maxLength - 1)}…`;
}

function pathPattern(path: string): RegExp | null {
  const parts = path
    .trim()
    .split(/[\\/]+/)
    .filter(Boolean)
    .map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  return parts.length > 0 ? new RegExp(parts.join("[\\\\/]+"), "gi") : null;
}

function redactHomeFolders(value: string, addonsPath: string): string {
  let redacted = value;
  const addonsPathPattern = pathPattern(addonsPath);
  if (addonsPathPattern) redacted = redacted.replace(addonsPathPattern, "[AddOns folder]");

  return redacted
    .replace(/([A-Za-z]:[\\/]+Users[\\/]+)[^\\/\s]+/gi, "$1[user]")
    .replace(/([\\/]+Users[\\/]+)[^\\/\s]+/gi, "$1[user]")
    .replace(/([\\/]+home[\\/]+)[^\\/\s]+/gi, "$1[user]");
}

function issueLabel(issueId: SupportIssueId): string {
  return SUPPORT_ISSUES.find((issue) => issue.id === issueId)?.label ?? "Something else";
}

function issueDiagnosticNote(issueId: SupportIssueId): string {
  return (
    SUPPORT_ISSUES.find((issue) => issue.id === issueId)?.diagnosticNote ??
    "Kalpa included only general app and addon state."
  );
}

function formatAttentionLines(
  addons: AddonManifest[],
  updateResults: UpdateCheckResult[]
): string[] {
  const addonByFolder = new Map(addons.map((addon) => [addon.folderName, addon]));
  const lines = new Map<string, string>();

  for (const result of updateResults.filter((item) => item.hasUpdate)) {
    const addon = addonByFolder.get(result.folderName);
    const name = addon?.title || result.folderName;
    lines.set(
      result.folderName,
      `- ${name} (${result.folderName}): Kalpa sees ${result.currentVersion || "unknown"} → ${result.remoteVersion || "unknown"}`
    );
  }

  for (const addon of addons) {
    const details: string[] = [];
    if (addon.missingDependencies.length > 0) {
      details.push(`missing dependencies: ${addon.missingDependencies.join(", ")}`);
    }
    if (addon.outdatedDependencies.length > 0) {
      details.push(`outdated dependencies: ${addon.outdatedDependencies.join(", ")}`);
    }
    if (addon.modifiedFileCount > 0) {
      details.push(`${addon.modifiedFileCount} locally modified file(s)`);
    }
    if (details.length === 0) continue;

    const existing = lines.get(addon.folderName);
    const prefix = existing ?? `- ${addon.title || addon.folderName} (${addon.folderName})`;
    lines.set(addon.folderName, `${prefix}; ${details.join("; ")}`);
  }

  return [...lines.values()].sort((a, b) => a.localeCompare(b));
}

export function buildSupportReport(input: SupportReportInput): string {
  const libraries = input.addons.filter((addon) => addon.isLibrary).length;
  const disabled = input.addons.filter((addon) => addon.disabled).length;
  const missingDependencies = input.addons.filter(
    (addon) => addon.missingDependencies.length > 0
  ).length;
  const modified = input.addons.filter((addon) => addon.modifiedFileCount > 0).length;
  const updates = input.updateResults.filter((result) => result.hasUpdate).length;
  const attentionLines = formatAttentionLines(input.addons, input.updateResults);
  const description = cleanMultiline(input.description, 500) || "No description provided.";
  const lastError = input.lastError
    ? cleanSingleLine(redactHomeFolders(input.lastError, input.addonsPath), 240)
    : "None recorded";

  const prefix = [
    "# Kalpa support request",
    "",
    `**Issue:** ${issueLabel(input.issueId)}`,
    "",
    "**What happened**",
    description,
    "",
    "## Automatic diagnostics",
    `- Generated: ${input.generatedAt.toISOString()}`,
    `- Kalpa version: ${cleanSingleLine(input.appVersion || "unknown", 40)}`,
    `- Platform: ${cleanSingleLine(input.platform, 40)}`,
    `- Connection: ${input.isOffline ? "offline" : "online"}`,
    `- ESO instance: ${cleanSingleLine(input.instanceLabel ?? "custom or not detected", 80)}`,
    `- AddOns folder: ${
      input.includeAddonsPath
        ? cleanSingleLine(input.addonsPath || "not configured", 300)
        : "hidden by user"
    }`,
    `- Scan summary: ${input.addons.length} addon(s), ${libraries} librar${libraries === 1 ? "y" : "ies"}, ${disabled} disabled`,
    `- Dependency warnings: ${missingDependencies} addon(s)`,
    `- Locally modified: ${modified} addon(s)`,
    `- Update check: ${input.checkingUpdates ? "in progress" : `${input.updateResults.length} checked, ${updates} update(s) reported`}`,
    `- Last app message: ${lastError}`,
    "",
    "## What Kalpa collected for this issue",
    issueDiagnosticNote(input.issueId),
    "",
    "## Addons needing attention",
  ];
  const suffix = [
    "",
    "## Privacy note",
    "This report does not include SavedVariables, account IDs, access tokens, or file contents.",
  ];

  if (attentionLines.length === 0) {
    return [...prefix, "- None detected automatically", ...suffix]
      .map((line) => neutralizeMentions(line))
      .join("\n");
  }

  const included: string[] = [];
  for (const line of attentionLines) {
    const candidate = cleanSingleLine(line, 180);
    const omitted = attentionLines.length - included.length - 1;
    const omissionLine = omitted > 0 ? `- …and ${omitted} more item(s)` : null;
    const next = [
      ...prefix,
      ...included,
      candidate,
      ...(omissionLine ? [omissionLine] : []),
      ...suffix,
    ]
      .map((item) => neutralizeMentions(item))
      .join("\n");
    if (next.length > SUPPORT_REPORT_MAX_LENGTH) break;
    included.push(candidate);
  }

  const omitted = attentionLines.length - included.length;
  const finalLines = [
    ...prefix,
    ...included,
    ...(omitted > 0 ? [`- …and ${omitted} more item(s)`] : []),
    ...suffix,
  ];

  return finalLines.map((line) => neutralizeMentions(line)).join("\n");
}
