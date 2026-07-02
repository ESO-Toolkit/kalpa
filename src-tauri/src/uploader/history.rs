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

use super::types::{ReportRef, UploadRecord, UploadStatus};
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
/// while native live owns a report in-process. A record stuck in
/// `Uploading`/`Live`/`Paused` from before a crash/quit can never resolve on its own,
/// so settle it to the most honest terminal state. Invoked once per process via
/// [`reconcile_stale_once`].
pub fn reconcile_stale(app: &tauri::AppHandle) {
    let Ok(path) = history_path(app) else {
        return;
    };
    let Ok(_guard) = MUTATION_LOCK.lock() else {
        return;
    };
    let mut file: HistoryFile = load_json_with_backup(&path);
    if apply_reconcile_stale(&mut file.records) {
        let _ = save_json_with_backup(&path, &file);
    }
}

fn apply_reconcile_stale(records: &mut [UploadRecord]) -> bool {
    let mut changed = false;
    for r in records {
        match r.status {
            // A leftover native-live session either knows its ESO Logs report code
            // because the direct uploader persisted it immediately after create-report,
            // or it was paused waiting for reauth. If this process died before final
            // settle, the report may be partial; keep the link and mark it
            // failed/incomplete rather than disguising it as an official handoff.
            UploadStatus::Paused => {
                r.status = UploadStatus::Failed;
                r.error.get_or_insert_with(|| {
                    "Kalpa closed before this native live report finished; the report may be \
                     incomplete."
                        .to_string()
                });
                changed = true;
            }
            UploadStatus::Live if r.report.is_some() => {
                r.status = UploadStatus::Failed;
                r.error.get_or_insert_with(|| {
                    "Kalpa closed before this native live report finished; the report may be \
                     incomplete."
                        .to_string()
                });
                changed = true;
            }
            // A leftover live session without a report code is the official-uploader
            // handoff path: the official uploader may well still be streaming, so settle
            // to the honest `HandedOff` (not `Completed`, which would claim the upload
            // finished).
            UploadStatus::Live => {
                r.status = UploadStatus::HandedOff;
                changed = true;
            }
            // A leftover manual upload may have handed off to the official UI.
            // We never observe its outcome, so keep the paste-link affordance
            // available instead of claiming it completed without a report.
            UploadStatus::Uploading => {
                r.status = UploadStatus::HandedOff;
                changed = true;
            }
            _ => {}
        }
    }
    changed
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
    if !apply_settle_started(&mut file.records, id) {
        return Ok(());
    }
    save_json_with_backup(&path, &file)
}

fn apply_settle_started(records: &mut [UploadRecord], id: &str) -> bool {
    let mut changed = false;
    for r in records {
        if r.id == id && matches!(r.status, UploadStatus::Live | UploadStatus::Uploading) {
            r.status = UploadStatus::Cancelled;
            changed = true;
        }
    }
    changed
}

/// Persist the native live report code as soon as create-report returns. This closes
/// the crash window between report creation and normal driver settlement: on next
/// launch, history reconciliation can preserve the link and mark the native report
/// incomplete instead of losing the code and showing a generic handoff.
pub fn attach_live_report(
    app: &tauri::AppHandle,
    id: &str,
    report: ReportRef,
) -> Result<(), String> {
    let path = history_path(app)?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "History lock poisoned")?;
    let mut file: HistoryFile = load_json_with_backup(&path);
    if !apply_attach_live_report(&mut file.records, id, report) {
        return Ok(());
    }
    save_json_with_backup(&path, &file)
}

fn apply_attach_live_report(records: &mut [UploadRecord], id: &str, report: ReportRef) -> bool {
    let mut changed = false;
    for r in records {
        if r.id == id && matches!(r.status, UploadStatus::Live | UploadStatus::Paused) {
            r.report = Some(report.clone());
            changed = true;
        }
    }
    changed
}

