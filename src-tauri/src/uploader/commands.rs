//! Tauri command handlers and managed state for the uploader.
//!
//! Follows the project convention: `async` commands offload blocking work
//! (filesystem, process spawn) onto `spawn_blocking` and return `Result<T,
//! String>`. Every caller-supplied path is canonicalized and confined to the ESO
//! `Logs` directory (or an app-owned output root) before any IO, mirroring the
//! `require_allowed_path` model in `commands.rs` so a compromised webview cannot
//! target arbitrary files or trigger outbound UNC/SMB connections.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use tauri::ipc::Channel;
use tauri::{Manager, State};

use super::types::*;
use super::watcher::{LiveEvent, LiveWatchHandle};
use super::{discovery, scanner, splitter, transport, watcher};
use crate::AllowedAddonsPath;

/// Managed state: active live-watch sessions keyed by session id.
#[derive(Default)]
pub struct UploaderState {
    pub live_sessions: Arc<Mutex<HashMap<String, LiveWatchHandle>>>,
}

// ── Path confinement ─────────────────────────────────────────────────────────

/// Reject Windows UNC / verbatim path prefixes (`\\server\share`, `\\?\…`),
/// which can trigger outbound SMB auth (NetNTLM credential theft) and bypass
/// normal drive-rooted assumptions.
fn has_unc_or_verbatim_prefix(p: &Path) -> bool {
    matches!(p.components().next(), Some(Component::Prefix(prefix)) if {
        use std::path::Prefix::*;
        matches!(
            prefix.kind(),
            Verbatim(_) | VerbatimUNC(_, _) | VerbatimDisk(_) | UNC(_, _)
        )
    })
}

/// Resolve the ESO `Logs` directory (the sibling of the approved AddOns dir).
fn logs_root(allowed: &State<'_, AllowedAddonsPath>) -> Result<PathBuf, String> {
    let guard = allowed
        .0
        .lock()
        .map_err(|_| "Failed to read addons path".to_string())?;
    let approved = guard
        .as_ref()
        .ok_or_else(|| "Set your AddOns folder first.".to_string())?;
    let logs = approved
        .canonical
        .parent()
        .map(|p| p.join("Logs"))
        .ok_or_else(|| "Could not resolve the Logs directory.".to_string())?;
    // Canonicalize if it exists; otherwise return the expected path (the dir may
    // not exist yet until logging is enabled — containment checks below still
    // compare against this lexical root).
    Ok(logs.canonicalize().unwrap_or(logs))
}

/// Validate that `path` is a `.log` file confined to the ESO Logs directory.
/// Canonicalizes to resolve symlinks/junctions and rejects UNC/verbatim paths.
fn confine_log_path(allowed: &State<'_, AllowedAddonsPath>, path: &str) -> Result<(), String> {
    let p = Path::new(path);

    if has_unc_or_verbatim_prefix(p) {
        return Err("Network and special paths are not allowed.".into());
    }

    let is_log = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("log"))
        .unwrap_or(false);
    if !is_log {
        return Err("Only .log files can be processed.".into());
    }

    let root = logs_root(allowed)?;
    // The file must exist to be read; canonicalize resolves symlinks/`..`.
    let canonical = p
        .canonicalize()
        .map_err(|_| "That log file could not be found in your Logs folder.".to_string())?;
    if !canonical.starts_with(&root) {
        return Err("Log files must live in your ESO Logs folder.".into());
    }
    Ok(())
}

/// App-owned output root for split files: `<app_data>/uploader-splits`.
fn split_output_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Could not resolve app data dir: {e}"))?
        .join("uploader-splits");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create output dir: {e}"))?;
    Ok(dir)
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

