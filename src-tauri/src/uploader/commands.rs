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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::ipc::Channel;
use tauri::{Manager, State};

use super::types::*;
use super::watcher::{LiveEvent, LiveWatchHandle};
use super::{discovery, scanner, splitter, transport, watcher};
use crate::AllowedAddonsPath;

/// A live session slot. A session is registered as `Starting` *before* the
/// (blocking) uploader handoff so a concurrent stop/unmount during that window
/// is observed; it transitions to `Running` once the watcher thread exists.
enum LiveSlot {
    /// Start is in flight. `cancelled` is set if a stop arrives before the
    /// watcher is registered, so the start can abort cleanly.
    Starting(Arc<AtomicBool>),
    Running(LiveWatchHandle),
}

/// Managed state: active live-watch sessions keyed by session id.
#[derive(Default)]
pub struct UploaderState {
    live_sessions: Mutex<HashMap<String, LiveSlot>>,
}

// ── Path confinement ─────────────────────────────────────────────────────────

/// Reject Windows UNC path prefixes (`\\server\share`, `\\?\UNC\…`), which can
/// trigger outbound SMB auth (NetNTLM credential theft). `VerbatimDisk`
/// (`\\?\C:\…`) is deliberately *allowed*: it is a harmless drive-rooted form and
/// is exactly what `std::fs::canonicalize` emits on Windows. We canonicalize with
/// `dunce` below so confined paths stay in drive-letter form, but a stray
/// verbatim-disk prefix arriving from the frontend must not be rejected — that
/// was the bug that broke every log selection. The non-verbatim `Verbatim(_)`
/// arm covers device namespaces (`\\?\GLOBALROOT\…`, `\\.\…`) which are not
/// legitimate log locations.
fn has_unc_or_verbatim_prefix(p: &Path) -> bool {
    matches!(p.components().next(), Some(Component::Prefix(prefix)) if {
        use std::path::Prefix::*;
        matches!(
            prefix.kind(),
            Verbatim(_) | VerbatimUNC(_, _) | UNC(_, _)
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
    // compare against this lexical root). `dunce` keeps the result in drive-letter
    // form so it prefix-matches the (also dunce-canonicalized) file paths.
    Ok(dunce::canonicalize(&logs).unwrap_or(logs))
}

/// Validate that `path` is a `.log` file confined to the ESO Logs directory and
/// return the **canonical** path. Callers must do all subsequent IO on the
/// returned path (not the raw caller string) so the bytes opened are the ones
/// that passed confinement — closing the check-then-open (TOCTOU) window where a
/// junction/symlink could be repointed between validation and use.
fn confine_log_path(allowed: &State<'_, AllowedAddonsPath>, path: &str) -> Result<PathBuf, String> {
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
    // The file must exist to be read; canonicalize resolves symlinks/`..`. Use
    // `dunce` so the canonical form is drive-letter (not verbatim `\\?\`), keeping
    // it consistent with `root` and safe to round-trip back through the frontend.
    let canonical = dunce::canonicalize(p)
        .map_err(|_| "That log file could not be found in your Logs folder.".to_string())?;
    if !canonical.starts_with(&root) {
        return Err("Log files must live in your ESO Logs folder.".into());
    }
    Ok(canonical)
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
    let canonical = dunce::canonicalize(requested)
        .map_err(|_| "That folder could not be found.".to_string())?;
    if !canonical.starts_with(&root) {
        return Err("Only your ESO Logs folder can be listed.".into());
    }
    // Enumerate the canonical path (not the raw caller string) so the directory
    // read targets exactly what passed confinement — see confine_log_path.
    let dir = canonical.to_string_lossy().into_owned();
    tokio::task::spawn_blocking(move || discovery::list_log_files(&dir))
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
    let safe = confine_log_path(&allowed, &file_path)?;
    let safe = safe.to_string_lossy().into_owned();
    tokio::task::spawn_blocking(move || {
        let size_bytes = std::fs::metadata(&safe)
            .map_err(|e| format!("Failed to read file: {e}"))?
            .len();
        let scan = scanner::scan_file(&safe)?;
        let total_fights = scan.fights.len();
        let recommend_split = size_bytes > scanner::SPLIT_RECOMMEND_BYTES;
        // Don't ship a huge fight list over IPC: bound by fight COUNT (a dense
        // sub-512-MiB log can still hold thousands of fights, which would be a
        // ~MB payload + thousands of DOM rows). `total_fights` still drives the
        // count pills, so omitting the list is safe.
        const MAX_SHIPPED_FIGHTS: usize = 500;
        let fights = if recommend_split || total_fights > MAX_SHIPPED_FIGHTS {
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
    sessions: Option<Vec<LogSession>>,
) -> Result<Vec<String>, String> {
    let safe = confine_log_path(&allowed, &file_path)?
        .to_string_lossy()
        .into_owned();
    let out_root = split_output_root(&app)?;
    // Each split goes in its own timestamped subfolder so repeated splits of
    // different logs don't collide.
    let out_dir = out_root.join(format!("split-{}", now_ms()));
    let out_str = out_dir.to_string_lossy().into_owned();
    // Reuse the preflight's sessions (the UI passes them) to avoid a second full
    // scan of a multi-GB file; fall back to scanning when not supplied.
    tokio::task::spawn_blocking(move || splitter::split_by_session(&safe, &out_str, sessions))
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
    let safe = confine_log_path(&allowed, &file_path)?
        .to_string_lossy()
        .into_owned();

    let file_name = Path::new(&safe)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Encounter.log")
        .to_string();

    // Use the preflight count if the UI supplied it; only re-scan as a fallback.
    // The count is for the history record only and never gates the upload, so a
    // scan failure degrades to 0 — but we log it rather than swallowing silently.
    let fight_count = match fight_count {
        Some(c) => c,
        None => {
            let scan_path = safe.clone();
            tokio::task::spawn_blocking(move || scanner::scan_file(&scan_path))
                .await
                .map_err(|e| format!("Fight-count task failed: {e}"))
                .and_then(|r| r)
                .map(|s| s.fights.len())
                .unwrap_or_else(|e| {
                    eprintln!("[uploader] fight count scan failed: {e}");
                    0
                })
        }
    };

    let record_id = super::history::next_record_id(now_ms(), &file_name);
    let mut record = UploadRecord {
        id: record_id.clone(),
        source_path: safe.clone(),
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

    // Force one-shot semantics: the persisted options blob may carry live-only
    // flags (real_time / include_entire_file) left over from a prior live
    // session, which would otherwise turn this manual upload into a fire-and-
    // forget real-time launch.
    let mut opts = options.clone();
    opts.real_time = false;
    opts.include_entire_file = false;
    let dispatch_path = safe.clone();
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
    channel: Channel<LiveEvent>,
) -> Result<UploadDispatch, String> {
    let safe = confine_log_path(&allowed, &file_path)?
        .to_string_lossy()
        .into_owned();

    let file_name = Path::new(&safe)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Encounter.log")
        .to_string();

    // Live mode genuinely requires the official uploader: only it can stream a
    // running log in real time (a lone fight slice has no BEGIN_LOG header). The
    // GUI-handoff fallback would just open a download page while Kalpa shows a
    // convincing-but-fake live timeline — so refuse rather than no-op.
    if transport::find_official_uploader().is_none() {
        return Err(
            "Live logging needs the ESO Logs Uploader installed. Install \
                    it, or use \"Upload a Log\" after your session instead."
                .into(),
        );
    }

    // Register a Starting slot BEFORE the blocking handoff so a stop/unmount
    // during that window is observed and the watcher is never orphaned. Replace
    // (and stop) any existing slot under this id.
    let cancelled = Arc::new(AtomicBool::new(false));
    let prev = {
        let mut sessions = state
            .live_sessions
            .lock()
            .map_err(|_| "Live session lock poisoned")?;
        sessions.insert(
            session_id.clone(),
            LiveSlot::Starting(Arc::clone(&cancelled)),
        )
    };
    if let Some(prev) = prev {
        stop_slot(prev);
    }

    // Hand the whole file to the official uploader once, with real-time on.
    // Live MUST use the CLI transport (which passes --enable-real-time-uploading);
    // the GUI handoff would open the uploader in one-shot mode while we show a
    // fake "LIVE" timeline. We already confirmed the uploader is installed above,
    // but detection can still fail (e.g. removed between the check and here), in
    // which case we error rather than silently falling back to a one-shot launch.
    let mut live_opts = options.clone();
    live_opts.real_time = true;
    let dispatch_path = safe.clone();
    let outcome = tokio::task::spawn_blocking(move || match transport::CliTransport::detect() {
        Some(cli) => {
            use transport::LogUploadTransport;
            cli.upload_file(&dispatch_path, &live_opts)
        }
        None => Err("The ESO Logs Uploader could not be launched for live logging.".to_string()),
    })
    .await
    .map_err(|e| format!("Task failed: {e}"));

    // On any failure (or task panic), vacate our Starting slot so it can't leak.
    let outcome = match outcome {
        Ok(o) => o,
        Err(e) => {
            remove_own_slot(&state, &session_id, &cancelled);
            return Err(e);
        }
    };
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
        Err(e) => {
            remove_own_slot(&state, &session_id, &cancelled);
            return Err(e);
        }
    };

    // Record the live session in history so a report link can be attached later.
    let record_id = super::history::next_record_id(now_ms(), &session_id);
    let record = UploadRecord {
        id: record_id,
        source_path: safe.clone(),
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
    // The UI timeline always starts at the current EOF. We deliberately do NOT
    // replay the whole file through the watcher when `include_entire_file` is
    // set — that would feed a multi-GB backlog through the tail loop just to
    // populate a display timeline. The official uploader already got the
    // --include-entire-file flag and handles the real historical upload itself.
    let start_offset = std::fs::metadata(&safe).map(|m| m.len()).unwrap_or(0);

    // Mirror the other fallible arms: vacate our Starting slot on failure (the
    // file can be rotated/deleted during the handoff above) so it doesn't leak.
    let handle = match watcher::start_live_watch(&safe, start_offset, channel) {
        Ok(h) => h,
        Err(e) => {
            remove_own_slot(&state, &session_id, &cancelled);
            return Err(e);
        }
    };

    // Promote Starting → Running, unless a stop arrived mid-start (cancelled set,
    // or our slot was replaced/removed): in that case stop the just-started
    // watcher immediately so nothing is orphaned.
    let promote = {
        let mut sessions = state
            .live_sessions
            .lock()
            .map_err(|_| "Live session lock poisoned")?;
        let still_ours = matches!(
            sessions.get(&session_id),
            Some(LiveSlot::Starting(c)) if Arc::ptr_eq(c, &cancelled)
        );
        if still_ours && !cancelled.load(Ordering::SeqCst) {
            sessions.insert(session_id, LiveSlot::Running(handle));
            None
        } else {
            // Leave any newer slot alone; just stop our now-unwanted watcher.
            Some(handle)
        }
    };
    if let Some(handle) = promote {
        handle.stop();
    }

    Ok(UploadDispatch {
        handed_off,
        detail,
        report,
    })
}

/// Stop a slot's watcher (if it has one). `Starting` slots have no thread yet,
/// but their cancel flag is set so the in-flight start aborts on promotion.
fn stop_slot(slot: LiveSlot) {
    match slot {
        LiveSlot::Starting(cancelled) => cancelled.store(true, Ordering::SeqCst),
        LiveSlot::Running(handle) => handle.stop(),
    }
}

/// Remove our own Starting slot on a failed start, but only if it's still ours
/// (a newer start under the same id may have replaced it).
fn remove_own_slot(
    state: &State<'_, UploaderState>,
    session_id: &str,
    cancelled: &Arc<AtomicBool>,
) {
    if let Ok(mut sessions) = state.live_sessions.lock() {
        let ours = matches!(
            sessions.get(session_id),
            Some(LiveSlot::Starting(c)) if Arc::ptr_eq(c, cancelled)
        );
        if ours {
            sessions.remove(session_id);
        }
    }
}

/// Stop a running (or starting) live watch (the official uploader keeps its own
/// session running regardless).
#[tauri::command]
pub fn uploader_stop_live(
    state: State<'_, UploaderState>,
    session_id: String,
) -> Result<(), String> {
    let slot = state
        .live_sessions
        .lock()
        .map_err(|_| "Live session lock poisoned")?
        .remove(&session_id);
    if let Some(slot) = slot {
        stop_slot(slot);
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
    let trimmed = report_url.trim();
    if trimmed.len() >= 256 {
        return Err("Enter a valid esologs.com report link.".into());
    }
    // Strip the known report prefix and validate the remaining code is a plain
    // alphanumeric report id — rejecting query/fragment/traversal segments.
    let code = trimmed
        .strip_prefix("https://www.esologs.com/reports/")
        .or_else(|| trimmed.strip_prefix("https://esologs.com/reports/"))
        .map(|rest| rest.trim_end_matches('/'))
        .filter(|code| !code.is_empty() && code.chars().all(|c| c.is_ascii_alphanumeric()))
        .ok_or_else(|| "Enter a valid esologs.com report link.".to_string())?;

    let mut records = super::history::load(&app);
    let Some(record) = records.iter_mut().find(|r| r.id == id) else {
        return Err("Upload record not found.".into());
    };
    // Build the canonical URL from the validated code (matches the other two
    // ReportRef construction sites).
    record.report = Some(ReportRef {
        url: watcher::report_url(code),
        code: code.to_string(),
    });
    let updated = record.clone();
    super::history::upsert(&app, updated)
}
