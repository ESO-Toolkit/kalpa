import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Pack, PackPage } from "@/types";

type Deferred<T> = {
  promise: Promise<T>;
  resolve: (value: T) => void;
};

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@/lib/store", () => ({
  getSetting: vi.fn().mockResolvedValue([]),
  setSetting: vi.fn().mockResolvedValue(true),
}));
vi.mock("@/lib/dependency-prompt-context", () => ({
  useResolvePendingDeps: () => vi.fn(),
}));
vi.mock("@/lib/eso-running-context", () => ({
  useEnsureEsoNotBlocking: () => vi.fn().mockResolvedValue(true),
}));
vi.mock("@/components/ui/dialog", () => ({
  Dialog: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  DialogContent: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  DialogHeader: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  DialogTitle: ({ children }: { children: React.ReactNode }) => <h1>{children}</h1>,
  DialogFooter: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));
vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => children,
  motion: { div: ({ children }: { children: React.ReactNode }) => <div>{children}</div> },
  useReducedMotion: () => false,
}));
vi.mock("../pack-browse", () => ({ PackListView: () => null }));
vi.mock("../pack-detail", () => ({ PackDetailView: () => null }));
vi.mock("../pack-create", () => ({ PackCreateView: () => null }));
vi.mock("../pack-import", () => ({ PackImportView: () => null }));
vi.mock("../pack-my-packs", () => ({
  MyPacksView: ({
    packs,
    loading,
    loadingMore,
  }: {
    packs: Pack[];
    loading: boolean;
    loadingMore: boolean;
  }) => (
    <div>
      <output data-testid="my-pack-titles">{packs.map((pack) => pack.title).join(",")}</output>
      <output data-testid="my-packs-loading">{String(loading)}</output>
      <output data-testid="my-packs-loading-more">{String(loadingMore)}</output>
    </div>
  ),
}));

import { Packs } from "../packs";

const latePack: Pack = {
  id: "late-private-pack",
  authorId: "42",
  title: "Late private pack",
  description: "must stay cleared after logout",
  packType: "addon-pack",
  authorName: "Ada",
  isAnonymous: false,
  voteCount: 0,
  installCount: 0,
  userVoted: false,
  tags: [],
  addons: [],
  createdAt: "2026-08-26T00:00:00Z",
  updatedAt: "2026-08-26T00:00:00Z",
  status: "published",
};

describe("Pack Hub logout invalidation", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
  });

  it("ignores a private pack load that resolves after logout and settles loading", async () => {
    const privateLoad = deferred<PackPage>();
    mocks.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "list_packs" && args?.author === "42") return privateLoad.promise;
      if (command === "list_packs") return Promise.resolve({ packs: [], page: 1 });
      if (command === "auth_logout") return Promise.resolve();
      throw new Error(`Unexpected command: ${command}`);
    });

    const onAuthChange = vi.fn();
    render(
      <Packs
        addonsPath="C:\\ESO\\AddOns"
        installedAddons={[]}
        authUser={{ userId: "42", userName: "Ada" }}
        onAuthChange={onAuthChange}
        onClose={vi.fn()}
        onRefresh={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "My Packs" }));
    await waitFor(() => expect(screen.getByTestId("my-packs-loading")).toHaveTextContent("true"));

    fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
    await waitFor(() => expect(onAuthChange).toHaveBeenCalledWith(null));
    expect(screen.getByTestId("my-pack-titles")).toBeEmptyDOMElement();
    expect(screen.getByTestId("my-packs-loading")).toHaveTextContent("false");
    expect(screen.getByTestId("my-packs-loading-more")).toHaveTextContent("false");

    await act(async () => privateLoad.resolve({ packs: [latePack], page: 1 }));

    expect(screen.getByTestId("my-pack-titles")).toBeEmptyDOMElement();
    expect(screen.getByTestId("my-packs-loading")).toHaveTextContent("false");
    expect(screen.getByTestId("my-packs-loading-more")).toHaveTextContent("false");
  });
});
