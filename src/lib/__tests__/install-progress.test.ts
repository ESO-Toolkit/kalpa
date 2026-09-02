import { describe, it, expect } from "vitest";
import { formatInstallProgress, installProgressFromEvent } from "@/lib/install-progress";
import type { InstallProgressEvent } from "@/types";

const MIB = 1024 * 1024;

function downloadEvent(overrides: Partial<InstallProgressEvent> = {}): InstallProgressEvent {
  return {
    operationId: "op-1",
    folderName: "LibCustomIcons",
    phase: "downloading",
    fileIndex: 0,
    fileTotal: 0,
    bytesDone: 4 * MIB,
    bytesTotal: 20 * MIB,
    ...overrides,
  };
}

describe("installProgressFromEvent", () => {
  it("reads bytes during the download phase", () => {
    const progress = installProgressFromEvent(downloadEvent());
    expect(progress).toEqual({
      phase: "downloading",
      name: "LibCustomIcons",
      done: 4 * MIB,
      total: 20 * MIB,
      determinate: true,
    });
  });

  it("falls back to indeterminate when the server sent no Content-Length", () => {
    // `bytesTotal` is omitted from the payload entirely in that case, not zeroed.
    const progress = installProgressFromEvent(downloadEvent({ bytesTotal: undefined }));
    expect(progress.determinate).toBe(false);
    expect(progress.done).toBe(4 * MIB);
  });

  it("reads file counts during the extract phase, ignoring the absent byte fields", () => {
    const progress = installProgressFromEvent({
      operationId: "op-1",
      folderName: "LibCustomIcons",
      phase: "extracting",
      fileIndex: 3201,
      fileTotal: 5642,
    });
    expect(progress).toEqual({
      phase: "extracting",
      name: "LibCustomIcons",
      done: 3201,
      total: 5642,
      determinate: true,
    });
  });

  it("carries the dependency's own name, not the addon that pulled it in", () => {
    const progress = installProgressFromEvent({
      operationId: "op-1",
      folderName: "LibCustomIcons",
      phase: "dependencies",
      fileIndex: 2,
      fileTotal: 3,
    });
    expect(progress.name).toBe("LibCustomIcons");
    expect(progress.done).toBe(2);
    expect(progress.total).toBe(3);
  });

  it("keeps the dependency phase indeterminate even when the counts look complete", () => {
    // A single-dependency round reports 1 of 1 the moment it starts, so a
    // determinate bar would sit at 100% for the whole library download.
    const progress = installProgressFromEvent({
      operationId: "op-1",
      folderName: "LibCustomIcons",
      phase: "dependencies",
      fileIndex: 1,
      fileTotal: 1,
    });
    expect(progress.determinate).toBe(false);
  });
});

describe("formatInstallProgress", () => {
  it("shows one shared unit for a byte range", () => {
    // Not "4.0 MB / 20.0 MB", and not a raw byte count on the left — the unit
    // is picked from the total so it never changes mid-download.
    expect(formatInstallProgress(installProgressFromEvent(downloadEvent()))).toBe(
      "Downloading 4.0 / 20.0 MB"
    );
  });

  it("drops the total when there is none to show", () => {
    const progress = installProgressFromEvent(downloadEvent({ bytesTotal: undefined }));
    expect(formatInstallProgress(progress)).toBe("Downloading 4.0 MB");
  });

  it("keeps the KB unit for a small archive", () => {
    const progress = installProgressFromEvent(
      downloadEvent({ bytesDone: 12 * 1024, bytesTotal: 50 * 1024 })
    );
    expect(formatInstallProgress(progress)).toBe("Downloading 12.0 / 50.0 KB");
  });

  it("counts files while extracting", () => {
    const progress = installProgressFromEvent({
      operationId: "op-1",
      folderName: "LibCustomIcons",
      phase: "extracting",
      fileIndex: 3201,
      fileTotal: 5642,
    });
    expect(formatInstallProgress(progress)).toBe(
      `Extracting ${(3201).toLocaleString()} / ${(5642).toLocaleString()} files`
    );
  });

  it("names a single dependency without a count", () => {
    const progress = installProgressFromEvent({
      operationId: "op-1",
      folderName: "LibCustomIcons",
      phase: "dependencies",
      fileIndex: 1,
      fileTotal: 1,
    });
    expect(formatInstallProgress(progress)).toBe("Installing LibCustomIcons…");
  });

  it("adds the position when several dependencies are queued", () => {
    const progress = installProgressFromEvent({
      operationId: "op-1",
      folderName: "LibAddonMenu-2.0",
      phase: "dependencies",
      fileIndex: 2,
      fileTotal: 3,
    });
    expect(formatInstallProgress(progress)).toBe("Installing LibAddonMenu-2.0… (2 of 3)");
  });
});
