import { describe, it, expect, vi, beforeAll } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DropdownControl } from "../sv-controls";
import type { DropdownOptionItem, EffectiveField } from "../../types";

// ── jsdom browser-API stubs ──────────────────────────────────────────────
// Base UI's Select/Combobox popups rely on layout/observer APIs that jsdom
// does not implement. Stub only what the primitives actually touch at runtime.
beforeAll(() => {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (globalThis as any).ResizeObserver = ResizeObserverStub;

  if (!Element.prototype.scrollIntoView) {
    Element.prototype.scrollIntoView = function scrollIntoView() {};
  }
  if (!Element.prototype.hasPointerCapture) {
    Element.prototype.hasPointerCapture = function hasPointerCapture() {
      return false;
    };
  }
  if (!Element.prototype.setPointerCapture) {
    Element.prototype.setPointerCapture = function setPointerCapture() {};
  }
  if (!Element.prototype.releasePointerCapture) {
    Element.prototype.releasePointerCapture = function releasePointerCapture() {};
  }
  if (!window.matchMedia) {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (window as any).matchMedia = (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    });
  }
});

// ── field factory (adapted from sv-dropdown.test.ts) ─────────────────────
function field(overrides: Partial<EffectiveField>): EffectiveField {
  return {
    nodeId: "TestAddon\0key",
    path: ["TestAddon", "key"],
    key: "key",
    label: "Key",
    widget: "dropdown",
    confidence: "inferred",
    context: "setting",
    props: {},
    hidden: false,
    readOnly: false,
    value: null,
    ...overrides,
  };
}

const FONTS = [
  "Arial",
  "Arial Narrow",
  "Consolas",
  "Courier",
  "Georgia",
  "Helvetica",
  "Impact",
  "Menlo",
  "Monaco",
  "Tahoma",
  "Times New Roman",
  "Verdana",
];

const NUMBERED = [
  "Option One",
  "Option Two",
  "Option Three",
  "Option Four",
  "Option Five",
  "Option Six",
  "Option Seven",
  "Option Eight",
  "Option Nine",
  "Option Ten",
  "Option Eleven",
  "Option Twelve",
];

function numberedItems(): DropdownOptionItem[] {
  return NUMBERED.map((label, i) => ({ label, value: i + 1 }));
}

async function openTrigger(user: ReturnType<typeof userEvent.setup>) {
  const trigger = screen.getByRole("combobox");
  await user.click(trigger);
  return trigger;
}

describe("DropdownControl", () => {
  it("renders a plain Select (no search input) below the search threshold", async () => {
    const user = userEvent.setup();
    render(
      <DropdownControl
        field={field({ props: { options: ["Alpha", "Beta", "Gamma"] }, value: "Alpha" })}
        onChange={vi.fn()}
      />
    );

    const trigger = screen.getByRole("combobox");
    await user.click(trigger);

    await screen.findByRole("option", { name: "Alpha" });
    expect(screen.getByRole("option", { name: "Beta" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Gamma" })).toBeInTheDocument();
    expect(screen.getAllByRole("option")).toHaveLength(3);

    expect(screen.queryByPlaceholderText("Search options…")).not.toBeInTheDocument();
  });

  it("renders a searchable Combobox with a search input above the threshold", async () => {
    const user = userEvent.setup();
    render(
      <DropdownControl
        field={field({ props: { options: FONTS }, value: "Arial" })}
        onChange={vi.fn()}
      />
    );

    await openTrigger(user);

    expect(await screen.findByPlaceholderText("Search options…")).toBeInTheDocument();
    expect(screen.getAllByRole("option")).toHaveLength(12);
  });

  it("filters options case-insensitively as you type and shows a status line", async () => {
    const user = userEvent.setup();
    render(
      <DropdownControl
        field={field({ props: { options: FONTS }, value: "Arial" })}
        onChange={vi.fn()}
      />
    );

    await openTrigger(user);
    const input = await screen.findByPlaceholderText("Search options…");
    await user.type(input, "narrow");

    const options = await screen.findAllByRole("option");
    expect(options).toHaveLength(1);
    expect(options[0]).toHaveTextContent("Arial Narrow");
    expect(screen.getByText("1 of 12 options")).toBeInTheDocument();
  });

  it("shows an empty state when nothing matches", async () => {
    const user = userEvent.setup();
    render(
      <DropdownControl
        field={field({ props: { options: FONTS }, value: "Arial" })}
        onChange={vi.fn()}
      />
    );

    await openTrigger(user);
    const input = await screen.findByPlaceholderText("Search options…");
    await user.type(input, "zzzz");

    expect(await screen.findByText("No matching options")).toBeInTheDocument();
    expect(screen.queryAllByRole("option")).toHaveLength(0);
  });

  it("commits the highlighted match on Enter, preserving the numeric value type", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(
      <DropdownControl
        field={field({ props: { optionItems: numberedItems() }, value: 1 })}
        onChange={onChange}
      />
    );

    await openTrigger(user);
    const input = await screen.findByPlaceholderText("Search options…");
    await user.type(input, "twelve");
    await screen.findByRole("option", { name: "Option Twelve" });
    await user.keyboard("{Enter}");

    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith(12);
  });

  it("commits the raw value when an option is clicked", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(
      <DropdownControl
        field={field({ props: { options: FONTS }, value: "Arial" })}
        onChange={onChange}
      />
    );

    await openTrigger(user);
    const input = await screen.findByPlaceholderText("Search options…");
    await user.type(input, "verdana");
    const option = await screen.findByRole("option", { name: "Verdana" });
    await user.click(option);

    expect(onChange).toHaveBeenCalledWith("Verdana");
  });

  it("disables the trigger and does not open when readOnly", async () => {
    const user = userEvent.setup();
    render(
      <DropdownControl
        field={field({ props: { options: FONTS }, value: "Arial", readOnly: true })}
        onChange={vi.fn()}
      />
    );

    const trigger = screen.getByRole("combobox");
    expect(trigger).toBeDisabled();

    await user.click(trigger);
    expect(screen.queryByPlaceholderText("Search options…")).not.toBeInTheDocument();
  });
});
