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

/**
 * Allow-listed environment details.
 *
 * Every field below is here because support triage repeatedly needs it and it
 * cannot be used to single a person out:
 *
 * - `osVersion`  OS product/build (for example `10.0.26200`). Windows feature
 *                builds change Controlled Folder Access, SmartScreen and
 *                WebView2 behaviour, and macOS/Linux releases change file
 *                permission prompts. Digits and dots only; anything else
 *                becomes `unknown`, so an edition or machine string cannot leak.
 * - `arch`       CPU architecture from a fixed allow-list. Distinguishes the
 *                x86_64 and aarch64 builds, and on macOS separates native
 *                Apple-silicon runs from Rosetta reports.
 * - `tauri`      Tauri runtime version. Bundled with the release, so it pins
 *                which windowing/opener/updater behaviour is in play.
 * - `webview`    Web view engine and MAJOR version only (for example
 *                `Chromium 138`). WebView2 and WebKit majors drive the CSS and
 *                clipboard differences behind most "the dialog looks wrong"
 *                reports. The major alone keeps the value coarse.
 *
 * Deliberately absent, and never collected anywhere in this flow: hostname or
 * computer name, user or home-directory name, hardware/device IDs, serial
 * numbers, MAC or IP addresses, Discord or account IDs, locale, environment
 * variables, tokens, credentials, cookies, SavedVariables, combat-log content,
 * raw file contents, and full local paths. Collection failures produce
 * `unknown` rather than a guess, and every value is bounded and re-validated
 * independently by the server.
 */
export interface SupportEnvironment {
  osVersion: string;
  arch: string;
  tauri: string;
  webview: string;
}

export const SUPPORT_UNKNOWN = "unknown";

/** Architectures Tauri's os plugin can report. Anything else becomes `unknown`. */
const SUPPORT_ARCHITECTURES = [
  "x86",
  "x86_64",
  "arm",
  "aarch64",
  "loongarch64",
  "mips",
  "mips64",
  "powerpc",
  "powerpc64",
  "riscv64",
  "s390x",
  "sparc",
  "sparc64",
] as const;

/** Numeric-only OS product/build, at most four components (`10.0.26200`). */
export function normalizeOsVersion(value: unknown): string {
  const text = typeof value === "string" ? value.trim() : "";
  return /^\d{1,6}(\.\d{1,6}){0,3}$/.test(text) ? text : SUPPORT_UNKNOWN;
}

export function normalizeArchitecture(value: unknown): string {
  const text = typeof value === "string" ? value.trim().toLowerCase() : "";
  return (SUPPORT_ARCHITECTURES as readonly string[]).includes(text) ? text : SUPPORT_UNKNOWN;
}

/** Bounded semver-shaped runtime version, pre-release tag preserved. */
export function normalizeRuntimeVersion(value: unknown): string {
  const text = typeof value === "string" ? value.trim() : "";
  return /^\d{1,4}(\.\d{1,4}){0,3}(-[0-9A-Za-z.]{1,16})?$/.test(text) ? text : SUPPORT_UNKNOWN;
}

/**
 * Engine plus major version only, derived from the web view's own user-agent.
 * The major is the whole signal support needs, and dropping the build/patch
 * components keeps the value shared by millions of installs.
 */
export function normalizeWebviewLabel(userAgent: unknown): string {
  const text = typeof userAgent === "string" ? userAgent : "";
  const chromium = /(?:Chrome|Chromium)\/(\d{1,4})\./.exec(text);
  if (chromium) return `Chromium ${chromium[1]}`;
  const webkit = /AppleWebKit\/(\d{1,4})\./.exec(text);
  if (webkit) return `WebKit ${webkit[1]}`;
  return SUPPORT_UNKNOWN;
}

export function normalizeSupportEnvironment(value: unknown): SupportEnvironment {
  const input = (value ?? {}) as Partial<Record<keyof SupportEnvironment, unknown>>;
  return {
    osVersion: normalizeOsVersion(input.osVersion),
    arch: normalizeArchitecture(input.arch),
    tauri: normalizeRuntimeVersion(input.tauri),
    webview: /^(?:Chromium|WebKit) \d{1,4}$/.test(String(input.webview ?? ""))
      ? String(input.webview)
      : SUPPORT_UNKNOWN,
  };
}

