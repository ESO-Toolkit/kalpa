// Types for the ESO Logs uploader feature. These mirror the Rust structs in
// `src-tauri/src/uploader/types.rs` (serialized as camelCase).

export interface LogPathDetection {
  path: string | null;
  fromAddonPath: boolean;
  encounterLogExists: boolean;
  message: string;
}

export interface LogFileInfo {
  path: string;
  fileName: string;
  sizeBytes: number;
  /** Unix epoch milliseconds of last modification (0 if unknown). */
  modifiedAtMs: number;
  /** Whether the file was modified recently enough to look "live". */
  isActive: boolean;
}

export interface LogSession {
  index: number;
  startOffset: number;
  endOffset: number;
  startTimeMs: number;
  logVersion: string;
  realm: string | null;
  fightCount: number;
  sizeBytes: number;
}

export interface FightSummary {
  index: number;
  startOffset: number;
  endOffset: number;
  startMs: number;
  endMs: number;
  zoneName: string | null;
  bossName: string | null;
}

export interface LogPreflight {
  path: string;
  sizeBytes: number;
  sessions: LogSession[];
  totalFights: number;
  /** Per-fight summaries from the single preflight scan. Empty for very large
   *  logs (see recommendSplit) to bound the IPC payload. */
  fights: FightSummary[];
  recommendSplit: boolean;
}

export type Visibility = "public" | "unlisted" | "private";

export interface UploadOptions {
  region: number;
  guildId: string | null;
  visibility: Visibility;
  description: string | null;
  realTime: boolean;
  includeEntireFile: boolean;
}

export interface ReportRef {
  code: string;
  url: string;
}

export type UploadStatus =
  | "queued"
  | "uploading"
  | "live"
  | "completed"
  | "failed"
  | "cancelled"
  // A live session the user stopped tracking in Kalpa; the official ESO Logs
  // uploader runs separately and may still be streaming. See the Rust
  // UploadStatus::HandedOff doc.
  | "handedOff";

export type UploadMode = "manual" | "live" | "splitOnly";

export interface UploadRecord {
  id: string;
  sourcePath: string;
  fileName: string;
  createdAtMs: number;
  status: UploadStatus;
  mode: UploadMode;
  visibility: Visibility;
  fightCount: number;
  report: ReportRef | null;
  error: string | null;
}

export interface TransportInfo {
  officialUploaderInstalled: boolean;
  activeTransport: string;
}

export interface UploadDispatch {
  handedOff: boolean;
  detail: string;
  report: ReportRef | null;
}

// Live event stream (tagged union mirroring the Rust `LiveEvent` enum).
// Fights are UI-only: the whole log is uploaded once to the official uploader
// with real-time enabled, so there is no per-fight upload status.
export type LiveEvent =
  | { type: "started"; file: string; startOffset: number }
  | {
      type: "fightDetected";
      index: number;
      zoneName: string | null;
      bossName: string | null;
      durationMs: number;
    }
  | { type: "sessionReset" }
  | { type: "fightSkipped"; reason: string }
  | { type: "warning"; message: string }
  | { type: "stopped"; reason: string };

/** The display-level status of the whole uploader, for the glanceable pill. */
export type UploaderStatus =
  | "idle"
  | "watching"
  | "uploading"
  | "upToDate"
  | "attention"
  | "retrying";

/** A single fight detected during a live session (UI timeline entry). The
 *  whole log uploads once to the official uploader, so there is no per-fight
 *  upload status — detection means the official uploader is streaming it. */
export interface LiveFight {
  index: number;
  zoneName: string | null;
  bossName: string | null;
  durationMs: number;
}

export const REGION_OPTIONS: { id: number; label: string }[] = [
  { id: 1, label: "North America (NA)" },
  { id: 2, label: "Europe (EU)" },
];
