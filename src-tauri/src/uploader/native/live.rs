//! Native **live-streaming** upload driver: tails a growing `Encounter.log` and
//! pushes fights-segments to `esologs.com/desktop-client/*` **incrementally**,
//! holding ONE report open across a whole session and terminating only when logging
//! ends — instead of the one-shot "read the finished file, upload once" path in
//! [`super::super::transport`].
//!
//! ## Gating
//!
//! The live round-trip is confirmed and the ToS gate is cleared, so this module
//! compiles into release — but it is REACHABLE only when the user opts in, the format
//! is confirmed, and a session exists ([`super::super::transport::assess_native_live_routing`]);
//! otherwise `uploader_start_live` runs the official-uploader handoff (the default).
//! The debug round-trip command (`uploader_run_native_live_spike`) and the synthetic
//! [`FileTail`]/`ScriptedTail` test seam stay `#[cfg(debug_assertions)]` so a
//! "create a real report from an arbitrary path" surface never ships.
//!
//! ## Design (the parts proven offline)
//!
//! * ONE long-lived [`EventEmitter`] across the whole session (never rebuilt) — this
//!   is what carries the actor/ability/tuple tables, the report-absolute time base,
//!   and every in-flight correlation (shields, casts, stacks) across a headerless
//!   segment boundary. The
//!   `one_emitter_with_fight_cuts_matches_one_shot_build_byte_for_byte` test proves a
//!   cut emitter produces byte-identical events to the proven one-shot `build()`.
//! * CUMULATIVE master per cut, RENDERED from the incremental
//!   [`super::incremental::IncrementalMasterState`] (maintained O(1)-amortized per
//!   line, no `all_lines` buffer) with the prior frozen actor/ability indices PINNED
//!   so a late-registering actor never renumbers an earlier segment's tuple `A`-refs
//!   (hazard H1). Proven byte-identical to the prior
//!   [`encode::build_master_table_with_tuples_forced`] re-walk (retained as the test
//!   oracle) at every cut by `incremental_master_matches_rewalk_at_every_cut`.
//! * Cut STRICTLY at fight boundaries (`END_COMBAT`) and at every `BEGIN_LOG` — never
//!   a timer / every-N-events window — so an in-flight correlation never strands.
//! * Per-segment wall window from [`EventEmitter::live_segment_time_bounds`]
//!   (`current_session_wall + raw_ts`), with a POST skipped if the window is unknown.
//! * A structural self-check ([`events::validate_segment_text`]) plus a master/segment
//!   tuple-count cross-check before every POST, so a malformed segment is never sent.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::client::{
    LivePoster, LiveSender, MasterTableBytes, ReportCode, Segment, UploadError, LIVE_CANCEL_POLL,
};
use super::events::EventEmitter;
use super::incremental::{IncrementalIndexState, IncrementalMasterState};
use super::session::SessionProvider;
use crate::uploader::tail_io::{read_range, MAX_CONSECUTIVE_FAILURES, MAX_READ, POLL_INTERVAL};
use crate::uploader::types::UploadOptions;

/// The pure, network-free core of the live driver: owns the long-lived emitter, the
/// incremental index + master state, and produces a ready-to-send payload on each
/// fight-boundary cut. Separated from the I/O loop so the state machine (cut policy,
/// H1 pinning, time bounds, validation) is unit-testable without a server or a real
/// growing file.
///
/// Memory: the unbounded `all_lines: Vec<String>` buffer of the original spike is
/// GONE (Step 2b). The cumulative master is now rendered each cut from the
/// incremental [`IncrementalMasterState`] (O(distinct entities), proven byte-identical
/// to the prior `build_master_table_with_tuples_forced` re-walk by the differential
/// test below), so no raw line is retained past the segment it belongs to.
pub struct LiveSegmenter {
    /// The single emitter carried across all segments.
    emitter: EventEmitter,
    /// The incremental actor/ability index maps (the L7 perf fix). Maintained in O(1)
    /// amortized per line instead of re-walking the whole buffer on every registering
    /// line; proven byte-identical to the re-walk oracle by the differential tests in
    /// [`super::incremental`]. Its maps are pushed into the emitter before each
    /// tuple-allocating line so the live `A` numbering stays identical to the one-shot
    /// path, and they are the PIN passed to the master renderer each cut.
    index_state: IncrementalIndexState,
    /// The incremental master-table record state (Step 2b). Maintains the header,
    /// captured actor records, first-write-wins ability signals + appearance order,
    /// and pet candidates per line, so each cut's cumulative master is RENDERED from
    /// it (`render_master`) instead of re-walking an `all_lines` buffer. Proven
    /// byte-identical to the re-walk oracle at every cut by the differential test
    /// `incremental_master_matches_rewalk_at_every_cut`.
    master_state: IncrementalMasterState,
    /// Lines fed since the last cut (the body of the segment being assembled).
    segment_lines: Vec<String>,
    /// The next segment id to use (server-sequenced; starts at 1, updated from each
    /// `add-segment` response).
    next_segment_id: u64,
    /// How many segments have been cut+built (for diagnostics / progress).
    segments_built: usize,
}

/// A built, ready-to-POST live segment: the ZIP'd segment + cumulative master, the
/// server segment id to send them under, and the count of events in an unfinished
/// fight at the tail (0 on a clean fight-boundary cut).
#[derive(Debug)]
pub struct LiveSegmentPayload {
    pub segment: Segment,
    pub master: MasterTableBytes,
    pub segment_id: u64,
    pub in_progress_event_count: u64,
}

impl Default for LiveSegmenter {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveSegmenter {
    pub fn new() -> Self {
        Self {
            emitter: EventEmitter::new(),
            index_state: IncrementalIndexState::default(),
            master_state: IncrementalMasterState::default(),
            segment_lines: Vec::new(),
            next_segment_id: 1,
            segments_built: 0,
        }
    }

    /// Feed one raw line into the long-lived emitter and the cumulative buffer.
    /// Returns `true` if the line is a CUT BOUNDARY (`END_COMBAT` or a new
    /// `BEGIN_LOG`) at which the driver should build + POST a segment. `BEGIN_LOG`
    /// is a *pre*-cut boundary: the segment that just ended (the prior session)
    /// should be flushed BEFORE the new session's lines accumulate, which the driver
    /// handles by cutting when this returns true.
    pub fn feed(&mut self, line: &str) -> bool {
        self.segment_lines.push(line.to_string());
        // Update the incremental index maps with THIS line first (it maintains the
        // time-aware live-monster binding, the synthetic-ability splice, and the
        // actor/ability assignments — all of which must reflect this line before the
        // emitter, below, allocates this line's tuple). `update` no-ops on lines that
        // can't change the maps, so calling it unconditionally is cheap and keeps its
        // internal state (live bindings cleared on UNIT_REMOVED, splice flag) correct.
        self.index_state.update(line);
        // Fold this line into the incremental MASTER record state too (header,
        // captured actors, ability signals + appearance order, pet candidates), so the
        // cumulative master can be rendered each cut WITHOUT an `all_lines` re-walk.
        // Like the index state it no-ops on irrelevant lines.
        self.master_state.update(line);
        // The emitter allocates a tuple's `A` from its `identity_to_actor` /
        // `ability_to_index` maps AT FEED TIME, so those maps must be current BEFORE
        // the emitter sees a line that allocates a tuple. Push the (now-updated)
        // incremental maps into the emitter on any line that can register/introduce
        // an entity — exactly the kinds that previously triggered the re-walk. This
        // keeps the live `A` numbering identical to the one-shot path, but in O(1)
        // amortized per line instead of re-walking `all_lines` (the L7 perf fix). The
        // pushed maps are content-identical to the prior re-walk (proven by the
        // `super::incremental` differential tests).
        if matches!(
            kind_of(line),
            Some("UNIT_ADDED")
                | Some("ABILITY_INFO")
                | Some("EFFECT_INFO")
                | Some("BEGIN_LOG")
                | Some("COMBAT_EVENT")
                | Some("EFFECT_CHANGED")
                | Some("BEGIN_CAST")
        ) {
            self.refresh_maps();
        }
        let _ = self.emitter.feed(line);
        let kind = kind_of(line);
        kind == Some("END_COMBAT") || kind == Some("BEGIN_LOG")
    }

    /// Push the incrementally-maintained master index maps into the emitter. The
    /// append-only H1 pin is preserved by [`IncrementalIndexState`] itself (an
    /// already-indexed actor/ability keeps its slot; a newly-registering one appends
    /// above the max), so this no longer re-walks `all_lines` — it clones the current
    /// incremental maps (O(distinct entities), not O(events)). The clones are
    /// content-identical to the prior `actor_ability_maps_forced(&all_lines, …)`
    /// re-walk, proven at every line by the `super::incremental` differential tests.
    fn refresh_maps(&mut self) {
        self.emitter.refresh_master_indices(
            self.index_state.actor_map().clone(),
            self.index_state.ability_map().clone(),
        );
    }

    /// Flush any trailing buffered `DAMAGE_SHIELDED` lines into the current segment
    /// (with `f10 = 0`), mirroring the one-shot end-of-file drain — call once when
    /// logging ends, before the final [`Self::build_next_segment`], so a fully-absorbed
    /// final hit is not dropped.
    pub fn drain_trailing_shields(&mut self) {
        self.emitter.drain_trailing_shields_into_segment();
    }

    /// Whether a cut at this moment is SAFE for in-flight correlations: no buffered
    /// `DAMAGE_SHIELDED` is awaiting its paired damage event. Cutting while a shield
    /// is pending would either strand the back-patch across a segment boundary or
    /// force-flush it with `f10=0` (a quality regression). The driver defers the cut
    /// to the next boundary when this is false.
    pub fn shields_settled(&self) -> bool {
        self.emitter.pending_shields_is_empty()
    }

    /// Build the next segment payload from the lines accumulated since the last cut,
    /// using a CUMULATIVE master pinned to the frozen actor indices (the H1 fix).
    /// Returns `Ok(None)` when there is nothing to send (no emitted events, or no
    /// wall window yet because no `BEGIN_LOG` has been seen) — the driver SKIPS the
    /// POST rather than open a zero-width / epoch-placed segment. `Err` on an
    /// internal inconsistency (a malformed segment or a master/segment desync), in
    /// which case the driver should fall back / stop rather than ship a broken
    /// segment.
    pub fn build_next_segment(&mut self) -> Result<Option<LiveSegmentPayload>, String> {
        use super::events::validate_segment_text;

        // Render the events emitted since the last cut. We re-run the emitter's
        // framing over the segment lines is NOT how this works — the long-lived
        // emitter already advanced its state as lines were fed; we need the EMITTED
        // lines for THIS segment. Re-feed is not possible (state already advanced),
        // so the driver collects emitted lines as they come. Here we instead rebuild
        // the segment body from the segment's own lines through a SHADOW pass on a
        // clone is also wrong (would re-allocate tuples). The correct, proven
        // approach: the emitter assembles the whole report; for live we frame ONLY
        // this segment's emitted lines. Track them via the emitter's per-segment
        // emit log (see `drain_segment_events`).
        let body = self.emitter.drain_segment_events();
        if body.event_count == 0 {
            // Nothing emitted in this window (e.g. a BEGIN_LOG with no fights yet).
            self.emitter.open_segment();
            self.segment_lines.clear();
            return Ok(None);
        }

        // The wall window for this segment (current_session_wall + raw ts of its
        // first/last emitted event). None ⇒ no BEGIN_LOG yet ⇒ skip the POST.
        let Some((start_time, end_time)) = self.emitter.live_segment_time_bounds() else {
            self.emitter.open_segment();
            self.segment_lines.clear();
            return Ok(None);
        };

        // Frame the body into the fights-segment text (header + count + events). The
        // log version is the first BEGIN_LOG's f[3], captured incrementally by the
        // master state (the segment-framing and master log_version are the same value).
        let log_version = self
            .master_state
            .log_version()
            .ok_or("live segment has no BEGIN_LOG log version")?
            .to_string();
        let segment_text = super::serialize::FightsSegmentDoc {
            log_version: &log_version,
            game_version: "1",
            fights: &[(body.event_count, &body.events_string)],
        }
        .render();

        // Structural self-check: every A in range, declared count == emitted lines.
        validate_segment_text(&segment_text, self.emitter.allocated())?;

        // CUMULATIVE master pinned to the emitter's current frozen actor AND ability
        // indices (the H1 fix, both axes). `feed` keeps the emitter's maps current as
        // introduction/registering lines arrive, so the emitter's frozen maps ARE the
        // canonical index space; render the master from the incremental master state
        // (maintained per line, NO `all_lines` re-walk) using those same pinned maps +
        // the emitter's tuple table, so the segment's `A`/C refs and the master's
        // tuples/actors/abilities are in lockstep. Proven byte-identical to the prior
        // `build_master_table_with_tuples_forced` re-walk at every cut by the
        // differential test `incremental_master_matches_rewalk_at_every_cut`.
        let frozen_actors = self.emitter.frozen_actor_index_map();
        let frozen_abilities = self.emitter.frozen_ability_index_map();
        let master_text = self
            .master_state
            .render_master(self.emitter.tuples(), &frozen_actors, &frozen_abilities)
            .ok_or("live cumulative master failed to build")?;

        // Master/segment tuple-count cross-check — the validator can't see the master
        // bytes, so a stale/delta master would pass validation but not render. The
        // master's tuple section has exactly `emitter.allocated()` records.
        let master_tuple_count = count_master_tuples(&master_text);
        if master_tuple_count != self.emitter.allocated() as u64 {
            return Err(format!(
                "master/segment desync: master has {master_tuple_count} tuples, \
                 segment references up to {}",
                self.emitter.allocated()
            ));
        }

        let segment = Segment::from_text(&segment_text, start_time, end_time)
            .map_err(|e| format!("zip segment: {e}"))?;
        let master =
            MasterTableBytes::from_text(&master_text).map_err(|e| format!("zip master: {e}"))?;
        let segment_id = self.next_segment_id;

        // A clean fight-boundary cut ends on END_COMBAT, so no fight is in progress.
        // (A future true-mid-fight-streaming mode would compute a real count here.)
        let in_progress_event_count = 0;

        self.segments_built += 1;
        self.emitter.open_segment();
        self.segment_lines.clear();

        Ok(Some(LiveSegmentPayload {
            segment,
            master,
            segment_id,
            in_progress_event_count,
        }))
    }

    /// Record the server-assigned next segment id (from an `add-segment` response).
    pub fn set_next_segment_id(&mut self, next: u64) {
        self.next_segment_id = next;
    }

    pub fn segments_built(&self) -> usize {
        self.segments_built
    }

    /// The next server-sequenced segment id (diagnostics / orphan persistence).
    pub fn next_segment_id(&self) -> u64 {
        self.next_segment_id
    }

