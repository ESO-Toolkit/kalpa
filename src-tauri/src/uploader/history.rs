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

/// Runs [`reconcile_stale`] at most once per process, lazily.
static RECONCILE_ONCE: std::sync::Once = std::sync::Once::new();

/// Reconcile stale records exactly once per process, on first uploader use.
///
/// Deferred from the eager `setup()` hook so a user who never opens the uploader
/// pays no history read/parse at startup. It must run before this process writes
/// any transient (`Uploading`/`Live`) record, because [`reconcile_stale`] can't
/// distinguish a leftover record from a current one — so it is invoked at the
/// top of `uploader_start_live` and `uploader_upload_log` (which create those
/// records) as well as from `uploader_list_history` (which displays them). The
/// `Once` makes all of those collapse to a single reconcile per process.
/// Idempotent and cheap on every call after the first.
pub fn reconcile_stale_once(app: &tauri::AppHandle) {
    RECONCILE_ONCE.call_once(|| reconcile_stale(app));
}

/// Reconcile records left in a transient state by a previous run.
///
/// Uploads hand off to the official uploader, whose progress we don't observe,
/// so a record stuck in `Uploading`/`Live` from before a crash/quit can never
/// resolve on its own. Settle them to `Completed` so the history panel doesn't
/// show a perpetual "Uploading"/"Live" badge. Invoked once per process via
/// [`reconcile_stale_once`].
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
        match r.status {
            // A leftover live session: the official uploader may well still be
            // streaming, so settle to the honest `HandedOff` (not `Completed`,
            // which would claim the upload finished).
            UploadStatus::Live => {
                r.status = UploadStatus::HandedOff;
                changed = true;
            }
            // A leftover manual upload handed off to the official UI — we never
            // observe its outcome, so `Completed` is the established semantic.
            UploadStatus::Uploading => {
                r.status = UploadStatus::Completed;
                changed = true;
            }
            _ => {}
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
/// `-{session_id}` (see [`next_record_id`]). On stop we can't observe — or stop —
/// the official uploader (it runs as a separate app and may still be streaming),
/// so flip the record to the honest `HandedOff` (not `Completed`, which would
/// claim the upload finished) and record the fight count the UI saw, rather than
/// leaving a perpetual red `Live` badge until the next `reconcile_stale`.
/// Idempotent and a no-op if the record is gone.
pub fn settle_live(
    app: &tauri::AppHandle,
    session_id: &str,
    fight_count: usize,
) -> Result<(), String> {
    let path = history_path(app)?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "History lock poisoned")?;
    let mut file: HistoryFile = load_json_with_backup(&path);
    if !apply_settle_live(&mut file.records, session_id, fight_count) {
        return Ok(());
    }
    save_json_with_backup(&path, &file)
}

/// The pure settling rule [`settle_live`] applies, factored out so it can be
/// unit-tested without a Tauri `AppHandle`.
///
/// Flips every record whose id ends with `-{session_id}` and is still `Live` to
/// `HandedOff`, stamping `fight_count`. Returns whether anything changed so the
/// caller can skip the disk write on a no-op — which makes a re-settle (e.g. the
/// watcher dying and the user also stopping) idempotent: once a record is
/// `HandedOff` this matches nothing, so a later call neither rewrites the status
/// nor overwrites the first-settle `fight_count`.
fn apply_settle_live(records: &mut [UploadRecord], session_id: &str, fight_count: usize) -> bool {
    let suffix = format!("-{session_id}");
    let mut changed = false;
    for r in records.iter_mut() {
        if r.id.ends_with(&suffix) && matches!(r.status, UploadStatus::Live) {
            r.status = UploadStatus::HandedOff;
            r.fight_count = fight_count;
            changed = true;
        }
    }
    changed
}

/// Settle a specific start record (matched by exact `id`) that lost ownership
/// mid-start to a stop/supersede.
///
/// `uploader_start_live` writes its `Live` record AFTER snapshotting the cancel
/// flag, so a stop landing in that gap runs `settle_live` before the record
/// exists (nothing to match) and leaves a stale `Live` badge. When the start
/// then detects it lost ownership at promotion, it calls this to settle its own
/// just-written record to `Cancelled` — by exact id, so it can't touch a newer
/// session's record. Only flips a still-transient (`Live`/`Uploading`) record;
/// idempotent and a no-op if the record was already settled or removed.
pub fn settle_started(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let path = history_path(app)?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "History lock poisoned")?;
    let mut file: HistoryFile = load_json_with_backup(&path);
    let mut changed = false;
    for r in &mut file.records {
        if r.id == id && matches!(r.status, UploadStatus::Live | UploadStatus::Uploading) {
            r.status = UploadStatus::Cancelled;
            changed = true;
        }
    }
    if !changed {
        return Ok(());
    }
    save_json_with_backup(&path, &file)
}

