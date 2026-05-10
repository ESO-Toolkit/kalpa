import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { formatRelativeDate, formatRelativeExpiry, formatBytes } from "../utils";

describe("formatRelativeDate — boundary conditions", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-05-10T12:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns 'just now' at exactly 0 seconds ago", () => {
    expect(formatRelativeDate("2026-05-10T12:00:00Z")).toBe("just now");
  });

  it("returns 'just now' at exactly 59 seconds ago", () => {
    expect(formatRelativeDate("2026-05-10T11:59:01Z")).toBe("just now");
  });

  it("switches to minutes at exactly 60 seconds", () => {
    expect(formatRelativeDate("2026-05-10T11:59:00Z")).toBe("1 minute ago");
  });

  it("switches to hours at exactly 60 minutes", () => {
    expect(formatRelativeDate("2026-05-10T11:00:00Z")).toBe("1 hour ago");
  });

  it("switches to days at exactly 24 hours", () => {
    expect(formatRelativeDate("2026-05-09T12:00:00Z")).toBe("1 day ago");
  });

  it("switches to months at exactly 30 days", () => {
    expect(formatRelativeDate("2026-04-10T12:00:00Z")).toBe("1 month ago");
  });

  it("switches to years at exactly 12 months (360 days)", () => {
    expect(formatRelativeDate("2025-05-15T12:00:00Z")).toBe("1 year ago");
  });

  it("handles very old dates", () => {
    expect(formatRelativeDate("2000-01-01T00:00:00Z")).toBe("26 years ago");
  });

  it("handles timestamps just barely in the past", () => {
    const almostNow = new Date("2026-05-10T11:59:59.500Z").toISOString();
    expect(formatRelativeDate(almostNow)).toBe("just now");
  });

  it("handles date-only ISO strings", () => {
    const result = formatRelativeDate("2026-05-09");
    expect(result).toMatch(/day/);
  });

  it("handles timezone offset ISO strings", () => {
    const result = formatRelativeDate("2026-05-10T06:00:00-06:00");
    expect(result).toBe("just now");
  });
});

describe("formatRelativeExpiry — boundary conditions", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-05-10T12:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns 'expired' at exactly the current time", () => {
    expect(formatRelativeExpiry("2026-05-10T12:00:00Z")).toBe("expired");
  });

  it("returns 'less than a minute' at 1 second remaining", () => {
    expect(formatRelativeExpiry("2026-05-10T12:00:01Z")).toBe("Expires in less than a minute");
  });

  it("returns 'less than a minute' at 59 seconds remaining", () => {
    expect(formatRelativeExpiry("2026-05-10T12:00:59Z")).toBe("Expires in less than a minute");
  });

  it("switches to minutes at exactly 60 seconds", () => {
    expect(formatRelativeExpiry("2026-05-10T12:01:00Z")).toBe("Expires in ~1 minute");
  });

  it("switches to hours at exactly 60 minutes", () => {
    expect(formatRelativeExpiry("2026-05-10T13:00:00Z")).toBe("Expires in ~1 hour");
  });

  it("switches to days at exactly 24 hours", () => {
    expect(formatRelativeExpiry("2026-05-11T12:00:00Z")).toBe("Expires in ~1 day");
  });

  it("handles very far future expiry", () => {
    expect(formatRelativeExpiry("2027-05-10T12:00:00Z")).toBe("Expires in ~365 days");
  });
});

describe("formatBytes — edge cases", () => {
  it("handles exactly 0 bytes", () => {
    expect(formatBytes(0)).toBe("0 B");
  });

  it("handles 1 byte", () => {
    expect(formatBytes(1)).toBe("1 B");
  });

  it("handles exactly 1 KB boundary", () => {
    expect(formatBytes(1024)).toBe("1.0 KB");
  });

  it("handles exactly 1 MB boundary", () => {
    expect(formatBytes(1024 * 1024)).toBe("1.0 MB");
  });

  it("handles exactly 1 GB boundary", () => {
    expect(formatBytes(1024 * 1024 * 1024)).toBe("1.0 GB");
  });

  it("handles very large values", () => {
    expect(formatBytes(1024 * 1024 * 1024 * 100)).toBe("100.0 GB");
  });

  it("rounds decimal places correctly", () => {
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(1587)).toBe("1.5 KB");
    expect(formatBytes(1600)).toBe("1.6 KB");
  });

  it("handles values just below boundaries", () => {
    expect(formatBytes(1023)).toBe("1023 B");
    expect(formatBytes(1024 * 1024 - 1)).toBe("1024.0 KB");
    expect(formatBytes(1024 * 1024 * 1024 - 1)).toBe("1024.0 MB");
  });
});
