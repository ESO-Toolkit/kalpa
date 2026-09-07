import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { TuningPanel } from "../tuning-panel";
import type { StackMutationCoordinator, StackMutationResult } from "../panel-props";
import type { ClientStack, TuningField, TuningForm, TuningSection } from "../types";

/**
 * Every tuning control has to say which field it is.
 *
 * `FieldLabel` renders a `<div>`, not a `<label>`, so nothing associates it
 * with the control beneath it. Only the `toggle` branch escaped that, because
 * it wraps its `Checkbox` in a real `<label>`; the other three of the four
 * `TuningControl` kinds were announced by their value or by nothing at all —
 * "Not set", an unlabelled slider, an unlabelled edit box. This panel's whole
 * point is telling one saved setting from another, which is impossible if the
 * only thing a screen reader hears is the number.
 *
 * The names are asserted while the section is read-only on purpose: a disabled
 * control is still reachable and still read out, and the read-only case is the
 * common one.
 */

const mocks = vi.hoisted(() => ({
  invokeOrThrow: vi.fn(),
  approveClientWrites: vi.fn(),
}));

vi.mock("@/lib/tauri", () => ({
  invokeOrThrow: mocks.invokeOrThrow,
  getTauriErrorMessage: (e: unknown) => String(e),
}));

vi.mock("@/components/client-stack/approve", () => ({
  approveClientWrites: mocks.approveClientWrites,
}));

function field(overrides: Partial<TuningField> & Pick<TuningField, "key" | "label">): TuningField {
  return {
    control: "toggle",
    group: "neural_rendering",
    choices: [],
    decimals: 0,
    help: "",
    current: null,
    slider_min: null,
    slider_max: null,
    ...overrides,
  };
}

const FIELDS: TuningField[] = [
  field({ key: "NeuralUplift", label: "Neural Uplift", control: "toggle", current: "1" }),
  field({
    key: "OutputPreset",
    label: "Output preset",
    control: "choice",
    current: "2",
    choices: [
      { value: 1, label: "Preset A" },
      { value: 2, label: "Preset B" },
    ],
  }),
  field({
    key: "NRIntensity",
    label: "Intensity",
    control: "float",
    decimals: 2,
    current: "0.75",
    slider_min: 0,
    slider_max: 1,
  }),
  field({ key: "ToggleKey", label: "Overlay key", control: "key_code", current: "192" }),
];

const section: TuningSection = {
  section: "RenoDX.DLSS5",
  path: "feed",
  owner: "renodx-dlss5.addon64",
  present: true,
  provenance: "live",
  writable: false,
  read_only_reason: "Read-only for this test; the names still have to be announced.",
  fields: FIELDS,
  entries: [],
};

const form: TuningForm = {
  client_dir: "C:/ESO/game/client",
  active_path: "feed",
  path_evidence: [],
  sections: [section],
  apply_note: "Applies at next launch.",
};

const STACK = { client_dir: "C:/ESO/game/client" } as ClientStack;

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

describe("TuningPanel control names", () => {
  beforeEach(() => {
    mocks.invokeOrThrow.mockReset();
    mocks.invokeOrThrow.mockResolvedValue(form);
  });

  it("names all four control kinds, not just the toggle", async () => {
    render(<TuningPanel clientDir="C:/ESO/game/client" stack={STACK} mutation={inertMutation()} />);
    await screen.findByText("[RenoDX.DLSS5]");

    // The one that already worked, via its wrapping <label>.
    expect(screen.getByRole("checkbox", { name: /Neural Uplift/ })).toBeInTheDocument();
    // Used to be named by `SelectValue`'s content — the current value, or the
    // "Not set" placeholder. Never the field.
    expect(screen.getByRole("combobox", { name: "Output preset" })).toBeInTheDocument();
    // Two controls for one field, so they need distinct names or the same
    // field is announced twice with no way to tell them apart.
    expect(screen.getByRole("slider", { name: "Intensity" })).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Intensity value" })).toBeInTheDocument();
    // The adjacent "key code" <span> is not a label and named nothing.
    expect(screen.getByRole("textbox", { name: "Overlay key key code" })).toBeInTheDocument();
  });
});
