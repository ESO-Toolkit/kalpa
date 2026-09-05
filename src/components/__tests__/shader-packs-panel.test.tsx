import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ShaderPacksPanel } from "../client-stack/shader-packs-panel";
import type { StackMutationCoordinator } from "../client-stack/panel-props";
import type { ClientStack, ShaderLibrary } from "../client-stack/types";

const { invokeOrThrow } = vi.hoisted(() => ({ invokeOrThrow: vi.fn() }));

vi.mock("@/lib/tauri", () => ({
  getTauriErrorMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  invokeOrThrow,
}));

const clientDir = "C:\\ESO";
const library: ShaderLibrary = {
  client_dir: clientDir,
  shader_tree_present: true,
  packs: [
    {
      id: "test-pack",
      name: "Test Pack",
      author: "Author",
      summary: "A test shader pack",
      licence: "MIT",
      source: {
        kind: "fetchable",
        owner: "owner",
        repo: "repo",
        branch: "main",
      },
      layout: "shaders_and_textures",
      techniques: [],
      markers: ["test.fx"],
      installed: false,
      found: [],
    },
  ],
};

const stack: ClientStack = {
  client_dir: clientDir,
  items: [],
  preserved_originals: [],
  parked: [],
  // Neutral: this fixture is about shader pack listing, not tuning or paths.
  user_parked: [],
  is_disabled: false,
  shaders: { present: true, effect_count: 0, texture_count: 0, effect_search_paths: null },
  preset: null,
  tuning: [],
  tuning_section: null,
  tuning_owner: "unknown",
  tuning_blocks: [],
  disabled_addons: [],
  load_from_dll_main: [],
  active_path: "unknown",
  slots: [],
  is_empty: false,
  findings: [],
};

function coordinator(pending = false): StackMutationCoordinator {
  return {
    pending,
    pendingLabel: pending ? "Another operation" : null,
    run: vi.fn(async (_label, _dir, operation) => ({
      status: "committed" as const,
      value: await operation(),
    })),
  };
}

describe("ShaderPacksPanel mutation coordination", () => {
  beforeEach(() => {
    invokeOrThrow.mockReset();
    invokeOrThrow.mockImplementation((command: string) => {
      if (command === "list_shader_packs") return Promise.resolve(library);
      if (command === "set_game_install_path") return Promise.resolve({ client_dir: clientDir });
      if (command === "install_shader_pack") {
        return Promise.resolve({
          pack_id: "test-pack",
          pack_name: "Test Pack",
          commit: "1234567890abcdef",
          files: ["reshade-shaders/Shaders/test.fx"],
        });
      }
      throw new Error(`Unexpected command: ${command}`);
    });
  });

  it("routes installation through the page coordinator", async () => {
    const user = userEvent.setup();
    const mutation = coordinator();
    render(<ShaderPacksPanel clientDir={clientDir} stack={stack} mutation={mutation} />);

    await user.click(await screen.findByRole("button", { name: "Install" }));
    await user.click(screen.getByRole("button", { name: "Confirm install" }));

    await waitFor(() => expect(mutation.run).toHaveBeenCalledTimes(1));
    expect(mutation.run).toHaveBeenCalledWith(
      "Installing shader pack",
      clientDir,
      expect.any(Function)
    );
    await waitFor(() =>
      expect(invokeOrThrow).toHaveBeenCalledWith("install_shader_pack", {
        clientDir,
        packId: "test-pack",
      })
    );
  });

  it("disables write controls while another mutation is pending", async () => {
    render(<ShaderPacksPanel clientDir={clientDir} stack={stack} mutation={coordinator(true)} />);

    expect(await screen.findByRole("button", { name: "Install" })).toBeDisabled();
  });
});
