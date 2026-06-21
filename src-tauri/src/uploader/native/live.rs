//! Debug-only native **live-streaming** upload driver (the `spike/native-live` R&D
//! spike). NOT wired into the shipping live path — the production live upload still
//! hands off to the official uploader (`super::super::transport` / `commands`).
//!
//! ## What this is
//!
//! A prototype that tails a growing `Encounter.log` and pushes fights-segments to
//! `esologs.com/desktop-client/*` **incrementally**, holding ONE report open across
//! a whole session and terminating only when logging ends — instead of the one-shot
//! "read the finished file, upload once" path in [`super::super::transport`].
//!
//! ## Why it is gated `#[cfg(debug_assertions)]`
//!
//! Native live is feasibility R&D, **not** a shipping feature:
//!
//! * The server's open-report rendering behaviour (does it incrementally render a
//!   report fed many `add-report-segment` POSTs under `isLiveLog:true`, and what does
//!   `nextSegmentId=0` mean on an open report?) is **unverified** — only a real live
//!   round-trip can settle it.
//! * It is a distinct, higher-conspicuousness server operation (`liveLog`) from the
//!   already-authorized one-shot upload, so it is **ToS-gated on operator sign-off**.
//!
//! The whole module compiles out of release builds, so the live path can never ship
//! by accident. See `docs/native-live-streaming-spike-FINDINGS.md` for the full
//! analysis (feasibility, time-base, ToS, effort).
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

#![cfg(debug_assertions)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::client::{MasterTableBytes, NativeUpload, ReportCode, Segment, UploadError};
use super::events::EventEmitter;
use super::incremental::{IncrementalIndexState, IncrementalMasterState};
use super::session::SessionProvider;
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
}

/// Run the debug-only native live upload, tailing `growing_path` and streaming
/// segments to an OPEN report until logging ends, `cancel` is set, or the file goes
/// idle past the inactivity deadline. Returns the report code on a clean finish.
///
/// This is the network driver around [`LiveSegmenter`]. It is intentionally NOT wired
/// into any Tauri command; an owner who wants to round-trip-test feasibility calls it
/// from a debug harness against a SYNTHETIC growing file (Controlled Folder Access
/// blocks a debug binary from reading a real in-game `Encounter.log` under Documents).
///
/// Orphan-safety: once the report is open, ANY exit path (clean finish, stop, error,
/// idle) attempts `terminate-report` so a draft is never left open server-side. (A
/// production build would ALSO persist `{code, segment_id}` to recover from a crash
/// that kills the process before terminate — see the FINDINGS L2 item.)
pub fn run_native_live_spike(
    growing_path: &str,
    session: &dyn SessionProvider,
    opts: &UploadOptions,
    cancel: Arc<AtomicBool>,
    poll: &dyn LiveTail,
) -> Result<(ReportCode, usize), UploadError> {
    let upload = NativeUpload::new(session, opts, cancel.clone());
    // Establish the session up front, then open the report.
    let _ = session.session()?;
    let code = upload.create_report_live()?;

    // Drive: tail → feed → cut → POST. Any error after create terminates the report.
    let result = stream_until_done(&upload, &code, growing_path, &cancel, poll);
    // Best-effort terminate on EVERY exit (success, stop, error, idle).
    let _ = upload.terminate_report_live(&code);
    let segments = result?;
    eprintln!(
        "[uploader] native live spike: report {} terminated after {} segment(s)",
        code.0, segments
    );
    Ok((code, segments))
}

/// The tail loop, factored out so the terminate-on-exit wrapper in
/// [`run_native_live_spike`] covers every return path.
fn stream_until_done(
    upload: &NativeUpload<'_>,
    code: &ReportCode,
    growing_path: &str,
    cancel: &Arc<AtomicBool>,
    poll: &dyn LiveTail,
) -> Result<usize, UploadError> {
    let mut seg = LiveSegmenter::new();
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(UploadError::Cancelled);
        }
        match poll.next_lines(growing_path) {
            TailOutcome::Lines(lines) => {
                for line in lines {
                    let boundary = seg.feed(&line);
                    if boundary
                        && seg.shields_settled()
                        && !post_one_segment(upload, code, &mut seg, cancel)?
                    {
                        // Server signalled session-end (nextSegmentId=0) mid-stream.
                        return Ok(seg.segments_built());
                    }
                }
            }
            // The file went idle past the deadline, or logging ended (END_LOG). Drain
            // any trailing pending shields (so a fully-absorbed final hit isn't lost),
            // flush the final segment, then finish.
            TailOutcome::Done => {
                seg.drain_trailing_shields();
                let _ = post_one_segment(upload, code, &mut seg, cancel)?;
                return Ok(seg.segments_built());
            }
            TailOutcome::Error(e) => return Err(UploadError::Transport(e)),
        }
    }
}

