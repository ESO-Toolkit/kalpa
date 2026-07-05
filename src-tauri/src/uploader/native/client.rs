//! Native ESO Logs report lifecycle client.
//!
//! Drives a report through the `/desktop-client/*` endpoints using an
//! authenticated [`session::Session`] from a [`session::SessionProvider`]:
//!
//! 1. `create-report`          → returns a report code.
//! 2. `add-report-segment/{c}` → one per converted segment (multipart).
//! 3. `set-report-master-table/{c}` → the interned master table (multipart).
//! 4. `terminate-report/{c}`   → close the report.
//!
//! **Cancellation** is a single [`AtomicBool`] checked between segments, mirroring
//! the existing `commands.rs` cancel pattern. Because nothing foreign is spawned,
//! Stop is a clean in-process flag flip plus a final `terminate-report` — the old
//! "Stop opens the official app" behavior is gone on this path.
//!
//! Clean-room: endpoint paths and the multipart envelope are protocol *facts*;
//! the request construction and lifecycle handling are implemented from scratch.
//!
//! The wire-level send is funneled through one private helper ([`NativeUpload::send`])
//! that builds each endpoint's body (JSON for create/terminate, multipart for the
//! segment/master-table uploads), attaches the session cookie, and applies a
//! single re-auth-then-retry on a `401`/`419`. The session itself is supplied by a
//! [`SessionProvider`] (the in-app login captures the cookie); the format version
//! gate ([`super::format::FORMAT_VERSION_CONFIRMED`]) still governs whether the
//! native transport is enabled by default.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use super::session::{Session, SessionError, SessionProvider};
use crate::uploader::types::UploadOptions;

/// Base for the report lifecycle endpoints. A fact about the service.
const DESKTOP_CLIENT_BASE: &str = "https://www.esologs.com/desktop-client";

/// How often the cancel-aware live send polls the cancel flag while a blocking POST
/// is in flight on a worker thread. Bounds Stop latency to ~this interval instead of
/// the 120s request timeout. Short enough to feel instant, long enough to add no
/// measurable busy-wait cost over a multi-hour session. `pub(crate)` so the live
/// driver reuses the same slice for its cancel-aware backoff/pause loops.
pub(crate) const LIVE_CANCEL_POLL: std::time::Duration = std::time::Duration::from_millis(250);

/// Grace window to let an in-flight `create-report` LAND after a Stop, so we capture its
/// report code and TERMINATE it instead of leaking an untracked remote report. Create
/// normally completes in well under a second; this only matters when a Stop races the
/// create POST. Past this window (a wedged network during create) we give up — a rare,
/// bounded leak — rather than block Stop indefinitely.
pub(crate) const CREATE_REPORT_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// A report code returned by `create-report` and used to address subsequent
/// segment/master-table/terminate calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportCode(pub String);

/// Errors from the native upload lifecycle.
#[derive(Debug, Clone)]
pub enum UploadError {
    /// Could not get/refresh an authenticated session.
    Session(SessionError),
    /// The server rejected the request (non-2xx) — carries status + short detail.
    Server { status: u16, detail: String },
    /// A transport/IO failure (network, timeout).
    Transport(String),
    /// The upload was cancelled between segments. Not a failure — the caller
    /// asked to stop; a final `terminate-report` is still attempted.
    Cancelled,
}

impl std::fmt::Display for UploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UploadError::Session(e) => write!(f, "{e}"),
            UploadError::Server { status, detail } => {
                write!(f, "ESO Logs returned {status}: {detail}")
            }
            UploadError::Transport(d) => write!(f, "Network error during upload: {d}"),
            UploadError::Cancelled => write!(f, "Upload stopped."),
        }
    }
}

impl std::error::Error for UploadError {}

impl From<SessionError> for UploadError {
    fn from(e: SessionError) -> Self {
        UploadError::Session(e)
    }
}

/// Progress callback: invoked as segments are accepted so the UI shows real
/// progress (segments POSTed / total) instead of "watch the other window".
pub type ProgressFn<'a> = dyn Fn(UploadProgress) + Send + Sync + 'a;

/// A progress tick for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadProgress {
    pub segments_done: usize,
    pub segments_total: usize,
}

/// One converted, ready-to-send segment (the serialized bytes from `convert`).
/// Holding bytes (not borrowing the log) lets the converter and the uploader run
/// on different cadences (e.g. live mode emits these per finished fight).
#[derive(Debug, Clone)]
pub struct Segment {
    pub bytes: Vec<u8>,
    /// Wall-clock ms of the segment's FIRST event, and of its LAST event. The
    /// `add-report-segment` request sends these as `startTime`/`endTime` so the
    /// server can place the segment on the report timeline and bound fight
    /// extraction. A zero-width window (both 0) yields a report with NO fights
    /// ("Fetching Fights: None") even though the segment is otherwise valid.
    pub start_time: u64,
    pub end_time: u64,
}

impl Segment {
    /// Build a segment from its rendered fights-segment **text** by ZIP-wrapping
    /// it into the `logfile` envelope (single `log.txt` entry, DEFLATE-9). This is
    /// the bridge from [`super::serialize`]'s rendered string to the wire bytes.
    /// `start_time`/`end_time` are the segment's first/last event wall-clock ms.
    pub fn from_text(text: &str, start_time: u64, end_time: u64) -> Result<Self, String> {
        Ok(Self {
            bytes: super::zip_segment::zip_log_txt(text)?,
            start_time,
            end_time,
        })
    }
}

/// The serialized master table for a report.
#[derive(Debug, Clone)]
pub struct MasterTableBytes {
    pub bytes: Vec<u8>,
}

impl MasterTableBytes {
    /// Build a master table from its rendered **text** by ZIP-wrapping it into the
    /// `logfile` envelope (single `log.txt` entry, DEFLATE-9), mirroring
    /// [`Segment::from_text`].
    pub fn from_text(text: &str) -> Result<Self, String> {
        Ok(Self {
            bytes: super::zip_segment::zip_log_txt(text)?,
        })
    }
}

/// A native upload run. Owns the session provider, the upload options (for the
/// `create-report` body), and the cancel flag; methods are the report lifecycle.
pub struct NativeUpload<'a> {
    session: &'a dyn SessionProvider,
    opts: &'a UploadOptions,
    cancel: Arc<AtomicBool>,
}

