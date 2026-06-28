import { describe, expect, it } from "vitest";

import {
  esoLogsReportUrl,
  esotkReportUrl,
  parseReportCode,
  primaryReportUrl,
} from "../uploader-shared";

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
