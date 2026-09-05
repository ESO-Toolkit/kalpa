import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { TuningPanel } from "../tuning-panel";
import type { StackMutationCoordinator, StackMutationResult } from "../panel-props";
import type {
  ActivePath,
  ClientStack,
  TuningField,
  TuningForm,
  TuningProvenance,
  TuningSection,
} from "../types";

/**
 * The panel's whole job is to distinguish configuration **in force** from
 * leftovers belonging to a parked add-on.
 *
 * It once read `[RenoDX.DLSS5]` and nothing else and presented it as *the*
 * tuning. On a direct-path install that section belongs to a renamed-aside
 * add-on, so `NeuralUplift=0` was a fossil rendered as a current setting — and
 * a user and a debugging session both concluded Neural Rendering was off while
 * it was running fine on the other path.
 *
 * So these assert the two halves of the fix that a type cannot enforce: a
 * fossil is labelled and not editable, and it is still *shown* — including its
 * typed controls, disabled, with their values in them. Hiding a fossil would be
 * the opposite failure: the user may switch paths back, and their saved
 * settings are theirs.
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

const NEURAL_UPLIFT: TuningField = {
  key: "NeuralUplift",
  label: "Neural Uplift",
  control: "toggle",
  group: "neural_rendering",
  choices: [],
  decimals: 0,
  help: "",
  current: "0",
  slider_min: null,
  slider_max: null,
};

const NR_INTENSITY: TuningField = {
  key: "NRIntensity",
  label: "Intensity",
  control: "float",
  group: "neural_rendering",
  choices: [],
  decimals: 2,
  help: "",
  current: "0.75",
  slider_min: 0,
  slider_max: 1,
};

function feedSection(provenance: TuningProvenance): TuningSection {
  const writable = provenance === "live";
  return {
    section: "RenoDX.DLSS5",
    path: "feed",
    owner: "renodx-dlss5.addon64",
    present: true,
    provenance,
    writable,
    read_only_reason: writable
      ? ""
      : "[RenoDX.DLSS5] belongs to renodx-dlss5.addon64, which is not loaded in this client folder.",
    fields: [NEURAL_UPLIFT, NR_INTENSITY],
    entries: [],
  };
}

function directSection(provenance: TuningProvenance): TuningSection {
  return {
    section: "RENODX-DLSS",
    path: "direct",
    owner: "renodx-dlss.addon64",
    present: true,
    provenance,
    // Never writable, whatever its provenance: no verified field table exists.
    writable: false,
    read_only_reason:
      "[RENODX-DLSS] is written by renodx-dlss.addon64, which is closed source and whose settings Kalpa has not been able to verify.",
    fields: [],
    entries: [
      { key: "DirectNeuralRenderingEncoding", value: "2" },
      { key: "StreamlineOutputPreset", value: "2" },
    ],
  };
}

function form(active: ActivePath, sections: TuningSection[]): TuningForm {
  return {
    client_dir: "C:/ESO/game/client",
    active_path: active,
    path_evidence: [
      "renodx-dlss.addon64 is present and not disabled.",
      "renodx-dlss5.addon64 is present but renamed aside as renodx-dlss5.addon64.off.",
    ],
    sections,
    apply_note:
      "Applies at next launch, and only with ESO closed: ReShade rewrites ReShade.ini from memory when the game exits.",
  };
}

const STACK = { client_dir: "C:/ESO/game/client" } as ClientStack;

// The panel no longer publishes its own mutation results: the page owns
// installation identity and re-inspects it before a write is considered
// committed. This test double just runs the operation inline and reports it
// committed — none of these tests exercise staleness or busy-state, only
// what provenance the panel renders, so the coordinator itself stays inert.
function inertMutation(): StackMutationCoordinator {
  return {
    pending: false,
    pendingLabel: null,
    async run<T>(
      _label: string,
      _clientDir: string,
      operation: () => Promise<T>
    ): Promise<StackMutationResult<T>> {
      const value = await operation();
      return { status: "committed", value };
    },
  };
}

function renderPanel() {
  return render(
    <TuningPanel clientDir="C:/ESO/game/client" stack={STACK} mutation={inertMutation()} />
  );
}

describe("TuningPanel provenance", () => {
  beforeEach(() => {
    mocks.invokeOrThrow.mockReset();
    mocks.approveClientWrites.mockReset();
  });

  it("labels the feed section a fossil, says why, and still shows its toggle", async () => {
    mocks.invokeOrThrow.mockResolvedValue(
      form("direct", [directSection("live"), feedSection("fossil")])
    );
    renderPanel();

    await screen.findByText("[RenoDX.DLSS5]");

    // Not colour alone: the pill carries the word.
    expect(screen.getByText("Not in force")).toBeInTheDocument();
    // The backend's own sentence, verbatim — never a bare disabled control.
    expect(
      screen.getByText(/belongs to renodx-dlss5\.addon64, which is not loaded/)
    ).toBeInTheDocument();

    // Task 7a: the DLSS 5 toggle was never hidden. It is visible, it carries
    // its stored value, and it cannot be moved, because moving it would do
    // nothing — which is the misdiagnosis this panel exists to prevent.
    const toggle = screen.getByRole("checkbox", { name: /Neural Uplift/ });
    expect(toggle).toBeDisabled();
    expect(toggle).toHaveAttribute("aria-checked", "false");

    // A fossil is never edited and never offered for editing.
    expect(screen.queryByRole("button", { name: "Edit anyway…" })).not.toBeInTheDocument();
  });

  it("offers editing only for the live, writable section", async () => {
    mocks.invokeOrThrow.mockResolvedValue(
      form("feed", [directSection("fossil"), feedSection("live")])
    );
    const user = userEvent.setup();
    renderPanel();

    await screen.findByText("[RenoDX.DLSS5]");
    expect(screen.getByText("In force")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Edit anyway…" }));

    const toggle = screen.getByRole("checkbox", { name: /Neural Uplift/ });
    expect(toggle).toBeEnabled();

    // The timing note is the backend's, shown verbatim: next launch, and only
    // with ESO closed.
    expect(
      screen.getByText(/Applies at next launch, and only with ESO closed/)
    ).toBeInTheDocument();
  });

  it("presents the direct path's keys as undocumented, with no invented labels", async () => {
    mocks.invokeOrThrow.mockResolvedValue(
      form("direct", [directSection("live"), feedSection("fossil")])
    );
    const user = userEvent.setup();
    renderPanel();

    await screen.findByText("[RENODX-DLSS]");

    const disclosure = screen.getByRole("button", { name: /2 undocumented settings/ });
    await user.click(disclosure);

    expect(screen.getByText("DirectNeuralRenderingEncoding")).toBeInTheDocument();
    expect(
      screen.getByText(/closed source, and Kalpa has not been able to verify/)
    ).toBeInTheDocument();
    // The asymmetry is intentional, and the copy has to say so rather than
    // reading as work somebody stopped halfway through.
    expect(screen.getByText(/deliberate rather than unfinished/)).toBeInTheDocument();
  });

  it("shows the evidence behind the verdict rather than asking for trust", async () => {
    mocks.invokeOrThrow.mockResolvedValue(
      form("direct", [directSection("live"), feedSection("fossil")])
    );
    renderPanel();

    const summary = await screen.findByText(/Direct path — renodx-dlss\.addon64 is loaded/);
    expect(summary).toBeInTheDocument();
    expect(
      screen.getByText(
        "renodx-dlss5.addon64 is present but renamed aside as renodx-dlss5.addon64.off."
      )
    ).toBeInTheDocument();
  });

  it("says a missing section means the add-on has never run, rather than offering to write one", async () => {
    const absent: TuningSection = {
      ...feedSection("fossil"),
      present: false,
      fields: [],
      entries: [],
      read_only_reason:
        "ReShade.ini has no [RenoDX.DLSS5] section, so renodx-dlss5.addon64 has never run here.",
    };
    mocks.invokeOrThrow.mockResolvedValue(form("direct", [directSection("live"), absent]));
    renderPanel();

    await waitFor(() =>
      expect(screen.getByText(/has never run in this client folder/)).toBeInTheDocument()
    );
    // Absence is stated, not silently skipped — and there is nothing to edit.
    expect(screen.queryByRole("checkbox", { name: /Neural Uplift/ })).not.toBeInTheDocument();
  });

  it("cannot write while every section is unknown", async () => {
    const unknownDirect = {
      ...directSection("unknown"),
      read_only_reason: "Kalpa could not read.",
    };
    const unknownFeed = {
      ...feedSection("fossil"),
      provenance: "unknown" as TuningProvenance,
      writable: false,
      read_only_reason:
        "Kalpa could not read the client folder, so it cannot tell whether renodx-dlss5.addon64 is loaded.",
    };
    mocks.invokeOrThrow.mockResolvedValue(form("unknown", [unknownDirect, unknownFeed]));
    renderPanel();

    await screen.findByText("[RenoDX.DLSS5]");
    // "Can't tell" is its own answer. It is not "fine" and it is not "broken".
    expect(screen.getAllByText("Can't tell").length).toBeGreaterThan(0);
    expect(screen.queryByRole("button", { name: "Edit anyway…" })).not.toBeInTheDocument();

    const feedCard = screen.getByText("[RenoDX.DLSS5]").closest("div[data-slot]");
    expect(feedCard).not.toBeNull();
    expect(
      within(feedCard as HTMLElement).getByRole("checkbox", { name: /Neural Uplift/ })
    ).toBeDisabled();
  });
});
