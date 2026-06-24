import { describe, it, expect, vi, beforeEach } from "vitest";

const mockGet = vi.fn();
const mockSet = vi.fn();
const mockDelete = vi.fn();
/** Persistence now goes through the `flush_settings` Tauri command (atomic
 * write in settings_store.rs), not the plugin's non-atomic store.save(). */
const mockInvoke = vi.fn();

vi.mock("@tauri-apps/plugin-store", () => ({
  load: vi.fn().mockResolvedValue({
    get: mockGet,
    set: mockSet,
    delete: mockDelete,
  }),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

beforeEach(async () => {
  vi.resetModules();
  mockGet.mockReset();
  mockSet.mockReset();
  mockDelete.mockReset();
  mockInvoke.mockReset();
  mockInvoke.mockResolvedValue(undefined);

  const { load } = await import("@tauri-apps/plugin-store");
  vi.mocked(load).mockResolvedValue({
    get: mockGet,
    set: mockSet,
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

  it("loads the store with autoSave disabled and flushes atomically", async () => {
    const { setSetting } = await import("../store");
    mockSet.mockResolvedValue(undefined);
    await setSetting("theme", "dark");

    const { load } = await import("@tauri-apps/plugin-store");
    expect(load).toHaveBeenCalledWith(
      "settings.json",
      expect.objectContaining({ autoSave: false })
    );
    // Persistence is the atomic command, never the plugin's save().
    expect(mockInvoke).toHaveBeenCalledWith("flush_settings");
  });

  it("handles set errors without throwing and reports failure", async () => {
    const { setSetting } = await import("../store");
    mockSet.mockRejectedValue(new Error("write error"));
    await expect(setSetting("key", "val")).resolves.toBe(false);
  });

  it("reports failure when the atomic flush fails", async () => {
    const { setSetting } = await import("../store");
    mockSet.mockResolvedValue(undefined);
    mockInvoke.mockRejectedValue(new Error("disk full"));
    await expect(setSetting("key", "val")).resolves.toBe(false);
  });
});

describe("setSettings", () => {
  /** Back the mocked store with a real Map so set/get/delete stay consistent —
   * compare-and-restore reads back what the batch wrote. Optionally seed prior
   * values. */
  function backStore(seed: Record<string, unknown> = {}) {
    const backing = new Map<string, unknown>(Object.entries(seed));
    mockGet.mockImplementation(async (key: string) => backing.get(key));
    mockSet.mockImplementation(async (key: string, value: unknown) => {
      backing.set(key, value);
    });
    mockDelete.mockImplementation(async (key: string) => backing.delete(key));
    return backing;
  }

  it("sets every key then flushes once, reporting success", async () => {
    const backing = backStore();
    const { setSettings } = await import("../store");

    await expect(setSettings({ a: 1, b: "two" })).resolves.toBe(true);

    expect(backing.get("a")).toBe(1);
    expect(backing.get("b")).toBe("two");
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(mockInvoke).toHaveBeenCalledWith("flush_settings");
  });

  it("rolls the cache back to its pre-batch snapshot when the flush fails", async () => {
    // "active" existed before; "marker" did not.
    const backing = backStore({ active: "old-theme" });
    mockInvoke.mockRejectedValue(new Error("disk full"));
    const { setSettings } = await import("../store");

    await expect(setSettings({ marker: 1, active: "new-theme" })).resolves.toBe(false);

    // Restored exactly: the existing key reverts, the newly-added key is dropped,
    // so a later flush can't write the half-applied batch.
    expect(backing.get("active")).toBe("old-theme");
    expect(backing.has("marker")).toBe(false);
  });

  it("does not clobber a concurrent write when rolling back", async () => {
    const backing = backStore({ active: "old-theme" });
    // A concurrent writer lands a new value right as the batch tries to flush.
    mockInvoke.mockImplementation(async () => {
      backing.set("active", "user-choice");
      throw new Error("disk full");
    });
    const { setSettings } = await import("../store");

    await expect(setSettings({ active: "batch-default" })).resolves.toBe(false);

    // The key no longer holds the batch's attempted value, so the rollback leaves
    // the concurrent write intact rather than restoring "old-theme".
    expect(backing.get("active")).toBe("user-choice");
  });

  it("serializes writes so a concurrent setSetting can't interleave a batch", async () => {
    backStore();
    const order: string[] = [];
    mockSet.mockImplementation(async (k: string) => {
      order.push(`set:${k}`);
    });
    mockInvoke.mockImplementation(async () => {
      order.push("flush");
    });
    const { setSetting, setSettings } = await import("../store");

    // Fire a batch and a single write concurrently.
    await Promise.all([setSettings({ a: 1, b: 2 }), setSetting("c", 3)]);

    // The batch (set a, set b, flush) must fully complete before the single write
    // (set c, flush) begins — no interleaving.
    expect(order).toEqual(["set:a", "set:b", "flush", "set:c", "flush"]);
  });
});
