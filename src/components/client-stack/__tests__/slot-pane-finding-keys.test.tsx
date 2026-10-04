import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SlotPane } from "../slot-pane";
import type { StackMutationCoordinator, StackMutationResult } from "../panel-props";
import type { ClientStack, HealthFinding } from "../types";

/**
 * Two of the same finding are two rows, not one row rendered twice.
 *
 * `HealthFinding.id` names the *kind* of problem: `build_findings` emits one
 * `stack-addon-disabled` per entry in `DisabledAddons`, and the feed add-on and
 * its host are switched off together, so two findings sharing that id is the
 * ordinary case rather than an edge one. Keying the list on the id alone gave
 * React two children with the same key, which reuses the wrong row's state and
 * DOM when one of the pair clears — and logs an error while doing it.
 */

const PAIR: HealthFinding[] = [
  {
    id: "stack-addon-disabled",
    level: "warning",
    title: "An addon is switched off in ReShade",
    detail:
      "renodx-dlss5.addon64 is listed in DisabledAddons in ReShade.ini, so ReShade will not " +
      "load it even though the file is present.",
    guide_url: null,
  },
  {
    id: "stack-addon-disabled",
    level: "warning",
    title: "An addon is switched off in ReShade",
    detail:
      "dlss5-feed.addon64 is listed in DisabledAddons in ReShade.ini, so ReShade will not " +
      "load it even though the file is present.",
    guide_url: null,
  },
];

function stackWith(findings: HealthFinding[]): ClientStack {
  return {
    client_dir: "C:\\eso\\game\\client",
    items: [],
    preserved_originals: [],
    parked: [],
    user_parked: [],
    is_disabled: false,
    shaders: { present: false, effect_count: 0, texture_count: 0, effect_search_paths: null },
    preset: null,
    tuning: [],
    tuning_section: null,
    tuning_owner: "unknown",
    tuning_blocks: [],
    disabled_addons: ["renodx-dlss5.addon64", "dlss5-feed.addon64"],
    load_from_dll_main: [],
    active_path: "feed",
    slots: [],
    is_empty: false,
    findings,
  };
}

function inertMutation(): StackMutationCoordinator {
  return {
    pending: false,
    pendingLabel: null,
    async run<T>(
      _label: string,
      _clientDir: string,
      operation: () => Promise<T>
    ): Promise<StackMutationResult<T>> {
      return { status: "committed", value: await operation() };
    },
  };
}

function renderAddonsPane(findings: HealthFinding[]) {
  return render(
    <SlotPane
      slot="addons"
      stack={stackWith(findings)}
      mutation={inertMutation()}
      onOpenGuide={() => {}}
    />
  );
}

describe("the findings list keys a row by the finding, not by its kind", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders the disabled feed pair without duplicate React keys", () => {
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    renderAddonsPane(PAIR);

    const duplicateKeyWarnings = errors.mock.calls.filter((call) =>
      call.some((arg) => typeof arg === "string" && arg.includes("same key"))
    );
    expect(duplicateKeyWarnings).toEqual([]);
  });

  it("shows both add-ons, each with its own detail line", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    renderAddonsPane(PAIR);

    expect(screen.getAllByRole("listitem")).toHaveLength(2);
    expect(screen.getByText(/^renodx-dlss5\.addon64 is listed/)).toBeInTheDocument();
    expect(screen.getByText(/^dlss5-feed\.addon64 is listed/)).toBeInTheDocument();
  });

  it("keeps two byte-identical findings apart, since a raw ini list is not deduped", () => {
    // `comma_list` splits `DisabledAddons` and trims; it does not dedupe, so a
    // hand-edited ini can name the same add-on twice. The two rows are then
    // interchangeable, but they must still not collide on one key.
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    renderAddonsPane([PAIR[0]!, PAIR[0]!]);

    const duplicateKeyWarnings = errors.mock.calls.filter((call) =>
      call.some((arg) => typeof arg === "string" && arg.includes("same key"))
    );
    expect(duplicateKeyWarnings).toEqual([]);
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
  });
});
