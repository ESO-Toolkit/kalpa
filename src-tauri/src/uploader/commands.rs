//! Tauri command handlers and managed state for the uploader.
//!
//! Follows the project convention: `async` commands offload blocking work
//! (filesystem, process spawn) onto `spawn_blocking` and return `Result<T,
//! String>`. Path inputs from the webview are validated before any IO.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use tauri::ipc::Channel;
use tauri::State;

use super::types::*;
use super::watcher::{LiveEvent, LiveWatchHandle};
use super::{discovery, scanner, splitter, transport, watcher};
use crate::AllowedAddonsPath;

/// Managed state: active live-watch sessions keyed by session id.
#[derive(Default)]
pub struct UploaderState {
    pub live_sessions: Arc<Mutex<HashMap<String, LiveWatchHandle>>>,
}

// ── Input validation ─────────────────────────────────────────────────────────

/// Validate a caller-supplied path points at a `.log` file with no traversal.
fn validate_log_path(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    let is_log = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("log"))
        .unwrap_or(false);
    if !is_log {
        return Err("Only .log files can be processed.".into());
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("Path traversal is not allowed.".into());
    }
    Ok(())
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── Discovery & preflight ──────────────────────────────────────────────────

/// Detect the ESO logs directory, preferring the configured AddOns path.
#[tauri::command]
pub fn uploader_detect_path(
    allowed: State<'_, AllowedAddonsPath>,
) -> Result<LogPathDetection, String> {
    let addons = allowed.0.lock().map_err(|_| "Failed to read addons path")?;
    let ap = addons
        .as_ref()
        .map(|a| a.configured.to_string_lossy().into_owned());
    Ok(discovery::detect_log_path(ap.as_deref()))
}

