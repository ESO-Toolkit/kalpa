//! Crash-recovery for unterminated native live reports (the L2 lifecycle item).
//!
//! A native live session holds ONE report open across the whole session and only
//! `terminate-report`s it when logging ends. If Kalpa is killed / panics / loses
//! power between `create-report` and `terminate-report`, that report is left OPEN
//! server-side. This module persists a `{reportCode, segmentId}` breadcrumb the
//! instant the report is created and best-effort terminates any leftover code on the
//! next launch / panel open, so a crash never strands a draft report.
//!
//! It is INDEPENDENT of the upload-history `UploadRecord` (which `reconcile_stale`
//! already settles) — orphan recovery keys on `reportCode` and only does server-side
//! hygiene, so the two never read each other and can't double-settle (see the L2
//! design). Persistence reuses the same atomic-write-with-backup helper as
//! [`super::super::history`] (`metadata::save_json_with_backup`), so a crash mid-write
//! recovers from `.tmp`/`.bak` identically.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, Once};

use serde::{Deserialize, Serialize};
use tauri::Manager;

use super::client::{desktop_client_base, is_definitively_closed, OwnedLiveRequest};
use super::session::{SessionProvider, StoredSessionProvider};
use crate::metadata::{load_json_with_backup, save_json_with_backup};

/// Serialize every load→mutate→save (mirrors `history::MUTATION_LOCK`).
static MUTATION_LOCK: Mutex<()> = Mutex::new(());

/// Run the recovery sweep at most once per process.
static RECOVER_ONCE: Once = Once::new();

/// Cap so a pathological run can't grow the file unbounded (mirrors history's cap).
const MAX_ORPHANS: usize = 32;

/// One unterminated live report we opened and have not confirmed closed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LiveOrphan {
    /// The report code from `create-report` — addresses `terminate-report`.
    pub code: String,
    /// The most recent segment id the server sequenced (diagnostics only;
    /// terminate needs just the code). Starts at 1, updated per accepted segment.
    pub last_segment_id: u64,
    /// The growing log this session was streaming (diagnostics / UI).
    pub source_path: String,
    /// Epoch-ms the report was opened (for ordering + stale display).
    pub created_at_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct OrphanFile {
    orphans: Vec<LiveOrphan>,
}

fn orphans_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Could not resolve app data dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create app data dir: {e}"))?;
    Ok(dir.join("native-live-orphans.json"))
}

/// Record a freshly-opened report so a crash before terminate leaves a recoverable
/// breadcrumb. Keyed by `code`: re-recording the same code updates in place (never
/// duplicates). Best-effort — a persist failure must NOT abort the upload (the caller
/// logs and continues; the cost is one un-recoverable orphan on a crash, exactly the
/// pre-L2 status quo).
pub fn record_open(app: &tauri::AppHandle, orphan: LiveOrphan) -> Result<(), String> {
    let path = orphans_path(app)?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "Orphan lock poisoned")?;
    let mut file: OrphanFile = load_json_with_backup(&path);
    if let Some(existing) = file.orphans.iter_mut().find(|o| o.code == orphan.code) {
        *existing = orphan;
    } else {
        file.orphans.push(orphan);
    }
    file.orphans
        .sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    file.orphans.truncate(MAX_ORPHANS);
    save_json_with_backup(&path, &file)
}

/// Update the last sequenced segment id for an already-recorded code. A no-op if the
/// code isn't recorded — `record_open` is the single authoritative "report exists"
/// write, so we never re-create an entry here. Best-effort.
pub fn note_segment(
    app: &tauri::AppHandle,
    code: &str,
    last_segment_id: u64,
) -> Result<(), String> {
    let path = orphans_path(app)?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "Orphan lock poisoned")?;
    let mut file: OrphanFile = load_json_with_backup(&path);
    let Some(o) = file.orphans.iter_mut().find(|o| o.code == code) else {
        return Ok(());
    };
    o.last_segment_id = last_segment_id;
    save_json_with_backup(&path, &file)
}

/// Remove an orphan by code after a confirmed-closed report (clean terminate, or a
/// definitive 404/410 during recovery). Idempotent — a no-op when absent.
pub fn clear(app: &tauri::AppHandle, code: &str) -> Result<(), String> {
    let path = orphans_path(app)?;
    let _guard = MUTATION_LOCK.lock().map_err(|_| "Orphan lock poisoned")?;
    let mut file: OrphanFile = load_json_with_backup(&path);
    let before = file.orphans.len();
    file.orphans.retain(|o| o.code != code);
    if file.orphans.len() == before {
        return Ok(()); // nothing to do
    }
    save_json_with_backup(&path, &file)
}

/// Snapshot the current orphan list (for the recovery sweep). No lock is held across
/// the network calls that follow.
pub fn load(app: &tauri::AppHandle) -> Vec<LiveOrphan> {
    let Ok(path) = orphans_path(app) else {
        return Vec::new();
    };
    let file: OrphanFile = load_json_with_backup(&path);
    file.orphans
}

