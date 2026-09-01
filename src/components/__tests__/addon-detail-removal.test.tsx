import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AddonDetail } from "@/components/addon-detail";
import type { AddonManifest } from "@/types";

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

function renderDetail(onRemoveAddon: (folderName: string) => void, reason?: string) {
  return render(
    <AddonDetail
      addon={addon}
      installedAddons={[addon, library]}
      addonsPath="C:\\test\\AddOns"
      onRefresh={vi.fn()}
      onRemoveAddon={onRemoveAddon}
      removalBlockedReason={reason}
      onToggleDisable={vi.fn()}
      updateResult={null}
      onAddonUpdated={vi.fn()}
      onTagsChange={vi.fn()}
    />
  );
}

describe("AddonDetail dependency removal", () => {
  it("keeps dependency removal inert and exposes the reason while Update All runs", () => {
    const onRemoveAddon = vi.fn();
    const reason = "Wait for the current update batch to finish before removing addons.";
    renderDetail(onRemoveAddon, reason);

    const explanation = screen.getByLabelText(
      `Remove ${library.folderName} unavailable. ${reason}`
    );
    expect(explanation).toHaveAttribute("tabindex", "0");
    const removeDependency = screen.getByRole("button", {
      name: `Remove ${library.folderName}`,
    });
    expect(removeDependency).toBeDisabled();
    fireEvent.click(removeDependency);

    expect(onRemoveAddon).not.toHaveBeenCalled();
    expect(invokeOrThrow).not.toHaveBeenCalledWith("remove_addon", expect.anything());
  });

  it("routes enabled dependency removal through the App callback", () => {
    const onRemoveAddon = vi.fn();
    renderDetail(onRemoveAddon);

    fireEvent.click(
      screen.getByRole("button", {
        name: `Remove ${library.folderName}`,
      })
    );

    expect(onRemoveAddon).toHaveBeenCalledWith(library.folderName);
    expect(invokeOrThrow).not.toHaveBeenCalledWith("remove_addon", expect.anything());
  });
});
