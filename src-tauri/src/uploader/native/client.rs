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
use std::sync::Arc;

use super::session::{Session, SessionError, SessionProvider};
use crate::uploader::types::UploadOptions;

/// Base for the report lifecycle endpoints. A fact about the service.
const DESKTOP_CLIENT_BASE: &str = "https://www.esologs.com/desktop-client";

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
}

/// The serialized master table for a report.
#[derive(Debug, Clone)]
pub struct MasterTableBytes {
    pub bytes: Vec<u8>,
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
    pub fn upload_finished(
        &self,
        segments: &[Segment],
        masters: &[MasterTableBytes],
        progress: &ProgressFn<'_>,
    ) -> Result<ReportCode, UploadError> {
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
        // work — there is no report to terminate yet.
        if self.is_cancelled() {
            return Err(UploadError::Cancelled);
        }

        // 1. Establish (or fail) the session up front — fail fast before any work.
        let _session = self.session.session()?;

        // 2. create-report
        let code = self.create_report()?;

        // 3. per-segment: master table → add segment → follow nextSegmentId.
        let total = segments.len();
        // segmentId starts at 1; the server returns the next id to use.
        let mut segment_id: u64 = 1;
        for (i, (seg, master)) in segments.iter().zip(masters.iter()).enumerate() {
            if self.is_cancelled() {
                let _ = self.terminate_report(&code);
                return Err(UploadError::Cancelled);
            }
            // a. master table for this segment id, then b. the fights segment.
            self.set_master_table(&code, segment_id, master)?;
            let next = self.add_segment(&code, segment_id, seg)?;
            progress(UploadProgress {
                segments_done: i + 1,
                segments_total: total,
            });
            // The server sequences ids; a non-positive next id means "done".
            if next == 0 {
                break;
            }
            segment_id = next;
        }

        // 4. terminate.
        self.terminate_report(&code)?;
        Ok(code)
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
            },
        )?;
        // Response is JSON `{ "nextSegmentId": <n> }`.
        Ok(extract_next_segment_id(&body))
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
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| format!("HTTP client error: {e}"))?;

        let req = client
            .post(url)
            .header(reqwest::header::COOKIE, session.cookie_header())
            .header(reqwest::header::ACCEPT, "application/json");

        let req = match kind {
            RequestKind::CreateReport => req
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(self.create_report_body()),
            RequestKind::Terminate => req, // no body
            RequestKind::AddSegment { segment_id, bytes } => {
                let form = reqwest::blocking::multipart::Form::new()
                    .text("parameters", segment_parameters_json(*segment_id))
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
        if status == reqwest::StatusCode::UNAUTHORIZED || status.as_u16() == 419
        // 419 = Laravel CSRF/session mismatch.
        {
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

    /// Build the `create-report` JSON body. Ten fields, matching the confirmed
    /// live request: a fresh report is created with `startTime == endTime` at
    /// creation time (the server backfills the real range from the segments).
    fn create_report_body(&self) -> String {
        let opts = self.opts;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let guild = opts
            .guild_id
            .as_deref()
            .map(|g| format!("\"{}\"", escape_json(g)))
            .unwrap_or_else(|| "null".to_string());
        let desc = opts
            .description
            .as_deref()
            .map(|d| format!("\"{}\"", escape_json(d)))
            .unwrap_or_default();
        format!(
            "{{\"clientVersion\":\"{cv}\",\"parserVersion\":{pv},\
             \"startTime\":{ts},\"endTime\":{ts},\"guildId\":{guild},\
             \"fileName\":\"{file}\",\"serverOrRegion\":{region},\
             \"visibility\":{vis},\"reportTagId\":null,\"description\":\"{desc}\"}}",
            cv = super::format::CLIENT_VERSION,
            pv = super::format::FORMAT_VERSION,
            ts = now_ms,
            guild = guild,
            file = "log.txt",
            region = opts.region,
            vis = opts.visibility.as_report_visibility_id(),
            desc = desc,
        )
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

/// Which lifecycle call a `send` is performing — selects the envelope shape.
/// The multipart calls carry the segment id (for the form/parameters) and the
/// already-serialized, ZIP-compressed segment/master bytes.
enum RequestKind<'a> {
    CreateReport,
    AddSegment { segment_id: u64, bytes: &'a [u8] },
    MasterTable { segment_id: u64, bytes: &'a [u8] },
    Terminate,
}

/// The `parameters` JSON for `add-report-segment` (a manual, finished upload:
/// not live, not real-time, no in-progress events). `startTime`/`endTime` are 0
/// here — the server derives the real range from the segment contents.
fn segment_parameters_json(segment_id: u64) -> String {
    format!(
        "{{\"startTime\":0,\"endTime\":0,\"mythic\":0,\"isLiveLog\":false,\
         \"isRealTime\":false,\"inProgressEventCount\":0,\"segmentId\":{segment_id}}}"
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

/// Minimal JSON string escaping for the few free-text fields (guild id,
/// description) interpolated into the hand-built `create-report` body.
fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
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
/// sequences segment ids; a missing/non-positive value means "no further
/// segments" (treated as 0 by the lifecycle loop). Lenient by design — an
/// unparseable body here should not abort an otherwise-successful upload.
fn extract_next_segment_id(body: &[u8]) -> u64 {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("nextSegmentId").and_then(|n| n.as_u64()))
        .unwrap_or(0)
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
        let segs = vec![Segment { bytes: vec![1] }, Segment { bytes: vec![2] }];
        let masters = vec![MasterTableBytes { bytes: vec![] }]; // only 1
        let err = up
            .upload_finished(&segs, &masters, &no_progress)
            .unwrap_err();
        assert!(matches!(err, UploadError::Server { .. }));
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
    fn extract_next_segment_id_reads_value_or_defaults_zero() {
        assert_eq!(extract_next_segment_id(br#"{"nextSegmentId":5}"#), 5);
        // Missing field, empty body, or non-JSON → 0 (no further segments).
        assert_eq!(extract_next_segment_id(br#"{"other":1}"#), 0);
        assert_eq!(extract_next_segment_id(b""), 0);
        assert_eq!(extract_next_segment_id(b"not json"), 0);
    }
}
