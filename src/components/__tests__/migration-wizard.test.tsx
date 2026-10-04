import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import type { DryRunResult } from "@/types";
import { MigrationWizard } from "../migration-wizard";

const mocks = vi.hoisted(() => ({
  invokeOrThrow: vi.fn(),
}));

vi.mock("@/lib/tauri", () => ({
  getTauriErrorMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  invokeOrThrow: mocks.invokeOrThrow,
}));

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    info: vi.fn(),
    success: vi.fn(),
  },
}));

function dryRunResult(overrides: Partial<DryRunResult> = {}): DryRunResult {
  return {
    planDigest: "a".repeat(64),
    willTrack: [{ folderName: "AddonA", esouiId: 10, minionVersion: "1.0", status: "will_track" }],
    alreadyTracked: [],
    missingOnDisk: [],
    unmanagedOnDisk: [],
    ...overrides,
  };
}

describe("MigrationWizard plan binding", () => {
  beforeAll(() => {
    Object.defineProperty(Element.prototype, "getAnimations", {
      configurable: true,
      value: () => [],
    });
    class IntersectionObserverStub {
      observe = vi.fn();
      unobserve = vi.fn();
      disconnect = vi.fn();
      takeRecords = vi.fn(() => []);
    }
    Object.defineProperty(globalThis, "IntersectionObserver", {
      configurable: true,
      writable: true,
      value: IntersectionObserverStub,
    });
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      writable: true,
      value: (query: string) =>
        ({
          matches: false,
          media: query,
          onchange: null,
          addListener: vi.fn(),
          removeListener: vi.fn(),
          addEventListener: vi.fn(),
          removeEventListener: vi.fn(),
          dispatchEvent: vi.fn(() => false),
        }) as MediaQueryList,
    });
  });

  beforeEach(() => {
    mocks.invokeOrThrow.mockReset();
  });

  it("passes the reviewed digest and re-previews when the plan changed", async () => {
    const user = userEvent.setup();
    const reviewed = dryRunResult();
    const fresh = dryRunResult({
      planDigest: "b".repeat(64),
      willTrack: [
        { folderName: "AddonA", esouiId: 10, minionVersion: "1.0", status: "will_track" },
        { folderName: "AddonB", esouiId: 20, minionVersion: "2.0", status: "will_track" },
      ],
    });
    const executeCalls: unknown[] = [];

    mocks.invokeOrThrow.mockImplementation((command: string, args?: unknown) => {
      switch (command) {
        case "migration_check_preconditions":
          return Promise.resolve({
            minionFound: true,
            addonsPathValid: true,
            savedVariablesExists: true,
            esoRunning: false,
            minionRunning: false,
            warnings: [],
          });
        case "backup_minion_config":
          return Promise.resolve(1);
        case "migration_create_snapshot":
          return Promise.resolve({ fileCount: 3, totalSize: 1024 });
        case "migration_dry_run":
          return Promise.resolve(reviewed);
        case "migration_execute":
          executeCalls.push(args);
          return Promise.resolve({
            status: "planChanged",
            ...{
              expectedDigest: reviewed.planDigest,
              actualDigest: fresh.planDigest,
              freshPlan: fresh,
            },
          });
        default:
          throw new Error(`Unexpected command: ${command}`);
      }
    });

    render(<MigrationWizard addonsPath="C:/ESO/AddOns" onClose={vi.fn()} onRefresh={vi.fn()} />);

    await user.click(await screen.findByRole("button", { name: "Continue to Backup" }));
    await user.click(await screen.findByRole("button", { name: "Create Snapshot & Continue" }));

    // Confirm phase for the reviewed plan.
    expect(await screen.findByText(/Will be tracked in Kalpa \(1\)/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Import Metadata Now" }));

    // The reviewed digest rode along with the execute call.
    await waitFor(() => expect(executeCalls).toHaveLength(1));
    expect(executeCalls[0]).toMatchObject({ expectedPlanDigest: reviewed.planDigest });

    // planChanged: no completion — back to confirm with the FRESH plan and a
    // visible explanation, requiring a second explicit confirmation.
    expect(await screen.findByText(/Migration plan changed/)).toBeInTheDocument();
    expect(screen.getByText(/Will be tracked in Kalpa \(2\)/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Import Metadata Now" })).toBeInTheDocument();
    expect(screen.queryByText(/Migration complete/i)).not.toBeInTheDocument();
  });
});
