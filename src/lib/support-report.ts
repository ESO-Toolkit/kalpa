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
      "Kalpa included the detected ESO instance. Local account names and the full folder path stay hidden.",
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
  addons: AddonManifest[];
  updateResults: UpdateCheckResult[];
  lastError: string | null;
}

export interface SupportAttentionItem {
  name: string;
  folder: string;
  currentVersion: string | null;
  availableVersion: string | null;
  missingDependencies: number;
  outdatedDependencies: number;
  modifiedFiles: number;
}

export interface SupportTicketPayload {
  version: 1;
  issueId: SupportIssueId;
  description: string;
  appVersion: string;
  platform: string;
  generatedAt: string;
  connection: "online" | "offline";
  updateState: "checking" | "complete";
  instanceLabel: string;
  diagnostics: {
    addons: number;
    libraries: number;
    disabled: number;
    checked: number;
    updates: number;
    dependencyWarnings: number;
    modified: number;
    lastError: string | null;
    attention: SupportAttentionItem[];
  };
}

export const SUPPORT_REPORT_MAX_LENGTH = 1950;
// Keep enough headroom below cmd.exe's 8,191-character command-line limit.
// tauri-plugin-opener launches browser URLs through `cmd /c start` on Windows.
export const SUPPORT_HANDOFF_MAX_FRAGMENT_LENGTH = 7000;
export const SUPPORT_HANDOFF_URL = "https://esotk.com/kalpa/support";
const MAX_ATTENTION_ITEMS = 12;