impl<'a> NativeUpload<'a> {
    pub fn new(
        session: &'a dyn SessionProvider,
        opts: &'a UploadOptions,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        Self {
            session,
            opts,
            cancel,
        }
    }

    /// True once a Stop has been requested.
    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Full lifecycle for a finished (manual) upload — a thin wrapper over
    /// [`Self::upload_finished_with_orphans`] with a no-op orphan sink, for callers that
    /// don't need crash-recovery breadcrumbs (existing call sites / tests). Preserves the
    /// original 3-argument signature so those callers compile unchanged.
    pub fn upload_finished(
        &self,
        segments: &[Segment],
        masters: &[MasterTableBytes],
        progress: &ProgressFn<'_>,
    ) -> Result<ReportCode, UploadError> {
        self.upload_finished_with_orphans(segments, masters, progress, &super::live::NoopOrphanSink)
    }

    /// Full lifecycle for a finished (manual) upload, matching the official
    /// protocol's real sequence:
    ///
    /// 1. `create-report` → report code; `segmentId` starts at **1**.
    /// 2. For each segment, **in this order**: (a) `set-report-master-table/{code}`
    ///    with the current `segmentId`, then (b) `add-report-segment/{code}` with
    ///    the **same** `segmentId`; the response carries `nextSegmentId`, which
    ///    becomes the next segment's id (the server, not the client, sequences
    ///    segment ids).
    /// 3. `terminate-report/{code}`.
    ///
    /// Master-table-before-add-segment, per segment, is the verified order (not a
    /// single master table at the end). A cancel check runs before each segment;
    /// on stop we still `terminate-report` so no dangling draft is left.
    ///
    /// `master` carries the per-segment master table aligned with `segments`
    /// (one entry each); the actual HTTP sends are routed through [`Self::send`].
    ///
    /// **Crash-recovery (L2, C2)**: `sink` receives the SAME breadcrumb discipline the
    /// live driver uses (`super::live::terminate_report_and_settle`): `record_open` the
    /// instant the report exists (right after `create-report`, BEFORE the first segment
    /// POST), `note_segment` per accepted segment, and `clear` ONLY on a confirmed close
    /// (a clean `terminate-report`, or a definitive `404`/`410` via
    /// [`is_definitively_closed`]). A transient terminate failure KEEPS the breadcrumb so
    /// next-launch recovery closes the orphaned draft. A kill/panic between
    /// `create-report` and `terminate-report` therefore always leaves a recoverable
    /// `{code}`.
    pub fn upload_finished_with_orphans(
        &self,
        segments: &[Segment],
        masters: &[MasterTableBytes],
        progress: &ProgressFn<'_>,
        sink: &dyn super::live::OrphanSink,
    ) -> Result<ReportCode, UploadError> {
        // Nothing to upload: never create+terminate an empty report (that would
        // record a zero-fight conversion or a routing bug as a "successful"
        // upload). Reject before any network work.
        if segments.is_empty() {
            return Err(UploadError::Server {
                status: 0,
                detail: "internal: no segments to upload (empty input)".into(),
            });
        }

        // Master table and fights segment are paired per segment id.
        if masters.len() != segments.len() {
            return Err(UploadError::Server {
                status: 0,
                detail: format!(
                    "internal: {} master tables for {} segments (must match)",
                    masters.len(),
                    segments.len()
                ),
            });
        }

        // Already cancelled before we started? Short-circuit before any network
        // work — there is no report to terminate yet (no breadcrumb written).
        if self.is_cancelled() {
            return Err(UploadError::Cancelled);
        }

        // 1. Establish (or fail) the session up front — fail fast before any work.
        let _session = self.session.session()?;

        // 2. create-report
        let code = self.create_report()?;

        // L2/C2: persist the crash-recovery breadcrumb the INSTANT the report exists —
        // after create-report, BEFORE the first segment POST — mirroring the live path
        // (`super::live::run_native_live`). `segmentId` starts at 1.
        sink.record_open(&code.0, 1);

        // 3+4. Push the segments and terminate. ANY error after create-report
        // (cancel, a failed master-table/segment upload, a server anomaly) must
        // still attempt `terminate-report` so we never leave an orphaned/partial
        // report open server-side (which would confuse retries). On success the
        // inner path terminates itself; on error we best-effort terminate here
        // and return the ORIGINAL error.
        match self.push_segments_and_terminate(&code, segments, masters, progress, sink) {
            Ok(()) => {
                // The inner path's own `terminate-report` returned Ok → the report is
                // confirmed closed; drop the breadcrumb (clear-only-on-confirmed-close).
                sink.clear(&code.0);
                Ok(code)
            }
            Err(e) => {
                // Best-effort terminate; clear the breadcrumb ONLY if THAT terminate
                // confirms the report is closed (clean Ok, or a definitive 404/410) —
                // the exact rule `super::live::terminate_report_and_settle` applies. A
                // transient failure KEEPS it for next-launch recovery.
                let term = self.terminate_report(&code);
                if terminate_confirms_closed(&term) {
                    sink.clear(&code.0);
                }
                Err(e)
            }
        }
    }

    /// The post-`create-report` lifecycle: per segment, master-table then
    /// add-segment (following the server-sequenced id), then `terminate-report`.
    /// Returns `Err` on cancel / upload failure / server anomaly WITHOUT
    /// terminating — the caller ([`Self::upload_finished`]) owns the
    /// terminate-on-error cleanup so every post-create error path is covered in
    /// one place.
    fn push_segments_and_terminate(
        &self,
        code: &ReportCode,
        segments: &[Segment],
        masters: &[MasterTableBytes],
        progress: &ProgressFn<'_>,
        sink: &dyn super::live::OrphanSink,
    ) -> Result<(), UploadError> {
        let total = segments.len();
        // segmentId starts at 1; the server returns the next id to use.
        let mut segment_id: u64 = 1;
        for (i, (seg, master)) in segments.iter().zip(masters.iter()).enumerate() {
            if self.is_cancelled() {
                return Err(UploadError::Cancelled);
            }
            // a. master table for this segment id, then b. the fights segment.
            self.set_master_table(code, segment_id, master)?;
            let next = self.add_segment(code, segment_id, seg)?;
            // Segment ACCEPTED → advance the breadcrumb's last-sequenced id (diagnostics;
            // terminate needs only the code). Mirrors the live driver's `note_segment`
            // after an accepted add-segment.
            sink.note_segment(&code.0, segment_id);
            progress(UploadProgress {
                segments_done: i + 1,
                segments_total: total,
            });
            let is_last_local = i + 1 == total;
            // The server sequences ids; `next == 0` is the protocol terminal. It
            // is only valid on our LAST local segment. A terminal `0` returned
            // while we still have segments to send is a server anomaly (schema
            // drift / mis-sequencing): finalizing here would silently truncate
            // the report. Fail loudly instead so a partial report never ships as
            // success. Conversely, a non-zero `next` on the last segment is fine
            // — we simply stop, having sent everything we have.
            if next == 0 && !is_last_local {
                return Err(UploadError::Server {
                    status: 0,
                    detail: format!(
                        "server returned terminal nextSegmentId=0 after segment {} of {}; \
                         remaining segments would be dropped",
                        i + 1,
                        total
                    ),
                });
            }
            if is_last_local {
                break;
            }
            segment_id = next;
        }

        // terminate (the success path).
        self.terminate_report(code)
    }

    // ── Lifecycle calls ──────────────────────────────────────────────────────
    // Each builds its endpoint and routes through `send`. Endpoint construction
    // is complete; the request *body envelope* is the pinned seam.

    fn create_report(&self) -> Result<ReportCode, UploadError> {
        let url = format!("{DESKTOP_CLIENT_BASE}/create-report");
        let body = self.send(&url, RequestKind::CreateReport)?;
        // Response is JSON `{ "code": <reportCode>, "message"?: <err> }`.
        extract_report_code(&body)
    }

    // NOTE: the live lifecycle (create / master / add-segment / terminate) is driven
    // through the cancel-aware [`LiveSender`] (below), NOT through these borrow-based
    // `send` methods — a live POST must be abandonable on Stop within ~250ms, which
    // the blocking `send` cannot do. The one-shot `upload_finished` path still uses
    // the methods below.

    /// POST the fights segment for `segment_id`; returns the server-assigned
    /// `nextSegmentId` (0 = no further segments).
    fn add_segment(
        &self,
        code: &ReportCode,
        segment_id: u64,
        seg: &Segment,
    ) -> Result<u64, UploadError> {
        let url = format!("{DESKTOP_CLIENT_BASE}/add-report-segment/{}", code.0);
        let body = self.send(
            &url,
            RequestKind::AddSegment {
                segment_id,
                bytes: &seg.bytes,
                start_time: seg.start_time,
                end_time: seg.end_time,
            },
        )?;
        // Response is JSON `{ "nextSegmentId": <n> }`. A malformed body is a hard
        // error (not a silent end-of-upload) — see `extract_next_segment_id`.
        extract_next_segment_id(&body)
    }

    fn set_master_table(
        &self,
        code: &ReportCode,
        segment_id: u64,
        master: &MasterTableBytes,
    ) -> Result<(), UploadError> {
        let url = format!("{DESKTOP_CLIENT_BASE}/set-report-master-table/{}", code.0);
        self.send(
            &url,
            RequestKind::MasterTable {
                segment_id,
                bytes: &master.bytes,
            },
        )
        .map(|_| ())
    }

    fn terminate_report(&self, code: &ReportCode) -> Result<(), UploadError> {
        let url = format!("{DESKTOP_CLIENT_BASE}/terminate-report/{}", code.0);
        self.send(&url, RequestKind::Terminate).map(|_| ())
    }

    /// The single wire-send seam. Builds the request body for `kind`, attaches the
    /// session cookie, sends it, and maps the response to bytes (or a structured
    /// error). On a `401`/`419` it invalidates the session once and retries a
    /// single time with a freshly-fetched session, mirroring the official client's
    /// re-auth-then-retry behaviour; a second auth rejection is surfaced as
    /// [`SessionError::Expired`] so the caller can prompt a re-login.
    ///
    /// Clean-room: the endpoint shapes (JSON create/terminate, multipart segment/
    /// master-table with these field names) are protocol facts; the request
    /// construction is implemented from scratch here.
    fn send(&self, url: &str, kind: RequestKind) -> Result<Vec<u8>, UploadError> {
        let mut session = self.session.session()?;
        // One re-auth retry on an auth rejection (401/419), then give up.
        for attempt in 0..2 {
            let resp = self.send_once(url, &kind, &session);
            match resp {
                Ok(SendResult::Ok(body)) => return Ok(body),
                Ok(SendResult::AuthRejected) if attempt == 0 => {
                    // Stale session: drop it and try once more with a fresh one.
                    self.session.invalidate();
                    session = match self.session.session() {
                        Ok(s) => s,
                        Err(_) => return Err(UploadError::Session(SessionError::Expired)),
                    };
                    continue;
                }
                Ok(SendResult::AuthRejected) => {
                    return Err(UploadError::Session(SessionError::Expired));
                }
                Ok(SendResult::ServerError { status, detail }) => {
                    return Err(UploadError::Server { status, detail });
                }
                Err(transport) => return Err(UploadError::Transport(transport)),
            }
        }
        // The loop always returns within two iterations.
        unreachable!("send retry loop must return")
    }

    /// Perform exactly one HTTP attempt for `kind` with `session`. Returns a
    /// [`SendResult`] classifying the outcome (so `send` can decide on retry), or
    /// `Err(String)` for a transport/IO failure. No retry logic lives here.
    fn send_once(
        &self,
        url: &str,
        kind: &RequestKind,
        session: &Session,
    ) -> Result<SendResult, String> {
        // C4: scale the total request timeout with the multipart body size — a large
        // segment/master-table upload on a slow uplink must finish inside ONE timeout
        // (the one-shot path has no retry); create/terminate carry no body → the flat
        // floor. Applied per request because it varies with the payload; the C3
        // no-redirect policy rides on the shared client (see `upload_client`).
        let payload_len = match kind {
            RequestKind::AddSegment { bytes, .. } | RequestKind::MasterTable { bytes, .. } => {
                bytes.len()
            }
            RequestKind::CreateReport | RequestKind::Terminate => 0,
        };
        let req = upload_client()
            .post(url)
            .timeout(request_timeout_for(payload_len))
            .header(reqwest::header::COOKIE, session.cookie_header())
            .header(reqwest::header::ACCEPT, "application/json");

        let req = match kind {
            RequestKind::CreateReport => req
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(self.create_report_body()),
            RequestKind::Terminate => req, // no body
            RequestKind::AddSegment {
                segment_id,
                bytes,
                start_time,
                end_time,
            } => {
                let form = reqwest::blocking::multipart::Form::new()
                    .text(
                        "parameters",
                        segment_parameters_json(*segment_id, *start_time, *end_time),
                    )
                    .part("logfile", segment_logfile_part(bytes)?);
                req.multipart(form)
            }
            RequestKind::MasterTable { segment_id, bytes } => {
                let form = reqwest::blocking::multipart::Form::new()
                    .text("segmentId", segment_id.to_string())
                    .text("isRealTime", "false")
                    .part("logfile", segment_logfile_part(bytes)?);
                req.multipart(form)
            }
        };

        let resp = req.send().map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status();
        // C3: classify by (status, headers) BEFORE reading the body so a 401/419 — or a
        // redirect to a login page (an expired session bounce) — engages the re-auth
        // retry instead of being read back as a fatal server error.
        if let StatusClass::AuthRejected = classify_status(status, resp.headers()) {
            return Ok(SendResult::AuthRejected);
        }
        let code = status.as_u16();
        let body = resp.bytes().map_err(|e| format!("read body failed: {e}"))?;
        if status.is_success() {
            Ok(SendResult::Ok(body.to_vec()))
        } else {
            // Surface a short, non-secret detail for diagnostics.
            let detail = String::from_utf8_lossy(&body)
                .chars()
                .take(200)
                .collect::<String>();
            Ok(SendResult::ServerError {
                status: code,
                detail,
            })
        }
    }

    /// Build the `create-report` JSON body. Delegates to [`create_report_body_for`]
    /// so the borrow-based one-shot path and the owned live path build an identical
    /// body.
    fn create_report_body(&self) -> Vec<u8> {
        create_report_body_for(self.opts)
    }
}

