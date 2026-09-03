import { useState, type ReactNode } from "react";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AddonManifest, ViewMode } from "@/types";
import { AddonList } from "@/components/addon-list";

vi.mock("@/components/discover-panel", () => ({
  DiscoverPanel: () => <div>Discover test panel</div>,
}));

vi.mock("motion/react", async () => {
  const React = await import("react");

  function withoutMotionProps(props: Record<string, unknown>) {
    const domProps = { ...props };
    for (const name of [
      "animate",
      "exit",
      "initial",
      "layout",
      "layoutId",
      "transition",
      "variants",
      "whileTap",
    ]) {
      delete domProps[name];
    }
    return domProps;
  }

  const motionElement = (tagName: string) =>
    React.forwardRef<Element, Record<string, unknown>>((props, ref) =>
      React.createElement(tagName, { ...withoutMotionProps(props), ref })
    );

  function AnimatePresence({ children }: { children: React.ReactNode }) {
    const [renderedChild, setRenderedChild] = React.useState(children);

    // `mode="wait"` keeps the exiting child for a commit, then mounts the
    // incoming child from AnimatePresence's own state update.
    React.useEffect(() => setRenderedChild(children), [children]);

    return React.createElement(React.Fragment, null, renderedChild);
  }

  return {
    AnimatePresence,
    motion: {
      create: (component: React.ElementType) => component,
      button: motionElement("button"),
      div: motionElement("div"),
      line: motionElement("line"),
      path: motionElement("path"),
      span: motionElement("span"),
      svg: motionElement("svg"),
    },
    useReducedMotion: () => false,
  };
});

vi.mock("@tanstack/react-virtual", async () => {
  const React = await import("react");

  interface VirtualizerOptions {
    count: number;
    estimateSize: () => number;
    getScrollElement: () => HTMLElement | null;
  }

  return {
    useVirtualizer(options: VirtualizerOptions) {
      const optionsRef = React.useRef(options);
      const observedElementRef = React.useRef<HTMLElement | null>(null);
      const [, forceRender] = React.useReducer((version: number) => version + 1, 0);
      optionsRef.current = options;

      React.useLayoutEffect(() => {
        const nextElement = optionsRef.current.getScrollElement();
        if (nextElement === observedElementRef.current) return;

        observedElementRef.current = nextElement;
        if (nextElement) forceRender();
      });

      return {
        getTotalSize: () => optionsRef.current.count * optionsRef.current.estimateSize(),
        getVirtualItems: () => {
          const currentElement = optionsRef.current.getScrollElement();
          if (!currentElement || currentElement !== observedElementRef.current) return [];

          const size = optionsRef.current.estimateSize();
          return Array.from({ length: optionsRef.current.count }, (_, index) => ({
            index,
            key: index,
            lane: 0,
            size,
            start: index * size,
            end: (index + 1) * size,
          }));
        },
        measureElement: () => undefined,
        scrollToIndex: () => undefined,
      };
    },
  };
});

