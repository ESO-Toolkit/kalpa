import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { AddonManifest, AutoLinkResult, UpdateCheckResult } from "../types";

const mocks = vi.hoisted(() => {
  const toast = Object.assign(vi.fn(), {
    error: vi.fn(),
    info: vi.fn(),
    success: vi.fn(),
    warning: vi.fn(),
  });

  return {
    getSetting: vi.fn(),
    setSetting: vi.fn(),
    invokeOrThrow: vi.fn(),
    invokeResult: vi.fn(),
    toast,
  };
});

vi.mock("sonner", () => ({ toast: mocks.toast }));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => undefined),
}));

vi.mock("@/lib/store", () => ({
  getSetting: mocks.getSetting,
  setSetting: mocks.setSetting,
}));

vi.mock("@/lib/tauri", () => ({
  getTauriErrorMessage: (error: unknown) => String(error),
  invokeOrThrow: mocks.invokeOrThrow,
  invokeResult: mocks.invokeResult,
  warnIfSessionNotPersisted: vi.fn(),
}));

vi.mock("@/lib/eso-running-context", () => ({
  EsoRunningProvider: ({ children }: { children: ReactNode }) => children,
}));

vi.mock("@/lib/dependency-prompt-context", () => ({
  DependencyPromptProvider: ({ children }: { children: ReactNode }) => children,
}));

vi.mock("@/components/app-update", () => ({
  useAppUpdate: () => ({
    state: { status: "idle" },
    checkForAppUpdate: vi.fn(),
    downloadAndInstall: vi.fn(),
    restartApp: vi.fn(),
  }),
}));

vi.mock("@/components/app-header", () => ({
  AppHeader: ({
    activeAddonsPath,
    onRefresh,
  }: {
    activeAddonsPath: string;
    onRefresh: () => void;
  }) => (
    <div>
      <output data-testid="active-addons-path">{activeAddonsPath}</output>
      <button type="button" onClick={onRefresh}>
        Refresh
      </button>
    </div>
  ),
}));

vi.mock("@/components/addon-list", () => ({
  AddonList: ({
    addons,
    updateResults,
    onRemoveAddon,
  }: {
    addons: AddonManifest[];
    updateResults: UpdateCheckResult[];
    onRemoveAddon: (folderName: string) => void;
  }) => (
    <div>
      <output data-testid="addons">{addons.map((addon) => addon.folderName).join(",")}</output>
      <output data-testid="updates">
        {updateResults.map((result) => result.folderName).join(",")}
      </output>
      {addons.map((addon) => (
        <button
          key={addon.folderName}
          type="button"
          onClick={() => onRemoveAddon(addon.folderName)}
        >
          Remove {addon.folderName}
        </button>
      ))}
    </div>
  ),
}));

vi.mock("@/components/addon-detail", () => ({ AddonDetail: () => null }));
vi.mock("@/components/app-background", () => ({ AppBackground: () => null }));
vi.mock("@/components/app-dialogs", () => ({ AppDialogs: () => null }));
vi.mock("@/components/cfa-guidance-dialog", () => ({ CfaGuidanceDialog: () => null }));
vi.mock("@/components/dependency-picker-dialog", () => ({ DependencyPickerDialog: () => null }));
vi.mock("@/components/discover-detail", () => ({ DiscoverDetail: () => null }));
vi.mock("@/components/eso-running-dialog", () => ({ EsoRunningDialog: () => null }));
vi.mock("@/components/roster-pack-install", () => ({ RosterPackInstall: () => null }));
vi.mock("@/components/setup-wizard", () => ({ SetupWizard: () => null }));
vi.mock("@/components/status-banners", () => ({ StatusBanners: () => null }));
vi.mock("@/components/update-banner", () => ({ UpdateBanner: () => null }));
vi.mock("@/components/uploader-intro-card", () => ({ UploaderIntroCard: () => null }));

import App from "../App";

const ADDONS_PATH = "C:\\Games\\ESO\\live\\AddOns";