/// Settle a NATIVE live record by EXACT id when its in-process driver exits. Native
/// live owns the upload end-to-end (unlike the official handoff, which may still be
/// streaming after we stop tracking). The terminal STATUS is decided by `succeeded`,
/// NOT by whether a report link exists: a Fatal/reauth-timeout end can leave a PARTIAL
/// report on the server, so we still record its link (for the user to inspect) but mark
/// the record `Failed` with `error` — never `Completed`, which would hide the data loss
/// from history and recovery. A genuinely clean/stopped end is `Completed`. Matched by
/// exact id (NOT the suffix-matching [`settle_live`]) so the driver thread is the single
/// owner of its record's terminal state and can't race / double-settle the official
/// path. Idempotent: only flips a still-transient (`Live`/`Paused`) record.
pub fn settle_native_live(
    app: &tauri::AppHandle,
    id: &str,
    segments_built: usize,
    report: Option<(String, String)>, // (url, code) — kept regardless of success
    succeeded: bool,
    error: Option<String>,
) -> Result<(), String> {
    use super::types::ReportRef;
    let path = history_path(app)?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "History lock poisoned")?;
    let mut file: HistoryFile = load_json_with_backup(&path);
    let mut changed = false;
    for r in &mut file.records {
        if r.id == id && matches!(r.status, UploadStatus::Live | UploadStatus::Paused) {
            r.status = if succeeded {
                UploadStatus::Completed
            } else {
                UploadStatus::Failed
            };
            r.fight_count = segments_built;
            if let Some((url, code)) = &report {
                r.report = Some(ReportRef {
                    url: url.clone(),
                    code: code.clone(),
                });
            }
            if let Some(e) = &error {
                r.error = Some(e.clone());
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uploader::types::{UploadMode, Visibility};

    fn rec(id: &str, status: UploadStatus) -> UploadRecord {
        UploadRecord {
            id: id.into(),
            source_path: "x".into(),
            file_name: "Encounter.log".into(),
            created_at_ms: 0,
            status,
            mode: UploadMode::Live,
            visibility: Visibility::Private,
            fight_count: 0,
            report: None,
            error: None,
        }
    }

    // The settling rule the stop / watcher-died path runs (uploader_stop_live →
    // settle_live → apply_settle_live): the session's still-`Live` record is
    // flipped to `HandedOff` (the official uploader may still be streaming) with
    // the observed fight count, and only that session's records are touched.
    #[test]
    fn settles_live_record_for_session_and_stamps_count() {
        let sid = "live-1700000000000";
        // Record ids follow next_record_id: "{now_ms}-{counter}-{session_id}".
        let mut recs = vec![
            rec(&format!("1700000000005-0-{sid}"), UploadStatus::Live),
            rec("1700000000005-1-live-OTHER", UploadStatus::Live), // different session
            rec(&format!("1700000000006-2-{sid}"), UploadStatus::Completed), // already done
        ];
        assert!(apply_settle_live(&mut recs, sid, 7));
        assert_eq!(recs[0].status, UploadStatus::HandedOff);
        assert_eq!(recs[0].fight_count, 7);
        assert_eq!(recs[1].status, UploadStatus::Live); // untouched: other session
        assert_eq!(recs[2].fight_count, 0); // untouched: already Completed
    }

    // The double-settle guard: a watcher death AND a user/unmount stop can both
    // drive settle_live for the same session. The second call must be a no-op —
    // not flip status again, and crucially not overwrite the first count.
    #[test]
    fn re_settle_is_idempotent_noop() {
        let sid = "live-42";
        let mut recs = vec![rec(&format!("1-0-{sid}"), UploadStatus::Live)];
        assert!(apply_settle_live(&mut recs, sid, 3));
        assert_eq!(recs[0].status, UploadStatus::HandedOff);
        assert!(!apply_settle_live(&mut recs, sid, 99));
        assert_eq!(recs[0].fight_count, 3); // first settle wins; not overwritten
        assert_eq!(recs[0].status, UploadStatus::HandedOff); // and status unchanged
    }
}
