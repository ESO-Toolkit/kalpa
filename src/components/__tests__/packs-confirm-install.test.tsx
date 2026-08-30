import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Pack } from "@/types";

import { Packs } from "../packs";

const mocks = vi.hoisted(() => ({
  getSetting: vi.fn(),
  invoke: vi.fn(),
  resolvePendingDeps: vi.fn(),
  setSetting: vi.fn(),
  ensureEsoNotBlocking: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
  save: vi.fn(),
}));

vi.mock("@/lib/store", () => ({
  getSetting: mocks.getSetting,
  setSetting: mocks.setSetting,
}));

vi.mock("@/lib/dependency-prompt-context", () => ({
  useResolvePendingDeps: () => mocks.resolvePendingDeps,
}));

vi.mock("@/lib/eso-running-context", () => ({
  useEnsureEsoNotBlocking: () => mocks.ensureEsoNotBlocking,
}));

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    info: vi.fn(),
    success: vi.fn(),
  },
}));

vi.mock("@/components/ui/dialog", () => ({
  Dialog: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DialogContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DialogDescription: ({ children }: { children: ReactNode }) => <p>{children}</p>,
  DialogFooter: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DialogHeader: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DialogTitle: ({ children }: { children: ReactNode }) => <h1>{children}</h1>,
}));

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: ReactNode }) => <>{children}</>,
  motion: {
    div: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  },
  useReducedMotion: () => false,
}));

vi.mock("../pack-browse", () => ({
  PackListView: ({ onSelectPack }: { onSelectPack: (packId: string) => void }) => (
    <button onClick={() => onSelectPack("pack-a")} type="button">
      Open alpha pack
    </button>
  ),
}));

vi.mock("../pack-detail", () => ({
  PackDetailView: ({ pack }: { pack: Pack | null }) => (
    <output data-testid="selected-pack">{pack?.title ?? "none"}</output>
  ),
}));

vi.mock("../pack-create", () => ({
  PackCreateView: () => null,
}));

vi.mock("../pack-my-packs", () => ({
  MyPacksView: () => null,
}));

vi.mock("../pack-import", () => ({
  PackImportView: () => null,
}));

function pack(id: string, title: string, esouiId: number): Pack {
  return {
    addons: [
      {
        esouiId,
        name: `${title} Addon`,
        required: true,
      },
    ],
    authorId: "author-1",
    authorName: "Pack Author",
    createdAt: "2026-01-01T00:00:00.000Z",
    description: `${title} description`,
    id,
    installCount: 0,
    isAnonymous: false,
    packType: "addon-pack",
    status: "published",
    tags: [],
    title,
    updatedAt: "2026-01-01T00:00:00.000Z",
    userVoted: false,
    voteCount: 0,
  };
}

describe("Packs install confirmation", () => {
  beforeEach(() => {
    const alphaPack = pack("pack-a", "Alpha Pack", 1001);
    const betaPack = pack("pack-b", "Beta Pack", 2001);

    mocks.getSetting.mockImplementation((_key: string, defaultValue: unknown) =>
      Promise.resolve(defaultValue)
    );
    mocks.invoke.mockImplementation((command: string, args?: { id?: string }) => {
      if (command === "list_packs") {
        return Promise.resolve({ packs: [], page: 1, sort: "votes" });
      }
      if (command === "get_pack") {
        return Promise.resolve(args?.id === "pack-b" ? betaPack : alphaPack);
      }
      throw new Error(`Unexpected command: ${command}`);
    });
    mocks.resolvePendingDeps.mockReset();
    mocks.setSetting.mockResolvedValue(undefined);
    mocks.ensureEsoNotBlocking.mockResolvedValue(true);
  });

  it("disarms a pending install confirmation when a deep link selects another pack", async () => {
    const user = userEvent.setup();
    const baseProps = {
      addonsPath: "C:/ESO/AddOns",
      authUser: null,
      installedAddons: [],
      onAuthChange: vi.fn(),
      onClose: vi.fn(),
      onRefresh: vi.fn(),
    };

    const { rerender } = render(<Packs {...baseProps} initialPackId="pack-a" />);

    await waitFor(() =>
      expect(screen.getByTestId("selected-pack")).toHaveTextContent("Alpha Pack")
    );
    await user.click(await screen.findByRole("button", { name: "Install 1 New Addon" }));
    expect(screen.getByText("Install 1 new addon?")).toBeInTheDocument();

    rerender(<Packs {...baseProps} initialPackId="pack-b" />);

    await waitFor(() => expect(screen.getByTestId("selected-pack")).toHaveTextContent("Beta Pack"));
    await waitFor(() => expect(screen.queryByText("Install 1 new addon?")).not.toBeInTheDocument());
    expect(await screen.findByRole("button", { name: "Install 1 New Addon" })).toBeInTheDocument();
  });
});
