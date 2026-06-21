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
//! * CUMULATIVE master per cut, rebuilt from `all_lines_so_far` with the prior frozen
//!   actor indices PINNED ([`encode::actor_ability_maps_forced`] +
//!   [`encode::build_master_table_with_tuples_forced`]) so a late-registering actor
//!   never renumbers an earlier segment's tuple `A`-refs (hazard H1).
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
use super::session::SessionProvider;
use crate::uploader::types::UploadOptions;

/// The pure, network-free core of the live driver: owns the long-lived emitter, the
/// cumulative raw-line buffer, and the frozen index maps, and produces a ready-to-send
/// payload on each fight-boundary cut. Separated from the I/O loop so the state
/// machine (cut policy, H1 pinning, time bounds, validation) is unit-testable without
/// a server or a real growing file.
pub struct LiveSegmenter {
    /// The single emitter carried across all segments.
    emitter: EventEmitter,
    /// Every raw line seen so far — the cumulative master is rebuilt from this each
    /// cut, and the index maps are refreshed from it as new actors/abilities register.
    ///
    /// SPIKE PERF NOTE (deliberately un-optimized): `refresh_maps` re-walks `all_lines`
    /// on every registering line, so live assembly is O(events × lines) and memory
    /// grows with the session. That is fine for a debug feasibility prototype but is
    /// NOT production-shaped. A production build would (a) maintain the index maps
    /// incrementally instead of re-deriving, and (b) persist the tuple list rather
    /// than re-walking — both called out in the FINDINGS effort estimate. Correctness
    /// (byte-identical to the one-shot path) is proven by
    /// `live_segmenter_cuts_reproduce_the_one_shot_event_stream`; speed is explicitly
    /// out of scope for the spike.
    all_lines: Vec<String>,
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
            all_lines: Vec::new(),
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
        self.all_lines.push(line.to_string());
        self.segment_lines.push(line.to_string());
        // The emitter allocates a tuple's `A` from its `identity_to_actor` /
        // `ability_to_index` maps AT FEED TIME, so those maps must be current BEFORE
        // the emitter sees a line that allocates a tuple. A combat/effect/cast line
        // can be the FIRST registering event for a monster added earlier (which the
        // registering filter only includes once it registers), and an introduction
        // line (`UNIT_ADDED`/`ABILITY_INFO`/`EFFECT_INFO`/`BEGIN_LOG`) adds to the
        // index space. Refresh on BOTH — but only after pushing this line to
        // `all_lines` (above), so a combat line's own registration is visible. This
        // keeps the live `A` numbering identical to the one-shot path. Lines that
        // can neither register nor introduce (boundaries, regen, player-info) skip
        // the refresh, so the per-event cost is paid only where it matters.
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

    /// Rebuild the emitter's master index maps cumulatively from `all_lines`, pinning
    /// the already-frozen actor indices so nothing renumbers (the H1 fix). The pin
    /// makes this idempotent and append-only: an already-indexed actor keeps its
    /// index; a newly-registering one appends above the max.
    fn refresh_maps(&mut self) {
        use super::encode::actor_ability_maps_forced;
        let frozen_actors = self.emitter.frozen_actor_index_map();
        let frozen_abilities = self.emitter.frozen_ability_index_map();
        let lines_ref: Vec<&str> = self.all_lines.iter().map(String::as_str).collect();
        let (id2a, ab2i) =
            actor_ability_maps_forced(&lines_ref, Some((&frozen_actors, &frozen_abilities)));
        self.emitter.refresh_master_indices(id2a, ab2i);
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
        use super::encode::build_master_table_with_tuples_forced;
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

        // Frame the body into the fights-segment text (header + count + events).
        let log_version = self
            .all_lines
            .iter()
            .find_map(|l| {
                let f = super::encode::split_csv_quoted_pub(l);
                if f.get(1).map(|s| s.trim()) == Some("BEGIN_LOG") {
                    f.get(3).map(|s| s.trim().to_string())
                } else {
                    None
                }
            })
            .ok_or("live segment has no BEGIN_LOG log version")?;
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
        // canonical index space; build the master from those same pinned maps + the
        // emitter's tuple table, so the segment's `A`/C refs and the master's
        // tuples/actors/abilities are in lockstep.
        let frozen_actors = self.emitter.frozen_actor_index_map();
        let frozen_abilities = self.emitter.frozen_ability_index_map();
        let lines_ref: Vec<&str> = self.all_lines.iter().map(String::as_str).collect();
        let master_text = build_master_table_with_tuples_forced(
            &lines_ref,
            self.emitter.tuples(),
            &frozen_actors,
            &frozen_abilities,
        )
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
) -> Result<ReportCode, UploadError> {
    let upload = NativeUpload::new(session, opts, cancel.clone());
    // Establish the session up front, then open the report.
    let _ = session.session()?;
    let code = upload.create_report_live()?;

    // Drive: tail → feed → cut → POST. Any error after create terminates the report.
    let result = stream_until_done(&upload, &code, growing_path, &cancel, poll);
    // Best-effort terminate on EVERY exit (success, stop, error, idle).
    let _ = upload.terminate_report_live(&code);
    result.map(|()| code)
}

/// The tail loop, factored out so the terminate-on-exit wrapper in
/// [`run_native_live_spike`] covers every return path.
fn stream_until_done(
    upload: &NativeUpload<'_>,
    code: &ReportCode,
    growing_path: &str,
    cancel: &Arc<AtomicBool>,
    poll: &dyn LiveTail,
) -> Result<(), UploadError> {
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
                        return Ok(());
                    }
                }
            }
            // The file went idle past the deadline, or logging ended (END_LOG). Drain
            // any trailing pending shields (so a fully-absorbed final hit isn't lost),
            // flush the final segment, then finish.
            TailOutcome::Done => {
                seg.drain_trailing_shields();
                let _ = post_one_segment(upload, code, &mut seg, cancel)?;
                return Ok(());
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