export const UNKNOWN_SUPPORT_ENVIRONMENT: SupportEnvironment = {
  osVersion: SUPPORT_UNKNOWN,
  arch: SUPPORT_UNKNOWN,
  tauri: SUPPORT_UNKNOWN,
  webview: SUPPORT_UNKNOWN,
};

export interface SupportReportInput {
  issueId: SupportIssueId;
  description: string;
  appVersion: string;
  platform: string;
  environment: SupportEnvironment;
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
  /** 1 kept only so a legacy report still renders; Kalpa now always emits 2. */
  version: 1 | 2;
  issueId: SupportIssueId;
  description: string;
  appVersion: string;
  platform: string;
  /** Present from version 2 onward. A version-1 report omits the key entirely. */
  environment?: SupportEnvironment;
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
// Keep the desktop-to-browser handoff comfortably bounded across OS and browser launchers.
// Report validation is stricter, but this also rejects unexpectedly large navigation payloads.
export const SUPPORT_HANDOFF_MAX_FRAGMENT_LENGTH = 7000;
export const SUPPORT_HANDOFF_URL = "https://esotk.com/kalpa/support";
const MAX_ATTENTION_ITEMS = 12;

export function neutralizeDiscordMentions(value: string): string {
  return value
    .replace(/@(everyone|here)/gi, "@\u200b$1")
    .replace(/<(?=(@[!&]?\d+|#\d+|t:\d+(?::[tTdDfFR])?|\/[^:>]{1,32}:\d+)>)/g, "<\u200b");
}

/**
 * Cut to a UTF-16 length without splitting a surrogate pair.
 *
 * A plain `slice` cuts by code unit, so an astral character (an emoji in an
 * addon name is enough) straddling the boundary leaves a lone high surrogate.
 * That string is not well-formed: `JSON.stringify` emits a bare `\ud83d`,
 * which Discord rejects. The report would then be un-postable while the
 * private channel already existed, and every retry would fail identically.
 */
function sliceCodeUnits(value: string, units: number): string {
  if (units <= 0) return "";
  const lastUnit = value.charCodeAt(units - 1);
  const splitsPair = lastUnit >= 0xd800 && lastUnit <= 0xdbff;
  return value.slice(0, splitsPair ? units - 1 : units);
}

function truncate(value: string, maxLength: number): string {
  if (value.length <= maxLength) return value;
  if (maxLength <= 3) return sliceCodeUnits(value, maxLength);
  return `${sliceCodeUnits(value, maxLength - 3)}...`;
}

function stripNonPrintingControlCharacters(value: string): string {
  return Array.from(value, (character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    const isAllowedWhitespace = codePoint === 9 || codePoint === 10 || codePoint === 13;
    const isControlCharacter = codePoint < 32 || (codePoint >= 127 && codePoint <= 159);
    // Array.from iterates code points, so a well-formed pair arrives as one
    // character whose code point is astral. A code point still inside the
    // surrogate range is therefore unpaired, and would make the string
    // un-serializable as JSON.
    const isLoneSurrogate = codePoint >= 0xd800 && codePoint <= 0xdfff;
    return (isControlCharacter && !isAllowedWhitespace) || isLoneSurrogate ? "" : character;
  }).join("");
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

  return stripNonPrintingControlCharacters(
    redacted
      .replace(
        /(?:[A-Za-z]:[\\/]+|[\\/]+(?:Users|home|media|mnt|opt|run|srv|tmp|var|etc|Volumes)[\\/]+|\bUsers[\\/]+)[^\r\n,;]+?(?=\s+(?:and|at|from|with|then)\b|[,;\r\n]|$)/gi,
        "[local path]"
      )
      .replace(/\\\\[^\r\n,;]+?(?=\s+(?:and|at|from|with|then)\b|[,;\r\n]|$)/g, "[local path]")
      .replace(
        /\b(authorization|bearer|access[_ -]?token|refresh[_ -]?token|api[_ -]?key|client[_ -]?secret)\b(?:\s*[:=]\s*|\s+)[^\s,;]{6,}|\b(token)\b(?:\s*[:=]\s*[^\s,;]+|\s+[A-Za-z0-9._~+/=-]{16,})/gi,
        "$1$2 [redacted]"
      )
      .replace(/\b\d{17,20}\b/g, "[account-id]")
  );
}

function collapseLines(value: string): string {
  return value.replace(/\s*[\r\n]+\s*/g, " ").trim();
}

function normalizeLines(value: string): string {
  return value.replace(/\r\n?/g, "\n").trim();
}

/**
 * Produce text that is already a fixed point of the shared redaction rules and
 * within the length bound, so re-running the identical validation on the hosted
 * page and in the Worker changes nothing.
 *
 * Redacting and then truncating is not enough, because the cut can create a NEW
 * match. `bearer [redacted]` cut short leaves `bearer [redac`, which the server
 * expands straight back to `bearer [redacted]` - longer than what the user
 * reviewed, and long enough to push a maximal report past the Discord cap and
 * have it rejected after consent. Cutting a 23-digit run down to 20 digits
 * likewise turns it into an account ID the server then replaces.
 *
 * So the limit is walked down until the result survives its own rules
 * unchanged. The check deliberately runs without the AddOns-folder rule,
 * because that rule is Kalpa's alone: the invariant that has to hold is
 * equality with what the *server* would compute. The loop always terminates -
 * the empty string is a fixed point - and in practice the full value already is
 * one, so it exits on the first pass.
 */
function clampRedacted(
  value: string,
  maxLength: number,
  addonsPath: string,
  normalize: (text: string) => string
): string {
  const full = normalize(redactSensitiveText(value, addonsPath));
  for (let limit = maxLength; limit > 0; limit -= 1) {
    const candidate = truncate(full, limit);
    if (normalize(redactSensitiveText(candidate, "")) === candidate) return candidate;
  }
  return "";
}

/**
 * True when `value` would survive the hosted page's and the Worker's identical
 * validation unchanged — that is, when the report the user reviewed is exactly
 * the report that gets posted to Discord. The AddOns-folder rule is excluded on
 * purpose: it is Kalpa's alone and the servers never run it.
 */
export function isCanonicalSupportText(
  value: string,
  maxLength: number,
  multiline = false
): boolean {
  const normalize = multiline ? normalizeLines : collapseLines;
  return value.length <= maxLength && normalize(redactSensitiveText(value, "")) === value;
}

function cleanSingleLine(value: string, maxLength: number, addonsPath = ""): string {
  return clampRedacted(value, maxLength, addonsPath, collapseLines);
}

function cleanMultiline(value: string, maxLength: number, addonsPath = ""): string {
  return clampRedacted(value, maxLength, addonsPath, normalizeLines);
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

  const attentionPriority = (folder: string): number => {
    const addon = addonByFolder.get(folder);
    if ((addon?.modifiedFileCount ?? 0) > 0) return 0;
    if ((addon?.missingDependencies.length ?? 0) > 0) return 1;
    if ((addon?.outdatedDependencies.length ?? 0) > 0) return 2;
    return 3;
  };

  const attention = [...folders]
    .sort((a, b) => attentionPriority(a) - attentionPriority(b) || a.localeCompare(b))
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

  return fitSupportTicketPayload({
    version: 2,
    issueId: input.issueId,
    description: cleanMultiline(input.description, 500, input.addonsPath),
    appVersion: cleanSingleLine(input.appVersion || "unknown", 40, input.addonsPath),
    platform: cleanSingleLine(input.platform || "unknown", 40, input.addonsPath),
    environment: normalizeSupportEnvironment(input.environment),
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
  });
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

function renderCompleteSupportReport(payload: SupportTicketPayload): string {
  const issue = issueDetails(payload.issueId);
  const d = payload.diagnostics;
  const heading = [
    "# Kalpa support request",
    "",
    `**Issue:** ${issue.label}`,
    "",
    "**What happened**",
  ];
  const environment = payload.environment
    ? [
        `- OS build: ${payload.environment.osVersion}`,
        `- CPU architecture: ${payload.environment.arch}`,
        `- App runtime: Tauri ${payload.environment.tauri}, web view ${payload.environment.webview}`,
      ]
    : [];
  const diagnostics = [
    "",
    "## Automatic diagnostics",
    `- Generated: ${payload.generatedAt}`,
    `- Kalpa version: ${payload.appVersion}`,
    `- Platform: ${payload.platform}`,
    ...environment,
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
  const attention = payload.diagnostics.attention.map(renderAttention);
  return neutralizeDiscordMentions(
    [
      ...heading,
      payload.description || "No description provided.",
      ...diagnostics,
      ...(attention.length > 0 ? attention : ["- None detected automatically"]),
      ...suffix,
    ].join("\n")
  );
}

function encodeSupportFragment(payload: SupportTicketPayload): string {
  const bytes = new TextEncoder().encode(JSON.stringify(payload));
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return `kalpa=${btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "")}`;
}

/** Shortest useful remainder of a free-text field before shrinking gives up. */
const MIN_FREE_TEXT_LENGTH = 40;

function reportOverflow(payload: SupportTicketPayload): number {
  return renderCompleteSupportReport(payload).length - SUPPORT_REPORT_MAX_LENGTH;
}

function exceedsTransport(payload: SupportTicketPayload): boolean {
  return (
    reportOverflow(payload) > 0 ||
    encodeSupportFragment(payload).length > SUPPORT_HANDOFF_MAX_FRAGMENT_LENGTH
  );
}

/**
 * Trim the longer of the two free-text fields by the amount the report is over,
 * with a visible ellipsis. Returns null once neither field can usefully shrink.
 */
function shrinkFreeText(
  payload: SupportTicketPayload,
  overflow: number
): SupportTicketPayload | null {
  const lastError = payload.diagnostics.lastError ?? "";
  const shrinkDescription = payload.description.length >= lastError.length;
  const current = shrinkDescription ? payload.description : lastError;
  if (current.length <= MIN_FREE_TEXT_LENGTH) return null;

  // Re-clamp rather than plain truncate: the cut must not resurrect a redaction
  // match that the server would then expand differently.
  const next = clampRedacted(
    current,
    Math.max(MIN_FREE_TEXT_LENGTH, current.length - Math.max(overflow, 1)),
    "",
    shrinkDescription ? normalizeLines : collapseLines
  );
  if (next === current) return null;
  return shrinkDescription
    ? { ...payload, description: next }
    : { ...payload, diagnostics: { ...payload.diagnostics, lastError: next } };
}

/**
 * Produce the one canonical payload used by both the review and browser handoff.
 * Lower-priority attention rows are removed first, then the free-text fields are
 * shortened, until the complete rendered report and its UTF-8/base64url handoff
 * both fit their transport limits. No field can therefore be transmitted without
 * also appearing in the user's review — and equally, a report the user reviewed
 * can never be one the hosted page and Worker would reject as too long.
 */
export function fitSupportTicketPayload(payload: SupportTicketPayload): SupportTicketPayload {
  let fitted = payload;

  while (fitted.diagnostics.attention.length > 0 && exceedsTransport(fitted)) {
    fitted = {
      ...fitted,
      diagnostics: {
        ...fitted.diagnostics,
        attention: fitted.diagnostics.attention.slice(0, -1),
      },
    };
  }

  while (exceedsTransport(fitted)) {
    const shrunk = shrinkFreeText(fitted, Math.max(reportOverflow(fitted), 1));
    if (!shrunk) break;
    fitted = shrunk;
  }

  return fitted;
}

export function renderSupportReport(payload: SupportTicketPayload): string {
  return renderCompleteSupportReport(fitSupportTicketPayload(payload));
}

export function buildSupportReport(input: SupportReportInput): string {
  return renderSupportReport(buildSupportTicketPayload(input));
}

/** Platform values the hosted page and Worker accept. */
const HANDOFF_PLATFORMS: readonly string[] = ["windows", "macos", "linux"];

/**
 * Everything the server validates that Kalpa can check locally. Today
 * `osType()` cannot return anything else, but a future default of "unknown"
 * would otherwise only surface as a rejection after the user had consented and
 * switched to their browser.
 */
function isServerAcceptable(payload: SupportTicketPayload): boolean {
  return (
    payload.version === 2 &&
    payload.environment !== undefined &&
    HANDOFF_PLATFORMS.includes(payload.platform) &&
    SUPPORT_ISSUES.some((issue) => issue.id === payload.issueId)
  );
}

export function buildSupportHandoffUrl(payload: SupportTicketPayload): string | null {
  const fitted = fitSupportTicketPayload(payload);
  // Refuse rather than hand the browser a report the hosted page would reject:
  // a dead end after consent is worse than the copy/manual fallback.
  if (exceedsTransport(fitted) || !isServerAcceptable(fitted)) return null;
  return `${SUPPORT_HANDOFF_URL}#${encodeSupportFragment(fitted)}`;
}
