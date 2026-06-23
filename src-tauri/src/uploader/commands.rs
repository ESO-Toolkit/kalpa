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

/// Internal sentinel returned by the live-handoff closure when it observes the
/// cancel flag set *before* it launches the official uploader. Distinguishes a
/// clean pre-launch cancellation (nothing spawned) from a real launch failure,
/// so the caller can report it as cancelled rather than an error. Not shown to
/// the user — the caller maps it to a friendly message.
const LIVE_CANCELLED_BEFORE_LAUNCH: &str = "__live_cancelled_before_launch__";

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

/// Reject Windows UNC and device-namespace path prefixes, which can trigger
/// outbound SMB auth (NetNTLM credential theft) or reach raw devices. The
/// dangerous forms are `\\server\share` (`UNC`), `\\?\UNC\…` (`VerbatimUNC`),
/// `\\?\GLOBALROOT\…` (`Verbatim`), and the entire `\\.\…` device namespace
/// (`DeviceNS`: `\\.\UNC\…`, `\\.\C:\…`, `\\.\PhysicalDrive0`, `\\.\pipe\…`).
/// `\\.\UNC\host\share\…` in particular makes `dunce::canonicalize` (below)
/// attempt SMB name resolution *before* the containment check can reject it, so
/// it must be blocked here at the prefix.
///
/// `VerbatimDisk` (`\\?\C:\…`) is deliberately *allowed*: it is a harmless
/// drive-rooted form and is exactly what `std::fs::canonicalize` emits on
/// Windows. We canonicalize with `dunce` below so confined paths stay in
/// drive-letter form, but a stray verbatim-disk prefix arriving from the
/// frontend must not be rejected — that was the bug that broke every log
/// selection.
fn has_unc_or_verbatim_prefix(p: &Path) -> bool {
    matches!(p.components().next(), Some(Component::Prefix(prefix)) if {
        use std::path::Prefix::*;
        matches!(
            prefix.kind(),
            Verbatim(_) | VerbatimUNC(_, _) | UNC(_, _) | DeviceNS(_)
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

/// Keep at most this many `split-*` output folders; older ones are pruned.
const KEEP_SPLIT_FOLDERS: usize = 3;

/// Remove the oldest `split-*` folders, keeping the `keep` most recent. Split
/// output is full-byte copies of (multi-GB) logs; without pruning, repeated
/// splits would accumulate in app data forever. Best-effort: errors are logged,
/// never propagated, and the prune runs before a new split so the just-created
/// folder is always retained. Mirrors `prune_auto_snapshots` in commands.rs.
fn prune_split_folders(root: &Path, keep: usize) {
    let prefix = "split-";
    let mut dirs: Vec<_> = match std::fs::read_dir(root) {
        Ok(rd) => rd
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(prefix) && e.path().is_dir())
            .collect(),
        Err(_) => return,
    };
    if dirs.len() <= keep {
        return;
    }
    // Names embed epoch-ms timestamps (constant 13-digit width through year
    // 2286), so lexicographic order == chronological order.
    dirs.sort_by_key(|e| e.file_name());
    let to_remove = dirs.len() - keep;
    for entry in dirs.into_iter().take(to_remove) {
        if let Err(e) = std::fs::remove_dir_all(entry.path()) {
            eprintln!(
                "Warning: failed to prune old split folder {:?}: {}",
                entry.path(),
                e
            );
        }
    }
}

/// App-owned recycle bin for deleted logs: `<app_data>/uploader-recycle`. A
/// deleted log is MOVED here (soft delete) rather than unlinked, because combat
/// logs are irreplaceable; it can be restored for [`RECYCLE_KEEP_DAYS`] days.
fn recycle_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Could not resolve app data dir: {e}"))?
        .join("uploader-recycle");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create recycle dir: {e}"))?;
    Ok(dir)
}

/// How long a soft-deleted log stays restorable before prune removes it.
const RECYCLE_KEEP_DAYS: u64 = 30;

/// Remove recycled files older than [`RECYCLE_KEEP_DAYS`]. Retention is based on
/// the DELETION time encoded in the recycle file name (`<epoch-ms>-<stem>.log`),
/// NOT the file's `modified()` mtime: a same-volume rename preserves the log's
/// original mtime, so an old archive deleted today would otherwise be pruned
/// immediately, breaking the restore window. Falls back to `modified()` only if
/// the name has no parseable prefix. Best-effort: errors logged, never propagated.
fn prune_recycle_folder(root: &Path) {
    let cutoff_ms = RECYCLE_KEEP_DAYS * 24 * 60 * 60 * 1000;
    let now = now_ms();
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        // Prefer the deletion timestamp from the file name.
        let deleted_at_ms = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.split_once('-'))
            .and_then(|(ts, _)| ts.parse::<u64>().ok());
        let too_old = match deleted_at_ms {
            Some(ts) => now.saturating_sub(ts) > cutoff_ms,
            // Fallback: no parseable prefix → use mtime age.
            None => entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|mt| std::time::SystemTime::now().duration_since(mt).ok())
                .map(|age| age.as_millis() as u64 > cutoff_ms)
                .unwrap_or(false),
        };
        if too_old {
            if let Err(e) = std::fs::remove_file(&path) {
                eprintln!("Warning: failed to prune recycled log {path:?}: {e}");
            }
        }
    }
}

