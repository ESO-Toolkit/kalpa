import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AddonDetail } from "@/components/addon-detail";
import type { AddonManifest, DependencyVersionMismatch, UpdateCheckResult } from "@/types";

/**
 * Two ways this pane could disagree with itself about a dependency.
 *
 * Both are cross-flow: nothing rescans between an action here and the next
 * render, so anything the pane does to the AddOns folder it has to remember on
 * its own — and anything it starts has to not stamp on something already
 * running.
 */

const { invokeOrThrow, openUrl, warningToast, successToast } = vi.hoisted(() => ({
  invokeOrThrow: vi.fn(),
  openUrl: vi.fn(),
  warningToast: vi.fn(),
  successToast: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("sonner", () => ({
  toast: { warning: warningToast, success: successToast, error: vi.fn(), info: vi.fn() },
}));

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
  beforeEach(() => {
    invokeOrThrow.mockReset();
    vi.clearAllMocks();
  });
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

  const mismatch: DependencyVersionMismatch = {
    name: library.folderName,
    minVersion: 210,
    downloadedVersion: 2,
    installedVersion: null,
    esouiId: 4367,
    archiveSha256: "download-digest",
    canInstall: true,
    blockedReason: null,
  };
  const result = {
    installedFolders: [library.folderName],
    installedDeps: [],
    failedDeps: [],
    skippedDeps: [],
    pendingDeps: [],
  };
  const missing = {
    ...addon,
    dependsOn: [{ name: library.folderName, min_version: 210 }],
    missingDependencies: [library.folderName],
  };

  it("requires an explicit choice, shows ESOUI, and keeps recovery visibly outdated without another update", async () => {
    invokeOrThrow
      .mockResolvedValueOnce({ status: "versionMismatch", mismatch })
      .mockResolvedValueOnce({ status: "installed", result, versionMismatch: mismatch });
    const onRefresh = vi.fn();
    renderDetail({ addon: missing, installedAddons: [missing], onRefresh });
    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    await screen.findByRole("button", { name: "Install published version" });
    expect(onRefresh).not.toHaveBeenCalled();
    expect(successToast).not.toHaveBeenCalled();
    expect(screen.getByText(/ESO cannot load Example Addon/)).toBeInTheDocument();
    fireEvent.click(
      within(screen.getByRole("alert")).getByRole("button", {
        name: /View on ESOUI/,
      })
    );
    expect(openUrl).toHaveBeenCalledWith("https://www.esoui.com/downloads/info4367.html");
    fireEvent.click(screen.getByRole("button", { name: "Install published version" }));
    await waitFor(() => expect(onRefresh).toHaveBeenCalledOnce());
    expect(invokeOrThrow).toHaveBeenNthCalledWith(
      2,
      "install_dependency",
      expect.objectContaining({ confirmation: mismatch })
    );
    expect(warningToast).toHaveBeenCalledWith(`Installed published ${library.folderName}`, {
      description: "Version 2 is still below the required 210.",
    });
    expect(screen.getByText("v210+ (outdated)")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: `Remove ${library.folderName}` })
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Update" })).toBeNull();
    expect(screen.queryByText("Updated")).toBeNull();
    expect(screen.queryByRole("button", { name: "Install published version" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: `Remove ${library.folderName}` }));
    expect(screen.queryByText(/The published LibExample/)).toBeNull();
    expect(screen.getByRole("button", { name: "Install" })).toBeInTheDocument();
  });

  it("presents a changed download for a fresh choice rather than silently installing it", async () => {
    const changed = { ...mismatch, downloadedVersion: 3, archiveSha256: "changed-digest" };
    invokeOrThrow
      .mockResolvedValueOnce({ status: "versionMismatch", mismatch })
      .mockResolvedValueOnce({ status: "versionMismatch", mismatch: changed });
    const onRefresh = vi.fn();
    renderDetail({ addon: missing, installedAddons: [missing], onRefresh });
    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    fireEvent.click(await screen.findByRole("button", { name: "Install published version" }));
    await screen.findByText(/has version 3, below the required 210/);
    expect(onRefresh).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: `Remove ${library.folderName}` })).toBeNull();
  });

  it("shows blocked recovery reasons and avoids claiming an optional dependency blocks its parent", async () => {
    const blocked = {
      ...mismatch,
      canInstall: false,
      blockedReason: "Bundled LibOther would replace a newer installed version.",
    };
    invokeOrThrow.mockResolvedValueOnce({ status: "versionMismatch", mismatch: blocked });
    const optional = {
      ...addon,
      dependsOn: [],
      optionalDependsOn: missing.dependsOn,
      missingOptionalDependencies: [library.folderName],
    };
    renderDetail({ addon: optional, installedAddons: [optional] });
    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    await screen.findByText(blocked.blockedReason);
    expect(screen.getByText(/The addon can still load without it/)).toBeInTheDocument();
    expect(screen.queryByText(/ESO cannot load/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Install published version" })).toBeNull();
    expect(screen.getByText("v210+ (outdated)")).toBeInTheDocument();
  });

  it("clears the mismatch when checking again finds a compatible published version", async () => {
    invokeOrThrow
      .mockResolvedValueOnce({ status: "versionMismatch", mismatch })
      .mockResolvedValueOnce({ status: "installed", result, versionMismatch: null });
    const onRefresh = vi.fn();
    renderDetail({ addon: missing, installedAddons: [missing], onRefresh });
    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    fireEvent.click(await screen.findByRole("button", { name: "Check again" }));
    await waitFor(() => expect(onRefresh).toHaveBeenCalledOnce());
    expect(invokeOrThrow).toHaveBeenNthCalledWith(
      2,
      "install_dependency",
      expect.objectContaining({ confirmation: null })
    );
    expect(successToast).toHaveBeenCalledWith(`Installed ${library.folderName}`);
    expect(warningToast).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText(/outdated/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Check again" })).toBeNull();
    expect(
      screen.getByRole("button", { name: `Remove ${library.folderName}` })
    ).toBeInTheDocument();
  });

  it.each([true, false])(
    "uses the selected parent's own floor after recovery (required: %s)",
    async (required) => {
      const localDependency = { name: library.folderName, min_version: 100 };
      const selected = {
        ...addon,
        dependsOn: required ? [localDependency] : [],
        optionalDependsOn: required ? [] : [localDependency],
        missingDependencies: required ? [library.folderName] : [],
        missingOptionalDependencies: required ? [] : [library.folderName],
      };
      const otherParent = {
        ...missing,
        folderName: "OtherAddon",
        title: "Other Addon",
      };
      const sharedMismatch = { ...mismatch, installedVersion: 99, downloadedVersion: 150 };
      invokeOrThrow
        .mockResolvedValueOnce({ status: "versionMismatch", mismatch: sharedMismatch })
        .mockResolvedValueOnce({
          status: "installed",
          result,
          versionMismatch: sharedMismatch,
        });
      renderDetail({ addon: selected, installedAddons: [selected, otherParent] });
      fireEvent.click(screen.getByRole("button", { name: "Install" }));
      await screen.findByRole("button", { name: "Install published version" });
      expect(screen.getByText("v100+ (outdated)")).toBeInTheDocument();
      expect(screen.getByText(/required 210 for another installed addon/)).toBeInTheDocument();
      if (required)
        expect(
          screen.getByText(/ESO cannot load Example Addon until LibExample version 100/)
        ).toBeInTheDocument();
      else expect(screen.queryByText(/ESO cannot load/)).toBeNull();
      fireEvent.click(screen.getByRole("button", { name: "Install published version" }));
      await screen.findByText(
        required
          ? /The installed version satisfies Example Addon\./
          : /The installed version satisfies Example Addon's optional requirement/
      );
      expect(screen.getByText("v100+")).toBeInTheDocument();
      expect(screen.queryByText(/outdated/)).toBeNull();
      expect(screen.queryByText(/ESO cannot load/)).toBeNull();
      expect(screen.getByRole("alert")).toHaveTextContent(
        "required 210 for another installed addon"
      );
      expect(screen.getByRole("button", { name: "Check again" })).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: `Remove ${library.folderName}` })
      ).toBeInTheDocument();
      expect(successToast).not.toHaveBeenCalled();
    }
  );

  it("allows a strict retry of blocked recovery and replaces the old warning", async () => {
    const blocked = {
      ...mismatch,
      canInstall: false,
      blockedReason: "Bundled LibOther would replace a newer installed version.",
    };
    const changed = { ...mismatch, downloadedVersion: 3, archiveSha256: "new-digest" };
    invokeOrThrow
      .mockResolvedValueOnce({ status: "versionMismatch", mismatch: blocked })
      .mockResolvedValueOnce({ status: "versionMismatch", mismatch: changed });
    const onRefresh = vi.fn();
    renderDetail({ addon: missing, installedAddons: [missing], onRefresh });
    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    fireEvent.click(await screen.findByRole("button", { name: "Check again" }));
    await screen.findByText(/has version 3, below the required 210/);
    expect(invokeOrThrow).toHaveBeenNthCalledWith(
      2,
      "install_dependency",
      expect.objectContaining({ confirmation: null })
    );
    expect(screen.queryByText(blocked.blockedReason)).toBeNull();
    expect(screen.getByRole("button", { name: "Install published version" })).toBeInTheDocument();
    expect(onRefresh).not.toHaveBeenCalled();
    expect(successToast).not.toHaveBeenCalled();
  });

  it("does not claim a parent is blocked when the installed copy already satisfies all floors", async () => {
    const blocked = {
      ...mismatch,
      installedVersion: 250,
      canInstall: false,
      blockedReason: "The same or a newer LibExample version is already installed.",
    };
    invokeOrThrow.mockResolvedValueOnce({ status: "versionMismatch", mismatch: blocked });
    renderDetail({ addon: missing, installedAddons: [missing] });
    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    await screen.findByText(/Keep the installed copy while checking for a compatible/);
    expect(screen.getByText("v210+")).toBeInTheDocument();
    expect(screen.queryByText(/outdated/)).toBeNull();
    expect(screen.queryByText(/ESO cannot load/)).toBeNull();
    expect(screen.queryByText(/Another installed addon still needs/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Install published version" })).toBeNull();
  });

  it("disables published recovery offline", async () => {
    invokeOrThrow.mockResolvedValueOnce({ status: "versionMismatch", mismatch });
    const rendered = renderDetail({ addon: missing, installedAddons: [missing] });
    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    await screen.findByRole("button", { name: "Install published version" });
    rendered.rerender(
      <AddonDetail
        addon={missing}
        installedAddons={[missing]}
        addonsPath="C:\\test\\AddOns"
        onRefresh={vi.fn()}
        onRemoveAddon={vi.fn()}
        onToggleDisable={vi.fn()}
        updateResult={null}
        onAddonUpdated={vi.fn()}
        onTagsChange={vi.fn()}
        isOffline
      />
    );
    expect(screen.getByRole("button", { name: "Install published version" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Check again" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Install published version" }));
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    expect(invokeOrThrow).toHaveBeenCalledOnce();
  });

  it("keeps the ordinary successful install path", async () => {
    invokeOrThrow.mockResolvedValueOnce({ status: "installed", result, versionMismatch: null });
    const onRefresh = vi.fn();
    renderDetail({ addon: missing, installedAddons: [missing], onRefresh });
    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    await waitFor(() => expect(onRefresh).toHaveBeenCalledOnce());
    expect(successToast).toHaveBeenCalledWith(`Installed ${library.folderName}`);
    expect(warningToast).not.toHaveBeenCalled();
    expect(screen.queryByText(/outdated/)).toBeNull();
  });
});