/// Build the `create-report` JSON body for `opts`. Ten fields, matching the confirmed
/// live request: a fresh report is created with `startTime == endTime` at creation
/// time (the server backfills the real range from the segments). Serialized via
/// `serde_json` so free-text fields (`guildId`, `description`) are correctly
/// quoted/escaped. Shared by [`NativeUpload::create_report_body`] (one-shot) and the
/// live driver (which builds an [`OwnedLiveRequest::CreateReport`]).
pub(crate) fn create_report_body_for(opts: &UploadOptions) -> Vec<u8> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let body = serde_json::json!({
        "clientVersion": super::format::CLIENT_VERSION,
        "parserVersion": super::format::FORMAT_VERSION,
        "startTime": now_ms,
        "endTime": now_ms,
        "guildId": opts.guild_id,
        "fileName": "log.txt",
        "serverOrRegion": opts.region,
        "visibility": opts.visibility.as_report_visibility_id(),
        "reportTagId": serde_json::Value::Null,
        "description": opts.description.as_deref().unwrap_or(""),
    });
    serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec())
}

/// The `/desktop-client` base URL (a protocol fact). Exposed so the live driver can
/// build its endpoint URLs without duplicating the constant.
pub(crate) fn desktop_client_base() -> &'static str {
    DESKTOP_CLIENT_BASE
}

/// Parse a `create-report` response body into a [`ReportCode`] (see
/// [`extract_report_code`]). Exposed for the live driver, which sends create-report
/// through the cancel-aware [`LiveSender`] rather than [`NativeUpload::create_report`].
pub(crate) fn parse_report_code(body: &[u8]) -> Result<ReportCode, UploadError> {
    extract_report_code(body)
}

/// Parse an `add-report-segment` response body into a `nextSegmentId` (see
/// [`extract_next_segment_id`]). Exposed for the live driver.
pub(crate) fn parse_next_segment_id(body: &[u8]) -> Result<u64, UploadError> {
    extract_next_segment_id(body)
}

/// Whether a terminate/upload error means the report is already closed/gone (so an
/// orphan breadcrumb can be dropped and a terminate is unnecessary). A 404/410 is
/// definitive (report doesn't exist / already terminated / server auto-expired).
/// Everything else — timeout, 5xx, connection error, a `Session(Expired)` that
/// survived the one re-auth — is TRANSIENT, so the orphan is KEPT for a later retry.
pub(crate) fn is_definitively_closed(e: &UploadError) -> bool {
    matches!(
        e,
        UploadError::Server { status: 404, .. } | UploadError::Server { status: 410, .. }
    )
}

/// Whether a `terminate-report` outcome confirms the report is CLOSED, so the L2
/// crash-recovery breadcrumb may be dropped: a clean terminate (`Ok`) or a definitive
/// already-gone (`404`/`410` via [`is_definitively_closed`]). Every transient failure
/// returns `false` → KEEP the breadcrumb for next-launch recovery. This is the SAME rule
/// `super::live::terminate_report_and_settle` applies, lifted into a pure fn so the
/// one-shot path (C2) shares the discipline and it stays unit-testable without a server.
fn terminate_confirms_closed(term: &Result<(), UploadError>) -> bool {
    match term {
        Ok(()) => true,
        Err(e) => is_definitively_closed(e),
    }
}

// ── Cancel-aware LIVE sender (L4) ────────────────────────────────────────────
//
// The shared `send`/`send_once` above use `reqwest::blocking` with a 120s timeout
// and check cancellation only BETWEEN segments, so a Stop during a mid-POST
// `req.send()` would hang up to ~120s. For a multi-hour live session that is
// unacceptable (and it would block the Tokio executor thread that joins the driver
// on Stop — the concurrency review's RACE-1). [`LiveSender`] is an OWNED,
// `'static`-safe analog used ONLY by the live driver: it runs each POST on a
// detached worker thread and lets the caller poll the cancel flag every
// [`LIVE_CANCEL_POLL`], returning [`UploadError::Cancelled`] within ~250ms and
// ABANDONING the in-flight POST (it completes server-side; we just stop waiting —
// and since cancel always leads to terminate, no further segment is sent, so the
// abandoned POST is harmless). This is ADDITIVE: it does not touch the proven
// one-shot `send`/`send_once`, which the manual `upload_finished` path still uses.
//
// Clean-room: the request envelopes are the same protocol facts; the construction
// here is implemented from scratch (it cannot borrow `NativeUpload`'s `&dyn
// SessionProvider`, which is not `'static`, so the sender owns an
// `Arc<dyn SessionProvider>` and clones the bytes to cross the thread boundary).

