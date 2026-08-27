import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useOptimisticSetting } from "../use-optimistic-setting";

const mockToastError = vi.fn();
vi.mock("sonner", () => ({
  toast: { error: (...args: unknown[]) => mockToastError(...args) },
}));

function deferredSave<T>() {
  const settles: Array<(ok: boolean) => void> = [];
  const save = vi.fn((_value: T) => new Promise<boolean>((resolve) => settles.push(resolve)));
  return { save, settles };
}

beforeEach(() => mockToastError.mockReset());

describe("useOptimisticSetting", () => {
  it("does not let late hydration overwrite a user change", async () => {
    const { save, settles } = deferredSave<boolean>();
    const { result } = renderHook(() => useOptimisticSetting<boolean>(false, save));

    act(() => {
      void result.current.commit(true);
    });
    act(() => result.current.hydrate(false));
    expect(result.current.value).toBe(true);

    await act(async () => settles[0]!(true));
    expect(result.current.value).toBe(true);
  });

  it("rolls a failed pending change back to late hydrated storage", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useOptimisticSetting<string>("default", save));

    act(() => {
      void result.current.commit("requested");
    });
    act(() => result.current.hydrate("stored"));
    await act(async () => settles[0]!(false));

    expect(result.current.value).toBe("stored");
    await waitFor(() => expect(mockToastError).toHaveBeenCalledTimes(1));
  });

  it("ignores an older failure after a newer write succeeds", async () => {
    const { save, settles } = deferredSave<boolean>();
    const { result } = renderHook(() => useOptimisticSetting<boolean>(false, save));

    act(() => {
      void result.current.commit(true);
    });
    act(() => {
      void result.current.commit(false);
    });
    await act(async () => {
      settles[1]!(true);
      settles[0]!(false);
    });

    expect(result.current.value).toBe(false);
    expect(mockToastError).not.toHaveBeenCalled();
  });

  it("rolls a newer failure back to an earlier confirmed toggle", async () => {
    const { save, settles } = deferredSave<boolean>();
    const { result } = renderHook(() => useOptimisticSetting<boolean>(false, save));

    act(() => {
      void result.current.commit(true);
    });
    act(() => {
      void result.current.commit(false);
    });
    await act(async () => settles[0]!(true));
    await act(async () => settles[1]!(false));

    expect(result.current.value).toBe(true);
    await waitFor(() => expect(mockToastError).toHaveBeenCalledTimes(1));
  });

  it("rolls the latest failure back to the latest confirmed array", async () => {
    const { save, settles } = deferredSave<string[]>();
    const { result } = renderHook(() => useOptimisticSetting(["old"], save));

    act(() => {
      void result.current.commit((current) => [...current, "new"]);
    });
    act(() => {
      void result.current.commit((current) => current.filter((item) => item !== "old"));
    });
    await act(async () => settles[0]!(true));
    await act(async () => settles[1]!(false));

    expect(result.current.value).toEqual(["old", "new"]);
    await waitFor(() => expect(mockToastError).toHaveBeenCalledTimes(1));
  });

  it("submits functional array mutations composed before either settles", () => {
    const { save } = deferredSave<string[]>();
    const { result } = renderHook(() => useOptimisticSetting<string[]>(["old"], save));

    act(() => {
      void result.current.commit((current) => [...current, "new"]);
      void result.current.commit((current) => current.filter((item) => item !== "old"));
    });

    expect(save).toHaveBeenNthCalledWith(1, ["old", "new"]);
    expect(save).toHaveBeenNthCalledWith(2, ["new"]);
    expect(result.current.value).toEqual(["new"]);
  });
});
