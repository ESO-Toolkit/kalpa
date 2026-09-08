import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { PresetPanel } from "../preset-panel";
import type { StackMutationCoordinator, StackMutationResult } from "../panel-props";
import type { ClientStack, PresetOptions } from "../types";

/**
 * A confirm belongs to the thing that was on screen when it was armed.
 *
 * The reset effect keys on `load`, and `load` only changes with `clientDir`, so
 * switching preset *inside the same folder* never re-ran it. `load()` then
 * recomputed `options.fix` for the new preset, and `FixOrderCard` re-rendered
 * already armed — one click from writing a reorder into a preset the user never
 * armed against. The copy is regenerated from the new summary, so the sentence
 * on screen stays true; what is lost is the arming click itself.
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

const STACK = { client_dir: "C:\\ESO" } as ClientStack;

function options(active: string, fixSummary: string): PresetOptions {
  return {
    client_dir: "C:\\ESO",
    active,
    choices: [
      {
        relative_path: "Alpha.ini",
        preset_path: "C:\\ESO\\Alpha.ini",
        is_active: active === "Alpha.ini",
        technique_count: 3,
      },
      {
        relative_path: "Beta.ini",
        preset_path: "C:\\ESO\\Beta.ini",
        is_active: active === "Beta.ini",
        technique_count: 4,
      },
    ],
    fix: {
      provider_technique: "Provider.fx",
      feed_technique: "Feed.fx",
      before: "Feed.fx,Provider.fx",
      after: "Provider.fx,Feed.fx",
      sorting_after: "Provider.fx,Feed.fx",
      summary: fixSummary,
    },
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

describe("PresetPanel confirm latches", () => {
  beforeEach(() => {
    mocks.invokeOrThrow.mockReset();
    mocks.approveClientWrites.mockReset();
  });

  it("disarms the technique-order confirm across a preset switch", async () => {
    let active = "Alpha.ini";
    mocks.invokeOrThrow.mockImplementation(async (command: string) => {
      if (command === "list_client_presets") {
        return options(active, `Reorders ${active} so Provider.fx runs first.`);
      }
      if (command === "set_client_preset") {
        active = "Beta.ini";
        return { relative_path: "Beta.ini", backup_id: null, summary: "Switched to Beta.ini." };
      }
      throw new Error(`Unexpected IPC command: ${command}`);
    });
    const user = userEvent.setup();
    render(<PresetPanel clientDir="C:\\ESO" stack={STACK} mutation={inertMutation()} />);

    // Arm the order fix against Alpha.ini.
    await user.click(await screen.findByRole("button", { name: "Fix technique order" }));
    expect(await screen.findByRole("button", { name: "Confirm fix" })).toBeInTheDocument();

    // Switch preset — same folder, so the mount reset effect does not re-run.
    await user.click(screen.getByText("Beta.ini"));
    await user.click(await screen.findByRole("button", { name: "Confirm switch" }));

    // The reload has landed: `load()` re-read the list for the new preset.
    await screen.findByText("Switched to Beta.ini.");

    // Before this fix the confirm was still armed here -- one click from
    // reordering a preset the user never armed against.
    expect(screen.queryByRole("button", { name: "Confirm fix" })).toBeNull();

    // The card itself is still offered, and arming it again names the new
    // preset -- so this disarms the latch rather than hiding the feature.
    await user.click(screen.getByRole("button", { name: "Fix technique order" }));
    expect(await screen.findByText(/Reorders Beta\.ini/)).toBeInTheDocument();
  });
});
