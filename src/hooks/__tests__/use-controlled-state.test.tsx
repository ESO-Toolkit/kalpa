import { describe, it, expect, vi } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useControlledState } from "../use-controlled-state";

describe("useControlledState", () => {
  it("uses defaultValue when uncontrolled", () => {
    const { result } = renderHook(() => useControlledState({ defaultValue: "initial" }));
    expect(result.current[0]).toBe("initial");
  });

  it("uses controlled value over defaultValue", () => {
    const { result } = renderHook(() =>
      useControlledState({ value: "controlled", defaultValue: "default" })
    );
    expect(result.current[0]).toBe("controlled");
  });

  it("allows setState in uncontrolled mode", () => {
    const { result } = renderHook(() => useControlledState({ defaultValue: "initial" }));
    act(() => {
      result.current[1]("updated");
    });
    expect(result.current[0]).toBe("updated");
  });

  it("calls onChange when setState is called", () => {
    const onChange = vi.fn();
    const { result } = renderHook(() => useControlledState({ defaultValue: "initial", onChange }));
    act(() => {
      result.current[1]("updated");
    });
    expect(onChange).toHaveBeenCalledWith("updated");
  });

  it("syncs with external value changes in controlled mode", () => {
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useControlledState({ value }),
      { initialProps: { value: "first" } }
    );
    expect(result.current[0]).toBe("first");
    rerender({ value: "second" });
    expect(result.current[0]).toBe("second");
  });

  it("works with numeric types", () => {
    const { result } = renderHook(() => useControlledState({ defaultValue: 0 }));
    expect(result.current[0]).toBe(0);
    act(() => {
      result.current[1](42);
    });
    expect(result.current[0]).toBe(42);
  });

  it("passes extra args to onChange", () => {
    const onChange = vi.fn();
    const { result } = renderHook(() =>
      useControlledState<string, [string]>({ defaultValue: "init", onChange })
    );
    act(() => {
      result.current[1]("val", "extra");
    });
    expect(onChange).toHaveBeenCalledWith("val", "extra");
  });
});
