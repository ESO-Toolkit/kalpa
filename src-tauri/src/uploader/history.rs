//! Persistence for the upload history panel.
//!
//! Records are stored as a single JSON array in the app data directory, using
//! the same atomic-write-with-backup helper the metadata store uses
//! (`metadata::save_json_with_backup`) so a crash mid-write can't corrupt it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::Manager;

use super::types::{UploadRecord, UploadStatus};
use crate::metadata::{load_json_with_backup, save_json_with_backup};

/// Cap the stored history so the file can't grow unbounded.
const MAX_RECORDS: usize = 200;

/// Serializes every load→mutate→save cycle so concurrent commands (e.g. an
/// async upload mid-await vs. a sync attach-report) can't lose a record to a
/// last-writer-wins race.
static MUTATION_LOCK: Mutex<()> = Mutex::new(());

/// Monotonic suffix making record ids unique even within the same millisecond.
static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Build a unique history record id: `{now_ms}-{counter}-{label}`. The counter
/// disambiguates two records created in the same millisecond (which would
/// otherwise collide and overwrite under upsert's id match).
pub fn next_record_id(now_ms: u64, label: &str) -> String {
    let n = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{now_ms}-{n}-{label}")
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct HistoryFile {
    records: Vec<UploadRecord>,
}

fn history_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Could not resolve app data dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create app data dir: {e}"))?;
    Ok(dir.join("upload-history.json"))
}

/// Load all records, newest first.
pub fn load(app: &tauri::AppHandle) -> Vec<UploadRecord> {
    let Ok(path) = history_path(app) else {
        return Vec::new();
    };
    let mut file: HistoryFile = load_json_with_backup(&path);
    file.records
        .sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    file.records
}

/// Reconcile records left in a transient state by a previous run.
///
/// Uploads hand off to the official uploader, whose progress we don't observe,
/// so a record stuck in `Uploading`/`Live` from before a crash/quit can never
/// resolve on its own. Settle them to `Completed` once at startup so the history
/// panel doesn't show a perpetual "Uploading"/"Live" badge.
pub fn reconcile_stale(app: &tauri::AppHandle) {
    let Ok(path) = history_path(app) else {
        return;
    };
    let Ok(_guard) = MUTATION_LOCK.lock() else {
        return;
    };
    let mut file: HistoryFile = load_json_with_backup(&path);
    let mut changed = false;
    for r in &mut file.records {
        if matches!(r.status, UploadStatus::Uploading | UploadStatus::Live) {
            r.status = UploadStatus::Completed;
            changed = true;
        }
    }
    if changed {
        let _ = save_json_with_backup(&path, &file);
    }
}

/// Insert or update a record (matched by `id`), then persist. The whole
/// read-modify-write is serialized so concurrent callers can't lose records.
pub fn upsert(app: &tauri::AppHandle, record: UploadRecord) -> Result<(), String> {
    let path = history_path(app)?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "History lock poisoned")?;
    let mut file: HistoryFile = load_json_with_backup(&path);

    if let Some(existing) = file.records.iter_mut().find(|r| r.id == record.id) {
        *existing = record;
    } else {
        file.records.push(record);
    }

    // Keep only the most recent MAX_RECORDS.
    file.records
        .sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    file.records.truncate(MAX_RECORDS);

    save_json_with_backup(&path, &file)
}

/// Settle the live record for a session when the user stops live mode.
///
/// `uploader_start_live` writes a `Live` record whose id ends with
/// `-{session_id}` (see [`next_record_id`]). On stop we can't observe the
/// official uploader's outcome, but we *do* know the session ended and how many
/// fights the UI saw, so flip the record to `Completed` and record that count
/// rather than leaving a perpetual red `Live / 0 fights` badge until the next
/// `reconcile_stale` at startup. Idempotent and a no-op if the record is gone.
pub fn settle_live(
    app: &tauri::AppHandle,
    session_id: &str,
    fight_count: usize,
) -> Result<(), String> {
    let path = history_path(app)?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "History lock poisoned")?;
    let mut file: HistoryFile = load_json_with_backup(&path);
    let suffix = format!("-{session_id}");
    let mut changed = false;
    for r in &mut file.records {
        if r.id.ends_with(&suffix) && matches!(r.status, UploadStatus::Live) {
            r.status = UploadStatus::Completed;
            r.fight_count = fight_count;
            changed = true;
        }
    }
    if !changed {
        return Ok(());
    }
    save_json_with_backup(&path, &file)
}

/// Delete a record by id, then persist (serialized with other mutations).
pub fn remove(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let path = history_path(app)?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "History lock poisoned")?;
    let mut file: HistoryFile = load_json_with_backup(&path);
    let before = file.records.len();
    file.records.retain(|r| r.id != id);
    if file.records.len() == before {
        return Ok(()); // nothing to do
    }
    save_json_with_backup(&path, &file)
}
