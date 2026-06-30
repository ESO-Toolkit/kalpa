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
    /// Whether the detected/expected logs directory exists on disk.
    pub logs_dir_exists: bool,
    /// Whether the path was derived from the configured AddOns folder.
    pub from_addon_path: bool,
    /// Whether an `Encounter.log` already exists in that directory.
    pub encounter_log_exists: bool,
    /// Human-readable guidance for the UI.
    pub message: String,
}

/// Best-effort guess (from peeking the log's tail + a short growth sample) at whether
/// a fresh logging session is coming, for the native-live "what to tell the user
/// before Go Live" hint. NEVER gates going live — it only picks which waiting-state
/// guidance to show; the driver's first `BEGIN_LOG` is the ground truth that flips to
/// streaming.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LiveReadinessVerdict {
    /// Logging is already running (open session in the tail + the file is growing), so
    /// no fresh `BEGIN_LOG` is coming — native needs a `/reloadui` to start streaming.
    ActiveNoHeader,
    /// Not logging yet (no open session / not growing) — turning on `/encounterlog`
    /// will write the `BEGIN_LOG` native waits for.
    LoggingOff,
    /// No log file present to peek.
    NoLog,
    /// Couldn't tell from the peek — show soft guidance, no hard "/reloadui" claim.
    Uncertain,
}

/// The native-live readiness probe result (see `LiveReadinessVerdict`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveReadiness {
    pub verdict: LiveReadinessVerdict,
    /// Whether a fight appears to be in progress right now (advisory; strengthens copy).
    pub fight_in_progress: bool,
    /// Whether the file grew during the probe window (the growth disambiguator).
    pub grew: bool,
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
    /// The numeric `reportVisibilityId` the official uploader's
    /// `--report-visibility` flag expects. Verified against the installed app
    /// (app.asar v8.20.113): "Public = 0, Private = 1, Unlisted = 2". Getting
    /// this wrong is a privacy bug (e.g. mapping Private to 0 would upload it as
    /// Public), so the values must match the uploader's table exactly.
    pub fn as_report_visibility_id(self) -> u8 {
        match self {
            Visibility::Public => 0,
            Visibility::Private => 1,
            Visibility::Unlisted => 2,
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

/// Exact build hints recovered from Kalpa's native raw-log path and handed to
/// ESO Log Aggregator through the analysis link.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KalpaBuildEvidence {
    pub schema_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extractor_version: Option<u16>,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_code: Option<String>,
    #[serde(default)]
    pub players: Vec<KalpaPlayerBuildEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KalpaPlayerBuildEvidence {
    pub unit_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_occurrence_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_id: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub race_id: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub champion_points: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub class_mastery_passives: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub champion_point_passives: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub food: Option<KalpaFoodEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scribed_skills: Vec<KalpaScribedSkillEvidence>,
    pub evidence: String,
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KalpaFoodEvidence {
    pub ability_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KalpaScribedSkillEvidence {
    pub ability_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
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
    /// The report's human title — the name the user gave the report
    /// (`UploadOptions::description`). Only meaningful on the direct (native)
    /// path, which actually applies it; the official uploader ignores it, so a
    /// handed-off record leaves this `None` rather than persisting a stale name.
    /// `serde(default)` so records written before this field deserialize cleanly.
    #[serde(default)]
    pub title: Option<String>,
    /// The derived content label for the row's headline — the log's dominant
    /// zone/trial (from the frontend's `dominantZone(fights)`), so history reads
    /// "Lucent Citadel · Jun 27" instead of a bare file name. `serde(default)` for
    /// backward compatibility with pre-existing records.
    #[serde(default)]
    pub zone: Option<String>,
    /// Native raw-log evidence that ESO Log Aggregator can use to repair lossy
    /// player-card build extraction. `serde(default)` preserves old history rows.
    #[serde(default)]
    pub build_evidence: Option<KalpaBuildEvidence>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UploadStatus {
    Queued,
    Uploading,
    Live,
    /// A native live session whose ESO Logs session expired mid-stream: posting is
    /// paused (the report stays open) until the user re-signs-in. Distinct from `Live`
    /// so the panel can prompt a re-login rather than show a healthy badge.
    Paused,
    Completed,
    Failed,
    Cancelled,
    /// A live session the user stopped *tracking* in Kalpa. The official ESO
    /// Logs uploader runs as a separate app Kalpa can't stop (no programmatic
    /// stop exists, and the spawned PID is a self-exiting launcher), so it may
    /// still be streaming the log. `Completed` would falsely claim the upload
    /// finished and `Cancelled` would falsely claim Kalpa stopped it; this is the
    /// honest terminal state — "handed off, may still be uploading."
    HandedOff,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UploadMode {
    Manual,
    Live,
    SplitOnly,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Privacy-critical: these ids are forwarded verbatim to the official
    // uploader's --report-visibility flag, whose own table is Public=0,
    // Private=1, Unlisted=2 (verified against the installed app.asar). A swap
    // here would upload reports more openly than the user chose (e.g. Private
    // leaking as Public), so pin the exact values.
    #[test]
    fn report_visibility_ids_match_the_uploader_table() {
        assert_eq!(Visibility::Public.as_report_visibility_id(), 0);
        assert_eq!(Visibility::Private.as_report_visibility_id(), 1);
        assert_eq!(Visibility::Unlisted.as_report_visibility_id(), 2);
    }
}
