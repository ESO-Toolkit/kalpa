import * as React from "react";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, describe, expect, it, vi } from "vitest";

import { StackBody, type ClientStack } from "../client-health";

vi.mock("@/components/client-stack/power-card", () => ({ StackPowerCard: () => null }));
vi.mock("@/components/client-stack/preset-panel", () => ({ PresetPanel: () => null }));
vi.mock("@/components/client-stack/tuning-panel", () => ({ TuningPanel: () => null }));
vi.mock("@/components/client-stack/runtime-drift-card", () => ({ RuntimeDriftCard: () => null }));

const stack: ClientStack = {
  client_dir: "C:\\ESO",
  items: [],
  preserved_originals: [],
  parked: [],
  is_disabled: false,
  shaders: { present: false, effect_count: 0, texture_count: 0, effect_search_paths: null },
  preset: null,
  tuning: [],
  disabled_addons: [],
  is_empty: false,
  findings: [],
};

const plan = {
  client_dir: stack.client_dir,
  entries: [],
  copy_bytes: 0,
  already_managed: true,
  is_empty: false,
  stack_switched_off: false,
} satisfies React.ComponentProps<typeof StackBody>["plan"];

type StackBodyProps = React.ComponentProps<typeof StackBody>;

function ControlledRail({ showLogs = true }: { showLogs?: boolean }) {
  const [selection, setSelection] =
    React.useState<StackBodyProps["effectiveSelection"]>("injector");
  const props: StackBodyProps = {
    stack,
    plan,
    planLoading: false,
    planError: null,
    effectiveSelection: selection,
    onSelect: setSelection,
    isHealthyManaged: true,
    keepCopies: false,
    onToggleKeepCopies: vi.fn(),
    adopting: false,
    adoptError: null,
    adoptOutcome: null,
    onAdopt: vi.fn(),
    onDismissAdoption: vi.fn(),
    managedLoading: false,
    managedError: null,
    managedInventory: { client_dir: stack.client_dir, files: [], orphan_injectors: [] },
    logExcerpts: showLogs ? [{ file: "ReShade.log", rule: "test", line: "test" }] : [],
    hideEmergency: false,
    isManaged: true,
    forgetConfirming: false,
    forgetting: false,
    forgetError: null,
    onRequestForget: vi.fn(),
    onCancelForget: vi.fn(),
    onConfirmForget: vi.fn(),
    selectedPaths: new Set(),
    onToggleSelect: vi.fn(),
    removeMode: null,
    removing: false,
    removeOutcome: null,
    removeError: null,
    onRequestRemove: vi.fn(),
    onCancelRemove: vi.fn(),
    onConfirmRemove: vi.fn(),
    emergencyOpen: false,
    onToggleEmergencyOpen: vi.fn(),
    emergencyTarget: null,
    onSetEmergencyTarget: vi.fn(),
    emergencyConfirmInput: "",
    onEmergencyConfirmInputChange: vi.fn(),
    emergencyBusy: false,
    emergencyError: null,
    emergencyResult: null,
    onEmergencyRemove: vi.fn(),
    onStackChanged: vi.fn(async () => {}),
  };
  return <StackBody {...props} />;
}

async function openDetails(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("tab", { name: "Details" }));
  return screen.getByRole("listbox", { name: "Stack layers" });
}

describe("Client Health details rail", () => {
  beforeAll(() => {
    HTMLElement.prototype.scrollTo = vi.fn();
  });

  it("uses one tab stop and keeps focus, selection, and aria-selected synchronized", async () => {
    const user = userEvent.setup();
    render(<ControlledRail />);
    const rail = await openDetails(user);
    const options = within(rail).getAllByRole("option");

    expect(options.filter((option) => option.tabIndex === 0)).toHaveLength(1);
    const injector = within(rail).getByRole("option", { name: /Injector/ });
    const neural = within(rail).getByRole("option", { name: /Neural Rendering runtime/ });
    expect(injector).toHaveAttribute("tabindex", "0");
    expect(injector).toHaveAttribute("aria-selected", "true");
    expect(neural).toHaveAttribute("tabindex", "-1");

    injector.focus();
    await user.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(neural);
    expect(neural).toHaveAttribute("aria-selected", "true");
    expect(injector).toHaveAttribute("aria-selected", "false");
    expect(neural).toHaveAttribute("tabindex", "0");

    await user.keyboard("{End}");
    expect(document.activeElement).toBe(within(rail).getByRole("option", { name: /Log signals/ }));
    expect(within(rail).getByRole("option", { name: /Log signals/ })).toHaveAttribute(
      "aria-selected",
      "true"
    );

    await user.keyboard("{Home}");
    expect(document.activeElement).toBe(within(rail).getByRole("option", { name: /Injector/ }));
  });

  it("activates the focused option with Enter or Space and restores focus after removal", async () => {
    const user = userEvent.setup();
    const view = render(<ControlledRail />);
    const rail = await openDetails(user);
    const injector = within(rail).getByRole("option", { name: /Injector/ });
    const records = within(rail).getByRole("option", { name: /Kalpa's records/ });

    records.focus();
    await user.keyboard("{Enter}");
    expect(records).toHaveAttribute("aria-selected", "true");
    await user.keyboard(" ");
    expect(records).toHaveAttribute("aria-selected", "true");

    const logs = within(rail).getByRole("option", { name: /Log signals/ });
    await user.click(logs);
    logs.focus();
    view.rerender(<ControlledRail showLogs={false} />);

    await waitFor(() => {
      expect(document.activeElement).toBe(injector);
      expect(
        within(screen.getByRole("listbox", { name: "Stack layers" })).queryByRole("option", {
          name: /Log signals/,
        })
      ).toBeNull();
    });
    expect(injector).toHaveAttribute("aria-selected", "true");
    expect(injector).toHaveAttribute("tabindex", "0");
  });
});