const addon: AddonManifest = {
  folderName: "Doomed",
  title: "Doomed",
  author: "Kalpa QA",
  version: "1.0.0",
  addonVersion: 1,
  apiVersion: [101047],
  description: "",
  isLibrary: false,
  dependsOn: [],
  optionalDependsOn: [],
  missingDependencies: [],
  outdatedDependencies: [],
  missingOptionalDependencies: [],
  esouiId: 42,
  tags: [],
  esouiLastUpdate: 0,
  installedAt: "2026-08-28T00:00:00.000Z",
  disabled: false,
  modifiedFileCount: 0,
};

const update: UpdateCheckResult = {
  folderName: addon.folderName,
  esouiId: 42,
  currentVersion: "1.0.0",
  remoteVersion: "2.0.0",
  downloadUrl: "https://example.invalid/doomed.zip",
  hasUpdate: true,
  remoteLastUpdate: 0,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((fulfill) => {
    resolve = fulfill;
  });
  return { promise, resolve };
}

function commandCalls(command: string) {
  return mocks.invokeOrThrow.mock.calls.filter(([calledCommand]) => calledCommand === command);
}

async function removeSuccessfully() {
  vi.useFakeTimers();
  fireEvent.click(screen.getByRole("button", { name: `Remove ${addon.folderName}` }));

  expect(screen.getByTestId("addons")).toBeEmptyDOMElement();
  expect(screen.getByTestId("updates")).toBeEmptyDOMElement();

  await act(async () => {
    await vi.advanceTimersByTimeAsync(3000);
  });

  expect(commandCalls("remove_addon")).toHaveLength(1);
}

