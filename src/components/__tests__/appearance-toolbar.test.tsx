import type * as React from "react";
import { useCallback, useRef, useState } from "react";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import type { FeatureId } from "@/lib/features";
import type { TauriResult } from "@/lib/tauri";
import { AppearanceSettings } from "../appearance-settings";

const mocks = vi.hoisted(() => ({
  getSetting: vi.fn(async (_key: string, fallback: unknown) => fallback),
  setSetting: vi.fn(async () => {}),
  invokeResult: vi.fn(async (): Promise<TauriResult<boolean>> => ({ ok: true, data: false })),
  toastError: vi.fn(),
}));

vi.mock("@/lib/store", () => ({
  getSetting: mocks.getSetting,
  setSetting: mocks.setSetting,
  setSettings: vi.fn(async () => {}),
}));

vi.mock("@/lib/tauri", () => ({
  getTauriErrorMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  invokeOrThrow: vi.fn(async () => undefined),
  invokeResult: mocks.invokeResult,
}));

vi.mock("sonner", () => ({
  toast: { error: mocks.toastError, info: vi.fn(), success: vi.fn() },
}));

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

// The theme gallery is the rest of this panel and has nothing to do with the
// Toolbar section; stub the store binding and the editor so this stays a
// focused render (no swatches, no persisted theme state).
vi.mock("@/lib/use-theme", () => ({
  useTheme: () => ({
    activeThemeId: "kalpa",
    activeTheme: { id: "kalpa", name: "Kalpa", description: "", category: "Core", colors: {} },
    builtinThemes: [],
    customThemes: [],
    setActiveTheme: vi.fn(),
    upsertCustomTheme: vi.fn(async () => true),
    deleteCustomTheme: vi.fn(),
  }),
}));
vi.mock("../theme-editor", () => ({ ThemeEditor: () => null }));

