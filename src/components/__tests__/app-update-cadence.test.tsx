import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppUpdate } from "../app-update";

const mocks = vi.hoisted(() => ({
  check: vi.fn(),
  relaunch: vi.fn(),
  invoke: vi.fn(),
  toast: Object.assign(vi.fn(), { error: vi.fn(), info: vi.fn(), success: vi.fn() }),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: mocks.relaunch }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("sonner", () => ({ toast: mocks.toast }));

const EIGHT_HOURS_MS = 8 * 60 * 60 * 1000;
const FOCUS_THROTTLE_MS = 30 * 60 * 1000;

describe("useAppUpdate check cadence", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    mocks.check.mockResolvedValue(null);
    mocks.invoke.mockResolvedValue(true);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("checks exactly once on mount", async () => {
    renderHook(() => useAppUpdate());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mocks.check).toHaveBeenCalledTimes(1);
  });

  it("checks again once the interval elapses", async () => {
    renderHook(() => useAppUpdate());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mocks.check).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(EIGHT_HOURS_MS);
    });

    expect(mocks.check).toHaveBeenCalledTimes(2);
  });

  it("checks on focus when the last check was long ago", async () => {
    renderHook(() => useAppUpdate());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mocks.check).toHaveBeenCalledTimes(1);

    // Past the 30-minute throttle floor, but well short of the 8h interval.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(FOCUS_THROTTLE_MS + 1000);
    });
    expect(mocks.check).toHaveBeenCalledTimes(1);

    await act(async () => {
      window.dispatchEvent(new Event("focus"));
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mocks.check).toHaveBeenCalledTimes(2);
  });

  it("does not check on focus when the last check was recent (throttle)", async () => {
    renderHook(() => useAppUpdate());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mocks.check).toHaveBeenCalledTimes(1);

    // Refocusing repeatedly within the throttle window must not fire ten checks.
    for (let i = 0; i < 10; i++) {
      await act(async () => {
        window.dispatchEvent(new Event("focus"));
        await vi.advanceTimersByTimeAsync(0);
      });
    }

    expect(mocks.check).toHaveBeenCalledTimes(1);
  });

  it("clears the interval and removes the focus listener on unmount", async () => {
    const { unmount } = renderHook(() => useAppUpdate());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mocks.check).toHaveBeenCalledTimes(1);

    unmount();

    await act(async () => {
      window.dispatchEvent(new Event("focus"));
      await vi.advanceTimersByTimeAsync(EIGHT_HOURS_MS * 2);
    });

    expect(mocks.check).toHaveBeenCalledTimes(1);
  });
});
