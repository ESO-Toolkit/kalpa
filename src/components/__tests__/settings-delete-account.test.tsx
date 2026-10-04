import { readFileSync } from "node:fs";
import { join } from "node:path";
import type * as React from "react";
import type { ComponentProps } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { Settings, DELETE_INCOMPLETE_MESSAGE } from "../settings";

const mocks = vi.hoisted(() => ({
  getSetting: vi.fn(async (_key: string, fallback: unknown) => fallback),
  setSetting: vi.fn(async () => {}),
  setSettings: vi.fn(async () => {}),
  invokeOrThrow: vi.fn(async (_command: string, _args?: Record<string, unknown>) => {
    return undefined as unknown;
  }),
  invokeResult: vi.fn(async () => ({ ok: false, error: "unavailable" })),
  toast: {
    error: vi.fn(),
    info: vi.fn(),
    success: vi.fn(),
    warning: vi.fn(),
  },
}));

vi.mock("@/lib/store", () => ({
  getSetting: mocks.getSetting,
  setSetting: mocks.setSetting,
  setSettings: mocks.setSettings,
}));

// Only the two invoke helpers are replaced. `getTauriErrorMessage` stays REAL
// on purpose: it rewrites anything matching `ERROR_HINTS` before the component
// ever sees it, so a hint pattern that happened to match the partial-deletion
// copy would silently push it back onto the hard-failure branch. A stubbed
// version would hide exactly that.
vi.mock("@/lib/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/tauri")>()),
  invokeOrThrow: mocks.invokeOrThrow,
  invokeResult: mocks.invokeResult,
}));

// AnimatePresence runs `mode="wait"` here, and in jsdom the outgoing panel's
// exit animation never completes — so the Data panel would never mount. Only
// that one export is replaced; the real motion components still render.
vi.mock("motion/react", async (importOriginal) => ({
  ...(await importOriginal<typeof import("motion/react")>()),
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("sonner", () => ({ toast: mocks.toast }));

// Siblings in the same dialog, unrelated to account deletion; stub them so this
// stays a focused render.
vi.mock("../account-settings", () => ({ AccountSettings: () => null }));
vi.mock("../appearance-settings", () => ({ AppearanceSettings: () => null }));

const toast = mocks.toast;

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
    authUser: { userId: "u1", userName: "@tester" },
    authVerifying: false,
    knownInstances: [],
    onAuthChange: vi.fn(),
    onInstancesDetected: vi.fn(),
    onPathChange: vi.fn(),
    onClose: vi.fn(),
    onRefresh: vi.fn(),
    onOpenLogUpload: vi.fn(),
    onOpenFeature: vi.fn(),
    graphicsStackDetected: false,
    minionDetected: false,
    onShowShortcuts: vi.fn(),
    onCheckForAppUpdate: vi.fn(),
    toolbarHidden: [],
    onToolbarHiddenChange: vi.fn(),
    ...overrides,
  };
}

/**
 * Routes `invoke` by command name rather than queueing a `...Once`: mounting
 * Settings fires `detect_game_instances` before any click, which would eat a
 * one-shot mock and leave the deletion resolving `undefined`.
 */
function mockDeleteAccount(outcome: () => unknown) {
  mocks.invokeOrThrow.mockImplementation(async (command: string) => {
    if (command === "delete_pack_hub_account") return outcome();
    return [];
  });
}

/** Walks the Data tab to the armed confirm and presses "Yes, delete everything". */
async function runDeleteAccount() {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Data" }));
  await user.click(screen.getByRole("button", { name: /Delete My Pack Hub Data/ }));
  await user.click(screen.getByRole("button", { name: "Yes, delete everything" }));
  return user;
}

describe("Settings > Data > delete Pack Hub account", () => {
  beforeAll(() => {
    defineDialogEnvironment();
  });

  beforeEach(() => {
    vi.clearAllMocks();
  });

  /**
   * The special case below is an equality test against a string that lives in
   * Rust, so the two copies drifting would not fail a type check, a lint or any
   * other test — it would just quietly restore the "your GDPR erasure failed"
   * bug for a message the backend deliberately worded as partial success. Read
   * the Rust and compare.
   */
  it("mirrors the backend's DELETE_INCOMPLETE_MESSAGE verbatim", () => {
    const source = readFileSync(
      join(process.cwd(), "src-tauri", "src", "pack_hub", "commands.rs"),
      "utf8"
    );
    const declaration = "const DELETE_INCOMPLETE_MESSAGE: &str =";
    const start = source.indexOf(declaration);
    expect(start, "the Rust constant was renamed or removed").toBeGreaterThanOrEqual(0);

    // Hand-parsed rather than regexed: the literal is split across lines with
    // Rust's `\`-newline continuation, which also swallows the next line's
    // indentation, so the raw text is not the runtime value.
    let i = source.indexOf('"', start + declaration.length) + 1;
    let literal = "";
    while (source[i] !== '"') {
      if (source[i] === "\\") {
        i += 1;
        if (source[i] === "\r" || source[i] === "\n") {
          while (/\s/.test(source[i]!)) i += 1;
          continue;
        }
      }
      literal += source[i];
      i += 1;
    }

    expect(literal).toBe(DELETE_INCOMPLETE_MESSAGE);
  });

  it("reports a partially finished erasure as progress, not as a failure", async () => {
    mockDeleteAccount(() => {
      throw new Error(DELETE_INCOMPLETE_MESSAGE);
    });
    render(<Settings {...settingsProps()} />);

    await runDeleteAccount();

    expect(toast.warning).toHaveBeenCalledWith(DELETE_INCOMPLETE_MESSAGE, expect.anything());
    expect(toast.error).not.toHaveBeenCalled();
  });

  it("keeps the user signed in so they can run the second pass", async () => {
    const onAuthChange = vi.fn();
    mockDeleteAccount(() => {
      throw new Error(DELETE_INCOMPLETE_MESSAGE);
    });
    render(<Settings {...settingsProps({ onAuthChange })} />);

    await runDeleteAccount();

    // Signing them out here would strand the leftover votes: only their own
    // session can delete them.
    expect(onAuthChange).not.toHaveBeenCalled();
    // And the confirm stays armed, so "run it once more" is one click away.
    expect(screen.getByRole("button", { name: "Yes, delete everything" })).toBeEnabled();
  });

  it("still reports a real deletion failure as a failure", async () => {
    const onAuthChange = vi.fn();
    mockDeleteAccount(() => {
      throw new Error("Session expired. Please sign in again.");
    });
    render(<Settings {...settingsProps({ onAuthChange })} />);

    await runDeleteAccount();

    expect(toast.error).toHaveBeenCalledWith(
      expect.stringContaining("Session expired. Please sign in again.")
    );
    expect(toast.warning).not.toHaveBeenCalled();
    expect(onAuthChange).not.toHaveBeenCalled();
  });

  it("still signs the user out when erasure completes", async () => {
    const onAuthChange = vi.fn();
    mockDeleteAccount(() => ({ packs: 2, votes: 7, shares: 1 }));
    render(<Settings {...settingsProps({ onAuthChange })} />);

    await runDeleteAccount();

    expect(onAuthChange).toHaveBeenCalledExactlyOnceWith(null);
    expect(toast.success).toHaveBeenCalledWith("Deleted 2 packs, 7 votes, and 1 share code.");
    expect(toast.warning).not.toHaveBeenCalled();
  });
});