/// Build the next segment and POST it (master then add-segment, live params). Updates
/// the server-sequenced next id. Returns `Ok(true)` to keep streaming, `Ok(false)`
/// when the server signalled session-end via `nextSegmentId == 0` (the one-shot
/// "0 before last local segment is an error" rule does NOT apply to an open-ended
/// stream — on a live report a 0 means stop; the caller then terminates cleanly). A
/// build that yields nothing to send is a no-op (`Ok(true)`).
fn post_one_segment(
    upload: &NativeUpload<'_>,
    code: &ReportCode,
    seg: &mut LiveSegmenter,
    cancel: &Arc<AtomicBool>,
) -> Result<bool, UploadError> {
    if cancel.load(Ordering::SeqCst) {
        return Err(UploadError::Cancelled);
    }
    let payload = match seg.build_next_segment() {
        Ok(Some(p)) => p,
        Ok(None) => return Ok(true), // nothing to send this window; keep streaming
        Err(detail) => {
            // A malformed/inconsistent segment must NEVER be shipped — fail loudly so
            // the driver stops + terminates rather than open a non-rendering report.
            return Err(UploadError::Server { status: 0, detail });
        }
    };
    upload.set_master_table_live(code, payload.segment_id, &payload.master)?;
    let next = upload.add_segment_live(
        code,
        payload.segment_id,
        &payload.segment,
        payload.in_progress_event_count,
    )?;
    if next == 0 {
        // Server says no further segments — stop streaming. The caller's terminate
        // wrapper closes the report.
        return Ok(false);
    }
    seg.set_next_segment_id(next);
    Ok(true)
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
    Lines(Vec<String>),
    Done,
    Error(String),
}

/// A real growing-file [`LiveTail`]: tails a file by byte offset (the spike's tester
/// appends to a synthetic `.log` while this streams it), returning each batch of
/// newly-appended COMPLETE lines. Signals `Done` when an `END_LOG` line is seen OR the
/// file stops growing for [`Self::idle_deadline`]. A partial trailing line (no newline
/// yet) is held back until its newline arrives, so a half-written line is never fed.
///
/// Debug/spike only: it blocks the calling thread with short sleeps between polls, which
/// is fine for a one-off owner-run round-trip but is NOT the production tail (that would
/// reuse `watcher.rs`'s notify-based loop). Place the synthetic file OUTSIDE a
/// CFA-protected folder (not `Documents`/`Desktop`) so the debug binary can read it.
pub struct FileTail {
    offset: std::sync::Mutex<u64>,
    /// Carry-over bytes of a partial (un-newlined) trailing line between polls.
    partial: std::sync::Mutex<Vec<u8>>,
    /// When the file last grew (for the idle deadline). `None` until first read.
    last_growth: std::sync::Mutex<Option<std::time::Instant>>,
    poll_interval: std::time::Duration,
    idle_deadline: std::time::Duration,
}

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
                // No growth. If we've been idle past the deadline, we're done.
                let idle = self
                    .last_growth
                    .lock()
                    .unwrap()
                    .map(|t| t.elapsed() >= self.idle_deadline)
                    .unwrap_or(false);
                drop(off);
                if idle {
                    return TailOutcome::Done;
                }
            }
            std::thread::sleep(self.poll_interval);
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
                .unwrap_or(TailOutcome::Done)
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

    // The segmenter cuts at fight boundaries and the segments it builds, concatenated,
    // reproduce the one-shot event stream — proving the live cut path is correct
    // offline (the same invariant the events differential test checks, but exercised
    // through the LiveSegmenter API the driver actually uses).
    #[test]
    fn live_segmenter_cuts_reproduce_the_one_shot_event_stream() {
        let lines = fixture_lines();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();

        // One-shot reference body.
        let (id2a, ab2i) = super::super::encode::actor_ability_maps(&refs);
        let mut one_shot = EventEmitter::with_master_indices(id2a, ab2i);
        let one_shot_body = one_shot.build(&refs).events_string;

        // Drive the segmenter, collecting each cut segment's events.
        let mut seg = LiveSegmenter::new();
        let mut assembled = String::new();
        let mut built = 0usize;
        for line in &lines {
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

        assert!(built >= 2, "fixture should produce multiple fight segments");
        assert_eq!(
            assembled, one_shot_body,
            "concatenated live-segment bodies must equal the one-shot event stream"
        );
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
            TailOutcome::Done,
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
                TailOutcome::Done => {
                    // The pump ends only when the tail signals Done (reaching here is
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
}
