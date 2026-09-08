import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { ClientHealthPanel } from "../client-health";
import type {
  AdoptionPlan,
  ClientHealthReport,
  ClientStack,
  EsoClientLocation,
  ManagedInventory,
  TogglePlan,
} from "../client-health";

/**
 * What "Kalpa's records" is allowed to say about a file Kalpa did not write.
 *
 * `ManagedFileStatus.origin` is the only thing that separates the two. An
 * untouched adopted file hashes clean and reports `present`, exactly like a
 * placed one, so on state alone the panel called the user's own DLL "Unchanged
 * since Kalpa wrote it. Safe to remove.", counted it into "Remove all", and
 * then reported the backend's refusal — `revert_placements` skips every
 * adopted entry — as "modified since Kalpa wrote them". Three false statements
 * about one file, the last of them an accusation.
 *
 * `restores_backup` is deliberately *not* the discriminator and these tests
 * pin that: for an adopted entry it is true because Kalpa kept a copy of the
 * user's own file, and the "Stop managing" caveat counts exactly those. Same
 * flag, opposite promise; only `origin` can tell them apart.
 */

const mocks = vi.hoisted(() => ({ invokeOrThrow: vi.fn() }));

vi.mock("@/lib/tauri", () => ({
  getTauriErrorMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  invokeOrThrow: mocks.invokeOrThrow,
}));

vi.mock("@/components/client-stack/preset-panel", () => ({ PresetPanel: () => null }));
vi.mock("@/components/client-stack/tuning-panel", () => ({ TuningPanel: () => null }));
vi.mock("@/components/client-stack/runtime-drift-card", () => ({ RuntimeDriftCard: () => null }));
vi.mock("@/components/client-stack/shader-packs-panel", () => ({ ShaderPacksPanel: () => null }));

const clients: EsoClientLocation[] = [
  { client_dir: "C:\\ESO-A", exe_path: "C:\\ESO-A\\eso64.exe", source: "manual" },
];

function stackFor(clientDir: string): ClientStack {
  return {
    client_dir: clientDir,
    items: [],
    preserved_originals: [],
    parked: [],
    user_parked: [],
    is_disabled: false,
    shaders: { present: false, effect_count: 0, texture_count: 0, effect_search_paths: null },
    preset: null,
    tuning: [],
    tuning_section: null,
    tuning_owner: "unknown",
    tuning_blocks: [],
    disabled_addons: [],
    load_from_dll_main: [],
    active_path: "direct",
    slots: [],
    is_empty: false,
    findings: [],
  };
}

function reportFor(): Partial<ClientHealthReport> {
  return {
    log_excerpts: [],
    neural_rendering: {
      state: "unknown",
      samples: 0,
      first_evaluation: null,
      last_evaluation: null,
    },
    log_benign_suppressed: 0,
    findings: [],
  };
}

type ManagedFile = ManagedInventory["files"][number];

/** Kalpa wrote this one, and displaced nothing doing it. */
const placed: ManagedFile = {
  relative_path: "dxgi.dll",
  kind: "re_shade_core",
  placed_at: "2026-08-28T00:00:00.000Z",
  state: "present",
  origin: "placed",
  restores_backup: false,
};

/** The user installed this one. `restores_backup` is true because Kalpa kept a
 *  copy when it took over — not because it displaced an original of theirs. */
const adopted: ManagedFile = {
  relative_path: "nvngx_dlss.dll",
  kind: "nvidia_runtime",
  placed_at: "2026-08-28T00:00:00.000Z",
  state: "present",
  origin: "adopted",
  restores_backup: true,
};

function installIpc(files: ManagedFile[]) {
  mocks.invokeOrThrow.mockImplementation(
    async (command: string, args?: Record<string, unknown>) => {
      const clientDir = args?.clientDir as string | undefined;
      switch (command) {
        case "detect_eso_clients":
          return clients;
        case "inspect_client_stack":
          return stackFor(clientDir!);
        case "plan_adoption":
          return {
            client_dir: clientDir!,
            entries: [],
            copy_bytes: 0,
            already_managed: true,
            is_empty: false,
            stack_switched_off: false,
          } satisfies AdoptionPlan;
        case "list_managed_client_files":
          return { client_dir: clientDir!, files, orphan_injectors: [] } satisfies ManagedInventory;
        case "inspect_eso_client":
          return reportFor();
        case "plan_client_toggle":
          return {
            client_dir: clientDir!,
            action: "disable",
            is_disabled: false,
            operations: [],
            blockers: [],
          } satisfies TogglePlan;
        case "uninstall_managed_client_files":
          return { removed: (args?.relativePaths as string[]) ?? [], skipped: [] };
        case "set_game_install_path":
          return clients.find((client) => client.client_dir === args?.path);
        case "clear_game_install_path":
        case "choose_client_path":
          return undefined;
        default:
          throw new Error(`Unexpected IPC command: ${command}`);
      }
    }
  );
}

