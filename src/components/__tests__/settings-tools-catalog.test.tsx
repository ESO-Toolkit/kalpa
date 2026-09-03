import type * as React from "react";
import type { ComponentProps } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { Settings } from "../settings";

const mocks = vi.hoisted(() => ({
  getSetting: vi.fn(async (_key: string, fallback: unknown) => fallback),
  setSetting: vi.fn(async () => {}),
  setSettings: vi.fn(async () => {}),
  invokeOrThrow: vi.fn(async () => undefined),
  invokeResult: vi.fn(async () => ({ ok: false, error: "unavailable" })),
}));

vi.mock("@/lib/store", () => ({
  getSetting: mocks.getSetting,
  setSetting: mocks.setSetting,
  setSettings: mocks.setSettings,
}));

vi.mock("@/lib/tauri", () => ({
  getTauriErrorMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  invokeOrThrow: mocks.invokeOrThrow,
  invokeResult: mocks.invokeResult,
}));

// AnimatePresence runs `mode="wait"` here, and in jsdom the outgoing panel's
// exit animation never completes — so the Tools panel would never mount. Only
// that one export is replaced; the real motion components still render.
vi.mock("motion/react", async (importOriginal) => ({
  ...(await importOriginal<typeof import("motion/react")>()),
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("sonner", () => ({
  toast: { error: vi.fn(), info: vi.fn(), success: vi.fn() },
}));

// The two sibling panels are unrelated to the Tools catalog and drag in their
// own Tauri surface area; stub them so this stays a focused render.
vi.mock("../account-settings", () => ({ AccountSettings: () => null }));
vi.mock("../appearance-settings", () => ({ AppearanceSettings: () => null }));

function defineDialogEnvironment() {
  Object.defineProperty(Element.prototype, "getAnimations", {
    configurable: true,
    value: () => [],
  });
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: (query: string) =>
      ({
        matches: false,
        media: query,
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(() => false),
      }) as MediaQueryList,
  });
}

type SettingsProps = ComponentProps<typeof Settings>;

function settingsProps(overrides: Partial<SettingsProps> = {}): SettingsProps {
  return {
    addonsPath: "C:/Games/ESO/AddOns",
    authUser: null,
    authVerifying: false,
    knownInstances: [],
    onAuthChange: vi.fn(),
    onInstancesDetected: vi.fn(),
    onPathChange: vi.fn(),
    onClose: vi.fn(),
    onRefresh: vi.fn(),
    onOpenLogUpload: vi.fn(),
    onOpenFeature: vi.fn(),
    minionDetected: false,
    onShowShortcuts: vi.fn(),
    onCheckForAppUpdate: vi.fn(),
    toolbarHidden: [],
    onToolbarHiddenChange: vi.fn(),
    ...overrides,
  };
}

async function openToolsTab() {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Tools" }));
  return user;
}

describe("Settings > Tools is the catalog for unpinned features", () => {
  beforeAll(() => {
    defineDialogEnvironment();
  });

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("does not list a pinnable feature while it is still pinned to the toolbar", async () => {
    render(<Settings {...settingsProps({ toolbarHidden: [] })} />);
    await openToolsTab();

    expect(screen.getByText("Backup & Restore")).toBeInTheDocument();
    expect(screen.queryByText("Pack Hub")).not.toBeInTheDocument();
  });

  it("lists an unpinned Pack Hub so it stays reachable without the toolbar button", async () => {
    render(<Settings {...settingsProps({ toolbarHidden: ["packs"] })} />);
    await openToolsTab();

    expect(screen.getByText("Pack Hub")).toBeInTheDocument();
  });

  it("opens the unpinned feature's dialog when its row is clicked", async () => {
    const onOpenFeature = vi.fn();
    render(<Settings {...settingsProps({ toolbarHidden: ["packs"], onOpenFeature })} />);
    const user = await openToolsTab();

    await user.click(screen.getByText("Pack Hub"));

    expect(onOpenFeature).toHaveBeenCalledExactlyOnceWith("packs");
  });

  it("keeps the static rows below the unpinned group", async () => {
    render(<Settings {...settingsProps({ toolbarHidden: ["log-upload"] })} />);
    await openToolsTab();

    expect(screen.getByText("Log Uploader")).toBeInTheDocument();
    expect(screen.getByText("Check for App Updates")).toBeInTheDocument();
    expect(screen.getByText("Safety Center")).toBeInTheDocument();
  });
});
