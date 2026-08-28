import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  openUrl: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));
vi.mock("sonner", () => ({ toast: { error: mocks.toastError } }));

const { openFeedbackUrl } = await import("../feedback");

describe("openFeedbackUrl", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("reports a successful browser handoff", async () => {
    mocks.openUrl.mockResolvedValue(undefined);

    await expect(openFeedbackUrl("https://example.com/help")).resolves.toBe(true);
    expect(mocks.toastError).not.toHaveBeenCalled();
  });

  it("reports failure without exposing a private URL fragment", async () => {
    mocks.openUrl.mockRejectedValue(new Error("browser unavailable"));
    const privateUrl = "https://esotk.com/kalpa/support#kalpa=private-diagnostics";

    await expect(openFeedbackUrl(privateUrl)).resolves.toBe(false);
    expect(mocks.toastError).toHaveBeenCalledOnce();
    expect(mocks.toastError.mock.calls.flat().join(" ")).not.toContain("private-diagnostics");
  });

  it("lets a caller provide its own contextual failure message", async () => {
    mocks.openUrl.mockRejectedValue(new Error("browser unavailable"));

    await expect(
      openFeedbackUrl("https://esotk.com/kalpa/support#kalpa=private", {
        toastOnError: false,
      })
    ).resolves.toBe(false);
    expect(mocks.toastError).not.toHaveBeenCalled();
  });
});