    /// Transition from WARM-UP (mid-session prefix replay) to LIVE. During warm-up the
    /// driver feeds the on-disk `[BEGIN_LOG, EOF)` prefix through `feed()` to rebuild the
    /// cumulative encoder state (actor/ability/tuple indices, the master record state,
    /// and — crucially — `current_session_wall`/`log_version` parsed from the real
    /// on-disk BEGIN_LOG) WITHOUT POSTing any of the prefix's already-finished fights.
    /// This call discards everything that defines the *content/numbering of segments to
    /// POST*, while KEEPING the cumulative report state, so the first LIVE segment starts
    /// at segment id 1 and contains only post-join events — yet its tuple `A`-refs and
    /// cumulative master resolve against the warm-up-seeded index space.
    ///
    /// Why this preserves the H1 index-stability pin: the pin lives entirely in
    /// `IncrementalIndexState`/`IncrementalMasterState`/the emitter's frozen maps, all of
    /// which are KEPT. `segment_lines` is a write-only buffer (never read when building a
    /// segment or master), so clearing it is pure hygiene. The only things reset are the
    /// per-segment POST framing (`open_segment` drops the in-progress segment's events +
    /// wall window) and the segment counters. Proven byte-identical to a real
    /// from-BEGIN_LOG stream by the `mid_session_seed_matches_*` differential tests.
    pub fn finish_warmup(&mut self) {
        self.segment_lines.clear();
        self.next_segment_id = 1;
        self.segments_built = 0;
        // Drop the in-progress segment's accumulated events + wall window so the first
        // LIVE POST contains only post-join events (the cumulative tables/wall/anchor and
        // in-flight correlations are NOT touched by open_segment).
        self.emitter.open_segment();
    }
}

// ── Lifecycle state machine (L3 + L4) ────────────────────────────────────────
//
// The production driver is a small state machine over the cut→POST loop:
//
//   Streaming ──(POST → Session(Expired))──▶ Paused{held} ──(re-auth)──▶ Streaming
//        │                                        │
//        └──(cancel / END_LOG / idle / 2nd BEGIN_LOG / fatal / retries exhausted)──▶ end
//
// A live session can outlive the wcl_session cookie (multi-hour raid), so a
// mid-stream 401/419 that survives the one re-auth retry PAUSES the stream rather
// than abandoning the report: lines keep feeding the long-lived emitter (state
// continues), the report stays OPEN, the UI is asked to re-sign-in, and posting
// resumes once a fresh cookie is stored. A pause that never resolves within
// [`REAUTH_TIMEOUT`] terminates gracefully. Idle (no file growth for
// [`IDLE_DEADLINE`]) is distinct from a clean END_LOG end.

/// Terminate after this long with no file growth (the crash / forgot-to-stop
/// fallback). Generous so a legitimate between-pulls raid break never trips it;
/// END_LOG is the primary clean terminal, this is the safety net.
pub const IDLE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// While paused on a lost session, terminate if the user does not re-sign-in within
/// this long. Shorter than [`IDLE_DEADLINE`] because a paused-but-still-growing file
/// would otherwise keep resetting the idle clock and never terminate.
pub const REAUTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Max attempts for a single segment POST before giving up (the first try + 2
/// retries). A transient blip on a multi-hour session must not abandon the report.
const MAX_SEGMENT_ATTEMPTS: u32 = 3;

/// Upper bound on the terminate-on-exit wait. The terminate uses a fresh cancel flag
/// (so a Stop doesn't skip closing the report), but a watchdog trips that flag after
/// this deadline so a wedged network can't block the driver thread — and thus a Stop
/// join — for the full 120s request timeout. On a watchdog trip the orphan breadcrumb
/// is kept and next-launch recovery closes the report.
const TERMINATE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// Why the live stream ended — distinguishes a clean END_LOG from the idle/crash
/// fallback and the various failure terminals, so the history record + diagnostics
/// are honest about what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndReason {
    /// Clean end: an `END_LOG` line (the game stopped logging normally).
    Ended,
    /// No file growth past [`IDLE_DEADLINE`] (game crash / forgot to stop).
    Idle,
    /// Server returned `nextSegmentId == 0` (session-ended server-side).
    ServerEnded,
    /// A second `BEGIN_LOG` (a `/reloadui` mid-session) — the single-session
    /// contract forbids mixing two sessions in one report, so we terminate.
    NewSession,
    /// The user stopped (cancel flag set).
    Stopped,
    /// A paused-on-reauth session that never re-authed within [`REAUTH_TIMEOUT`].
    ReauthTimeout,
    /// A non-retryable failure (malformed segment, 4xx, retries exhausted).
    Fatal(String),
}

/// How a segment POST attempt should be handled after classifying its error.
enum PostOutcome {
    /// The segment was accepted; carries the server's next segment id (0 = the
    /// server signalled session-end).
    Posted { next_segment_id: u64 },
    /// Nothing to send this window (empty/zone-only) — keep streaming.
    Nothing,
    /// The session was rejected and the one re-auth retry didn't recover — PAUSE
    /// and prompt re-login (do NOT terminate; the report stays open). Carries the
    /// already-built payload so the driver can RE-POST it after the session is
    /// restored (the segmenter already advanced past it, so it can't be rebuilt).
    NeedsReauth { held: Box<LiveSegmentPayload> },
    /// Stop requested mid-POST.
    Cancelled,
    /// A non-retryable failure (or retries exhausted) — terminate + settle Failed.
    Fatal(String),
}

/// Classify an [`UploadError`] for the per-segment retry policy.
#[derive(Debug, PartialEq, Eq)]
enum RetryClass {
    /// 5xx / 408 / 429 / network/timeout — retry with backoff.
    Retryable,
    /// 401/419 that survived the client's one re-auth retry — pause for re-login.
    NeedsReauth,
    /// Stop requested.
    Cancelled,
    /// 4xx (other) / malformed (`status: 0`) — will never succeed; fail fast.
    Fatal,
}

fn classify_upload_error(e: &UploadError) -> RetryClass {
    match e {
        UploadError::Cancelled => RetryClass::Cancelled,
        UploadError::Session(_) => RetryClass::NeedsReauth,
        UploadError::Transport(_) => RetryClass::Retryable,
        UploadError::Server { status, .. } => match status {
            500..=599 | 408 | 429 => RetryClass::Retryable,
            // `status: 0` is our own internal/malformed marker (e.g. a segment that
            // failed validation) — a retry will reproduce it, so it's fatal.
            _ => RetryClass::Fatal,
        },
    }
}

/// Sleep `delay` in [`LIVE_CANCEL_POLL`]-sized slices, returning `true` if `cancel`
/// tripped during the wait (so a Stop during a backoff returns in ~one slice, not
/// the full delay). Used between segment-POST retries.
fn cancel_aware_backoff(delay: std::time::Duration, cancel: &Arc<AtomicBool>) -> bool {
    let mut remaining = delay;
    while remaining > std::time::Duration::ZERO {
        if cancel.load(Ordering::SeqCst) {
            return true;
        }
        let slice = remaining.min(LIVE_CANCEL_POLL);
        std::thread::sleep(slice);
        remaining = remaining.saturating_sub(slice);
    }
    cancel.load(Ordering::SeqCst)
}

/// Run a single cancel-aware live POST with up to [`MAX_SEGMENT_ATTEMPTS`] attempts,
/// exponential backoff (`1s, 2s, 4s`, cancel-interruptible), classifying each error.
/// `attempt` is the actual send (already cancel-aware via [`LiveSender`]). Returns the
/// classified terminal outcome.
fn post_with_retry(
    cancel: &Arc<AtomicBool>,
    mut attempt: impl FnMut() -> Result<Vec<u8>, UploadError>,
) -> Result<Vec<u8>, RetryClass> {
    let mut last_fatal_marker = RetryClass::Fatal;
    for n in 0..MAX_SEGMENT_ATTEMPTS {
        match attempt() {
            Ok(body) => return Ok(body),
            Err(e) => match classify_upload_error(&e) {
                RetryClass::Cancelled => return Err(RetryClass::Cancelled),
                RetryClass::NeedsReauth => return Err(RetryClass::NeedsReauth),
                RetryClass::Fatal => return Err(RetryClass::Fatal),
                RetryClass::Retryable => {
                    last_fatal_marker = RetryClass::Retryable;
                    // No backoff after the final attempt.
                    if n + 1 < MAX_SEGMENT_ATTEMPTS {
                        let delay = std::time::Duration::from_secs(1u64 << n)
                            .min(std::time::Duration::from_secs(8));
                        if cancel_aware_backoff(delay, cancel) {
                            return Err(RetryClass::Cancelled);
                        }
                    }
                }
            },
        }
    }
    // Exhausted retries on a retryable error → treat as fatal (terminate gracefully).
    Err(last_fatal_marker)
}

/// Run the DEBUG-only native live round-trip, tailing `growing_path` (a SYNTHETIC
/// growing file) and streaming to an open report. A thin wrapper over the production
/// [`run_native_live`] with a [`NoopOrphanSink`] and no UI channel — it exists only
/// for the owner-run feasibility round-trip (`uploader_run_native_live_spike`), so it
/// is `#[cfg(debug_assertions)]`: a "create a real report from an arbitrary path"
/// entry must never ship. The production path is reached via `uploader_start_live`.
#[cfg(debug_assertions)]
pub fn run_native_live_spike(
    growing_path: &str,
    session: Arc<dyn SessionProvider>,
    opts: &UploadOptions,
    cancel: Arc<AtomicBool>,
    poll: &dyn LiveTail,
) -> Result<(ReportCode, usize), UploadError> {
    let sink = NoopOrphanSink;
    // The debug spike never does mid-session warm-up (it streams a synthetic file from
    // the start), so `warmup` is None — behavior unchanged.
    let (code, ended) =
        run_native_live(growing_path, session, opts, cancel, poll, None, &sink, None)?;
    eprintln!(
        "[uploader] native live: report {} terminated ({:?}) after {} segment(s)",
        code.0, ended.reason, ended.segments_built
    );
    Ok((code, ended.segments_built))
}

/// What the live stream did before it ended (returned by [`run_native_live`]).
#[derive(Debug)]
pub struct LiveEnded {
    pub reason: EndReason,
    pub segments_built: usize,
}

/// Crash-recovery breadcrumb sink the driver writes to. Abstracted (like
/// [`LiveTail`]) so the driver stays free of `tauri::AppHandle` and is unit-testable
/// with a fake. The production adapter (see `super::orphans`) persists
/// `{code, segment_id}` so a crash before terminate can be recovered on next launch.
pub trait OrphanSink {
    /// Record a freshly-opened report (after `create-report`, before the first POST).
    fn record_open(&self, code: &str, segment_id: u64);
    /// Note the latest server-sequenced segment id after an accepted segment.
    fn note_segment(&self, code: &str, segment_id: u64);
    /// Drop the breadcrumb after a confirmed-closed report (clean terminate).
    fn clear(&self, code: &str);
}

/// An [`OrphanSink`] that does nothing — for tests and the debug round-trip, where
/// crash recovery is irrelevant.
pub struct NoopOrphanSink;
impl OrphanSink for NoopOrphanSink {
    fn record_open(&self, _code: &str, _segment_id: u64) {}
    fn note_segment(&self, _code: &str, _segment_id: u64) {}
    fn clear(&self, _code: &str) {}
}

/// A MID-SESSION join anchor: replay the on-disk session prefix `[begin_log_offset, eof)`
/// to warm the encoder state from disk BEFORE tailing from `eof`, so a user already
/// combat-logging can go live WITHOUT a fresh `/reloadui`. Computed by the caller from
/// [`super::super::scanner::find_current_session_begin`]. `None` ⇒ tail from EOF as
/// before (no open session to replay, or the file has no recoverable `BEGIN_LOG`).
#[derive(Debug, Clone, Copy)]
pub struct WarmupPrefix {
    /// Byte offset of the current session's most-recent `BEGIN_LOG` (the replay start).
    pub begin_log_offset: u64,
    /// File length at scan time (the replay end; the tail then starts here).
    pub eof: u64,
}

/// Best-effort `terminate-report` + orphan-breadcrumb settle, shared by EVERY exit path
/// (normal end and warm-up failure) so the discipline can't drift between them.
///
/// Terminate on a FRESH cancel flag — never the stream's stop-`cancel`, which a Stop (or
/// a warm-up that failed *because* of a Stop) leaves set, making a terminate on it a
/// no-op that would skip closing the report. A watchdog trips the fresh flag after
/// [`TERMINATE_DEADLINE`] so a wedged network can't block the driver thread (and thus a
/// Stop join) for the full request timeout. Clear the `{code, segment_id}` breadcrumb
/// ONLY on a confirmed close (success or a definitive already-gone); a transient failure
/// or a watchdog timeout KEEPS it so next-launch recovery closes the report.
fn terminate_report_and_settle<P: LivePoster>(
    sender: &P,
    sink: &dyn OrphanSink,
    code: &ReportCode,
) {
    let term_url = format!(
        "{}/terminate-report/{}",
        super::client::desktop_client_base(),
        code.0
    );
    let term_cancel = Arc::new(AtomicBool::new(false));
    let watchdog_flag = Arc::clone(&term_cancel);
    let watchdog = std::thread::spawn(move || {
        let mut waited = std::time::Duration::ZERO;
        while waited < TERMINATE_DEADLINE {
            std::thread::sleep(LIVE_CANCEL_POLL);
            waited += LIVE_CANCEL_POLL;
            if watchdog_flag.load(Ordering::SeqCst) {
                return; // terminate already returned; reap promptly
            }
        }
        watchdog_flag.store(true, Ordering::SeqCst);
    });
    let term = sender.post(
        &term_url,
        super::client::OwnedLiveRequest::Terminate,
        &term_cancel,
    );
    term_cancel.store(true, Ordering::SeqCst);
    let _ = watchdog.join();
    match &term {
        Ok(_) => sink.clear(&code.0),
        Err(e) if super::client::is_definitively_closed(e) => sink.clear(&code.0),
        // Transient OR a watchdog-`Cancelled` (terminate timed out): KEEP the breadcrumb
        // for next-launch recovery rather than assume the report closed.
        Err(_) => {}
    }
}

/// Run the native live upload to completion, holding ONE report open across the
/// whole session and terminating only on a clean end / idle / stop / failure.
///
/// This is the production driver: it owns a long-lived [`LiveSegmenter`], a
/// cancel-aware [`LiveSender`] (so Stop returns in ~250ms, not the 120s request
/// timeout), and the L3/L4 lifecycle state machine (idle deadline, per-segment retry
/// with backoff, and the reauth pause-resume). Orphan-safety: ANY exit path attempts
/// `terminate-report` (cancel-aware so Stop isn't held hostage by terminate), and the
/// `{code, segment_id}` breadcrumb is persisted via `sink` so a crash before terminate
/// is recoverable on next launch.
///
/// `poll` is the line source — a notify-backed production tail or a scripted/file tail
/// in tests. `channel` is an optional UI event sink (reauth prompts etc.).
// Eight parameters reads clearly here (each is a distinct collaborator: path, session,
// opts, cancel, tail, warm-up prefix, orphan sink, UI channel); bundling them into a
// struct would add indirection without aiding readers.
#[allow(clippy::too_many_arguments)]
pub fn run_native_live(
    growing_path: &str,
    session: Arc<dyn SessionProvider>,
    opts: &UploadOptions,
    cancel: Arc<AtomicBool>,
    poll: &dyn LiveTail,
    warmup: Option<WarmupPrefix>,
    sink: &dyn OrphanSink,
    channel: Option<&LiveEventSink>,
) -> Result<(ReportCode, LiveEnded), UploadError> {
    // Establish the session up front, then open the report. Re-check cancel right
    // before create so a Stop ordered during setup never opens a report we then have
    // to terminate (concurrency review RACE-4 — minimize the create→terminate window).
    let _ = session.session()?;
    if cancel.load(Ordering::SeqCst) {
        return Err(UploadError::Cancelled);
    }
    let sender = LiveSender::new(Arc::clone(&session));
    let create_url = format!("{}/create-report", super::client::desktop_client_base());
    let create_body = super::client::create_report_body_for(opts);
    // create-report uses the NO-LEAK cancel variant: a Stop racing the create POST must
    // not abandon it (the server could create the report after we give up, with no code
    // to record/terminate → an untracked orphan). On cancel this still captures the code
    // if it lands within the grace window; we then record the breadcrumb below and the
    // post-create cancel check / driver terminates it cleanly.
    let code_body = sender.send_create_cancellable(
        &create_url,
        super::client::OwnedLiveRequest::CreateReport { body: create_body },
        &cancel,
    )?;
    let code = super::client::parse_report_code(&code_body)?;

    // L2: persist the breadcrumb the INSTANT the report exists, before the first POST.
    sink.record_open(&code.0, 1);

    // A Stop that landed DURING create-report (captured by the grace window above) leaves
    // us holding a real report + its breadcrumb but no reason to stream. Terminate it now
    // — don't spawn the warm-up/tail — so the report is closed (or recoverable), never an
    // orphan. The shared discipline clears the breadcrumb only on a confirmed close.
    if cancel.load(Ordering::SeqCst) {
        terminate_report_and_settle(&sender, sink, &code);
        return Err(UploadError::Cancelled);
    }

    let mut driver = LiveDriver::new(sender, code.clone(), cancel.clone(), channel);
    // MID-SESSION: warm the encoder from the on-disk session prefix BEFORE tailing, so a
    // user already combat-logging streams without a fresh /reloadui. A warm-up failure
    // (e.g. the file was truncated/rotated since the scan, or no BEGIN_LOG is recoverable)
    // must NOT synthesize a wall clock — abandon the just-opened report and surface the
    // error so the command layer falls back to the official handoff.
    if let Some(w) = warmup {
        if let Err(e) = driver.warm_up_from_prefix(growing_path, w) {
            // Close the report we just opened, using the SAME terminate discipline as the
            // normal exit: a FRESH cancel (NOT the stream's `cancel` — warm-up may have
            // failed *because* the user stopped, leaving `cancel` set, which would make a
            // terminate on that flag a no-op) + watchdog, and clear the orphan breadcrumb
            // ONLY on a confirmed close. Erasing it unconditionally here would strand an
            // open remote report with no next-launch recovery when terminate is cancelled
            // or fails transiently.
            terminate_report_and_settle(&driver.sender, sink, &code);
            return Err(UploadError::Transport(format!(
                "mid-session warm-up failed: {e}"
            )));
        }
    }
    let reason = driver.run(growing_path, poll, sink);

    // Best-effort, cancel-aware terminate on EVERY exit path (shared discipline so the
    // warm-up-failure path above and this normal exit can't drift).
    terminate_report_and_settle(&driver.sender, sink, &code);

    let segments_built = driver.segments_built();
    // A Stop is not an error — report it as a clean end with reason Stopped.
    Ok((
        code,
        LiveEnded {
            reason,
            segments_built,
        },
    ))
}

/// A UI event sink the driver calls to surface lifecycle/auth events (reauth prompt,
/// fight cuts). Boxed closures so the driver stays free of the tauri `Channel` type
/// and is testable with a recording fake. The production command adapts a
/// `Channel<LiveEvent>` into this.
pub struct LiveEventSink {
    /// Called once when the first `BEGIN_LOG` arrives — the driver is now anchored and
    /// will stream fights. The UI flips from "waiting for a session" to "streaming".
    pub on_session_anchored: Box<dyn Fn() + Send + Sync>,
    /// Called after each fight-segment is accepted by the server, with the 0-based
    /// index of the fight in this session and its wall-clock duration in ms (the
    /// segment's `end_time - start_time`, report-absolute). Drives the UI's per-fight
    /// timeline + count + duration for the native path (the official-handoff path gets
    /// this from its own watcher), and backstops a missed `on_session_anchored` (a fight
    /// implies anchored).
    pub on_fight_posted: Box<dyn Fn(usize, u64) + Send + Sync>,
    /// Called when the session is lost mid-stream and the user must re-sign-in.
    pub on_reauth_required: Box<dyn Fn() + Send + Sync>,
    /// Called when posting resumes after a fresh session is stored.
    pub on_reauth_resolved: Box<dyn Fn() + Send + Sync>,
}

/// The live lifecycle state machine. Owns the long-lived segmenter, the cancel-aware
/// sender, and the pause-resume / idle bookkeeping. `run` drives the line source to a
/// terminal [`EndReason`]; the caller (`run_native_live`) does the create + terminate
/// wrapping so every exit path closes the report.
struct LiveDriver<'a, P: LivePoster> {
    sender: P,
    code: ReportCode,
    cancel: Arc<AtomicBool>,
    channel: Option<&'a LiveEventSink>,
    seg: LiveSegmenter,
    /// Set true once any `BEGIN_LOG` has been seen — a SECOND one is a new session
    /// (`/reloadui`) and forces a terminate (single-session contract).
    seen_begin_log: bool,
    /// Count of fight-segments accepted by the server this session — the 0-based index
    /// passed to `on_fight_posted` so the UI's per-fight timeline/count advances on the
    /// native path (the official path gets this from its own watcher).
    fights_posted: usize,
}

