import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { SlotPane } from "../slot-pane";
import type { StackMutationCoordinator, StackMutationResult } from "../panel-props";
import type { ClientStack, RuntimeReport } from "../types";

/**
 * Which runtimes the drift card is allowed to talk about.
 *
 * Two halves of the same mistake. The card used to be scoped by *file name*,
 * built from `stack.items` — which `client_stack::probe` fills only with files
 * that exist. A runtime the manifest lists and the folder no longer has was
 * therefore filtered out of the card by the very fact that made it worth
 * reporting, so `DriftState::Missing` could never render. And the card was
 * mounted only for the `nr` and `sr` slots, so `d3dcompiler_47.dll` — which
 * `client_runtime.rs` explicitly treats as drift-prone and `ROLE_TO_SLOT`
 * files under ReShade — was computed for a surface that did not exist.
 *
 * Scoping by role fixes both, and it is the same table the backend used to
 * decide the role in the first place.
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

// Not under test here, and both fetch on mount.
vi.mock("../preset-panel", () => ({ PresetPanel: () => null }));
vi.mock("../tuning-panel", () => ({ TuningPanel: () => null }));
vi.mock("../shader-packs-panel", () => ({ ShaderPacksPanel: () => null }));

/** No `d3dcompiler_47.dll` and no `nvngx_dlss.dll` in `items`: that is the
 *  point. `probe()` skips anything not on disk, and the two states worth
 *  reporting here are exactly "gone" and "put back by an update". */
const stack: ClientStack = {
  client_dir: "C:\\ESO",
  items: [
    {
      role: "injector",
      file_name: "dxgi.dll",
      display_name: "ReShade",
      version: null,
      company: null,
      description: null,
      size_bytes: 1,
    },
  ],
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
  disabled_addons: [],
  load_from_dll_main: [],
  active_path: "direct",
  slots: [],
  is_empty: false,
  findings: [],
};

const report: RuntimeReport = {
  client_dir: "C:\\ESO",
  runtimes: [
    {
      relative_path: "d3dcompiler_47.dll",
      role: "shader_compiler",
      state: "drifted_recoverable",
      current_version: "6.3.9600",
      kept_version: "10.0.19041",
      kept_backup_id: "abc",
      size_bytes: 1,
      displaced_in_place: null,
    },
    {
      relative_path: "nvngx_dlss.dll",
      role: "super_sampling",
      state: "missing",
      current_version: null,
      kept_version: "3.7.0",
      kept_backup_id: null,
      size_bytes: 0,
      displaced_in_place: null,
    },
  ],
  recoverable: ["d3dcompiler_47.dll"],
  unrecoverable: [],
};

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

function renderSlot(slot: "reshade" | "sr" | "nr") {
  return render(
    <SlotPane slot={slot} stack={stack} mutation={inertMutation()} onOpenGuide={vi.fn()} />
  );
}

describe("Runtime drift scope", () => {
  beforeEach(() => {
    mocks.invokeOrThrow.mockReset();
    mocks.invokeOrThrow.mockResolvedValue(report);
  });

  it("reports the shader compiler on the ReShade slot", async () => {
    renderSlot("reshade");

    expect(await screen.findByText("d3dcompiler_47.dll")).toBeInTheDocument();
    // Scoped by role, so the DLSS runtime stays on its own slot.
    expect(screen.queryByText("nvngx_dlss.dll")).toBeNull();
  });

  it("renders a runtime that is missing from the folder", async () => {
    renderSlot("sr");

    // `stack.items` cannot contain this file — it is not on disk — so the old
    // file-name filter dropped the one state it was written to report.
    expect(await screen.findByText("nvngx_dlss.dll")).toBeInTheDocument();
    expect(screen.getByText("This file is not in the folder at all.")).toBeInTheDocument();
    expect(screen.queryByText("d3dcompiler_47.dll")).toBeNull();
  });

  it("says nothing on a slot with no drifted runtime", async () => {
    renderSlot("nr");

    await screen.findAllByText(/Neural Rendering/);
    expect(screen.queryByText("Runtime drift")).toBeNull();
  });
});
