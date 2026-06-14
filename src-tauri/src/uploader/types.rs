//! Shared types for the ESO Logs uploader feature.
//!
//! These are serialized across the Tauri IPC boundary, so all field names use
//! `camelCase` to match the TypeScript side (`src/types/uploader.ts`).

use serde::{Deserialize, Serialize};

/// Result of attempting to locate the ESO `Logs` directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPathDetection {
    /// The detected (or expected) logs directory, if any.
    pub path: Option<String>,
    /// Whether the path was derived from the configured AddOns folder.
    pub from_addon_path: bool,
    /// Whether an `Encounter.log` already exists in that directory.
    pub encounter_log_exists: bool,
    /// Human-readable guidance for the UI.
    pub message: String,
}

/// Metadata about a single log file in the logs directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFileInfo {
    pub path: String,
    pub file_name: String,
    pub size_bytes: u64,
    /// Unix epoch milliseconds of last modification (0 if unknown).
    pub modified_at_ms: u64,
    /// Whether the file appears to currently be written to (recently modified).
    pub is_active: bool,
}

/// A single self-describing logging session inside an `Encounter.log`.
///
/// The game appends to one file across play sessions; each `/encounterlog`
/// (re)enable writes a fresh `BEGIN_LOG` line. A session spans from one
/// `BEGIN_LOG` to the next (or EOF / `END_LOG`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogSession {
    /// Zero-based index of this session within the file.
    pub index: usize,
    /// Byte offset of the `BEGIN_LOG` line (start of session, inclusive).
    pub start_offset: u64,
    /// Byte offset just past the end of the session (exclusive).
    pub end_offset: u64,
    /// Absolute wall-clock start time (unix ms) from the `BEGIN_LOG` line.
    pub start_time_ms: u64,
    /// The log format version declared in `BEGIN_LOG` (e.g. 15).
    pub log_version: String,
    /// Realm / megaserver string (e.g. "NA Megaserver"), if parsed.
    pub realm: Option<String>,
    /// Number of fights (BEGIN_COMBAT..END_COMBAT) detected in this session.
    pub fight_count: usize,
    /// Size of the session in bytes.
    pub size_bytes: u64,
}

/// A detected combat encounter (fight) within a session, expressed purely as
/// byte ranges so the uploader never has to hold the whole file in memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FightSummary {
    /// Zero-based index of the fight within the whole file.
    pub index: usize,
    /// Byte offset of the `BEGIN_COMBAT` line.
    pub start_offset: u64,
    /// Byte offset just past the matching `END_COMBAT` line.
    pub end_offset: u64,
    /// Relative time offset (ms since the session's BEGIN_LOG) of combat start.
    pub start_ms: u64,
    /// Relative time offset (ms) of combat end.
    pub end_ms: u64,
    /// Best-effort encounter / zone name, if one was seen near the fight.
    pub zone_name: Option<String>,
    /// Best-effort boss / monster name, if one was seen.
    pub boss_name: Option<String>,
}

/// A preflight summary of a whole log file: cheap-to-compute info shown before
/// any upload so the UI never looks frozen on a multi-GB file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPreflight {
    pub path: String,
    pub size_bytes: u64,
    pub sessions: Vec<LogSession>,
    pub total_fights: usize,
    /// The per-fight summaries from the single preflight scan. Empty for very
    /// large logs (see `recommend_split`) to bound the IPC payload — the counts
    /// in `sessions`/`total_fights` still drive the UI in that case.
    pub fights: Vec<FightSummary>,
    /// True if the file exceeds the size at which we recommend splitting.
    pub recommend_split: bool,
}

/// Where a report should be published.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Visibility {
    Public,
    Unlisted,
    Private,
}

impl Visibility {
    /// The numeric reportVisibilityId the ESO Logs uploader uses.
    pub fn as_id(self) -> u8 {
        match self {
            Visibility::Public => 1,
            Visibility::Unlisted => 2,
            Visibility::Private => 0,
        }
    }
}

/// User-selected options that apply to a manual or live upload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadOptions {
    /// Region ID for Personal Logs (US/NA = 1, EU = 2).
    pub region: u8,
    /// Guild ID to upload to; `None` => Personal Logs.
    pub guild_id: Option<String>,
    pub visibility: Visibility,
    /// Optional human description for the report.
    pub description: Option<String>,
    /// Live mode only: stream events in real time (vs. per-fight after combat).
    #[serde(default)]
    pub real_time: bool,
    /// Live mode only: include fights that occurred before watching started.
    #[serde(default)]
    pub include_entire_file: bool,
}

impl Default for UploadOptions {
    fn default() -> Self {
        Self {
            region: 1,
            guild_id: None,
            visibility: Visibility::Unlisted,
            description: None,
            real_time: false,
            include_entire_file: false,
        }
    }
}

/// A reference to a completed or in-progress report on ESO Logs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportRef {
    pub code: String,
    pub url: String,
}

/// A persisted record of an upload Kalpa initiated, shown in the history panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadRecord {
    /// Stable local id (uuid-like, generated client side or from time+path).
    pub id: String,
    pub source_path: String,
    pub file_name: String,
    /// Unix ms when the upload was started.
    pub created_at_ms: u64,
    pub status: UploadStatus,
    pub mode: UploadMode,
    pub visibility: Visibility,
    pub fight_count: usize,
    /// The resulting report, if one was created.
    pub report: Option<ReportRef>,
    /// Last error message, if the upload failed.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UploadStatus {
    Queued,
    Uploading,
    Live,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UploadMode {
    Manual,
    Live,
    SplitOnly,
}
