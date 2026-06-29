import { describe, expect, it } from "vitest";

import {
  esoLogsReportUrl,
  esotkReportUrl,
  KALPA_BUILD_EVIDENCE_PARAM,
  parseReportCode,
  primaryReportUrl,
} from "../uploader-shared";

function decodeBase64UrlJson(encoded: string): unknown {
  const base64 = encoded.replace(/-/g, "+").replace(/_/g, "/");
  const padded = base64.padEnd(Math.ceil(base64.length / 4) * 4, "=");
  const binary = atob(padded);
  const percentEncoded = Array.from(
    binary,
    (char) => `%${char.charCodeAt(0).toString(16).padStart(2, "0")}`
  ).join("");
  return JSON.parse(decodeURIComponent(percentEncoded));
}

describe("esotkReportUrl", () => {
  it("uses the hash route for stable esotk deep links", () => {
    expect(esotkReportUrl("1ZG23pzRcvMT8V46")).toBe("https://esotk.com/#/report/1ZG23pzRcvMT8V46");
  });

  it("targets the live view through the hash route", () => {
    expect(esotkReportUrl("1ZG23pzRcvMT8V46", { live: true })).toBe(
      "https://esotk.com/#/report/1ZG23pzRcvMT8V46/live"
    );
  });

  it("encodes malformed codes without escaping the fragment path", () => {
    expect(esotkReportUrl("bad/code?x=1", { live: true })).toBe(
      "https://esotk.com/#/report/bad%2Fcode%3Fx%3D1/live"
    );
  });

  it("carries native build evidence on completed report analysis links", () => {
    const evidence = {
      schemaVersion: 1,
      source: "kalpa-native-player-info",
      reportCode: "1ZG23pzRcvMT8V46",
      players: [
        {
          unitId: "1",
          characterName: "Arc Spark",
          accountName: "@tester",
          classId: 2,
          raceId: 9,
          level: 50,
          championPoints: 1700,
          className: "Sorcerer",
          classMasteryPassives: [263870, 263871],
          frontBarSkillIds: [38901, 29489],
          backBarSkillIds: [23231, 23234],
          evidence: "raw-player-info",
          confidence: "exact",
        },
      ],
    };

    const url = esotkReportUrl("1ZG23pzRcvMT8V46", { buildEvidence: evidence });
    const hashQuery = new URL(url).hash.split("?")[1];
    const encoded = new URLSearchParams(hashQuery).get(KALPA_BUILD_EVIDENCE_PARAM);

    expect(encoded).toBeTruthy();
    expect(decodeBase64UrlJson(encoded!)).toEqual(evidence);
  });

  it("does not carry mismatched evidence or live evidence", () => {
    const evidence = {
      schemaVersion: 1,
      source: "kalpa-native-player-info",
      reportCode: "OTHER",
      players: [
        {
          unitId: "1",
          classMasteryPassives: [263870],
          frontBarSkillIds: [],
          backBarSkillIds: [],
          evidence: "raw-player-info",
          confidence: "exact",
        },
      ],
    };

    expect(esotkReportUrl("1ZG23pzRcvMT8V46", { buildEvidence: evidence })).toBe(
      "https://esotk.com/#/report/1ZG23pzRcvMT8V46"
    );
    expect(esotkReportUrl("1ZG23pzRcvMT8V46", { live: true, buildEvidence: evidence })).toBe(
      "https://esotk.com/#/report/1ZG23pzRcvMT8V46/live"
    );
  });
});

describe("primaryReportUrl", () => {
  const report = {
    code: "1ZG23pzRcvMT8V46",
    url: "https://www.esologs.com/reports/1ZG23pzRcvMT8V46",
  };

  it("uses esotk analysis for public and unlisted completed reports", () => {
    expect(primaryReportUrl(report, "public")).toBe("https://esotk.com/#/report/1ZG23pzRcvMT8V46");
    expect(primaryReportUrl(report, "unlisted")).toBe(
      "https://esotk.com/#/report/1ZG23pzRcvMT8V46"
    );
  });

  it("uses raw ESO Logs fight=last for active live reports", () => {
    expect(primaryReportUrl(report, "unlisted", { live: true })).toBe(
      "https://www.esologs.com/reports/1ZG23pzRcvMT8V46?fight=last"
    );
  });

  it("uses raw ESO Logs links for private reports", () => {
    expect(primaryReportUrl(report, "private")).toBe(report.url);
    expect(primaryReportUrl(report, "private", { live: true })).toBe(
      "https://www.esologs.com/reports/1ZG23pzRcvMT8V46?fight=last"
    );
  });
});

describe("esoLogsReportUrl", () => {
  it("adds fight=last for live reports", () => {
    expect(
      esoLogsReportUrl(
        {
          code: "1ZG23pzRcvMT8V46",
          url: "https://www.esologs.com/reports/1ZG23pzRcvMT8V46",
        },
        { live: true }
      )
    ).toBe("https://www.esologs.com/reports/1ZG23pzRcvMT8V46?fight=last");
  });

  it("preserves existing query parameters when adding fight=last", () => {
    expect(
      esoLogsReportUrl(
        {
          code: "1ZG23pzRcvMT8V46",
          url: "https://www.esologs.com/reports/1ZG23pzRcvMT8V46?source=2",
        },
        { live: true }
      )
    ).toBe("https://www.esologs.com/reports/1ZG23pzRcvMT8V46?source=2&fight=last");
  });
});

describe("parseReportCode", () => {
  it("accepts canonical ESO Logs report links", () => {
    expect(parseReportCode("https://www.esologs.com/reports/1ZG23pzRcvMT8V46?fight=2")).toBe(
      "1ZG23pzRcvMT8V46"
    );
  });

  it("accepts esotk analysis and live hash links", () => {
    expect(parseReportCode("https://esotk.com/#/report/1ZG23pzRcvMT8V46/live")).toBe(
      "1ZG23pzRcvMT8V46"
    );
  });

  it("accepts bare mixed report codes", () => {
    expect(parseReportCode(" 1ZG23pzRcvMT8V46 ")).toBe("1ZG23pzRcvMT8V46");
  });

  it("rejects ordinary words and short numeric ids", () => {
    expect(parseReportCode("unlisted")).toBeNull();
    expect(parseReportCode("123456789012")).toBeNull();
  });
});
