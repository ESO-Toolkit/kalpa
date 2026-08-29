import { useState, type ReactNode } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
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

function Harness({ children }: { children?: ReactNode }) {
  const [viewMode, setViewMode] = useState<ViewMode>("installed");

  return (
    <>
      <AddonList
        addons={[addon]}
        allAddons={[addon]}
        selectedAddon={null}
        onSelect={vi.fn()}
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
        selectedFolders={new Set()}
        onToggleSelect={vi.fn()}
        viewMode={viewMode}
        onViewModeChange={setViewMode}
        discoverTab="search"
        onDiscoverTabChange={vi.fn()}
        addonsPath="C:\\Users\\test\\Documents\\Elder Scrolls Online\\live\\AddOns"
        onInstalled={vi.fn()}
        onSelectDiscoverResult={vi.fn()}
        selectedDiscoverResultId={null}
        installedEsouiIds={new Set([addon.esouiId!])}
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
});
