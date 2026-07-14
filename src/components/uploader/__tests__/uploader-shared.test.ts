/// <reference types="node" />

import { describe, expect, it, vi } from "vitest";
import { CompressionStream as NodeCompressionStream } from "node:stream/web";
import { inflateRawSync } from "node:zlib";

import {
  attachRaceSafe,
  deriveNativeState,
  esoLogsReportUrl,
  esotkReportUrl,
  esotkReportUrlForOpen,
  KALPA_BUILD_EVIDENCE_DEFLATE_PARAM,
  KALPA_BUILD_EVIDENCE_PARAM,
  liveExitConfirmCopy,
  parseReportCode,
  primaryReportUrl,
  shouldConfirmLiveExit,
} from "../uploader-shared";

const globalWithStreams = globalThis as typeof globalThis & {
  CompressionStream?: typeof CompressionStream;
};

if (typeof globalWithStreams.CompressionStream === "undefined") {
  globalWithStreams.CompressionStream = NodeCompressionStream as typeof CompressionStream;
}

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

function decodeDeflateBase64UrlJson(encoded: string): unknown {
  const base64 = encoded.replace(/-/g, "+").replace(/_/g, "/");
  const padded = base64.padEnd(Math.ceil(base64.length / 4) * 4, "=");
  return JSON.parse(inflateRawSync(Buffer.from(padded, "base64")).toString("utf8"));
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
          scribedSkills: [
            { abilityId: 220543, name: "Dazing Trample", icon: "ability_grimoire_assault" },
          ],
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

  it("uses a compressed native build evidence param for opened analysis links", async () => {
    const evidence = {
      schemaVersion: 1,
      source: "kalpa-native-player-info",
      reportCode: "1ZG23pzRcvMT8V46",
      players: Array.from({ length: 70 }, (_, index) => ({
        unitId: String(index + 1),
        unitOccurrenceId: String(index + 1),
        characterName: `Player ${index + 1}`,
        accountName: `@player${index + 1}`,
        classId: 2,
        raceId: 9,
        level: 50,
        championPoints: 1700 + index,
        className: "Sorcerer",
        classMasteryPassives: [263870, 263871],
        championPointPassives: [142210, 142079, 141993, 141991],
        evidence: "raw-player-info",
        confidence: "exact",
      })),
    };

    const url = await esotkReportUrlForOpen("1ZG23pzRcvMT8V46", { buildEvidence: evidence });
    const hashQuery = new URL(url).hash.split("?")[1];
    const params = new URLSearchParams(hashQuery);
    const encoded = params.get(KALPA_BUILD_EVIDENCE_DEFLATE_PARAM);

    expect(params.get(KALPA_BUILD_EVIDENCE_PARAM)).toBeNull();
    expect(encoded).toBeTruthy();
    expect(url.length).toBeLessThan(8_000);
    expect(decodeDeflateBase64UrlJson(encoded!)).toEqual(evidence);
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

describe("shouldConfirmLiveExit", () => {
  it("confirms only while a live session is active", () => {
    expect(shouldConfirmLiveExit(true)).toBe(true);
    expect(shouldConfirmLiveExit(false)).toBe(false);
  });
});

describe("liveExitConfirmCopy", () => {
  it("uses stop-tracking copy on the handoff path (uploader keeps streaming)", () => {
    const copy = liveExitConfirmCopy(true);
    expect(copy.title).toBe("Stop tracking in Kalpa?");
    expect(copy.confirmLabel).toBe("Stop tracking");
    expect(copy.description).toContain("keeps streaming");
  });

  it("warns the report closes on the native path", () => {
    const copy = liveExitConfirmCopy(false);
    expect(copy.title).toContain("close the report on ESO Logs");
    expect(copy.confirmLabel).toBe("Stop upload");
    expect(copy.description).toContain("closes the report on ESO Logs");
  });
});

describe("attachRaceSafe", () => {
  it("tears the listener down when unmount races the subscription resolution", async () => {
    let resolveSub: (fn: () => void) => void = () => {};
    const subscribe = () => new Promise<() => void>((res) => (resolveSub = res));
    const unlisten = vi.fn();

    const cleanup = attachRaceSafe(subscribe);
    // Unmount BEFORE the subscription resolves.
    cleanup();
    // Now the subscription resolves — the listener must be torn down immediately
    // instead of leaking.
    resolveSub(unlisten);
    await Promise.resolve();
    await Promise.resolve();

    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("runs the listener cleanup on a normal unmount", async () => {
    const unlisten = vi.fn();
    const cleanup = attachRaceSafe(() => Promise.resolve(unlisten));
    await Promise.resolve();
    await Promise.resolve();

    expect(unlisten).not.toHaveBeenCalled();
    cleanup();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("swallows a rejected subscription without throwing", async () => {
    const cleanup = attachRaceSafe(() => Promise.reject(new Error("no webview")));
    await Promise.resolve();
    await Promise.resolve();
    // Cleanup is a no-op (nothing to tear down) and must not throw.
    expect(() => cleanup()).not.toThrow();
  });
});

describe("deriveNativeState", () => {
  it("opts into native when neither opt-out is set and the store is trusted", () => {
    expect(
      deriveNativeState({
        manual: { ok: true, value: false },
        live: { ok: true, value: false },
        session: true,
        tainted: false,
      })
    ).toEqual({ nativeOptIn: true, hasNativeSession: true, liveUseOfficial: false });
  });

  it("fails closed on a failed store read (opted out; official for live)", () => {
    expect(
      deriveNativeState({
        manual: { ok: false, value: false },
        live: { ok: true, value: false },
        session: true,
        tainted: false,
      })
    ).toEqual({ nativeOptIn: false, hasNativeSession: true, liveUseOfficial: true });
  });

  it("fails closed on a tainted store", () => {
    expect(
      deriveNativeState({
        manual: { ok: true, value: false },
        live: { ok: true, value: false },
        session: false,
        tainted: true,
      })
    ).toEqual({ nativeOptIn: false, hasNativeSession: false, liveUseOfficial: true });
  });

  it("reports no session but still applies the opt-outs when the session check failed", () => {
    // uploader_has_session rejects → session=false via the caller's .catch; the
    // derivation still applies the opt-outs. This is the F3 fail-closed guarantee:
    // the old named copy lacked the .catch and threw here, skipping every setState.
    expect(
      deriveNativeState({
        manual: { ok: true, value: true },
        live: { ok: true, value: false },
        session: false,
        tainted: false,
      })
    ).toEqual({ nativeOptIn: false, hasNativeSession: false, liveUseOfficial: false });
  });
});
