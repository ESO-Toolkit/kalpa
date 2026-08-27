import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { LogFileInfo, LogPathDetection, TransportInfo } from "@/types/uploader";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  open: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock("@/lib/tauri", () => ({
  getTauriErrorMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  invokeOrThrow: mocks.invoke,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open }));

vi.mock("sonner", () => ({
  toast: {
    error: mocks.toastError,
    info: vi.fn(),
    success: vi.fn(),
  },
}));

vi.mock("@/lib/store", () => ({
  getSetting: vi.fn(async (_key: string, fallback: unknown) => fallback),
  getSettingChecked: vi.fn(async (_key: string, fallback: unknown) => ({
    value: fallback,
    trusted: true,
  })),
  settingsWritesSettled: vi.fn(async () => undefined),
}));

vi.mock("@/lib/account-auth", () => ({
  cancelProfileSignIn: vi.fn(),
  setupDirectUploadSession: vi.fn(),
  signInWithDirectUploadSetup: vi.fn(),
}));

vi.mock("@/hooks/use-capped-animation-rate", () => ({
  useCappedAnimationRate: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class Channel<T> {
    onmessage?: (message: T) => void;
  },
}));

import { UploaderWorkspace } from "../uploader-workspace";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const detection: LogPathDetection = {
  path: "A",
  logsDirExists: true,
  fromAddonPath: false,
  encounterLogExists: true,
  message: "Detected",
};

const transport: TransportInfo = {
  officialUploaderInstalled: false,
  activeTransport: "native",
};

function log(path: string): LogFileInfo {
  return {
    path,
    fileName: `${path}.log`,
    sizeBytes: 1,
    modifiedAtMs: 1,
    isActive: false,
  };
}

describe("UploaderWorkspace log-directory sequencing", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
  });

  it("keeps list, loading, error, and selection scoped to directory B when A resolves last", async () => {
    const listA = deferred<LogFileInfo[]>();
    const listB = deferred<LogFileInfo[]>();

    mocks.open.mockResolvedValue("B");
    mocks.invoke.mockImplementation((command: string, args?: { logsDir?: string }) => {
      if (command === "uploader_detect_path") return Promise.resolve(detection);
      if (command === "uploader_transport_info") return Promise.resolve(transport);
      if (command === "uploader_list_history") return Promise.resolve([]);
      if (command === "uploader_has_session") return Promise.resolve(false);
      if (command === "settings_tainted") return Promise.resolve(false);
      if (command === "uploader_list_logs" && args?.logsDir === "A") return listA.promise;
      if (command === "uploader_list_logs" && args?.logsDir === "B") return listB.promise;
      if (command === "uploader_preflight") {
        return Promise.resolve({ sessions: [], fights: [], scannedLen: 1, fileSize: 1 });
      }
      return Promise.resolve(null);
    });

    render(
      <UploaderWorkspace
        open
        authUser={null}
        onAuthChange={vi.fn()}
        onClose={vi.fn()}
        onOpen={vi.fn()}
      />
    );

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("uploader_list_logs", { logsDir: "A" })
    );
    fireEvent.click(screen.getByRole("button", { name: "Choose folder" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("uploader_list_logs", { logsDir: "B" })
    );

    expect(screen.getByRole("button", { name: "Refresh logs" })).toBeDisabled();

    listB.resolve([log("B-file")]);
    await screen.findByText("B-file.log");
    const bRow = document.querySelector<HTMLButtonElement>('[data-log-path="B-file"]');
    expect(bRow).not.toBeNull();
    expect(screen.queryByText("Couldn't read this folder")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Refresh logs" })).toBeEnabled();

    fireEvent.click(bRow!);
    await waitFor(() => expect(bRow).toHaveAttribute("aria-pressed", "true"));

    listA.resolve([log("A-file")]);
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("uploader_list_history"));

    expect(document.querySelector('[data-log-path="B-file"]')).toHaveAttribute(
      "aria-pressed",
      "true"
    );
    expect(document.querySelector('[data-log-path="A-file"]')).not.toBeInTheDocument();
    expect(screen.queryByText("Couldn't read this folder")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Refresh logs" })).toBeEnabled();
    expect(mocks.toastError).not.toHaveBeenCalledWith(
      expect.stringContaining("Couldn't list logs")
    );
  });
});
