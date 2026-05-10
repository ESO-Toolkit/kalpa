import { describe, it, expect, vi, beforeEach } from "vitest";
import { getTauriErrorMessage, invokeResult, invokeOrThrow } from "../tauri";
import { invoke } from "@tauri-apps/api/core";

vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));

describe("getTauriErrorMessage", () => {
  it("returns friendly message for zip bomb error", () => {
    expect(getTauriErrorMessage("zip extraction aborted: zip bomb detected")).toBe(
      "The archive is too large or may be corrupt. Try re-downloading it."
    );
  });

  it("returns friendly message for no addon folders", () => {
    expect(getTauriErrorMessage("zip archive contained no addon folders")).toBe(
      "This file doesn't look like a valid ESO addon archive."
    );
  });

  it("returns friendly message for permission denied", () => {
    expect(getTauriErrorMessage("Permission denied (os error 13)")).toBe(
      "Permission denied — antivirus or another program may be blocking the file."
    );
  });

  it("returns friendly message for access denied (os error 5)", () => {
    expect(getTauriErrorMessage("Access is denied")).toBe(
      "Permission denied — antivirus or another program may be blocking the file."
    );
  });

  it("returns friendly message for ESOUI rate limit", () => {
    expect(getTauriErrorMessage("Too many requests to ESOUI")).toBe(
      "ESOUI rate limit reached. Wait a moment and try again."
    );
  });

  it("returns friendly message for ESOUI unreachable", () => {
    expect(getTauriErrorMessage("Could not reach ESOUI")).toBe(
      "ESOUI could not be reached. Check your internet connection and try again."
    );
  });

  it("returns friendly message for ESOUI unavailable", () => {
    expect(getTauriErrorMessage("ESOUI is currently unavailable")).toBe(
      "ESOUI appears to be down. Try again in a few minutes."
    );
  });

  it("returns friendly message for addon not found", () => {
    expect(getTauriErrorMessage("addon not found on ESOUI")).toBe(
      "This addon was not found on ESOUI — it may have been removed by its author."
    );
  });

  it("returns raw message for unrecognized errors", () => {
    expect(getTauriErrorMessage("some random error")).toBe("some random error");
  });

  it("returns fallback for non-string, non-Error values", () => {
    expect(getTauriErrorMessage(undefined)).toBe("Something went wrong");
    expect(getTauriErrorMessage(null)).toBe("Something went wrong");
    expect(getTauriErrorMessage(42)).toBe("Something went wrong");
  });

  it("extracts message from Error objects", () => {
    expect(getTauriErrorMessage(new Error("failed to open zip file"))).toBe(
      "Could not open the downloaded file. It may be corrupt or incomplete — try again."
    );
  });
});

describe("invokeResult", () => {
  const mockInvoke = vi.mocked(invoke);

  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("returns ok result on success", async () => {
    mockInvoke.mockResolvedValue({ data: "test" });
    const result = await invokeResult("test_command", { arg: "val" });
    expect(result).toEqual({ ok: true, data: { data: "test" } });
    expect(mockInvoke).toHaveBeenCalledWith("test_command", { arg: "val" });
  });

  it("returns error result on failure", async () => {
    mockInvoke.mockRejectedValue(new Error("backend error"));
    const result = await invokeResult("test_command");
    expect(result).toEqual({ ok: false, error: "backend error" });
  });

  it("maps known errors to friendly messages", async () => {
    mockInvoke.mockRejectedValue(new Error("zip extraction aborted: zip bomb"));
    const result = await invokeResult("test_command");
    expect(result).toEqual({
      ok: false,
      error: "The archive is too large or may be corrupt. Try re-downloading it.",
    });
  });
});

describe("invokeOrThrow", () => {
  const mockInvoke = vi.mocked(invoke);

  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("returns data on success", async () => {
    mockInvoke.mockResolvedValue(42);
    const result = await invokeOrThrow<number>("test_command");
    expect(result).toBe(42);
  });

  it("throws on failure", async () => {
    mockInvoke.mockRejectedValue(new Error("backend error"));
    await expect(invokeOrThrow("test_command")).rejects.toThrow("backend error");
  });
});
