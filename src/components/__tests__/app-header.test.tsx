import type { ComponentProps } from "react";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { FEATURES } from "@/lib/features";
import { AppHeader } from "../app-header";

vi.mock("@/components/account-chip", () => ({
  AccountChip: () => <div data-testid="account-chip-stub" />,
}));

function defineHeaderEnvironment() {
  Object.defineProperty(Element.prototype, "getAnimations", {
    configurable: true,
    value: () => [],
  });

  // CountingNumber (rendered by AppHeader) uses framer-motion's useInView,
  // which needs IntersectionObserver — jsdom doesn't implement it.
  class MockIntersectionObserver implements Partial<IntersectionObserver> {
    readonly root: Element | Document | null = null;
    readonly rootMargin: string = "";
    readonly scrollMargin: string = "";
    readonly thresholds: ReadonlyArray<number> = [];
    disconnect() {}
    observe() {}
    takeRecords(): IntersectionObserverEntry[] {
      return [];
    }
    unobserve() {}
  }
  vi.stubGlobal("IntersectionObserver", MockIntersectionObserver);

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

describe("AppHeader", () => {
  beforeAll(() => {
    defineHeaderEnvironment();
  });

  type AppHeaderProps = ComponentProps<typeof AppHeader>;

  function appHeaderProps(overrides: Partial<AppHeaderProps> = {}): AppHeaderProps {
    const toolbarFeatures = FEATURES.filter((f) => f.pinnableToToolbar);
    return {
      addonsCount: 0,
      batchMode: false,
      batchDisabling: false,
      checkingUpdates: false,
      loading: false,
      selectedCount: 0,
      updatingAll: false,
      authUser: null,
      authVerifying: false,
      instances: [],
      activeAddonsPath: "",
      onBatchCancel: vi.fn(),
      onBatchDisable: vi.fn(),
      onBatchRemove: vi.fn(),
      onBatchTag: vi.fn(),
      onBatchUpdate: vi.fn(),
      toolbarFeatures,
      onOpenFeature: vi.fn(),
      onOpenSettings: vi.fn(),
      onOpenSupport: vi.fn(),
      onAuthChange: vi.fn(),
      onRefresh: vi.fn(),
      onSwitchInstance: vi.fn(),
      ...overrides,
    };
  }

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders every pinned aria-label, including the five the e2e suite depends on", () => {
    render(<AppHeader {...appHeaderProps()} />);

    for (const label of [
      "Addon Packs",
      "Profiles",
      "Saved Vars",
      "Upload to ESO Logs",
      "Settings",
    ]) {
      expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
    }
  });

  it("renders the uploader button icon-only, with no visible text label", () => {
    render(<AppHeader {...appHeaderProps()} />);

    const uploadButton = screen.getByRole("button", { name: "Upload to ESO Logs" });
    expect(within(uploadButton).queryByText("Upload logs")).not.toBeInTheDocument();
    expect(uploadButton).toHaveTextContent("");
  });

  it("invokes onOpenFeature('log-upload') when the uploader button is clicked", async () => {
    const user = userEvent.setup();
    const onOpenFeature = vi.fn();
    render(<AppHeader {...appHeaderProps({ onOpenFeature })} />);

    await user.click(screen.getByRole("button", { name: "Upload to ESO Logs" }));

    expect(onOpenFeature).toHaveBeenCalledExactlyOnceWith("log-upload");
  });

  it("renders no Tools overflow control — unpinned features live in Settings > Tools", () => {
    render(<AppHeader {...appHeaderProps({ toolbarFeatures: [] })} />);

    expect(screen.queryByRole("button", { name: "Tools" })).not.toBeInTheDocument();
  });
});
