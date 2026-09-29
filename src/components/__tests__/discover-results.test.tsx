import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { EsouiSearchResult } from "@/types";

const { invokeOrThrow } = vi.hoisted(() => ({ invokeOrThrow: vi.fn() }));

vi.mock("@/lib/tauri", () => ({
  invokeOrThrow,
  getTauriErrorMessage: (error: unknown) => String(error),
}));

vi.mock("sonner", () => ({
  toast: Object.assign(vi.fn(), { success: vi.fn(), error: vi.fn(), info: vi.fn() }),
}));

vi.mock("@/lib/eso-running-context", () => ({
  useEnsureEsoNotBlocking: () => async () => true,
}));

vi.mock("@/lib/dependency-prompt-context", () => ({
  useResolvePendingDeps: () => vi.fn(),
}));

vi.mock("motion/react", async () => {
  const React = await import("react");
  const motionElement = (tag: string) =>
    React.forwardRef<Element, Record<string, unknown>>((props, ref) => {
      const { animate, exit, initial, layoutId, transition, ...domProps } = props;
      void animate;
      void exit;
      void initial;
      void layoutId;
      void transition;
      return React.createElement(tag, { ...domProps, ref });
    });
  return {
    AnimatePresence: ({ children }: { children: React.ReactNode }) => children,
    useInView: () => true,
    useReducedMotion: () => false,
    motion: {
      div: motionElement("div"),
      span: motionElement("span"),
    },
  };
});

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count, estimateSize }: { count: number; estimateSize: () => number }) => ({
    getTotalSize: () => count * estimateSize(),
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({ index, start: index * estimateSize() })),
    measureElement: () => undefined,
    scrollToIndex: () => undefined,
  }),
}));

import { DiscoverPanel } from "@/components/discover-panel";

const addon: EsouiSearchResult = {
  id: 1710,
  title: "Skyshards",
  author: "Test Author",
  category: "Maps",
  downloads: "100",
  updated: "2026-08-01",
};

function renderDiscover(installedIds = new Set<number>()) {
  const onSelectResult = vi.fn();
  render(
    <DiscoverPanel
      activeTab="search"
      onTabChange={vi.fn()}
      addonsPath="C:\\ESO\\AddOns"
      onInstalled={vi.fn()}
      onSelectResult={onSelectResult}
      selectedResultId={null}
      installedEsouiIds={installedIds}
      isOffline={false}
    />
  );
  return onSelectResult;
}

beforeEach(() => {
  invokeOrThrow.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("Discover search results", () => {
  it("does not restore a late result after the query is cleared", async () => {
    let resolveSearch!: (value: unknown) => void;
    invokeOrThrow.mockImplementation(() => new Promise((resolve) => (resolveSearch = resolve)));
    renderDiscover();

    const input = screen.getByRole("textbox", { name: "Search or ask about addons" });
    fireEvent.change(input, { target: { value: "sky" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() =>
      expect(invokeOrThrow).toHaveBeenCalledWith("search_addon_index", { query: "sky" })
    );

    fireEvent.change(input, { target: { value: "" } });
    await act(async () => {
      resolveSearch({ results: [addon], source: "index", hasMore: false });
    });

    expect(screen.queryByRole("button", { name: "Skyshards" })).not.toBeInTheDocument();
    expect(screen.getByText("Search ESOUI")).toBeInTheDocument();
  });

  it("opens a result by keyboard and shows a focusable Install action", async () => {
    invokeOrThrow.mockResolvedValue({ results: [addon], source: "index", hasMore: false });
    const onSelectResult = renderDiscover();
    const input = screen.getByRole("textbox", { name: "Search or ask about addons" });
    fireEvent.change(input, { target: { value: "sky" } });
    fireEvent.keyDown(input, { key: "Enter" });

    const title = await screen.findByRole("button", { name: "Skyshards" });
    title.focus();
    await userEvent.keyboard("{Enter}");
    expect(onSelectResult).toHaveBeenCalledWith(addon);

    const install = screen.getByRole("button", { name: "Install" });
    expect(install).toHaveClass("focus-visible:opacity-100");
    expect(install).not.toBeDisabled();
  });

  it("does not offer installation for an already installed addon", async () => {
    invokeOrThrow.mockResolvedValue({ results: [addon], source: "index", hasMore: false });
    renderDiscover(new Set([addon.id]));
    const input = screen.getByRole("textbox", { name: "Search or ask about addons" });
    fireEvent.change(input, { target: { value: "sky" } });
    fireEvent.keyDown(input, { key: "Enter" });

    const status = await screen.findByRole("button", { name: "Installed" });
    expect(status).toBeDisabled();
    fireEvent.click(status);
    expect(invokeOrThrow).toHaveBeenCalledTimes(1);
  });
});
