import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppUpdate } from "../app-update";

const mocks = vi.hoisted(() => ({
  check: vi.fn(),
  relaunch: vi.fn(),
  invoke: vi.fn(),
  openUrl: vi.fn(),
  toast: Object.assign(vi.fn(), { error: vi.fn(), info: vi.fn(), success: vi.fn() }),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: mocks.relaunch }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));
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

  it("waits for the platform probe before choosing the download path", async () => {
    let finishProbe: (supported: boolean) => void = () => {};
    mocks.invoke.mockReturnValue(
      new Promise<boolean>((resolve) => {
        finishProbe = resolve;
      })
    );
    const downloadAndInstall = vi.fn();
    mocks.check.mockResolvedValue({ version: "2.0", downloadAndInstall });
    const { result } = renderHook(() => useAppUpdate());

    await act(async () => {
      await result.current.checkForAppUpdate(false);
    });
    let download: Promise<void> = Promise.resolve();
    act(() => {
      download = result.current.downloadAndInstall();
    });
    expect(downloadAndInstall).not.toHaveBeenCalled();
    expect(mocks.openUrl).not.toHaveBeenCalled();

    await act(async () => {
      finishProbe(false);
      await download;
    });
    expect(mocks.openUrl).toHaveBeenCalledWith(
      "https://github.com/ESO-Toolkit/kalpa/releases/latest"
    );
    expect(downloadAndInstall).not.toHaveBeenCalled();
  });

  it("starts only one install when Update Now is clicked twice during the platform probe", async () => {
    let finishProbe: (supported: boolean) => void = () => {};
    mocks.invoke.mockReturnValue(
      new Promise<boolean>((resolve) => {
        finishProbe = resolve;
      })
    );
    const downloadAndInstall = vi.fn().mockResolvedValue(undefined);
    mocks.check.mockResolvedValue({ version: "2.0", downloadAndInstall });
    const { result } = renderHook(() => useAppUpdate());

    await act(async () => {
      await result.current.checkForAppUpdate(false);
    });
    let first: Promise<void> = Promise.resolve();
    let second: Promise<void> = Promise.resolve();
    act(() => {
      first = result.current.downloadAndInstall();
      second = result.current.downloadAndInstall();
    });

    await act(async () => {
      finishProbe(true);
      await Promise.all([first, second]);
    });

    expect(downloadAndInstall).toHaveBeenCalledTimes(1);
    expect(result.current.state.status).toBe("ready");
  });

  it("opens releases when the platform probe fails", async () => {
    mocks.invoke.mockRejectedValue(new Error("probe failed"));
    const downloadAndInstall = vi.fn();
    mocks.check.mockResolvedValue({ version: "2.0", downloadAndInstall });
    const { result } = renderHook(() => useAppUpdate());

    await act(async () => {
      await result.current.checkForAppUpdate(false);
    });
    await act(async () => {
      await result.current.downloadAndInstall();
    });

    expect(mocks.openUrl).toHaveBeenCalledTimes(1);
    expect(downloadAndInstall).not.toHaveBeenCalled();
  });
});
