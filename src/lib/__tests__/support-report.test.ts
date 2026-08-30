import { describe, expect, it } from "vitest";
import type { AddonManifest, UpdateCheckResult } from "../../types";
import supportContractFixture from "./support-contract-fixture.json";
import {
  buildSupportHandoffUrl,
  buildSupportReport,
  buildSupportTicketPayload,
  fitSupportTicketPayload,
  isCanonicalSupportText,
  normalizeArchitecture,
  normalizeOsVersion,
  normalizeRuntimeVersion,
  normalizeWebviewLabel,
  renderSupportReport,
  sealSupportTicketPayload,
  sha256Hex,
  SUPPORT_HANDOFF_MAX_FRAGMENT_LENGTH,
  SUPPORT_REPORT_MAX_LENGTH,
  UNKNOWN_SUPPORT_ENVIRONMENT,
  UNSEALED_REPORT_SHA256,
  type SupportTicketPayload,
  type SupportReportInput,
} from "../support-report";

function addon(overrides: Partial<AddonManifest> = {}): AddonManifest {
  return {
    folderName: "MapPins",
    title: "Map Pins",
    author: "Author",
    version: "1.0",
    addonVersion: 1,
    apiVersion: [101047],
    description: "",
    isLibrary: false,
    dependsOn: [],
    optionalDependsOn: [],
    missingDependencies: [],
    outdatedDependencies: [],
    missingOptionalDependencies: [],
    esouiId: 101,
    tags: [],
    esouiLastUpdate: 0,
    installedAt: "",
    disabled: false,
    modifiedFileCount: 0,
    ...overrides,
  };
}

function update(overrides: Partial<UpdateCheckResult> = {}): UpdateCheckResult {
  return {
    folderName: "MapPins",
    esouiId: 101,
    currentVersion: "1.0",
    remoteVersion: "2.0",
    downloadUrl: "https://example.invalid/addon.zip",
    hasUpdate: true,
    remoteLastUpdate: 0,
    ...overrides,
  };
}

function input(overrides: Partial<SupportReportInput> = {}): SupportReportInput {
  return {
    issueId: "addon-status",
    description: "Minion updated it, but Kalpa still shows an update.",
    appVersion: "0.1.0-beta.18",
    platform: "windows",
    environment: {
      osVersion: "10.0.26200",
      arch: "x86_64",
      tauri: "2.9.1",
      webview: "Chromium 138",
    },
    generatedAt: new Date("2026-08-28T12:00:00.000Z"),
    isOffline: false,
    checkingUpdates: false,
    addonsPath: "C:\\Users\\Brayden\\Documents\\Elder Scrolls Online\\live\\AddOns",
    instanceLabel: "Live — NA",
    addons: [addon({ missingDependencies: ["LibAddonMenu-2.0"], modifiedFileCount: 2 })],
    updateResults: [update()],
    lastError: "Could not scan C:\\Users\\Brayden\\Documents\\Elder Scrolls Online\\live\\AddOns",
    ...overrides,
  };
}

function decodeHandoff(url: string): SupportTicketPayload {
  const encoded = url.split("#kalpa=")[1]!;
  const base64 = encoded.replace(/-/g, "+").replace(/_/g, "/");
  const padded = base64.padEnd(Math.ceil(base64.length / 4) * 4, "=");
  const bytes = Uint8Array.from(atob(padded), (character) => character.charCodeAt(0));
  return JSON.parse(new TextDecoder().decode(bytes)) as SupportTicketPayload;
}

/**
 * True when the string contains no unpaired surrogate. `String#isWellFormed`
 * would say the same thing, but it is ES2024 and this project targets ES2022.
 */
function isWellFormed(value: string): boolean {
  for (let i = 0; i < value.length; i += 1) {
    const unit = value.charCodeAt(i);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = value.charCodeAt(i + 1);
      if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
      i += 1;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      return false;
    }
  }
  return true;
}

