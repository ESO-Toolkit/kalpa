import { describe, it, expect, vi, beforeEach } from "vitest";

const mockGet = vi.fn();
const mockSet = vi.fn();
const mockSave = vi.fn();
const mockDelete = vi.fn();

vi.mock("@tauri-apps/plugin-store", () => ({
  load: vi.fn().mockResolvedValue({
    get: mockGet,
    set: mockSet,
    save: mockSave,
    delete: mockDelete,
  }),
}));

beforeEach(async () => {
  vi.resetModules();
  mockGet.mockReset();
  mockSet.mockReset();
  mockSave.mockReset();
  mockDelete.mockReset();

  const { load } = await import("@tauri-apps/plugin-store");
  vi.mocked(load).mockResolvedValue({
    get: mockGet,
    set: mockSet,
    save: mockSave,
    delete: mockDelete,
  } as never);
});

describe("getSetting", () => {
  it("returns stored value when it exists", async () => {
    const { getSetting } = await import("../store");
    mockGet.mockResolvedValue("stored-value");
    const result = await getSetting("theme", "default");
    expect(result).toBe("stored-value");
  });

  it("returns fallback when value is undefined", async () => {
    const { getSetting } = await import("../store");
    mockGet.mockResolvedValue(undefined);
    const result = await getSetting("missing", "fallback");
    expect(result).toBe("fallback");
  });

  it("returns fallback when store throws", async () => {
    const { load } = await import("@tauri-apps/plugin-store");
    vi.mocked(load).mockRejectedValue(new Error("store unavailable"));
    const { getSetting } = await import("../store");
    const result = await getSetting("key", "safe");
    expect(result).toBe("safe");
  });
});

describe("setSetting", () => {
  it("sets value in store and reports success", async () => {
    const { setSetting } = await import("../store");
    mockSet.mockResolvedValue(undefined);
    await expect(setSetting("theme", "dark")).resolves.toBe(true);
    expect(mockSet).toHaveBeenCalledWith("theme", "dark");
  });

  it("handles errors without throwing and reports failure", async () => {
    const { setSetting } = await import("../store");
    mockSet.mockRejectedValue(new Error("write error"));
    await expect(setSetting("key", "val")).resolves.toBe(false);
  });
});

describe("setSettings", () => {
  it("sets every key then saves once, reporting success", async () => {
    const { setSettings } = await import("../store");
    mockGet.mockResolvedValue(undefined);
    mockSet.mockResolvedValue(undefined);
    mockSave.mockResolvedValue(undefined);

    await expect(setSettings({ a: 1, b: "two" })).resolves.toBe(true);

    expect(mockSet).toHaveBeenCalledWith("a", 1);
    expect(mockSet).toHaveBeenCalledWith("b", "two");
    expect(mockSave).toHaveBeenCalledTimes(1);
  });

  it("rolls the cache back to its pre-batch snapshot when save fails", async () => {
    const { setSettings } = await import("../store");
    // Prior values: "active" existed, "marker" did not.
    mockGet.mockImplementation(async (key: string) => (key === "active" ? "old-theme" : undefined));
    mockSet.mockResolvedValue(undefined);
    mockDelete.mockResolvedValue(undefined);
    mockSave.mockRejectedValue(new Error("disk full"));

    await expect(setSettings({ marker: 1, active: "new-theme" })).resolves.toBe(false);

    // Rollback restores the prior value for an existing key and deletes one that
    // was absent, so a later autosave can't flush the half-written batch.
    expect(mockSet).toHaveBeenCalledWith("active", "old-theme");
    expect(mockDelete).toHaveBeenCalledWith("marker");
  });
});