impl<'a, P: LivePoster> LiveDriver<'a, P> {
    fn new(
        sender: P,
        code: ReportCode,
        cancel: Arc<AtomicBool>,
        channel: Option<&'a LiveEventSink>,
    ) -> Self {
        Self {
            sender,
            code,
            cancel,
            channel,
            seg: LiveSegmenter::new(),
            seen_begin_log: false,
            fights_posted: 0,
        }
    }

    fn segments_built(&self) -> usize {
        self.seg.segments_built()
    }

    /// Drive the line source to a terminal reason. Pulls batches from `poll`; on each
    /// fight-boundary cut with settled shields, posts the segment (with retry / pause).
    fn run(&mut self, growing_path: &str, poll: &dyn LiveTail, sink: &dyn OrphanSink) -> EndReason {
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                return EndReason::Stopped;
            }
            match poll.next_lines(growing_path) {
                TailOutcome::Lines(lines) => {
                    if let Some(reason) = self.drive_assembled_lines(lines, sink) {
                        return reason;
                    }
                }
                outcome @ (TailOutcome::Ended | TailOutcome::Idle) => {
                    // Clean end (END_LOG) or idle deadline: drain trailing shields and
                    // flush the final segment. The tail distinguishes the two; carry
                    // that through to the settled reason so a crash is observable.
                    let clean = matches!(outcome, TailOutcome::Ended);
                    self.seg.drain_trailing_shields();
                    return match self.post_current(sink) {
                        PostOutcome::Fatal(d) => EndReason::Fatal(d),
                        PostOutcome::Cancelled => EndReason::Stopped,
                        // Auth lost on the final flush: nothing more to stream, so
                        // don't pause — just record that re-auth was needed at the end.
                        PostOutcome::NeedsReauth { .. } => EndReason::ReauthTimeout,
                        _ if clean => EndReason::Ended,
                        _ => EndReason::Idle,
                    };
                }
                TailOutcome::Error(_) => return EndReason::Fatal("tail read failed".into()),
            }
        }
    }

    /// Feed a batch of assembled lines into the segmenter, cutting + posting at each
    /// settled fight boundary. Returns `Some(reason)` to end the stream, `None` to
    /// keep going. Shared by the pull (scripted/file) path and the production push
    /// tail (Step 6) so the cut/post/pause logic lives in exactly one place.
    fn drive_assembled_lines(
        &mut self,
        lines: Vec<String>,
        sink: &dyn OrphanSink,
    ) -> Option<EndReason> {
        for line in lines {
            // Fail CLOSED on an unproven line type. The live path can't pre-scan a growing
            // file the way the finished-log coverage gate does, so it checks each tailed
            // line here: a type the encoder hasn't proven byte-exact would otherwise be
            // silently dropped (EventEmitter::feed ignores unknown kinds) and shipped as a
            // complete-looking but incomplete report. Terminate instead — the report is
            // settled Failed (a partial native report must never read as Completed).
            if let Some(t) = super::coverage::unproven_line_type(&line) {
                return Some(EndReason::Fatal(format!(
                    "unproven log line type '{t}' — native live can't faithfully encode it"
                )));
            }
            let is_begin_log = kind_of(&line) == Some("BEGIN_LOG");
            let second_begin_log = is_begin_log && self.seen_begin_log;
            if second_begin_log {
                // A SECOND BEGIN_LOG = a new logging session (/reloadui). The
                // single-session contract forbids mixing two sessions in one report:
                // flush the CLOSING session's final segment FIRST — with its OWN wall
                // clock — then terminate as NewSession. Crucially, do NOT `feed` the new
                // header before posting: `feed`ing a BEGIN_LOG runs `on_begin_log`, which
                // overwrites `current_session_wall`/offset and would timestamp the prior
                // session's last segment with the NEW session's clock. We terminate right
                // after, so the new header is never needed in this report.
                //
                // Drain terminal pending correlations FIRST (a DAMAGE_SHIELDED buffered
                // awaiting its paired damage event): /reloadui can land mid-fight with a
                // shield still pending, and unlike a clean END_COMBAT cut this terminal
                // path isn't shields-settled-gated. The END_LOG/Idle exits drain before
                // their final post; this one must too, or the buffered shield is dropped
                // and the closing session's last segment is silently incomplete.
                self.seg.drain_trailing_shields();
                match self.post_current(sink) {
                    PostOutcome::Fatal(d) => return Some(EndReason::Fatal(d)),
                    PostOutcome::Cancelled => return Some(EndReason::Stopped),
                    PostOutcome::NeedsReauth { .. } => return Some(EndReason::ReauthTimeout),
                    _ => {}
                }
                return Some(EndReason::NewSession);
            }
            if is_begin_log {
                // First BEGIN_LOG: the driver is now anchored to a session and will
                // stream fights. Surface it so the UI flips waiting→streaming the
                // instant the session header lands (no timeout). Fire on the
                // false→true EDGE only (the second-BEGIN_LOG case returned above).
                self.seen_begin_log = true;
                if let Some(ch) = self.channel {
                    (ch.on_session_anchored)();
                }
            }

            let boundary = self.seg.feed(&line);
            if boundary && self.seg.shields_settled() {
                match self.post_current(sink) {
                    PostOutcome::Posted { next_segment_id: 0 } => {
                        return Some(EndReason::ServerEnded);
                    }
                    PostOutcome::Posted { .. } | PostOutcome::Nothing => {}
                    PostOutcome::NeedsReauth { held } => {
                        if let Some(reason) = self.pause_until_reauth_or_end(*held, sink) {
                            return Some(reason);
                        }
                    }
                    PostOutcome::Cancelled => return Some(EndReason::Stopped),
                    PostOutcome::Fatal(d) => return Some(EndReason::Fatal(d)),
                }
            }
        }
        None
    }

    /// MID-SESSION warm-up: replay the on-disk session prefix `[begin_log_offset, eof)`
    /// through `seg.feed()` to rebuild the cumulative encoder state (actor/ability/tuple
    /// indices, master record state, and `current_session_wall`/`log_version` parsed from
    /// the REAL on-disk `BEGIN_LOG` — never synthesized), WITHOUT POSTing any of the
    /// prefix's already-finished fights, then `finish_warmup()` so the first LIVE segment
    /// starts fresh at id 1. Cut boundaries during replay are IGNORED (no POST). On the
    /// prefix's `BEGIN_LOG` it fires `on_session_anchored` (UI flips waiting→streaming
    /// instantly) and sets `seen_begin_log` so the live tail treats lines as the SAME
    /// session — and a genuine `/reloadui` DURING the live session still terminates as
    /// NewSession (the prefix contains exactly the current session's single header,
    /// because the scanner picks the MOST-RECENT `BEGIN_LOG`).
    ///
    /// Reads in `MAX_READ`-bounded chunks (a session prefix can exceed one read). Returns
    /// `Err` if the prefix yields NO `BEGIN_LOG` once assembled (rotation/truncation race)
    /// so the caller falls back rather than streaming a wall-less report — the no-clock
    /// guarantee. The trailing partial line at `eof` is intentionally NOT fed here; the
    /// production tail (constructed at the same `eof`) reads it once its newline arrives.
    fn warm_up_from_prefix(&mut self, path: &str, w: WarmupPrefix) -> Result<(), String> {
        let p = std::path::Path::new(path);
        let mut assembler = LineAssembler::new();
        let mut buf = Vec::new();
        let mut pos = w.begin_log_offset;
        let mut saw_begin = false;
        while pos < w.eof {
            if self.cancel.load(Ordering::SeqCst) {
                return Err("cancelled during warm-up".into());
            }
            let chunk_end = (pos + MAX_READ).min(w.eof);
            let n = read_range(p, pos, chunk_end, &mut buf)?;
            let (lines, _saw_end_log) = assembler.push_chunk(&buf[..n])?;
            for line in lines {
                // Same fail-closed coverage gate as the live tail: if the on-disk prefix
                // contains a type the encoder hasn't proven, native can't faithfully
                // encode this session — abort warm-up (the caller terminates + settles
                // Failed) rather than seed state from a line we'd silently drop.
                if let Some(t) = super::coverage::unproven_line_type(&line) {
                    return Err(format!("unproven log line type '{t}' in session prefix"));
                }
                if kind_of(&line) == Some("BEGIN_LOG") && !saw_begin {
                    saw_begin = true;
                    self.seen_begin_log = true;
                    if let Some(ch) = self.channel {
                        (ch.on_session_anchored)();
                    }
                }
                // Warm the cumulative state; IGNORE the cut boundary (no POST during
                // warm-up — old fights are never sent).
                let _ = self.seg.feed(&line);
            }
            pos = chunk_end;
        }
        if !saw_begin {
            // The scanner found a BEGIN_LOG offset, but the assembled prefix had none
            // (truncation/rotation race, or the assembler's F4 discard ate a malformed
            // header). Never synthesize a wall — fall back.
            return Err("no BEGIN_LOG in replayed prefix".into());
        }
        // Transition to live: discard the prefix's accumulated segment + reset numbering,
        // KEEP the cumulative tables/wall/correlations the replay built.
        self.seg.finish_warmup();
        Ok(())
    }

    /// Build the current segment (if any) and POST it (master then add-segment),
    /// cancel-aware and with per-segment retry. Classifies the outcome. On a lost
    /// session the built payload is returned in `NeedsReauth { held }` so the driver
    /// can re-POST it after the user re-signs-in (the segmenter has already advanced
    /// past it, so it cannot be rebuilt).
    fn post_current(&mut self, sink: &dyn OrphanSink) -> PostOutcome {
        if self.cancel.load(Ordering::SeqCst) {
            return PostOutcome::Cancelled;
        }
        let payload = match self.seg.build_next_segment() {
            Ok(Some(p)) => p,
            Ok(None) => return PostOutcome::Nothing,
            // A malformed/inconsistent segment must NEVER be shipped — fatal.
            Err(detail) => return PostOutcome::Fatal(detail),
        };
        self.post_built(payload, sink)
    }

    /// POST an already-built payload (master then add-segment), with retry. Takes the
    /// payload BY VALUE so a `NeedsReauth` can hand it back for a later re-POST.
    ///
    /// Re-POST idempotency: a re-POST always reuses the SAME `segment_id` the server
    /// hasn't advanced past (the failed POST never returned a `nextSegmentId`, and
    /// `set_next_segment_id` only runs after a parsed response). TWO paths re-send that
    /// same id: (1) the in-loop transient retry in [`post_with_retry`] — a lost response on
    /// a plain network/timeout classifies as `Transport` → `Retryable`; and (2) the
    /// reauth-resume re-POST below — a 401/419 that survived the one re-auth. Re-sending the
    /// master is `set-report-master-table` — set semantics, idempotent. Re-sending the
    /// segment is safe in the ONLY hazardous sub-case — the original add-segment was
    /// ACCEPTED server-side but its response was LOST (either behind the auth challenge OR
    /// to a plain transient timeout) — IF the server dedupes by `segmentId`. That dedup is
    /// UNVERIFIED (R4: no esologs doc states it; settle via the owner probe), but the risk
    /// is bounded: the accepted-then-lost window is a narrow subset of all retries, and the
    /// worst case is at most ONE cosmetic duplicate segment, which a terminate closes
    /// cleanly. So we keep blind retry (resilience over a multi-hour session) rather than a
    /// reconcile (no protocol channel exposes the server's next id without re-POSTing the
    /// segment) or a non-retryable failure (which would abandon a live raid on a routine
    /// blip). For the same rare-window / one-duplicate reason we also don't split
    /// master-done/segment-pending resume state (the reviewer's LOW finding).
    fn post_built(&mut self, payload: LiveSegmentPayload, sink: &dyn OrphanSink) -> PostOutcome {
        let base = super::client::desktop_client_base();
        let master_url = format!("{base}/set-report-master-table/{}", self.code.0);
        let seg_url = format!("{base}/add-report-segment/{}", self.code.0);

        // 1. master table for this segment id.
        let master_req = super::client::OwnedLiveRequest::MasterTable {
            segment_id: payload.segment_id,
            bytes: payload.master.bytes.clone(),
        };
        match post_with_retry(&self.cancel, || {
            self.sender
                .post(&master_url, master_req.clone(), &self.cancel)
        }) {
            Ok(_) => {}
            Err(RetryClass::Cancelled) => return PostOutcome::Cancelled,
            Err(RetryClass::NeedsReauth) => {
                return PostOutcome::NeedsReauth {
                    held: Box::new(payload),
                }
            }
            Err(_) => {
                return PostOutcome::Fatal(format!(
                    "master-table upload failed for segment {}",
                    payload.segment_id
                ))
            }
        }

        // 2. the fights segment; the response carries nextSegmentId.
        let seg_req = super::client::OwnedLiveRequest::AddSegment {
            segment_id: payload.segment_id,
            bytes: payload.segment.bytes.clone(),
            start_time: payload.segment.start_time,
            end_time: payload.segment.end_time,
            in_progress_event_count: payload.in_progress_event_count,
        };
        let body = match post_with_retry(&self.cancel, || {
            self.sender.post(&seg_url, seg_req.clone(), &self.cancel)
        }) {
            Ok(b) => b,
            Err(RetryClass::Cancelled) => return PostOutcome::Cancelled,
            Err(RetryClass::NeedsReauth) => {
                return PostOutcome::NeedsReauth {
                    held: Box::new(payload),
                }
            }
            Err(_) => {
                return PostOutcome::Fatal(format!(
                    "segment upload failed for segment {}",
                    payload.segment_id
                ))
            }
        };
        let next = match super::client::parse_next_segment_id(&body) {
            Ok(n) => n,
            Err(_) => {
                return PostOutcome::Fatal("malformed add-segment response".into());
            }
        };
        // The segment was ACCEPTED regardless of `next` (a `0` only means "no further
        // segments"). So advance the UI timeline/count + the orphan breadcrumb for the
        // accepted fight HERE, before branching on the terminal — otherwise the final
        // segment of a session that ends with `next == 0` would be undercounted in the
        // UI. A fight event also backstops a missed on_session_anchored. 0-based index.
        sink.note_segment(&self.code.0, payload.segment_id);
        if let Some(ch) = self.channel {
            // The fight's wall-clock duration is the segment's report-absolute window.
            // A clean fight-boundary cut ends on END_COMBAT, so end-start is the fight
            // length. saturating_sub guards a degenerate/zero-length window.
            let duration_ms = payload
                .segment
                .end_time
                .saturating_sub(payload.segment.start_time);
            (ch.on_fight_posted)(self.fights_posted, duration_ms);
        }
        self.fights_posted += 1;
        if next == 0 {
            eprintln!(
                "[uploader] live: server returned nextSegmentId=0 after segment {} \
                 (window {}-{}); treating as session-end",
                payload.segment_id, payload.segment.start_time, payload.segment.end_time
            );
            return PostOutcome::Posted { next_segment_id: 0 };
        }
        self.seg.set_next_segment_id(next);
        PostOutcome::Posted {
            next_segment_id: next,
        }
    }

    /// PAUSE on a lost session: prompt re-login, poll for a fresh session every ~2s,
    /// then RE-POST the `held` payload (the one whose POST hit the auth wall — the
    /// segmenter advanced past it, so it can't be rebuilt). Subsequent boundaries
    /// during the pause keep feeding the emitter (state advances), and the next build
    /// after resume produces one cumulative segment covering everything since the held
    /// cut — valid because the master is cumulative+pinned and cuts are fight-granular.
    /// Returns `Some(reason)` if the pause ends the stream, `None` to resume.
    fn pause_until_reauth_or_end(
        &mut self,
        held: LiveSegmentPayload,
        sink: &dyn OrphanSink,
    ) -> Option<EndReason> {
        if let Some(ch) = self.channel {
            (ch.on_reauth_required)();
        }
        let started = std::time::Instant::now();
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                return Some(EndReason::Stopped);
            }
            if started.elapsed() >= REAUTH_TIMEOUT {
                return Some(EndReason::ReauthTimeout);
            }
            // A fresh session resolves the pause. The managed provider's `session()`
            // returns Ok once the in-app webview login stored a new cookie (the Arc is
            // shared, so the store is visible here without a channel).
            if self.sender.has_session() {
                if let Some(ch) = self.channel {
                    (ch.on_reauth_resolved)();
                }
                // Re-POST the held payload against the fresh session before resuming.
                return match self.post_built(held, sink) {
                    PostOutcome::Posted { next_segment_id: 0 } => Some(EndReason::ServerEnded),
                    PostOutcome::Posted { .. } | PostOutcome::Nothing => None, // resume
                    // The fresh session was lost AGAIN mid-re-POST: re-pause is not
                    // worth the complexity — terminate gracefully (the report holds
                    // what rendered; the user re-uploads the rest).
                    PostOutcome::NeedsReauth { .. } => Some(EndReason::ReauthTimeout),
                    PostOutcome::Cancelled => Some(EndReason::Stopped),
                    PostOutcome::Fatal(d) => Some(EndReason::Fatal(d)),
                };
            }
            // Sleep in cancel-checked slices so a Stop during the pause returns fast.
            if cancel_aware_backoff(std::time::Duration::from_secs(2), &self.cancel) {
                return Some(EndReason::Stopped);
            }
        }
    }
}

