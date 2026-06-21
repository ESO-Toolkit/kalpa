import { describe, it, expect, vi, beforeEach } from "vitest";

const mockGet = vi.fn();
const mockSet = vi.fn();

vi.mock("@tauri-apps/plugin-store", () => ({
  load: vi.fn().mockResolvedValue({
    get: mockGet,
    set: mockSet,
  }),
}));

beforeEach(async () => {
  vi.resetModules();
  mockGet.mockReset();
  mockSet.mockReset();

  const { load } = await import("@tauri-apps/plugin-store");
  vi.mocked(load).mockResolvedValue({
    get: mockGet,
    set: mockSet,
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
