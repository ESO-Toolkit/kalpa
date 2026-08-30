import { describe, expect, it } from "vitest";
import { rememberBounded } from "../src/bounded-map";

describe("rememberBounded", () => {
  it("evicts only the oldest entry when full", () => {
    const memo = new Map<string, boolean>([["oldest", true], ["middle", false], ["newest", true]]);

    rememberBounded(memo, "incoming", false, 3);

    expect([...memo.entries()]).toEqual([
      ["middle", false],
      ["newest", true],
      ["incoming", false],
    ]);
  });

  it("refreshes an updated entry instead of evicting another key", () => {
    const memo = new Map<string, number>([["a", 1], ["b", 2]]);

    rememberBounded(memo, "a", 3, 2);

    expect([...memo.entries()]).toEqual([["b", 2], ["a", 3]]);
  });
});
