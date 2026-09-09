import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { StackPowerCard } from "../power-card";
import type { StackMutationCoordinator, StackMutationResult } from "../panel-props";
import type { ClientStack, PlannedOp, TogglePlan } from "../types";

/**
 * The confirm button has to be reachable, whatever the plan says.
 *
 * The plan list sits directly above the only control that acts on it, so an
 * unbounded list puts "Confirm switch off" below however many rows the backend
 * chose to emit. beta.23 collapsed shader packs into a single row in Rust, so
 * no plan reaches the cap today — this pins the behaviour so a future plan that
 * does cannot push the button off screen, and so the count stays honest when it
 * happens rather than the extra steps disappearing silently.
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

const STACK = {
  client_dir: "C:\\ESO",
  is_empty: false,
  is_disabled: false,
  parked: [],
} as unknown as ClientStack;

function op(n: number): PlannedOp {
  const name = `file-${String(n).padStart(2, "0")}.dll`;
  return {
    kind: "park",
    file_name: name,
    summary: `Park ${name}`,
    detail: `${name} moves aside into the parked folder.`,
    partner: null,
  };
}

function plan(count: number): TogglePlan {
  return {
    client_dir: "C:\\ESO",
    action: "disable",
    is_disabled: false,
    operations: Array.from({ length: count }, (_, i) => op(i + 1)),
    blockers: [],
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

async function showPlan(operationCount: number) {
  mocks.invokeOrThrow.mockResolvedValue(plan(operationCount));
  render(<StackPowerCard clientDir="C:\\ESO" stack={STACK} mutation={inertMutation()} />);
  await userEvent.click(screen.getByRole("button", { name: /Switch off/ }));
  await screen.findByRole("button", { name: "Confirm switch off" });
}

describe("a long plan does not bury the confirm button", () => {
  beforeEach(() => {
    mocks.invokeOrThrow.mockReset();
  });

  it("renders every row of a plan that fits", async () => {
    await showPlan(3);
    expect(screen.getByText("Park file-01.dll")).toBeInTheDocument();
    expect(screen.getByText("Park file-03.dll")).toBeInTheDocument();
    expect(screen.queryByText(/more step/)).toBeNull();
  });

  it("caps the rows and says how many it did not draw", async () => {
    await showPlan(20);
    expect(screen.getByText("Park file-08.dll")).toBeInTheDocument();
    expect(screen.queryByText("Park file-09.dll")).toBeNull();
    expect(screen.getByText(/and 12 more steps/)).toBeInTheDocument();
  });

  it("keeps the full count on screen, so the summary is not a shorter plan", async () => {
    await showPlan(20);
    expect(screen.getByText(/20 steps in this plan/)).toBeInTheDocument();
  });

  it("counts one hidden step as a step", async () => {
    await showPlan(9);
    expect(screen.getByText(/and 1 more step[^s]/)).toBeInTheDocument();
  });
});
