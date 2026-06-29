// Types for the ESO Logs uploader feature. These mirror the Rust structs in
// `src-tauri/src/uploader/types.rs` (serialized as camelCase).

export interface LogPathDetection {
  path: string | null;
  logsDirExists: boolean;
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

export interface KalpaPlayerBuildEvidence {
  unitId: string;
  characterName?: string | null;
  accountName?: string | null;
  characterId?: string | null;
  classId?: number | null;
  raceId?: number | null;
  level?: number | null;
  championPoints?: number | null;
  className?: string | null;
  classMasteryPassives?: number[];
  scribedSkills?: KalpaScribedSkillEvidence[];
  evidence: string;
  confidence: string;
}

export interface KalpaScribedSkillEvidence {
  abilityId: number;
  name?: string | null;
  icon?: string | null;
}

export interface KalpaBuildEvidence {
  schemaVersion: number;
  source: string;
  reportCode?: string | null;
  players: KalpaPlayerBuildEvidence[];
}

export type UploadStatus =
  | "queued"
  | "uploading"
  | "live"
  // A native live session whose ESO Logs session expired mid-stream: posting is
  // paused (the report stays open) until the user re-signs-in.
  | "paused"
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
  /** The report's human title (the name the user gave it). Only set on the
   *  direct/native path, which applies it; null for handed-off uploads and
   *  records written before this field existed. */
  title: string | null;
  /** The derived content label (dominant zone/trial) for the row headline, so
   *  history reads "Lucent Citadel" rather than a bare file name. Null when no
   *  zone could be derived or for pre-existing records. */
  zone: string | null;
  /** Exact native raw-log build evidence for ESO Log Aggregator links. */
  buildEvidence: KalpaBuildEvidence | null;
}

export interface TransportInfo {
  officialUploaderInstalled: boolean;
  activeTransport: string;
}

export interface UploadDispatch {
  handedOff: boolean;
  detail: string;
  report: ReportRef | null;
  buildEvidence: KalpaBuildEvidence | null;
}

// Live event stream (tagged union mirroring the Rust `LiveEvent` enum).
// Fights drive the local timeline/count for both native direct live and the
// official-uploader handoff; individual fights still do not have separate upload
// status in the UI.
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
  // Native live only: the report was created and now has a code — emitted the
  // instant create-report returns, so the UI can surface the report link / a live
  // analysis deep-link while the raid is still streaming (not just after settle).
  | { type: "reportOpened"; code: string; url: string }
  // Native live only: the first BEGIN_LOG arrived, so the driver is anchored and now
  // streaming — the UI flips from "waiting for a session" to "streaming".
  | { type: "sessionAnchored" }
  // Native live only: the ESO Logs session expired mid-stream (re-login prompt) /
  // a fresh session resumed posting.
  | { type: "reauthRequired"; message: string }
  | { type: "reauthResolved" }
  // `clean` is true for normal/user stops and false for watcher/native failures.
  | { type: "stopped"; reason: string; clean: boolean };

/// Result of the pre-Go-Live readiness probe (uploader_probe_live_readiness): a
/// best-effort guess at whether a fresh logging session is coming, used only to pick
/// which guidance the "waiting" state opens with — never to gate going live.
export interface LiveReadiness {
  /// `activeNoHeader` = logging is already running (no fresh BEGIN_LOG coming → needs
  /// /reloadui); `loggingOff` = not logging yet (turn on /encounterlog); `noLog` = no
  /// log file found; `uncertain` = couldn't tell (soft guidance).
  verdict: "activeNoHeader" | "loggingOff" | "noLog" | "uncertain";
  /// Whether a fight appears to be in progress right now (advisory; strengthens copy).
  fightInProgress: boolean;
  /// Whether the file grew during the probe window (the growth disambiguator).
  grew: boolean;
}

/** The display-level status of the whole uploader, for the glanceable pill. */
export type UploaderStatus =
  | "idle"
  | "watching"
  | "uploading"
  | "upToDate"
  | "attention"
  | "retrying";

/** A single fight detected during a live session (UI timeline entry). Detection
 *  means the live path accepted a completed fight for the session timeline; it is
 *  not a separate per-fight upload status. */
export interface LiveFight {
  index: number;
  zoneName: string | null;
  bossName: string | null;
  durationMs: number;
}

/** One session's choice in the split workbench: which session (by `index`,
 *  matching `LogSession.index`) and an optional custom name. Mirrors the Rust
 *  `SplitSelection`. Only sessions present in the selection are written. */
export interface SplitSelection {
  index: number;
  name: string | null;
  /** The session's startTimeMs at selection time; the backend verifies it still
   *  matches after any rescan so a shifted index can't mislabel a split. */
  startTimeMs: number | null;
}

/** One fight's choice in the per-fight split workbench: which fight (by `index`,
 *  matching `FightSummary.index`) and an optional custom name. Mirrors the Rust
 *  `FightSelection`. Each selected fight is written as its own self-contained
 *  single-fight `.log` (session preamble + only that fight's combat). */
export interface FightSelection {
  index: number;
  name: string | null;
  /** The fight's startMs at selection time; the backend verifies it still matches
   *  after any rescan so a shifted index can't extract/mislabel the wrong fight. */
  startMs: number | null;
}

export const REGION_OPTIONS: { id: number; label: string }[] = [
  { id: 1, label: "North America (NA)" },
  { id: 2, label: "Europe (EU)" },
];
