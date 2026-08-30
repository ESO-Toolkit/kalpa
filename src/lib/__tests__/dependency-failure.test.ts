import { beforeEach, describe, expect, it, vi } from "vitest";

const mockToastError = vi.fn();

vi.mock("sonner", () => ({
  toast: { error: (...args: unknown[]) => mockToastError(...args) },
}));

beforeEach(() => {
  mockToastError.mockReset();
});

describe("reportDependencyFailures", () => {
  it("does nothing when every automatic dependency succeeded", async () => {
    const { reportDependencyFailures } = await import("../dependency-failure");

    reportDependencyFailures([]);

    expect(mockToastError).not.toHaveBeenCalled();
  });

  it("shows the complete reserved-folder refusal", async () => {
    const { reportDependencyFailures } = await import("../dependency-failure");
    const refusal =
      "LibReserved: Refusing to extract reserved Kalpa state folder: .KALPA-STAGING. /payload";

    reportDependencyFailures([refusal]);

    expect(mockToastError).toHaveBeenCalledWith("A dependency could not be installed", {
      description: refusal,
    });
  });

  it("shows every failure in backend order", async () => {
    const { reportDependencyFailures } = await import("../dependency-failure");

    reportDependencyFailures(["LibOne: unavailable", "LibTwo: invalid archive"]);

    expect(mockToastError).toHaveBeenCalledWith("2 dependencies could not be installed", {
      description: "LibOne: unavailable; LibTwo: invalid archive",
    });
  });
});
