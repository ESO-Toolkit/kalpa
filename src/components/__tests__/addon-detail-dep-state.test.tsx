import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AddonDetail } from "@/components/addon-detail";
import type { AddonManifest, UpdateCheckResult } from "@/types";

/**
 * Two ways this pane could disagree with itself about a dependency.
 *
 * Both are cross-flow: nothing rescans between an action here and the next
 * render, so anything the pane does to the AddOns folder it has to remember on
 * its own — and anything it starts has to not stamp on something already
 * running.
 */

const { invokeOrThrow } = vi.hoisted(() => ({ invokeOrThrow: vi.fn() }));

vi.mock("@/lib/tauri", () => ({
  getTauriErrorMessage: (error: unknown) => String(error),
  invokeOrThrow,
}));

vi.mock("@/lib/eso-running-context", () => ({
  useEnsureEsoNotBlocking: () => vi.fn().mockResolvedValue(true),
}));

vi.mock("@/lib/dependency-prompt-context", () => ({
  useResolvePendingDeps: () => vi.fn().mockResolvedValue(undefined),
}));

const library: AddonManifest = {
  folderName: "LibExample",
  title: "LibExample",
  author: "Kalpa QA",
  version: "1.0.0",
  addonVersion: 1,
  apiVersion: [101047],
  description: "Dependency fixture",
  isLibrary: true,
  dependsOn: [],
  optionalDependsOn: [],
  missingDependencies: [],
  outdatedDependencies: [],
  missingOptionalDependencies: [],
  esouiId: 2,
  tags: [],
  esouiLastUpdate: 0,
  installedAt: "2026-08-28T00:00:00.000Z",
  disabled: false,
  modifiedFileCount: 0,
};

const addon: AddonManifest = {
  ...library,
  folderName: "ExampleAddon",
  title: "Example Addon",
  isLibrary: false,
  dependsOn: [{ name: library.folderName, min_version: null }],
  esouiId: 1,
};

const updateAvailable: UpdateCheckResult = {
  folderName: addon.folderName,
  esouiId: addon.esouiId!,
  currentVersion: "1.0.0",
  remoteVersion: "1.1.0",
  downloadUrl: "https://example.invalid/addon.zip",
  hasUpdate: true,
  remoteLastUpdate: 0,
};

function renderDetail(overrides: Partial<React.ComponentProps<typeof AddonDetail>> = {}) {
  return render(
    <AddonDetail
      addon={addon}
      installedAddons={[addon, library]}
      addonsPath="C:\\test\\AddOns"
      onRefresh={vi.fn()}
      onRemoveAddon={vi.fn()}
      onToggleDisable={vi.fn()}
      updateResult={null}
      onAddonUpdated={vi.fn()}
      onTagsChange={vi.fn()}
      {...overrides}
    />
  );
}

describe("AddonDetail dependency rows", () => {
  it("stops showing a removed dependency as satisfied", () => {
    const onRemoveAddon = vi.fn();
    const onRefresh = vi.fn();
    renderDetail({ onRemoveAddon, onRefresh });

    // Satisfied to begin with: the backend says nothing is missing.
    expect(
      screen.getByRole("button", { name: `Remove ${library.folderName}` })
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Install" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: `Remove ${library.folderName}` }));
    expect(onRemoveAddon).toHaveBeenCalledWith(library.folderName);
    // NOT a rescan: removal is optimistic in App and the folder is still on disk
    // for the Undo window, so `onRefresh` would resurrect the row.
    expect(onRefresh).not.toHaveBeenCalled();

    // The row must stop claiming the dependency is there. It used to keep the
    // green tick from the selected addon's stale manifest while its own trash
    // button vanished with `removeTarget`.
    expect(screen.queryByRole("button", { name: `Remove ${library.folderName}` })).toBeNull();
    expect(screen.getByRole("button", { name: "Install" })).toBeInTheDocument();
  });

  it("does not let a dependency install start while an update is in flight", async () => {
    // One `operationIdRef` serves both flows: a dependency install started here
    // would overwrite the update's id and permanently disable its Stop button.
    invokeOrThrow.mockImplementation((command: string) => {
      if (command === "scan_update_conflicts") return new Promise(() => {});
      return Promise.resolve(undefined);
    });
    const missingDep: AddonManifest = { ...addon, missingDependencies: [library.folderName] };
    renderDetail({
      addon: missingDep,
      installedAddons: [missingDep],
      updateResult: updateAvailable,
    });

    const install = screen.getByRole("button", { name: "Install" });
    expect(install).not.toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Update" }));

    await waitFor(() => expect(screen.getByText("Updating…")).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "Install" })).toBeDisabled();

    // And the handler refuses too, for the click already queued when the update
    // started.
    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    expect(
      invokeOrThrow.mock.calls.filter(([command]) => command === "install_dependency")
    ).toHaveLength(0);
  });
});
