import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { SlotRail } from "../slot-rail";
import {
  NEED_META,
  PATH_META,
  SOURCE_META,
  SLOT_ORDER,
  SLOT_SOURCE,
  slotLevel,
  slotNeed,
  slotRailLine,
} from "../slots";
import type { Slot } from "../slots";
import type { ActivePath, ClientStack, SlotNeed, SlotStatus } from "../types";

/**
 * Guards the third axis: whether the live path *wants* what is in a slot.
 *
 * The bug this covers is not a crash and not a wrong string — it is a correct
 * install rendering as a list of holes. On the direct path `renodx-dlss.addon64`
 * hooks the Neural Rendering runtime itself, so motion vectors, the preset and
 * the `[RenoDX.DLSS5]` tuning block are all correctly absent; the panel painted
 * all three as Info-level "nothing here" and then announced that everything
 * agreed. Presence alone cannot tell those two situations apart, so every
 * assertion below is about presence *and* need read together.
 *
 * The tables are asserted total on purpose. A `SlotNeed` with no word renders
 * as no mark at all on the three light and two high-contrast themes, where the
 * status hues collapse and the word is the entire signal.
 */

const NEEDS: SlotNeed[] = ["required", "not_on_this_path", "installed_unused", "unknown"];
const PATHS: ActivePath[] = ["direct", "feed", "both", "neither", "unknown"];

/** A stack with nothing in it, so each test adds only what it is about. */
function emptyStack(overrides: Partial<ClientStack> = {}): ClientStack {
  return {
    client_dir: "C:\\eso\\game\\client",
    items: [],
    preserved_originals: [],
    parked: [],
    user_parked: [],
    is_disabled: false,
    shaders: {
      present: false,
      effect_count: 0,
      texture_count: 0,
      effect_search_paths: null,
    },
    preset: null,
    tuning: [],
    tuning_section: null,
    tuning_owner: "unknown",
    tuning_blocks: [],
    disabled_addons: [],
    load_from_dll_main: [],
    active_path: "direct",
    slots: [],
    is_empty: true,
    findings: [],
    ...overrides,
  };
}

function slotStatuses(entries: Partial<Record<Slot, Partial<SlotStatus>>>): SlotStatus[] {
  return SLOT_ORDER.map((slot) => ({
    slot,
    need: "required" as SlotNeed,
    reason: `${slot} is wanted here.`,
    keep_because: null,
    ...entries[slot],
  }));
}

describe("the need axis", () => {
  it("reads the backend's answer for the slot", () => {
    const stack = emptyStack({
      slots: slotStatuses({ motion: { need: "not_on_this_path" } }),
    });
    expect(slotNeed("motion", stack)).toBe("not_on_this_path");
    expect(slotNeed("reshade", stack)).toBe("required");
  });

  it("falls back to `required`, not `unknown`, when the backend sent no slots", () => {
    // A missing field is a contract violation, not a statement about the
    // client folder. `required` reproduces the panel's behaviour from before
    // this axis existed; `unknown` would have every row claim Kalpa could not
    // read a folder it never tried to read.
    const stack = emptyStack({ slots: [] });
    expect(slotNeed("motion", stack)).toBe("required");
  });
});

describe("an empty slot on the direct path is not a gap", () => {
  const stack = emptyStack({
    active_path: "direct",
    slots: slotStatuses({
      motion: {
        need: "not_on_this_path",
        reason:
          "renodx-dlss.addon64 gets motion vectors from the game through its own hooks. " +
          "Nothing in ReShade has to produce them, so an empty slot here is correct.",
      },
      preset: { need: "not_on_this_path", reason: "The direct path enables no technique." },
    }),
  });

  it("is `ok`, so it draws no Info tint and no Info-level finding", () => {
    expect(slotLevel("motion", stack)).toBe("ok");
    expect(slotLevel("preset", stack)).toBe("ok");
  });

  it("says so affirmatively rather than reporting an absence", () => {
    const line = slotRailLine("motion", stack);
    expect(line.id).toBe("not on this path");
    expect(line.id).not.toBe("nothing here");
    // Prose, not an identifier — mono is reserved for strings that exist in a
    // file, and reading "not on this path" as a filename is the whole error.
    expect(line.prose).toBe(true);
  });

  it("still reports a required empty slot as Info and says nothing is here", () => {
    const required = emptyStack({ slots: slotStatuses({}) });
    expect(slotLevel("motion", required)).toBe("info");
    expect(slotRailLine("motion", required).id).toBe("nothing here");
  });

  it("never renders an unreadable folder as correctly empty", () => {
    // `unknown` asserts nothing about the install. It must keep its Info level
    // and must not borrow the "not on this path" sentence, which is a verdict.
    const unreadable = emptyStack({
      active_path: "unknown",
      slots: slotStatuses({ motion: { need: "unknown", reason: "Kalpa could not read it." } }),
    });
    expect(slotLevel("motion", unreadable)).toBe("info");
    expect(slotRailLine("motion", unreadable).id).toBe("not checked");
  });
});

describe("installed but unused", () => {
  const stack = emptyStack({
    active_path: "direct",
    shaders: { present: true, effect_count: 28, texture_count: 4, effect_search_paths: ".\\" },
    slots: slotStatuses({
      shaders: {
        need: "installed_unused",
        reason: "The direct path runs no ReShade technique.",
        keep_because: "iMMERSE LaunchPad is link-only; Kalpa can never download it for you.",
      },
    }),
  });

  it("is not a fault", () => {
    expect(slotLevel("shaders", stack)).toBe("ok");
  });

  it("says it is unused rather than reporting a count that implies it is working", () => {
    expect(slotRailLine("shaders", stack).meta).toBe("not used here");
  });
});

