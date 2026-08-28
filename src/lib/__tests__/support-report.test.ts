import { describe, expect, it } from "vitest";
import type { AddonManifest, UpdateCheckResult } from "../../types";
import {
  buildSupportHandoffUrl,
  buildSupportReport,
  buildSupportTicketPayload,
  SUPPORT_REPORT_MAX_LENGTH,
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

describe("buildSupportReport", () => {
  it("summarizes the state support needs for a stale addon report", () => {
    const report = buildSupportReport(input());

    expect(report).toContain("**Issue:** Addon status looks wrong");
    expect(report).toContain("1 checked, 1 update(s) reported");
    expect(report).toContain("Kalpa sees 1.0 -> 2.0");
    expect(report).toContain("1 missing dependency warning(s)");
    expect(report).toContain("2 locally modified file(s)");
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

    const report = buildSupportReport(
      input({
        description: "Hello @everyone and <@123456789>",
        addons,
        updateResults,
        lastError: "Ping @here and <@&987654321>",
      })
    );

    expect(report).not.toMatch(/@(everyone|here)/i);
    expect(report).not.toContain("<@123456789>");
    expect(report).not.toContain("<@&987654321>");
    expect(report).toContain("...and ");
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
});
