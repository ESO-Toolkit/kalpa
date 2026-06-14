//! Persistence for the upload history panel.
//!
//! Records are stored as a single JSON array in the app data directory, using
//! the same atomic-write-with-backup helper the metadata store uses
//! (`metadata::save_json_with_backup`) so a crash mid-write can't corrupt it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::Manager;

use super::types::UploadRecord;
use crate::metadata::{load_json_with_backup, save_json_with_backup};

/// Cap the stored history so the file can't grow unbounded.
const MAX_RECORDS: usize = 200;

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

/// Insert or update a record (matched by `id`), then persist.
pub fn upsert(app: &tauri::AppHandle, record: UploadRecord) -> Result<(), String> {
    let path = history_path(app)?;
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

/// Delete a record by id, then persist.
pub fn remove(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let path = history_path(app)?;
    let mut file: HistoryFile = load_json_with_backup(&path);
    let before = file.records.len();
    file.records.retain(|r| r.id != id);
    if file.records.len() == before {
        return Ok(()); // nothing to do
    }
    save_json_with_backup(&path, &file)
}