describe("tuning names the section the backend found", () => {
  it("does not hardcode the parked feed add-on's block", () => {
    const stack = emptyStack({
      tuning: [{ key: "NeuralUplift", value: "0" }],
      tuning_section: "RENODX-DLSS",
      tuning_owner: "live",
      tuning_blocks: [
        {
          section: "RENODX-DLSS",
          owner: "renodx-dlss.addon64",
          provenance: "live",
          values: [{ key: "NeuralUplift", value: "0" }],
        },
        // The feed's block is still carried beside it, and must not become the
        // headline: this is the direct path.
        {
          section: "RenoDX.DLSS5",
          owner: "renodx-dlss5.addon64",
          provenance: "fossil",
          values: [{ key: "NeuralUplift", value: "0" }],
        },
      ],
      slots: slotStatuses({ tuning: { need: "required" } }),
    });
    expect(slotRailLine("tuning", stack).id).toBe("[RENODX-DLSS]");
  });

  it("marks a fossil as not in use rather than as current tuning", () => {
    // The backend falls back to a fossil for the headline only when nothing is
    // live, so this is a direct-path install whose direct add-on has never
    // saved a block. The row still names the section it did find.
    const stack = emptyStack({
      tuning: [{ key: "NeuralUplift", value: "0" }],
      tuning_section: "RenoDX.DLSS5",
      tuning_owner: "fossil",
      tuning_blocks: [
        {
          section: "RenoDX.DLSS5",
          owner: "renodx-dlss5.addon64",
          provenance: "fossil",
          values: [{ key: "NeuralUplift", value: "0" }],
        },
      ],
      slots: slotStatuses({
        tuning: {
          need: "installed_unused",
          reason: "[RenoDX.DLSS5] belongs to renodx-dlss5.addon64, which is not running here.",
          keep_because: "Left exactly as saved.",
        },
      }),
    });
    const line = slotRailLine("tuning", stack);
    expect(line.id).toBe("[RenoDX.DLSS5]");
    expect(line.meta).toBe("not used here");
  });
});

describe("every state has a word, because colour is never the only signal", () => {
  it.each(NEEDS)("%s", (need) => {
    // `required` is the one deliberate null: it is the default reading of a
    // row, and a badge on all eight rows is a badge on none.
    if (need === "required") expect(NEED_META[need].word).toBeNull();
    else expect(NEED_META[need].word?.trim()).toBeTruthy();
  });

  it.each(PATHS)("the %s path is named and summarised", (path) => {
    expect(PATH_META[path].label.trim()).toBeTruthy();
    expect(PATH_META[path].blurb.trim()).toBeTruthy();
    // The rail leaves 190px of text width at 11px. A path summary that ends in
    // an ellipsis has summarised nothing.
    expect(PATH_META[path].blurb.length).toBeLessThanOrEqual(32);
  });
});

describe("the rail", () => {
  const stack = emptyStack({
    active_path: "direct",
    slots: slotStatuses({ motion: { need: "not_on_this_path" } }),
  });

  function renderRail() {
    render(
      <SlotRail
        stack={stack}
        selected="reshade"
        onSelect={() => {}}
        isManaged
        trackedCount={3}
        logCount={0}
        nrState="unknown"
      />
    );
  }

  it("names the active path, so the empty rows below have a reason", () => {
    renderRail();
    expect(screen.getByText("Neural Rendering path")).toBeTruthy();
    expect(screen.getByText(PATH_META.direct.label)).toBeTruthy();
  });

  it("carries provenance as a word, not only as a glyph tint", () => {
    // Elsweyr Moons reseeds `--primary` to within a couple of dE of
    // `--foreground`, so the tint alone marks nothing at all on that theme.
    // `SOURCE_META` has always had the word; nothing displayed it.
    //
    // The rail's single `listbox` (nested listboxes are invalid ARIA) wraps
    // two `role="presentation"` groups: the slots and the whole-stack rows.
    // Querying `[role=option]` off the outer listbox would also catch the
    // whole-stack rows, so this scopes to the slot group specifically —
    // identified by its `flex-1` class (the whole-stack group is `shrink-0`)
    // rather than by DOM order, so it cannot quietly latch onto the wrong
    // group. If the group stops existing the null check fails the test outright.
    renderRail();
    const rail = screen.getByRole("listbox", { name: "Graphics stack views" });
    const slotGroup = rail.querySelector<HTMLElement>('[role="presentation"].flex-1');
    expect(slotGroup).not.toBeNull();
    const rows = within(slotGroup as HTMLElement).getAllByRole("option");
    expect(rows).toHaveLength(SLOT_ORDER.length);
    SLOT_ORDER.forEach((slot, i) => {
      expect(rows[i]?.textContent).toContain(SOURCE_META[SLOT_SOURCE[slot]].word);
    });
  });

  it("announces the need on rows the live path does not want", () => {
    renderRail();
    expect(screen.getAllByText("not on this path").length).toBeGreaterThan(0);
    expect(screen.getAllByText(/Not on this path\./).length).toBeGreaterThan(0);
  });
});