/// An owned, cloneable request body for the live path (the bytes are owned so the
/// request can be built on a worker thread). Mirrors the live arms of
/// [`RequestKind`] but carries `Vec<u8>` instead of a borrow.
///
/// Gated with the rest of the live path until the de-gate step; the live module is
/// unreachable in release until then, so this can't ship by accident.
#[derive(Clone)]
pub(crate) enum OwnedLiveRequest {
    /// `create-report` — JSON body.
    CreateReport { body: Vec<u8> },
    /// `set-report-master-table/{code}` in live mode (`isRealTime=true`).
    MasterTable { segment_id: u64, bytes: Vec<u8> },
    /// `add-report-segment/{code}` in live mode.
    AddSegment {
        segment_id: u64,
        bytes: Vec<u8>,
        start_time: u64,
        end_time: u64,
        in_progress_event_count: u64,
    },
    /// `terminate-report/{code}` — no body.
    Terminate,
}

/// The POST + session-probe seam the live driver depends on. [`LiveSender`] is the
/// production impl (real cancel-aware HTTP); a scripted fake implements it in tests so
/// the driver's retry / pause-resume / idle state machine is deterministically
/// testable without a server.
pub(crate) trait LivePoster {
    /// Send one live request, cancel-aware (returns `Cancelled` fast on stop).
    fn post(
        &self,
        url: &str,
        req: OwnedLiveRequest,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Vec<u8>, UploadError>;
    /// Whether a usable session is currently available (polled while paused on a lost
    /// session to detect a re-login).
    fn has_session(&self) -> bool;
}

/// An owned, `'static`-safe sender for the live path. Holds the cookie session
/// provider as an `Arc` (so it can move into a detached worker) and applies the same
/// single 401/419 re-auth-then-retry the shared `send` does. Built from the managed
/// [`super::session::StoredSessionProvider`] for the production live driver.
#[derive(Clone)]
pub(crate) struct LiveSender {
    session: Arc<dyn SessionProvider>,
}

impl LiveSender {
    pub(crate) fn new(session: Arc<dyn SessionProvider>) -> Self {
        Self { session }
    }

