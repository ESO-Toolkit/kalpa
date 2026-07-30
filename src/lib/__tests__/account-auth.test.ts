import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AuthUser } from "@/types";

const mocks = vi.hoisted(() => ({
  invokeOrThrow: vi.fn(),
  setSettings: vi.fn(),
  warnIfSessionNotPersisted: vi.fn(),
  toast: {
    error: vi.fn(),
    info: vi.fn(),
    loading: vi.fn(),
    success: vi.fn(),
  },
}));

vi.mock("@/lib/tauri", () => ({
  getTauriErrorMessage: (error: unknown) =>
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : "Something went wrong",
  invokeOrThrow: mocks.invokeOrThrow,
  warnIfSessionNotPersisted: mocks.warnIfSessionNotPersisted,
}));

vi.mock("@/lib/store", () => ({
  setSettings: mocks.setSettings,
}));

vi.mock("sonner", () => ({
  toast: mocks.toast,
}));

const { signInWithDirectUploadSetup } = await import("@/lib/account-auth");

const user: AuthUser = { userId: "42", userName: "Ada" };

function invokeByCommand(results: Record<string, unknown[]>) {
  mocks.invokeOrThrow.mockImplementation((command: string) => {
    const queue = results[command];
    if (!queue || queue.length === 0) {
      throw new Error(`unexpected command: ${command}`);
    }
    const result = queue.shift();
    if (result instanceof Error) throw result;
    return Promise.resolve(result);
  });
}

describe("signInWithDirectUploadSetup", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.toast.loading.mockReturnValue("toast-1");
    mocks.setSettings.mockResolvedValue(true);
  });

  it("finishes direct upload setup as part of profile sign-in", async () => {
    const onAuthChange = vi.fn();
    const onDirectUploadSetupComplete = vi.fn();
    invokeByCommand({
      auth_login: [user],
      uploader_has_session: [false, true],
      uploader_try_login_esologs_silent: [null],
      uploader_login_esologs: [{ sessionPersisted: true }],
    });

    const result = await signInWithDirectUploadSetup({
      context: "account",
      onAuthChange,
      onDirectUploadSetupComplete,
    });

    expect(result).toEqual({ user, directUploadStatus: "ready" });
    expect(onAuthChange).toHaveBeenCalledWith(user);
    expect(mocks.toast.success).toHaveBeenCalledWith("Signed in as Ada");

    expect(mocks.invokeOrThrow).toHaveBeenNthCalledWith(1, "auth_login");
    expect(mocks.invokeOrThrow).toHaveBeenNthCalledWith(2, "uploader_has_session");
    expect(mocks.invokeOrThrow).toHaveBeenNthCalledWith(3, "uploader_try_login_esologs_silent");
    expect(mocks.invokeOrThrow).toHaveBeenNthCalledWith(4, "uploader_login_esologs");
    expect(mocks.invokeOrThrow).toHaveBeenNthCalledWith(5, "uploader_has_session");
    expect(mocks.setSettings).toHaveBeenCalledWith({
      manualUseOfficialUploader: false,
      liveUseOfficialUploader: false,
    });
    expect(mocks.toast.loading).toHaveBeenCalledWith(
      "You're signed in. Finishing ESO Logs sign-in for direct upload."
    );
    expect(mocks.toast.success).toHaveBeenCalledWith(
      "Direct upload is on. Logs can go straight from Kalpa.",
      { id: "toast-1" }
    );
    expect(onDirectUploadSetupComplete).toHaveBeenCalledWith("ready");
  });

  it("uses silent direct upload capture before opening a visible window", async () => {
    invokeByCommand({
      auth_login: [user],
      uploader_has_session: [false, true],
      uploader_try_login_esologs_silent: [{ sessionPersisted: true }],
    });

    const result = await signInWithDirectUploadSetup({
      context: "uploader",
      onAuthChange: vi.fn(),
    });

    expect(result).toEqual({ user, directUploadStatus: "ready" });
    expect(mocks.invokeOrThrow).not.toHaveBeenCalledWith("uploader_login_esologs");
    expect(mocks.toast.loading).not.toHaveBeenCalled();
    expect(mocks.setSettings).toHaveBeenCalledWith({
      manualUseOfficialUploader: false,
      liveUseOfficialUploader: false,
    });
    expect(mocks.toast.success).toHaveBeenCalledWith(
      "Direct upload is on. Logs can go straight from Kalpa.",
      undefined
    );
  });

  it("reuses an existing upload session and enables direct upload", async () => {
    invokeByCommand({
      auth_login: [user],
      uploader_has_session: [true],
    });

    const result = await signInWithDirectUploadSetup({
      context: "uploader",
      onAuthChange: vi.fn(),
    });

    expect(result).toEqual({ user, directUploadStatus: "ready" });
    expect(mocks.invokeOrThrow).not.toHaveBeenCalledWith("uploader_try_login_esologs_silent");
    expect(mocks.invokeOrThrow).not.toHaveBeenCalledWith("uploader_login_esologs");
    expect(mocks.setSettings).toHaveBeenCalledWith({
      manualUseOfficialUploader: false,
      liveUseOfficialUploader: false,
    });
    expect(mocks.toast.loading).not.toHaveBeenCalled();
    expect(mocks.toast.success).toHaveBeenCalledWith("Signed in as Ada");
    expect(mocks.toast.success).toHaveBeenCalledWith(
      "Direct upload is on. Logs can go straight from Kalpa."
    );
  });

  it("keeps profile sign-in on direct upload cancel or timeout", async () => {
    const onAuthChange = vi.fn();
    invokeByCommand({
      auth_login: [user],
      uploader_has_session: [false],
      uploader_try_login_esologs_silent: [null],
      uploader_login_esologs: [new Error("Login timed out.")],
    });

    const result = await signInWithDirectUploadSetup({
      context: "packHub",
      onAuthChange,
    });

    expect(result).toEqual({ user, directUploadStatus: "fallback" });
    expect(onAuthChange).toHaveBeenCalledWith(user);

    expect(mocks.setSettings).toHaveBeenCalledWith({
      manualUseOfficialUploader: true,
      liveUseOfficialUploader: true,
    });
    expect(mocks.toast.info).toHaveBeenCalledWith(
      "You're signed in. Kalpa couldn't finish direct upload setup, so uploads will use the official uploader for now.",
      { id: "toast-1", duration: 9000 }
    );
    expect(mocks.toast.error).not.toHaveBeenCalled();
  });
});
