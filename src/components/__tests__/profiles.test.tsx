import * as React from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { EsoRunningProvider } from "@/lib/eso-running-context";
import type { AddonProfile, ProfilePlan } from "@/types";
import { Profiles } from "../profiles";

const mocks = vi.hoisted(() => ({
  invokeOrThrow: vi.fn(),
  toast: {
    error: vi.fn(),
    info: vi.fn(),
    success: vi.fn(),
  },
}));

vi.mock("@/lib/tauri", () => ({
  getTauriErrorMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  invokeOrThrow: mocks.invokeOrThrow,
}));

vi.mock("sonner", () => ({
  toast: mocks.toast,
}));

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useInView: () => true,
  useReducedMotion: () => false,
  motion: {
    div: ({
      children,
      initial: _initial,
      animate: _animate,
      exit: _exit,
      transition: _transition,
      onAnimationComplete: _onAnimationComplete,
      ...props
    }: React.HTMLAttributes<HTMLDivElement> & {
      initial?: unknown;
      animate?: unknown;
      exit?: unknown;
      transition?: unknown;
      onAnimationComplete?: unknown;
    }) => <div {...props}>{children}</div>,
  },
}));

const reviewedPlan: ProfilePlan = {
  digest: "reviewed-digest",
  toEnable: ["AddonB"],
  toDisable: ["AddonA"],
  keptDependencies: [],
  missing: [],
  blocked: [],
};

const changedPlan: ProfilePlan = {
  digest: "fresh-digest",
  toEnable: ["AddonB"],
  toDisable: ["AddonA", "AddonC"],
  keptDependencies: [],
  missing: [],
  blocked: [],
};

const profiles: AddonProfile[] = [
  {
    name: "Raid",
    enabledAddons: ["AddonB"],
    createdAt: "2026-08-30T00:00:00Z",
  },
];

function renderProfiles(onRefresh = vi.fn()) {
  render(
    <EsoRunningProvider value={async () => true}>
      <Profiles
        addonsPath="C:/ESO/AddOns"
        instanceLabel={null}
        enabledFolders={["AddonA"]}
        onClose={vi.fn()}
        onRefresh={onRefresh}
      />
    </EsoRunningProvider>
  );
}

describe("Profiles", () => {
  beforeAll(() => {
    Object.defineProperty(HTMLElement.prototype, "getAnimations", {
      configurable: true,
      value: () => [],
    });
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn().mockImplementation((query: string) => ({
        matches: false,
        media: query,
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(() => false),
      })),
    });
  });

  beforeEach(() => {
    mocks.invokeOrThrow.mockReset();
    mocks.toast.error.mockReset();
    mocks.toast.info.mockReset();
    mocks.toast.success.mockReset();
  });

  it("refreshes the preview without applying when activation reports a changed plan", async () => {
    const onRefresh = vi.fn();
    const user = userEvent.setup();

    mocks.invokeOrThrow.mockImplementation((command: string) => {
      if (command === "list_profiles") return Promise.resolve([profiles, null]);
      if (command === "preview_profile") return Promise.resolve(reviewedPlan);
      if (command === "activate_profile") {
        return Promise.resolve({ status: "planChanged", plan: changedPlan });
      }
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    });

    renderProfiles(onRefresh);

    await user.click(await screen.findByRole("button", { name: "Activate" }));
    await screen.findByText("1 addon will be disabled");

    await user.click(screen.getByRole("button", { name: "Activate" }));

    await waitFor(() => {
      expect(mocks.invokeOrThrow).toHaveBeenCalledWith("activate_profile", {
        addonsPath: "C:/ESO/AddOns",
        profileName: "Raid",
        expectedPlanDigest: "reviewed-digest",
      });
    });
    expect(
      await screen.findByText(
        "The plan changed since you reviewed it - review the updated plan and activate again."
      )
    ).toBeInTheDocument();
    expect(screen.getByText("2 addons will be disabled")).toBeInTheDocument();
    expect(screen.getByText("AddonA, AddonC")).toBeInTheDocument();
    expect(onRefresh).not.toHaveBeenCalled();
  });
});
