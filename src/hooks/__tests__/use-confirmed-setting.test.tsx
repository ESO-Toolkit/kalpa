import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import { useConfirmedSetting } from "../use-confirmed-setting";

const mockToastError = vi.fn();
vi.mock("sonner", () => ({
  toast: { error: (...args: unknown[]) => mockToastError(...args) },
}));

beforeEach(() => {
  mockToastError.mockReset();
});

/** A save whose individual calls can be settled by hand, so a test can model
 *  two writes in flight and choose the order they resolve in. */
function deferredSave<T>() {
  const settles: Array<(ok: boolean) => void> = [];
  const save = vi.fn((_value: T) => new Promise<boolean>((resolve) => settles.push(resolve)));
  return { save, settles };
}

describe("useConfirmedSetting", () => {
  // The invariant the whole hook exists for: other code reads this preference
  // from storage and acts on it, so the control must never show a value the
  // store does not have.
  it("does not move until the write confirms", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useConfirmedSetting<string>("skip", save));

    act(() => result.current.commit("ask"));
    expect(result.current.value).toBe("skip");
    expect(save).toHaveBeenCalledWith("ask");

    await act(async () => settles[0]!(true));
    expect(result.current.value).toBe("ask");
  });

  it("stays put and says so when the write fails", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useConfirmedSetting<string>("skip", save));

    act(() => result.current.commit("ask"));
    await act(async () => settles[0]!(false));

    expect(result.current.value).toBe("skip");
    await waitFor(() => expect(mockToastError).toHaveBeenCalledTimes(1));
  });

  it("treats a rejected save as a failed one", async () => {
    const save = vi.fn(() => Promise.reject(new Error("store exploded")));
    const { result } = renderHook(() => useConfirmedSetting<string>("skip", save));

    await act(async () => {
      result.current.commit("ask");
    });

    expect(result.current.value).toBe("skip");
    await waitFor(() => expect(mockToastError).toHaveBeenCalledTimes(1));
  });

  // A write that never settles is the case no rollback scheme could handle:
  // here it simply leaves the control alone, which is the truth.
  it("leaves the control alone when a write never settles", async () => {
    const save = vi.fn(() => new Promise<boolean>(() => {}));
    const { result } = renderHook(() => useConfirmedSetting<string>("skip", save));

    act(() => result.current.commit("ask"));
    await act(async () => {});

    expect(result.current.value).toBe("skip");
    expect(mockToastError).not.toHaveBeenCalled();
  });

  it("ignores an older write that settles after a newer one", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useConfirmedSetting<string>("skip", save));

    act(() => result.current.commit("ask"));
    act(() => result.current.commit("auto"));
    await act(async () => {
      settles[1]!(true);
      settles[0]!(true);
    });

    // "ask" landing last must not paint over the newer "auto".
    expect(result.current.value).toBe("auto");
  });

  it("does not toast for an older failure the user has already moved past", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useConfirmedSetting<string>("skip", save));

    act(() => result.current.commit("ask"));
    act(() => result.current.commit("auto"));
    await act(async () => {
      settles[0]!(false);
      settles[1]!(true);
    });

    expect(result.current.value).toBe("auto");
    expect(mockToastError).not.toHaveBeenCalled();
  });

  it("shows the loaded value on mount", async () => {
    const { save } = deferredSave<string>();
    const { result } = renderHook(() => useConfirmedSetting<string>("ask", save));

    act(() => result.current.hydrate("skip"));

    expect(result.current.value).toBe("skip");
  });

  // The mount load is async, so it can arrive after a click. It read storage
  // before that click, so it is simply out of date.
  it("drops a mount load that arrives after the user has chosen", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useConfirmedSetting<string>("ask", save));

    act(() => result.current.commit("auto"));
    act(() => result.current.hydrate("skip"));
    await act(async () => settles[0]!(true));

    expect(result.current.value).toBe("auto");
  });

  it("works for a boolean setting", async () => {
    const { save, settles } = deferredSave<boolean>();
    const { result } = renderHook(() => useConfirmedSetting<boolean>(false, save));

    act(() => result.current.commit(true));
    await act(async () => settles[0]!(true));
    expect(result.current.value).toBe(true);

    act(() => result.current.commit(false));
    await act(async () => settles[1]!(false));
    expect(result.current.value).toBe(true);
  });
});
