import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mockGet = vi.fn();
const mockSet = vi.fn();
const mockLoad = vi.fn();

vi.mock("@tauri-apps/plugin-store", () => ({
  load: (...args: unknown[]) => mockLoad(...args),
}));

describe("store", () => {
  beforeEach(() => {
    vi.resetModules();
    mockGet.mockReset();
    mockSet.mockReset();
    mockLoad.mockReset();
    mockLoad.mockResolvedValue({ get: mockGet, set: mockSet });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("getSetting", () => {
    it("returns the stored value when present", async () => {
      mockGet.mockResolvedValueOnce("stored");
      const { getSetting } = await import("./store");
      await expect(getSetting("theme", "light")).resolves.toBe("stored");
      expect(mockGet).toHaveBeenCalledWith("theme");
    });

    it("returns fallback when value is undefined", async () => {
      mockGet.mockResolvedValueOnce(undefined);
      const { getSetting } = await import("./store");
      await expect(getSetting("missing", "default")).resolves.toBe("default");
    });

    it("returns fallback when value is null", async () => {
      mockGet.mockResolvedValueOnce(null);
      const { getSetting } = await import("./store");
      await expect(getSetting("missing", 42)).resolves.toBe(42);
    });

    it("returns falsy stored value (not fallback) for 0/false/'' ", async () => {
      mockGet.mockResolvedValueOnce(0);
      const { getSetting } = await import("./store");
      await expect(getSetting("count", 99)).resolves.toBe(0);
    });

    it("returns fallback when load() throws", async () => {
      mockLoad.mockReset();
      mockLoad.mockRejectedValueOnce(new Error("disk full"));
      const { getSetting } = await import("./store");
      await expect(getSetting("k", "fallback")).resolves.toBe("fallback");
    });

    it("returns fallback when get() throws", async () => {
      mockGet.mockRejectedValueOnce(new Error("corrupt"));
      const { getSetting } = await import("./store");
      await expect(getSetting("k", "fallback")).resolves.toBe("fallback");
    });

    it("loads the store with autoSave enabled at settings.json", async () => {
      mockGet.mockResolvedValueOnce("v");
      const { getSetting } = await import("./store");
      await getSetting("k", "");
      expect(mockLoad).toHaveBeenCalledWith("settings.json", { autoSave: true, defaults: {} });
    });

    it("memoizes the store across multiple calls", async () => {
      mockGet.mockResolvedValue("v");
      const { getSetting } = await import("./store");
      await getSetting("a", "");
      await getSetting("b", "");
      await getSetting("c", "");
      expect(mockLoad).toHaveBeenCalledTimes(1);
    });
  });

  describe("setSetting", () => {
    it("writes the value through the store", async () => {
      mockSet.mockResolvedValueOnce(undefined);
      const { setSetting } = await import("./store");
      await setSetting("theme", "dark");
      expect(mockSet).toHaveBeenCalledWith("theme", "dark");
    });

    it("swallows errors silently", async () => {
      mockSet.mockRejectedValueOnce(new Error("io error"));
      const { setSetting } = await import("./store");
      await expect(setSetting("theme", "dark")).resolves.toBeUndefined();
    });

    it("swallows load() errors silently", async () => {
      mockLoad.mockReset();
      mockLoad.mockRejectedValueOnce(new Error("locked"));
      const { setSetting } = await import("./store");
      await expect(setSetting("theme", "dark")).resolves.toBeUndefined();
    });
  });
});
