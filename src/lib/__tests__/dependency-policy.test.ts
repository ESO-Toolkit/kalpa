import { describe, it, expect, vi, beforeEach } from "vitest";

const mockGetSetting = vi.fn();
const mockSetSetting = vi.fn();
const mockSettled = vi.fn();

vi.mock("@/lib/store", () => ({
  getSetting: (...args: unknown[]) => mockGetSetting(...args),
  setSetting: (...args: unknown[]) => mockSetSetting(...args),
  settingsWritesSettled: () => mockSettled(),
}));

beforeEach(() => {
  mockGetSetting.mockReset();
  mockSetSetting.mockReset();
  mockSettled.mockReset();
  mockSetSetting.mockResolvedValue(true);
  mockSettled.mockResolvedValue(undefined);
});

/**
 * Both readers feed the install path, which is what decides whether a missing
 * required library is offered at all. Settings writes them fire-and-forget and
 * writes are queued, so a read that does not wait can hand Rust the policy the
 * user just replaced — and a stale "skip" means no prompt at all.
 */
describe("stale-read ordering", () => {
  it("waits for pending settings writes before reading the policy", async () => {
    const { getDependencyPolicy } = await import("../dependency-policy");
    const order: string[] = [];
    mockSettled.mockImplementation(() => {
      order.push("settled");
      return Promise.resolve();
    });
    mockGetSetting.mockImplementation(() => {
      order.push("read");
      return Promise.resolve("ask");
    });

    await getDependencyPolicy();

    expect(order).toEqual(["settled", "read"]);
  });

  // The round-5 finding: the settle wait is on the GLOBAL write chain, so an
  // unrelated wedged write must not be able to hang the install path — that is
  // the same "required library never offered" outcome, with no way out.
  it("reads anyway when a settings write never settles", async () => {
    vi.useFakeTimers();
    try {
      const { getDependencyPolicy } = await import("../dependency-policy");
      mockSettled.mockReturnValue(new Promise<void>(() => {}));
      mockGetSetting.mockResolvedValue("skip");

      const read = getDependencyPolicy();
      await vi.advanceTimersByTimeAsync(5000);

      await expect(read).resolves.toBe("skip");
    } finally {
      vi.useRealTimers();
    }
  });

  it("waits for pending settings writes before reading the required-only scope", async () => {
    const { getAskRequiredDependenciesOnly } = await import("../dependency-policy");
    const order: string[] = [];
    mockSettled.mockImplementation(() => {
      order.push("settled");
      return Promise.resolve();
    });
    mockGetSetting.mockImplementation(() => {
      order.push("read");
      return Promise.resolve(true);
    });

    await getAskRequiredDependenciesOnly();

    expect(order).toEqual(["settled", "read"]);
  });
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

describe("setDependencyPolicy", () => {
  // The settings radio reverts its optimistic selection when this resolves
  // false. If the signal were ever swallowed the radio would silently show a
  // policy that is not the stored one, and the install path reads the STORED
  // value — so "ask" on screen with "skip" on disk means a missing required
  // library is never offered at all.
  it("propagates a failed write so the radio can revert", async () => {
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
