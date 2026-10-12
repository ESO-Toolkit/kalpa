import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { BackupInfo, BackupStorage } from "@/types";
import { Backups } from "./backups";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@/lib/tauri", () => ({ invokeOrThrow: invoke, getTauriErrorMessage: String }));
vi.mock("@/components/animate-ui/primitives/effects/fade", () => ({
  Fade: ({ children }: { children: ReactNode }) => children,
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
vi.mock("@/hooks/use-capped-animation-rate", () => ({ useCappedAnimationRate: vi.fn() }));

const sharedBackup: BackupInfo = {
  name: "Before changes.zip",
  createdAt: "2026-10-12",
  createdAtEpoch: Math.floor(Date.now() / 1000),
  fileCount: 2,
  totalSize: 2048,
  kind: "manual",
  format: "zip",
  legacy: false,
  storage: "shared",
};
const legacyBackup: BackupInfo = {
  ...sharedBackup,
  format: "folder",
  legacy: true,
  storage: "legacy",
};

function renderBackups(onSharedOpInFlightChange = vi.fn()) {
  return render(
    <Backups
      addonsPath="/eso/live/AddOns"
      onClose={vi.fn()}
      sharedOpInFlight={false}
      onSharedOpInFlightChange={onSharedOpInFlightChange}
    />
  );
}

beforeEach(() => {
  invoke.mockReset();
  invoke.mockImplementation(async (command: string) => {
    if (command === "list_backups") return [sharedBackup, legacyBackup];
    if (command === "restore_backup_safe") return { restoredFiles: 2, safetySnapshot: null };
    if (command === "delete_backup") return undefined;
    throw new Error(`Unexpected command: ${command}`);
  });
});

describe("Copying older settings backups", () => {
  it.each([
    { ...legacyBackup, kind: "character" as const },
    { ...legacyBackup, legacy: false },
    { ...legacyBackup, format: "zip" as const },
  ])("does not offer conversion for an ineligible backup %j", async (backup) => {
    invoke.mockResolvedValue([backup]);
    renderBackups();
    await screen.findByRole("button", { name: "Restore" });
    expect(screen.queryByRole("button", { name: "Review older backups" })).not.toBeInTheDocument();
  });

  it("requires confirmation and reports partial results with each skipped reason", async () => {
    const user = userEvent.setup();
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_backups") return [sharedBackup, legacyBackup];
      if (command === "consolidate_legacy_backups")
        return {
          copied: 1,
          alreadyPresent: 2,
          skipped: [
            { name: "Damaged backup", reason: "Settings folder could not be read" },
            { name: "Old preset", reason: "Destination contains different settings" },
          ],
        };
      throw new Error(`Unexpected command: ${command}`);
    });
    renderBackups();
    await user.click(await screen.findByRole("button", { name: "Review older backups" }));
    const review = screen.getByRole("dialog", { name: "Copy older backups to KalpaBackups?" });
    expect(review).toHaveTextContent(
      "settings folders from kalpa-backups into verified ZIPs in KalpaBackups/Manual"
    );
    expect(review).toHaveTextContent("Your original folders are kept");
    expect(review).toHaveTextContent("pre-restore backups into KalpaBackups/Automatic");
    expect(review).toHaveTextContent("Cleanup backups are not copied.");
    expect(review).toHaveTextContent(
      "Character backups remain supported in their original folders"
    );
    expect(
      invoke.mock.calls.filter(([command]) => command === "consolidate_legacy_backups")
    ).toHaveLength(0);
    await user.click(within(review).getByRole("button", { name: "Cancel" }));
    expect(
      invoke.mock.calls.filter(([command]) => command === "consolidate_legacy_backups")
    ).toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Review older backups" }));
    await user.click(
      within(screen.getByRole("dialog", { name: "Copy older backups to KalpaBackups?" })).getByRole(
        "button",
        { name: "Copy older backups" }
      )
    );
    expect(invoke).toHaveBeenCalledWith("consolidate_legacy_backups", {
      addonsPath: "/eso/live/AddOns",
    });
    const result = await screen.findByRole("status");
    expect(result).toHaveTextContent("1 copied · 2 already present · 2 skipped");
    expect(result).toHaveTextContent("Original folders were kept.");
    expect(result).toHaveTextContent("Damaged backup: Settings folder could not be read");
    expect(result).toHaveTextContent("Old preset: Destination contains different settings");
    await waitFor(() =>
      expect(invoke.mock.calls.filter(([command]) => command === "list_backups")).toHaveLength(2)
    );
  });

  it("holds the shared mutation lock until copying and the list reload finish", async () => {
    const user = userEvent.setup();
    const onSharedChange = vi.fn();
    let finishCopy!: (value: { copied: number; alreadyPresent: number; skipped: never[] }) => void;
    const copy = new Promise((resolve) => {
      finishCopy = resolve;
    });
    let finishReload!: (value: BackupInfo[]) => void;
    const reload = new Promise<BackupInfo[]>((resolve) => {
      finishReload = resolve;
    });
    let listCalls = 0;
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_backups") {
        listCalls += 1;
        return listCalls === 1 ? [sharedBackup, legacyBackup] : reload;
      }
      if (command === "consolidate_legacy_backups") return copy;
      throw new Error(`Unexpected command: ${command}`);
    });
    renderBackups(onSharedChange);
    await user.click(await screen.findByRole("button", { name: "Review older backups" }));
    await user.click(
      within(screen.getByRole("dialog", { name: "Copy older backups to KalpaBackups?" })).getByRole(
        "button",
        { name: "Copy older backups" }
      )
    );
    expect(onSharedChange).toHaveBeenLastCalledWith(true);
    expect(screen.getByRole("button", { name: "Copying…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Back Up Now", hidden: true })).toBeDisabled();
    for (const button of screen
      .getAllByRole("button", { hidden: true })
      .filter(
        (button) =>
          button.textContent === "Restore" ||
          button.getAttribute("aria-label")?.startsWith("Delete ")
      ))
      expect(button).toBeDisabled();
    finishCopy({ copied: 1, alreadyPresent: 0, skipped: [] });
    await waitFor(() => expect(listCalls).toBe(2));
    expect(onSharedChange).toHaveBeenLastCalledWith(true);
    expect(screen.getByRole("button", { name: "Back Up Now" })).toBeDisabled();
    finishReload([sharedBackup, legacyBackup]);
    await waitFor(() => expect(onSharedChange).toHaveBeenLastCalledWith(false));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "1 copied · 0 already present · 0 skipped"
    );
  });
});
afterEach(cleanup);

