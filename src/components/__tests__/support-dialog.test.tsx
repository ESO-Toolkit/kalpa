import type { ComponentProps } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { UNKNOWN_SUPPORT_ENVIRONMENT } from "@/lib/support-report";
import { SupportDialog } from "../support-dialog";

const mocks = vi.hoisted(() => ({
  collectSupportEnvironment: vi.fn(),
  getVersion: vi.fn(),
  openFeedbackUrl: vi.fn(),
  osType: vi.fn(),
}));

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: mocks.getVersion,
}));

vi.mock("@/lib/support-environment", () => ({
  collectSupportEnvironment: mocks.collectSupportEnvironment,
}));

vi.mock("@/lib/platform", () => ({
  osType: mocks.osType,
}));

vi.mock("@/lib/feedback", () => ({
  FEEDBACK_DISCORD_SUPPORT_URL: "https://discord.example/support",
  openFeedbackUrl: mocks.openFeedbackUrl,
}));

type SupportDialogProps = ComponentProps<typeof SupportDialog>;

function supportDialogProps(overrides: Partial<SupportDialogProps> = {}): SupportDialogProps {
  return {
    addons: [],
    addonsPath: "C:/Users/you/Documents/Elder Scrolls Online/live/AddOns",
    checkingUpdates: false,
    instanceLabel: "Native NA",
    isOffline: false,
    lastError: "Initial diagnostic message",
    onClose: vi.fn(),
    updateResults: [],
    ...overrides,
  };
}

describe("SupportDialog consent binding", () => {
  beforeAll(() => {
    Object.defineProperty(Element.prototype, "getAnimations", {
      configurable: true,
      value: () => [],
    });

    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      writable: true,
      value: (query: string) =>
        ({
          matches: false,
          media: query,
          onchange: null,
          addListener: vi.fn(),
          removeListener: vi.fn(),
          addEventListener: vi.fn(),
          removeEventListener: vi.fn(),
          dispatchEvent: vi.fn(() => false),
        }) as MediaQueryList,
    });
  });

  beforeEach(() => {
    mocks.collectSupportEnvironment.mockReset();
    mocks.getVersion.mockReset();
    mocks.openFeedbackUrl.mockReset();
    mocks.osType.mockReset();

    mocks.collectSupportEnvironment.mockResolvedValue(UNKNOWN_SUPPORT_ENVIRONMENT);
    mocks.getVersion.mockResolvedValue("0.1.0-test");
    mocks.openFeedbackUrl.mockResolvedValue(true);
    mocks.osType.mockReturnValue("windows");
  });

  it("announces when a report change invalidates consent", async () => {
    const user = userEvent.setup();
    const consentLabel = "I agree to share the exact report shown below with ESO Toolkit support";
    const reportChangedMessage =
      "The report changed since you agreed. Review it and tick the box again.";
    const initialProps = supportDialogProps();

    const { rerender } = render(<SupportDialog {...initialProps} />);

    await waitFor(() => {
      expect(screen.getByText(/- Kalpa version: 0\.1\.0-test/)).toBeInTheDocument();
    });

    expect(screen.getByRole("status")).toHaveTextContent("Your report is prepared");

    await user.click(screen.getByRole("checkbox", { name: consentLabel }));

    expect(screen.getByRole("checkbox", { name: consentLabel })).toBeChecked();

    rerender(
      <SupportDialog
        {...supportDialogProps({
          lastError: "Changed diagnostic message",
          onClose: initialProps.onClose,
        })}
      />
    );

    expect(screen.getByRole("checkbox", { name: consentLabel })).not.toBeChecked();
    expect(screen.getByRole("status")).toHaveAttribute("id", "support-stage");
    expect(screen.getByRole("status")).toHaveTextContent(reportChangedMessage);
  });

  it("announces browser handoff and report changes from a deferred opener race", async () => {
    const user = userEvent.setup();
    const consentLabel = "I agree to share the exact report shown below with ESO Toolkit support";
    const continuingMessage =
      "Continuing in your browser. No ticket exists until ESO Toolkit confirms it there.";
    const reportChangedMessage =
      "The report changed since you agreed. Review it and tick the box again.";
    let resolveOpen!: (opened: boolean) => void;
    mocks.openFeedbackUrl.mockImplementation(
      () =>
        new Promise<boolean>((resolve) => {
          resolveOpen = resolve;
        })
    );
    const initialProps = supportDialogProps();

    const { rerender } = render(<SupportDialog {...initialProps} />);

    await waitFor(() => {
      expect(screen.getByText(/- Kalpa version: 0\.1\.0-test/)).toBeInTheDocument();
    });

    await user.click(screen.getByRole("checkbox", { name: consentLabel }));
    await user.click(screen.getByRole("button", { name: /Create private ticket/i }));

    rerender(
      <SupportDialog
        {...supportDialogProps({
          lastError: "Changed diagnostic message",
          onClose: initialProps.onClose,
        })}
      />
    );

    await act(async () => {
      resolveOpen(true);
    });

    const status = screen.getByRole("status");
    expect(status).toHaveAttribute("id", "support-stage");
    expect(status).toHaveTextContent(continuingMessage);
    expect(status).toHaveTextContent(reportChangedMessage);
    expect(screen.getByRole("checkbox", { name: consentLabel })).not.toBeChecked();
  });
});
