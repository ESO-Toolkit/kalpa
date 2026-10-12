import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { SnapshotManifest } from "@/types";
import { SafetyCenter } from "./safety-center";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@/lib/tauri", () => ({ invokeOrThrow: invoke, getTauriErrorMessage: String }));
vi.mock("@/components/animate-ui/primitives/effects/fade", () => ({
  Fade: ({ children }: { children: ReactNode }) => children,
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
vi.mock("@/hooks/use-capped-animation-rate", () => ({ useCappedAnimationRate: vi.fn() }));

const snapshot: SnapshotManifest = {
  id: "snapshot-1",
  label: "Before migration",
  createdAt: "2026-10-12",
  sourcePaths: ["SavedVariables"],
  fileCount: 2,
  totalSize: 2048,
  archiveSha256: "test-digest",
};

beforeEach(() => {
  invoke.mockReset();
});
afterEach(cleanup);

describe("Safety Center snapshot integration", () => {
  it("clears a failed listing when a changed addon path loads successfully", async () => {
    invoke.mockImplementation(async (command: string, args: { addonsPath: string }) => {
      if (command !== "list_snapshots") throw new Error(`Unexpected command: ${command}`);
      if (args.addonsPath === "/eso/live/AddOns") throw new Error("Storage unavailable");
      return [snapshot];
    });
    const props = { onClose: vi.fn(), onRefresh: vi.fn() };
    const view = render(<SafetyCenter {...props} addonsPath="/eso/live/AddOns" />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Storage unavailable");

    view.rerender(<SafetyCenter {...props} addonsPath="/eso/liveeu/AddOns" />);
    expect(await screen.findByText(snapshot.label)).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith("list_snapshots", {
      addonsPath: "/eso/liveeu/AddOns",
    });
  });

  it("reveals snapshot storage through the native allowed-path command", async () => {
    const user = userEvent.setup();
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_snapshots") return [snapshot];
      if (command === "get_backups_folder_path") return "/eso/live/KalpaBackups";
      if (command === "reveal_allowed_path") return undefined;
      throw new Error(`Unexpected command: ${command}`);
    });
    render(<SafetyCenter addonsPath="/eso/live/AddOns" onClose={vi.fn()} onRefresh={vi.fn()} />);
    await user.click(await screen.findByRole("button", { name: "Show in folder" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("reveal_allowed_path", {
        path: "/eso/live/KalpaBackups",
      })
    );
    expect(invoke).toHaveBeenCalledWith("get_backups_folder_path", {
      addonsPath: "/eso/live/AddOns",
    });
  });
});
