import { describe, it, expect, vi, beforeEach } from "vitest";

const mockGetSetting = vi.fn();
const mockSetSetting = vi.fn();

vi.mock("@/lib/store", () => ({
  getSetting: (...args: unknown[]) => mockGetSetting(...args),
  setSetting: (...args: unknown[]) => mockSetSetting(...args),
}));

beforeEach(() => {
  mockGetSetting.mockReset();
  mockSetSetting.mockReset();
  mockSetSetting.mockResolvedValue(true);
});

describe("getAskRequiredDependenciesOnly", () => {
  it("defaults to off when unset, so the prompt keeps listing both groups", async () => {
    const { getAskRequiredDependenciesOnly } = await import("../dependency-policy");
    mockGetSetting.mockResolvedValue(undefined);
    await expect(getAskRequiredDependenciesOnly()).resolves.toBe(false);
  });

  it("returns the stored boolean", async () => {
    const { getAskRequiredDependenciesOnly } = await import("../dependency-policy");
    mockGetSetting.mockResolvedValue(true);
    await expect(getAskRequiredDependenciesOnly()).resolves.toBe(true);
  });

  // settings.json is user-editable and survives downgrades, so the stored value
  // is untrusted. A truthy non-boolean must NOT read as "on": that would
  // silently suppress optional libraries the user never opted out of.
  it.each([["yes"], [1], [{}], [null], [[]]])("falls back to the default for %s", async (raw) => {
    const { getAskRequiredDependenciesOnly } = await import("../dependency-policy");
    mockGetSetting.mockResolvedValue(raw);
    await expect(getAskRequiredDependenciesOnly()).resolves.toBe(false);
  });
});

describe("setAskRequiredDependenciesOnly", () => {
  it("writes under the documented key", async () => {
    const { setAskRequiredDependenciesOnly, ASK_REQUIRED_ONLY_KEY } =
      await import("../dependency-policy");
    await setAskRequiredDependenciesOnly(true);
    expect(mockSetSetting).toHaveBeenCalledWith(ASK_REQUIRED_ONLY_KEY, true);
  });

  it("propagates a failed write so the caller can revert its toggle", async () => {
    const { setAskRequiredDependenciesOnly } = await import("../dependency-policy");
    mockSetSetting.mockResolvedValue(false);
    await expect(setAskRequiredDependenciesOnly(true)).resolves.toBe(false);
  });
});