/// Move a file from `src` to `dst` (which the caller has reserved as a fresh,
/// non-existing name). A plain rename is preferred; we fall back to copy+remove
/// ONLY when the rename failed because the two paths are on different volumes
/// (the app-data recycle bin can sit on a different drive than the user's Logs
/// folder). Any other rename error is surfaced as-is rather than masked by a
/// blind copy. If the post-copy source removal fails, the partial destination is
/// cleaned up so a failed move never leaves a duplicate behind.
fn move_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        // CrossesDevices is the portable EXDEV signal; some platforms report it
        // as a raw error code, so also accept a generic Other as a cross-volume
        // candidate. Permission/NotFound/etc. are NOT cross-volume — re-raise.
        Err(e)
            if e.kind() == std::io::ErrorKind::CrossesDevices
                || e.kind() == std::io::ErrorKind::Other =>
        {
            std::fs::copy(src, dst)?;
            if let Err(rm) = std::fs::remove_file(src) {
                // Couldn't remove the source after copying — don't leave a
                // duplicate in the destination; roll back the copy.
                let _ = std::fs::remove_file(dst);
                return Err(rm);
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Validate user-supplied upload options before they reach the official
/// uploader's CLI. `region` is the one user-controlled argv integer with no
/// downstream allowlist (the transport forwards it verbatim), so a buggy/
/// compromised webview could pass an unsupported value; reject anything but the
/// two real megaservers. Mirrors the numeric `guild_id` allowlist in transport.
fn validate_upload_options(opts: &UploadOptions) -> Result<(), String> {
    // 1 = NA/US, 2 = EU. These are the only meaningful Personal Logs regions.
    if !matches!(opts.region, 1 | 2) {
        return Err("Unsupported region (choose NA or EU).".into());
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
    // Prune old split folders before creating the new one so the total stays at
    // KEEP_SPLIT_FOLDERS (these hold full multi-GB copies — see prune_split_folders).
    prune_split_folders(&out_root, KEEP_SPLIT_FOLDERS.saturating_sub(1));
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

/// Import a `.log` file that lives OUTSIDE the ESO Logs folder (e.g. dropped from
/// the Desktop or Downloads) by copying it into the Logs folder, then returning
/// the new in-folder path. Everything downstream (preflight, upload, split) still
/// runs the existing `confine_log_path` guard, so the imported copy is treated
/// exactly like any other log in the folder.
///
/// Validation: the source must be an existing `.log` file with no UNC/verbatim/
/// device prefix (same rejections as `confine_log_path`). The destination name is
/// the source file's own name, sanitized to a single safe segment and made
/// collision-free in the Logs folder — the caller never controls the directory.
#[tauri::command]
pub async fn uploader_import_log(
    allowed: State<'_, AllowedAddonsPath>,
    src_path: String,
) -> Result<String, String> {
    let src = Path::new(&src_path);
    if has_unc_or_verbatim_prefix(src) {
        return Err("Network and special paths are not allowed.".into());
    }
    let is_log = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("log"))
        .unwrap_or(false);
    if !is_log {
        return Err("Only .log files can be imported.".into());
    }
    // Resolve the real source (rejects a dangling path / missing file).
    let canonical_src =
        dunce::canonicalize(src).map_err(|_| "That file could not be found.".to_string())?;
    if !canonical_src.is_file() {
        return Err("That isn't a file.".into());
    }

    let root = logs_root(&allowed)?;
    std::fs::create_dir_all(&root).map_err(|e| format!("Could not access the Logs folder: {e}"))?;

    // If the file is ALREADY inside the Logs folder, just use it in place — no
    // copy needed (e.g. a drop of a file the picker already lists).
    if canonical_src.starts_with(&root) {
        return Ok(canonical_src.to_string_lossy().into_owned());
    }

    // Build a safe destination name from the source's stem + ".log", made unique
    // in the Logs folder so an import never overwrites an existing log.
    let mut stem = canonical_src
        .file_stem()
        .and_then(|s| s.to_str())
        .map(super::splitter::sanitize_split_stem)
        .unwrap_or(None)
        .unwrap_or_else(|| "imported-log".to_string());
    // `Encounter.log` is RESERVED for the live file ESO writes (and which delete
    // refuses to touch). An imported file named Encounter.log must NOT take that
    // name — it would become an un-deletable archive that a later live session
    // could append to. Prefix it so it lands as a distinct importable archive.
    if stem.eq_ignore_ascii_case("Encounter") {
        stem = format!("imported-{stem}");
    }
    let mut candidate = format!("{stem}.log");
    let mut n = 2;
    while root.join(&candidate).exists() {
        candidate = format!("{stem}-{n}.log");
        n += 1;
        if n > 1000 {
            return Err("Too many imported copies — clean up the Logs folder.".into());
        }
    }
    let dst = root.join(&candidate);

    // Stream the copy on a blocking thread (logs can be multi-GB).
    let dst_str = dst.to_string_lossy().into_owned();
    tokio::task::spawn_blocking(move || {
        std::fs::copy(&canonical_src, &dst).map_err(|e| {
            use std::io::ErrorKind;
            // Windows Controlled Folder Access blocks third-party writes into
            // Documents (where the ESO Logs folder lives) and surfaces as
            // PermissionDenied or a misleading NotFound on the destination.
            // Give actionable guidance instead of a raw OS error.
            if matches!(e.kind(), ErrorKind::PermissionDenied | ErrorKind::NotFound) {
                "Couldn't copy the log into your ESO Logs folder. If Windows \
                 Controlled Folder Access is on, allow Kalpa to write there (or \
                 move the log into your Logs folder manually), then try again."
                    .to_string()
            } else {
                format!("Couldn't import the log: {e}")
            }
        })?;
        Ok::<String, String>(dst_str)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Soft-delete a log: MOVE it from the Logs folder into the app-owned recycle
/// bin (kept [`RECYCLE_KEEP_DAYS`] days), returning the recycle path so the UI can
/// offer a one-tap Restore. Never a hard unlink — combat logs are irreplaceable.
///
/// `confine_log_path` is the security boundary: the source must be a `.log` inside
/// the Logs folder (canonical, no UNC/verbatim), exactly like every other
/// destructive/IO command. The recycle file name is timestamp-prefixed so repeated
/// deletes of same-named logs never collide.
#[tauri::command]
pub async fn uploader_delete_log(
    allowed: State<'_, AllowedAddonsPath>,
    app: tauri::AppHandle,
    file_path: String,
) -> Result<String, String> {
    let safe = confine_log_path(&allowed, &file_path)?;

    // Fail CLOSED for the current log: ESO always writes the live stream to the
    // file literally named `Encounter.log`, and may hold it open even during an
    // idle gap between pulls (when mtime looks stale). Never move that file — only
    // rotated archives (`Archive-*.log`) and other named logs are deletable.
    let is_current = safe
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case("Encounter.log"))
        .unwrap_or(false);
    if is_current {
        return Err(
            "This is the current Encounter.log that ESO writes to. Turn off in-game logging \
             first — only archived logs can be deleted."
                .into(),
        );
    }

    // Defence in depth: also refuse any log modified within the active window —
    // it's hot even if not named Encounter.log (e.g. a just-rotated archive). The
    // UI disables delete for active rows; this is the authoritative backend check.
    const DELETE_ACTIVE_WINDOW_MS: u64 = 90 * 1000;
    if let Ok(meta) = std::fs::metadata(&safe) {
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let now = now_ms();
        if modified_ms > 0 && modified_ms <= now && now - modified_ms < DELETE_ACTIVE_WINDOW_MS {
            return Err(
                "This log is still being written. Stop logging in-game (or wait a moment) \
                 before deleting it."
                    .into(),
            );
        }
    }

    let recycle = recycle_root(&app)?;
    // Opportunistically prune expired entries whenever the bin is touched.
    prune_recycle_folder(&recycle);

    let stem = safe
        .file_stem()
        .and_then(|s| s.to_str())
        .map(super::splitter::sanitize_split_stem)
        .unwrap_or(None)
        .unwrap_or_else(|| "log".to_string());
    // Timestamp prefix keeps deletes of identically-named logs distinct and lets
    // restore strip it back to the original stem.
    let mut candidate = format!("{}-{stem}.log", now_ms());
    let mut n = 2;
    while recycle.join(&candidate).exists() {
        candidate = format!("{}-{stem}-{n}.log", now_ms());
        n += 1;
        if n > 1000 {
            return Err("Recycle bin is full — empty it and try again.".into());
        }
    }
    let dst = recycle.join(&candidate);
    let dst_str = dst.to_string_lossy().into_owned();

    tokio::task::spawn_blocking(move || {
        move_file(&safe, &dst).map_err(|e| {
            use std::io::ErrorKind;
            if matches!(e.kind(), ErrorKind::PermissionDenied | ErrorKind::NotFound) {
                "Couldn't delete the log. If Windows Controlled Folder Access is \
                 on, allow Kalpa to manage your Logs folder, then try again."
                    .to_string()
            } else {
                format!("Couldn't delete the log: {e}")
            }
        })?;
        Ok::<String, String>(dst_str)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Restore a soft-deleted log from the recycle bin back into the Logs folder (the
/// undo for [`uploader_delete_log`]). Confinement here checks the RECYCLE root (not
/// the Logs root — `confine_log_path` would reject a recycle file), then writes
/// back into the Logs folder with a collision-safe name. Returns the restored path.
#[tauri::command]
pub async fn uploader_restore_log(
    allowed: State<'_, AllowedAddonsPath>,
    app: tauri::AppHandle,
    recycle_path: String,
) -> Result<String, String> {
    let p = Path::new(&recycle_path);
    if has_unc_or_verbatim_prefix(p) {
        return Err("Network and special paths are not allowed.".into());
    }
    let recycle = recycle_root(&app)?;
    let canonical =
        dunce::canonicalize(p).map_err(|_| "That recycled log could not be found.".to_string())?;
    // The recycle-bin equivalent of confine_log_path: the file must live inside
    // the app-owned recycle root, so a crafted path can't restore arbitrary files.
    if !canonical.starts_with(&recycle) {
        return Err("That file isn't in the recycle bin.".into());
    }

    let root = logs_root(&allowed)?;
    std::fs::create_dir_all(&root).map_err(|e| format!("Could not access the Logs folder: {e}"))?;

    // Strip the leading "<epoch-ms>-" prefix delete added; fall back to sanitize.
    let raw_stem = canonical
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("restored-log");
    let stripped = raw_stem
        .split_once('-')
        .map(|(_, rest)| rest)
        .unwrap_or(raw_stem);
    let stem = super::splitter::sanitize_split_stem(stripped)
        .unwrap_or_else(|| "restored-log".to_string());
    let mut candidate = format!("{stem}.log");
    let mut n = 2;
    while root.join(&candidate).exists() {
        candidate = format!("{stem}-{n}.log");
        n += 1;
        if n > 1000 {
            return Err("Too many copies — clean up the Logs folder.".into());
        }
    }
    let dst = root.join(&candidate);
    let dst_str = dst.to_string_lossy().into_owned();

    tokio::task::spawn_blocking(move || {
        move_file(&canonical, &dst).map_err(|e| {
            use std::io::ErrorKind;
            if matches!(e.kind(), ErrorKind::PermissionDenied | ErrorKind::NotFound) {
                "Couldn't restore the log into your Logs folder. If Windows \
                 Controlled Folder Access is on, allow Kalpa to write there."
                    .to_string()
            } else {
                format!("Couldn't restore the log: {e}")
            }
        })?;
        Ok::<String, String>(dst_str)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Split only the sessions the user selected in the split workbench, naming each
/// from the user's (sanitized) custom name. Like [`uploader_split_to_disk`] the
/// destination is app-owned, not caller-controlled, and every custom name is
/// sanitized to a single safe path segment in the splitter — a compromised
/// webview cannot write outside the split folder or traverse via a crafted name.
#[tauri::command]
pub async fn uploader_split_to_disk_named(
    app: tauri::AppHandle,
    allowed: State<'_, AllowedAddonsPath>,
    file_path: String,
    sessions: Option<Vec<LogSession>>,
    selections: Vec<splitter::SplitSelection>,
) -> Result<Vec<String>, String> {
    let safe = confine_log_path(&allowed, &file_path)?
        .to_string_lossy()
        .into_owned();
    let out_root = split_output_root(&app)?;
    prune_split_folders(&out_root, KEEP_SPLIT_FOLDERS.saturating_sub(1));
    let out_dir = out_root.join(format!("split-{}", now_ms()));
    let out_str = out_dir.to_string_lossy().into_owned();
    tokio::task::spawn_blocking(move || {
        splitter::split_selected(&safe, &out_str, sessions, selections)
    })
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

// ── Native upload session (in-app ESO Logs login) ────────────────────────────

/// Open the in-app ESO Logs sign-in window and capture the upload-session cookie.
///
/// This establishes the **website session** the native `/desktop-client/*`
/// uploader authenticates with (a different credential from the OAuth API token
/// used for Pack Hub). The user logs in on ESO Logs' own page inside a webview
/// Kalpa owns; on success the `laravel_session` cookie is read from that
/// webview's jar and persisted via the shared [`StoredSessionProvider`].
///
/// Returns an [`UploadLoginResult`] whose `sessionPersisted` mirrors
/// `AuthUser.sessionPersisted`, so the frontend can reuse the same memory-only
/// warning. `async` is required: the cookie read deadlocks the WebView2 if run on
/// a synchronous command thread (see `native::login`).
#[tauri::command]
pub async fn uploader_login_esologs(
    app: tauri::AppHandle,
    session: State<'_, std::sync::Arc<super::native::session::StoredSessionProvider>>,
) -> Result<super::native::login::UploadLoginResult, String> {
    // State derefs through the Arc to the provider; `run_login` takes
    // `&StoredSessionProvider`.
    super::native::login::run_login(app, &session)
        .await
        .map_err(|e| e.to_string())
}

/// Whether a native upload session cookie is currently available (signed in for
/// uploads). Does not prove the server still accepts it — only that one is
/// present without prompting a fresh login.
#[tauri::command]
pub fn uploader_has_session(
    session: State<'_, std::sync::Arc<super::native::session::StoredSessionProvider>>,
) -> bool {
    session.has_session()
}

/// Clear the native upload session cookie (sign out of uploads), both in memory
/// and from the credential store.
#[tauri::command]
pub fn uploader_logout_esologs(
    session: State<'_, std::sync::Arc<super::native::session::StoredSessionProvider>>,
) -> Result<(), String> {
    use super::native::session::SessionProvider;
    session.invalidate();
    Ok(())
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
// Each parameter is a distinct injected dependency (app, allowed, session) or
// required user input — they cannot be meaningfully grouped (mirrors
// `uploader_start_live`).
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn uploader_upload_log(
    app: tauri::AppHandle,
    allowed: State<'_, AllowedAddonsPath>,
    session: State<'_, std::sync::Arc<super::native::session::StoredSessionProvider>>,
    file_path: String,
    options: UploadOptions,
    prefer_cli: bool,
    fight_count: Option<usize>,
    // Whether the user has opted into Kalpa's direct (native) upload (the
    // Settings toggle, gated behind a ToS disclosure). `Option` so an
    // older/omitting caller deserializes to `None` (treated as `false`) and never
    // enables native by accident. Native still only runs when the coverage gate
    // ALSO allows it (`FORMAT_VERSION_CONFIRMED` + proven event types), so today
    // this changes the observable routing reason but not the actual transport.
    native_opt_in: Option<bool>,
) -> Result<UploadDispatch, String> {
    validate_upload_options(&options)?;
    // Reconcile prior-run stale records before this upload writes its transient
    // `Uploading` record (same invariant as `uploader_start_live`: reconcile
    // must run while no current-process transient record exists). Unlike live
    // mode there is no cancellable slot/watcher here, so the pre-record position
    // is fine — a one-shot upload has nothing to orphan.
    super::history::reconcile_stale_once(&app);
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

    // Coverage-gated native routing. Native upload runs ONLY when the user opted
    // in, the format is confirmed, AND every event type in this log is within
    // proven byte-exact coverage; otherwise we route to the official uploader.
    // This guarantees the native path never produces a less-accurate report than
    // the official app. Today `FORMAT_VERSION_CONFIRMED` is false, so this always
    // resolves to the official path — wiring it now keeps behavior unchanged
    // while making the safe-routing decision real and observable.
    //
    // Driven by the Settings opt-in toggle (passed from the frontend). Absent →
    // false. With the format-version gate OPEN (native rendering confirmed
    // 2026-06-19), an opted-in user whose log is all proven types routes native; an
    // un-opted-in user, or a log with any unproven event type, falls back to the
    // official uploader. The native path itself also self-checks the built segment
    // and falls back if it is ever malformed (see `run_native_upload`).
    let native_opt_in = native_opt_in.unwrap_or(false);
    let routing = transport::assess_native_routing(&dispatch_path, native_opt_in);
    let use_native = matches!(routing, transport::NativeRouting::Native);
    if let transport::NativeRouting::Fallback(reason) = &routing {
        // Honest diagnostics: why native wasn't used. Logged only (not user-facing
        // noise).
        eprintln!("[uploader] native routing → official: {}", reason.explain());
    }

    let outcome = if use_native {
        // Native path: build the payload + drive the report lifecycle in-process,
        // using the shared (managed) session provider so a mid-upload `invalidate`
        // is visible to the login path. A fresh cancel flag scopes this upload.
        // TODO(manual-stop): this flag is never set today — a one-shot manual
        // upload has no Stop UI (unlike live mode, which wires its cancel into
        // `LiveSlot`/`stop_slot_in_map`). The per-segment cancellation in
        // `upload_finished` is therefore inert here. If a manual-upload Stop button
        // is added, lift this flag into managed state keyed by `record_id`.
        let provider = std::sync::Arc::clone(&session);
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        tokio::task::spawn_blocking(move || {
            transport::run_native_upload(&dispatch_path, &opts, provider.as_ref(), cancel)
        })
        .await
        .map_err(|e| format!("Task failed: {e}"))?
    } else {
        tokio::task::spawn_blocking(move || {
            transport::select_transport(prefer_cli).upload_file(&dispatch_path, &opts)
        })
        .await
        .map_err(|e| format!("Task failed: {e}"))?
    };

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
    validate_upload_options(&options)?;

    // Register the cancellable `Starting` slot AS SOON AS the session id is
    // accepted — before the blocking/fallible setup below (`confine_log_path`
    // canonicalizes, `find_official_uploader` stats dozens of paths). Without a
    // slot here, a stop/unmount arriving during that setup window would reach
    // `uploader_stop_live`, find nothing, and no-op — then this start would
    // proceed and launch the uploader after the UI already stopped. With the
    // slot up front, such a stop sets `cancelled` (and is honored at every check
    // below). Replace (and cancel) any existing slot under this id first.
    let cancelled = Arc::new(AtomicBool::new(false));
    let prev_running = {
        let mut sessions = state
            .live_sessions
            .lock()
            .map_err(|_| "Live session lock poisoned")?;
        // Cancel any existing slot under this id FIRST — `stop_slot_in_map` sets a
        // replaced `Starting` slot's flag before removing it, so that slot's own
        // in-flight start can't observe "slot replaced, flag still false" and
        // launch anyway. THEN insert our new slot. Defer a `Running` join to after
        // unlock.
        let prev_running = stop_slot_in_map(&mut sessions, &session_id);
        sessions.insert(
            session_id.clone(),
            LiveSlot::Starting(Arc::clone(&cancelled)),
        );
        prev_running
    };
    if let Some(handle) = prev_running {
        handle.stop();
    }

    // Fallible setup now runs WITH the cancellable slot in place. On any error we
    // must vacate our own slot (only if still ours) so a failed start doesn't
    // leak a `Starting` slot; `confine_err`/the detection branch do exactly that.
    let safe = match confine_log_path(&allowed, &file_path) {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(e) => {
            remove_own_slot(&state, &session_id, &cancelled);
            return Err(e);
        }
    };

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
        remove_own_slot(&state, &session_id, &cancelled);
        return Err(
            "Live logging needs the official ESO Logs uploader (the Archon App) \
                    installed. Install it, or use \"Upload a Log\" after your \
                    session instead."
                .into(),
        );
    }

    // Settle any prior-run stale records BEFORE we write this process's `Live`
    // record below. `reconcile_stale` can't tell a leftover record from a live
    // one, so it must run while none of ours exist — otherwise a later first
    // `uploader_list_history` would flip THIS active session to `Completed`, and
    // `settle_live` (which only touches `Live` records) would then silently drop
    // the observed fight count on stop. The cancellable `Starting` slot is
    // already registered above, so a stop/unmount arriving while this first-use
    // reconcile blocks on history I/O or `MUTATION_LOCK` still sets `cancelled`
    // and is honored by the `aborted_before_handoff` check just below — the start
    // can't launch the uploader or orphan a watcher the frontend already asked to
    // stop. The `Once` keeps this at most once per process.
    super::history::reconcile_stale_once(&app);

    // If a stop/unmount (or a superseding start under the same id) arrived while
    // we were registering or reconciling, bail BEFORE launching the external
    // uploader. Without this, a cancel during reconcile would still fire the
    // real-time handoff — and once the official uploader is launched we can't
    // recall it (see the `cancelled_during_start` note below, which only handles
    // a stop during the handoff itself). Checking here keeps a known-cancelled
    // start from ever touching the uploader, and avoids a duplicate handoff when
    // a newer start replaced our slot. `still_ours` mirrors the promote step:
    // our slot must still be the exact `Starting(cancelled)` we inserted.
    let aborted_before_handoff = {
        let sessions = state
            .live_sessions
            .lock()
            .map_err(|_| "Live session lock poisoned")?;
        let still_ours = matches!(
            sessions.get(&session_id),
            Some(LiveSlot::Starting(c)) if Arc::ptr_eq(c, &cancelled)
        );
        !still_ours || cancelled.load(Ordering::SeqCst)
    };
    if aborted_before_handoff {
        // Fast path: already cancelled/superseded before we scheduled anything.
        // Remove only our own slot (a newer start may have replaced it; leave
        // that one alone) and return without launching the uploader or watcher.
        remove_own_slot(&state, &session_id, &cancelled);
        return Err("Live logging was cancelled before it started.".into());
    }

    // Hand the whole file to the official uploader once, with real-time on.
    // Live MUST use the CLI transport (which passes --enable-real-time-uploading);
    // the GUI handoff would open the uploader in one-shot mode while we show a
    // fake "LIVE" timeline. We already confirmed the uploader is installed above,
    // but detection can still fail (e.g. removed between the check and here), in
    // which case we error rather than silently falling back to a one-shot launch.
    //
    // The `aborted_before_handoff` check above runs under the lock but then
    // releases it before this `spawn_blocking` is scheduled — a stop can still
    // arrive in that gap and set `cancelled`. The AUTHORITATIVE pre-launch check
    // is therefore pushed all the way down into the transport: we hand
    // `upload_file_cancellable` a `should_abort` closure that reads `cancelled`,
    // and it runs that check as the LAST statement before `cmd.spawn()`, after
    // all detection, path validation and argv construction. So any stop ordered
    // before that final read aborts with no external process spawned; only a stop
    // ordered after it (the irreducible instruction gap before `spawn`, which an
    // OS process launch can never be made atomic with) is the documented,
    // unrecallable "stop during handoff" case (settled to `Cancelled` below).
    // `uploader_stop_live` and the supersede path publish the flag via
    // `stop_slot_in_map` (stored while the slot is still mapped), so the read here
    // observes it.
    let mut live_opts = options.clone();
    live_opts.real_time = true;
    let dispatch_path = safe.clone();
    let launch_cancelled = Arc::clone(&cancelled);
    let outcome = tokio::task::spawn_blocking(move || {
        // Fast-path: skip the (fallible, ~54-stat) detection entirely if already
        // cancelled.
        if launch_cancelled.load(Ordering::SeqCst) {
            return Err(LIVE_CANCELLED_BEFORE_LAUNCH.to_string());
        }
        match transport::CliTransport::detect() {
            Some(cli) => {
                let should_abort = || launch_cancelled.load(Ordering::SeqCst);
                match cli.upload_file_cancellable(&dispatch_path, &live_opts, &should_abort) {
                    Ok(result) => result,
                    Err(transport::LaunchAborted) => Err(LIVE_CANCELLED_BEFORE_LAUNCH.to_string()),
                }
            }
            None => Err(
                "The official ESO Logs uploader could not be launched for live logging."
                    .to_string(),
            ),
        }
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
    // Pre-launch cancellation observed atomically inside the closure: nothing was
    // launched, so clean up our slot and report the cancellation (not a failure).
    if matches!(&outcome, Err(e) if e == LIVE_CANCELLED_BEFORE_LAUNCH) {
        remove_own_slot(&state, &session_id, &cancelled);
        return Err("Live logging was cancelled before it started.".into());
    }
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
    // The id ends with `-{session_id}` so `uploader_stop_live` can settle exactly
    // this record (see history::settle_live / next_record_id). If a stop arrived
    // during the blocking handoff above, the uploader is already launched (we
    // can't recall it), but the persisted record should reflect that the user
    // cancelled rather than showing a stuck `Live` badge forever.
    let cancelled_during_start = cancelled.load(Ordering::SeqCst);
    let record_id = super::history::next_record_id(now_ms(), &session_id);
    let record = UploadRecord {
        id: record_id.clone(),
        source_path: safe.clone(),
        file_name,
        created_at_ms: now_ms(),
        status: if cancelled_during_start {
            UploadStatus::Cancelled
        } else {
            UploadStatus::Live
        },
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
            // The local fight-timeline watcher failed to start (the log can be
            // rotated/deleted, or the folder watch can fail, between the handoff
            // above and here). We have no watcher handle to track, so vacate our
            // own slot regardless.
            remove_own_slot(&state, &session_id, &cancelled);
            if handed_off {
                // The official uploader was ALREADY launched and is streaming —
                // only Kalpa's in-app timeline is unavailable. Reporting this as a
                // start *failure* would be wrong (the upload is live and can't be
                // recalled) and, worse, the UI resets its gate on error and a
                // retry would spawn a SECOND real-time uploader against the same
                // log. So return degraded SUCCESS and leave the record `Live`
                // (it genuinely is) — there is no watcher to go stale, and a real
                // stop or next-launch reconcile will settle it.
                eprintln!("[uploader] live timeline watcher failed (upload still running): {e}");
                return Ok(UploadDispatch {
                    handed_off,
                    detail: "Live logging started in the ESO Logs Uploader. The in-app \
                             fight timeline is unavailable for this session."
                        .into(),
                    report,
                });
            }
            // Nothing was launched (no handoff): settle our just-written `Live`
            // record so it can't get stuck with no watcher (reconcile_stale_once
            // already ran this process), and surface the failure.
            let _ = super::history::settle_started(&app, &record_id);
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
            sessions.insert(session_id.clone(), LiveSlot::Running(handle));
            None
        } else {
            // Leave any newer slot alone; just stop our now-unwanted watcher.
            Some(handle)
        }
    };
    if let Some(handle) = promote {
        handle.stop();
        // We lost ownership (a stop/supersede arrived mid-start). The `Live`
        // record we wrote above won't be settled by `uploader_stop_live`'s
        // `settle_live` if that stop ran BEFORE our `upsert` (no record existed
        // yet to match). Settle it ourselves now by id so the panel can't show a
        // perpetual `Live` badge for a session whose watcher we just stopped —
        // and because `reconcile_stale_once` has already run this process, nothing
        // else would settle it until the next launch.
        let _ = super::history::settle_started(&app, &record_id);
    }

    Ok(UploadDispatch {
        handed_off,
        detail,
        report,
    })
}

/// Stop whatever slot is under `key`, **while the session lock is held**, and
/// return any `Running` watcher handle for the caller to join *after* releasing
/// the lock (a `handle.stop()` joins a thread and must not run under the lock).
///
/// The load-bearing detail is ORDER: for a `Starting` slot the cancel flag is
/// stored *while the slot is still in the map*, BEFORE it is removed. The
/// in-flight start's pre-launch check (`cancelled.load` in the handoff closure)
/// is lock-free, so holding the map lock does not by itself serialize against it
/// — only the store-before-remove order does. With this order, that load either
/// happens after the store (sees `true` → aborts) or before it (a genuinely
/// concurrent stop, the documented unrecallable case); it can never see "slot
/// gone, flag still false" and launch after an observable stop. Storing after
/// the removal (the previous bug) left exactly that window.
#[must_use = "the returned Running handle must be stopped after the lock is released"]
fn stop_slot_in_map(
    sessions: &mut HashMap<String, LiveSlot>,
    key: &str,
) -> Option<LiveWatchHandle> {
    // Set a Starting slot's flag in place FIRST (while still mapped), then remove.
    if let Some(LiveSlot::Starting(cancelled)) = sessions.get(key) {
        cancelled.store(true, Ordering::SeqCst);
    }
    match sessions.remove(key) {
        Some(LiveSlot::Running(handle)) => Some(handle),
        _ => None,
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
/// session running regardless). `fight_count` is the number of fights the UI
/// observed this session, recorded onto the settled history record so it doesn't
/// show a stale `Live / 0 fights` badge. `fight_count` defaults to 0 when the
/// caller doesn't supply it (e.g. an unmount-driven best-effort stop).
#[tauri::command]
pub fn uploader_stop_live(
    app: tauri::AppHandle,
    state: State<'_, UploaderState>,
    session_id: String,
    fight_count: Option<usize>,
) -> Result<(), String> {
    // Publish a `Starting` slot's cancel flag BEFORE removing it (handled inside
    // `stop_slot_in_map`), under the lock, so an in-flight start's lock-free
    // pre-launch `cancelled.load` can't see "slot gone, flag still false" and
    // launch the uploader after this stop. Only a `Running` watcher's join is
    // deferred until after the lock is released.
    let running = {
        let mut sessions = state
            .live_sessions
            .lock()
            .map_err(|_| "Live session lock poisoned")?;
        stop_slot_in_map(&mut sessions, &session_id)
    };
    if let Some(handle) = running {
        handle.stop();
    }
    // Settle the matching history record (Live → Completed, with the observed
    // fight count) so the panel reflects reality immediately rather than waiting
    // for the next-launch reconcile. Best-effort: a missing record is fine.
    let _ = super::history::settle_live(&app, &session_id, fight_count.unwrap_or(0));
    Ok(())
}

// ── History ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn uploader_list_history(app: tauri::AppHandle) -> Vec<UploadRecord> {
    // Settle any records left in a transient state by a previous run before the
    // panel renders. Deferred from startup to first uploader use and run at most
    // once per process (see history::reconcile_stale_once).
    super::history::reconcile_stale_once(&app);
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