async function openRecords() {
  fireEvent.click(await screen.findByRole("option", { name: /Kalpa's records/ }));
}

describe("Kalpa's records: placed versus adopted", () => {
  beforeAll(() => {
    Object.defineProperty(Element.prototype, "getAnimations", {
      configurable: true,
      value: () => [],
    });
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn().mockImplementation((query: string) => ({
        matches: false,
        media: query,
        onchange: null,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        addListener: vi.fn(),
        removeListener: vi.fn(),
        dispatchEvent: vi.fn(),
      })),
    });
  });

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("describes an adopted file as the user's own rather than as safe to remove", async () => {
    installIpc([placed, adopted]);
    render(<ClientHealthPanel open onClose={vi.fn()} />);
    await openRecords();

    // The placed row keeps the sentence it has always had.
    expect(
      await screen.findByText(/Unchanged since Kalpa wrote it\. Safe to remove\./)
    ).toBeInTheDocument();

    // The adopted row is `present` too, so state alone produced that same
    // sentence twice. It must say who owns the bytes instead.
    expect(
      await screen.findByText(/Your own file\. Kalpa recorded it but never wrote it/)
    ).toBeVisible();
    // And point at the operation that does exist for it.
    expect(await screen.findByText(/stop managing this folder to drop the record/)).toBeVisible();
    // Counted last, once both rows have settled: the placed row keeps the
    // sentence and the adopted row must not have produced a second one.
    expect(screen.getAllByText(/Safe to remove\./)).toHaveLength(1);
  });

  it("does not promise an adopted file restores an original when removed", async () => {
    installIpc([adopted]);
    render(<ClientHealthPanel open onClose={vi.fn()} />);
    await openRecords();

    // `restores_backup` is true on this entry, which on a placed file means
    // "removing puts your original back". On an adopted one there is no
    // original of Kalpa's to put back and removal does nothing at all.
    expect(
      await screen.findByText(/Kalpa kept a copy of this file when it took over/)
    ).toBeInTheDocument();
    expect(screen.queryByText("Restores your original file when removed.")).toBeNull();
  });

  it("labels the adopted row and refuses to offer it for removal", async () => {
    installIpc([placed, adopted]);
    const user = userEvent.setup();
    render(<ClientHealthPanel open onClose={vi.fn()} />);
    await openRecords();

    expect(await screen.findByText("Adopted")).toBeVisible();

    const adoptedBox = screen.getByRole("checkbox", {
      name: /nvngx_dlss\.dll is your own file and cannot be removed by Kalpa/,
    });
    expect(adoptedBox).toBeDisabled();

    // The placed row is still selectable, and selecting it is what arms
    // "Remove selected" -- the adopted row must not contribute to that count.
    await user.click(screen.getByRole("checkbox", { name: "Select dxgi.dll" }));
    expect(screen.getByRole("button", { name: /Remove selected \(1\)/ })).toBeEnabled();
  });

  it("excludes adopted files from Remove all, in the count and in the payload", async () => {
    installIpc([placed, adopted]);
    const user = userEvent.setup();
    render(<ClientHealthPanel open onClose={vi.fn()} />);
    await openRecords();

    // Two rows, one removable. The button used to read "(2)" and remove one.
    expect(await screen.findByRole("button", { name: /Remove all \(1\)/ })).toBeInTheDocument();
    expect(await screen.findByText(/One of these files is your own/)).toBeVisible();

    await user.click(screen.getByRole("button", { name: /Remove all \(1\)/ }));
    expect(await screen.findByText(/Remove 1 file\?/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Confirm remove" }));

    await waitFor(() =>
      expect(mocks.invokeOrThrow).toHaveBeenCalledWith("uninstall_managed_client_files", {
        clientDir: "C:\\ESO-A",
        relativePaths: ["dxgi.dll"],
      })
    );
  });

  it("disables Remove all when every record is adopted", async () => {
    installIpc([adopted]);
    render(<ClientHealthPanel open onClose={vi.fn()} />);
    await openRecords();

    // Not hidden: the records exist and the list shows them. There is simply
    // nothing here that removal is permitted to act on.
    expect(await screen.findByRole("button", { name: /Remove all \(0\)/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Remove selected \(0\)/ })).toBeDisabled();
  });

  it("still counts an adopted kept copy in the Stop managing caveat", async () => {
    installIpc([adopted]);
    const user = userEvent.setup();
    render(<ClientHealthPanel open onClose={vi.fn()} />);
    await openRecords();

    // The caveat is about backups Kalpa holds, which is what `restores_backup`
    // means on both origins. Narrowing that flag to placed files instead of
    // branching on `origin` would silently drop this warning.
    await user.click(await screen.findByRole("button", { name: "Stop managing" }));
    expect(await screen.findByText(/the 1 kept copy of your swapped runtimes/)).toBeInTheDocument();
  });
});
