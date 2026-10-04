import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useLatestRequest } from "../use-latest-request";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("useLatestRequest", () => {
  it("applies only the newest refresh when responses resolve out of order", async () => {
    const first = deferred<string[]>();
    const second = deferred<string[]>();
    const applied: string[][] = [];
    const settled = vi.fn();
    const { result } = renderHook(() => useLatestRequest());

    act(() => {
      void result.current(() => first.promise, {
        onSuccess: (value) => applied.push(value),
        onSettled: settled,
      });
      void result.current(() => second.promise, {
        onSuccess: (value) => applied.push(value),
        onSettled: settled,
      });
    });

    await act(async () => second.resolve(["new"]));
    await act(async () => first.resolve(["stale"]));

    expect(applied).toEqual([["new"]]);
    expect(settled).toHaveBeenCalledTimes(1);
  });

  it("suppresses stale errors", async () => {
    const first = deferred<string[]>();
    const second = deferred<string[]>();
    const onError = vi.fn();
    const { result } = renderHook(() => useLatestRequest());

    act(() => {
      void result.current(() => first.promise, { onError });
      void result.current(() => second.promise, { onError });
    });

    await act(async () => second.resolve(["new"]));
    await act(async () => first.reject(new Error("stale")));

    expect(onError).not.toHaveBeenCalled();
  });

  it("invalidates every handler when the owner unmounts", async () => {
    const pending = deferred<string[]>();
    const handlers = {
      onSuccess: vi.fn(),
      onError: vi.fn(),
      onSettled: vi.fn(),
    };
    const { result, unmount } = renderHook(() => useLatestRequest());

    act(() => {
      void result.current(() => pending.promise, handlers);
    });
    unmount();
    await act(async () => pending.resolve(["late"]));

    expect(handlers.onSuccess).not.toHaveBeenCalled();
    expect(handlers.onError).not.toHaveBeenCalled();
    expect(handlers.onSettled).not.toHaveBeenCalled();
  });

  it("suppresses rejection and settlement after unmount", async () => {
    const pending = deferred<string[]>();
    const handlers = {
      onError: vi.fn(),
      onSettled: vi.fn(),
    };
    const { result, unmount } = renderHook(() => useLatestRequest());

    act(() => {
      void result.current(() => pending.promise, handlers);
    });
    unmount();
    await act(async () => pending.reject(new Error("late failure")));

    expect(handlers.onError).not.toHaveBeenCalled();
    expect(handlers.onSettled).not.toHaveBeenCalled();
  });
});
