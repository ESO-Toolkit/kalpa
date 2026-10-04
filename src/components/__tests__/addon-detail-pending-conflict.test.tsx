import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AddonManifest, BatchConflictAddon } from "@/types";

import { AddonDetail } from "../addon-detail";

const mocks = vi.hoisted(() => ({
  getDependencyPolicy: vi.fn(),
  getSetting: vi.fn(),
  invokeOrThrow: vi.fn(),
  listen: vi.fn(),
  openUrl: vi.fn(),
  reportDependencyFailures: vi.fn(),
  resolvePendingDeps: vi.fn(),
  ensureEsoNotBlocking: vi.fn(),
}));

vi.mock("@/lib/tauri", () => ({
  getTauriErrorMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  invokeOrThrow: mocks.invokeOrThrow,
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: mocks.openUrl,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    info: vi.fn(),
    success: vi.fn(),
  },
}));

vi.mock("@/lib/store", () => ({
  getSetting: mocks.getSetting,
}));

vi.mock("@/lib/dependency-policy", () => ({
  getDependencyPolicy: mocks.getDependencyPolicy,
}));

vi.mock("@/lib/dependency-failure", () => ({
  reportDependencyFailures: mocks.reportDependencyFailures,
}));

vi.mock("@/lib/dependency-prompt-context", () => ({
  useResolvePendingDeps: () => mocks.resolvePendingDeps,
}));

vi.mock("@/lib/eso-running-context", () => ({
  useEnsureEsoNotBlocking: () => mocks.ensureEsoNotBlocking,
}));

vi.mock("@/components/ui/tabs", () => ({
  Tabs: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  TabsContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  TabsIndicator: () => null,
  TabsList: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  TabsTrigger: ({ children }: { children: ReactNode }) => <button type="button">{children}</button>,
}));

vi.mock("@/components/addon-file-browser", () => ({
  AddonFileBrowser: () => <div data-testid="addon-file-browser" />,
}));

vi.mock("@/components/esoui-tab", () => ({
  EsouiTab: () => <div data-testid="esoui-tab" />,
}));

vi.mock("@/components/changelog-dialog", () => ({
  ChangelogDialog: () => null,
}));

vi.mock("@/components/ui/rich-description", () => ({
  RichDescription: ({
    children,
    description,
    html,
  }: {
    children?: ReactNode;
    description?: string;
    html?: string;
  }) => <p>{description ?? html ?? children}</p>,
}));

vi.mock("@/components/ui/tooltip", () => ({
  SimpleTooltip: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

vi.mock("@/components/animate-ui/primitives/effects/fade", () => ({
  Fade: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

vi.mock("@/components/ui/animated-checkmark", () => ({
  AnimatedCheckmark: () => <span data-testid="animated-checkmark" />,
}));

type MockConflictPanelProps = {
  conflicts: { relativePath: string }[];
  onSkip: () => void;
  sessionId: string;
};

vi.mock("@/components/update-conflict-panel", async () => {
  const React = await vi.importActual<typeof import("react")>("react");

  return {
    UpdateConflictPanel: ({ conflicts, onSkip, sessionId }: MockConflictPanelProps) => {
      const [decision, setDecision] = React.useState("unset");

      return React.createElement(
        "section",
        { "data-testid": "pending-conflict-panel" },
        React.createElement("output", { "data-testid": "pending-conflict-session" }, sessionId),
        React.createElement("output", { "data-testid": "pending-conflict-decision" }, decision),
        React.createElement(
          "button",
          {
            onClick: () => setDecision(conflicts[0]?.relativePath ?? "missing conflict"),
            type: "button",
          },
          "Choose first file"
        ),
        React.createElement(
          "button",
          {
            onClick: onSkip,
            type: "button",
          },
          "Skip Update"
        )
      );
    },
  };
});

const addon: AddonManifest = {
  addonVersion: null,
  apiVersion: [],
  author: "Tester",
  dependsOn: [],
  description: "Addon description",
  disabled: false,
  esouiId: null,
  esouiLastUpdate: 0,
  folderName: "DemoAddon",
  hasProtectedEditsBaseline: true,
  installedAt: "2026-01-01T00:00:00.000Z",
  isLibrary: false,
  missingDependencies: [],
  missingOptionalDependencies: [],
  modifiedFileCount: 0,
  optionalDependsOn: [],
  outdatedDependencies: [],
  tags: [],
  title: "Demo Addon",
  version: "1.0.0",
};

function conflict(sessionId: string, relativePath: string): BatchConflictAddon {
  return {
    autoKeptFiles: [],
    conflicts: [
      {
        relativePath,
        upstreamHash: "upstream",
        userHash: "user",
      },
    ],
    folderName: addon.folderName,
    sessionId,
    updateVersion: "2.0.0",
  };
}

function renderAddonDetail(pendingConflict: BatchConflictAddon) {
  return (
    <AddonDetail
      addon={addon}
      addonsPath="C:/ESO/AddOns"
      installedAddons={[addon]}
      onAddonUpdated={vi.fn()}
      onConflictResolved={vi.fn()}
      onRefresh={vi.fn()}
      onRemoveAddon={vi.fn()}
      onTagsChange={vi.fn()}
      onToggleDisable={vi.fn()}
      pendingConflict={pendingConflict}
      updateResult={null}
    />
  );
}

describe("AddonDetail pending conflict acknowledgments", () => {
  beforeEach(() => {
    mocks.getDependencyPolicy.mockResolvedValue("ask");
    mocks.getSetting.mockResolvedValue("ask");
    mocks.invokeOrThrow.mockReset();
    mocks.listen.mockResolvedValue(() => {});
    mocks.openUrl.mockReset();
    mocks.reportDependencyFailures.mockReset();
    mocks.resolvePendingDeps.mockReset();
    mocks.ensureEsoNotBlocking.mockResolvedValue(true);
  });

  it("scopes pending conflict dismissal and panel decisions to the session id", async () => {
    const user = userEvent.setup();
    const { rerender } = render(renderAddonDetail(conflict("session-a", "DemoAddon/a.lua")));

    expect(await screen.findByTestId("pending-conflict-session")).toHaveTextContent("session-a");

    await user.click(screen.getByRole("button", { name: "Choose first file" }));
    expect(screen.getByTestId("pending-conflict-decision")).toHaveTextContent("DemoAddon/a.lua");

    rerender(renderAddonDetail(conflict("session-b", "DemoAddon/b.lua")));

    await waitFor(() =>
      expect(screen.getByTestId("pending-conflict-session")).toHaveTextContent("session-b")
    );
    expect(screen.getByTestId("pending-conflict-decision")).toHaveTextContent("unset");

    await user.click(screen.getByRole("button", { name: "Skip Update" }));

    await waitFor(() =>
      expect(screen.queryByTestId("pending-conflict-panel")).not.toBeInTheDocument()
    );

    rerender(renderAddonDetail(conflict("session-c", "DemoAddon/c.lua")));

    expect(await screen.findByTestId("pending-conflict-session")).toHaveTextContent("session-c");
  });
});
