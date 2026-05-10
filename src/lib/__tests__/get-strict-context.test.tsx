import { describe, it, expect } from "vitest";
import { renderHook } from "@testing-library/react";
import React from "react";
import { getStrictContext } from "../get-strict-context";

describe("getStrictContext", () => {
  it("provides and consumes context value", () => {
    const [Provider, useCtx] = getStrictContext<{ count: number }>("TestContext");

    const wrapper = ({ children }: { children?: React.ReactNode }) => (
      <Provider value={{ count: 42 }}>{children}</Provider>
    );

    const { result } = renderHook(() => useCtx(), { wrapper });
    expect(result.current.count).toBe(42);
  });

  it("throws when used outside provider", () => {
    const [, useCtx] = getStrictContext<string>("MissingProvider");

    expect(() => {
      renderHook(() => useCtx());
    }).toThrow("useContext must be used within MissingProvider");
  });

  it("uses default error message when no name provided", () => {
    const [, useCtx] = getStrictContext<string>();

    expect(() => {
      renderHook(() => useCtx());
    }).toThrow("useContext must be used within a Provider");
  });
});