/// Attach a user-pasted report link to a terminal record (matched by exact `id`),
/// serialized under [`MUTATION_LOCK`] so the whole load→mutate→save cycle can't lose
/// a concurrent driver settle (the lost-update race: attach reads a stale `Live`
/// snapshot while the native driver upserts `Completed`, then writes the stale
/// status back). Loading under the lock means attach sees the driver's latest state,
/// and it only ever mutates `report` — never `status` — so a settled record keeps its
/// terminal status. Only a terminal `HandedOff | Completed | Failed` record accepts a
/// pasted link; a still-transient (`Live`/`Uploading`/`Paused`) record is owned by its
/// driver and must not be touched here. Returns an error if no such record exists.
pub fn attach_report(app: &tauri::AppHandle, id: &str, report: ReportRef) -> Result<(), String> {
    let path = history_path(app)?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "History lock poisoned")?;
    let mut file: HistoryFile = load_json_with_backup(&path);
    if !apply_attach_report(&mut file.records, id, report) {
        return Err("Upload record not found.".into());
    }
    save_json_with_backup(&path, &file)
}

/// The pure attach rule [`attach_report`] applies, factored out for unit testing.
/// Sets `report` on the record with exact `id` when it is in a terminal handoff /
/// complete / failed state, preserving its `status`. Returns whether anything
/// changed (a missing id, or a still-transient record, matches nothing).
fn apply_attach_report(records: &mut [UploadRecord], id: &str, report: ReportRef) -> bool {
    let mut changed = false;
    for r in records {
        if r.id == id
            && matches!(
                r.status,
                UploadStatus::HandedOff | UploadStatus::Completed | UploadStatus::Failed
            )
        {
            r.report = Some(report.clone());
            changed = true;
        }
    }
    changed
}

/// Mark a native live record as paused while its ESO Logs session is refreshed.
///
/// This is intentionally exact-id and native-driver-owned, like
/// [`settle_native_live`]. If Kalpa exits during the pause, startup reconciliation sees
/// `Paused` and marks the saved report incomplete instead of treating the session as a
/// healthy official-uploader handoff.
pub fn pause_native_live(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let path = history_path(app)?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "History lock poisoned")?;
    let mut file: HistoryFile = load_json_with_backup(&path);
    if !apply_pause_native_live(&mut file.records, id) {
        return Ok(());
    }
    save_json_with_backup(&path, &file)
}

fn apply_pause_native_live(records: &mut [UploadRecord], id: &str) -> bool {
    let mut changed = false;
    for r in records {
        if r.id == id && matches!(r.status, UploadStatus::Live) {
            r.status = UploadStatus::Paused;
            changed = true;
        }
    }
    changed
}

/// Return a reauthenticated native live record to the healthy live state.
pub fn resume_native_live(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let path = history_path(app)?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "History lock poisoned")?;
    let mut file: HistoryFile = load_json_with_backup(&path);
    if !apply_resume_native_live(&mut file.records, id) {
        return Ok(());
    }
    save_json_with_backup(&path, &file)
}

fn apply_resume_native_live(records: &mut [UploadRecord], id: &str) -> bool {
    let mut changed = false;
    for r in records {
        if r.id == id && matches!(r.status, UploadStatus::Paused) {
            r.status = UploadStatus::Live;
            changed = true;
        }
    }
    changed
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
    fight_count: usize,
    report: Option<(String, String)>, // (url, code) — kept regardless of success
    succeeded: bool,
    error: Option<String>,
) -> Result<(), String> {
    let path = history_path(app)?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "History lock poisoned")?;
    let mut file: HistoryFile = load_json_with_backup(&path);
    let changed = apply_settle_native_live(
        &mut file.records,
        id,
        fight_count,
        report.as_ref().map(|(url, code)| ReportRef {
            url: url.clone(),
            code: code.clone(),
        }),
        succeeded,
        error.as_deref(),
    );
    if !changed {
        return Ok(());
    }
    save_json_with_backup(&path, &file)
}