/// List `*.log` files in the ESO Logs directory (newest first). The directory is
/// confined to the Logs root, so an arbitrary `logs_dir` cannot be enumerated.
#[tauri::command]
pub async fn uploader_list_logs(
    allowed: State<'_, AllowedAddonsPath>,
    logs_dir: String,
) -> Result<Vec<LogFileInfo>, String> {
    let root = logs_root(&allowed)?;
    let requested = Path::new(&logs_dir);
    if has_unc_or_verbatim_prefix(requested) {
        return Err("Network and special paths are not allowed.".into());
    }
    let canonical = requested
        .canonicalize()
        .map_err(|_| "That folder could not be found.".to_string())?;
    if !canonical.starts_with(&root) {
        return Err("Only your ESO Logs folder can be listed.".into());
    }
    tokio::task::spawn_blocking(move || discovery::list_log_files(&logs_dir))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

/// Preflight a log file: sessions, fights, and a split recommendation. Runs a
/// single streaming scan; the fight list is included so the UI doesn't need a
/// second scan (it is omitted for very large files to bound IPC payload size —
/// the counts in `sessions`/`total_fights` still populate the UI).
#[tauri::command]
pub async fn uploader_preflight(
    allowed: State<'_, AllowedAddonsPath>,
    file_path: String,
) -> Result<LogPreflight, String> {
    confine_log_path(&allowed, &file_path)?;
    tokio::task::spawn_blocking(move || {
        let size_bytes = std::fs::metadata(&file_path)
            .map_err(|e| format!("Failed to read file: {e}"))?
            .len();
        let scan = scanner::scan_file(&file_path)?;
        let total_fights = scan.fights.len();
        let recommend_split = size_bytes > scanner::SPLIT_RECOMMEND_BYTES;
        // Avoid shipping a huge fight list over IPC for oversized logs; the
        // counts still drive the UI, and the user splits before reviewing.
        let fights = if recommend_split {
            Vec::new()
        } else {
            scan.fights
        };
        Ok::<_, String>(LogPreflight {
            path: file_path,
            size_bytes,
            sessions: scan.sessions,
            total_fights,
            fights,
            recommend_split,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// ── Split to disk ──────────────────────────────────────────────────────────

/// Split an oversized log into one file per session inside an app-owned output
/// directory. The destination is not caller-controlled, so a compromised
/// webview cannot write outside the app's split folder.
#[tauri::command]
pub async fn uploader_split_to_disk(
    app: tauri::AppHandle,
    allowed: State<'_, AllowedAddonsPath>,
    file_path: String,
) -> Result<Vec<String>, String> {
    confine_log_path(&allowed, &file_path)?;
    let out_root = split_output_root(&app)?;
    // Each split goes in its own timestamped subfolder so repeated splits of
    // different logs don't collide.
    let out_dir = out_root.join(format!("split-{}", now_ms()));
    let out_str = out_dir.to_string_lossy().into_owned();
    tokio::task::spawn_blocking(move || splitter::split_by_session(&file_path, &out_str))
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
///
/// `fight_count` is supplied by the UI from the preflight it already ran, so we
/// don't re-scan a multi-GB log just to fill the history record. If omitted
/// (`None`) we fall back to a scan.
#[tauri::command]
pub async fn uploader_upload_log(
    app: tauri::AppHandle,
    allowed: State<'_, AllowedAddonsPath>,
    file_path: String,
    options: UploadOptions,
    prefer_cli: bool,
    fight_count: Option<usize>,
) -> Result<UploadDispatch, String> {
    confine_log_path(&allowed, &file_path)?;

    let file_name = Path::new(&file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Encounter.log")
        .to_string();

    // Use the preflight count if the UI supplied it; only re-scan as a fallback.
    let fight_count = match fight_count {
        Some(c) => c,
        None => {
            let scan_path = file_path.clone();
            tokio::task::spawn_blocking(move || {
                scanner::scan_file(&scan_path)
                    .map(|s| s.fights.len())
                    .unwrap_or(0)
            })
            .await
            .unwrap_or(0)
        }
    };

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

/// Start live logging on `file_path`.
///
/// The actual upload is performed **once** by handing the whole `Encounter.log`
/// to the official ESO Logs uploader with real-time uploading enabled — the
/// official client tails the file and streams fights itself, which is the only
/// way to produce a valid report (a lone fight slice has no `BEGIN_LOG` header
/// or actor context). The watcher runs purely for the UI's per-fight timeline,
/// streaming [`LiveEvent`]s over `channel`.
///
/// `include_entire_file` controls only what the UI timeline backfills; the
/// official uploader is launched with `--include-entire-file` accordingly.
// Each parameter is a distinct injected dependency (app, state, allowed,
// channel) or required user input — they cannot be meaningfully grouped.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn uploader_start_live(
    app: tauri::AppHandle,
    state: State<'_, UploaderState>,
    allowed: State<'_, AllowedAddonsPath>,
    session_id: String,
    file_path: String,
    options: UploadOptions,
    prefer_cli: bool,
    channel: Channel<LiveEvent>,
) -> Result<UploadDispatch, String> {
    confine_log_path(&allowed, &file_path)?;

    let file_name = Path::new(&file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Encounter.log")
        .to_string();

    // Hand the whole file to the official uploader once, with real-time on.
    let mut live_opts = options.clone();
    live_opts.real_time = true;
    let dispatch_path = file_path.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let t = transport::select_transport(prefer_cli);
        t.upload_file(&dispatch_path, &live_opts)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?;

    // A failed launch is fatal for the session; a handoff is the expected path.
    let (report, handed_off, detail) = match outcome {
        Ok(transport::UploadOutcome::Completed { report_code }) => (
            report_code.map(|code| ReportRef {
                url: watcher::report_url(&code),
                code,
            }),
            false,
            "Live logging started.".to_string(),
        ),
        Ok(transport::UploadOutcome::HandedOff { detail }) => (None, true, detail),
        Err(e) => return Err(e),
    };

    // Record the live session in history so a report link can be attached later.
    let record_id = format!("{}-{}", now_ms(), session_id);
    let record = UploadRecord {
        id: record_id,
        source_path: file_path.clone(),
        file_name,
        created_at_ms: now_ms(),
        status: UploadStatus::Live,
        mode: UploadMode::Live,
        visibility: options.visibility,
        fight_count: 0,
        report: report.clone(),
        error: None,
    };
    let _ = super::history::upsert(&app, record);

    // The UI timeline starts from the current EOF unless the user asked to
    // backfill earlier fights.
    let start_offset = if options.include_entire_file {
        0
    } else {
        std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0)
    };

    let handle = watcher::start_live_watch(&file_path, start_offset, channel)?;

    // Stop any prior handle reused under the same id before replacing it.
    let mut sessions = state
        .live_sessions
        .lock()
        .map_err(|_| "Live session lock poisoned")?;
    if let Some(prev) = sessions.insert(session_id, handle) {
        prev.stop();
    }

    Ok(UploadDispatch {
        handed_off,
        detail,
        report,
    })
}

/// Stop a running live watch (the official uploader keeps its own session).
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
    // Only accept esologs.com report URLs.
    let trimmed = report_url.trim();
    let is_report = (trimmed.starts_with("https://www.esologs.com/reports/")
        || trimmed.starts_with("https://esologs.com/reports/"))
        && trimmed.len() < 256;
    if !is_report {
        return Err("Enter a valid esologs.com report link.".into());
    }

    let mut records = super::history::load(&app);
    let Some(record) = records.iter_mut().find(|r| r.id == id) else {
        return Err("Upload record not found.".into());
    };
    let code = trimmed
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();
    record.report = Some(ReportRef {
        code,
        url: trimmed.to_string(),
    });
    let updated = record.clone();
    super::history::upsert(&app, updated)
}
