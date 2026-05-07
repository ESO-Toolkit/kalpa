import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cn, decodeHtml, formatBytes, formatRelativeDate, formatRelativeExpiry } from "./utils";

describe("cn", () => {
  it("merges class names", () => {
    expect(cn("a", "b")).toBe("a b");
  });

  it("dedupes conflicting tailwind classes (twMerge)", () => {
    expect(cn("p-2", "p-4")).toBe("p-4");
  });

  it("handles falsy values from clsx", () => {
    const flag: boolean = false;
    expect(cn("a", flag && "b", undefined, null, "c")).toBe("a c");
  });

  it("handles conditional object syntax", () => {
    expect(cn("a", { b: true, c: false })).toBe("a b");
  });
});

describe("formatBytes", () => {
  it("formats bytes under 1KB as B", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1023)).toBe("1023 B");
  });

  it("formats KB with one decimal", () => {
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(1536)).toBe("1.5 KB");
  });

  it("formats MB with one decimal", () => {
    expect(formatBytes(1024 * 1024)).toBe("1.0 MB");
    expect(formatBytes(5 * 1024 * 1024 + 512 * 1024)).toBe("5.5 MB");
  });

  it("formats GB with one decimal", () => {
    expect(formatBytes(1024 * 1024 * 1024)).toBe("1.0 GB");
    expect(formatBytes(2.5 * 1024 * 1024 * 1024)).toBe("2.5 GB");
  });

  it("crosses unit boundaries cleanly", () => {
    // Just under 1 MB should still be KB
    expect(formatBytes(1024 * 1024 - 1)).toMatch(/KB$/);
    // Just at 1 MB
    expect(formatBytes(1024 * 1024)).toMatch(/MB$/);
  });
});

describe("decodeHtml", () => {
  it("decodes named entities", () => {
    expect(decodeHtml("Tom &amp; Jerry")).toBe("Tom & Jerry");
  });

  it("decodes numeric entities", () => {
    expect(decodeHtml("&#169; 2026")).toBe("© 2026");
  });

  it("returns empty string unchanged", () => {
    expect(decodeHtml("")).toBe("");
  });

  it("returns plain text unchanged", () => {
    expect(decodeHtml("hello world")).toBe("hello world");
  });
});

describe("formatRelativeDate", () => {
  const now = new Date("2026-05-07T12:00:00Z");

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(now);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns empty string for invalid date", () => {
    expect(formatRelativeDate("not-a-date")).toBe("");
  });

  it("returns 'Today' for future dates", () => {
    const future = new Date(now.getTime() + 60_000).toISOString();
    expect(formatRelativeDate(future)).toBe("Today");
  });

  it("returns 'just now' for under a minute", () => {
    const ts = new Date(now.getTime() - 30_000).toISOString();
    expect(formatRelativeDate(ts)).toBe("just now");
  });

  it("pluralizes minutes correctly", () => {
    expect(formatRelativeDate(new Date(now.getTime() - 60_000).toISOString())).toBe("1 minute ago");
    expect(formatRelativeDate(new Date(now.getTime() - 5 * 60_000).toISOString())).toBe(
      "5 minutes ago"
    );
  });

  it("formats hours", () => {
    expect(formatRelativeDate(new Date(now.getTime() - 60 * 60_000).toISOString())).toBe(
      "1 hour ago"
    );
    expect(formatRelativeDate(new Date(now.getTime() - 3 * 60 * 60_000).toISOString())).toBe(
      "3 hours ago"
    );
  });

  it("formats days", () => {
    const oneDay = 24 * 60 * 60_000;
    expect(formatRelativeDate(new Date(now.getTime() - oneDay).toISOString())).toBe("1 day ago");
    expect(formatRelativeDate(new Date(now.getTime() - 7 * oneDay).toISOString())).toBe(
      "7 days ago"
    );
  });

  it("formats months", () => {
    const days = (n: number) => new Date(now.getTime() - n * 24 * 60 * 60_000).toISOString();
    expect(formatRelativeDate(days(30))).toBe("1 month ago");
    expect(formatRelativeDate(days(90))).toBe("3 months ago");
  });

  it("formats years", () => {
    const days = (n: number) => new Date(now.getTime() - n * 24 * 60 * 60_000).toISOString();
    // 12 * 30 = 360 days = 1 year
    expect(formatRelativeDate(days(365))).toBe("1 year ago");
    expect(formatRelativeDate(days(365 * 3))).toBe("3 years ago");
  });
});

describe("formatRelativeExpiry", () => {
  const now = new Date("2026-05-07T12:00:00Z");

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(now);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns empty string for invalid date", () => {
    expect(formatRelativeExpiry("not-a-date")).toBe("");
  });

  it("returns 'expired' for past or now", () => {
    expect(formatRelativeExpiry(now.toISOString())).toBe("expired");
    expect(formatRelativeExpiry(new Date(now.getTime() - 1000).toISOString())).toBe("expired");
  });

  it("returns 'less than a minute' for sub-minute future", () => {
    expect(formatRelativeExpiry(new Date(now.getTime() + 30_000).toISOString())).toBe(
      "Expires in less than a minute"
    );
  });

  it("pluralizes minutes", () => {
    expect(formatRelativeExpiry(new Date(now.getTime() + 60_000).toISOString())).toBe(
      "Expires in ~1 minute"
    );
    expect(formatRelativeExpiry(new Date(now.getTime() + 5 * 60_000).toISOString())).toBe(
      "Expires in ~5 minutes"
    );
  });

  it("formats hours and days", () => {
    expect(formatRelativeExpiry(new Date(now.getTime() + 60 * 60_000).toISOString())).toBe(
      "Expires in ~1 hour"
    );
    expect(formatRelativeExpiry(new Date(now.getTime() + 2 * 24 * 60 * 60_000).toISOString())).toBe(
      "Expires in ~2 days"
    );
  });
});
