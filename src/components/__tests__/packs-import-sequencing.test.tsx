import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { EsoPackFile, SharedPack } from "@/types";

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
  openFile: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: mocks.openFile,
  save: vi.fn(),
}));
vi.mock("@/lib/store", () => ({
  getSetting: vi.fn().mockResolvedValue([]),
  setSetting: vi.fn().mockResolvedValue(undefined),
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
vi.mock("../pack-my-packs", () => ({ MyPacksView: () => null }));
vi.mock("../pack-import", () => ({
  PackImportView: (props: {
    importedPack: SharedPack | null;
    hasSettings: boolean;
    onResolveCode: (code: string) => void;
    onImportFile: () => void;
  }) => (
    <div>
      <output data-testid="import-title">{props.importedPack?.title ?? "none"}</output>
      <output data-testid="has-settings">{String(props.hasSettings)}</output>
      <button onClick={() => props.onResolveCode("ABC123")}>resolve share</button>
      <button onClick={props.onImportFile}>import file</button>
    </div>
  ),
}));

import { Packs } from "../packs";

const sharePack: SharedPack = {
  title: "Share pack",
  description: "from share",
  packType: "addon-pack",
  tags: [],
  addons: [],
  sharedBy: "Share user",
  sharedAt: "2026-08-26T00:00:00Z",
  expiresAt: "2026-09-26T00:00:00Z",
};

const filePack: EsoPackFile = {
  format: "esopack",
  version: 2,
  pack: {
    title: "File pack",
    description: "from file",
    packType: "addon-pack",
    tags: [],
    addons: [],
  },
  sharedBy: "File user",
  sharedAt: "2026-08-26T00:00:00Z",
  settings: {
    FileAddon: {
      encoding: "utf-8",
      lua: "file settings",
      originalBytes: 13,
      scrubbedBytes: 13,
      finalBytes: 13,
      scrubSummary: {
        dropCount: 0,
        templateCount: 0,
        originalBytes: 13,
        scrubbedBytes: 13,
      },
    },
  },
};

function renderPacks() {
  return render(
    <Packs
      addonsPath="C:\\ESO\\AddOns"
      installedAddons={[]}
      authUser={null}
      onAuthChange={vi.fn()}
      onClose={vi.fn()}
      onRefresh={vi.fn()}
    />
  );
}

describe("pack import source sequencing", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.openFile.mockReset();
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_packs") return Promise.resolve({ packs: [], page: 1, sort: "votes" });
      throw new Error(`Unexpected command: ${command}`);
    });
    mocks.openFile.mockResolvedValue("C:\\Temp\\new.esopack");
  });

  it("ignores a late share-code result after a newer file import resolves", async () => {
    const share = deferred<SharedPack>();
    const file = deferred<EsoPackFile>();
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_packs") return Promise.resolve({ packs: [], page: 1, sort: "votes" });
      if (command === "resolve_share_code") return share.promise;
      if (command === "import_pack_file") return file.promise;
      throw new Error(`Unexpected command: ${command}`);
    });
    renderPacks();
    fireEvent.click(screen.getByRole("button", { name: "Import" }));
    fireEvent.click(screen.getByRole("button", { name: "resolve share" }));
    fireEvent.click(screen.getByRole("button", { name: "import file" }));

    await act(async () => file.resolve(filePack));
    expect(screen.getByTestId("import-title")).toHaveTextContent("File pack");
    expect(screen.getByTestId("has-settings")).toHaveTextContent("true");

    await act(async () => share.resolve(sharePack));
    expect(screen.getByTestId("import-title")).toHaveTextContent("File pack");
    expect(screen.getByTestId("has-settings")).toHaveTextContent("true");
  });

  it("ignores a late file result after a newer share-code import resolves", async () => {
    const file = deferred<EsoPackFile>();
    const share = deferred<SharedPack>();
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_packs") return Promise.resolve({ packs: [], page: 1, sort: "votes" });
      if (command === "import_pack_file") return file.promise;
      if (command === "resolve_share_code") return share.promise;
      throw new Error(`Unexpected command: ${command}`);
    });
    renderPacks();
    fireEvent.click(screen.getByRole("button", { name: "Import" }));
    fireEvent.click(screen.getByRole("button", { name: "import file" }));
    fireEvent.click(screen.getByRole("button", { name: "resolve share" }));

    await act(async () => share.resolve(sharePack));
    expect(screen.getByTestId("import-title")).toHaveTextContent("Share pack");
    expect(screen.getByTestId("has-settings")).toHaveTextContent("false");

    await act(async () => file.resolve(filePack));
    expect(screen.getByTestId("import-title")).toHaveTextContent("Share pack");
    expect(screen.getByTestId("has-settings")).toHaveTextContent("false");
  });
});