    /// Whether a usable session is currently available (without prompting). The live
    /// driver polls this while paused on a lost session to detect a re-login. A `true`
    /// result does not guarantee the server still accepts it — only a request can —
    /// but it rules out the not-signed-in case the pause is waiting on.
    pub(crate) fn has_live_session(&self) -> bool {
        self.session.session().is_ok()
    }
}

impl LivePoster for LiveSender {
    fn post(
        &self,
        url: &str,
        req: OwnedLiveRequest,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Vec<u8>, UploadError> {
        self.send_cancellable(url, req, cancel)
    }
    fn has_session(&self) -> bool {
        self.has_live_session()
    }
}

impl LiveSender {
    /// Send `req` to `url`, cancel-aware: the blocking POST runs on a detached
    /// worker; this returns [`UploadError::Cancelled`] within ~[`LIVE_CANCEL_POLL`]
    /// if `cancel` is set, abandoning the in-flight POST. The 120s reqwest timeout
    /// stays as the absolute backstop on the abandoned worker, not the Stop latency.
    pub(crate) fn send_cancellable(
        &self,
        url: &str,
        req: OwnedLiveRequest,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Vec<u8>, UploadError> {
        if cancel.load(Ordering::SeqCst) {
            return Err(UploadError::Cancelled);
        }
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Vec<u8>, UploadError>>(1);
        let session = Arc::clone(&self.session);
        let url = url.to_string();
        // Detached worker: it owns everything it needs and outlives an abandoned
        // wait. NOT `thread::scope` — scope's implicit join at scope end would
        // re-block on cancel, defeating the whole purpose.
        std::thread::spawn(move || {
            let result = live_send_with_reauth(&session, &url, &req);
            // The receiver may already be gone (we abandoned the POST on cancel);
            // a failed send just drops the result, which is fine.
            let _ = tx.send(result);
        });
        wait_for_send_or_cancel(rx, cancel)
    }

    /// Like [`Self::send_cancellable`] but for `create-report`, which must NEVER be
    /// abandoned mid-flight: the server can create the report after we give up, leaving
    /// an untracked orphan (no code to record/terminate, uncatchable by next-launch
    /// recovery). On cancel this waits up to [`CREATE_REPORT_GRACE`] for the worker to
    /// return the code so the caller can record + terminate it; only if the grace expires
    /// with no result does it surface `Cancelled`. Fast-Stop is preserved for the normal
    /// (non-create) cancel path — this divergence is create-only.
    pub(crate) fn send_create_cancellable(
        &self,
        url: &str,
        req: OwnedLiveRequest,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Vec<u8>, UploadError> {
        if cancel.load(Ordering::SeqCst) {
            return Err(UploadError::Cancelled);
        }
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Vec<u8>, UploadError>>(1);
        let session = Arc::clone(&self.session);
        let url = url.to_string();
        std::thread::spawn(move || {
            let _ = tx.send(live_send_with_reauth(&session, &url, &req));
        });
        wait_for_create_or_cancel(rx, cancel, CREATE_REPORT_GRACE)
    }
}

/// Wait on a worker's result channel, polling `cancel` every [`LIVE_CANCEL_POLL`].
/// Returns the worker's result on completion, [`UploadError::Cancelled`] if cancel
/// trips first, or a transport error if the worker dropped the channel. Extracted
/// from [`LiveSender::send_cancellable`] so the cancel-latency contract is unit-
/// testable without a real HTTP worker (a fake sender that never completes).
fn wait_for_send_or_cancel(
    rx: std::sync::mpsc::Receiver<Result<Vec<u8>, UploadError>>,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<u8>, UploadError> {
    loop {
        match rx.recv_timeout(LIVE_CANCEL_POLL) {
            Ok(result) => return result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if cancel.load(Ordering::SeqCst) {
                    return Err(UploadError::Cancelled);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(UploadError::Transport(
                    "live upload worker terminated unexpectedly".into(),
                ));
            }
        }
    }
}

/// Like [`wait_for_send_or_cancel`] but for CREATE-REPORT: on cancel it does NOT abandon
/// the in-flight POST. It first waits cancel-aware (fast Stop); once cancel trips it waits
/// up to `grace` for the worker to deliver the report code, RETURNING it (`Ok`) so the
/// caller can record + terminate the just-created report instead of leaking it. Only if
/// the grace expires (or the worker dies) with no code does it surface `Cancelled`.
/// Extracted so the no-leak-on-cancel contract is unit-testable without a real worker.
fn wait_for_create_or_cancel(
    rx: std::sync::mpsc::Receiver<Result<Vec<u8>, UploadError>>,
    cancel: &Arc<AtomicBool>,
    grace: std::time::Duration,
) -> Result<Vec<u8>, UploadError> {
    // Phase 1: fast cancel-aware wait (same as the normal path).
    loop {
        match rx.recv_timeout(LIVE_CANCEL_POLL) {
            Ok(result) => return result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(UploadError::Transport(
                    "live upload worker terminated unexpectedly".into(),
                ));
            }
        }
    }
    // Phase 2 (cancelled mid-create): wait `grace` for the report to land so we can
    // capture its code and clean it up. No code in time ⇒ accept the rare bounded leak.
    match rx.recv_timeout(grace) {
        Ok(result) => result,
        Err(_) => Err(UploadError::Cancelled),
    }
}

/// One live request with the shared single 401/419 re-auth-then-retry, run on a
/// worker thread (so it takes owned data). Mirrors [`NativeUpload::send`]'s retry
/// loop but for the owned live request; the actual wire attempt is
/// [`live_send_once`]. Kept separate from the borrow-based `send` so the one-shot
/// path is untouched.
fn live_send_with_reauth(
    session: &Arc<dyn SessionProvider>,
    url: &str,
    req: &OwnedLiveRequest,
) -> Result<Vec<u8>, UploadError> {
    let mut sess = session.session()?;
    for attempt in 0..2 {
        match live_send_once(&sess, url, req) {
            Ok(SendResult::Ok(body)) => return Ok(body),
            Ok(SendResult::AuthRejected) if attempt == 0 => {
                session.invalidate();
                sess = match session.session() {
                    Ok(s) => s,
                    Err(_) => return Err(UploadError::Session(SessionError::Expired)),
                };
                continue;
            }
            Ok(SendResult::AuthRejected) => {
                return Err(UploadError::Session(SessionError::Expired));
            }
            Ok(SendResult::ServerError { status, detail }) => {
                return Err(UploadError::Server { status, detail });
            }
            Err(transport) => return Err(UploadError::Transport(transport)),
        }
    }
    unreachable!("live send retry loop must return within two iterations")
}

/// Perform exactly one HTTP attempt for an owned live request. Builds the same
/// multipart/JSON envelopes the borrow-based [`NativeUpload::send_once`] does, from
/// owned data. No retry logic here.
fn live_send_once(
    session: &Session,
    url: &str,
    req: &OwnedLiveRequest,
) -> Result<SendResult, String> {
    // Same C3/C4 hardening as the one-shot `send_once`: a size-scaled per-request
    // timeout for the (small, live) segment/master bodies, and redirects disabled on
    // the shared `upload_client` so an auth bounce is classified for re-auth rather
    // than followed and misread. Live payloads are small, so the timeout formula
    // naturally yields ~120s here.
    let payload_len = match req {
        OwnedLiveRequest::AddSegment { bytes, .. }
        | OwnedLiveRequest::MasterTable { bytes, .. } => bytes.len(),
        OwnedLiveRequest::CreateReport { body } => body.len(),
        OwnedLiveRequest::Terminate => 0,
    };
    let base = upload_client()
        .post(url)
        .timeout(request_timeout_for(payload_len))
        .header(reqwest::header::COOKIE, session.cookie_header())
        .header(reqwest::header::ACCEPT, "application/json");

    let base = match req {
        OwnedLiveRequest::CreateReport { body } => base
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.clone()),
        OwnedLiveRequest::Terminate => base, // no body
        OwnedLiveRequest::MasterTable { segment_id, bytes } => {
            let form = reqwest::blocking::multipart::Form::new()
                .text("segmentId", segment_id.to_string())
                .text("isRealTime", "true")
                .part("logfile", segment_logfile_part(bytes)?);
            base.multipart(form)
        }
        OwnedLiveRequest::AddSegment {
            segment_id,
            bytes,
            start_time,
            end_time,
            in_progress_event_count,
        } => {
            let form = reqwest::blocking::multipart::Form::new()
                .text(
                    "parameters",
                    segment_parameters_json_live(
                        *segment_id,
                        *start_time,
                        *end_time,
                        *in_progress_event_count,
                    ),
                )
                .part("logfile", segment_logfile_part(bytes)?);
            base.multipart(form)
        }
    };

    let resp = base.send().map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    // C3: same status+headers classification as the one-shot path.
    if let StatusClass::AuthRejected = classify_status(status, resp.headers()) {
        return Ok(SendResult::AuthRejected);
    }
    let code = status.as_u16();
    let body = resp.bytes().map_err(|e| format!("read body failed: {e}"))?;
    if status.is_success() {
        Ok(SendResult::Ok(body.to_vec()))
    } else {
        let detail = String::from_utf8_lossy(&body)
            .chars()
            .take(200)
            .collect::<String>();
        Ok(SendResult::ServerError {
            status: code,
            detail,
        })
    }
}

/// The classified outcome of a single HTTP attempt, so [`NativeUpload::send`] can
/// apply retry/auth logic without re-inspecting the response.
enum SendResult {
    /// 2xx — the response body.
    Ok(Vec<u8>),
    /// 401/419 — the session was rejected; caller may re-auth and retry.
    AuthRejected,
    /// Other non-2xx — a hard server error with a short detail.
    ServerError { status: u16, detail: String },
}

/// The status-only classification of a response, decided from `(status, headers)`
/// BEFORE the body is read, so the send loops can engage the re-auth retry without
/// misreading a login page as a fatal error.
enum StatusClass {
    /// Re-authenticate: a `401`/`419`, or a redirect whose target is a login page (an
    /// expired session bounce) — see [`classify_status`].
    AuthRejected,
    /// A normal 2xx/4xx/5xx: read the body and decide success vs. server error by status.
    ReadBody,
}

/// Classify an HTTP response by status + headers alone (C3). `401`/`419` are the
/// server's direct auth rejections (`419` = Laravel CSRF/session mismatch). With
/// client-side redirect-following DISABLED (see the builders in
/// [`NativeUpload::send_once`] / [`live_send_once`]), a `3xx` on these same-host POST
/// endpoints is anomalous; the dominant cause is an expired session redirecting to the
/// login page. Under reqwest's default policy that redirect would be followed POST→GET,
/// return login HTML with `200`, fail JSON extraction, and be misclassified as a fatal
/// `Server` error — bypassing the re-auth machinery. So a redirect to a login page (or
/// one whose `Location` we cannot read, failing conservatively toward re-auth) is
/// reported as [`StatusClass::AuthRejected`] to engage the single re-auth retry and the
/// live pause-reauth. Everything else defers to a body read.
///
/// Pure over `(status, headers)` so the classification is unit-testable without a server.
fn classify_status(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
) -> StatusClass {
    if status == reqwest::StatusCode::UNAUTHORIZED || status.as_u16() == 419 {
        return StatusClass::AuthRejected;
    }
    if status.is_redirection() && redirect_is_auth_related(headers) {
        return StatusClass::AuthRejected;
    }
    StatusClass::ReadBody
}

/// Whether a `3xx`'s `Location` indicates an auth bounce: it points at a login/auth page,
/// OR it is missing/unreadable (fail conservatively toward re-auth rather than treat the
/// redirect as a fatal server error). Same-host redirects are the only ones these
/// endpoints emit, so a login `Location` is the expired-session signal.
fn redirect_is_auth_related(headers: &reqwest::header::HeaderMap) -> bool {
    match headers.get(reqwest::header::LOCATION) {
        Some(loc) => match loc.to_str() {
            Ok(s) => {
                let l = s.to_ascii_lowercase();
                l.contains("/login") || l.contains("signin") || l.contains("sign-in")
            }
            // An unreadable Location on a redirect: fail toward re-auth.
            Err(_) => true,
        },
        // No Location on a 3xx: unusual for these endpoints — fail toward re-auth.
        None => true,
    }
}

/// Shared blocking client for every desktop-client POST (one-shot and live paths).
/// Reusing one client keeps the connection pool — and its TLS session — alive across
/// the many per-segment uploads of a report, instead of paying a fresh handshake and
/// client runtime per request. Built WITHOUT a client-level timeout: the C4
/// size-scaled timeout varies per request and is applied via
/// `RequestBuilder::timeout` at each call site (which overrides any client default).
/// C3 lives here: redirect-following is disabled so an expired-session bounce to
/// /login is classified as an auth rejection (see `classify_status`) instead of being
/// followed POST→GET and misread as a fatal server error.
fn upload_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            // Idle connections are dropped before typical server keep-alive
            // windows close them: the one-shot path has no retry, so a pooled
            // connection the server already closed during an encoding gap
            // between segments must not be handed back to it.
            .pool_idle_timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build native upload HTTP client")
    })
}

/// Total per-request timeout for a POST carrying a `payload_bytes` multipart body (C4).
/// `create-report`/`terminate-report` carry no large body → the flat `BASE_SECS` floor
/// (payload 0 ⇒ exactly the floor). Segment/master-table POSTs can be tens of MB near
/// the native size ceiling; the whole multipart body must upload inside this single
/// total timeout, so on a slow uplink a flat 120s would abort a large one-shot segment
/// mid-upload (a `Transport` error → the no-retry one-shot path fails the report). We
/// add time proportional to the payload at an assumed worst-case throughput floor,
/// capped so a genuinely wedged upload still fails in bounded time. Live segments are
/// small, so this yields ~`BASE_SECS` for them (behavior effectively unchanged).
///
/// Pure fn so the computation is unit-testable.
fn request_timeout_for(payload_bytes: usize) -> std::time::Duration {
    // Flat floor for control-plane POSTs and small bodies.
    const BASE_SECS: u64 = 120;
    // Assumed worst-case sustained upload throughput (~0.5 Mbps): grant 1s of headroom
    // per this many payload bytes so a large segment still fits inside the total timeout.
    const MIN_THROUGHPUT_BYTES_PER_SEC: u64 = 64 * 1024;
    // Absolute ceiling so a wedged connection can't hold the request open ~forever.
    const CAP_SECS: u64 = 15 * 60;
    let extra_secs = payload_bytes as u64 / MIN_THROUGHPUT_BYTES_PER_SEC;
    let total_secs = BASE_SECS.saturating_add(extra_secs).min(CAP_SECS);
    std::time::Duration::from_secs(total_secs)
}

