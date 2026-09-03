import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { ClientHealthPanel } from "../client-health";
import type {
  AdoptionPlan,
  ClientStack,
  EsoClientLocation,
  FileOpOutcome,
  ManagedInventory,
  TogglePlan,
} from "../client-health";

const mocks = vi.hoisted(() => ({
  invokeOrThrow: vi.fn(),
  open: vi.fn(),
}));

vi.mock("@/lib/tauri", () => ({
  getTauriErrorMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  invokeOrThrow: mocks.invokeOrThrow,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open }));
vi.mock("@/components/client-stack/preset-panel", () => ({ PresetPanel: () => null }));
vi.mock("@/components/client-stack/tuning-panel", () => ({ TuningPanel: () => null }));
vi.mock("@/components/client-stack/runtime-drift-card", () => ({ RuntimeDriftCard: () => null }));

const clients: EsoClientLocation[] = [
  {
    client_dir: "C:\\ESO-A",
    exe_path: "C:\\ESO-A\\eso64.exe",
    source: "manual",
  },
  {
    client_dir: "C:\\ESO-B",
    exe_path: "C:\\ESO-B\\eso64.exe",
    source: "manual",
  },
];

function stackFor(clientDir: string): ClientStack {
  return {
    client_dir: clientDir,
    items: [
      {
        role: "injector",
        file_name: "dxgi.dll",
        display_name: "ReShade",
        version: null,
        company: null,
        description: null,
        size_bytes: 1,
      },
    ],
    preserved_originals: [],
    parked: [],
    is_disabled: false,
    shaders: { present: false, effect_count: 0, texture_count: 0, effect_search_paths: null },
    preset: null,
    tuning: [],
    disabled_addons: [],
    is_empty: false,
    findings: [],
  };
}

function adoptionPlan(clientDir: string): AdoptionPlan {
  return {
    client_dir: clientDir,
    entries: [],
    copy_bytes: 0,
    already_managed: true,
    is_empty: false,
    stack_switched_off: false,
  };
}

function inventory(clientDir: string): ManagedInventory {
  return { client_dir: clientDir, files: [], orphan_injectors: [] };
}

