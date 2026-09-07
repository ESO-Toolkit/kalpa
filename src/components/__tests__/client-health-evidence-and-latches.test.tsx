import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { ClientHealthPanel, liveNeuralRenderingState } from "../client-health";
import type {
  AdoptionPlan,
  ClientHealthReport,
  ClientStack,
  EsoClientLocation,
  FileOpOutcome,
  ManagedInventory,
  TogglePlan,
} from "../client-health";

/**
 * Three ways this panel could contradict itself, all of them cross-surface.
 *
 * The gate that says "the stack has to be able to run before stale log
 * evidence means anything" lived in `stackVerdict` alone, so the header pill
 * applied it and the rail row and the log pane did not — and the disagreement
 * was on screen at the same time. `ClientHealthReport.findings` was fetched
 * and dropped. A destructive confirm and the install selection both survived a
 * reload that replaced what they were about.
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
  { client_dir: "C:\\ESO-B", exe_path: "C:\\ESO-B\\eso64.exe", source: "manual" },
];

function stackFor(clientDir: string, overrides: Partial<ClientStack> = {}): ClientStack {
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
    ...overrides,
  };
}

/** A report whose log says Neural Rendering ran, plus one report-level
 *  diagnosis. `ReShade.log` is truncated per launch and untouched by parking a
 *  DLL, so "it ran" alongside a switched-off stack is the ordinary case. */
function reportFor(): Partial<ClientHealthReport> {
  return {
    log_excerpts: [],
    neural_rendering: {
      state: "running",
      samples: 12,
      first_evaluation: 1,
      last_evaluation: 412,
    },
    log_benign_suppressed: 6,
    findings: [
      {
        id: "dlss-stale",
        level: "warning",
        title: "Bundled DLSS runtime is stale",
        detail: "nvngx_dlss.dll in the client folder reports version 2.2.16.",
        guide_url: null,
      },
      {
        id: "dlss-current",
        level: "ok",
        title: "DLSS runtime is current",
        detail: "Never worth a row: this pane is about what is worth reading.",
        guide_url: null,
      },
    ],
  };
}

