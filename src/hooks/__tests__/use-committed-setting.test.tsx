import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import { useCommittedSetting } from "../use-committed-setting";

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

describe("useCommittedSetting", () => {
  it("shows the new value immediately, before the write settles", async () => {
    const { save } = deferredSave<string>();
    const { result } = renderHook(() => useCommittedSetting<string>("skip", save));

    act(() => result.current.commit("ask"));

    expect(result.current.value).toBe("ask");
    expect(save).toHaveBeenCalledWith("ask");
  });

  it("keeps the value and stays quiet when the write succeeds", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useCommittedSetting<string>("skip", save));

    act(() => result.current.commit("ask"));
    await act(async () => settles[0]!(true));

    expect(result.current.value).toBe("ask");
    expect(mockToastError).not.toHaveBeenCalled();
  });

  it("rolls back to the stored value and says so when the write fails", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useCommittedSetting<string>("skip", save));

    act(() => result.current.commit("ask"));
    await act(async () => settles[0]!(false));

    expect(result.current.value).toBe("skip");
    await waitFor(() => expect(mockToastError).toHaveBeenCalledTimes(1));
  });

  // The round-3 review finding. Rolling back to a value captured at click time
  // lands on "ask" here — which never reached disk — leaving the UI claiming a
  // policy the install path will not use.
  it("never rolls back to a value that was never persisted", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useCommittedSetting<string>("skip", save));

    act(() => result.current.commit("ask"));
    act(() => result.current.commit("auto"));
    await act(async () => {
      settles[0]!(false);
      settles[1]!(false);
    });

    expect(result.current.value).toBe("skip");
  });

  it("lets a late failure be superseded by a newer selection", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useCommittedSetting<string>("skip", save));

    act(() => result.current.commit("ask"));
    act(() => result.current.commit("auto"));
    // The older write fails after the newer one already succeeded: the newer
    // selection owns the display and must not be dragged backwards.
    await act(async () => {
      settles[1]!(true);
      settles[0]!(false);
    });

    expect(result.current.value).toBe("auto");
    expect(mockToastError).not.toHaveBeenCalled();
  });

  it("rolls back to the newer committed value, not the original one", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useCommittedSetting<string>("skip", save));

    act(() => result.current.commit("ask"));
    await act(async () => settles[0]!(true));
    act(() => result.current.commit("auto"));
    await act(async () => settles[1]!(false));

    // "ask" is what the store actually holds now — not the "skip" it started at.
    expect(result.current.value).toBe("ask");
  });

  it("treats a load as already persisted, so a later failure rolls back to it", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useCommittedSetting<string>("skip", save));

    act(() => result.current.hydrate("auto"));
    expect(result.current.value).toBe("auto");

    act(() => result.current.commit("ask"));
    await act(async () => settles[0]!(false));

    expect(result.current.value).toBe("auto");
  });

  // The round-4 finding. The mount load is async, so a click can land while it
  // is still in flight; letting the stale read win puts the control back while
  // the click's write goes on to succeed — display behind the store, silently.
  it("drops a mount load that resolves after the user has already chosen", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useCommittedSetting<string>("skip", save));

    act(() => result.current.commit("auto"));
    act(() => result.current.hydrate("skip"));
    await act(async () => settles[0]!(true));

    expect(result.current.value).toBe("auto");
  });

  it("does not let a dropped load become the rollback target", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useCommittedSetting<string>("skip", save));

    act(() => result.current.commit("auto"));
    act(() => result.current.hydrate("ask"));
    await act(async () => settles[0]!(false));

    // Rolls back to "skip" — what the store held — not to the load that arrived
    // after the click and was discarded.
    expect(result.current.value).toBe("skip");
  });

  it("treats a rejected save as a failed one", async () => {
    const save = vi.fn(() => Promise.reject(new Error("store exploded")));
    const { result } = renderHook(() => useCommittedSetting<string>("skip", save));

    await act(async () => {
      result.current.commit("ask");
    });

    expect(result.current.value).toBe("skip");
    await waitFor(() => expect(mockToastError).toHaveBeenCalledTimes(1));
  });

  it("works for a boolean setting toggled twice in a row", async () => {
    const { save, settles } = deferredSave<boolean>();
    const { result } = renderHook(() => useCommittedSetting<boolean>(false, save));

    act(() => result.current.commit(true));
    act(() => result.current.commit(false));
    await act(async () => {
      settles[0]!(false);
      settles[1]!(false);
    });

    expect(result.current.value).toBe(false);
  });
});
