import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { LogFileInfo, LogPathDetection, TransportInfo } from "@/types/uploader";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  open: vi.fn(),
  toastError: vi.fn(),
  dragHandler: null as null | ((event: { payload: { type: string; paths?: string[] } }) => void),
}));

vi.mock("@/lib/tauri", () => ({
  getTauriErrorMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  invokeOrThrow: mocks.invoke,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open }));

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async (
      handler: (event: { payload: { type: string; paths?: string[] } }) => void
    ) => {
      mocks.dragHandler = handler;
      return vi.fn();
    },
  }),
}));

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
    mocks.dragHandler = null;
  });

  it.each(["resolve", "reject"] as const)(
    "keeps list, loading, error, and selection scoped to directory B when A %ss last",
    async (settlement) => {
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

      if (settlement === "resolve") listA.resolve([log("A-file")]);
      else listA.reject(new Error("A became unavailable"));
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
    }
  );

  it("does not restart the current directory when a picker resolves after refresh", async () => {
    const picker = deferred<string>();
    const refreshedDetection: LogPathDetection = { ...detection, path: "B" };
    let detectionCalls = 0;

    mocks.open.mockReturnValue(picker.promise);
    mocks.invoke.mockImplementation((command: string, args?: { logsDir?: string }) => {
      if (command === "uploader_detect_path") {
        detectionCalls++;
        return Promise.resolve(detectionCalls === 1 ? detection : refreshedDetection);
      }
      if (command === "uploader_transport_info") return Promise.resolve(transport);
      if (command === "uploader_list_logs" && args?.logsDir === "A") {
        return Promise.resolve([log("A-file")]);
      }
      if (command === "uploader_list_logs" && args?.logsDir === "B") {
        return Promise.resolve([log("B-file")]);
      }
      if (command === "uploader_list_history") return Promise.resolve([]);
      if (command === "uploader_has_session") return Promise.resolve(false);
      if (command === "settings_tainted") return Promise.resolve(false);
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

    await screen.findByText("A-file.log");
    fireEvent.click(screen.getByRole("button", { name: "Choose folder" }));

    // Change the active directory while the OS dialog is still unresolved.
    fireEvent.click(screen.getByRole("button", { name: "Refresh logs" }));
    await screen.findByText("B-file.log");

    picker.resolve("B");
    await waitFor(() => expect(screen.getByTitle("B")).toBeInTheDocument());

    const bListings = mocks.invoke.mock.calls.filter(
      ([command, args]) => command === "uploader_list_logs" && args?.logsDir === "B"
    );
    expect(bListings).toHaveLength(1);
  });

  it("preserves the current list while a same-folder refresh is pending", async () => {
    const refreshedList = deferred<LogFileInfo[]>();
    let listCalls = 0;

    mocks.invoke.mockImplementation((command: string, args?: { logsDir?: string }) => {
      if (command === "uploader_detect_path") return Promise.resolve(detection);
      if (command === "uploader_transport_info") return Promise.resolve(transport);
      if (command === "uploader_list_logs" && args?.logsDir === "A") {
        listCalls++;
        return listCalls === 1 ? Promise.resolve([log("A-before-refresh")]) : refreshedList.promise;
      }
      if (command === "uploader_list_history") return Promise.resolve([]);
      if (command === "uploader_has_session") return Promise.resolve(false);
      if (command === "settings_tainted") return Promise.resolve(false);
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

    await screen.findByText("A-before-refresh.log");
    fireEvent.click(screen.getByRole("button", { name: "Refresh logs" }));
    await waitFor(() => expect(listCalls).toBe(2));

    // The old successful snapshot remains usable until the replacement arrives.
    expect(screen.getByText("A-before-refresh.log")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Refresh logs" })).toBeDisabled();

    refreshedList.resolve([log("A-after-refresh")]);
    await screen.findByText("A-after-refresh.log");
    expect(screen.queryByText("A-before-refresh.log")).not.toBeInTheDocument();
  });

  it.each(["resolve", "reject"] as const)(
    "does not let late initial detection %s replace or report over a manual directory",
    async (settlement) => {
      const detectA = deferred<LogPathDetection>();

      mocks.open.mockResolvedValue("B");
      mocks.invoke.mockImplementation((command: string, args?: { logsDir?: string }) => {
        if (command === "uploader_detect_path") return detectA.promise;
        if (command === "uploader_transport_info") return Promise.resolve(transport);
        if (command === "uploader_list_logs" && args?.logsDir === "B") {
          return Promise.resolve([log("B-file")]);
        }
        if (command === "uploader_list_history") return Promise.resolve([]);
        if (command === "uploader_has_session") return Promise.resolve(false);
        if (command === "settings_tainted") return Promise.resolve(false);
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

      fireEvent.click(screen.getByRole("button", { name: "Choose folder" }));
      await screen.findByText("B-file.log");

      if (settlement === "resolve") detectA.resolve(detection);
      else detectA.reject(new Error("stale detection failed"));
      await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("uploader_list_history"));

      expect(screen.getByTitle("B")).toBeInTheDocument();
      expect(screen.getByText("B-file.log")).toBeInTheDocument();
      expect(mocks.invoke).not.toHaveBeenCalledWith("uploader_list_logs", { logsDir: "A" });
      expect(mocks.toastError).not.toHaveBeenCalledWith("stale detection failed");
    }
  );

  it("clears the old selection when refreshed detection changes directories and listing fails", async () => {
    let detectionCalls = 0;
    const detectionB: LogPathDetection = { ...detection, path: "B" };

    mocks.invoke.mockImplementation((command: string, args?: { logsDir?: string }) => {
      if (command === "uploader_detect_path") {
        detectionCalls++;
        return Promise.resolve(detectionCalls === 1 ? detection : detectionB);
      }
      if (command === "uploader_transport_info") return Promise.resolve(transport);
      if (command === "uploader_list_logs" && args?.logsDir === "A") {
        return Promise.resolve([log("A-file")]);
      }
      if (command === "uploader_list_logs" && args?.logsDir === "B") {
        return Promise.reject(new Error("B cannot be listed"));
      }
      if (command === "uploader_list_history") return Promise.resolve([]);
      if (command === "uploader_has_session") return Promise.resolve(false);
      if (command === "settings_tainted") return Promise.resolve(false);
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

    await screen.findByText("A-file.log");
    const aRow = document.querySelector<HTMLButtonElement>('[data-log-path="A-file"]');
    fireEvent.click(aRow!);
    await waitFor(() => expect(aRow).toHaveAttribute("aria-pressed", "true"));

    fireEvent.click(screen.getByRole("button", { name: "Refresh logs" }));
    await screen.findByText("Couldn't read this folder");

    expect(screen.getByTitle("B")).toBeInTheDocument();
    expect(screen.queryByText("A-file.log")).not.toBeInTheDocument();
  });

  it("does not select an imported path after its directory is replaced", async () => {
    const imported = deferred<string>();

    mocks.open.mockResolvedValue("B");
    mocks.invoke.mockImplementation((command: string, args?: { logsDir?: string }) => {
      if (command === "uploader_detect_path") return Promise.resolve(detection);
      if (command === "uploader_transport_info") return Promise.resolve(transport);
      if (command === "uploader_list_logs" && args?.logsDir === "A") return Promise.resolve([]);
      if (command === "uploader_list_logs" && args?.logsDir === "B") {
        return Promise.resolve([log("B-file")]);
      }
      if (command === "uploader_import_log") return imported.promise;
      if (command === "uploader_list_history") return Promise.resolve([]);
      if (command === "uploader_has_session") return Promise.resolve(false);
      if (command === "settings_tainted") return Promise.resolve(false);
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

    await waitFor(() => expect(mocks.dragHandler).not.toBeNull());
    mocks.dragHandler!({ payload: { type: "drop", paths: ["outside.log"] } });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("uploader_import_log", { srcPath: "outside.log" })
    );
    await screen.findByText(/Adding log to your folder/);

    fireEvent.click(screen.getByRole("button", { name: "Choose folder" }));
    await screen.findByText("B-file.log");
    imported.resolve("A-imported.log");

    await waitFor(() =>
      expect(screen.queryByText(/Adding log to your folder/)).not.toBeInTheDocument()
    );
    expect(mocks.invoke).not.toHaveBeenCalledWith("uploader_preflight", {
      filePath: "A-imported.log",
    });
    expect(screen.getByText("B-file.log")).toBeInTheDocument();
    expect(document.querySelector('[data-log-path="B-file"]')).toHaveAttribute(
      "aria-pressed",
      "false"
    );
  });
});