describe("App refresh and removal epochs", () => {
  const scanResponses: Array<Promise<AddonManifest[]>> = [];
  const updateResponses: Array<Promise<UpdateCheckResult[]>> = [];
  const linkResponses: Array<Promise<{ ok: true; data: AutoLinkResult }>> = [];

  beforeEach(() => {
    scanResponses.length = 0;
    updateResponses.length = 0;
    linkResponses.length = 0;
    vi.clearAllMocks();

    mocks.getSetting.mockImplementation(async (key: string, fallback: unknown) =>
      key === "addonsPath" ? ADDONS_PATH : fallback
    );
    mocks.setSetting.mockResolvedValue(true);
    mocks.invokeResult.mockImplementation(async (command: string) => {
      if (command === "debug_addons_dir_override") return { ok: true, data: null };
      if (command === "uploader_detect_path") {
        return { ok: true, data: { encounterLogExists: false } };
      }
      if (command === "auto_link_addons") {
        return (await linkResponses.shift()) ?? { ok: true, data: { linked: [], notFound: [] } };
      }
      return { ok: true, data: null };
    });
    mocks.invokeOrThrow.mockImplementation(async (command: string) => {
      if (command === "consume_initial_deep_link") {
        return {
          packId: null,
          shareCode: null,
          installPackId: null,
          appUpdate: false,
          logUpload: false,
          packHub: false,
        };
      }
      if (command === "native_boot_failure_pending") return false;
      if (command === "scan_installed_addons") return await scanResponses.shift();
      if (command === "check_for_updates") return await updateResponses.shift();
      if (command === "remove_addon") return { cleanupWarning: null };
      if (command === "detect_game_instances") return [];
      return undefined;
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("links externally installed addons before the first scan and update check", async () => {
    const linked = deferred<{ ok: true; data: AutoLinkResult }>();
    linkResponses.push(linked.promise);
    scanResponses.push(Promise.resolve([addon]));
    updateResponses.push(Promise.resolve([update]));

    render(<App />);

    await waitFor(() =>
      expect(mocks.invokeResult).toHaveBeenCalledWith("auto_link_addons", {
        addonsPath: ADDONS_PATH,
      })
    );
    expect(commandCalls("scan_installed_addons")).toHaveLength(0);
    expect(commandCalls("check_for_updates")).toHaveLength(0);

    await act(async () =>
      linked.resolve({
        ok: true,
        data: { linked: [addon.folderName], notFound: [], skippedBundled: [] },
      })
    );
    await waitFor(() => expect(screen.getByTestId("addons")).toHaveTextContent(addon.folderName));
    await waitFor(() => expect(screen.getByTestId("updates")).toHaveTextContent(addon.folderName));
    expect(commandCalls("scan_installed_addons")).toHaveLength(1);
    expect(commandCalls("check_for_updates")).toHaveLength(1);
  });

  it("discovers external installs again on refresh and discards an older discovery", async () => {
    scanResponses.push(Promise.resolve([]), Promise.resolve([addon]));
    updateResponses.push(Promise.resolve([]), Promise.resolve([update]));
    render(<App />);
    await waitFor(() => expect(commandCalls("check_for_updates")).toHaveLength(1));
    await waitFor(() =>
      expect(screen.getByTestId("active-addons-path")).toHaveTextContent(ADDONS_PATH)
    );

    const staleLink = deferred<{ ok: true; data: AutoLinkResult }>();
    const currentLink = deferred<{ ok: true; data: AutoLinkResult }>();
    linkResponses.push(staleLink.promise);
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() =>
      expect(
        mocks.invokeResult.mock.calls.filter(([name]) => name === "auto_link_addons")
      ).toHaveLength(2)
    );
    expect(commandCalls("scan_installed_addons")).toHaveLength(1);

    linkResponses.push(currentLink.promise);
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await act(async () =>
      currentLink.resolve({
        ok: true,
        data: { linked: [addon.folderName], notFound: [], skippedBundled: [] },
      })
    );
    await waitFor(() => expect(screen.getByTestId("updates")).toHaveTextContent(addon.folderName));
    await act(async () =>
      staleLink.resolve({ ok: true, data: { linked: [], notFound: [], skippedBundled: [] } })
    );

    expect(commandCalls("scan_installed_addons")).toHaveLength(2);
    expect(commandCalls("check_for_updates")).toHaveLength(2);
    expect(screen.getByTestId("addons")).toHaveTextContent(addon.folderName);
    expect(screen.getByTestId("updates")).toHaveTextContent(addon.folderName);
  });

  it("does not reintroduce a removed addon when an older scan resolves", async () => {
    const staleScan = deferred<AddonManifest[]>();
    scanResponses.push(Promise.resolve([addon]), staleScan.promise, Promise.resolve([]));
    updateResponses.push(Promise.resolve([update]), Promise.resolve([]));

    render(<App />);

    await waitFor(() => expect(screen.getByTestId("addons")).toHaveTextContent(addon.folderName));
    await waitFor(() => expect(screen.getByTestId("updates")).toHaveTextContent(addon.folderName));
    expect(screen.getByTestId("active-addons-path")).toHaveTextContent(ADDONS_PATH);

    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(commandCalls("scan_installed_addons")).toHaveLength(2));

    await removeSuccessfully();
    await act(async () => staleScan.resolve([addon]));
    expect(commandCalls("scan_installed_addons")).toHaveLength(3);

    expect(screen.getByTestId("addons")).toBeEmptyDOMElement();
    expect(screen.queryByRole("button", { name: `Remove ${addon.folderName}` })).toBeNull();
  });

  it("does not republish a removed addon's update when an older check resolves", async () => {
    const staleUpdate = deferred<UpdateCheckResult[]>();
    scanResponses.push(Promise.resolve([addon]), Promise.resolve([addon]));
    updateResponses.push(Promise.resolve([update]), staleUpdate.promise, Promise.resolve([]));

    render(<App />);

    await waitFor(() => expect(screen.getByTestId("addons")).toHaveTextContent(addon.folderName));
    await waitFor(() => expect(screen.getByTestId("updates")).toHaveTextContent(addon.folderName));
    expect(screen.getByTestId("active-addons-path")).toHaveTextContent(ADDONS_PATH);

    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(commandCalls("check_for_updates")).toHaveLength(2));

    await removeSuccessfully();
    await act(async () => staleUpdate.resolve([update]));
    expect(commandCalls("check_for_updates")).toHaveLength(3);

    expect(screen.getByTestId("updates")).toBeEmptyDOMElement();
  });
});