// Only `AnimatePresence` is replaced, following settings-tools-catalog.test.tsx.
// A FULL mock of motion/react breaks Checkbox, which renders through
// `motion.button` / `motion.svg` / `motion.line` / `motion.path` — the real
// components must keep working for the checkboxes to exist at all.
vi.mock("motion/react", async (importOriginal) => ({
  ...(await importOriginal<typeof import("motion/react")>()),
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

function defineToolbarEnvironment() {
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

/**
 * Stands in for App.tsx as the single owner of the preference: it applies the
 * updater against its own latest value (mirrored in a ref, exactly as
 * `handleToolbarHiddenChange` does) and feeds the result back down as a prop.
 * The child never persists anything itself, so there is nothing else to fake.
 */
function ToolbarOwner({
  initialHidden = [],
  onApplied,
}: {
  initialHidden?: FeatureId[];
  onApplied?: (next: FeatureId[]) => void;
}) {
  const [hidden, setHidden] = useState<FeatureId[]>(initialHidden);
  const latest = useRef<FeatureId[]>(initialHidden);

  const handleChange = useCallback(
    (update: (prev: FeatureId[]) => FeatureId[]) => {
      const next = update(latest.current);
      latest.current = next;
      setHidden(next);
      onApplied?.(next);
    },
    [onApplied]
  );

  return (
    <AppearanceSettings
      onShowShortcuts={vi.fn()}
      toolbarHidden={hidden}
      onToolbarHiddenChange={handleChange}
    />
  );
}

/** The Toolbar section only — the Effects section has a checkbox too. */
function toolbarSection(): HTMLElement {
  const section = screen.getByText("Toolbar").closest("section");
  if (!section) throw new Error("Toolbar section not found");
  return section as HTMLElement;
}

function featureRow(label: string): HTMLElement {
  const row = within(toolbarSection()).getByText(label).closest("label");
  if (!row) throw new Error(`row for ${label} not found`);
  return row as HTMLElement;
}

function featureCheckbox(label: string): HTMLElement {
  return within(featureRow(label)).getByRole("checkbox");
}

describe("Settings > Appearance > Toolbar", () => {
  beforeAll(() => {
    defineToolbarEnvironment();
  });

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.invokeResult.mockResolvedValue({ ok: true, data: false });
  });

  it("unpins the feature whose checkbox was unchecked", async () => {
    const onApplied = vi.fn();
    const user = userEvent.setup();
    render(<ToolbarOwner onApplied={onApplied} />);

    await user.click(featureCheckbox("Pack Hub"));

    expect(onApplied).toHaveBeenCalledExactlyOnceWith(["packs"]);
  });

  it("re-pins a feature whose checkbox was checked again", async () => {
    const onApplied = vi.fn();
    const user = userEvent.setup();
    render(<ToolbarOwner initialHidden={["packs"]} onApplied={onApplied} />);

    await user.click(featureCheckbox("Pack Hub"));

    expect(onApplied).toHaveBeenCalledExactlyOnceWith([]);
  });

  it("marks an unpinned feature as living in Settings > Tools", () => {
    render(<ToolbarOwner initialHidden={["packs"]} />);

    expect(within(featureRow("Pack Hub")).getByText("In Settings › Tools")).toBeInTheDocument();
    expect(within(featureRow("Profiles")).queryByText("In Settings › Tools")).toBeNull();
  });

  it("refuses to unpin the uploader while a live upload is running", async () => {
    mocks.invokeResult.mockResolvedValue({ ok: true, data: true });
    const onApplied = vi.fn();
    const user = userEvent.setup();
    render(<ToolbarOwner onApplied={onApplied} />);

    await user.click(featureCheckbox("Log Uploader"));

    expect(mocks.invokeResult).toHaveBeenCalledWith("uploader_live_active");
    expect(mocks.toastError).toHaveBeenCalled();
    // The preference must be untouched, not merely re-rendered as checked.
    expect(onApplied).not.toHaveBeenCalled();
    expect(within(featureRow("Log Uploader")).queryByText("In Settings › Tools")).toBeNull();
  });

  it("keeps the uploader pinned when the live-session check returns an error", async () => {
    mocks.invokeResult.mockResolvedValue({ ok: false, error: "IPC unavailable" });
    const onApplied = vi.fn();
    const user = userEvent.setup();
    render(<ToolbarOwner onApplied={onApplied} />);

    await user.click(featureCheckbox("Log Uploader"));

    expect(onApplied).not.toHaveBeenCalled();
    expect(mocks.toastError).toHaveBeenCalledWith("Couldn't verify live log upload status.", {
      description: "IPC unavailable",
    });
    expect(within(featureRow("Log Uploader")).queryByText("In Settings › Tools")).toBeNull();
  });

  it("keeps the uploader pinned when the live-session check rejects", async () => {
    mocks.invokeResult.mockRejectedValue(new Error("bridge disconnected"));
    const onApplied = vi.fn();
    const user = userEvent.setup();
    render(<ToolbarOwner onApplied={onApplied} />);

    await user.click(featureCheckbox("Log Uploader"));

    expect(onApplied).not.toHaveBeenCalled();
    expect(mocks.toastError).toHaveBeenCalledWith("Couldn't verify live log upload status.", {
      description: "bridge disconnected",
    });
    expect(within(featureRow("Log Uploader")).queryByText("In Settings › Tools")).toBeNull();
  });

  it("coalesces concurrent uploader unpin attempts while status is unknown", async () => {
    let resolveLiveCheck: (value: { ok: true; data: boolean }) => void = () => {};
    mocks.invokeResult.mockReturnValue(
      new Promise((resolve) => {
        resolveLiveCheck = resolve;
      })
    );
    const onApplied = vi.fn();
    render(<ToolbarOwner onApplied={onApplied} />);

    const checkbox = featureCheckbox("Log Uploader");
    fireEvent.click(checkbox);
    fireEvent.click(checkbox);
    expect(mocks.invokeResult).toHaveBeenCalledExactlyOnceWith("uploader_live_active");
    expect(onApplied).not.toHaveBeenCalled();

    resolveLiveCheck({ ok: true, data: false });
    await act(async () => {
      await Promise.resolve();
    });
    expect(onApplied).toHaveBeenCalledExactlyOnceWith(["log-upload"]);
  });

  it("does not lose a toggle made while the uploader's live check is in flight", async () => {
    let resolveLiveCheck: (value: { ok: true; data: boolean }) => void = () => {};
    mocks.invokeResult.mockReturnValue(
      new Promise((resolve) => {
        resolveLiveCheck = resolve;
      })
    );
    const onApplied = vi.fn();
    const user = userEvent.setup();
    render(<ToolbarOwner onApplied={onApplied} />);

    // Uncheck the uploader — its live check is now pending.
    await user.click(featureCheckbox("Log Uploader"));
    expect(onApplied).not.toHaveBeenCalled();

    // Uncheck another row while it is still pending. This one is synchronous.
    await user.click(featureCheckbox("Pack Hub"));
    expect(onApplied).toHaveBeenCalledExactlyOnceWith(["packs"]);

    await act(async () => {
      resolveLiveCheck({ ok: true, data: false });
    });

    // Neither toggle was discarded: the uploader's change was applied on top of
    // the Pack Hub one rather than on top of the snapshot it captured earlier.
    expect(onApplied).toHaveBeenLastCalledWith(["packs", "log-upload"]);
    expect(within(featureRow("Pack Hub")).getByText("In Settings › Tools")).toBeInTheDocument();
    expect(within(featureRow("Log Uploader")).getByText("In Settings › Tools")).toBeInTheDocument();
  });
});
