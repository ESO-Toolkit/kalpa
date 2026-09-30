import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  info: vi.fn(),
  read: vi.fn(),
  write: vi.fn(),
}));
vi.mock("sonner", () => ({ toast: { info: mocks.info } }));
vi.mock("@/lib/store", () => ({ getSettingChecked: mocks.read, setSetting: mocks.write }));

const { notifyUnmatchedAddons } = await import("../auto-link");

describe("unmatched addon startup notices", () => {
  let settings: Map<string, unknown>;

  beforeEach(() => {
    vi.clearAllMocks();
    settings = new Map();
    mocks.read.mockImplementation(async (key: string, fallback: unknown) => ({
      ok: true,
      value: settings.get(key) ?? fallback,
    }));
    mocks.write.mockImplementation(async (key: string, value: unknown) => {
      settings.set(key, value);
      return true;
    });
  });

  it("explains unmatched folders with names and update limitations", async () => {
    await notifyUnmatchedAddons("AddOns", ["PrivateAddon", "CustomAddon"]);
    expect(mocks.info).toHaveBeenCalledWith("2 local addons have no ESOUI match", {
      description: expect.stringContaining("CustomAddon, PrivateAddon"),
      duration: 10000,
    });
    expect(mocks.info.mock.calls[0]?.[1].description).toContain(
      "Kalpa cannot check them for updates"
    );
  });

  it("persists notice history across launches, reordering, and removals", async () => {
    await notifyUnmatchedAddons("AddOns", ["A", "B", "A"]);
    await notifyUnmatchedAddons("AddOns", ["B", "A"]);
    await notifyUnmatchedAddons("AddOns", ["B"]);
    await notifyUnmatchedAddons("AddOns", []);
    await notifyUnmatchedAddons("AddOns", ["A", "B"]);
    expect(mocks.info).toHaveBeenCalledOnce();
    expect(mocks.write).toHaveBeenCalledOnce();
  });

  it("notifies a new folder even when the unmatched count is unchanged", async () => {
    await notifyUnmatchedAddons("AddOns", ["A", "B"]);
    await notifyUnmatchedAddons("AddOns", ["A", "C"]);
    expect(mocks.info).toHaveBeenLastCalledWith("1 local addon has no ESOUI match", {
      description: expect.stringMatching(/^C\./),
      duration: 10000,
    });
    expect([...settings.values()]).toEqual([["A", "B", "C"]]);
  });

  it("keeps notice history separate for different AddOns directories", async () => {
    await notifyUnmatchedAddons("live/AddOns", ["Custom"]);
    await notifyUnmatchedAddons("pts/AddOns", ["Custom"]);
    expect(mocks.info).toHaveBeenCalledTimes(2);
  });

  it("stays silent when all folders were linked or deliberately skipped", async () => {
    await notifyUnmatchedAddons("AddOns", []);
    expect(mocks.info).not.toHaveBeenCalled();
    expect(mocks.read).not.toHaveBeenCalled();
    expect(mocks.write).not.toHaveBeenCalled();
  });

  it("leaves notice history alone when settings cannot be read", async () => {
    mocks.read.mockResolvedValueOnce({ ok: false, value: [] });
    await notifyUnmatchedAddons("AddOns", ["Custom"]);
    expect(mocks.info).not.toHaveBeenCalled();
    expect(mocks.write).not.toHaveBeenCalled();
    await notifyUnmatchedAddons("AddOns", ["Custom"]);
    expect(mocks.info).toHaveBeenCalledOnce();
  });

  it("retries persistence on the next launch after a failed save", async () => {
    mocks.write.mockResolvedValueOnce(false);
    await notifyUnmatchedAddons("AddOns", ["Custom"]);
    await notifyUnmatchedAddons("AddOns", ["Custom"]);
    await notifyUnmatchedAddons("AddOns", ["Custom"]);
    expect(mocks.info).toHaveBeenCalledTimes(2);
    expect(mocks.write).toHaveBeenCalledTimes(2);
  });
});