/// Run the recovery sweep at most once per process, OFF the calling thread, using the
/// managed [`StoredSessionProvider`] `Arc` directly (so the cancel-aware sender can own
/// it). Does NOT spend the `Once` while signed-out, so a later sign-in still triggers
/// the real sweep. Mirrors `history::reconcile_stale_once`'s once-per-process contract
/// but spawns because recovery does network I/O and must not block startup.
pub fn recover_orphans_once(app: tauri::AppHandle, session: Arc<StoredSessionProvider>) {
    // Don't burn the one-shot before a usable session exists — a panel-open while
    // signed-out would otherwise permanently skip recovery this process. Gate on
    // `session()` (a real usable session), NOT just `has_session()` (a cookie is
    // present): the two differ by a logout/invalidate race, and gating on the weaker
    // check could spend the `Once` on a sweep that then bails signed-out and never
    // retries (the inner re-check at `recover_orphans_with_sender` would fire after the
    // `Once` is already consumed).
    use super::session::SessionProvider;
    if session.session().is_err() {
        return;
    }
    RECOVER_ONCE.call_once(move || {
        std::thread::spawn(move || {
            recover_orphans_with_sender(&app, session);
        });
    });
}

/// The production recovery body: best-effort terminate each recorded orphan, clearing
/// only on a confirmed close (success or a definitive 404/410 via
/// [`is_definitively_closed`]); transients KEEP the breadcrumb. Owns the managed `Arc`
/// so it can build a real cancel-aware [`super::client::LiveSender`].
fn recover_orphans_with_sender(app: &tauri::AppHandle, session: Arc<StoredSessionProvider>) {
    let orphans = load(app);
    if orphans.is_empty() {
        return;
    }
    if session.session().is_err() {
        eprintln!(
            "[uploader] {} orphan live report(s) pending; not signed in, will retry later",
            orphans.len()
        );
        return;
    }
    let sender = super::client::LiveSender::new(session as Arc<dyn SessionProvider>);
    let no_cancel = Arc::new(AtomicBool::new(false));
    for o in orphans {
        let url = format!("{}/terminate-report/{}", desktop_client_base(), o.code);
        match sender.send_cancellable(&url, OwnedLiveRequest::Terminate, &no_cancel) {
            Ok(_) => {
                let _ = clear(app, &o.code);
            }
            Err(e) if is_definitively_closed(&e) => {
                let _ = clear(app, &o.code);
            }
            Err(e) => {
                eprintln!("[uploader] orphan terminate {} deferred: {e}", o.code);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uploader::native::client::UploadError;

    // Pure list ops are tested without a Tauri AppHandle by exercising the OrphanFile
    // upsert/clear/note logic directly.
    fn orphan(code: &str, ms: u64) -> LiveOrphan {
        LiveOrphan {
            code: code.into(),
            last_segment_id: 1,
            source_path: "C:/x/Encounter.log".into(),
            created_at_ms: ms,
        }
    }

    /// Apply `record_open`'s pure upsert to a list (factored so it's testable without
    /// disk). Mirrors the in-fn logic exactly.
    fn apply_record(list: &mut Vec<LiveOrphan>, o: LiveOrphan) {
        if let Some(existing) = list.iter_mut().find(|e| e.code == o.code) {
            *existing = o;
        } else {
            list.push(o);
        }
        list.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        list.truncate(MAX_ORPHANS);
    }

    #[test]
    fn record_upserts_by_code() {
        let mut list = Vec::new();
        apply_record(&mut list, orphan("A", 10));
        apply_record(&mut list, orphan("A", 20)); // same code → update, not duplicate
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].created_at_ms, 20);
    }

    #[test]
    fn record_keeps_newest_under_cap() {
        let mut list = Vec::new();
        for i in 0..(MAX_ORPHANS as u64 + 5) {
            apply_record(&mut list, orphan(&format!("c{i}"), i));
        }
        assert_eq!(list.len(), MAX_ORPHANS);
        // Sorted newest-first; the oldest 5 were truncated.
        assert!(list.iter().all(|o| o.created_at_ms >= 5));
    }

    #[test]
    fn clear_removes_by_code_and_is_idempotent() {
        let mut list = vec![orphan("A", 10), orphan("B", 20)];
        list.retain(|o| o.code != "A");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].code, "B");
        // Clearing an absent code is a no-op.
        let before = list.len();
        list.retain(|o| o.code != "ZZZ");
        assert_eq!(list.len(), before);
    }

    // The load-bearing keep/clear rule: only a definitive 404/410 drops the breadcrumb;
    // every transient surfaces as KEEP.
    #[test]
    fn is_definitively_closed_only_for_404_410() {
        assert!(is_definitively_closed(&UploadError::Server {
            status: 404,
            detail: String::new()
        }));
        assert!(is_definitively_closed(&UploadError::Server {
            status: 410,
            detail: String::new()
        }));
        // Transients must be KEPT (not definitively closed).
        for e in [
            UploadError::Transport("net".into()),
            UploadError::Server {
                status: 500,
                detail: String::new(),
            },
            UploadError::Server {
                status: 503,
                detail: String::new(),
            },
            UploadError::Cancelled,
        ] {
            assert!(
                !is_definitively_closed(&e),
                "{e:?} is transient and must KEEP the orphan"
            );
        }
    }
}