/// Which lifecycle call a `send` is performing — selects the envelope shape.
/// The multipart calls carry the segment id (for the form/parameters) and the
/// already-serialized, ZIP-compressed segment/master bytes.
enum RequestKind<'a> {
    CreateReport,
    AddSegment {
        segment_id: u64,
        bytes: &'a [u8],
        start_time: u64,
        end_time: u64,
    },
    MasterTable {
        segment_id: u64,
        bytes: &'a [u8],
    },
    Terminate,
}

/// The `parameters` JSON for `add-report-segment` (a manual, finished upload:
/// not live, not real-time, no in-progress events). `startTime`/`endTime` are the
/// segment's first/last event **wall-clock ms** — the server uses them to place
/// the segment on the report timeline; sending 0/0 yields a zero-width segment and
/// a report with no extractable fights ("Fetching Fights: None").
fn segment_parameters_json(segment_id: u64, start_time: u64, end_time: u64) -> String {
    format!(
        "{{\"startTime\":{start_time},\"endTime\":{end_time},\"mythic\":0,\"isLiveLog\":false,\
         \"isRealTime\":false,\"inProgressEventCount\":0,\"segmentId\":{segment_id}}}"
    )
}

/// The `parameters` JSON for a LIVE `add-report-segment` (debug-only). Same shape as
/// the one-shot variant but with `isLiveLog`/`isRealTime` true and a caller-supplied
/// `inProgressEventCount` (the count of events in an unfinished fight at the segment
/// tail; 0 when the segment ends on a fight boundary). The exact server semantics of
/// these flags on an OPEN report are UNVERIFIED — settling them is the live spike's
/// one open question (see `docs/native-live-streaming-spike-FINDINGS.md`).
fn segment_parameters_json_live(
    segment_id: u64,
    start_time: u64,
    end_time: u64,
    in_progress_event_count: u64,
) -> String {
    format!(
        "{{\"startTime\":{start_time},\"endTime\":{end_time},\"mythic\":0,\"isLiveLog\":true,\
         \"isRealTime\":true,\"inProgressEventCount\":{in_progress_event_count},\
         \"segmentId\":{segment_id}}}"
    )
}

/// Build the `logfile` multipart part from already-compressed segment bytes. The
/// part is sent with filename `"blob"` and an octet-stream type, matching the
/// confirmed envelope. `bytes` is the ZIP-compressed segment produced by the
/// serializer (a single `log.txt` entry).
fn segment_logfile_part(bytes: &[u8]) -> Result<reqwest::blocking::multipart::Part, String> {
    reqwest::blocking::multipart::Part::bytes(bytes.to_vec())
        .file_name("blob")
        .mime_str("application/octet-stream")
        .map_err(|e| format!("multipart part error: {e}"))
}

/// Pull the report code from a `create-report` response body.
///
/// The response is JSON `{ "code": <reportCode>, "message"?: <error> }`. The
/// `code` field addresses every subsequent call, so a missing/empty code is a
/// hard error (with the server's `message` surfaced when present). `code` is
/// accepted as either a JSON string or number (its concrete type is unconfirmed,
/// so we normalize both).
fn extract_report_code(body: &[u8]) -> Result<ReportCode, UploadError> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|e| UploadError::Server {
        status: 0,
        detail: format!("create-report response was not JSON: {e}"),
    })?;
    let code = match v.get("code") {
        Some(serde_json::Value::String(s)) if !s.is_empty() => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => {
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("no report code in response")
                .to_string();
            return Err(UploadError::Server {
                status: 0,
                detail: format!("create-report did not return a code: {msg}"),
            });
        }
    };
    Ok(ReportCode(code))
}