export function neutralizeDiscordMentions(value: string): string {
  return value
    .replace(/@(everyone|here)/gi, "@\u200b$1")
    .replace(/<(?=(@[!&]?\d+|#\d+|t:\d+(?::[tTdDfFR])?|\/[^:>]{1,32}:\d+)>)/g, "<\u200b");
}

function truncate(value: string, maxLength: number): string {
  if (value.length <= maxLength) return value;
  if (maxLength <= 3) return value.slice(0, maxLength);
  return `${value.slice(0, maxLength - 3)}...`;
}

function redactSensitiveText(value: string, addonsPath: string): string {
  let redacted = neutralizeDiscordMentions(value);
  const escaped = addonsPath
    .trim()
    .split(/[\\/]+/)
    .filter(Boolean)
    .map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  if (escaped.length > 0) {
    redacted = redacted.replace(new RegExp(escaped.join("[\\\\/]+"), "gi"), "[AddOns folder]");
  }

  return redacted
    .replace(
      /(?:[A-Za-z]:[\\/]+Users|[\\/]+(?:Users|home))[\\/]+[^\s\\/]+(?:[\\/]+[^\s,;]+)*/gi,
      "[local path]"
    )
    .replace(/\\\\[^\s\\/]+[\\/]+[^\s\\/]+(?:[\\/]+[^\s,;]+)*/g, "[local path]")
    .replace(
      /\b(authorization|bearer|access[_ -]?token|refresh[_ -]?token|api[_ -]?key|client[_ -]?secret)\b(?:\s*[:=]\s*|\s+)[^\s,;]{6,}|\b(token)\b(?:\s*[:=]\s*[^\s,;]+|\s+[A-Za-z0-9._~+/=-]{16,})/gi,
      "$1$2 [redacted]"
    )
    .replace(/\b\d{17,20}\b/g, "[account-id]");
}

function cleanSingleLine(value: string, maxLength: number, addonsPath = ""): string {
  const clean = redactSensitiveText(value, addonsPath)
    .replace(/\s*[\r\n]+\s*/g, " ")
    .trim();
  return truncate(clean, maxLength);
}

function cleanMultiline(value: string, maxLength: number, addonsPath = ""): string {
  const clean = redactSensitiveText(value, addonsPath).replace(/\r\n?/g, "\n").trim();
  return truncate(clean, maxLength);
}

function boundedCount(value: number): number {
  return Math.max(0, Math.min(9999, Math.trunc(Number.isFinite(value) ? value : 0)));
}

function issueDetails(issueId: SupportIssueId) {
  return (
    SUPPORT_ISSUES.find((issue) => issue.id === issueId) ??
    SUPPORT_ISSUES[SUPPORT_ISSUES.length - 1]!
  );
}

export function buildSupportTicketPayload(input: SupportReportInput): SupportTicketPayload {
  const addonByFolder = new Map(input.addons.map((addon) => [addon.folderName, addon]));
  const updateByFolder = new Map(input.updateResults.map((result) => [result.folderName, result]));
  const folders = new Set<string>();

  for (const result of input.updateResults) {
    if (result.hasUpdate) folders.add(result.folderName);
  }
  for (const addon of input.addons) {
    if (
      addon.missingDependencies.length > 0 ||
      addon.outdatedDependencies.length > 0 ||
      addon.modifiedFileCount > 0
    ) {
      folders.add(addon.folderName);
    }
  }

  const attention = [...folders]
    .sort((a, b) => a.localeCompare(b))
    .slice(0, MAX_ATTENTION_ITEMS)
    .map((folder): SupportAttentionItem => {
      const addon = addonByFolder.get(folder);
      const update = updateByFolder.get(folder);
      return {
        name: cleanSingleLine(addon?.title || folder, 80, input.addonsPath),
        folder: cleanSingleLine(folder, 80, input.addonsPath),
        currentVersion: update?.currentVersion
          ? cleanSingleLine(update.currentVersion, 40, input.addonsPath)
          : null,
        availableVersion:
          update?.hasUpdate && update.remoteVersion
            ? cleanSingleLine(update.remoteVersion, 40, input.addonsPath)
            : null,
        missingDependencies: boundedCount(addon?.missingDependencies.length ?? 0),
        outdatedDependencies: boundedCount(addon?.outdatedDependencies.length ?? 0),
        modifiedFiles: boundedCount(addon?.modifiedFileCount ?? 0),
      };
    });

  return {
    version: 1,
    issueId: input.issueId,
    description: cleanMultiline(input.description, 500, input.addonsPath),
    appVersion: cleanSingleLine(input.appVersion || "unknown", 40, input.addonsPath),
    platform: cleanSingleLine(input.platform || "unknown", 40, input.addonsPath),
    generatedAt: input.generatedAt.toISOString(),
    connection: input.isOffline ? "offline" : "online",
    updateState: input.checkingUpdates ? "checking" : "complete",
    instanceLabel: cleanSingleLine(
      input.instanceLabel ?? "custom or not detected",
      80,
      input.addonsPath
    ),
    diagnostics: {
      addons: boundedCount(input.addons.length),
      libraries: boundedCount(input.addons.filter((addon) => addon.isLibrary).length),
      disabled: boundedCount(input.addons.filter((addon) => addon.disabled).length),
      checked: boundedCount(input.updateResults.length),
      updates: boundedCount(input.updateResults.filter((result) => result.hasUpdate).length),
      dependencyWarnings: boundedCount(
        input.addons.filter((addon) => addon.missingDependencies.length > 0).length
      ),
      modified: boundedCount(input.addons.filter((addon) => addon.modifiedFileCount > 0).length),
      lastError: input.lastError ? cleanSingleLine(input.lastError, 240, input.addonsPath) : null,
      attention,
    },
  };
}

function renderAttention(item: SupportAttentionItem): string {
  const details: string[] = [];
  if (item.availableVersion) {
    details.push(`Kalpa sees ${item.currentVersion ?? "unknown"} -> ${item.availableVersion}`);
  }
  if (item.missingDependencies > 0) {
    details.push(`${item.missingDependencies} missing dependency warning(s)`);
  }
  if (item.outdatedDependencies > 0) {
    details.push(`${item.outdatedDependencies} outdated dependency warning(s)`);
  }
  if (item.modifiedFiles > 0) details.push(`${item.modifiedFiles} locally modified file(s)`);
  return `- ${item.name} (${item.folder}): ${details.join("; ") || "needs attention"}`;
}

export function renderSupportReport(payload: SupportTicketPayload): string {
  const issue = issueDetails(payload.issueId);
  const d = payload.diagnostics;
  const heading = [
    "# Kalpa support request",
    "",
    `**Issue:** ${issue.label}`,
    "",
    "**What happened**",
  ];
  const diagnostics = [
    "",
    "## Automatic diagnostics",
    `- Generated: ${payload.generatedAt}`,
    `- Kalpa version: ${payload.appVersion}`,
    `- Platform: ${payload.platform}`,
    `- Connection: ${payload.connection}`,
    `- ESO instance: ${payload.instanceLabel}`,
    "- AddOns folder: hidden (local account names and full paths are never shared)",
    `- Scan summary: ${d.addons} addon(s), ${d.libraries} libraries, ${d.disabled} disabled`,
    `- Dependency warnings: ${d.dependencyWarnings} addon(s)`,
    `- Locally modified: ${d.modified} addon(s)`,
    `- Update check: ${payload.updateState === "checking" ? "in progress" : `${d.checked} checked, ${d.updates} update(s) reported`}`,
    `- Last app message: ${d.lastError ?? "None recorded"}`,
    "",
    "## What Kalpa collected for this issue",
    issue.diagnosticNote,
    "",
    "## Addons needing attention",
  ];
  const suffix = [
    "",
    "## Privacy note",
    "This report does not include SavedVariables, account IDs, access tokens, or file contents.",
  ];
  const items = payload.diagnostics.attention.map(renderAttention);
  const noneOrOmitted =
    items.length > 0
      ? `- ${items.length} item(s) omitted to keep the report within Discord's limit`
      : "- None detected automatically";
  const assemble = (description: string, attention: string[]) =>
    neutralizeDiscordMentions(
      [...heading, description, ...diagnostics, ...attention, ...suffix].join("\n")
    );

  const desiredDescription = payload.description || "No description provided.";
  const fixed = assemble("", [noneOrOmitted]);
  const description = truncate(
    desiredDescription,
    Math.max(0, SUPPORT_REPORT_MAX_LENGTH - fixed.length)
  );
  const included: string[] = [];

  for (const item of items) {
    const candidate = truncate(item, 180);
    const remaining = items.length - included.length - 1;
    const attention = [
      ...included,
      candidate,
      ...(remaining > 0 ? [`- ...and ${remaining} more item(s)`] : []),
    ];
    if (assemble(description, attention).length > SUPPORT_REPORT_MAX_LENGTH) break;
    included.push(candidate);
  }

  const omitted = items.length - included.length;
  const attention =
    included.length > 0
      ? [...included, ...(omitted > 0 ? [`- ...and ${omitted} more item(s)`] : [])]
      : [noneOrOmitted];
  return assemble(description, attention);
}

export function buildSupportReport(input: SupportReportInput): string {
  return renderSupportReport(buildSupportTicketPayload(input));
}

export function buildSupportHandoffUrl(payload: SupportTicketPayload): string | null {
  const bytes = new TextEncoder().encode(JSON.stringify(payload));
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  const encoded = btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
  const fragment = `kalpa=${encoded}`;
  return fragment.length <= SUPPORT_HANDOFF_MAX_FRAGMENT_LENGTH
    ? `${SUPPORT_HANDOFF_URL}#${fragment}`
    : null;
}
