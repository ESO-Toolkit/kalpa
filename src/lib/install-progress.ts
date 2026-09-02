import type { InstallPhase, InstallProgressEvent } from "@/types";

/**
 * One addon install's current progress, derived from the latest
 * `update-progress` event for its operation id.
 *
 * The backend reports two different units — bytes while downloading, files
 * while extracting — so this normalises both into `done`/`total` plus the label
 * that explains which is which. `determinate` is false when the backend has no
 * total to report (a response without `Content-Length`), which is the one case
 * the bar must render as indeterminate rather than as a bar frozen at 0%.
 */
export interface InstallProgress {
  phase: InstallPhase;
  /** The addon being downloaded/extracted, or the dependency being installed. */
  name: string;
  done: number;
  total: number;
  determinate: boolean;
}

const KB = 1024;
const MB = 1024 * 1024;
const GB = 1024 * 1024 * 1024;

/**
 * Format a byte range in ONE shared unit — "4.2 / 19.1 MB", never
 * "4.2 MB / 19.1 MB" and never "4,404,019 / 19.1 MB". The unit comes from the
 * total so it stays fixed for the whole download instead of stepping up as the
 * bytes arrive, which would make the label flicker between units.
 */
function formatByteRange(done: number, total: number): string {
  if (total >= GB) return `${(done / GB).toFixed(1)} / ${(total / GB).toFixed(1)} GB`;
  if (total >= MB) return `${(done / MB).toFixed(1)} / ${(total / MB).toFixed(1)} MB`;
  if (total >= KB) return `${(done / KB).toFixed(1)} / ${(total / KB).toFixed(1)} KB`;
  return `${done} / ${total} B`;
}

function formatLooseBytes(bytes: number): string {
  if (bytes >= GB) return `${(bytes / GB).toFixed(1)} GB`;
  if (bytes >= MB) return `${(bytes / MB).toFixed(1)} MB`;
  if (bytes >= KB) return `${(bytes / KB).toFixed(1)} KB`;
  return `${bytes} B`;
}

/** Normalise one `update-progress` payload into renderable progress. */
export function installProgressFromEvent(payload: InstallProgressEvent): InstallProgress {
  if (payload.phase === "downloading") {
    const done = payload.bytesDone ?? 0;
    const total = payload.bytesTotal ?? 0;
    return {
      phase: "downloading",
      name: payload.folderName,
      done,
      total,
      // ESOUI's CDN always sends Content-Length, but a proxy or a mirror may
      // not, and a determinate bar stuck at 0% reads as a hang.
      determinate: total > 0,
    };
  }
  if (payload.phase === "dependencies") {
    return {
      phase: "dependencies",
      name: payload.folderName,
      done: payload.fileIndex,
      total: payload.fileTotal,
      // Never determinate. The counts here are dependencies resolved so far,
      // and a single-dependency round reports 1 of 1 the moment it STARTS —
      // so a determinate bar would sit at 100% for the whole multi-second
      // library download, which reads as a hang rather than as progress. The
      // label names the dependency; the bar just has to keep moving.
      determinate: false,
    };
  }
  return {
    phase: payload.phase,
    name: payload.folderName,
    done: payload.fileIndex,
    total: payload.fileTotal,
    determinate: payload.fileTotal > 0,
  };
}

/**
 * The user-facing label for a progress snapshot: "Downloading 4.2 / 19.1 MB",
 * "Extracting 3,201 / 5,642 files", "Installing LibCustomIcons…".
 *
 * Dependencies are named rather than counted in files: a library is pulled in
 * behind the user's back, so the useful information is *which* one is holding
 * up the install they asked for.
 */
export function formatInstallProgress(progress: InstallProgress): string {
  switch (progress.phase) {
    case "downloading":
      return progress.determinate
        ? `Downloading ${formatByteRange(progress.done, progress.total)}`
        : `Downloading ${formatLooseBytes(progress.done)}`;
    case "extracting":
      return progress.determinate
        ? `Extracting ${progress.done.toLocaleString()} / ${progress.total.toLocaleString()} files`
        : "Extracting…";
    case "dependencies":
      return progress.total > 1
        ? `Installing ${progress.name}… (${progress.done} of ${progress.total})`
        : `Installing ${progress.name}…`;
  }
}