/// Pull `nextSegmentId` from an `add-report-segment` response body. The server
/// sequences segment ids; a numeric `0` is the protocol's explicit "no further
/// segments" terminal. A missing/non-numeric field or a non-JSON body is NOT
/// treated as a terminal — that would let a transient schema drift, proxy error
/// page, or partial response silently stop the upload and finalize an INCOMPLETE
/// report as success. Such bodies are a hard [`UploadError::Server`] so the
/// caller fails loudly instead of shipping a truncated report.
fn extract_next_segment_id(body: &[u8]) -> Result<u64, UploadError> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|e| UploadError::Server {
        status: 0,
        detail: format!("add-report-segment response was not JSON: {e}"),
    })?;
    match v.get("nextSegmentId").and_then(|n| n.as_u64()) {
        Some(n) => Ok(n),
        None => Err(UploadError::Server {
            status: 0,
            detail: "add-report-segment response missing a numeric nextSegmentId".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uploader::native::session::Session;
    use std::sync::atomic::AtomicBool;

    /// A session provider that always returns a fixed session — lets us test the
    /// lifecycle control flow (cancellation, ordering) without any network.
    struct FakeSession {
        invalidated: std::sync::Mutex<bool>,
    }
    impl SessionProvider for FakeSession {
        fn session(&self) -> Result<Session, SessionError> {
            Ok(Session::from_cookie_header("laravel_session=test"))
        }
        fn invalidate(&self) {
            *self.invalidated.lock().unwrap() = true;
        }
    }

    fn no_progress(_p: UploadProgress) {}

    #[test]
    fn cancel_before_first_segment_short_circuits() {
        let sess = FakeSession {
            invalidated: std::sync::Mutex::new(false),
        };
        let cancel = Arc::new(AtomicBool::new(true)); // already cancelled
        let opts = UploadOptions::default();
        let up = NativeUpload::new(&sess, &opts, cancel);
        let segs = vec![Segment {
            bytes: vec![1, 2, 3],
            start_time: 0,
            end_time: 0,
        }];
        let masters = vec![MasterTableBytes { bytes: vec![] }];
        let err = up
            .upload_finished(&segs, &masters, &no_progress)
            .unwrap_err();
        // An already-cancelled upload short-circuits BEFORE any network work
        // (the early cancel check), so this is a clean `Cancelled` with no HTTP.
        assert!(matches!(err, UploadError::Cancelled));
    }

    #[test]
    fn mismatched_master_and_segment_counts_are_rejected() {
        let sess = FakeSession {
            invalidated: std::sync::Mutex::new(false),
        };
        let opts = UploadOptions::default();
        let up = NativeUpload::new(&sess, &opts, Arc::new(AtomicBool::new(false)));
        let segs = vec![
            Segment {
                bytes: vec![1],
                start_time: 0,
                end_time: 0,
            },
            Segment {
                bytes: vec![2],
                start_time: 0,
                end_time: 0,
            },
        ];
        let masters = vec![MasterTableBytes { bytes: vec![] }]; // only 1
        let err = up
            .upload_finished(&segs, &masters, &no_progress)
            .unwrap_err();
        assert!(matches!(err, UploadError::Server { .. }));
    }

    #[test]
    fn empty_segments_are_rejected_before_any_report() {
        // An empty input must NOT create+terminate an empty report and report
        // success — it is a local error caught before any network work.
        let sess = FakeSession {
            invalidated: std::sync::Mutex::new(false),
        };
        let opts = UploadOptions::default();
        let up = NativeUpload::new(&sess, &opts, Arc::new(AtomicBool::new(false)));
        let err = up.upload_finished(&[], &[], &no_progress).unwrap_err();
        match err {
            UploadError::Server { detail, .. } => assert!(detail.contains("no segments")),
            other => panic!("expected empty-input Server error, got {other:?}"),
        }
    }

    #[test]
    fn upload_error_messages_are_human_readable() {
        assert!(UploadError::Cancelled.to_string().contains("stopped"));
        assert!(UploadError::Server {
            status: 419,
            detail: "CSRF".into()
        }
        .to_string()
        .contains("419"));
    }

    #[test]
    fn extract_report_code_parses_string_and_number() {
        assert_eq!(
            extract_report_code(br#"{"code":"aBcD123"}"#).unwrap(),
            ReportCode("aBcD123".into())
        );
        assert_eq!(
            extract_report_code(br#"{"code":987654}"#).unwrap(),
            ReportCode("987654".into())
        );
    }

    #[test]
    fn extract_report_code_surfaces_server_message_on_missing_code() {
        let err = extract_report_code(br#"{"message":"guild not found"}"#).unwrap_err();
        match err {
            UploadError::Server { detail, .. } => assert!(detail.contains("guild not found")),
            other => panic!("expected Server error, got {other:?}"),
        }
        // Non-JSON body is also a structured error, not a panic.
        assert!(extract_report_code(b"<html>nope</html>").is_err());
    }

    #[test]
    fn create_report_body_is_valid_json_for_tricky_descriptions() {
        use crate::uploader::types::Visibility;
        let sess = FakeSession {
            invalidated: std::sync::Mutex::new(false),
        };
        // A description with quotes + backslashes + a guild id: the previous
        // hand-built body double-quoted these and produced invalid JSON.
        let opts = UploadOptions {
            region: 1,
            guild_id: Some("g\"123\\x".into()),
            visibility: Visibility::Private,
            description: Some(r#"raid "night" \o/"#.into()),
            real_time: false,
            include_entire_file: false,
        };
        let up = NativeUpload::new(&sess, &opts, Arc::new(AtomicBool::new(false)));
        let body = up.create_report_body();
        let v: serde_json::Value =
            serde_json::from_slice(&body).expect("create-report body must be valid JSON");
        assert_eq!(v["description"], serde_json::json!(r#"raid "night" \o/"#));
        assert_eq!(v["guildId"], serde_json::json!("g\"123\\x"));
        assert_eq!(v["visibility"], serde_json::json!(1));
        assert_eq!(
            v["parserVersion"],
            serde_json::json!(super::super::format::FORMAT_VERSION)
        );
        assert_eq!(v["fileName"], serde_json::json!("log.txt"));
        // Absent description/guild: description → "" , guildId → null.
        let opts2 = UploadOptions::default();
        let up2 = NativeUpload::new(&sess, &opts2, Arc::new(AtomicBool::new(false)));
        let v2: serde_json::Value =
            serde_json::from_slice(&up2.create_report_body()).expect("valid JSON");
        assert_eq!(v2["description"], serde_json::json!(""));
        assert_eq!(v2["guildId"], serde_json::Value::Null);
    }

    #[test]
    fn create_report_body_pins_all_visibility_ids() {
        use crate::uploader::types::Visibility;

        for (visibility, expected) in [
            (Visibility::Public, 0),
            (Visibility::Private, 1),
            (Visibility::Unlisted, 2),
        ] {
            let opts = UploadOptions {
                visibility,
                ..UploadOptions::default()
            };
            let body = create_report_body_for(&opts);
            let v: serde_json::Value =
                serde_json::from_slice(&body).expect("create-report body must be valid JSON");
            assert_eq!(
                v["visibility"],
                serde_json::json!(expected),
                "{visibility:?} must match ESO Logs' reportVisibilityId table"
            );
        }
    }

    #[test]
    fn live_segment_parameters_pin_fight_extraction_fields() {
        let params = segment_parameters_json_live(7, 1_700_000_000_100, 1_700_000_003_900, 42);
        let v: serde_json::Value =
            serde_json::from_str(&params).expect("live segment parameters must be valid JSON");

        assert_eq!(v["segmentId"], serde_json::json!(7));
        assert_eq!(v["startTime"], serde_json::json!(1_700_000_000_100u64));
        assert_eq!(v["endTime"], serde_json::json!(1_700_000_003_900u64));
        assert_eq!(v["isLiveLog"], serde_json::json!(true));
        assert_eq!(v["isRealTime"], serde_json::json!(true));
        assert_eq!(v["inProgressEventCount"], serde_json::json!(42));
        assert_eq!(v["mythic"], serde_json::json!(0));
    }

    #[test]
    fn finished_segment_parameters_are_not_realtime_or_in_progress() {
        let params = segment_parameters_json(3, 1_700_000_000_000, 1_700_000_010_000);
        let v: serde_json::Value =
            serde_json::from_str(&params).expect("finished segment parameters must be valid JSON");

        assert_eq!(v["segmentId"], serde_json::json!(3));
        assert_eq!(v["startTime"], serde_json::json!(1_700_000_000_000u64));
        assert_eq!(v["endTime"], serde_json::json!(1_700_000_010_000u64));
        assert_eq!(v["isLiveLog"], serde_json::json!(false));
        assert_eq!(v["isRealTime"], serde_json::json!(false));
        assert_eq!(v["inProgressEventCount"], serde_json::json!(0));
        assert_eq!(v["mythic"], serde_json::json!(0));
    }

    #[test]
    fn extract_next_segment_id_reads_numeric_value_including_terminal_zero() {
        // A numeric value (incl the explicit terminal 0) parses successfully.
        assert_eq!(
            extract_next_segment_id(br#"{"nextSegmentId":5}"#).unwrap(),
            5
        );
        assert_eq!(
            extract_next_segment_id(br#"{"nextSegmentId":0}"#).unwrap(),
            0
        );
    }

    #[test]
    fn extract_next_segment_id_rejects_malformed_responses() {
        // Missing field, non-numeric, empty body, or non-JSON must be a HARD
        // error — never a silent "done" that finalizes an incomplete report.
        for bad in [
            &br#"{"other":1}"#[..],
            &br#"{"nextSegmentId":"x"}"#[..],
            &b""[..],
            &b"not json"[..],
            &b"<html>error</html>"[..],
        ] {
            assert!(
                matches!(
                    extract_next_segment_id(bad),
                    Err(UploadError::Server { .. })
                ),
                "malformed body must be a Server error, not a terminal: {:?}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    // ── Cancel-aware live send (L4) ──────────────────────────────────────────

    /// A Stop set while a live POST is in flight must return `Cancelled` within a
    /// couple of poll intervals (~250ms each), NOT wait for the 120s request
    /// timeout. We exercise the exact wait loop with a worker that never completes.
    #[test]
    fn live_wait_returns_cancelled_fast_when_no_result_arrives() {
        let (_tx, rx) = std::sync::mpsc::channel::<Result<Vec<u8>, UploadError>>();
        let cancel = Arc::new(AtomicBool::new(false));
        // Trip cancel from another thread shortly after we start waiting.
        let c = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(120));
            c.store(true, Ordering::SeqCst);
        });
        let start = std::time::Instant::now();
        let r = wait_for_send_or_cancel(rx, &cancel);
        let elapsed = start.elapsed();
        assert!(matches!(r, Err(UploadError::Cancelled)), "{r:?}");
        // Bounded by the cancel set (~120ms) + one poll interval (~250ms), with
        // generous slack for CI scheduling — the point is it's NOT the 120s timeout.
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "cancel must return promptly, took {elapsed:?}"
        );
        // `_tx` is held to keep the channel open (so it's a Timeout-then-cancel
        // path, not a Disconnected path).
        drop(_tx);
    }

    /// A completed worker result is returned as-is (cancel never trips).
    #[test]
    fn live_wait_returns_worker_result_when_it_completes() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Vec<u8>, UploadError>>(1);
        tx.send(Ok(b"ok-body".to_vec())).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let r = wait_for_send_or_cancel(rx, &cancel).unwrap();
        assert_eq!(r, b"ok-body");
    }

    /// A worker that drops its sender without producing a result (a panic) surfaces
    /// as a transport error, never a hang.
    #[test]
    fn live_wait_maps_dropped_worker_to_transport_error() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Vec<u8>, UploadError>>(1);
        drop(tx); // worker "panicked" before sending
        let cancel = Arc::new(AtomicBool::new(false));
        let r = wait_for_send_or_cancel(rx, &cancel);
        assert!(matches!(r, Err(UploadError::Transport(_))), "{r:?}");
    }

    /// CREATE wait: a result that completes normally is returned (cancel never trips).
    #[test]
    fn create_wait_returns_result_when_it_completes() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Vec<u8>, UploadError>>(1);
        tx.send(Ok(b"code".to_vec())).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let r = wait_for_create_or_cancel(rx, &cancel, std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(r, b"code");
    }

    /// CREATE wait, THE FIX: a Stop racing create must NOT abandon the report. With cancel
    /// already set, a code landing within the grace window is captured (returned `Ok`), so
    /// the caller can terminate it — not surfaced as `Cancelled` (which would orphan it).
    #[test]
    fn create_wait_captures_a_late_landing_report_on_cancel() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Vec<u8>, UploadError>>(1);
        let cancel = Arc::new(AtomicBool::new(true)); // stopped during create
        std::thread::spawn(move || {
            // Land AFTER a phase-1 poll timeout, so phase-2 grace is what captures it.
            std::thread::sleep(std::time::Duration::from_millis(400));
            let _ = tx.send(Ok(b"report-code".to_vec()));
        });
        let r = wait_for_create_or_cancel(rx, &cancel, std::time::Duration::from_secs(5));
        assert_eq!(
            r.unwrap(),
            b"report-code",
            "a report landing within grace must be captured, not abandoned"
        );
    }

    /// CREATE wait: if no code lands within the grace (wedged create), give up as
    /// `Cancelled` — bounded by the grace, never the 120s request timeout.
    #[test]
    fn create_wait_gives_up_after_grace_with_no_report() {
        let (_tx, rx) = std::sync::mpsc::channel::<Result<Vec<u8>, UploadError>>();
        let cancel = Arc::new(AtomicBool::new(true));
        let start = std::time::Instant::now();
        let r = wait_for_create_or_cancel(rx, &cancel, std::time::Duration::from_millis(300));
        assert!(matches!(r, Err(UploadError::Cancelled)), "{r:?}");
        assert!(
            start.elapsed() < std::time::Duration::from_secs(3),
            "bounded by the grace, took {:?}",
            start.elapsed()
        );
        drop(_tx);
    }

    /// An already-cancelled send short-circuits before spawning any worker.
    #[test]
    fn live_send_cancellable_short_circuits_when_already_cancelled() {
        let sess: Arc<dyn SessionProvider> = Arc::new(FakeSession {
            invalidated: std::sync::Mutex::new(false),
        });
        let sender = LiveSender::new(sess);
        let cancel = Arc::new(AtomicBool::new(true)); // already stopped
        let r = sender.send_cancellable(
            "https://example.invalid/x",
            OwnedLiveRequest::Terminate,
            &cancel,
        );
        assert!(matches!(r, Err(UploadError::Cancelled)), "{r:?}");
    }

    // ── C3: status classification ────────────────────────────────────────────

    #[test]
    fn classify_status_maps_auth_and_login_redirects() {
        use reqwest::header::{HeaderMap, HeaderValue, LOCATION};
        use reqwest::StatusCode;

        let empty = HeaderMap::new();
        // 401 / 419 → AuthRejected (unchanged behavior).
        assert!(matches!(
            classify_status(StatusCode::UNAUTHORIZED, &empty),
            StatusClass::AuthRejected
        ));
        assert!(matches!(
            classify_status(StatusCode::from_u16(419).unwrap(), &empty),
            StatusClass::AuthRejected
        ));

        // THE C3 FIX: a 302 → /login (an expired-session bounce) is an auth rejection,
        // not a fatal server error.
        let mut login = HeaderMap::new();
        login.insert(
            LOCATION,
            HeaderValue::from_static("https://www.esologs.com/login"),
        );
        assert!(matches!(
            classify_status(StatusCode::FOUND, &login),
            StatusClass::AuthRejected
        ));

        // A 3xx with no readable Location → conservatively AuthRejected.
        assert!(matches!(
            classify_status(StatusCode::FOUND, &empty),
            StatusClass::AuthRejected
        ));

        // 2xx / 5xx defer to a body read.
        assert!(matches!(
            classify_status(StatusCode::OK, &empty),
            StatusClass::ReadBody
        ));
        assert!(matches!(
            classify_status(StatusCode::INTERNAL_SERVER_ERROR, &empty),
            StatusClass::ReadBody
        ));
        // A redirect to a non-login page is treated as a normal (anomalous) response.
        let mut other = HeaderMap::new();
        other.insert(
            LOCATION,
            HeaderValue::from_static("https://www.esologs.com/reports/abc"),
        );
        assert!(matches!(
            classify_status(StatusCode::SEE_OTHER, &other),
            StatusClass::ReadBody
        ));
    }

    // ── C4: timeout scaling ──────────────────────────────────────────────────

    #[test]
    fn request_timeout_scales_with_payload_and_is_capped() {
        use std::time::Duration;
        // Control-plane / small bodies → flat 120s floor.
        assert_eq!(request_timeout_for(0), Duration::from_secs(120));
        assert_eq!(request_timeout_for(1024), Duration::from_secs(120));
        // A 10 MiB segment scales above the floor (10 MiB / 64 KiB/s = 160s extra).
        assert_eq!(
            request_timeout_for(10 * 1024 * 1024),
            Duration::from_secs(280)
        );
        // A pathological payload is capped at 15 minutes, never unbounded.
        assert_eq!(
            request_timeout_for(usize::MAX),
            Duration::from_secs(15 * 60)
        );
        // Monotonic: a bigger payload never shrinks the timeout.
        assert!(request_timeout_for(100 * 1024 * 1024) >= request_timeout_for(10 * 1024 * 1024));
    }

    // ── C2: one-shot crash-recovery breadcrumb ───────────────────────────────

    /// A fake [`super::super::live::OrphanSink`] recording the ordered breadcrumb calls,
    /// so the one-shot lifecycle's record/note/clear discipline is assertable without a
    /// server (mirrors the pure list tests in `orphans.rs`).
    #[derive(Default)]
    struct RecordingSink {
        calls: std::sync::Mutex<Vec<String>>,
    }
    impl crate::uploader::native::live::OrphanSink for RecordingSink {
        fn record_open(&self, code: &str, segment_id: u64) {
            self.calls
                .lock()
                .unwrap()
                .push(format!("open:{code}:{segment_id}"));
        }
        fn note_segment(&self, code: &str, segment_id: u64) {
            self.calls
                .lock()
                .unwrap()
                .push(format!("note:{code}:{segment_id}"));
        }
        fn clear(&self, code: &str) {
            self.calls.lock().unwrap().push(format!("clear:{code}"));
        }
    }

    #[test]
    fn rejected_inputs_write_no_breadcrumb_before_a_report_exists() {
        // Empty input, count mismatch, and already-cancelled all return BEFORE
        // create-report, so no breadcrumb may be written — record_open must never fire
        // for a report that never existed (else next-launch recovery chases a ghost).
        let sess = FakeSession {
            invalidated: std::sync::Mutex::new(false),
        };
        let opts = UploadOptions::default();

        // empty input
        let sink = RecordingSink::default();
        let up = NativeUpload::new(&sess, &opts, Arc::new(AtomicBool::new(false)));
        assert!(up
            .upload_finished_with_orphans(&[], &[], &no_progress, &sink)
            .is_err());
        assert!(
            sink.calls.lock().unwrap().is_empty(),
            "empty input must not record a breadcrumb"
        );

        // count mismatch
        let sink = RecordingSink::default();
        let up = NativeUpload::new(&sess, &opts, Arc::new(AtomicBool::new(false)));
        let segs = vec![
            Segment {
                bytes: vec![1],
                start_time: 0,
                end_time: 0,
            },
            Segment {
                bytes: vec![2],
                start_time: 0,
                end_time: 0,
            },
        ];
        let masters = vec![MasterTableBytes { bytes: vec![] }];
        assert!(up
            .upload_finished_with_orphans(&segs, &masters, &no_progress, &sink)
            .is_err());
        assert!(
            sink.calls.lock().unwrap().is_empty(),
            "count mismatch must not record a breadcrumb"
        );

        // already cancelled before start
        let sink = RecordingSink::default();
        let up = NativeUpload::new(&sess, &opts, Arc::new(AtomicBool::new(true)));
        let segs = vec![Segment {
            bytes: vec![1, 2, 3],
            start_time: 0,
            end_time: 0,
        }];
        let masters = vec![MasterTableBytes { bytes: vec![] }];
        assert!(matches!(
            up.upload_finished_with_orphans(&segs, &masters, &no_progress, &sink),
            Err(UploadError::Cancelled)
        ));
        assert!(
            sink.calls.lock().unwrap().is_empty(),
            "a pre-create cancel must not record a breadcrumb"
        );
    }

    #[test]
    fn terminate_confirms_closed_matches_live_settle_rule() {
        // Confirmed close → the breadcrumb may be dropped (clear-only-on-confirmed-close).
        assert!(terminate_confirms_closed(&Ok(())));
        assert!(terminate_confirms_closed(&Err(UploadError::Server {
            status: 404,
            detail: String::new()
        })));
        assert!(terminate_confirms_closed(&Err(UploadError::Server {
            status: 410,
            detail: String::new()
        })));
        // Every transient → KEEP the breadcrumb for next-launch recovery.
        assert!(!terminate_confirms_closed(&Err(UploadError::Transport(
            "net".into()
        ))));
        assert!(!terminate_confirms_closed(&Err(UploadError::Server {
            status: 500,
            detail: String::new()
        })));
        assert!(!terminate_confirms_closed(&Err(UploadError::Session(
            SessionError::Expired
        ))));
        assert!(!terminate_confirms_closed(&Err(UploadError::Cancelled)));
    }
}