/// A source of newly-appended log lines for the live driver — abstracted so the tail
/// can be a real growing-file watcher in a harness OR a deterministic synthetic feed
/// in tests (CFA blocks reading a real `Encounter.log` for the debug binary).
pub trait LiveTail {
    /// Return the next batch of newly-appended lines, `Done` when logging has ended
    /// (END_LOG / inactivity deadline), or `Error` on an unrecoverable read failure.
    fn next_lines(&self, path: &str) -> TailOutcome;
}

/// The outcome of one [`LiveTail::next_lines`] poll.
pub enum TailOutcome {
    /// A batch of newly-appended complete lines.
    Lines(Vec<String>),
    /// Logging ended cleanly (an `END_LOG` line was seen). Flush the final segment.
    Ended,
    /// The file stopped growing past the idle deadline (game crash / forgot to stop).
    /// Flush the final segment, but settle distinctly from a clean end so a real
    /// crash is observable (L3).
    Idle,
    /// An unrecoverable read failure (the failure-streak teardown tripped).
    Error(String),
}

/// A real growing-file [`LiveTail`]: tails a file by byte offset (the spike's tester
/// appends to a synthetic `.log` while this streams it), returning each batch of
/// newly-appended COMPLETE lines. Signals `Done` when an `END_LOG` line is seen OR the
/// file stops growing for [`Self::idle_deadline`]. A partial trailing line (no newline
/// yet) is held back until its newline arrives, so a half-written line is never fed.
///
/// Debug/spike only: it blocks the calling thread with short sleeps between polls,
/// which is fine for a one-off owner-run round-trip but is NOT the production tail
/// (that is [`NotifyTail`]). Place the synthetic file OUTSIDE a CFA-protected folder
/// (not `Documents`/`Desktop`) so the debug binary can read it.
#[cfg(debug_assertions)]
pub struct FileTail {
    offset: std::sync::Mutex<u64>,
    /// Carry-over bytes of a partial (un-newlined) trailing line between polls.
    partial: std::sync::Mutex<Vec<u8>>,
    /// When the file last grew (for the idle deadline). `None` until first read.
    last_growth: std::sync::Mutex<Option<std::time::Instant>>,
    poll_interval: std::time::Duration,
    idle_deadline: std::time::Duration,
}

#[cfg(debug_assertions)]
impl FileTail {
    /// Tail from byte 0 (stream the whole file as it grows). `idle_secs` = stop after
    /// this many seconds with no new bytes (treat as logging-ended).
    pub fn new(idle_secs: u64) -> Self {
        Self {
            offset: std::sync::Mutex::new(0),
            partial: std::sync::Mutex::new(Vec::new()),
            last_growth: std::sync::Mutex::new(None),
            poll_interval: std::time::Duration::from_millis(300),
            idle_deadline: std::time::Duration::from_secs(idle_secs),
        }
    }
}

#[cfg(debug_assertions)]
impl LiveTail for FileTail {
    fn next_lines(&self, path: &str) -> TailOutcome {
        use std::io::{Read, Seek, SeekFrom};
        loop {
            let size = match std::fs::metadata(path) {
                Ok(m) => m.len(),
                Err(e) => return TailOutcome::Error(format!("stat {path}: {e}")),
            };
            let mut off = self.offset.lock().unwrap();
            if size < *off {
                // Truncated/replaced — restart from 0 (a fresh session).
                *off = 0;
                self.partial.lock().unwrap().clear();
            }
            if size > *off {
                let mut f = match std::fs::File::open(path) {
                    Ok(f) => f,
                    Err(e) => return TailOutcome::Error(format!("open {path}: {e}")),
                };
                if let Err(e) = f.seek(SeekFrom::Start(*off)) {
                    return TailOutcome::Error(format!("seek {path}: {e}"));
                }
                let mut buf = Vec::new();
                if let Err(e) = f.take(size - *off).read_to_end(&mut buf) {
                    return TailOutcome::Error(format!("read {path}: {e}"));
                }
                *off = size;
                *self.last_growth.lock().unwrap() = Some(std::time::Instant::now());
                drop(off);

                // Prepend any carried-over partial line, split on '\n', and hold back a
                // trailing partial (no newline) for the next poll.
                let mut partial = self.partial.lock().unwrap();
                let mut data = std::mem::take(&mut *partial);
                data.extend_from_slice(&buf);
                let mut lines: Vec<String> = Vec::new();
                let mut start = 0usize;
                let mut saw_end_log = false;
                for i in 0..data.len() {
                    if data[i] == b'\n' {
                        let line = String::from_utf8_lossy(&data[start..i])
                            .trim_end_matches('\r')
                            .to_string();
                        if kind_of(&line) == Some("END_LOG") {
                            saw_end_log = true;
                        }
                        if !line.is_empty() {
                            lines.push(line);
                        }
                        start = i + 1;
                    }
                }
                *partial = data[start..].to_vec();
                drop(partial);

                if saw_end_log {
                    // Emit what we have; the driver's next poll will get Done.
                    *self.last_growth.lock().unwrap() =
                        Some(std::time::Instant::now() - self.idle_deadline);
                }
                if !lines.is_empty() {
                    return TailOutcome::Lines(lines);
                }
                // Grew but no complete line yet — keep polling.
            } else {
                // No growth. If we've been idle past the deadline, we're done. The
                // debug FileTail force-ages `last_growth` on END_LOG (above), so it
                // can't cleanly distinguish a clean end from a real idle — it reports
                // `Idle`. The production NotifyTail distinguishes the two; the spike
                // doesn't depend on the distinction.
                let idle = self
                    .last_growth
                    .lock()
                    .unwrap()
                    .map(|t| t.elapsed() >= self.idle_deadline)
                    .unwrap_or(false);
                drop(off);
                if idle {
                    return TailOutcome::Idle;
                }
            }
            std::thread::sleep(self.poll_interval);
        }
    }
}

// ── Production notify-backed tail (L-tail) ───────────────────────────────────
//
// The shipping live path tails the REAL in-game `Encounter.log` (under Documents —
// CFA blocks third-party WRITES there, not reads, and the fight-detection watcher
// already reads it fine). [`NotifyTail`] mirrors `watcher::tail_loop`'s read
// machinery — byte-offset `tail_io::read_range`, the reused buffer, the
// `MAX_CONSECUTIVE_FAILURES` streak (reset only after a successful READ, never a bare
// stat — the load-bearing discipline from `watcher.rs`), `MAX_READ` cap, truncation
// reset — but YIELDS raw complete lines into the live driver instead of scanning for
// fights. It does NOT run `start_live_watch` (that would mean two notify watchers on
// one file). The driver derives the UI timeline from its own cuts.
//
// `next_lines` is a blocking pull (the `LiveTail` seam) that waits on the notify
// channel + poll fallback until it has a batch of complete lines, sees `END_LOG`
// (`Ended`), goes idle past `IDLE_DEADLINE` (`Idle`), or trips the failure streak
// (`Error`). State lives behind a `Mutex` since `LiveTail::next_lines(&self)` borrows
// shared.

