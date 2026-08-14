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

  it("reports a failed write so the checkbox can stay put", async () => {
    const { setAskRequiredDependenciesOnly } = await import("../dependency-policy");
    mockSetSetting.mockResolvedValue(false);
    await expect(setAskRequiredDependenciesOnly(true)).resolves.toBe(false);
  });
});

describe("setDependencyPolicy", () => {
  // The settings radio only moves once this reports success. If the signal were
  // ever swallowed the radio would show a policy that is not the stored one,
  // and the install path reads the STORED value — so "ask" on screen with
  // "skip" on disk means a required library is never offered at all.
  it("reports a failed write so the radio does not move", async () => {
    const { setDependencyPolicy } = await import("../dependency-policy");
    mockSetSetting.mockResolvedValue(false);
    await expect(setDependencyPolicy("ask")).resolves.toBe(false);
  });

  it("reports success so a good write keeps the selection", async () => {
    const { setDependencyPolicy, DEPENDENCY_POLICY_KEY } = await import("../dependency-policy");
    await expect(setDependencyPolicy("ask")).resolves.toBe(true);
    expect(mockSetSetting).toHaveBeenCalledWith(DEPENDENCY_POLICY_KEY, "ask");
  });
});