describe("Backups storage compatibility", () => {
  const locations: { storage: BackupStorage; path: string }[] = [
    { storage: "manual", path: "KalpaBackups/Manual" },
    { storage: "automatic", path: "KalpaBackups/Automatic" },
    { storage: "characters", path: "KalpaBackups/Characters" },
    { storage: "shared", path: "KalpaBackups" },
    { storage: "legacy", path: "kalpa-backups" },
  ];

  it.each(locations)(
    "restores and deletes the selected $storage location when names match",
    async ({ storage, path }) => {
      const user = userEvent.setup();
      invoke.mockImplementation(async (command: string) => {
        if (command === "list_backups")
          return locations.map((location) => ({
            ...sharedBackup,
            storage: location.storage,
            legacy: location.storage === "legacy",
            format: location.storage === "legacy" ? "folder" : "zip",
          }));
        if (command === "restore_backup_safe") return { restoredFiles: 2, safetySnapshot: null };
        if (command === "delete_backup") return undefined;
        throw new Error(`Unexpected command: ${command}`);
      });
      renderBackups();
      const restoreButtons = await screen.findAllByRole("button", { name: "Restore" });
      expect(restoreButtons).toHaveLength(5);
      for (const location of locations)
        expect(screen.getAllByText(location.path, { exact: true }).length).toBeGreaterThan(0);
      await user.click(restoreButtons[locations.findIndex((item) => item.storage === storage)]!);
      const confirmation = screen.getByRole("dialog", { name: "Restore this backup?" });
      expect(confirmation).toHaveTextContent(path);
      await user.click(within(confirmation).getByRole("button", { name: "Yes, Restore" }));
      const args = {
        addonsPath: "/eso/live/AddOns",
        backupName: sharedBackup.name,
        legacy: storage === "legacy",
        storage,
      };
      await waitFor(() => expect(invoke).toHaveBeenCalledWith("restore_backup_safe", args));
      const deleteButton = await screen.findByRole("button", {
        name: `Delete ${sharedBackup.name} (${path})`,
      });
      await waitFor(() => expect(deleteButton).toBeEnabled());
      await user.click(deleteButton);
      const confirmations = await screen.findAllByRole("button", { name: "Yes" });
      expect(confirmations).toHaveLength(1);
      await user.click(confirmations[0]!);
      await waitFor(() => expect(invoke).toHaveBeenCalledWith("delete_backup", args));
    }
  );

  it.each([false, true])(
    "keeps cleanup folders restorable without full protection or consolidation when legacy=%s",
    async (legacy) => {
      const user = userEvent.setup();
      const backup: BackupInfo = {
        ...sharedBackup,
        name: "auto-cleanup-2026",
        kind: "autoCleanup",
        format: "folder",
        legacy,
        storage: legacy ? "legacy" : "shared",
      };
      invoke.mockImplementation(async (command: string) => {
        if (command === "list_backups") return [backup];
        if (command === "restore_backup_safe") return { restoredFiles: 2, safetySnapshot: null };
        throw new Error(`Unexpected command: ${command}`);
      });
      renderBackups();
      const restore = await screen.findByRole("button", { name: "Restore" });
      expect(screen.getByText("Cleanup backup", { exact: true })).toBeInTheDocument();
      expect(screen.getByText("No full backup yet")).toBeInTheDocument();
      expect(screen.queryByText("Your settings are protected")).not.toBeInTheDocument();
      expect(screen.queryByText("Latest", { exact: true })).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: "Review older backups" })
      ).not.toBeInTheDocument();
      expect(
        screen.getByText(/Selected SavedVariables files saved before cleanup/)
      ).toHaveTextContent("Not a full settings backup.");
      await user.click(restore);
      const confirmation = screen.getByRole("dialog", { name: "Restore this backup?" });
      expect(confirmation).toHaveTextContent(
        "restore only the selected SavedVariables files saved before cleanup"
      );
      expect(confirmation).toHaveTextContent(
        "Other addon settings, addon files, and ESO game configuration stay unchanged."
      );
      await user.click(within(confirmation).getByRole("button", { name: "Yes, Restore" }));
      await waitFor(() =>
        expect(invoke).toHaveBeenCalledWith("restore_backup_safe", {
          addonsPath: "/eso/live/AddOns",
          backupName: backup.name,
          legacy,
          storage: legacy ? "legacy" : "shared",
        })
      );
    }
  );

  it("does not let a fresh cleanup backup refresh old full-backup protection", async () => {
    invoke.mockResolvedValue([
      { ...sharedBackup, createdAtEpoch: Math.floor(Date.now() / 1000) - 60 * 86400 },
      {
        ...sharedBackup,
        name: "auto-cleanup-fresh",
        kind: "autoCleanup",
        format: "folder",
      },
    ]);
    renderBackups();
    expect(await screen.findByText("Last backup was a while ago")).toBeInTheDocument();
    expect(screen.queryByText("Your settings are protected")).not.toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Restore" })).toHaveLength(2);
    const latestRow = screen.getByText("Latest").closest(".flex-1");
    expect(latestRow).toHaveTextContent(sharedBackup.name);
    expect(latestRow).not.toHaveTextContent("auto-cleanup-fresh");
  });

  it("reveals shared storage through the native allowed-path command", async () => {
    const user = userEvent.setup();
    invoke.mockImplementation(async (command: string) => {
      if (command === "list_backups") return [sharedBackup, legacyBackup];
      if (command === "get_backups_folder_path") return "/eso/live/KalpaBackups";
      if (command === "reveal_allowed_path") return undefined;
      throw new Error(`Unexpected command: ${command}`);
    });
    renderBackups();
    await user.click(await screen.findByRole("button", { name: "Show in folder" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("reveal_allowed_path", {
        path: "/eso/live/KalpaBackups",
      })
    );
    expect(invoke).toHaveBeenCalledWith("get_backups_folder_path", {
      addonsPath: "/eso/live/AddOns",
    });
  });

  it.each([false, true])(
    "restores the selected origin when legacy=%s and names match",
    async (legacy) => {
      const user = userEvent.setup();
      renderBackups();
      const restoreButtons = await screen.findAllByRole("button", { name: "Restore" });
      expect(restoreButtons).toHaveLength(2);
      await user.click(restoreButtons[legacy ? 1 : 0]!);
      const confirmation = screen.getByRole("dialog", { name: "Restore this backup?" });
      expect(confirmation).toHaveTextContent(
        legacy ? "Legacy folder · kalpa-backups" : "ZIP · KalpaBackups"
      );
      await user.click(within(confirmation).getByRole("button", { name: "Yes, Restore" }));
      await waitFor(() =>
        expect(invoke).toHaveBeenCalledWith("restore_backup_safe", {
          addonsPath: "/eso/live/AddOns",
          backupName: sharedBackup.name,
          legacy,
          storage: legacy ? "legacy" : "shared",
        })
      );
    }
  );

  it.each([false, true])(
    "deletes only the selected origin when legacy=%s and names match",
    async (legacy) => {
      const user = userEvent.setup();
      renderBackups();
      const label = `Delete ${sharedBackup.name} (${legacy ? "kalpa-backups" : "KalpaBackups"})`;
      await user.click(await screen.findByRole("button", { name: label }));
      const confirmations = await screen.findAllByRole("button", { name: "Yes" });
      expect(confirmations).toHaveLength(1);
      await user.click(confirmations[0]!);
      await waitFor(() =>
        expect(invoke).toHaveBeenCalledWith("delete_backup", {
          addonsPath: "/eso/live/AddOns",
          backupName: sharedBackup.name,
          legacy,
          storage: legacy ? "legacy" : "shared",
        })
      );
    }
  );

  it("shows archive formats, legacy origin, shared storage, and original settings sizes", async () => {
    invoke.mockResolvedValue([
      sharedBackup,
      legacyBackup,
      { ...sharedBackup, name: "char-Hero", kind: "character", format: "folder" },
    ]);
    renderBackups();
    expect(await screen.findByText("ZIP", { exact: true })).toBeInTheDocument();
    expect(screen.getByText("Folder", { exact: true })).toBeInTheDocument();
    expect(screen.getByText("Legacy folder", { exact: true })).toBeInTheDocument();
    const dialog = screen.getByRole("dialog", { name: "Backup & Restore" });
    expect(dialog).toHaveTextContent("New manual ZIPs are saved in KalpaBackups/Manual.");
    expect(dialog).toHaveTextContent(
      "KalpaBackups/Automatic; character folders use KalpaBackups/Characters."
    );
    expect(dialog).toHaveTextContent(
      "Existing backups in KalpaBackups and kalpa-backups remain available below."
    );
    expect(dialog).toHaveTextContent("6.0 KB of settings");
    expect(dialog).toHaveTextContent("Save all your addon settings in a compressed ZIP.");
  });

  it("shows list failures without claiming an empty collection and retries successfully", async () => {
    const user = userEvent.setup();
    invoke.mockRejectedValueOnce(new Error("Storage unavailable"));
    renderBackups();
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Couldn't load backups");
    expect(alert).toHaveTextContent("Storage unavailable");
    expect(screen.queryByText("No backups yet")).not.toBeInTheDocument();
    await user.click(within(alert).getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("ZIP", { exact: true })).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(invoke.mock.calls.filter(([command]) => command === "list_backups")).toHaveLength(2);
  });
});
