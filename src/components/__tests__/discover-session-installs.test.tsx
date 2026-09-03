import { renderHook, act, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeOrThrow } = vi.hoisted(() => ({ invokeOrThrow: vi.fn() }));

vi.mock("@/lib/tauri", () => ({
  invokeOrThrow,
  getTauriErrorMessage: (e: unknown) => String(e),
}));

vi.mock("sonner", () => ({
  toast: Object.assign(vi.fn(), { success: vi.fn(), error: vi.fn(), info: vi.fn() }),
}));

vi.mock("@/lib/eso-running-context", () => ({
  useEnsureEsoNotBlocking: () => async () => true,
}));

vi.mock("@/lib/dependency-prompt-context", () => ({
  useResolvePendingDeps: () => vi.fn(),
}));

vi.mock("@/lib/dependency-policy", () => ({
  getDependencyPolicy: async () => "auto",
}));

vi.mock("@/lib/dependency-failure", () => ({
  reportDependencyFailures: vi.fn(),
}));

import { useAddonInstall } from "@/components/discover-panel";

const SKYSHARDS = 1710;
const ADDONS_PATH = "C:\\Games\\ESO\\live\\AddOns";

/** Drive the hook the way DiscoverPanel does: click Install, then let it settle. */
async function install(
  result: { current: ReturnType<typeof useAddonInstall> },
  id: number
): Promise<void> {
  await act(async () => {
    await result.current.install(id);
  });
}

beforeEach(() => {
  invokeOrThrow.mockReset();
  invokeOrThrow.mockImplementation((command: string) => {
    if (command === "resolve_esoui_addon") {
      return Promise.resolve({ downloadUrl: "https://esoui.test/x.zip", title: "Skyshards" });
    }
    if (command === "install_addon") {
      return Promise.resolve({
        installedFolders: ["Skyshards"],
        installedDeps: [],
        failedDeps: [],
        skippedDeps: [],
        pendingDeps: [],
      });
    }
    return Promise.resolve(undefined);
  });
});

describe("Discover session-install overlay", () => {
  it("badges an addon as installed before its rescan lands", async () => {
    const persisted = new Set<number>();
    const { result } = renderHook(() => useAddonInstall(ADDONS_PATH, vi.fn(), persisted));

    await install(result, SKYSHARDS);

    expect(result.current.installedIds.has(SKYSHARDS)).toBe(true);
  });

  it("stops badging it once a scan reports it uninstalled", async () => {
    // The shipped bug: install from Discover, uninstall, and the catalog row
    // kept its Installed badge until the tab was switched away, because the
    // overlay was only ever added to.
    let persisted = new Set<number>();
    const { result, rerender } = renderHook(
      ({ ids }: { ids: Set<number> }) => useAddonInstall(ADDONS_PATH, vi.fn(), ids),
      { initialProps: { ids: persisted } }
    );

    await install(result, SKYSHARDS);
    expect(result.current.installedIds.has(SKYSHARDS)).toBe(true);

    // The post-install scan lands and confirms it from disk.
    persisted = new Set([SKYSHARDS]);
    rerender({ ids: persisted });
    await waitFor(() => expect(result.current.installedIds.has(SKYSHARDS)).toBe(true));

    // The user uninstalls; the next scan reports it gone. The overlay must not
    // put the badge back — and must not revive because the freshly empty set
    // happens to match the one it was recorded against.
    persisted = new Set<number>();
    rerender({ ids: persisted });
    await waitFor(() => expect(result.current.installedIds.has(SKYSHARDS)).toBe(false));
  });

  it("does not badge an addon whose install failed", async () => {
    invokeOrThrow.mockImplementation((command: string) => {
      if (command === "resolve_esoui_addon") {
        return Promise.resolve({ downloadUrl: "https://esoui.test/x.zip", title: "Skyshards" });
      }
      return Promise.reject(new Error("download failed"));
    });

    const persisted = new Set<number>();
    const { result } = renderHook(() => useAddonInstall(ADDONS_PATH, vi.fn(), persisted));

    await install(result, SKYSHARDS);

    expect(result.current.installedIds.has(SKYSHARDS)).toBe(false);
  });

  it("passes the persisted set straight through when nothing is overlaid", () => {
    const persisted = new Set([SKYSHARDS]);
    const { result } = renderHook(() => useAddonInstall(ADDONS_PATH, vi.fn(), persisted));

    // Identity, so consumers' memos do not rerun for a no-op merge.
    expect(result.current.installedIds).toBe(persisted);
  });
});