function inventory(clientDir: string, files: ManagedInventory["files"] = []): ManagedInventory {
  return { client_dir: clientDir, files, orphan_injectors: [] };
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

function togglePlan(clientDir: string): TogglePlan {
  return {
    client_dir: clientDir,
    action: "disable",
    is_disabled: false,
    operations: [],
    blockers: [],
  };
}

const managedFile: ManagedInventory["files"][number] = {
  relative_path: "dxgi.dll",
  kind: "re_shade_core",
  placed_at: "2026-08-28T00:00:00.000Z",
  state: "present",
  restores_backup: false,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

function installIpc(options: { stack?: Partial<ClientStack>; files?: ManagedInventory["files"] }) {
  mocks.invokeOrThrow.mockImplementation(
    async (command: string, args?: Record<string, unknown>) => {
      const clientDir = args?.clientDir as string | undefined;
      switch (command) {
        case "detect_eso_clients":
          return clients;
        case "inspect_client_stack":
          return stackFor(clientDir!, options.stack);
        case "plan_adoption":
          return adoptionPlan(clientDir!);
        case "list_managed_client_files":
          return inventory(clientDir!, options.files ?? []);
        case "inspect_eso_client":
          return reportFor();
        case "plan_client_toggle":
          return togglePlan(clientDir!);
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

describe("liveNeuralRenderingState", () => {
  const evidence = {
    log_excerpts: [],
    neural_rendering: {
      state: "running" as const,
      samples: 12,
      first_evaluation: 1,
      last_evaluation: 412,
    },
  };

  it("downgrades a running log to unknown when the stack cannot be live", () => {
    expect(liveNeuralRenderingState(evidence, stackFor("C:\\ESO-A"))).toBe("running");
    expect(liveNeuralRenderingState(evidence, stackFor("C:\\ESO-A", { is_disabled: true }))).toBe(
      "unknown"
    );
    expect(
      liveNeuralRenderingState(evidence, stackFor("C:\\ESO-A", { active_path: "neither" }))
    ).toBe("unknown");
    // A stack that has not loaded yet proves nothing either.
    expect(liveNeuralRenderingState(evidence, null)).toBe("unknown");
  });

  it("never upgrades, and never turns absence of evidence into failure", () => {
    const stalled = {
      ...evidence,
      neural_rendering: { ...evidence.neural_rendering, state: "stalled" as const },
    };
    expect(liveNeuralRenderingState(stalled, stackFor("C:\\ESO-A", { is_disabled: true }))).toBe(
      "stalled"
    );
    const unknown = {
      ...evidence,
      neural_rendering: { ...evidence.neural_rendering, state: "unknown" as const },
    };
    expect(liveNeuralRenderingState(unknown, stackFor("C:\\ESO-A"))).toBe("unknown");
  });
});

describe("Client Health evidence and latches", () => {
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

  it("applies the stack-must-be-live gate to the rail row and the log pane, not just the header", async () => {
    installIpc({ stack: { is_disabled: true } });
    render(<ClientHealthPanel open onClose={vi.fn()} />);

    // The header gate already worked.
    await screen.findByRole("button", { name: /No proof it ran/ });

    // The rail row read the ungated field and said the opposite, in the same
    // viewport as the header pill.
    const logRow = await screen.findByRole("option", { name: /Log check/ });
    expect(logRow).toHaveAccessibleName(/no proof it ran/);
    expect(logRow).not.toHaveAccessibleName(/Neural Rendering ran/);

    fireEvent.click(logRow);
    expect(await screen.findByText("No evidence either way")).toBeInTheDocument();
    expect(screen.queryByText("Neural Rendering ran")).toBeNull();
    expect(screen.queryByText(/it is real proof/)).toBeNull();
  });

  it("renders the report's own findings, dropping the ok ones", async () => {
    installIpc({});
    render(<ClientHealthPanel open onClose={vi.fn()} />);

    fireEvent.click(await screen.findByRole("option", { name: /Log check/ }));

    // Computed by `client_health.rs::build_findings`, kept by `loadLogs`, and
    // shown here -- this family has no `FINDING_SLOT` entry, so no rail row
    // can carry it and it used to reach no surface at all.
    expect(await screen.findByText("Bundled DLSS runtime is stale")).toBeInTheDocument();
    expect(
      screen.getByText(/nvngx_dlss.dll in the client folder reports version 2.2.16/)
    ).toBeInTheDocument();
    expect(screen.queryByText("DLSS runtime is current")).toBeNull();
  });

  it("disarms a managed-file removal confirm when another mutation reloads the inventory", async () => {
    installIpc({ files: [managedFile] });
    const apply = deferred<FileOpOutcome>();
    const base = mocks.invokeOrThrow.getMockImplementation()!;
    mocks.invokeOrThrow.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "apply_client_toggle") return apply.promise;
      return base(command, args);
    });
    const user = userEvent.setup();
    render(<ClientHealthPanel open onClose={vi.fn()} />);

    fireEvent.click(await screen.findByRole("option", { name: /Kalpa's records/ }));
    await user.click(await screen.findByRole("button", { name: /Remove all \(1\)/ }));
    expect(await screen.findByText(/Remove 1 file\?/)).toBeInTheDocument();

    // A different flow entirely, and it never leaves the panel.
    fireEvent.click(screen.getByRole("option", { name: /^Power/ }));
    await user.click(await screen.findByRole("button", { name: "Switch off" }));
    await user.click(await screen.findByRole("button", { name: "Confirm switch off" }));
    await act(async () => {
      apply.resolve({ applied: ["dxgi.dll"], skipped: [], preserved: [] });
      await apply.promise;
    });

    fireEvent.click(await screen.findByRole("option", { name: /Kalpa's records/ }));
    // `pendingPaths` is derived live from the reloaded inventory and the prompt
    // only ever states a count, so leaving this armed executes over whatever is
    // in the folder now.
    await waitFor(() => expect(screen.queryByText(/Remove 1 file\?/)).toBeNull());
    expect(screen.queryByRole("button", { name: "Confirm remove" })).toBeNull();
    // Same reload, same rule: the "Stop managing" confirm hides the parked-stack
    // guard that lives in the other branch.
    expect(screen.queryByText(/your stack keeps working/)).toBeNull();
  });

  it("keeps the selected install across Refresh", async () => {
    installIpc({});
    const user = userEvent.setup();
    render(<ClientHealthPanel open onClose={vi.fn()} />);

    const installSelect = await screen.findByRole("combobox");
    await user.click(installSelect);
    await user.click(await screen.findByRole("option", { name: "ESO-B" }));
    await waitFor(() => expect(installSelect).toHaveTextContent("ESO-B"));

    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));

    // Refresh re-runs `detect`, which used to re-seed the selection from
    // `found[0]` unconditionally and drop a multi-install user back on the
    // first install without saying so.
    await waitFor(() =>
      expect(
        mocks.invokeOrThrow.mock.calls.filter(([command]) => command === "detect_eso_clients")
      ).toHaveLength(2)
    );
    await waitFor(() => expect(installSelect).toHaveTextContent("ESO-B"));
    expect(
      mocks.invokeOrThrow.mock.calls.filter(
        ([command, args]) => command === "inspect_client_stack" && args?.clientDir === "C:\\ESO-A"
      )
    ).toHaveLength(1);
  });
});
