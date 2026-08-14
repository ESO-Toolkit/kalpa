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

  // Writes complete in submission order — `setSetting` serializes them on a
  // shared chain (`enqueueWrite` in lib/store.ts), so the later click's value
  // is also the later one in storage. The hook relies on that: with genuinely
  // out-of-order completions, "the last success wins" is the only rule it could
  // apply, and it would be wrong. An earlier revision seq-gated successes to
  // guard the reordered case, which cost far more than it bought — it let a
  // never-settling newer write hide a landed older one indefinitely.
  it("ends on the later click's value when writes land in order", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useConfirmedSetting<string>("skip", save));

    act(() => result.current.commit("ask"));
    act(() => result.current.commit("auto"));
    await act(async () => {
      settles[0]!(true);
      settles[1]!(true);
    });

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

  // The round-7 finding, and it needs no out-of-order completion: an earlier
  // click lands while a later one fails, so the store moved even though the
  // user's final choice did not take.
  it("shows the earlier write that landed when the later one fails", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useConfirmedSetting<string>("ask", save));

    act(() => result.current.commit("skip"));
    act(() => result.current.commit("ask"));
    await act(async () => {
      settles[0]!(true);
      settles[1]!(false);
    });

    // Storage holds "skip" — the control must say so rather than keep showing
    // the "ask" the user asked for and did not get.
    expect(result.current.value).toBe("skip");
    await waitFor(() => expect(mockToastError).toHaveBeenCalledTimes(1));
  });

  // Inverted from an earlier revision, which held a superseded success back
  // until the newer write settled. Round 8: the newer write may never settle,
  // and then the control shows a value storage does not have, forever.
  it("shows a superseded success immediately, then the newer one", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useConfirmedSetting<string>("ask", save));

    act(() => result.current.commit("skip"));
    act(() => result.current.commit("auto"));

    // "skip" is in storage the moment it lands, so that is what to show.
    await act(async () => settles[0]!(true));
    expect(result.current.value).toBe("skip");

    await act(async () => settles[1]!(true));
    expect(result.current.value).toBe("auto");
  });

  it("keeps showing a landed write when the newer one never settles", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useConfirmedSetting<string>("ask", save));

    act(() => result.current.commit("skip"));
    act(() => result.current.commit("ask"));
    await act(async () => settles[0]!(true));
    // settles[1] is never called: the second write hangs forever.
    await act(async () => {});

    // Storage holds "skip". Showing "ask" here would be the silent-suppression
    // case with no toast and no way for the user to notice.
    expect(result.current.value).toBe("skip");
  });

  // The round-5 case, re-checked under the new model: a click before the mount
  // load lands, then a failed write. The control must not fall back to the
  // constructor default when storage says otherwise.
  it("falls back to a late load rather than the default it started on", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useConfirmedSetting<string>("ask", save));

    act(() => result.current.commit("auto"));
    act(() => result.current.hydrate("skip"));
    await act(async () => settles[0]!(false));

    expect(result.current.value).toBe("skip");
  });

  it("does not let a late load overrule a write that already landed", async () => {
    const { save, settles } = deferredSave<string>();
    const { result } = renderHook(() => useConfirmedSetting<string>("ask", save));

    act(() => result.current.commit("auto"));
    await act(async () => settles[0]!(true));
    // The mount load finally arrives, holding what storage had BEFORE that write.
    act(() => result.current.hydrate("skip"));
    act(() => result.current.commit("ask"));
    await act(async () => settles[1]!(false));

    expect(result.current.value).toBe("auto");
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