fn apply_settle_native_live(
    records: &mut [UploadRecord],
    id: &str,
    fight_count: usize,
    report: Option<ReportRef>,
    succeeded: bool,
    error: Option<&str>,
) -> bool {
    let mut changed = false;
    for r in records {
        if r.id == id && matches!(r.status, UploadStatus::Live | UploadStatus::Paused) {
            r.status = if succeeded {
                UploadStatus::Completed
            } else {
                UploadStatus::Failed
            };
            r.fight_count = fight_count;
            if let Some(report) = &report {
                r.report = Some(report.clone());
            }
            if let Some(error) = error {
                r.error = Some(error.to_string());
            }
            changed = true;
        }
    }
    changed
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
            title: None,
            zone: None,
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

    #[test]
    fn native_live_report_code_attaches_before_terminal_settle() {
        let mut recs = vec![
            rec("native-live", UploadStatus::Live),
            rec("native-done", UploadStatus::Completed),
        ];

        assert!(apply_attach_live_report(
            &mut recs,
            "native-live",
            ReportRef {
                url: "https://www.esologs.com/reports/EARLY".into(),
                code: "EARLY".into(),
            },
        ));

        assert_eq!(
            recs[0].report.as_ref().map(|r| r.code.as_str()),
            Some("EARLY")
        );
        assert_eq!(recs[0].status, UploadStatus::Live);
        assert!(!apply_attach_live_report(
            &mut recs,
            "native-done",
            ReportRef {
                url: "https://www.esologs.com/reports/LATE".into(),
                code: "LATE".into(),
            },
        ));
        assert!(recs[1].report.is_none());
    }

    // C6: a user-pasted link attaches to a TERMINAL record (HandedOff/Completed/
    // Failed) and preserves its status, while a still-transient record — owned by its
    // driver — is left untouched, and an unknown id matches nothing. Under
    // `MUTATION_LOCK` (in the real `attach_report`) this makes the paste lose no
    // concurrent settle: it only ever writes `report`, never `status`.
    #[test]
    fn attach_report_targets_terminal_records_and_preserves_status() {
        let report = ReportRef {
            url: "https://www.esologs.com/reports/ABC".into(),
            code: "ABC".into(),
        };
        let mut recs = vec![
            rec("handed-off", UploadStatus::HandedOff),
            rec("live", UploadStatus::Live),
            rec("uploading", UploadStatus::Uploading),
        ];

        // Terminal HandedOff record accepts the link and keeps its status.
        assert!(apply_attach_report(&mut recs, "handed-off", report.clone()));
        assert_eq!(
            recs[0].report.as_ref().map(|r| r.code.as_str()),
            Some("ABC")
        );
        assert_eq!(recs[0].status, UploadStatus::HandedOff);

        // A still-Live record is driver-owned — the pasted link is rejected.
        assert!(!apply_attach_report(&mut recs, "live", report.clone()));
        assert!(recs[1].report.is_none());
        // A transient Uploading record is likewise rejected.
        assert!(!apply_attach_report(&mut recs, "uploading", report.clone()));
        assert!(recs[2].report.is_none());
        // An unknown id changes nothing.
        assert!(!apply_attach_report(&mut recs, "nope", report));
    }

    #[test]
    fn settle_started_cancel_preserves_early_native_report_link() {
        let mut recs = vec![rec("native-start", UploadStatus::Live)];
        recs[0].report = Some(ReportRef {
            url: "https://www.esologs.com/reports/STOPPED".into(),
            code: "STOPPED".into(),
        });

        assert!(apply_settle_started(&mut recs, "native-start"));

        assert_eq!(recs[0].status, UploadStatus::Cancelled);
        assert_eq!(
            recs[0].report.as_ref().map(|r| r.code.as_str()),
            Some("STOPPED")
        );
    }

    #[test]
    fn native_live_reauth_pause_and_resume_are_exact_and_idempotent() {
        let mut recs = vec![
            rec("native-live", UploadStatus::Live),
            rec("other-live", UploadStatus::Live),
            rec("native-done", UploadStatus::Completed),
        ];

        assert!(apply_pause_native_live(&mut recs, "native-live"));
        assert_eq!(recs[0].status, UploadStatus::Paused);
        assert_eq!(recs[1].status, UploadStatus::Live);
        assert_eq!(recs[2].status, UploadStatus::Completed);
        assert!(!apply_pause_native_live(&mut recs, "native-live"));

        assert!(apply_resume_native_live(&mut recs, "native-live"));
        assert_eq!(recs[0].status, UploadStatus::Live);
        assert!(!apply_resume_native_live(&mut recs, "native-live"));
        assert!(!apply_pause_native_live(&mut recs, "native-done"));
    }

    #[test]
    fn reconcile_stale_native_live_with_report_preserves_link_and_marks_failed() {
        let mut native = rec("native-live", UploadStatus::Live);
        native.report = Some(ReportRef {
            url: "https://www.esologs.com/reports/CRASHED".into(),
            code: "CRASHED".into(),
        });
        let mut paused = rec("native-paused", UploadStatus::Paused);
        paused.report = Some(ReportRef {
            url: "https://www.esologs.com/reports/PAUSED".into(),
            code: "PAUSED".into(),
        });
        let mut manual = rec("manual-uploading", UploadStatus::Uploading);
        manual.mode = UploadMode::Manual;
        let mut recs = vec![
            native,
            rec("official-live", UploadStatus::Live),
            paused,
            manual,
        ];

        assert!(apply_reconcile_stale(&mut recs));

        assert_eq!(recs[0].status, UploadStatus::Failed);
        assert_eq!(
            recs[0].report.as_ref().map(|r| r.code.as_str()),
            Some("CRASHED")
        );
        assert!(
            recs[0]
                .error
                .as_deref()
                .is_some_and(|message| message.contains("may be incomplete")),
            "{:?}",
            recs[0].error
        );
        assert_eq!(
            recs[1].status,
            UploadStatus::HandedOff,
            "official live records without a report code still reconcile as handoff"
        );
        assert_eq!(recs[2].status, UploadStatus::Failed);
        assert_eq!(
            recs[2].report.as_ref().map(|r| r.code.as_str()),
            Some("PAUSED")
        );
        assert_eq!(
            recs[3].status,
            UploadStatus::HandedOff,
            "stale manual handoffs need the paste-link affordance"
        );
    }

    #[test]
    fn native_live_settle_success_completes_exact_record_with_report_and_count() {
        let mut recs = vec![
            rec("native-1", UploadStatus::Live),
            rec("native-10", UploadStatus::Live),
            rec("native-done", UploadStatus::Completed),
        ];
        assert!(apply_settle_native_live(
            &mut recs,
            "native-1",
            2,
            Some(ReportRef {
                url: "https://www.esologs.com/reports/ABC123".into(),
                code: "ABC123".into(),
            }),
            true,
            None,
        ));

        assert_eq!(recs[0].status, UploadStatus::Completed);
        assert_eq!(recs[0].fight_count, 2);
        assert_eq!(
            recs[0].report.as_ref().map(|r| r.code.as_str()),
            Some("ABC123")
        );
        assert_eq!(
            recs[1].status,
            UploadStatus::Live,
            "native settle must match exact id, not a suffix/prefix"
        );
        assert_eq!(
            recs[2].status,
            UploadStatus::Completed,
            "already-terminal records are not rewritten"
        );
    }

    #[test]
    fn native_live_settle_failure_keeps_partial_report_link_and_error() {
        let mut recs = vec![rec("native-failed", UploadStatus::Paused)];
        assert!(apply_settle_native_live(
            &mut recs,
            "native-failed",
            4,
            Some(ReportRef {
                url: "https://www.esologs.com/reports/PARTIAL".into(),
                code: "PARTIAL".into(),
            }),
            false,
            Some("session expired before final segment"),
        ));

        assert_eq!(recs[0].status, UploadStatus::Failed);
        assert_eq!(recs[0].fight_count, 4);
        assert_eq!(
            recs[0].report.as_ref().map(|r| r.code.as_str()),
            Some("PARTIAL")
        );
        assert_eq!(
            recs[0].error.as_deref(),
            Some("session expired before final segment"),
            "failed native live records need an actionable history error"
        );
    }

    #[test]
    fn native_live_re_settle_is_idempotent_noop() {
        let mut recs = vec![rec("native-once", UploadStatus::Live)];
        assert!(apply_settle_native_live(
            &mut recs,
            "native-once",
            5,
            Some(ReportRef {
                url: "https://www.esologs.com/reports/FIRST".into(),
                code: "FIRST".into(),
            }),
            true,
            None,
        ));
        assert!(!apply_settle_native_live(
            &mut recs,
            "native-once",
            99,
            Some(ReportRef {
                url: "https://www.esologs.com/reports/SECOND".into(),
                code: "SECOND".into(),
            }),
            false,
            Some("late failure"),
        ));

        assert_eq!(recs[0].status, UploadStatus::Completed);
        assert_eq!(recs[0].fight_count, 5);
        assert_eq!(
            recs[0].report.as_ref().map(|r| r.code.as_str()),
            Some("FIRST")
        );
        assert_eq!(recs[0].error, None);
    }
}