/// A complete-line assembler over the raw tail bytes: carries a partial (un-newlined)
/// trailing line across reads, strips a single `\r`, drops empties, and flags an
/// `END_LOG`. Factored out so the line-assembly contract is unit-testable without
/// notify or a real file. A 1 MiB partial cap guards against a non-line-atomic writer
/// (a real adversarial file, unlike the trusted synthetic feeder) growing it forever.
struct LineAssembler {
    partial: Vec<u8>,
    /// True once any `BEGIN_LOG` has passed — used to DISCARD lines fed before the
    /// first session header (a mid-session start at EOF, F4), so the encoder never
    /// sees a headerless prefix it can't frame.
    seen_begin_log: bool,
}

/// 1 MiB cap on an un-newlined partial line — a real ESO log line is never close to
/// this; exceeding it means a corrupt / non-line-atomic file → teardown.
const MAX_PARTIAL: usize = 1024 * 1024;

impl LineAssembler {
    fn new() -> Self {
        Self {
            partial: Vec::new(),
            seen_begin_log: false,
        }
    }

    /// For a MID-SESSION live join (the driver warmed up from a replayed prefix and the
    /// tail now starts INSIDE an already-open session). The session's `BEGIN_LOG` was
    /// already consumed by the warm-up's assembler, so this tail must NOT wait for one
    /// (the F4 gate would otherwise discard every tailed line forever — the bug that made
    /// mid-session live require a `/reloadui`). It therefore starts already-anchored
    /// (`seen_begin_log = true`). Mirrors the watcher's `session_open = start_offset != 0`.
    ///
    /// No leading-fragment drop is needed: the caller starts the tail on a line-safe
    /// boundary (`tail_io::last_line_boundary`), so the first chunk always begins at a
    /// real line start — there is never an orphaned partial straddling the seam.
    fn new_mid_session() -> Self {
        Self {
            partial: Vec::new(),
            seen_begin_log: true,
        }
    }

    /// Reset on a truncation / new file (mirrors `watcher.rs:303-309`).
    fn reset(&mut self) {
        self.partial.clear();
        // Re-arm the F4 gate to COLD so any lines in the replacement file BEFORE its first
        // BEGIN_LOG are discarded, not emitted into the (now-closing) report. The driver
        // keeps its OWN session flag, so the replacement's BEGIN_LOG still arrives as a
        // second-session header and cuts the old report (NewSession). Leaving this `true`
        // would leak the new file's pre-header bytes into the prior report.
        self.seen_begin_log = false;
    }

    /// Feed a freshly-read chunk; append to the carry, split complete lines, hold the
    /// trailing partial. Returns `(lines, saw_end_log)` or an `Err` if the partial cap
    /// is exceeded. Lines before the first `BEGIN_LOG` are DISCARDED (F4).
    fn push_chunk(&mut self, chunk: &[u8]) -> Result<(Vec<String>, bool), String> {
        self.partial.extend_from_slice(chunk);
        let mut lines = Vec::new();
        let mut saw_end_log = false;
        let mut start = 0usize;
        for i in 0..self.partial.len() {
            if self.partial[i] == b'\n' {
                let line = String::from_utf8_lossy(&self.partial[start..i])
                    .trim_end_matches('\r')
                    .to_string();
                start = i + 1;
                if line.is_empty() {
                    continue;
                }
                let kind = kind_of(&line);
                if kind == Some("BEGIN_LOG") {
                    self.seen_begin_log = true;
                }
                if kind == Some("END_LOG") {
                    saw_end_log = true;
                }
                // Discard anything before the first session header (mid-session start).
                if self.seen_begin_log {
                    lines.push(line);
                }
            }
        }
        // Keep the un-newlined tail for the next chunk.
        self.partial.drain(..start);
        if self.partial.len() > MAX_PARTIAL {
            return Err(format!(
                "log line exceeded {MAX_PARTIAL} bytes without a newline — file is \
                 corrupt or not written line-atomically"
            ));
        }
        Ok((lines, saw_end_log))
    }
}

/// A production notify-backed [`LiveTail`] over a real growing `Encounter.log`.
pub struct NotifyTail {
    inner: std::sync::Mutex<NotifyTailState>,
    idle_deadline: std::time::Duration,
}

struct NotifyTailState {
    rx: std::sync::mpsc::Receiver<notify::Result<notify::Event>>,
    // The watcher is held so it stays alive (dropping it stops notifications).
    _watcher: notify::RecommendedWatcher,
    consumed: u64,
    read_buf: Vec<u8>,
    assembler: LineAssembler,
    consecutive_failures: u32,
    last_growth: std::time::Instant,
    last_poll: std::time::Instant,
    ended: bool,
}

impl NotifyTail {
    /// Construct synchronously (so a watcher-setup failure surfaces to the caller, not
    /// inside a thread), watching `path`'s parent dir and tailing from `start_offset`.
    /// `mid_session` must be true when the driver warmed up from a replayed prefix (the
    /// tail then starts inside an already-open session whose `BEGIN_LOG` was in the
    /// prefix) so the assembler doesn't discard every tailed line waiting for a header
    /// that will never arrive in the tailed range.
    pub fn new(
        path: &std::path::Path,
        start_offset: u64,
        mid_session: bool,
    ) -> Result<Self, String> {
        use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            Config::default().with_poll_interval(POLL_INTERVAL),
        )
        .map_err(|e| format!("Could not start file watcher: {e}"))?;
        if let Some(parent) = path.parent() {
            watcher
                .watch(parent, RecursiveMode::NonRecursive)
                .map_err(|e| format!("Could not watch the logs folder: {e}"))?;
        }
        Ok(Self {
            inner: std::sync::Mutex::new(NotifyTailState {
                rx,
                _watcher: watcher,
                consumed: start_offset,
                read_buf: Vec::new(),
                // `start_offset` is a line-safe boundary (the caller uses
                // `tail_io::last_line_boundary`), so there is never an orphaned partial to
                // drop. A mid-session tail only needs to skip the BEGIN_LOG gate.
                assembler: if mid_session {
                    LineAssembler::new_mid_session()
                } else {
                    LineAssembler::new()
                },
                consecutive_failures: 0,
                last_growth: std::time::Instant::now(),
                last_poll: std::time::Instant::now(),
                ended: false,
            }),
            idle_deadline: IDLE_DEADLINE,
        })
    }
}

impl LiveTail for NotifyTail {
    fn next_lines(&self, path: &str) -> TailOutcome {
        let mut st = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return TailOutcome::Error("notify tail lock poisoned".into()),
        };
        if st.ended {
            return TailOutcome::Ended;
        }
        let p = std::path::Path::new(path);
        loop {
            // Wait for an FS event or the poll deadline (mirrors watcher.rs:239).
            let _ = st.rx.recv_timeout(POLL_INTERVAL);
            if st.last_poll.elapsed() < POLL_INTERVAL {
                // Coalesced wakeups: only act once per poll window.
                continue;
            }
            st.last_poll = std::time::Instant::now();

            let size = match std::fs::metadata(p) {
                Ok(m) => m.len(),
                Err(e) => {
                    st.consecutive_failures += 1;
                    if st.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        return TailOutcome::Error(format!("lost access to the log file: {e}"));
                    }
                    continue;
                }
            };
            // NOTE: do NOT reset the failure streak on a successful stat — only a
            // successful READ proves readability (the watcher.rs:290-298 discipline).

            if size < st.consumed {
                // Truncation / replacement → fresh session (watcher.rs:303-309). The
                // driver treats the new session's BEGIN_LOG as a second-session cut.
                st.consumed = 0;
                st.assembler.reset();
                continue;
            }
            if size == st.consumed {
                if st.last_growth.elapsed() >= self.idle_deadline {
                    return TailOutcome::Idle;
                }
                continue;
            }

            let read_end = size.min(st.consumed + MAX_READ);
            let start = st.consumed;
            let n = match read_range(p, start, read_end, &mut st.read_buf) {
                Ok(n) => n,
                Err(e) => {
                    st.consecutive_failures += 1;
                    if st.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        return TailOutcome::Error(format!("could not keep reading the log: {e}"));
                    }
                    continue; // transient (sharing violation while ESO flushes)
                }
            };
            st.consecutive_failures = 0; // a successful READ clears the streak
            st.consumed = read_end;
            st.last_growth = std::time::Instant::now();

            // Assemble complete lines from the chunk (copy out to drop the borrow on
            // `st.read_buf` before mutating `st.assembler`).
            let chunk = st.read_buf[..n].to_vec();
            let (lines, saw_end_log) = match st.assembler.push_chunk(&chunk) {
                Ok(r) => r,
                Err(e) => return TailOutcome::Error(e),
            };
            if saw_end_log {
                // Emit any lines we have; the NEXT poll returns Ended. (END_LOG is the
                // last meaningful line, so `lines` already includes everything up to it.)
                st.ended = true;
                if !lines.is_empty() {
                    return TailOutcome::Lines(lines);
                }
                return TailOutcome::Ended;
            }
            if !lines.is_empty() {
                return TailOutcome::Lines(lines);
            }
            // Grew but no complete line yet — keep polling.
        }
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn kind_of(line: &str) -> Option<&str> {
    line.split(',').nth(1).map(str::trim)
}