describe("buildSupportReport", () => {
  it.each(supportContractFixture.cases.map((entry) => [entry.name, entry] as const))(
    "renders the shared client/server contract fixture exactly: %s",
    (_name, entry) => {
      expect(renderSupportReport(entry.payload as SupportTicketPayload)).toBe(entry.report);
    }
  );

  it("hashes the report text with real SHA-256", () => {
    // The Worker uses the platform digest; this one is written out because the
    // dialog derives the report, its hash and the handoff URL synchronously.
    // Fixed vectors are what keep the two the same function.
    expect(sha256Hex("")).toBe("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    expect(sha256Hex("abc")).toBe(
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // Block-boundary and multi-block lengths, where a padding mistake hides.
    expect(sha256Hex("a".repeat(55))).toBe(
      "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"
    );
    expect(sha256Hex("a".repeat(56))).toBe(
      "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"
    );
    expect(sha256Hex("a".repeat(1000))).toBe(
      "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
    );
    // Astral characters are hashed as UTF-8, the same bytes every consumer sees.
    expect(sha256Hex("😀")).toBe(
      "f0443a342c5ef54783a111b51ba56c938e474c32324d90c3a60c9c8e3a37e2d9"
    );
  });

  it("seals every fixture case's hash against its own rendered report", () => {
    // The cross-repository invariant. The hosted page and the Worker each hold
    // their own hand-copied copy of these rules; the fixture pins one report
    // text and one digest of it, so any implementation whose rules drift fails
    // this assertion in its own repository.
    for (const entry of supportContractFixture.cases) {
      const payload = entry.payload as SupportTicketPayload;
      if (!("reportSha256" in payload)) continue;
      expect(payload.reportSha256).toBe(sha256Hex(entry.report));
      expect(payload.reportSha256).toBe(sha256Hex(renderSupportReport(payload)));
    }
  });

  it("seals the payload with the hash of the report the user reviewed", () => {
    const payload = buildSupportTicketPayload(input());

    expect(payload.version).toBe(3);
    expect(payload.reportSha256).toMatch(/^[0-9a-f]{64}$/);
    // The hash covers the rendered text, not the payload JSON: the invariant is
    // about what the user read.
    expect(payload.reportSha256).toBe(sha256Hex(renderSupportReport(payload)));
    // Sealing is idempotent, so the handoff carries the same payload the review
    // was built from rather than a second, subtly different one.
    expect(sealSupportTicketPayload(payload)).toEqual(payload);
  });

  it("seals a payload that had to be shrunk to fit, not the one before shrinking", () => {
    const payload = buildSupportTicketPayload(
      input({
        description: "Beim Aktualisieren erscheint eine unerwartete Fehlermeldung. ".repeat(20),
        lastError: "Ein unerwarteter Fehler ist aufgetreten. ".repeat(10),
        addons: [],
        updateResults: [],
      })
    );

    expect(renderSupportReport(payload).length).toBeLessThanOrEqual(SUPPORT_REPORT_MAX_LENGTH);
    expect(payload.reportSha256).toBe(sha256Hex(renderSupportReport(payload)));
  });

  it("keeps the fragment the same size whether or not the hash is real yet", () => {
    // Fitting measures the fragment while the hash is still the placeholder, so
    // a placeholder of a different width would make every size decision wrong.
    const payload = buildSupportTicketPayload(input());
    const unsealed = { ...payload, reportSha256: UNSEALED_REPORT_SHA256 };

    expect(UNSEALED_REPORT_SHA256).toHaveLength(64);
    expect(JSON.stringify(unsealed)).toHaveLength(JSON.stringify(payload).length);
  });

  it("refuses the handoff when the hash no longer covers the report", () => {
    // Stands in for a payload mutated after sealing. Both servers would reject
    // it; refusing here turns that into the copy/manual fallback instead of a
    // dead end the user only reaches after consenting in their browser.
    const payload = buildSupportTicketPayload(input());

    expect(buildSupportHandoffUrl({ ...payload, reportSha256: "0".repeat(64) })).toBeNull();
    expect(buildSupportHandoffUrl({ ...payload, instanceLabel: "Some other instance" })).toBeNull();
  });

  it("reports the allow-listed environment and nothing else", () => {
    const payload = buildSupportTicketPayload(input());
    const report = renderSupportReport(payload);

    expect(payload.version).toBe(3);
    expect(Object.keys(payload.environment!).sort()).toEqual([
      "arch",
      "osVersion",
      "tauri",
      "webview",
    ]);
    expect(report).toContain("- OS build: 10.0.26200");
    expect(report).toContain("- CPU architecture: x86_64");
    expect(report).toContain("- App runtime: Tauri 2.9.1, web view Chromium 138");
  });

  it("falls back to unknown instead of guessing when collection fails", () => {
    const report = buildSupportReport(input({ environment: UNKNOWN_SUPPORT_ENVIRONMENT }));

    expect(report).toContain("- OS build: unknown");
    expect(report).toContain("- CPU architecture: unknown");
    expect(report).toContain("- App runtime: Tauri unknown, web view unknown");
  });

  it("drops any environment value that is not a bounded allow-listed shape", () => {
    const payload = buildSupportTicketPayload(
      input({
        environment: {
          osVersion: "Windows 11 Home on DESKTOP-ABC123",
          arch: "quantum",
          tauri: "nightly-build",
          webview: "Chromium 138.0.3296.62",
        },
      })
    );

    expect(payload.environment).toEqual(UNKNOWN_SUPPORT_ENVIRONMENT);
    expect(renderSupportReport(payload)).not.toContain("DESKTOP-ABC123");
  });

  it("normalizes each environment field independently", () => {
    expect(normalizeOsVersion("10.0.26200")).toBe("10.0.26200");
    expect(normalizeOsVersion("14.5")).toBe("14.5");
    expect(normalizeOsVersion("6.8.0-51-generic")).toBe("unknown");
    expect(normalizeOsVersion("DESKTOP-ABC123")).toBe("unknown");
    expect(normalizeArchitecture("x86_64")).toBe("x86_64");
    expect(normalizeArchitecture("AArch64")).toBe("aarch64");
    expect(normalizeArchitecture("")).toBe("unknown");
    expect(normalizeRuntimeVersion("2.9.1")).toBe("2.9.1");
    expect(normalizeRuntimeVersion("2.9.1-beta.2")).toBe("2.9.1-beta.2");
    expect(normalizeRuntimeVersion("../../etc/passwd")).toBe("unknown");
    expect(
      normalizeWebviewLabel(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36"
      )
    ).toBe("Chromium 138");
    expect(
      normalizeWebviewLabel(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15"
      )
    ).toBe("WebKit 605");
    expect(normalizeWebviewLabel(undefined)).toBe("unknown");
  });

  it("never carries an identifying environment field through the handoff", () => {
    const encoded = JSON.stringify(buildSupportTicketPayload(input()));

    for (const forbidden of [
      "hostname",
      "username",
      "machineId",
      "deviceId",
      "serialNumber",
      "macAddress",
      "ipAddress",
      "locale",
      "discordUserId",
      "savedVariables",
    ]) {
      expect(encoded).not.toContain(forbidden);
    }
  });

  it("summarizes the state support needs for a stale addon report", () => {
    const report = buildSupportReport(input());

    expect(report).toContain("**Issue:** Addon status looks wrong");
    expect(report).toContain("1 checked, 1 update(s) reported");
    expect(report).toContain("Kalpa sees 1.0 -> 2.0");
    expect(report).toContain("1 missing dependency warning(s)");
    expect(report).toContain("2 locally modified file(s)");
  });

  it("counts outdated-only addons as dependency warnings and attention items", () => {
    const payload = buildSupportTicketPayload(
      input({
        addons: [
          addon({
            missingDependencies: [],
            outdatedDependencies: ["LibAsync"],
            modifiedFileCount: 0,
          }),
        ],
        updateResults: [],
      })
    );

    expect(payload.diagnostics.dependencyWarnings).toBe(1);
    expect(payload.diagnostics.attention).toEqual([
      expect.objectContaining({
        folder: "MapPins",
        missingDependencies: 0,
        outdatedDependencies: 1,
        modifiedFiles: 0,
      }),
    ]);
  });

  it("never includes the full local path or local identity", () => {
    const report = buildSupportReport(input());

    expect(report).toContain("AddOns folder: hidden");
    expect(report).not.toContain("Brayden");
    expect(report).toContain("Could not scan [AddOns folder]");
  });

  it("redacts usernames containing spaces, secrets, and account-like IDs in every free-text field", () => {
    const report = buildSupportReport(
      input({
        description:
          "See C:\\Users\\Jane Player\\Desktop and access_token=super-secret for 123456789012345678",
        lastError: "Bearer abc.def.ghi at /home/Jane Player/cache",
      })
    );

    expect(report).not.toContain("Jane Player");
    expect(report).not.toContain("Player");
    expect(report).not.toContain("super-secret");
    expect(report).not.toContain("abc.def.ghi");
    expect(report).not.toContain("123456789012345678");
    expect(report).toContain("[local path]");
    expect(report).toContain("[redacted]");
    expect(report).toContain("[account-id]");
  });

  it("redacts home and AddOns paths across Windows case and separator variants", () => {
    const report = buildSupportReport(
      input({
        lastError:
          "Could not scan c:/users/BRAYDEN/documents/elder scrolls online/live/addons; cache at D:/USERS/OtherName/AppData",
      })
    );

    expect(report).toContain("Could not scan [AddOns folder]");
    expect(report).toContain("[local path]");
    expect(report.toLowerCase()).not.toContain("brayden");
    expect(report).not.toContain("OtherName");
  });

  it("redacts non-home absolute paths and removes non-printing control characters", () => {
    const report = buildSupportReport(
      input({
        description: "Install failed at D:\\Games\\ESO\\AddOns and /opt/eso/private\u0007",
      })
    );

    expect(report.match(/\[local path\]/g)).toHaveLength(2);
    expect(report).not.toContain("Games");
    expect(report).not.toContain("/opt/");
    expect(report).not.toContain("\u0007");
  });

  it("redacts removable-media mounts and drive-less Windows paths", () => {
    // udisks mounts a secondary Steam library under /media or
    // /run/media/<username>, which Kalpa explicitly supports. Those carry the
    // account name exactly as /home does, and so does a Windows path that has
    // lost its drive letter.
    const report = buildSupportReport(
      input({
        description: "/run/media/bob/SteamLibrary/ESO failed; /media/bob/ext broke",
        lastError: "Users\\Brayden\\Documents\\Elder Scrolls Online\\live\\Backups is wrong",
      })
    );

    expect(report).not.toContain("bob");
    expect(report).not.toContain("Brayden");
    expect(report.match(/\[local path\]/g)).toHaveLength(3);
  });

  it("leaves ordinary prose that merely starts with a path keyword alone", () => {
    const report = buildSupportReport(input({ description: "Users of this addon report a crash" }));
    expect(report).toContain("Users of this addon report a crash");
  });

  it("never truncates through a surrogate pair", () => {
    // A cut by UTF-16 unit can leave a lone high surrogate. The string is then
    // not well-formed: JSON.stringify emits a bare \\ud83d, Discord rejects the
    // message, and because the private channel already exists every retry fails
    // identically -- the user could never be given the ticket that exists.
    const report = buildSupportReport(
      input({
        addons: [
          addon({
            // Addon names clamp at 80, so `truncate` cuts at 77. Placing the
            // emoji at units 76-77 puts the cut inside the pair.
            title: `${"A".repeat(76)}\u{1F600}${"B".repeat(20)}`,
            folderName: "SurrogateAddon",
            missingDependencies: ["LibAddonMenu-2.0"],
          }),
        ],
        updateResults: [update({ folderName: "SurrogateAddon" })],
      })
    );

    expect(isWellFormed(report)).toBe(true);
    expect(JSON.parse(JSON.stringify(report))).toBe(report);
  });

  it("preserves useful description paragraphs and explains category-specific collection", () => {
    const report = buildSupportReport(
      input({
        issueId: "log-upload",
        description: "The log was visible.\nThe upload stopped at the preparation step.",
      })
    );

    expect(report).toContain("The log was visible.\nThe upload stopped");
    expect(report).toContain(
      "Combat-log contents and account credentials are deliberately not collected"
    );
  });

  it("neutralizes Discord broadcast mentions and fits the ticket modal", () => {
    const addons = Array.from({ length: 50 }, (_, index) =>
      addon({
        folderName: `Addon-${index}`,
        title: `@everyone Addon ${index} ${"x".repeat(250)}`,
        missingDependencies: [`@here-Library-${index}`],
      })
    );
    const updateResults = addons.map((item, index) =>
      update({ folderName: item.folderName, esouiId: index + 1 })
    );

    const payload = buildSupportTicketPayload(
      input({
        description: "Hello @everyone and <@123456789>",
        addons,
        updateResults,
        lastError: "Ping @here and <@&987654321>",
      })
    );
    const report = renderSupportReport(payload);

    expect(report).not.toMatch(/@(everyone|here)/i);
    expect(report).not.toContain("<@123456789>");
    expect(report).not.toContain("<@&987654321>");
    expect(payload.diagnostics.attention.length).toBeLessThan(12);
    for (const item of payload.diagnostics.attention) {
      expect(report).toContain(`- ${item.name} (${item.folder}):`);
    }
    expect(report.length).toBeLessThanOrEqual(SUPPORT_REPORT_MAX_LENGTH);
  });

  it("encodes only the bounded structured payload in the secure handoff fragment", () => {
    const payload = buildSupportTicketPayload(input());
    const url = buildSupportHandoffUrl(payload);

    expect(url).toMatch(/^https:\/\/esotk\.com\/kalpa\/support#kalpa=/);
    expect(url).not.toContain("Brayden");
    expect(JSON.stringify(payload)).not.toContain("downloadUrl");
    expect(JSON.stringify(payload)).not.toContain("LibAddonMenu-2.0");
  });

  it("shows every transmitted attention item in the reviewed report", () => {
    const addons = Array.from({ length: 40 }, (_, index) =>
      addon({
        folderName: `Attention-${index.toString().padStart(2, "0")}`,
        title: `Attention item ${index.toString().padStart(2, "0")} ${"x".repeat(60)}`,
        missingDependencies: ["Missing"],
        modifiedFileCount: index % 2,
      })
    );
    const payload = buildSupportTicketPayload(
      input({
        addons,
        updateResults: addons.map((item) => update({ folderName: item.folderName })),
      })
    );
    const report = renderSupportReport(payload);
    const url = buildSupportHandoffUrl(payload)!;
    const transmitted = decodeHandoff(url);

    expect(transmitted).toEqual(payload);
    for (const item of transmitted.diagnostics.attention) {
      expect(report).toContain(`- ${item.name} (${item.folder}):`);
    }
    expect(report).not.toContain("...and");
  });

  // Filler lengths chosen so the 500/240-character cut lands INSIDE the
  // `[redacted]` token the first redaction pass produced. That is the case that
  // used to diverge: the server sees `bearer [red...`, treats it as a fresh
  // secret, and expands it back to `bearer [redacted]`.
  const CUT_INTO_REDACTION = Array.from({ length: 12 }, (_, index) => 480 + index);

  it.each(CUT_INTO_REDACTION)(
    "emits text the hosted page and Worker re-validate unchanged (filler %i)",
    (filler) => {
      const description = `${"x".repeat(filler)} bearer supersecretlongvalue`;
      const lastError = `${"y".repeat(filler - 258)} bearer supersecretlongvalue`;
      const payload = buildSupportTicketPayload(
        input({ description, lastError, addonsPath: "", addons: [], updateResults: [] })
      );

      expect(payload.description.length).toBeLessThanOrEqual(500);
      expect(isCanonicalSupportText(payload.description, 500, true)).toBe(true);
      expect(isCanonicalSupportText(payload.diagnostics.lastError!, 240)).toBe(true);
      // The reviewed report is what the server would re-derive, byte for byte.
      expect(renderSupportReport(payload)).toBe(renderSupportReport(payload));
    }
  );

  it("keeps a shrunk report byte-identical to what the server would re-derive", () => {
    const payload = buildSupportTicketPayload(
      input({
        // Long enough to force the shrink loop, with a redaction target sitting
        // right where the cut lands.
        description: `${"Beim Aktualisieren erscheint eine Fehlermeldung. ".repeat(9)}bearer supersecretvalue`,
        lastError: `${"Ein unerwarteter Fehler ist aufgetreten. ".repeat(5)}token abcdefghijklmnopqrstuvwxyz`,
        instanceLabel: "Live — NA (Steam) — sehr langer Instanzname zum Testen",
        addonsPath: "",
        addons: [],
        updateResults: [],
      })
    );

    expect(renderSupportReport(payload).length).toBeLessThanOrEqual(SUPPORT_REPORT_MAX_LENGTH);
    expect(isCanonicalSupportText(payload.description, 500, true)).toBe(true);
    expect(isCanonicalSupportText(payload.diagnostics.lastError!, 240)).toBe(true);
    expect(buildSupportHandoffUrl(payload)).not.toBeNull();
  });

  it("shrinks the reviewed free text so a maximal report still fits the transport", () => {
    const payload = buildSupportTicketPayload(
      input({
        description: "Beim Aktualisieren erscheint eine unerwartete Fehlermeldung. ".repeat(20),
        instanceLabel: "Live — NA (Steam) — sehr langer Instanzname zum Testen",
        lastError: "Ein unerwarteter Fehler ist aufgetreten. ".repeat(10),
        addons: [],
        updateResults: [],
      })
    );
    const report = renderSupportReport(payload);

    expect(report.length).toBeLessThanOrEqual(SUPPORT_REPORT_MAX_LENGTH);
    // What was shortened is shortened in the review the user reads, not silently
    // on the way out, so the reviewed and transmitted reports still match.
    expect(report).toContain(payload.description);
    expect(report).toContain(payload.diagnostics.lastError!);
    expect(buildSupportHandoffUrl(payload)).not.toBeNull();
    expect(decodeHandoff(buildSupportHandoffUrl(payload)!)).toEqual(payload);
  });

  it("refuses a handoff whose platform the hosted page and Worker would reject", () => {
    const payload = buildSupportTicketPayload(input({ platform: "haiku-os" }));

    expect(renderSupportReport(payload)).toContain("- Platform: haiku-os");
    // Better a visible "could not be prepared" with copy/manual still offered
    // than a rejection the user only meets after consenting.
    expect(buildSupportHandoffUrl(payload)).toBeNull();
  });

  it("refuses the handoff instead of preparing a report the hosted page would reject", () => {
    const payload = buildSupportTicketPayload(input());
    const oversized: SupportTicketPayload = {
      ...payload,
      instanceLabel: "x".repeat(80),
      appVersion: "y".repeat(40),
      platform: "z".repeat(40),
      diagnostics: {
        ...payload.diagnostics,
        attention: Array.from({ length: 12 }, () => ({
          name: "n".repeat(80),
          folder: "f".repeat(80),
          currentVersion: "1.0",
          availableVersion: "2.0",
          missingDependencies: 9999,
          outdatedDependencies: 9999,
          modifiedFiles: 9999,
        })),
      },
    };

    expect(renderSupportReport(oversized).length).toBeLessThanOrEqual(SUPPORT_REPORT_MAX_LENGTH);
    expect(renderSupportReport(fitSupportTicketPayload(oversized))).toBe(
      renderSupportReport(oversized)
    );
  });

  it("fits a worst-case Unicode payload within the actual fragment limit", () => {
    const addons = Array.from({ length: 40 }, (_, index) =>
      addon({
        folderName: `插件😀-${index}-${"界".repeat(60)}`,
        title: `修改😀-${index}-${"界".repeat(60)}`,
        missingDependencies: ["缺少"],
        outdatedDependencies: ["过时"],
        modifiedFileCount: 9999,
      })
    );
    const payload = buildSupportTicketPayload(
      input({
        description: "😀".repeat(500),
        appVersion: "界".repeat(40),
        instanceLabel: "界".repeat(80),
        lastError: "界".repeat(240),
        addons,
        updateResults: addons.map((item, index) =>
          update({
            folderName: item.folderName,
            esouiId: index + 1,
            currentVersion: "界".repeat(40),
            remoteVersion: "界".repeat(40),
          })
        ),
      })
    );
    const report = renderSupportReport(payload);
    const url = buildSupportHandoffUrl(payload);

    expect(url).not.toBeNull();
    expect(url!.split("#")[1]!.length).toBeLessThanOrEqual(SUPPORT_HANDOFF_MAX_FRAGMENT_LENGTH);
    expect(report.length).toBeLessThanOrEqual(SUPPORT_REPORT_MAX_LENGTH);
    expect(decodeHandoff(url!)).toEqual(payload);
  });
});
