import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { UpdateConflictPanel } from "../update-conflict-panel";

const mocks = vi.hoisted(() => ({
  invokeOrThrow: vi.fn(),
}));

vi.mock("@/lib/tauri", () => ({
  invokeOrThrow: mocks.invokeOrThrow,
}));

vi.mock("@/components/diff-viewer", () => ({
  DiffViewer: () => <div>diff viewer</div>,
}));

describe("UpdateConflictPanel", () => {
  beforeEach(() => {
    mocks.invokeOrThrow.mockReset();
    mocks.invokeOrThrow.mockResolvedValue({
      userContent: "user",
      upstreamContent: "upstream",
      isBinary: false,
    });
  });

  it("keeps sibling decisions distinct, omits auto-kept files, and diffs by qualified path", async () => {
    const onResolve = vi.fn();
    const user = userEvent.setup();

    render(
      <UpdateConflictPanel
        folderName="Main"
        currentVersion="1.0.0"
        updateVersion="2.0.0"
        conflicts={[
          { relativePath: "Main/init.lua", userHash: "main-user", upstreamHash: "main-new" },
          {
            relativePath: "LibFoo/init.lua",
            userHash: "lib-user",
            upstreamHash: "lib-new",
          },
        ]}
        autoKeptFiles={["LibFoo/user-only.lua"]}
        safeFileCount={0}
        sessionId="session-123"
        addonsPath="C:/ESO/AddOns"
        onResolve={onResolve}
        onSkip={vi.fn()}
      />
    );

    const keepButtons = screen.getAllByRole("button", { name: "Keep my version" });
    const takeButtons = screen.getAllByRole("button", { name: "Take the update" });
    const diffButtons = screen.getAllByRole("button", { name: "View differences" });
    expect(keepButtons).toHaveLength(2);
    expect(takeButtons).toHaveLength(2);
    expect(diffButtons).toHaveLength(2);

    await user.click(keepButtons[0]!);
    await user.click(takeButtons[1]!);
    await user.click(diffButtons[1]!);

    await waitFor(() => {
      expect(mocks.invokeOrThrow).toHaveBeenCalledWith("get_conflict_diff", {
        addonsPath: "C:/ESO/AddOns",
        sessionId: "session-123",
        relativePath: "LibFoo/init.lua",
      });
    });

    await user.click(screen.getByRole("button", { name: "Apply & Update" }));

    expect(onResolve).toHaveBeenCalledWith([
      { relativePath: "Main/init.lua", action: "keep_mine" },
      { relativePath: "LibFoo/init.lua", action: "take_update" },
    ]);
  });
});