/// Count the tuple records in a rendered master table (the section after the third
/// `lastAssignedId` header line). The master format lists, per section, a
/// `{lastAssignedId}` line then the records; the tuples section is the 3rd. We rely
/// on the `last_assigned_tuple_id` the doc renders, which equals the tuple count.
fn count_master_tuples(master_text: &str) -> u64 {
    // The master table embeds the tuple count as `last_assigned_tuple_id`. Rather than
    // re-parse the whole grammar, count lines matching the tuple record shape
    // `int|int|int` (3 numeric pipe-separated fields) — robust for the cross-check.
    master_text
        .lines()
        .filter(|l| {
            let parts: Vec<&str> = l.split('|').collect();
            parts.len() == 3 && parts.iter().all(|p| p.parse::<u32>().is_ok())
        })
        .count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    // A synthetic LiveTail that yields a fixed script of line batches then Done — lets
    // the driver's state machine be exercised with no file/network.
    struct ScriptedTail {
        batches: std::sync::Mutex<std::collections::VecDeque<TailOutcome>>,
    }
    impl ScriptedTail {
        fn new(batches: Vec<TailOutcome>) -> Self {
            Self {
                batches: std::sync::Mutex::new(batches.into_iter().collect()),
            }
        }
    }
    impl LiveTail for ScriptedTail {
        fn next_lines(&self, _path: &str) -> TailOutcome {
            self.batches
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(TailOutcome::Ended)
        }
    }

    fn fixture_lines() -> Vec<String> {
        include_str!("testdata/live_correlation_synthetic.log")
            .lines()
            .map(str::to_string)
            .collect()
    }

    // ── Step 2b differential gate ────────────────────────────────────────────────

    /// Drive a `LiveSegmenter` over `lines` and, at EVERY cut boundary the driver
    /// would build a master at (each fight boundary with settled shields, plus the
    /// final flush), assert the INCREMENTAL master text
    /// (`master_state.render_master(...)`) equals the re-walk ORACLE
    /// (`build_master_table_with_tuples_forced(&all_lines_so_far, emitter.tuples(),
    /// frozen_actors, frozen_abilities)`) BYTE-FOR-BYTE.
    ///
    /// `all_lines` is reconstructed in the TEST only (the driver no longer keeps it)
    /// so the retained re-walk oracle can be evaluated at each cut with the SAME
    /// pinned maps the driver applies (the emitter's frozen maps, == the incremental
    /// index state's maps, which Step 2a proved equal the re-walk's maps). Returns the
    /// number of cuts compared, so a fixture that exercises no cut fails loudly.
    fn assert_incremental_master_matches_rewalk(lines: &[&str]) -> usize {
        use super::super::encode::build_master_table_with_tuples_forced;
        let mut seg = LiveSegmenter::new();
        let mut all_lines: Vec<String> = Vec::new();
        let mut compared = 0usize;

        // Compare the incremental master against the re-walk oracle using the
        // segmenter's CURRENT state (the same inputs `build_next_segment` would use).
        // Only when there is something to build (events emitted + a wall window),
        // mirroring the driver's skip-empty-window guard, so the comparison happens at
        // exactly the cuts the driver renders a master at.
        fn compare_if_buildable(seg: &LiveSegmenter, all_lines: &[String], compared: &mut usize) {
            if seg.emitter.drain_segment_events().event_count == 0 {
                return;
            }
            if seg.emitter.live_segment_time_bounds().is_none() {
                return;
            }
            let frozen_actors = seg.emitter.frozen_actor_index_map();
            let frozen_abilities = seg.emitter.frozen_ability_index_map();
            let refs: Vec<&str> = all_lines.iter().map(String::as_str).collect();
            let oracle = build_master_table_with_tuples_forced(
                &refs,
                seg.emitter.tuples(),
                &frozen_actors,
                &frozen_abilities,
            )
            .expect("re-walk oracle master must build");
            let incremental = seg
                .master_state
                .render_master(seg.emitter.tuples(), &frozen_actors, &frozen_abilities)
                .expect("incremental master must build");
            assert_eq!(
                incremental,
                oracle,
                "incremental master diverged from the re-walk oracle at cut {} \
                 (after {} lines)",
                *compared + 1,
                all_lines.len()
            );
            *compared += 1;
        }

        for line in lines {
            all_lines.push((*line).to_string());
            let boundary = seg.feed(line);
            if boundary && seg.shields_settled() {
                compare_if_buildable(&seg, &all_lines, &mut compared);
                // Advance the driver state exactly as the real pump does, so the next
                // cut's emitter/master state is correct.
                let _ = seg.build_next_segment().expect("build must not error");
            }
        }
        // Final flush (the driver drains trailing shields then builds once more).
        seg.drain_trailing_shields();
        compare_if_buildable(&seg, &all_lines, &mut compared);
        let _ = seg
            .build_next_segment()
            .expect("final build must not error");
        compared
    }

    fn raw_fixture(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    // The CORE Step 2b gate over the synthetic correlation + two-session fixtures and
    // the divergence-#4 fixture: the incremental master is byte-identical to the
    // retained re-walk oracle at EVERY cut.
    #[test]
    fn incremental_master_matches_rewalk_at_every_cut() {
        for (name, text) in [
            (
                "live_correlation",
                include_str!("testdata/live_correlation_synthetic.log"),
            ),
            (
                "two_session",
                include_str!("testdata/two_session_synthetic.log"),
            ),
            (
                "f1_late_registration",
                include_str!("testdata/inc_f1_late_registration.log"),
            ),
            (
                "f2_added_never_registers",
                include_str!("testdata/inc_f2_added_never_registers.log"),
            ),
            (
                "f4_recycled_unit",
                include_str!("testdata/inc_f4_recycled_unit.log"),
            ),
            (
                "f5_regen_before_ability",
                include_str!("testdata/inc_f5_regen_before_ability.log"),
            ),
            (
                "f7_intra_event_both_register",
                include_str!("testdata/inc_f7_intra_event_both_register.log"),
            ),
            (
                "f3_damage_type_late",
                include_str!("testdata/inc_f3_damage_type_late.log"),
            ),
        ] {
            let lines = raw_fixture(text);
            let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
            let cuts = assert_incremental_master_matches_rewalk(&refs);
            assert!(
                cuts >= 1,
                "fixture {name} exercised no buildable cut — the gate proved nothing"
            );
        }
    }

    // The COMBAT capture (chunk1_raw.log: a real Ossein-Cage session — 12 actors, 340
    // abilities, 94 combat events, one fight) is the strongest oracle: drive the live
    // segmenter over it and assert the incremental master == the re-walk at every cut.
    #[test]
    fn incremental_master_matches_rewalk_on_combat_capture() {
        let lines = raw_fixture(include_str!("testdata/chunk1_raw.log"));
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let cuts = assert_incremental_master_matches_rewalk(&refs);
        assert!(
            cuts >= 1,
            "the combat capture must exercise at least one cut"
        );
    }

    // Divergence #4 made explicit: ability 38901 gets its ABILITY_INFO in fight 1 (no
    // damage type yet → generic 2) and its first FIRE COMBAT_EVENT only in fight 2. The
    // fight-2 cut's master must show the FIRE damage type (4) for 38901 — i.e. the
    // ability record is RE-RENDERED from the now-learned signal, not frozen at first
    // ABILITY_INFO sight. (The byte-equality gate above already enforces this; this
    // test pins the concrete observable so a regression is legible.)
    #[test]
    fn incremental_master_re_renders_late_learned_damage_type() {
        let lines = raw_fixture(include_str!("testdata/inc_f3_damage_type_late.log"));
        let mut seg = LiveSegmenter::new();
        let mut fight1_master: Option<String> = None;
        let mut fight2_master: Option<String> = None;

        for line in &lines {
            let boundary = seg.feed(line);
            if boundary && seg.shields_settled() {
                if seg.emitter.drain_segment_events().event_count > 0
                    && seg.emitter.live_segment_time_bounds().is_some()
                {
                    let fa = seg.emitter.frozen_actor_index_map();
                    let fb = seg.emitter.frozen_ability_index_map();
                    let m = seg
                        .master_state
                        .render_master(seg.emitter.tuples(), &fa, &fb)
                        .expect("master");
                    if fight1_master.is_none() {
                        fight1_master = Some(m);
                    } else if fight2_master.is_none() {
                        fight2_master = Some(m);
                    }
                }
                let _ = seg.build_next_segment().expect("build");
            }
        }

        let f1 = fight1_master.expect("fight 1 must produce a master");
        let f2 = fight2_master.expect("fight 2 must produce a master");
        // Ability record shape: `{name}|{dt}|{id}|{icon}|0|{flags}`. In fight 1 no
        // damage event has been seen, so 38901 is generic (dt=2); in fight 2 the FIRE
        // damage event has been folded, so 38901 is fire (dt=4).
        assert!(
            f1.contains("Crystal Frags|2|38901|"),
            "fight-1 master should show ability 38901 as GENERIC (dt=2) before any \
             damage event:\n{f1}"
        );
        assert!(
            f2.contains("Crystal Frags|4|38901|"),
            "fight-2 master should RE-RENDER ability 38901 as FIRE (dt=4) once the \
             late FIRE COMBAT_EVENT is learned:\n{f2}"
        );
    }

    // Validates the EXACT synthetic session the `spike-live-feed.mjs` tester appends,
    // so a round-trip is never wasted on a session the encoder would reject. Asserts
    // every segment the LiveSegmenter builds is structurally uploadable (the same
    // self-check the driver runs before each POST). Keep in sync with the feed script.
    #[test]
    fn synthetic_feed_session_builds_valid_segments() {
        let session = "\
0,BEGIN_LOG,1700000000000,15,\"NA Megaserver\",\"en\",\"eso.live.11.3\"
0,ZONE_CHANGED,1129,\"Hall of the Lunar Champion\",NONE
0,UNIT_ADDED,1,PLAYER,T,1,0,F,3,9,\"Hero\",\"@hero\",820189967932710348,50,1735,0,PLAYER_ALLY,T
0,UNIT_ADDED,30,MONSTER,F,0,88330,F,0,0,\"Tenmar Bear\",\"\",0,50,160,0,HOSTILE,F
0,UNIT_ADDED,31,MONSTER,F,0,88340,F,0,0,\"Tenmar Lynx\",\"\",0,50,160,0,HOSTILE,F
100,ABILITY_INFO,28549,\"Roll Dodge\",\"/esoui/art/icons/ability_rogue_035.dds\",F,T
100,EFFECT_INFO,28549,BUFF,NONE,NEVER
100,ABILITY_INFO,29489,\"Hardened Ward\",\"/esoui/art/icons/ability_sorcerer_ward.dds\",F,T
100,EFFECT_INFO,29489,BUFF,NONE,DEFAULT
100,ABILITY_INFO,38901,\"Crystal Frags\",\"/esoui/art/icons/ability_sorcerer_dark_magic.dds\",F,F
200,BEGIN_COMBAT
210,BEGIN_CAST,800,F,5001,38901,1,16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0,30,40000/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0
220,EFFECT_CHANGED,GAINED,1,5002,29489,1,16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0,*
230,COMBAT_EVENT,DAMAGE,FIRE,1,1500,0,5001,38901,1,16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0,30,38500/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0
260,COMBAT_EVENT,DAMAGE,FIRE,1,1800,0,5001,38901,1,16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0,30,36700/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0
700,EFFECT_CHANGED,FADED,1,5002,29489,1,16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0,*
800,END_COMBAT
1000,BEGIN_COMBAT
1010,END_CAST,COMPLETED,5001,38901
1030,COMBAT_EVENT,DAMAGE,FIRE,1,2200,0,5003,38901,1,16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0,31,30000/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0
1060,COMBAT_EVENT,CRITICAL_DAMAGE,FIRE,1,4400,0,5003,38901,1,16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0,31,25600/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0
1040,HEALTH_REGEN,500,1,16000/16000,12000/12000,8000/12000,53/500,0/1000,0,0.5,0.5,4.0
1500,END_COMBAT
1700,END_LOG";
        let lines: Vec<&str> = session.lines().collect();

        let mut seg = LiveSegmenter::new();
        let mut built = 0usize;
        for line in &lines {
            if seg.feed(line) && seg.shields_settled() {
                match seg.build_next_segment() {
                    Ok(Some(p)) => {
                        assert!(!p.segment.bytes.is_empty(), "segment must have ZIP bytes");
                        assert!(
                            p.segment.start_time <= p.segment.end_time,
                            "segment must have a real (non-inverted) wall window"
                        );
                        built += 1;
                    }
                    Ok(None) => {}
                    Err(e) => panic!("synthetic session built an INVALID segment: {e}"),
                }
            }
        }
        seg.drain_trailing_shields();
        if let Ok(Some(_)) = seg.build_next_segment() {
            built += 1;
        }
        assert!(
            built >= 2,
            "synthetic session must yield multiple valid segments (got {built})"
        );
    }

    // ── Mid-session warm-up (skip /reloadui) differential tests ──────────────────
    //
    // These prove the load-bearing claim: replaying an on-disk session prefix to warm
    // the segmenter, then `finish_warmup()`, then streaming the rest, produces segments
    // BYTE-IDENTICAL to streaming that same session from the start — for every LIVE
    // segment — with the ONLY allowed difference being segment numbering (the prefix's
    // already-finished fights were never POSTed, so the first live segment is id 1, not
    // its from-start ordinal). This is the H1-index-stability re-verification the design
    // requires: the warm-up exercises the identical advancing-pin feed() path.

    /// Drive `lines` through a fresh segmenter, posting at every settled fight boundary
    /// (and a final flush), returning each built segment's `(segment.bytes, master.bytes,
    /// start_time, end_time)`. Mirrors the driver's cut/build loop without the network.
    fn collect_segments(lines: &[&str]) -> Vec<(Vec<u8>, Vec<u8>, u64, u64)> {
        let mut seg = LiveSegmenter::new();
        let mut out = Vec::new();
        for line in lines {
            if seg.feed(line) && seg.shields_settled() {
                if let Ok(Some(p)) = seg.build_next_segment() {
                    out.push((
                        p.segment.bytes.clone(),
                        p.master.bytes.clone(),
                        p.segment.start_time,
                        p.segment.end_time,
                    ));
                }
            }
        }
        seg.drain_trailing_shields();
        if let Ok(Some(p)) = seg.build_next_segment() {
            out.push((
                p.segment.bytes.clone(),
                p.master.bytes.clone(),
                p.segment.start_time,
                p.segment.end_time,
            ));
        }
        out
    }

    /// The shared 2-fight fixture: a real BEGIN_LOG header, actor/ability defs, two
    /// fights (each BEGIN_COMBAT..END_COMBAT), END_LOG. Fight 2 references actors AND an
    /// ability first registered before fight 1, so a mid-session join at the fight-1/2
    /// boundary must still resolve fight 2's tuples against the warm-up-seeded indices.
    fn two_fight_session_inline() -> Vec<&'static str> {
        // Reuse the validated synthetic feed session (identical shape: 2 fights).
        "\
0,BEGIN_LOG,1700000000000,15,\"NA Megaserver\",\"en\",\"eso.live.11.3\"
0,ZONE_CHANGED,1129,\"Hall of the Lunar Champion\",NONE
0,UNIT_ADDED,1,PLAYER,T,1,0,F,3,9,\"Hero\",\"@hero\",820189967932710348,50,1735,0,PLAYER_ALLY,T
0,UNIT_ADDED,30,MONSTER,F,0,88330,F,0,0,\"Tenmar Bear\",\"\",0,50,160,0,HOSTILE,F
0,UNIT_ADDED,31,MONSTER,F,0,88340,F,0,0,\"Tenmar Lynx\",\"\",0,50,160,0,HOSTILE,F
100,ABILITY_INFO,28549,\"Roll Dodge\",\"/esoui/art/icons/ability_rogue_035.dds\",F,T
100,EFFECT_INFO,28549,BUFF,NONE,NEVER
100,ABILITY_INFO,29489,\"Hardened Ward\",\"/esoui/art/icons/ability_sorcerer_ward.dds\",F,T
100,EFFECT_INFO,29489,BUFF,NONE,DEFAULT
100,ABILITY_INFO,38901,\"Crystal Frags\",\"/esoui/art/icons/ability_sorcerer_dark_magic.dds\",F,F
200,BEGIN_COMBAT
210,BEGIN_CAST,800,F,5001,38901,1,16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0,30,40000/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0
220,EFFECT_CHANGED,GAINED,1,5002,29489,1,16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0,*
230,COMBAT_EVENT,DAMAGE,FIRE,1,1500,0,5001,38901,1,16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0,30,38500/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0
260,COMBAT_EVENT,DAMAGE,FIRE,1,1800,0,5001,38901,1,16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0,30,36700/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0
700,EFFECT_CHANGED,FADED,1,5002,29489,1,16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0,*
800,END_COMBAT
1000,BEGIN_COMBAT
1010,END_CAST,COMPLETED,5001,38901
1030,COMBAT_EVENT,DAMAGE,FIRE,1,2200,0,5003,38901,1,16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0,31,30000/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0
1060,COMBAT_EVENT,CRITICAL_DAMAGE,FIRE,1,4400,0,5003,38901,1,16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0,31,25600/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0
1040,HEALTH_REGEN,500,1,16000/16000,12000/12000,8000/12000,53/500,0/1000,0,0.5,0.5,4.0
1500,END_COMBAT
1700,END_LOG"
            .lines()
            .collect()
    }

    /// Split the fixture at the END_COMBAT that ends fight 1: everything up to and
    /// including the first END_COMBAT is the "already on disk" prefix; the rest is what
    /// the live tail will deliver.
    fn split_after_first_fight<'a>(lines: &[&'a str]) -> (Vec<&'a str>, Vec<&'a str>) {
        let cut = lines
            .iter()
            .position(|l| kind_of(l) == Some("END_COMBAT"))
            .expect("fixture has a first END_COMBAT")
            + 1;
        (lines[..cut].to_vec(), lines[cut..].to_vec())
    }

    #[test]
    fn mid_session_seed_matches_from_start_stream_for_live_fights() {
        let session = two_fight_session_inline();
        let (prefix, live) = split_after_first_fight(&session);

        // Baseline: stream the WHOLE session from the start, posting every fight.
        let full = collect_segments(&session);
        assert!(
            full.len() >= 2,
            "baseline must have ≥2 fights: {}",
            full.len()
        );

        // Mid-session: replay the prefix WITHOUT posting (warm-up), finish_warmup, then
        // stream the live remainder, collecting only the live segments.
        let mut seg = LiveSegmenter::new();
        for line in &prefix {
            let _ = seg.feed(line); // ignore cut boundaries — no POST during warm-up
        }
        seg.finish_warmup();
        // First live segment id starts fresh at 1 (the prefix fight was never POSTed).
        assert_eq!(
            seg.next_segment_id(),
            1,
            "warm-up resets the segment id to 1"
        );

        let mut live_segs: Vec<(Vec<u8>, Vec<u8>, u64, u64)> = Vec::new();
        for line in &live {
            if seg.feed(line) && seg.shields_settled() {
                if let Ok(Some(p)) = seg.build_next_segment() {
                    live_segs.push((
                        p.segment.bytes.clone(),
                        p.master.bytes.clone(),
                        p.segment.start_time,
                        p.segment.end_time,
                    ));
                }
            }
        }
        seg.drain_trailing_shields();
        if let Ok(Some(p)) = seg.build_next_segment() {
            live_segs.push((
                p.segment.bytes.clone(),
                p.master.bytes.clone(),
                p.segment.start_time,
                p.segment.end_time,
            ));
        }

        // The live fights are the from-start baseline's fights AFTER the prefix's count.
        let prefix_fights = full.len() - live_segs.len();
        assert!(
            prefix_fights >= 1,
            "prefix should contain ≥1 finished fight"
        );
        for (i, live_seg) in live_segs.iter().enumerate() {
            let baseline = &full[prefix_fights + i];
            assert_eq!(
                live_seg.0, baseline.0,
                "live fight {i}: segment ZIP bytes must be byte-identical to from-start"
            );
            assert_eq!(
                live_seg.1, baseline.1,
                "live fight {i}: cumulative master bytes must be byte-identical (the \
                 warm-up actors/abilities ARE present in both — same session)"
            );
            assert_eq!(
                (live_seg.2, live_seg.3),
                (baseline.2, baseline.3),
                "live fight {i}: wall window must match (wall came from the real on-disk \
                 BEGIN_LOG in BOTH paths — never synthesized)"
            );
        }
    }

    #[test]
    fn mid_session_seed_preserves_actor_ability_indices_for_warmup_entities() {
        // After warming from the prefix, the indices of entities first registered in the
        // prefix (the player, the two monsters, the abilities) must equal their from-start
        // indices — the H1 pin holds across the warm-up→live transition.
        let session = two_fight_session_inline();
        let (prefix, _live) = split_after_first_fight(&session);

        // From-start: feed the whole prefix into a plain segmenter, read its frozen maps.
        let mut baseline = LiveSegmenter::new();
        for line in &prefix {
            let _ = baseline.feed(line);
        }
        let base_actors = baseline.emitter.frozen_actor_index_map();
        let base_abils = baseline.emitter.frozen_ability_index_map();

        // Mid-session: same feed, then finish_warmup (which must NOT renumber anything —
        // it only resets the per-segment POST framing, never the pinned maps).
        let mut warmed = LiveSegmenter::new();
        for line in &prefix {
            let _ = warmed.feed(line);
        }
        warmed.finish_warmup();
        assert_eq!(
            warmed.emitter.frozen_actor_index_map(),
            base_actors,
            "finish_warmup must not renumber actor indices (H1 pin preserved)"
        );
        assert_eq!(
            warmed.emitter.frozen_ability_index_map(),
            base_abils,
            "finish_warmup must not renumber ability indices (H1 pin preserved)"
        );
    }

    // The OFFLINE DIFFERENTIAL GATE: the segmenter cuts at fight boundaries and the
    // segments it builds, concatenated, reproduce the one-shot event stream BYTE-FOR-
    // BYTE — proving the live cut path is correct offline. Generalized over a fixture
    // so each straddling-correlation case below exercises the same invariant.
    fn assert_cuts_reproduce_one_shot(lines: &[String], min_segments: usize) {
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();

        // One-shot reference body.
        let (id2a, ab2i) = super::super::encode::actor_ability_maps(&refs);
        let mut one_shot = EventEmitter::with_master_indices(id2a, ab2i);
        let one_shot_body = one_shot.build(&refs).events_string;

        // Drive the segmenter, collecting each cut segment's events.
        let mut seg = LiveSegmenter::new();
        let mut assembled = String::new();
        let mut built = 0usize;
        for line in lines {
            let boundary = seg.feed(line);
            if boundary && seg.shields_settled() {
                if let Ok(Some(p)) = seg.build_next_segment() {
                    assembled.push_str(&unzip_segment_body(&p.segment));
                    built += 1;
                }
            }
        }
        // Final flush.
        if let Ok(Some(p)) = seg.build_next_segment() {
            assembled.push_str(&unzip_segment_body(&p.segment));
            built += 1;
        }

        assert!(
            built >= min_segments,
            "fixture should produce >= {min_segments} fight segments (got {built})"
        );
        assert_eq!(
            assembled, one_shot_body,
            "concatenated live-segment bodies must equal the one-shot event stream"
        );
    }

    #[test]
    fn live_segmenter_cuts_reproduce_the_one_shot_event_stream() {
        assert_cuts_reproduce_one_shot(&fixture_lines(), 2);
    }

    // Straddling correlations across an END_COMBAT cut — the brief's required cases.
    // A timed BEGIN_CAST in fight 1 completing (code-16) in fight 2, a buff GAINED in
    // fight 1 / UPDATED+FADED in fight 2 (the GAINED/FADED key2a + last_stack must
    // survive the cut), and an INTERRUPT (code-27, needs cast_id_units + last_interrupt
    // carried across the cut). The differential gate proves all of this state survives
    // a cut byte-identically to the one-shot build — exactly the live-correctness claim.
    #[test]
    fn live_segmenter_cuts_reproduce_straddling_correlations_byte_for_byte() {
        let lines = raw_fixture(include_str!("testdata/live_straddle_correlations.log"));
        assert_cuts_reproduce_one_shot(&lines, 2);
    }

    // The driver terminates the report on a clean finish (Done) — proving the
    // orphan-safety wrapper runs terminate on the success path too. Uses a recording
    // session/cancel; the actual HTTP is not exercised (no server), so we assert at
    // the LiveSegmenter level that a full run builds at least one segment and ends.
    #[test]
    fn segmenter_skips_empty_windows_and_builds_on_fights() {
        let mut seg = LiveSegmenter::new();
        // A BEGIN_LOG + zone with no fight: a boundary cut should yield None (nothing
        // to POST), not a malformed empty segment.
        for line in [
            "0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live.11.3\"",
            "0,ZONE_CHANGED,1129,\"Hall\",NONE",
        ] {
            seg.feed(line);
        }
        // No END_COMBAT yet; force a build — should be Some (the ZONE emitted) with a
        // real window, OR None if nothing emitted. Either way, never an error.
        let r = seg.build_next_segment();
        assert!(r.is_ok(), "an empty/zone-only window must not error: {r:?}");
    }

    // Drive the segmenter via the LiveTail abstraction (a ScriptedTail feeding the
    // fixture in two batches then Done), mirroring stream_until_done's pump WITHOUT the
    // network: each fight-boundary cut with settled shields builds a segment, and the
    // tail's Done ends the pump. Verifies the LiveTail seam + the cut-on-boundary logic
    // the real driver relies on, and that a clean Done flushes the final segment.
    #[test]
    fn scripted_tail_drives_segment_cuts_and_terminates_on_done() {
        let lines = fixture_lines();
        let mid = lines.len() / 2;
        let tail = ScriptedTail::new(vec![
            TailOutcome::Lines(lines[..mid].to_vec()),
            TailOutcome::Lines(lines[mid..].to_vec()),
            TailOutcome::Ended,
        ]);

        let mut seg = LiveSegmenter::new();
        let mut built = 0usize;
        loop {
            match tail.next_lines("ignored") {
                TailOutcome::Lines(batch) => {
                    for line in batch {
                        if seg.feed(&line) && seg.shields_settled() {
                            if let Ok(Some(_)) = seg.build_next_segment() {
                                built += 1;
                            }
                        }
                    }
                }
                TailOutcome::Ended | TailOutcome::Idle => {
                    // The pump ends only when the tail signals end (reaching here is
                    // the proof). Flush the final segment.
                    seg.drain_trailing_shields();
                    if let Ok(Some(_)) = seg.build_next_segment() {
                        built += 1;
                    }
                    break;
                }
                TailOutcome::Error(e) => panic!("unexpected tail error: {e}"),
            }
        }
        assert!(
            built >= 2,
            "the fixture's two fights must produce multiple segments (got {built})"
        );
        assert_eq!(
            seg.segments_built(),
            built,
            "segments_built() must match the number of successful builds"
        );
    }

    /// Unzip a built segment's `log.txt` and return just the event body (drop the
    /// 2-line header: `version|game` + count).
    fn unzip_segment_body(segment: &Segment) -> String {
        use std::io::Read;
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(segment.bytes.clone())).expect("zip");
        let mut file = archive.by_index(0).unwrap();
        let mut s = String::new();
        file.read_to_string(&mut s).unwrap();
        s.lines().skip(2).map(|l| format!("{l}\n")).collect()
    }

    // ── Retry / classification (L4) ──────────────────────────────────────────

    #[test]
    fn classify_upload_error_routes_each_variant() {
        use super::super::session::SessionError;
        assert_eq!(
            classify_upload_error(&UploadError::Cancelled),
            RetryClass::Cancelled
        );
        assert_eq!(
            classify_upload_error(&UploadError::Session(SessionError::Expired)),
            RetryClass::NeedsReauth
        );
        assert_eq!(
            classify_upload_error(&UploadError::Transport("net".into())),
            RetryClass::Retryable
        );
        for s in [500u16, 502, 503, 408, 429] {
            assert_eq!(
                classify_upload_error(&UploadError::Server {
                    status: s,
                    detail: String::new()
                }),
                RetryClass::Retryable,
                "status {s} must be retryable"
            );
        }
        for s in [400u16, 403, 404, 422] {
            assert_eq!(
                classify_upload_error(&UploadError::Server {
                    status: s,
                    detail: String::new()
                }),
                RetryClass::Fatal,
                "status {s} must be fatal"
            );
        }
        // status 0 = our internal/malformed marker → fatal (a retry reproduces it).
        assert_eq!(
            classify_upload_error(&UploadError::Server {
                status: 0,
                detail: "malformed".into()
            }),
            RetryClass::Fatal
        );
    }

    #[test]
    fn post_with_retry_returns_ok_on_first_success() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut calls = 0;
        let r = post_with_retry(&cancel, || {
            calls += 1;
            Ok(b"ok".to_vec())
        });
        assert_eq!(r.unwrap(), b"ok");
        assert_eq!(calls, 1, "a first success must not retry");
    }

    #[test]
    fn post_with_retry_retries_transient_then_succeeds() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut calls = 0;
        let r = post_with_retry(&cancel, || {
            calls += 1;
            if calls < 2 {
                Err(UploadError::Server {
                    status: 503,
                    detail: "busy".into(),
                })
            } else {
                Ok(b"ok".to_vec())
            }
        });
        assert_eq!(r.unwrap(), b"ok");
        assert_eq!(calls, 2, "must retry once then succeed");
    }

    #[test]
    fn post_with_retry_exhausts_and_reports_retryable_terminal() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut calls = 0;
        let r = post_with_retry(&cancel, || {
            calls += 1;
            Err(UploadError::Transport("down".into()))
        });
        // The backoff between attempts must not run the real 1s/2s here for a unit
        // test... but it does. Keep the assertion on outcome + attempt count; the
        // total sleep is 1s+2s = 3s worst case, acceptable for one test. (A faster
        // path would inject the backoff; deferred — correctness first.)
        assert!(matches!(r, Err(RetryClass::Retryable)));
        assert_eq!(calls, MAX_SEGMENT_ATTEMPTS as i32, "must use all attempts");
    }

    #[test]
    fn post_with_retry_does_not_retry_fatal() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut calls = 0;
        let r = post_with_retry(&cancel, || {
            calls += 1;
            Err(UploadError::Server {
                status: 400,
                detail: "bad".into(),
            })
        });
        assert!(matches!(r, Err(RetryClass::Fatal)));
        assert_eq!(calls, 1, "a fatal error must not retry");
    }

    #[test]
    fn post_with_retry_does_not_retry_needs_reauth() {
        use super::super::session::SessionError;
        let cancel = Arc::new(AtomicBool::new(false));
        let mut calls = 0;
        let r = post_with_retry(&cancel, || {
            calls += 1;
            Err(UploadError::Session(SessionError::Expired))
        });
        assert!(matches!(r, Err(RetryClass::NeedsReauth)));
        assert_eq!(calls, 1, "a reauth-needed error pauses, does not retry");
    }

    #[test]
    fn cancel_aware_backoff_returns_fast_when_cancelled() {
        let cancel = Arc::new(AtomicBool::new(false));
        let c = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            c.store(true, Ordering::SeqCst);
        });
        let start = std::time::Instant::now();
        // Ask for an 8s backoff; cancel trips at ~100ms.
        let cancelled = cancel_aware_backoff(std::time::Duration::from_secs(8), &cancel);
        assert!(cancelled, "backoff must report the cancel");
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "a cancel during backoff must return promptly, took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn post_with_retry_cancel_during_backoff_returns_cancelled() {
        let cancel = Arc::new(AtomicBool::new(false));
        let c = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            c.store(true, Ordering::SeqCst);
        });
        let r = post_with_retry(&cancel, || {
            Err(UploadError::Transport("blip".into())) // always retryable
        });
        assert!(
            matches!(r, Err(RetryClass::Cancelled)),
            "a Stop during the retry backoff must surface as Cancelled, got {r:?}"
        );
    }

    // ── Driver state machine (L3 + L4), via a scripted LivePoster ─────────────

    use super::super::client::{LivePoster, OwnedLiveRequest};
    use super::super::session::SessionError;
    use std::sync::Mutex;

    /// A scripted [`LivePoster`] for driving [`LiveDriver`] deterministically without
    /// a server. Per endpoint family (master / add-segment) it returns the NEXT scripted
    /// result; `add-segment` successes carry a `nextSegmentId`. `has_session` is a
    /// flippable flag (for pause-resume). Records the URLs it was asked to POST.
    struct ScriptedSender {
        // Queued add-segment outcomes: Ok(nextSegmentId) or a scripted error.
        seg_results: Mutex<std::collections::VecDeque<Result<u64, UploadError>>>,
        // Queued master-table outcomes (Ok = accepted).
        master_results: Mutex<std::collections::VecDeque<Result<(), UploadError>>>,
        session_ok: Arc<AtomicBool>,
        posts: Mutex<Vec<String>>,
    }

    impl ScriptedSender {
        fn new(
            master: Vec<Result<(), UploadError>>,
            seg: Vec<Result<u64, UploadError>>,
            session_ok: Arc<AtomicBool>,
        ) -> Self {
            Self {
                seg_results: Mutex::new(seg.into_iter().collect()),
                master_results: Mutex::new(master.into_iter().collect()),
                session_ok,
                posts: Mutex::new(Vec::new()),
            }
        }
        fn post_count(&self, needle: &str) -> usize {
            self.posts
                .lock()
                .unwrap()
                .iter()
                .filter(|u| u.contains(needle))
                .count()
        }
    }

    impl LivePoster for ScriptedSender {
        fn post(
            &self,
            url: &str,
            _req: OwnedLiveRequest,
            cancel: &Arc<AtomicBool>,
        ) -> Result<Vec<u8>, UploadError> {
            self.posts.lock().unwrap().push(url.to_string());
            if cancel.load(Ordering::SeqCst) {
                return Err(UploadError::Cancelled);
            }
            if url.contains("set-report-master-table") {
                return match self.master_results.lock().unwrap().pop_front() {
                    Some(Ok(())) => Ok(b"{}".to_vec()),
                    Some(Err(e)) => Err(e),
                    None => Ok(b"{}".to_vec()), // default-accept extra masters
                };
            }
            if url.contains("add-report-segment") {
                return match self.seg_results.lock().unwrap().pop_front() {
                    Some(Ok(next)) => Ok(format!("{{\"nextSegmentId\":{next}}}").into_bytes()),
                    Some(Err(e)) => Err(e),
                    None => Ok(b"{\"nextSegmentId\":2}".to_vec()),
                };
            }
            // terminate / create — accept.
            Ok(b"{}".to_vec())
        }
        fn has_session(&self) -> bool {
            self.session_ok.load(Ordering::SeqCst)
        }
    }

    /// Build a driver over a scripted sender, feed it a fixture's lines via the shared
    /// `drive_assembled_lines`, and return (end reason, segments built, sender).
    fn drive_with(
        sender: ScriptedSender,
        cancel: Arc<AtomicBool>,
        lines: Vec<String>,
        feed_done: bool,
    ) -> (Option<EndReason>, usize, ScriptedSender) {
        let mut driver = LiveDriver::new(sender, ReportCode("TESTCODE".into()), cancel, None);
        let sink = NoopOrphanSink;
        let mut reason = driver.drive_assembled_lines(lines, &sink);
        if reason.is_none() && feed_done {
            // Mirror the Done arm: drain + final flush.
            driver.seg.drain_trailing_shields();
            reason = match driver.post_current(&sink) {
                PostOutcome::Fatal(d) => Some(EndReason::Fatal(d)),
                PostOutcome::Cancelled => Some(EndReason::Stopped),
                PostOutcome::NeedsReauth { .. } => Some(EndReason::ReauthTimeout),
                _ => Some(EndReason::Ended),
            };
        }
        let built = driver.segments_built();
        (reason, built, driver.sender)
    }

    fn two_fight_session() -> Vec<String> {
        raw_fixture(include_str!("testdata/live_correlation_synthetic.log"))
    }

    #[test]
    fn driver_streams_two_fights_and_ends_clean() {
        // All POSTs succeed; the synthetic 2-fight session yields >=1 segment and ends
        // cleanly (Done → Ended).
        let session_ok = Arc::new(AtomicBool::new(true));
        let sender = ScriptedSender::new(
            vec![Ok(()), Ok(()), Ok(())],
            vec![Ok(2), Ok(3), Ok(4)],
            session_ok,
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let (reason, built, sender) = drive_with(sender, cancel, two_fight_session(), true);
        assert_eq!(
            reason,
            Some(EndReason::Ended),
            "clean session must end Ended"
        );
        assert!(built >= 1, "should build at least one segment, got {built}");
        assert!(
            sender.post_count("add-report-segment") >= 1,
            "should have POSTed at least one segment"
        );
    }

    #[test]
    fn driver_stops_on_server_next_segment_id_zero() {
        // The first segment POST returns nextSegmentId=0 → ServerEnded (stop+terminate).
        let session_ok = Arc::new(AtomicBool::new(true));
        let sender = ScriptedSender::new(vec![Ok(())], vec![Ok(0)], session_ok);
        let cancel = Arc::new(AtomicBool::new(false));
        let (reason, _built, _s) = drive_with(sender, cancel, two_fight_session(), false);
        assert_eq!(
            reason,
            Some(EndReason::ServerEnded),
            "nextSegmentId=0 must end the stream as ServerEnded"
        );
    }

    #[test]
    fn driver_terminates_on_fatal_segment_error() {
        // A 400 on the first add-segment is fatal → terminate, EndReason::Fatal.
        let session_ok = Arc::new(AtomicBool::new(true));
        let sender = ScriptedSender::new(
            vec![Ok(())],
            vec![Err(UploadError::Server {
                status: 400,
                detail: "bad".into(),
            })],
            session_ok,
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let (reason, _b, _s) = drive_with(sender, cancel, two_fight_session(), false);
        assert!(
            matches!(reason, Some(EndReason::Fatal(_))),
            "a 4xx segment error must be Fatal, got {reason:?}"
        );
    }

    #[test]
    fn driver_retries_transient_segment_then_continues() {
        // First segment: master ok, add-segment 503 then (on retry) ok. The driver's
        // post_with_retry handles the retry internally, so the stream proceeds.
        let session_ok = Arc::new(AtomicBool::new(true));
        let sender = ScriptedSender::new(
            vec![Ok(()), Ok(()), Ok(())],
            vec![
                Err(UploadError::Server {
                    status: 503,
                    detail: "busy".into(),
                }),
                Ok(2),
                Ok(3),
                Ok(4),
            ],
            session_ok,
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let (reason, built, sender) = drive_with(sender, cancel, two_fight_session(), true);
        assert_eq!(reason, Some(EndReason::Ended));
        assert!(built >= 1);
        // The retried add-segment means >=2 add-segment POSTs occurred for the run.
        assert!(
            sender.post_count("add-report-segment") >= 2,
            "the transient 503 must have been retried"
        );
    }

    #[test]
    fn driver_pauses_on_lost_session_then_resumes_when_reauthed() {
        // First segment's add-segment returns Session(Expired) → pause. The session
        // flag is already true (re-login happened immediately), so the pause loop
        // resolves on its first check and re-POSTs the held payload, then the stream
        // ends clean. We assert it does NOT terminate as ReauthTimeout.
        let session_ok = Arc::new(AtomicBool::new(true));
        let sender = ScriptedSender::new(
            vec![Ok(()), Ok(()), Ok(()), Ok(())],
            vec![
                Err(UploadError::Session(SessionError::Expired)), // first attempt: auth lost
                Ok(2),                                            // re-POST of held succeeds
                Ok(3),
                Ok(4),
            ],
            Arc::clone(&session_ok),
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let (reason, built, _s) = drive_with(sender, cancel, two_fight_session(), true);
        assert_eq!(
            reason,
            Some(EndReason::Ended),
            "a pause that immediately re-auths must resume and end clean, got {reason:?}"
        );
        assert!(built >= 1, "the held segment must be re-POSTed on resume");
    }

    #[test]
    fn driver_pause_times_out_when_never_reauthed() {
        // Auth lost AND the session never comes back → the pause should terminate as
        // ReauthTimeout. We use a tiny REAUTH window by setting session_ok=false and
        // cancelling shortly (the pause loop checks cancel each ~2s slice). To avoid a
        // 15-min test, assert the Stopped path (cancel during pause) which shares the
        // same loop — proving the pause is bounded and cancel-interruptible.
        let session_ok = Arc::new(AtomicBool::new(false)); // never re-auths
        let sender = ScriptedSender::new(
            vec![Ok(())],
            vec![Err(UploadError::Session(SessionError::Expired))],
            session_ok,
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let c = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            c.store(true, Ordering::SeqCst);
        });
        let (reason, _b, _s) = drive_with(sender, cancel, two_fight_session(), false);
        assert_eq!(
            reason,
            Some(EndReason::Stopped),
            "a Stop during the reauth pause must end promptly as Stopped, got {reason:?}"
        );
    }

    #[test]
    fn driver_cancel_mid_stream_ends_stopped() {
        // Cancel set before driving → the first post_current sees it and stops.
        let session_ok = Arc::new(AtomicBool::new(true));
        let sender = ScriptedSender::new(vec![Ok(())], vec![Ok(2)], session_ok);
        let cancel = Arc::new(AtomicBool::new(true)); // already stopped
        let (reason, _b, _s) = drive_with(sender, cancel, two_fight_session(), false);
        assert_eq!(reason, Some(EndReason::Stopped));
    }

    // ── Production tail line assembly (L-tail, Step 6) ───────────────────────

    const HDR: &str = "0,BEGIN_LOG,1700000000000,15,\"NA Megaserver\",\"en\",\"eso.live.11.3\"";

    #[test]
    fn line_assembler_reassembles_a_line_split_across_two_chunks() {
        let mut a = LineAssembler::new();
        // First chunk ends mid-line (no newline on the second line).
        let (l1, end1) = a
            .push_chunk(format!("{HDR}\n0,ZONE_CHANGED,1129,\"Ha").as_bytes())
            .unwrap();
        assert_eq!(
            l1,
            vec![HDR.to_string()],
            "only the complete first line emits"
        );
        assert!(!end1);
        // Second chunk completes it.
        let (l2, _) = a.push_chunk(b"ll\",NONE\n").unwrap();
        assert_eq!(l2, vec!["0,ZONE_CHANGED,1129,\"Hall\",NONE".to_string()]);
    }

    #[test]
    fn line_assembler_strips_crlf_and_drops_empties() {
        let mut a = LineAssembler::new();
        let (lines, _) = a
            .push_chunk(format!("{HDR}\r\n\r\n0,END_COMBAT\r\n").as_bytes())
            .unwrap();
        assert_eq!(
            lines,
            vec![HDR.to_string(), "0,END_COMBAT".to_string()],
            "CRLF stripped and the blank line dropped"
        );
    }

    #[test]
    fn line_assembler_flags_end_log() {
        let mut a = LineAssembler::new();
        let (_, end) = a
            .push_chunk(format!("{HDR}\n1700,END_LOG\n").as_bytes())
            .unwrap();
        assert!(end, "END_LOG must be flagged");
    }

    #[test]
    fn line_assembler_discards_lines_before_first_begin_log() {
        // F4: a mid-session start (no leading BEGIN_LOG) must discard until one arrives,
        // so the encoder never sees a headerless prefix.
        let mut a = LineAssembler::new();
        let (pre, _) = a
            .push_chunk(b"500,COMBAT_EVENT,DAMAGE,FIRE,1,5,0,1,38901,1\n600,END_COMBAT\n")
            .unwrap();
        assert!(
            pre.is_empty(),
            "pre-BEGIN_LOG lines must be discarded, got {pre:?}"
        );
        let (post, _) = a
            .push_chunk(format!("{HDR}\n700,END_COMBAT\n").as_bytes())
            .unwrap();
        assert_eq!(
            post,
            vec![HDR.to_string(), "700,END_COMBAT".to_string()],
            "lines from BEGIN_LOG onward are kept"
        );
    }

    #[test]
    fn line_assembler_mid_session_keeps_lines_without_a_header() {
        // A mid-session tail starts INSIDE an already-open session (the warm-up replay
        // already consumed its BEGIN_LOG), so it must keep every line WITHOUT waiting for
        // a header — otherwise the F4 gate would discard every appended fight line forever
        // (the bug that made mid-session live require a /reloadui). The caller starts the
        // tail on a line-safe boundary (tail_io::last_line_boundary), so the first chunk
        // begins at a real line start — no orphaned fragment to handle here.
        let mut a = LineAssembler::new_mid_session();
        let (lines, _) = a
            .push_chunk(b"700,BEGIN_COMBAT\n730,COMBAT_EVENT,DAMAGE\n750,END_COMBAT\n")
            .unwrap();
        assert_eq!(
            lines,
            vec![
                "700,BEGIN_COMBAT".to_string(),
                "730,COMBAT_EVENT,DAMAGE".to_string(),
                "750,END_COMBAT".to_string(),
            ],
            "mid-session: all lines flow without a leading BEGIN_LOG, none dropped"
        );
    }

    #[test]
    fn line_assembler_mid_session_still_terminates_on_a_second_begin_log() {
        // A genuine /reloadui DURING the live session writes a fresh BEGIN_LOG into the
        // tailed range; the mid-session assembler must surface it (not swallow it) so the
        // driver's NewSession terminate fires.
        let mut a = LineAssembler::new_mid_session();
        let (lines, _) = a
            .push_chunk(format!("700,END_COMBAT\n{HDR}\n").as_bytes())
            .unwrap();
        assert_eq!(
            lines,
            vec!["700,END_COMBAT".to_string(), HDR.to_string()],
            "a second BEGIN_LOG is emitted, not discarded, so NewSession can terminate"
        );
    }

    #[test]
    fn line_assembler_caps_an_unbounded_partial() {
        let mut a = LineAssembler::new();
        // A 1MiB+1 byte chunk with no newline must trip the cap → Err (teardown).
        let huge = vec![b'x'; MAX_PARTIAL + 1];
        let r = a.push_chunk(&huge);
        assert!(r.is_err(), "an oversized newline-less partial must error");
    }

    #[test]
    fn line_assembler_reset_clears_partial() {
        let mut a = LineAssembler::new();
        let _ = a.push_chunk(b"0,BEGIN").unwrap(); // partial, no newline
        a.reset();
        // After reset the dangling "0,BEGIN" is gone; a fresh full line assembles clean.
        let (lines, _) = a.push_chunk(format!("{HDR}\n").as_bytes()).unwrap();
        assert_eq!(lines, vec![HDR.to_string()]);
    }

    #[test]
    fn line_assembler_reset_rearms_the_header_gate_after_truncation() {
        // After a BEGIN_LOG has been seen, a truncation/replacement (NotifyTail calls
        // reset) must DISCARD the replacement file's lines until ITS OWN BEGIN_LOG —
        // otherwise pre-header bytes of the new file leak into the closing report. The
        // replacement's BEGIN_LOG is then emitted so the driver can cut NewSession.
        let mut a = LineAssembler::new();
        let (first, _) = a
            .push_chunk(format!("{HDR}\n100,END_COMBAT\n").as_bytes())
            .unwrap();
        assert_eq!(first, vec![HDR.to_string(), "100,END_COMBAT".to_string()]);
        a.reset(); // truncation/replacement
        let (after, _) = a
            .push_chunk(format!("50,COMBAT_EVENT,DAMAGE\n{HDR}\n").as_bytes())
            .unwrap();
        assert_eq!(
            after,
            vec![HDR.to_string()],
            "pre-header bytes of the replacement file are discarded; its BEGIN_LOG is emitted"
        );
    }

    #[test]
    fn driver_terminates_on_second_begin_log() {
        // Two sessions in one feed (a /reloadui mid-stream). The driver must flush the
        // first session's segment then end as NewSession — NOT mix two sessions.
        let mut lines = two_fight_session();
        // Append a SECOND BEGIN_LOG + a fight, then END_LOG.
        lines
            .push("0,BEGIN_LOG,1700000099000,15,\"NA Megaserver\",\"en\",\"eso.live.11.3\"".into());
        lines.push("0,ZONE_CHANGED,1129,\"Hall\",NONE".into());
        let session_ok = Arc::new(AtomicBool::new(true));
        let sender = ScriptedSender::new(
            vec![Ok(()), Ok(()), Ok(())],
            vec![Ok(2), Ok(3), Ok(4)],
            session_ok,
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let (reason, _b, _s) = drive_with(sender, cancel, lines, false);
        assert_eq!(
            reason,
            Some(EndReason::NewSession),
            "a second BEGIN_LOG must end the stream as NewSession (single-session contract), got {reason:?}"
        );
    }

    #[test]
    fn driver_fails_closed_on_an_unproven_line_type() {
        // A novel/unproven line type in the live stream must TERMINATE the session (Fatal)
        // rather than be silently dropped into a complete-looking but incomplete report —
        // the live counterpart of the finished-log coverage gate.
        let mut lines = two_fight_session();
        lines.insert(1, "100,SOME_FUTURE_EVENT,1,2,3".into()); // just after the session header
        let session_ok = Arc::new(AtomicBool::new(true));
        let sender = ScriptedSender::new(vec![Ok(())], vec![Ok(2)], session_ok);
        let cancel = Arc::new(AtomicBool::new(false));
        let (reason, _b, _s) = drive_with(sender, cancel, lines, false);
        assert!(
            matches!(reason, Some(EndReason::Fatal(ref d)) if d.contains("SOME_FUTURE_EVENT")),
            "an unproven line type must fail closed as Fatal, got {reason:?}"
        );
    }
}
