import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  ESO_RUNNING_SETTINGS_REFUSAL,
  esoIsRunningForSettingsWrite,
} from "@/lib/eso-settings-gate";
import { getSetting } from "@/lib/store";

/**
 * The bug these pin: `ensureEsoNotBlocking` returns true WITHOUT prompting when
 * `suppressEsoRunningWarning` is set, and two SavedVariables writers used it —
 * the pack settings import and the snapshot restore. Dismissing the addon
 * reminder, whose copy is about needing /reloadui, therefore became silent
 * permission to write SavedVariables under a running client, where ESO discards
 * them at the next loading screen while Kalpa reports success.
 *
 * `@/lib/store` is mocked rather than driven through `setSetting`. Its real
 * `getSetting` goes through `@tauri-apps/plugin-store`, which is not the mocked
 * IPC channel — under jsdom it throws and returns the fallback, so a test that
 * "set" the preference was silently setting nothing. A first version of this
 * file did exactly that and passed even when the preference WAS consulted,
 * which is the failure mode it exists to prevent.
 */
vi.mock("@/lib/store", () => ({
  getSetting: vi.fn(),
  setSetting: vi.fn(),
}));

describe("esoIsRunningForSettingsWrite", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(getSetting).mockReset();
    // Anything that reads a setting here is a regression: the whole point is
    // that no preference reaches this decision.
    vi.mocked(getSetting).mockResolvedValue(true as never);
  });

  it("reports ESO running even when the addon reminder is suppressed", async () => {
    vi.mocked(invoke).mockResolvedValue(true);

    expect(await esoIsRunningForSettingsWrite()).toBe(true);
    // Load-bearing: consulting the preference at all is the bug.
    expect(vi.mocked(getSetting)).not.toHaveBeenCalled();
  });

  it("reports not-running so a normal import proceeds", async () => {
    vi.mocked(invoke).mockResolvedValue(false);

    expect(await esoIsRunningForSettingsWrite()).toBe(false);
    expect(vi.mocked(getSetting)).not.toHaveBeenCalled();
  });

  it("asks the backend every call, so a caller can re-check at the write", async () => {
    // Sampling once and reusing the answer is the other half of the bug: the
    // pack import checked before an addon install that runs for minutes.
    vi.mocked(invoke).mockResolvedValueOnce(false).mockResolvedValueOnce(true);

    expect(await esoIsRunningForSettingsWrite()).toBe(false);
    expect(await esoIsRunningForSettingsWrite()).toBe(true);
    expect(vi.mocked(invoke)).toHaveBeenCalledTimes(2);
  });

  it("fails open when detection errors, matching every other caller", async () => {
    // Failing closed would make settings imports impossible wherever process
    // detection is broken. Flipping that direction is a decision for every
    // caller at once, not this one.
    vi.mocked(invoke).mockRejectedValue(new Error("detection unavailable"));

    expect(await esoIsRunningForSettingsWrite()).toBe(false);
  });

  it("names SavedVariables rather than promising a /reloadui", async () => {
    // The addon notice says changes load after /reloadui, which is true for
    // addon files and false here — reusing it was the misleading-message half
    // of this bug.
    expect(ESO_RUNNING_SETTINGS_REFUSAL).toMatch(/SavedVariables/);
    expect(ESO_RUNNING_SETTINGS_REFUSAL).not.toMatch(/reloadui/i);
  });
});
