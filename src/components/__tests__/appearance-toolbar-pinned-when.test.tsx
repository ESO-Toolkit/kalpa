import type * as React from "react";
import type { ComponentProps } from "react";
import { useCallback, useRef, useState } from "react";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import type { FeatureContext, FeatureId } from "@/lib/features";
import type { TauriResult } from "@/lib/tauri";
import { AppearanceSettings } from "../appearance-settings";
import { Settings } from "../settings";

const mocks = vi.hoisted(() => ({
  getSetting: vi.fn(async (_key: string, fallback: unknown) => fallback),
  setSetting: vi.fn(async () => {}),
  setSettings: vi.fn(async () => {}),
  invokeOrThrow: vi.fn(async () => undefined),
  invokeResult: vi.fn(async (): Promise<TauriResult<boolean>> => ({ ok: true, data: false })),
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

vi.mock("sonner", () => ({
  toast: { error: vi.fn(), info: vi.fn(), success: vi.fn() },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

// The theme gallery is the rest of this panel and has nothing to do with the
// Toolbar list; stub the store binding and the editor so this stays a focused
// render. `account-settings` is the same story for the Settings-level renders.
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
vi.mock("../account-settings", () => ({ AccountSettings: () => null }));

// Only `AnimatePresence` is replaced, following appearance-toolbar.test.tsx: it
// runs `mode="wait"` in Settings and the outgoing panel's exit animation never
// completes in jsdom, so the Appearance panel would never mount. A FULL mock of
// motion/react breaks Checkbox, which renders through `motion.button`.
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

/** Stands in for App.tsx as the single owner of the preference. */
function ToolbarOwner({
  initialHidden = [],
  featureCtx,
  onApplied,
}: {
  initialHidden?: FeatureId[];
  featureCtx: FeatureContext;
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
      featureCtx={featureCtx}
    />
  );
}

const NO_STACK: FeatureContext = { minionDetected: false, graphicsStackDetected: false };
const STACK: FeatureContext = { minionDetected: false, graphicsStackDetected: true };

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

const CAVEAT = "Not in the header yet";
const CAVEAT_WHY = /joins the header once Kalpa detects the setup it manages/;

describe("Settings > Appearance > Toolbar respects pinnedWhen", () => {
  beforeAll(() => {
    defineToolbarEnvironment();
  });

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.invokeResult.mockResolvedValue({ ok: true, data: false });
  });

  // The defect: `pinnableToToolbar` alone said "Graphics stack" was pinned, so
  // the row read as a header button on a machine that has none.
  it("says a feature held back by pinnedWhen is not in the header", () => {
    render(<ToolbarOwner featureCtx={NO_STACK} />);

    const row = featureRow("Graphics stack");
    expect(within(row).getByText(CAVEAT)).toBeInTheDocument();
    expect(within(row).getByText(CAVEAT_WHY)).toBeInTheDocument();
  });

  it("says nothing of the sort about a feature that is genuinely pinned", () => {
    render(<ToolbarOwner featureCtx={NO_STACK} />);

    expect(within(featureRow("Pack Hub")).queryByText(CAVEAT)).toBeNull();
  });

  it("drops the caveat once a graphics stack is detected", () => {
    render(<ToolbarOwner featureCtx={STACK} />);

    expect(within(featureRow("Graphics stack")).queryByText(CAVEAT)).toBeNull();
  });

  // Honest, not hidden: the preference is still there to express, and it is what
  // takes effect the moment a stack shows up.
  it("still records a preference for a feature that has not earned its slot", async () => {
    const onApplied = vi.fn();
    const user = userEvent.setup();
    render(<ToolbarOwner featureCtx={NO_STACK} onApplied={onApplied} />);

    const checkbox = within(featureRow("Graphics stack")).getByRole("checkbox");
    expect(checkbox).toBeEnabled();
    await user.click(checkbox);

    expect(onApplied).toHaveBeenCalledExactlyOnceWith(["client-health"]);
  });

  // Unpinned by choice and held back by `pinnedWhen` are both "not in the
  // header", but only one is the user's doing — stacking both messages on one
  // row would blame them for the machine's missing ReShade.
  it("prefers the unpinned label when the user is the reason it is absent", () => {
    render(<ToolbarOwner initialHidden={["client-health"]} featureCtx={NO_STACK} />);

    const row = featureRow("Graphics stack");
    expect(within(row).getByText("In Settings › Tools")).toBeInTheDocument();
    expect(within(row).queryByText(CAVEAT)).toBeNull();
  });
});

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
    graphicsStackDetected: false,
    minionDetected: false,
    onShowShortcuts: vi.fn(),
    onCheckForAppUpdate: vi.fn(),
    toolbarHidden: [],
    onToolbarHiddenChange: vi.fn(),
    ...overrides,
  };
}

async function openAppearanceTab() {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Appearance" }));
  return user;
}

// Settings owns the detection snapshot; the Toolbar list is only as honest as
// what it is handed. Without the pass-through the component above is correct in
// isolation and still wrong in the app, so this covers the wiring itself.
describe("Settings hands the Appearance tab the same context the Tools tab uses", () => {
  beforeAll(() => {
    defineToolbarEnvironment();
  });

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.invokeResult.mockResolvedValue({ ok: true, data: false });
  });

  it("marks Graphics stack as not-in-the-header when no stack was detected", async () => {
    render(<Settings {...settingsProps({ graphicsStackDetected: false })} />);
    await openAppearanceTab();

    expect(within(featureRow("Graphics stack")).getByText(CAVEAT)).toBeInTheDocument();
  });

  it("leaves the row unqualified when a stack was detected", async () => {
    render(<Settings {...settingsProps({ graphicsStackDetected: true })} />);
    await openAppearanceTab();

    expect(within(featureRow("Graphics stack")).queryByText(CAVEAT)).toBeNull();
  });
});
