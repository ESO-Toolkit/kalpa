import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { UpdateBanner } from "../update-banner";

class TestIntersectionObserver implements IntersectionObserver {
  readonly root = null;
  readonly rootMargin = "0px";
  readonly scrollMargin = "0px";
  readonly thresholds = [0];
  disconnect() {}
  observe() {}
  takeRecords(): IntersectionObserverEntry[] {
    return [];
  }
  unobserve() {}
}

vi.stubGlobal("IntersectionObserver", TestIntersectionObserver);

describe("UpdateBanner Protected Edits disclosure", () => {
  it("warns before update when an addon has no trusted baseline", () => {
    render(
      <UpdateBanner
        availableCount={1}
        updatingAll={false}
        updateProgress={null}
        addonStatuses={new Map()}
        updates={[
          {
            folderName: "MigratedAddon",
            title: "Migrated Addon",
            currentVersion: "1.0",
            remoteVersion: "2.0",
            hasProtectedEditsBaseline: false,
          },
        ]}
        onUpdateAll={vi.fn()}
        onUpdateSelected={vi.fn()}
      />
    );

    expect(screen.getByRole("status")).toHaveTextContent("Protected Edits unavailable");
    expect(screen.getByRole("status")).toHaveTextContent(
      "Kalpa cannot detect which files you changed"
    );
    expect(screen.getByRole("button", { name: "Update All" })).toBeEnabled();
  });

  it("does not warn when every update has a trusted baseline", () => {
    render(
      <UpdateBanner
        availableCount={1}
        updatingAll={false}
        updateProgress={null}
        addonStatuses={new Map()}
        updates={[
          {
            folderName: "ManagedAddon",
            title: "Managed Addon",
            currentVersion: "1.0",
            remoteVersion: "2.0",
            hasProtectedEditsBaseline: true,
          },
        ]}
        onUpdateAll={vi.fn()}
        onUpdateSelected={vi.fn()}
      />
    );

    expect(screen.queryByText(/Protected Edits unavailable/)).not.toBeInTheDocument();
  });
});