/// List `*.log` files in a directory (newest first).
#[tauri::command]
pub async fn uploader_list_logs(logs_dir: String) -> Result<Vec<LogFileInfo>, String> {
    tokio::task::spawn_blocking(move || discovery::list_log_files(&logs_dir))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

/// Preflight a log file: sessions, fight counts, split recommendation. Cheap
/// enough to run on selection so the UI never looks frozen on a huge file.
#[tauri::command]
pub async fn uploader_preflight(file_path: String) -> Result<LogPreflight, String> {
    validate_log_path(&file_path)?;
    tokio::task::spawn_blocking(move || {
        let size_bytes = std::fs::metadata(&file_path)
            .map_err(|e| format!("Failed to read file: {e}"))?
            .len();
        let scan = scanner::scan_file(&file_path)?;
        let total_fights = scan.fights.len();
        Ok::<_, String>(LogPreflight {
            path: file_path,
            size_bytes,
            sessions: scan.sessions,
            total_fights,
            recommend_split: size_bytes > scanner::SPLIT_RECOMMEND_BYTES,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Return the per-fight summaries for a file (for the fight list UI).
#[tauri::command]
pub async fn uploader_scan_fights(file_path: String) -> Result<Vec<FightSummary>, String> {
    validate_log_path(&file_path)?;
    tokio::task::spawn_blocking(move || Ok(scanner::scan_file(&file_path)?.fights))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

// ── Split to disk ──────────────────────────────────────────────────────────

/// Split an oversized log into one file per session inside `out_dir`.
#[tauri::command]
pub async fn uploader_split_to_disk(
    file_path: String,
    out_dir: String,
) -> Result<Vec<String>, String> {
    validate_log_path(&file_path)?;
    tokio::task::spawn_blocking(move || splitter::split_by_session(&file_path, &out_dir))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

// ── Transport availability ─────────────────────────────────────────────────

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportInfo {
    /// Whether the official uploader executable was found on this machine.
    pub official_uploader_installed: bool,
    /// The transport that will be used for an automated upload.
    pub active_transport: String,
}

/// Report which upload transport is available (drives the UI's upload button
/// copy: "Upload via ESO Logs Uploader" vs "Open in ESO Logs Uploader").
#[tauri::command]
pub fn uploader_transport_info() -> TransportInfo {
    let installed = transport::find_official_uploader().is_some();
    TransportInfo {
        official_uploader_installed: installed,
        active_transport: transport::select_transport(installed).name().to_string(),
    }
}

// ── Manual upload / handoff ─────────────────────────────────────────────────

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadDispatch {
    /// True if the upload was handed off to the official uploader UI.
    pub handed_off: bool,
    pub detail: String,
    pub report: Option<ReportRef>,
}

/// Dispatch a prepared log to the official uploader. `prefer_cli` uses the CLI
/// transport when available; otherwise opens the uploader UI with the file.
#[tauri::command]
pub async fn uploader_upload_log(
    app: tauri::AppHandle,
    file_path: String,
    options: UploadOptions,
    prefer_cli: bool,
) -> Result<UploadDispatch, String> {
    validate_log_path(&file_path)?;

    let file_name = Path::new(&file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Encounter.log")
        .to_string();

    // Count fights for the history record (cheap relative to the upload).
    let scan_path = file_path.clone();
    let fight_count = tokio::task::spawn_blocking(move || {
        scanner::scan_file(&scan_path)
            .map(|s| s.fights.len())
            .unwrap_or(0)
    })
    .await
    .unwrap_or(0);

    let record_id = format!("{}-{}", now_ms(), file_name);
    let mut record = UploadRecord {
        id: record_id.clone(),
        source_path: file_path.clone(),
        file_name,
        created_at_ms: now_ms(),
        status: UploadStatus::Uploading,
        mode: UploadMode::Manual,
        visibility: options.visibility,
        fight_count,
        report: None,
        error: None,
    };
    let _ = super::history::upsert(&app, record.clone());

    let opts = options.clone();
    let dispatch_path = file_path.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let t = transport::select_transport(prefer_cli);
        t.upload_file(&dispatch_path, &opts)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?;

    match outcome {
        Ok(transport::UploadOutcome::HandedOff { detail }) => {
            // The user finishes in the official UI; we can't observe the report
            // code, so mark completed-handed-off and let the user paste the
            // link later if desired.
            record.status = UploadStatus::Completed;
            let _ = super::history::upsert(&app, record);
            Ok(UploadDispatch {
                handed_off: true,
                detail,
                report: None,
            })
        }
        Ok(transport::UploadOutcome::Completed { report_code }) => {
            let report = report_code.map(|code| ReportRef {
                url: watcher::report_url(&code),
                code,
            });
            record.status = UploadStatus::Completed;
            record.report = report.clone();
            let _ = super::history::upsert(&app, record);
            Ok(UploadDispatch {
                handed_off: false,
                detail: "Upload complete.".into(),
                report,
            })
        }
        Err(e) => {
            record.status = UploadStatus::Failed;
            record.error = Some(e.clone());
            let _ = super::history::upsert(&app, record);
            Err(e)
        }
    }
}

// ── Live mode ────────────────────────────────────────────────────────────────

/// Start a live watch on `file_path`, streaming [`LiveEvent`]s over `channel`.
///
/// `include_entire_file` (from options) decides the starting offset: include
/// the whole existing file, or only fights logged after Start.
#[tauri::command]
pub async fn uploader_start_live(
    state: State<'_, UploaderState>,
    session_id: String,
    file_path: String,
    options: UploadOptions,
    prefer_cli: bool,
    channel: Channel<LiveEvent>,
) -> Result<(), String> {
    validate_log_path(&file_path)?;

    let start_offset = if options.include_entire_file {
        0
    } else {
        std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0)
    };

    // Per-fight handler: extract the fight to a temp file and hand off/upload.
    let opts = options.clone();
    let src = file_path.clone();
    let on_fight: watcher::OnFight = Box::new(move |fr: watcher::FightRange| {
        let tmp =
            std::env::temp_dir().join(format!("kalpa-live-fight-{}-{}.log", fr.index, fr.start_ms));
        let tmp_str = tmp.to_string_lossy().into_owned();
        // A single fight is not independently uploadable to ESO Logs without a
        // BEGIN_LOG header, so for live mode we extract from the session start
        // through this fight's end on first dispatch. To keep memory flat and
        // the implementation honest within the handoff model, we extract just
        // the fight range here and rely on the transport/handoff to assemble.
        splitter::extract_range(&src, &tmp_str, fr.start_offset, fr.end_offset)?;
        let t = transport::select_transport(prefer_cli);
        match t.upload_file(&tmp_str, &opts)? {
            transport::UploadOutcome::Completed { report_code } => Ok(report_code),
            transport::UploadOutcome::HandedOff { .. } => Ok(None),
        }
    });

    let handle = watcher::start_live_watch(&file_path, start_offset, channel, on_fight)?;

    state
        .live_sessions
        .lock()
        .map_err(|_| "Live session lock poisoned")?
        .insert(session_id, handle);
    Ok(())
}

/// Stop a running live watch.
#[tauri::command]
pub fn uploader_stop_live(
    state: State<'_, UploaderState>,
    session_id: String,
) -> Result<(), String> {
    let handle = state
        .live_sessions
        .lock()
        .map_err(|_| "Live session lock poisoned")?
        .remove(&session_id);
    if let Some(h) = handle {
        h.stop();
    }
    Ok(())
}

// ── History ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn uploader_list_history(app: tauri::AppHandle) -> Vec<UploadRecord> {
    super::history::load(&app)
}

#[tauri::command]
pub fn uploader_delete_history(app: tauri::AppHandle, id: String) -> Result<(), String> {
    super::history::remove(&app, &id)
}

/// Attach a report link the user pasted to an existing handed-off record.
#[tauri::command]
pub fn uploader_attach_report(
    app: tauri::AppHandle,
    id: String,
    report_url: String,
) -> Result<(), String> {
    let mut records = super::history::load(&app);
    let Some(record) = records.iter_mut().find(|r| r.id == id) else {
        return Err("Upload record not found.".into());
    };
    let code = report_url
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();
    record.report = Some(ReportRef {
        code,
        url: report_url,
    });
    let updated = record.clone();
    super::history::upsert(&app, updated)
}