function togglePlan(clientDir: string): TogglePlan {
  return {
    client_dir: clientDir,
    action: "disable",
    is_disabled: false,
    operations: [],
    blockers: [],
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function installDefaultIpc() {
  mocks.invokeOrThrow.mockImplementation(
    async (command: string, args?: Record<string, unknown>) => {
      const clientDir = args?.clientDir as string | undefined;
      switch (command) {
        case "detect_eso_clients":
          return clients;
        case "inspect_client_stack":
          return stackFor(clientDir!);
        case "plan_adoption":
          return adoptionPlan(clientDir!);
        case "list_managed_client_files":
          return inventory(clientDir!);
        case "inspect_eso_client":
          return { log_excerpts: [] };
        case "plan_client_toggle":
          return togglePlan(clientDir!);
        case "set_game_install_path":
          return clients.find((client) => client.client_dir === args?.path);
        case "clear_game_install_path":
          return undefined;
        default:
          throw new Error(`Unexpected IPC command: ${command}`);
      }
    }
  );
}

async function renderReady(onClose = vi.fn()) {
  const view = render(<ClientHealthPanel open onClose={onClose} />);
  fireEvent.click(await screen.findByRole("option", { name: /^Power/ }));
  await screen.findByRole("button", { name: "Switch off" });
  return { ...view, onClose };
}

describe("Client Health mutation coordination", () => {
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
    mocks.open.mockResolvedValue(null);
    installDefaultIpc();
  });

  it("locks install, refresh, close, and conflicting actions until the write reloads", async () => {
    const apply = deferred<FileOpOutcome>();
    const defaultImplementation = mocks.invokeOrThrow.getMockImplementation()!;
    mocks.invokeOrThrow.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "apply_client_toggle") return apply.promise;
      return defaultImplementation(command, args);
    });
    const user = userEvent.setup();
    const { onClose } = await renderReady();

    await user.click(screen.getByRole("button", { name: "Switch off" }));
    const confirm = await screen.findByRole("button", { name: "Confirm switch off" });
    fireEvent.click(confirm);
    fireEvent.click(confirm);

    await screen.findByText(/Client controls are temporarily locked/);
    expect(mocks.invokeOrThrow).toHaveBeenCalledWith("apply_client_toggle", {
      clientDir: clients[0]!.client_dir,
      expected: "disable",
    });
    expect(
      mocks.invokeOrThrow.mock.calls.filter(([command]) => command === "apply_client_toggle")
    ).toHaveLength(1);

    const installSelect = screen.getByRole("combobox");
    expect(installSelect).toBeDisabled();
    expect(screen.getByRole("button", { name: "Change" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Refresh" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();

    fireEvent.click(installSelect);
    fireEvent.click(screen.getByRole("button", { name: "Change" }));
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(screen.queryByRole("button", { name: "Close" })).not.toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "Graphics stack" })).toBeInTheDocument();
    expect(
      mocks.invokeOrThrow.mock.calls.filter(([command]) => command === "detect_eso_clients")
    ).toHaveLength(1);

    await act(async () => {
      apply.resolve({ applied: ["dxgi.dll"], skipped: [], preserved: [] });
      await apply.promise;
    });

    await waitFor(() => {
      expect(screen.queryByText(/Client controls are temporarily locked/)).not.toBeInTheDocument();
    });
    expect(
      mocks.invokeOrThrow.mock.calls.filter(
        ([command, args]) => command === "inspect_client_stack" && args?.clientDir === "C:\\ESO-A"
      )
    ).toHaveLength(2);
  });

  it("discards a child request that completes after the selected install changes", async () => {
    const stalePlan = deferred<TogglePlan>();
    const defaultImplementation = mocks.invokeOrThrow.getMockImplementation()!;
    mocks.invokeOrThrow.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "plan_client_toggle" && args?.clientDir === "C:\\ESO-A") {
        return stalePlan.promise;
      }
      return defaultImplementation(command, args);
    });
    const user = userEvent.setup();
    await renderReady();

    await user.click(screen.getByRole("button", { name: "Switch off" }));
    await screen.findByText("Working out the plan...");
    const installSelect = screen.getByRole("combobox");
    await user.click(installSelect);
    await user.click(await screen.findByRole("option", { name: "ESO-B" }));
    await waitFor(() => {
      expect(installSelect).toHaveTextContent("ESO-B");
    });
    fireEvent.click(await screen.findByRole("option", { name: /^Power/ }));
    await screen.findByRole("button", { name: /^Switch off/ });

    await act(async () => {
      stalePlan.resolve(togglePlan("C:\\ESO-A"));
      await stalePlan.promise;
    });

    expect(screen.queryByRole("button", { name: "Confirm switch off" })).not.toBeInTheDocument();
    expect(installSelect).toHaveTextContent("ESO-B");
  });

  it("does not reload or publish a completion after unmount", async () => {
    const apply = deferred<FileOpOutcome>();
    const defaultImplementation = mocks.invokeOrThrow.getMockImplementation()!;
    mocks.invokeOrThrow.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "apply_client_toggle") return apply.promise;
      return defaultImplementation(command, args);
    });
    const user = userEvent.setup();
    const view = await renderReady();

    await user.click(screen.getByRole("button", { name: "Switch off" }));
    await user.click(await screen.findByRole("button", { name: "Confirm switch off" }));
    await screen.findByText(/Client controls are temporarily locked/);
    view.unmount();

    await act(async () => {
      apply.resolve({ applied: ["dxgi.dll"], skipped: [], preserved: [] });
      await apply.promise;
    });

    expect(
      mocks.invokeOrThrow.mock.calls.filter(([command]) => command === "inspect_client_stack")
    ).toHaveLength(1);
    expect(screen.queryByText(/Applied/)).not.toBeInTheDocument();
  });
});
