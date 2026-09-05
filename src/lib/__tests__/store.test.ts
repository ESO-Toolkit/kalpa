import { describe, it, expect, vi, beforeEach } from "vitest";

const mockGet = vi.fn();
const mockSet = vi.fn();
const mockDelete = vi.fn();
/** Persistence now goes through the `flush_settings` Tauri command (atomic
 * write in settings_store.rs), not the plugin's non-atomic store.save(). */
const mockInvoke = vi.fn();

vi.mock("@tauri-apps/plugin-store", () => ({
  getStore: vi.fn().mockResolvedValue({
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

  const { getStore } = await import("@tauri-apps/plugin-store");
  vi.mocked(getStore).mockResolvedValue({
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
    const { getStore } = await import("@tauri-apps/plugin-store");
    vi.mocked(getStore).mockRejectedValue(new Error("store unavailable"));
    const { getSetting } = await import("../store");
    const result = await getSetting("key", "safe");
    expect(result).toBe("safe");
  });
});

describe("toolbarHidden persistence", () => {
  it("returns the [] fallback when the key was never written", async () => {
    const { getSetting } = await import("../store");
    mockGet.mockResolvedValue(undefined);
    const result = await getSetting("toolbarHidden", []);
    expect(result).toEqual([]);
  });

  it("round-trips a written array through setSetting/getSetting", async () => {
    const { setSetting, getSetting } = await import("../store");
    let stored: unknown;
    mockSet.mockImplementation(async (_key: string, value: unknown) => {
      stored = value;
    });
    mockGet.mockImplementation(async () => stored);

    await expect(setSetting("toolbarHidden", ["packs", "profiles"])).resolves.toBe(true);
    const result = await getSetting<string[]>("toolbarHidden", []);

    expect(result).toEqual(["packs", "profiles"]);
  });
});

describe("setSetting", () => {
  it("sets value in store and reports success", async () => {
    const { setSetting } = await import("../store");
    mockSet.mockResolvedValue(undefined);
    await expect(setSetting("theme", "dark")).resolves.toBe(true);
    expect(mockSet).toHaveBeenCalledWith("theme", "dark");
  });

  it("looks the store up rather than opening a path, and flushes atomically", async () => {
    const { setSetting } = await import("../store");
    mockSet.mockResolvedValue(undefined);
    await setSetting("theme", "dark");

    const plugin = await import("@tauri-apps/plugin-store");
    // `load` would let the webview name any path; the capability no longer
    // grants it. Only the lookup is used, and only for the one known file.
    expect(plugin.getStore).toHaveBeenCalledWith("settings.json");
    expect("load" in plugin && typeof plugin.load === "function").toBe(false);
    // Persistence is the atomic command, never the plugin's save().
    expect(mockInvoke).toHaveBeenCalledWith("flush_settings", {
      entries: { theme: "dark" },
    });
  });

  it("reports failure when native code never opened the store", async () => {
    const { getStore } = await import("@tauri-apps/plugin-store");
    vi.mocked(getStore).mockResolvedValue(null as never);
    const { getSetting, setSetting } = await import("../store");

    // A missing store must not silently look like an empty one: reads fall back
    // and writes report failure rather than reporting a save that never landed.
    await expect(getSetting("theme", "fallback")).resolves.toBe("fallback");
    await expect(setSetting("theme", "dark")).resolves.toBe(false);
    expect(mockInvoke).not.toHaveBeenCalledWith("flush_settings", expect.anything());
    // Setup is the only opener, so a failed open must at least be retried
    // through the no-argument reopen command before settings are given up on.
    expect(mockInvoke).toHaveBeenCalledWith("ensure_settings_store");
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
    mockGet.mockImplementation(async (key: string) => {
      const v = backing.get(key);
      // Simulate IPC: reads come back as fresh deserialized values (new object
      // refs), so callers can't rely on reference equality.
      return v === undefined ? undefined : JSON.parse(JSON.stringify(v));
    });
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
    expect(mockInvoke).toHaveBeenCalledWith("flush_settings", {
      entries: { a: 1, b: "two" },
    });
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

  it("rolls back object and array values structurally when the flush fails", async () => {
    // "list" existed before; "obj" is new. Reads return fresh refs (see backStore),
    // so a reference-equality guard would skip both — deep equality restores them.
    const backing = backStore({ list: [1, 2] });
    mockInvoke.mockRejectedValue(new Error("disk full"));
    const { setSettings } = await import("../store");

    await expect(setSettings({ obj: { a: 1 }, list: [3, 4] })).resolves.toBe(false);

    expect(backing.get("list")).toEqual([1, 2]); // existing key reverted
    expect(backing.has("obj")).toBe(false); // newly-added key dropped
  });

  it("skips rollback when the store was reloaded from disk", async () => {
    const backing = backStore({ existing: "disk" });
    // The flush command reloaded the store from disk (it had opened over a
    // transiently unreadable file) and signalled so via this error.
    mockInvoke.mockRejectedValue("settings-store-reloaded");
    const { setSettings } = await import("../store");

    await expect(setSettings({ existing: "attempted", fresh: 1 })).resolves.toBe(false);

    // Rollback was skipped (its snapshot would be stale): the newly-added key was
    // NOT deleted, unlike a normal flush failure.
    expect(backing.has("fresh")).toBe(true);
  });

  it("preserves the cache and reports success after a committed reload failure", async () => {
    const backing = backStore({ existing: "old" });
    mockInvoke.mockRejectedValue(
      "settings-store-committed: cache reload failed: transient read error"
    );
    const { setSettings } = await import("../store");

    await expect(setSettings({ existing: "new", fresh: 1 })).resolves.toBe(true);

    expect(backing.get("existing")).toBe("new");
    expect(backing.get("fresh")).toBe(1);
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
