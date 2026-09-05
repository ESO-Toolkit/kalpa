import * as React from "react";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { SlotRail } from "../client-stack/slot-rail";
import type { StackView } from "../client-stack/slot-rail";
import type { Slot } from "../client-stack/slots";
import type { ClientStack } from "../client-stack/types";

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

function ControlledRail({ value = stack }: { value?: ClientStack }) {
  const [selection, setSelection] = React.useState<Slot | StackView | null>("reshade");
  return (
    <SlotRail
      stack={value}
      selected={selection}
      onSelect={setSelection}
      isManaged={false}
      trackedCount={null}
      logCount={0}
    />
  );
}

describe("Client Health details rail", () => {
  it("uses one tab stop and keeps focus, selection, and aria-selected synchronized", async () => {
    const user = userEvent.setup();
    render(<ControlledRail />);
    const rail = screen.getByRole("listbox", { name: "Graphics stack views" });
    const options = within(rail).getAllByRole("option");

    expect(options.filter((option) => option.tabIndex === 0)).toHaveLength(1);
    const reshade = within(rail).getByRole("option", {
      name: /^ReShadenothing/,
    });
    const addons = within(rail).getByRole("option", { name: /^ReShade add-onsnothing/ });
    expect(reshade).toHaveAttribute("tabindex", "0");
    expect(reshade).toHaveAttribute("aria-selected", "true");
    expect(addons).toHaveAttribute("tabindex", "-1");

    reshade.focus();
    await user.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(addons);
    expect(addons).toHaveAttribute("aria-selected", "true");
    expect(reshade).toHaveAttribute("aria-selected", "false");
    expect(addons).toHaveAttribute("tabindex", "0");

    await user.keyboard("{End}");
    const logs = within(rail).getByRole("option", { name: /Log check/ });
    expect(document.activeElement).toBe(logs);
    expect(logs).toHaveAttribute("aria-selected", "true");

    await user.keyboard("{Home}");
    expect(document.activeElement).toBe(reshade);
    expect(reshade).toHaveAttribute("aria-selected", "true");
  });

  it("supports native Enter and Space activation and preserves focus across refresh", async () => {
    const user = userEvent.setup();
    const view = render(<ControlledRail />);
    const rail = screen.getByRole("listbox", { name: "Graphics stack views" });
    const neural = within(rail).getByRole("option", { name: /Neural Rendering/ });

    expect(neural).toHaveAttribute("tabindex", "-1");
    neural.focus();
    await user.keyboard("{Enter}");
    expect(neural).toHaveAttribute("aria-selected", "true");
    expect(neural).toHaveAttribute("tabindex", "0");

    await user.keyboard(" ");
    expect(neural).toHaveAttribute("aria-selected", "true");

    view.rerender(
      <ControlledRail
        value={{
          ...stack,
          findings: [
            {
              id: "stack-nr-runtime-missing",
              level: "warning",
              title: "Runtime missing",
              detail: "Refresh changed the row's status.",
              guide_url: null,
            },
          ],
        }}
      />
    );

    const refreshedRail = screen.getByRole("listbox", { name: "Graphics stack views" });
    const refreshedNeural = within(refreshedRail).getByRole("option", {
      name: /Neural Rendering/,
    });
    expect(document.activeElement).toBe(refreshedNeural);
    expect(refreshedNeural).toHaveAttribute("aria-selected", "true");
    expect(refreshedNeural).toHaveAttribute("tabindex", "0");
  });
});
