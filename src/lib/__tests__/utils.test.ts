import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { cn, formatRelativeDate, formatRelativeExpiry, formatBytes, decodeHtml } from "../utils";

describe("cn", () => {
  it("merges class names", () => {
    expect(cn("foo", "bar")).toBe("foo bar");
  });

  it("handles conditional classes", () => {
    const condition = false;
    expect(cn("base", condition && "hidden", "visible")).toBe("base visible");
  });

  it("deduplicates tailwind conflicts", () => {
    expect(cn("p-4", "p-2")).toBe("p-2");
  });

  it("returns empty string for no input", () => {
    expect(cn()).toBe("");
  });
});

describe("formatRelativeDate", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-05-10T12:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns empty string for invalid ISO", () => {
    expect(formatRelativeDate("not-a-date")).toBe("");
  });

  it("returns 'just now' for timestamps within the last minute", () => {
    const thirtySecsAgo = new Date("2026-05-10T11:59:30Z").toISOString();
    expect(formatRelativeDate(thirtySecsAgo)).toBe("just now");
  });

  it("returns 'Today' for future timestamps", () => {
    const future = new Date("2026-05-10T13:00:00Z").toISOString();
    expect(formatRelativeDate(future)).toBe("Today");
  });

  it("formats minutes correctly (singular)", () => {
    const oneMinAgo = new Date("2026-05-10T11:59:00Z").toISOString();
    expect(formatRelativeDate(oneMinAgo)).toBe("1 minute ago");
  });

  it("formats minutes correctly (plural)", () => {
    const fiveMinAgo = new Date("2026-05-10T11:55:00Z").toISOString();
    expect(formatRelativeDate(fiveMinAgo)).toBe("5 minutes ago");
  });

  it("formats hours correctly (singular)", () => {
    const oneHourAgo = new Date("2026-05-10T11:00:00Z").toISOString();
    expect(formatRelativeDate(oneHourAgo)).toBe("1 hour ago");
  });

  it("formats hours correctly (plural)", () => {
    const threeHoursAgo = new Date("2026-05-10T09:00:00Z").toISOString();
    expect(formatRelativeDate(threeHoursAgo)).toBe("3 hours ago");
  });

  it("formats days correctly", () => {
    const twoDaysAgo = new Date("2026-05-08T12:00:00Z").toISOString();
    expect(formatRelativeDate(twoDaysAgo)).toBe("2 days ago");
  });

  it("formats months correctly", () => {
    const twoMonthsAgo = new Date("2026-03-10T12:00:00Z").toISOString();
    expect(formatRelativeDate(twoMonthsAgo)).toBe("2 months ago");
  });

  it("formats years correctly", () => {
    const twoYearsAgo = new Date("2024-05-10T12:00:00Z").toISOString();
    expect(formatRelativeDate(twoYearsAgo)).toBe("2 years ago");
  });
});

describe("formatRelativeExpiry", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-05-10T12:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns empty string for invalid ISO", () => {
    expect(formatRelativeExpiry("bad")).toBe("");
  });

  it("returns 'expired' for past timestamps", () => {
    const past = new Date("2026-05-10T11:00:00Z").toISOString();
    expect(formatRelativeExpiry(past)).toBe("expired");
  });

  it("formats less than a minute", () => {
    const soonish = new Date("2026-05-10T12:00:30Z").toISOString();
    expect(formatRelativeExpiry(soonish)).toBe("Expires in less than a minute");
  });

  it("formats minutes (singular)", () => {
    const oneMin = new Date("2026-05-10T12:01:30Z").toISOString();
    expect(formatRelativeExpiry(oneMin)).toBe("Expires in ~1 minute");
  });

  it("formats minutes (plural)", () => {
    const fiveMin = new Date("2026-05-10T12:05:30Z").toISOString();
    expect(formatRelativeExpiry(fiveMin)).toBe("Expires in ~5 minutes");
  });

  it("formats hours (singular)", () => {
    const oneHour = new Date("2026-05-10T13:00:00Z").toISOString();
    expect(formatRelativeExpiry(oneHour)).toBe("Expires in ~1 hour");
  });

  it("formats hours (plural)", () => {
    const threeHours = new Date("2026-05-10T15:00:00Z").toISOString();
    expect(formatRelativeExpiry(threeHours)).toBe("Expires in ~3 hours");
  });

  it("formats days (plural)", () => {
    const threeDays = new Date("2026-05-13T12:00:00Z").toISOString();
    expect(formatRelativeExpiry(threeDays)).toBe("Expires in ~3 days");
  });
});

describe("decodeHtml", () => {
  it("decodes HTML entities", () => {
    expect(decodeHtml("&amp;")).toBe("&");
    expect(decodeHtml("&lt;script&gt;")).toBe("<script>");
  });

  it("returns plain text unchanged", () => {
    expect(decodeHtml("hello world")).toBe("hello world");
  });

  it("handles empty string", () => {
    expect(decodeHtml("")).toBe("");
  });

  it("decodes numeric entities", () => {
    expect(decodeHtml("&#39;")).toBe("'");
    expect(decodeHtml("&#x27;")).toBe("'");
  });

  it("decodes astral numeric entities without corrupting surrogate pairs", () => {
    expect(decodeHtml("&#128512; &#x1F600;")).toBe("😀 😀");
  });

  it("leaves invalid numeric entities unchanged", () => {
    expect(decodeHtml("&#999999999;")).toBe("&#999999999;");
  });

  it("leaves NUL and surrogate numeric entities unchanged", () => {
    expect(decodeHtml("&#0;")).toBe("&#0;");
    expect(decodeHtml("&#xD800;")).toBe("&#xD800;");
    expect(decodeHtml("&#xDFFF;")).toBe("&#xDFFF;");
    expect(decodeHtml("&#55296;")).toBe("&#55296;");
  });
});

describe("formatBytes", () => {
  it("formats bytes", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1023)).toBe("1023 B");
  });

  it("formats kilobytes", () => {
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(10240)).toBe("10.0 KB");
  });

  it("formats megabytes", () => {
    expect(formatBytes(1048576)).toBe("1.0 MB");
    expect(formatBytes(1572864)).toBe("1.5 MB");
  });

  it("formats gigabytes", () => {
    expect(formatBytes(1073741824)).toBe("1.0 GB");
    expect(formatBytes(2147483648)).toBe("2.0 GB");
  });

  it("handles boundary values", () => {
    expect(formatBytes(1024 * 1024 - 1)).toBe("1024.0 KB");
    expect(formatBytes(1024 * 1024)).toBe("1.0 MB");
  });
});
