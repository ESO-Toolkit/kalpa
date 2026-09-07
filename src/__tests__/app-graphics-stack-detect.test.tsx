import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";

/**
 * Startup's graphics-stack probe has to survive a command that resolves to
 * nothing.
 *
 * `invokeResult` reports `ok` for a command that resolves to null — which is
 * what an unstubbed command returns under test, and what a backend that finds
 * nothing may return in production. The `detect_eso_clients` half already
 * guards for it, in a comment that spells the reason out; the
 * `inspect_client_stack` half seven lines later did not, and `data` is typed
 * non-nullable so TypeScript said nothing. The failure mode is quiet: a
 * TypeError inside an un-`catch`ed async `.then`, an unhandled rejection, and a
 * toolbar slot that simply never appears.
 */

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

// The one probe: App's own answer, read straight off the prop it hands down.
vi.mock("@/components/app-dialogs", () => ({
  AppDialogs: ({ graphicsStackDetected }: { graphicsStackDetected: boolean }) => (
    <output data-testid="graphics-stack-detected">{String(graphicsStackDetected)}</output>
  ),
}));

vi.mock("@/components/app-header", () => ({ AppHeader: () => null }));
vi.mock("@/components/addon-list", () => ({ AddonList: () => null }));
vi.mock("@/components/addon-detail", () => ({ AddonDetail: () => null }));
vi.mock("@/components/app-background", () => ({ AppBackground: () => null }));
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

const client = {
  client_dir: "C:\\ESO",
  exe_path: "C:\\ESO\\eso64.exe",
  source: "manual",
};

function installStartupIpc(stackData: unknown) {
  mocks.getSetting.mockImplementation(async (key: string, fallback: unknown) =>
    key === "addonsPath" ? ADDONS_PATH : fallback
  );
  mocks.setSetting.mockResolvedValue(true);
  mocks.invokeOrThrow.mockResolvedValue([]);
  mocks.invokeResult.mockImplementation(async (command: string) => {
    if (command === "debug_addons_dir_override") return { ok: true, data: null };
    if (command === "uploader_detect_path") {
      return { ok: true, data: { encounterLogExists: false } };
    }
    if (command === "auto_link_addons") return { ok: true, data: { linked: [], notFound: [] } };
    if (command === "detect_eso_clients") return { ok: true, data: [client] };
    if (command === "inspect_client_stack") return { ok: true, data: stackData };
    return { ok: true, data: null };
  });
}

describe("App graphics-stack detection", () => {
  const rejections: unknown[] = [];
  const record = (reason: unknown) => rejections.push(reason);

  beforeEach(() => {
    rejections.length = 0;
    vi.clearAllMocks();
    process.on("unhandledRejection", record);
  });

  afterEach(() => {
    process.off("unhandledRejection", record);
  });

  it("survives a successful inspect that resolves to nothing", async () => {
    installStartupIpc(null);
    render(<App />);

    await waitFor(() =>
      expect(
        mocks.invokeResult.mock.calls.some(([command]) => command === "inspect_client_stack")
      ).toBe(true)
    );
    // Let Node settle the microtask queue so an unhandled rejection would be
    // reported by now.
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(rejections).toEqual([]);
    expect(screen.getByTestId("graphics-stack-detected")).toHaveTextContent("false");
  });

  it("still reports a stack when the inspect returns one", async () => {
    installStartupIpc({ is_empty: false });
    render(<App />);

    await waitFor(() =>
      expect(screen.getByTestId("graphics-stack-detected")).toHaveTextContent("true")
    );
    expect(rejections).toEqual([]);
  });
});
