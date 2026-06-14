//! Locating the ESO `Logs` directory and enumerating log files.
//!
//! Kalpa already knows the configured AddOns folder, which sits next to the
//! `Logs` folder (`…/Elder Scrolls Online/<env>/AddOns` ⇄ `…/<env>/Logs`), so
//! we can auto-detect with high confidence and fall back to a Documents scan.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::types::{LogFileInfo, LogPathDetection};

/// A file modified within this window is considered "active" (being written).
const ACTIVE_WINDOW_SECS: u64 = 90;

fn modified_ms(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Build a [`LogPathDetection`] for a candidate logs directory.
fn detection_for(logs_dir: &Path, from_addon_path: bool) -> LogPathDetection {
    let exists = logs_dir.is_dir();
    let encounter = logs_dir.join("Encounter.log");
    let encounter_log_exists = encounter.is_file();

    let message = if !exists {
        "Expected log directory found, but it doesn't exist yet. Enable combat \
         logging in-game with /encounterlog to create it."
            .into()
    } else if encounter_log_exists {
        "Log directory and an Encounter.log were found.".into()
    } else {
        "Log directory found, but no Encounter.log yet. Type /encounterlog in \
         chat (or use a logging addon) to start recording."
            .into()
    };

    LogPathDetection {
        path: Some(logs_dir.to_string_lossy().into_owned()),
        from_addon_path,
        encounter_log_exists,
        message,
    }
}

/// Attempt to discover the ESO logs directory.
pub fn detect_log_path(addons_path: Option<&str>) -> LogPathDetection {
    // Strategy 1: derive from the configured AddOns path (Logs is its sibling).
    if let Some(ap) = addons_path {
        if let Some(parent) = PathBuf::from(ap).parent() {
            return detection_for(&parent.join("Logs"), true);
        }
    }

    // Strategy 2: scan the common ESO environments under Documents.
    if let Some(docs) = dirs::document_dir() {
        for env in ["live", "liveeu", "pts"] {
            let logs_dir = docs.join("Elder Scrolls Online").join(env).join("Logs");
            if logs_dir.is_dir() {
                return detection_for(&logs_dir, false);
            }
        }
    }

    LogPathDetection {
        path: None,
        from_addon_path: false,
        encounter_log_exists: false,
        message: "Could not find an ESO log directory. Select it manually, or \
                  enable combat logging in-game first."
            .into(),
    }
}

/// List all `*.log` files in a directory, newest first.
pub fn list_log_files(logs_dir: &str) -> Result<Vec<LogFileInfo>, String> {
    let dir = Path::new(logs_dir);
    if !dir.is_dir() {
        return Err(format!("Not a directory: {logs_dir}"));
    }

    let now = now_ms();
    let mut files: Vec<LogFileInfo> = Vec::new();

    for entry in fs::read_dir(dir)
        .map_err(|e| format!("Failed to read directory: {e}"))?
        .flatten()
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_log = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("log"))
            .unwrap_or(false);
        if !is_log {
            continue;
        }

        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        let modified_at_ms = modified_ms(&meta);
        // Guard against a future-dated mtime (clock skew / VM resume): otherwise
        // `now.saturating_sub` underflows to 0 and a long-idle log reads "active".
        let is_active = modified_at_ms > 0
            && modified_at_ms <= now
            && now.saturating_sub(modified_at_ms) < ACTIVE_WINDOW_SECS * 1000;

        files.push(LogFileInfo {
            path: path.to_string_lossy().into_owned(),
            file_name: path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string(),
            size_bytes: meta.len(),
            modified_at_ms,
            is_active,
        });
    }

    files.sort_by(|a, b| b.modified_at_ms.cmp(&a.modified_at_ms));
    Ok(files)
}
