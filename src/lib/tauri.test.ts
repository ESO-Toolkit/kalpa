import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    success: vi.fn(),
  },
}));

import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { getTauriErrorMessage, invokeOrThrow, invokeResult, toastTauriError } from "./tauri";

const mockInvoke = vi.mocked(invoke);
const mockToastError = vi.mocked(toast.error);

describe("getTauriErrorMessage", () => {
  it.each([
    [
      "Zip extraction aborted: zip bomb detected",
      "The archive is too large or may be corrupt. Try re-downloading it.",
    ],
    [
      "zip archive contained no addon folders",
      "This file doesn't look like a valid ESO addon archive.",
    ],
    [
      "failed to open zip file: bad header",
      "Could not open the downloaded file. It may be corrupt or incomplete — try again.",
    ],
    [
      "failed to read zip archive: unexpected EOF",
      "The downloaded file is not a valid ZIP. It may be corrupt — try re-downloading.",
    ],
    [
      "AddOns folder not found at /tmp/eso",
      "Your AddOns folder could not be found. It may have been moved or the drive disconnected.",
    ],
    [
      "Could not reach ESOUI: dns failure",
      "ESOUI could not be reached. Check your internet connection and try again.",
    ],
    ["Too many requests to ESOUI", "ESOUI rate limit reached. Wait a moment and try again."],
    [
      "ESOUI is currently unavailable (503)",
      "ESOUI appears to be down. Try again in a few minutes.",
    ],
    [
      "addon not found on ESOUI",
      "This addon was not found on ESOUI — it may have been removed by its author.",
    ],
    [
      "Permission denied (os error 13)",
      "Permission denied — antivirus or another program may be blocking the file.",
    ],
    [
      "Access is denied",
      "Permission denied — antivirus or another program may be blocking the file.",
    ],
    [
      "io error: os error 5",
      "Permission denied — antivirus or another program may be blocking the file.",
    ],
  ])("maps %j to a friendly hint", (raw, expected) => {
    expect(getTauriErrorMessage(raw)).toBe(expected);
  });

  it("matches case-insensitively", () => {
    expect(getTauriErrorMessage("ZIP EXTRACTION ABORTED: ZIP BOMB")).toMatch(
      /archive is too large/i
    );
  });

  it("extracts message from Error instances", () => {
    expect(getTauriErrorMessage(new Error("addon not found on esoui"))).toBe(
      "This addon was not found on ESOUI — it may have been removed by its author."
    );
  });

  it("returns the raw message when no pattern matches a string error", () => {
    expect(getTauriErrorMessage("unexpected widget failure")).toBe("unexpected widget failure");
  });

  it("returns the raw message when no pattern matches an Error", () => {
    expect(getTauriErrorMessage(new Error("kaboom"))).toBe("kaboom");
  });

  it.each([
    ["empty string", ""],
    ["whitespace string", "   "],
    ["null", null],
    ["undefined", undefined],
    ["plain object", { foo: "bar" }],
    ["number", 42],
    ["empty Error", new Error("")],
  ])("returns generic fallback for %s", (_label, value) => {
    expect(getTauriErrorMessage(value)).toBe("Something went wrong");
  });
});

describe("invokeResult", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("returns ok with data on success", async () => {
    mockInvoke.mockResolvedValueOnce({ id: 1, name: "x" });
    const result = await invokeResult<{ id: number; name: string }>("get_thing", { id: 1 });
    expect(result).toEqual({ ok: true, data: { id: 1, name: "x" } });
    expect(mockInvoke).toHaveBeenCalledWith("get_thing", { id: 1 });
  });

  it("forwards args to invoke", async () => {
    mockInvoke.mockResolvedValueOnce(null);
    await invokeResult("noop");
    expect(mockInvoke).toHaveBeenCalledWith("noop", undefined);
  });

  it("returns ok=false with mapped error on failure", async () => {
    mockInvoke.mockRejectedValueOnce("Could not reach ESOUI");
    const result = await invokeResult("fetch_thing");
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.error).toMatch(/ESOUI could not be reached/);
    }
  });

  it("preserves unmapped raw error messages", async () => {
    mockInvoke.mockRejectedValueOnce(new Error("totally novel failure"));
    const result = await invokeResult("do_thing");
    expect(result).toEqual({ ok: false, error: "totally novel failure" });
  });

  it("logs raw error and mapped message when they differ", async () => {
    const errorSpy = vi.spyOn(console, "error");
    mockInvoke.mockRejectedValueOnce("addon not found on esoui");
    await invokeResult("install");
    const calls = errorSpy.mock.calls.map((c) => c.join(" "));
    expect(calls.some((c) => c.includes("[tauri:install]") && c.includes("shown to user"))).toBe(
      true
    );
  });

  it("logs raw error without 'shown to user' when message is unchanged", async () => {
    const errorSpy = vi.spyOn(console, "error");
    mockInvoke.mockRejectedValueOnce(new Error("plain unmapped"));
    await invokeResult("do_thing");
    const calls = errorSpy.mock.calls.map((c) => c.map(String).join(" "));
    expect(calls.some((c) => c.includes("[tauri:do_thing]"))).toBe(true);
    expect(calls.some((c) => c.includes("shown to user"))).toBe(false);
  });
});

describe("invokeOrThrow", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("returns data on success", async () => {
    mockInvoke.mockResolvedValueOnce("value");
    await expect(invokeOrThrow<string>("cmd")).resolves.toBe("value");
  });

  it("throws an Error containing the mapped message on failure", async () => {
    mockInvoke.mockRejectedValueOnce("addon not found on esoui");
    await expect(invokeOrThrow("cmd")).rejects.toThrow(/not found on ESOUI/);
  });
});

describe("toastTauriError", () => {
  beforeEach(() => {
    mockToastError.mockReset();
  });

  it("calls toast.error with action prefix and mapped message", () => {
    toastTauriError("Install failed", "Permission denied (os error 13)");
    expect(mockToastError).toHaveBeenCalledWith(
      "Install failed: Permission denied — antivirus or another program may be blocking the file."
    );
  });

  it("uses generic fallback for unknown error shapes", () => {
    toastTauriError("Update failed", { weird: true });
    expect(mockToastError).toHaveBeenCalledWith("Update failed: Something went wrong");
  });
});
