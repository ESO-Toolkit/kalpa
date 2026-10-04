import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ShaderPacksPanel } from "../shader-packs-panel";
import type { StackMutationCoordinator, StackMutationResult } from "../panel-props";
import type { ClientStack, PackStatus, ShaderLibrary } from "../types";

/**
 * "Installed" here means less than the word does, and the panel has to say so.
 *
 * `PackStatus.installed` is marker-file presence and nothing else: the commit
 * `install_shader_pack` pinned is reported once in the post-install line and
 * never persisted, so there is no revision to compare and no update or repair
 * control to offer. Left unqualified, a green "Installed" implies Kalpa is
 * keeping the pack current — the same gap the `link_only` rows already close
 * by printing `source.reason` verbatim.
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

function pack(overrides: Partial<PackStatus> & Pick<PackStatus, "id" | "name">): PackStatus {
  return {
    author: "Kalpa QA",
    summary: "Fixture pack",
    licence: "MIT",
    source: { kind: "fetchable", owner: "owner", repo: "repo", branch: "main" },
    layout: "shaders_and_textures",
    techniques: [],
    markers: [],
    installed: false,
    found: [],
    ...overrides,
  };
}

function library(packs: PackStatus[]): ShaderLibrary {
  return { client_dir: "C:\\ESO", shader_tree_present: true, packs };
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

describe("ShaderPacksPanel", () => {
  beforeEach(() => {
    mocks.invokeOrThrow.mockReset();
  });

  it("qualifies what Installed means, and offers no update control", async () => {
    mocks.invokeOrThrow.mockResolvedValue(
      library([pack({ id: "a", name: "Alpha FX", installed: true, found: ["Alpha.fx"] })])
    );
    render(<ShaderPacksPanel clientDir="C:\\ESO" stack={STACK} mutation={inertMutation()} />);

    await screen.findByText("Alpha FX");
    expect(
      await screen.findByText(/Kalpa does not\s+record which revision that is/)
    ).toBeInTheDocument();
    // Nothing here can update or repair, so nothing must look like it can.
    expect(screen.queryByRole("button", { name: /Update/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /Repair/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "Install" })).toBeNull();
  });

  it("says nothing about revisions when no pack is installed", async () => {
    mocks.invokeOrThrow.mockResolvedValue(library([pack({ id: "a", name: "Alpha FX" })]));
    render(<ShaderPacksPanel clientDir="C:\\ESO" stack={STACK} mutation={inertMutation()} />);

    await screen.findByText("Alpha FX");
    expect(screen.queryByText(/record which revision/)).toBeNull();
    expect(screen.getByRole("button", { name: "Install" })).toBeInTheDocument();
  });
});