const addon: AddonManifest = {
  folderName: "LifecycleTestAddon",
  title: "Lifecycle Test Addon",
  author: "Kalpa QA",
  version: "1.0.0",
  addonVersion: 1,
  apiVersion: [101047],
  description: "Exercises the installed-list virtualizer lifecycle.",
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

function Harness({
  children,
  onRemoveAddon,
  removalBlockedReason,
  onSelect,
  onToggleSelect,
  selectedFolders,
  selectedAddon,
}: {
  children?: ReactNode;
  onRemoveAddon?: (folderName: string) => void;
  removalBlockedReason?: string;
  onSelect?: (addon: AddonManifest) => void;
  onToggleSelect?: (folderName: string) => void;
  selectedFolders?: Set<string>;
  selectedAddon?: AddonManifest | null;
}) {
  const [viewMode, setViewMode] = useState<ViewMode>("installed");

  return (
    <>
      <AddonList
        addons={[addon]}
        allAddons={[addon]}
        selectedAddon={selectedAddon ?? null}
        onSelect={onSelect ?? vi.fn()}
        searchQuery=""
        onSearchChange={vi.fn()}
        loading={false}
        updateResults={[]}
        sortMode="name"
        onSortChange={vi.fn()}
        filterMode="all"
        onFilterChange={vi.fn()}
        activeTagFilter={null}
        onActiveTagFilterChange={vi.fn()}
        selectedFolders={selectedFolders ?? new Set()}
        onToggleSelect={onToggleSelect ?? vi.fn()}
        viewMode={viewMode}
        onViewModeChange={setViewMode}
        discoverTab="search"
        onDiscoverTabChange={vi.fn()}
        addonsPath="C:\\Users\\test\\Documents\\Elder Scrolls Online\\live\\AddOns"
        onInstalled={vi.fn()}
        onSelectDiscoverResult={vi.fn()}
        selectedDiscoverResultId={null}
        installedEsouiIds={new Set([addon.esouiId!])}
        onRemoveAddon={onRemoveAddon}
        removalBlockedReason={removalBlockedReason}
      />
      {children}
    </>
  );
}

describe("AddonList", () => {
  it("repopulates installed addons after returning from Discover", async () => {
    render(<Harness />);

    expect(await screen.findByText(addon.title)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: "Discover" }));
    expect(await screen.findByText("Discover test panel")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: "My Addons" }));
    expect(await screen.findByText(addon.title)).toBeInTheDocument();
  });

  it("blocks context-menu removal with an explanation, then restores it", async () => {
    const onRemoveAddon = vi.fn();
    const reason = "Wait for the current update batch to finish before removing addons.";
    const { rerender } = render(
      <Harness onRemoveAddon={onRemoveAddon} removalBlockedReason={reason} />
    );

    fireEvent.contextMenu(await screen.findByText(addon.title), { clientX: 10, clientY: 10 });
    const blockedRemove = await screen.findByRole("menuitem", { name: `Remove — ${reason}` });
    expect(blockedRemove).toHaveAttribute("aria-disabled", "true");
    fireEvent.click(blockedRemove);
    expect(onRemoveAddon).not.toHaveBeenCalled();

    rerender(<Harness onRemoveAddon={onRemoveAddon} />);
    const enabledRemove = await screen.findByRole("menuitem", { name: "Remove" });
    expect(enabledRemove).toBeEnabled();
    fireEvent.click(enabledRemove);
    expect(onRemoveAddon).toHaveBeenCalledWith(addon.folderName);
  });
  it("exposes exactly one checkbox per row, with no nested interactive elements", async () => {
    const { container } = render(<Harness />);
    const row = await screen.findByRole("option");

    expect(within(row).getAllByRole("checkbox")).toHaveLength(1);
    expect(container.querySelectorAll("button button")).toHaveLength(0);
  });

  it("keeps the row checkbox out of the tab order until batch mode is active", async () => {
    const { rerender } = render(<Harness />);
    expect(await screen.findByRole("checkbox", { name: `Select ${addon.title}` })).toHaveAttribute(
      "tabindex",
      "-1"
    );

    rerender(<Harness selectedFolders={new Set([addon.folderName])} />);
    expect(screen.getByRole("checkbox", { name: `Select ${addon.title}` })).toHaveAttribute(
      "tabindex",
      "0"
    );
  });

  it("toggles selection by pointer without opening the addon", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const onToggleSelect = vi.fn();
    render(<Harness onSelect={onSelect} onToggleSelect={onToggleSelect} />);

    await user.click(await screen.findByRole("checkbox", { name: `Select ${addon.title}` }));

    expect(onToggleSelect).toHaveBeenCalledTimes(1);
    expect(onToggleSelect).toHaveBeenCalledWith(addon.folderName);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("toggles selection from the keyboard once batch mode is active", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const onToggleSelect = vi.fn();
    render(
      <Harness
        onSelect={onSelect}
        onToggleSelect={onToggleSelect}
        selectedFolders={new Set([addon.folderName])}
      />
    );

    const checkbox = await screen.findByRole("checkbox", { name: `Select ${addon.title}` });
    expect(checkbox).toHaveAttribute("aria-checked", "true");

    checkbox.focus();
    expect(checkbox).toHaveFocus();
    await user.keyboard("[Space]");

    expect(onToggleSelect).toHaveBeenCalledTimes(1);
    expect(onToggleSelect).toHaveBeenCalledWith(addon.folderName);
    expect(onSelect).not.toHaveBeenCalled();
  });
  it("drives batch-select mode end to end from the row checkbox", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();

    function BatchHarness() {
      const [selectedFolders, setSelectedFolders] = useState<Set<string>>(new Set());
      return (
        <Harness
          onSelect={onSelect}
          selectedFolders={selectedFolders}
          onToggleSelect={(folderName) =>
            setSelectedFolders((prev) => {
              const next = new Set(prev);
              if (next.has(folderName)) next.delete(folderName);
              else next.add(folderName);
              return next;
            })
          }
        />
      );
    }

    render(<BatchHarness />);

    const checkbox = await screen.findByRole("checkbox", { name: `Select ${addon.title}` });
    expect(checkbox).toHaveAttribute("aria-checked", "false");
    expect(checkbox).toHaveAttribute("tabindex", "-1");

    // Entering batch mode checks the row and pulls the checkbox into the tab order.
    await user.click(checkbox);
    expect(checkbox).toHaveAttribute("aria-checked", "true");
    expect(checkbox).toHaveAttribute("tabindex", "0");
    expect(await screen.findByText("· 1 selected")).toBeInTheDocument();

    // Leaving it again clears the row and drops the checkbox back out.
    await user.click(checkbox);
    expect(checkbox).toHaveAttribute("aria-checked", "false");
    expect(checkbox).toHaveAttribute("tabindex", "-1");
    expect(screen.queryByText("· 1 selected")).not.toBeInTheDocument();
    expect(onSelect).not.toHaveBeenCalled();
  });
  it("still navigates the list from the keyboard when the listbox holds focus", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();

    // Space only re-selects the current row, so the harness has to track which
    // row ArrowDown landed on.
    function NavHarness() {
      const [selected, setSelected] = useState<AddonManifest | null>(null);
      return (
        <Harness
          selectedAddon={selected}
          onSelect={(next) => {
            setSelected(next);
            onSelect(next);
          }}
        />
      );
    }

    render(<NavHarness />);

    const listbox = await screen.findByRole("listbox");
    listbox.focus();
    expect(listbox).toHaveFocus();

    await user.keyboard("{ArrowDown}");
    expect(onSelect).toHaveBeenCalledWith(addon);

    onSelect.mockClear();
    await user.keyboard("[Space]");
    expect(onSelect).toHaveBeenCalledWith(addon);
  });
});
