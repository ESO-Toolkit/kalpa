//! Streaming, byte-offset-aware scanner for ESO `Encounter.log` files.
//!
//! ESO combat logs are append-only and can reach multiple gigabytes, so this
//! scanner never holds the whole file in memory. It reads line by line, folding
//! boundary detection directly into the read loop so only running state
//! (O(sessions + fights)) is retained — line content is dropped each iteration.
//! It detects:
//!
//! * **Sessions** — bounded by `BEGIN_LOG` lines. The game appends to one file
//!   across play sessions; each `/encounterlog` re-enable writes a fresh
//!   `BEGIN_LOG`, so one physical file can contain many sessions. This is the
//!   canonical split point for oversized logs.
//! * **Fights** — bounded by `BEGIN_COMBAT` … `END_COMBAT`. Expressed purely as
//!   byte ranges so segmentation stays O(1) in memory.
//!
//! Every ESO log line is `<relativeMs>,<LINE_TYPE>,<fields…>`. We only need the
//! first two tokens to find boundaries, plus light parsing of `BEGIN_LOG`,
//! `ZONE_CHANGED` and `UNIT_ADDED` for naming. Field counts are variable (due to
//! `<unitState>` expansion and bracketed arrays), so we deliberately avoid full
//! CSV parsing — we split only what we need.
//!
//! **Byte-offset discipline:** all offsets are tracked from raw byte counts, not
//! from decoded-string lengths. A corrupt byte decoded lossily to U+FFFD is 3
//! bytes in a `String` but 1 byte on disk, so deriving offsets from a decoded
//! string would misalign every subsequent range. Content is decoded lossily
//! only for field parsing, never for offset math.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use super::types::{FightSummary, LogSession};

/// Files larger than this get a "recommend split" hint in the UI.
pub const SPLIT_RECOMMEND_BYTES: u64 = 512 * 1024 * 1024; // 512 MiB

/// Parse the leading relative-ms value (first comma-separated token) of a line.
fn parse_rel_ms(line: &str) -> u64 {
    line.split(',')
        .next()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

/// Return the Nth comma-separated field (0-based), trimming surrounding quotes.
/// Naive split is fine here because the fields we read (line type, version,
/// zone/unit names) never themselves contain an embedded unquoted comma before
/// the index we want for these specific line types.
fn field(line: &str, idx: usize) -> Option<&str> {
    line.split(',').nth(idx).map(|s| s.trim().trim_matches('"'))
}

/// The line types we care about for boundary/naming detection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LineType {
    BeginLog,
    EndLog,
    ZoneChanged,
    UnitAdded,
    BeginCombat,
    EndCombat,
    Other,
}

/// Classify a line by its type token (field index 1) without allocating —
/// compares case-insensitively against the known tokens.
fn classify(line: &str) -> LineType {
    let Some(tok) = field(line, 1) else {
        return LineType::Other;
    };
    if tok.eq_ignore_ascii_case("BEGIN_LOG") {
        LineType::BeginLog
    } else if tok.eq_ignore_ascii_case("END_LOG") {
        LineType::EndLog
    } else if tok.eq_ignore_ascii_case("ZONE_CHANGED") {
        LineType::ZoneChanged
    } else if tok.eq_ignore_ascii_case("UNIT_ADDED") {
        LineType::UnitAdded
    } else if tok.eq_ignore_ascii_case("BEGIN_COMBAT") {
        LineType::BeginCombat
    } else if tok.eq_ignore_ascii_case("END_COMBAT") {
        LineType::EndCombat
    } else {
        LineType::Other
    }
}

/// What the TAIL of an `Encounter.log` tells us about the current logging session —
/// the input to the native-live readiness probe. `open_session` is true when the last
/// SESSION boundary seen is a `BEGIN_LOG` not yet closed by an `END_LOG` (logging looks
/// active and no fresh header is coming until a `/reloadui`). `fight_in_progress` is
/// true when the last combat boundary is a `BEGIN_COMBAT` without a following
/// `END_COMBAT`. The two `saw_*_boundary` flags are kept SEPARATE: a peek can contain
/// combat boundaries but NO session boundary (a long fight whose `BEGIN_LOG` scrolled
/// off the top of the peek window) — in that case `open_session` is unknown, so the
/// caller must treat it as Uncertain rather than "logging off".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct TailState {
    pub open_session: bool,
    pub fight_in_progress: bool,
    /// Saw a `BEGIN_LOG` or `END_LOG` (so `open_session` is authoritative).
    pub saw_session_boundary: bool,
    /// Saw a `BEGIN_COMBAT` or `END_COMBAT` (activity, even without a session header).
    pub saw_combat_boundary: bool,
}

/// Derive the [`TailState`] from a chunk of a log (the bytes read by the readiness
/// probe). `drop_first_line` should be true for a mid-file peek (the chunk almost
/// always starts inside a line, so the leading partial is discarded — the scanner's
/// chunked-read discipline) and FALSE when the chunk starts at byte 0 (a small whole
/// file — dropping its real first line could hide the `BEGIN_LOG`). Tracks the latest
/// session + combat boundary; a `BEGIN_LOG` re-opens the session and (since the game
/// starts a session fresh) clears any stale in-combat flag.
pub(crate) fn tail_session_state(chunk: &[u8], drop_first_line: bool) -> TailState {
    let text = String::from_utf8_lossy(chunk);
    let mut lines = text.lines();
    if drop_first_line {
        let _ = lines.next();
    }
    let mut st = TailState::default();
    for line in lines {
        match classify(line) {
            LineType::BeginLog => {
                st.open_session = true;
                st.fight_in_progress = false;
                st.saw_session_boundary = true;
            }
            LineType::EndLog => {
                st.open_session = false;
                st.fight_in_progress = false;
                st.saw_session_boundary = true;
            }
            LineType::BeginCombat => {
                st.fight_in_progress = true;
                st.saw_combat_boundary = true;
            }
            LineType::EndCombat => {
                st.fight_in_progress = false;
                st.saw_combat_boundary = true;
            }
            _ => {}
        }
    }
    st
}

/// The disk anchor for a MID-SESSION live join: the byte offset of the most-recent
/// `BEGIN_LOG` at or before `eof`, plus whether that session is still open (no `END_LOG`
/// after it). The native-live driver replays `[begin_log_offset, eof)` to warm its
/// encoder tables + wall clock from disk, so a user who is ALREADY combat-logging can go
/// live WITHOUT a fresh `/reloadui`. `open_session` distinguishes "logging now" (replay
/// the prefix) from "last session already ended" (no prefix — tail from EOF as before).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SessionAnchor {
    /// Absolute byte offset of the most-recent `BEGIN_LOG` line's FIRST byte.
    pub begin_log_offset: u64,
    /// True iff NO `END_LOG` follows that `BEGIN_LOG` up to `eof` (logging still active).
    pub open_session: bool,
}

/// Window size for the backward scan — reuse the readiness-probe peek (256 KiB is many
/// thousands of lines, vastly more than the few-hundred-byte gap between a `BEGIN_LOG`
/// and its session, and far larger than any single line).
const ANCHOR_WINDOW: u64 = super::tail_io::TAIL_PEEK;

/// Find the most-recent `BEGIN_LOG` at/before `eof` by scanning fixed windows BACKWARD,
/// so a multi-GB log never reads more than the tail back to the current session header.
/// Returns `Ok(None)` when the file contains no `BEGIN_LOG` at all (rotated/truncated) —
/// the caller MUST then route to the official handoff and NEVER synthesize a wall clock.
///
/// Correctness details:
/// * Windows overlap by `ANCHOR_WINDOW/8` (32 KiB ≫ any line) so a `BEGIN_LOG` straddling
///   a window seam is still seen whole in at least one window. The first (newest) window
///   that contains a `BEGIN_LOG` wins — older windows are not read, which is exactly the
///   "never cross an earlier `/reloadui`" rule (we want the CURRENT session's header).
/// * Within a window we compute each line's ABSOLUTE offset (window start + intra-window
///   byte position) so `begin_log_offset` is a true file offset usable by `read_range`.
/// * `open_session` is decided by a forward pass over `[begin_log_offset, eof)`'s tail:
///   any `END_LOG` after the chosen `BEGIN_LOG` means the session already closed.
pub(crate) fn find_current_session_begin(
    path: &Path,
    eof: u64,
) -> Result<Option<SessionAnchor>, String> {
    let mut buf = Vec::new();
    let Some(begin_log_offset) = find_latest_begin_log_offset(path, eof, &mut buf)? else {
        return Ok(None);
    };
    let open_session = !any_end_log_after(path, begin_log_offset, eof, &mut buf)?;
    Ok(Some(SessionAnchor {
        begin_log_offset,
        open_session,
    }))
}

/// Find the latest `BEGIN_LOG` offset without the forward `END_LOG` pass.
///
/// Use this when the caller only needs a copy boundary (for example the "split latest
/// session" fast path). `find_current_session_begin` adds the open/closed state for
/// live-readiness, which requires scanning forward from the header.
pub(crate) fn find_latest_session_begin(path: &Path, eof: u64) -> Result<Option<u64>, String> {
    let mut buf = Vec::new();
    find_latest_begin_log_offset(path, eof, &mut buf)
}

fn find_latest_begin_log_offset(
    path: &Path,
    eof: u64,
    buf: &mut Vec<u8>,
) -> Result<Option<u64>, String> {
    if eof == 0 {
        return Ok(None);
    }
    let overlap = ANCHOR_WINDOW / 8;
    // Walk windows from newest (ending at eof) to oldest (starting at 0).
    let mut win_end = eof;
    loop {
        let win_start = win_end.saturating_sub(ANCHOR_WINDOW);
        let n = super::tail_io::read_range(path, win_start, win_end, buf)?;
        // Find the absolute offset of the LAST BEGIN_LOG whose line START lies in this
        // window. We iterate line starts by tracking byte position; a line "starts" right
        // after a preceding '\n' (or at window byte 0). Skip the leading partial of a
        // mid-file window (win_start > 0): its true start is in the previous window, so a
        // BEGIN_LOG there will be caught with its real offset by the overlap.
        let mut last_begin: Option<u64> = None;
        let mut line_start: usize = 0;
        let bytes = &buf[..n];
        for i in 0..n {
            if bytes[i] == b'\n' {
                let line = &bytes[line_start..i];
                // Only count a line whose start is genuinely within this window (not the
                // leading partial of a non-zero-based window — that belongs to the prior
                // window and would have a bogus offset here).
                let is_partial_lead = win_start > 0 && line_start == 0;
                if !is_partial_lead && line_is_begin_log(line) {
                    last_begin = Some(win_start + line_start as u64);
                }
                line_start = i + 1;
            }
        }
        // Do not count an unterminated final line at EOF. ESO can be mid-append, and
        // a half-written BEGIN_LOG must not become a split/preflight boundary.
        if let Some(begin_log_offset) = last_begin {
            return Ok(Some(begin_log_offset));
        }
        if win_start == 0 {
            return Ok(None); // scanned the whole file, no BEGIN_LOG
        }
        // Step back by a full window minus the overlap so a seam-straddling line is seen.
        win_end = win_start + overlap;
    }
}

/// True iff the line (sans trailing CR) is a `BEGIN_LOG` record. Mirrors `classify`'s
/// token test but works on raw bytes so the backward scanner avoids a full decode.
fn line_is_begin_log(line: &[u8]) -> bool {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    // ESO line shape: `<relMs>,BEGIN_LOG,...`. The type is the 2nd comma field.
    let mut fields = line.split(|&b| b == b',');
    let _rel = fields.next();
    matches!(fields.next(), Some(t) if t.eq_ignore_ascii_case(b"BEGIN_LOG"))
}

/// Scan `[from, eof)` forward (in `MAX_READ`-bounded chunks) for any `END_LOG` line,
/// which would mean the session opened at `from` has already closed. Reuses `buf`.
fn any_end_log_after(path: &Path, from: u64, eof: u64, buf: &mut Vec<u8>) -> Result<bool, String> {
    let mut pos = from;
    let mut carry: Vec<u8> = Vec::new();
    while pos < eof {
        let chunk_end = (pos + super::tail_io::MAX_READ).min(eof);
        let n = super::tail_io::read_range(path, pos, chunk_end, buf)?;
        // Stitch the carry (a partial line from the previous chunk) onto this chunk's
        // bytes so a line split across the chunk boundary is classified whole.
        let mut combined = std::mem::take(&mut carry);
        combined.extend_from_slice(&buf[..n]);
        let mut line_start = 0usize;
        for i in 0..combined.len() {
            if combined[i] == b'\n' {
                if line_is_end_log(&combined[line_start..i]) {
                    return Ok(true);
                }
                line_start = i + 1;
            }
        }
        // Whatever follows the last '\n' is a partial; carry it to the next chunk.
        carry = combined[line_start..].to_vec();
        pos = chunk_end;
    }
    // A final unterminated END_LOG can be a partial append, so only complete lines
    // decide whether the latest session has closed.
    Ok(false)
}

/// True iff the line (sans trailing CR) is an `END_LOG` record.
fn line_is_end_log(line: &[u8]) -> bool {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let mut fields = line.split(|&b| b == b',');
    let _rel = fields.next();
    matches!(fields.next(), Some(t) if t.eq_ignore_ascii_case(b"END_LOG"))
}

/// Translate common IO errors into the friendly, project-consistent strings the
/// frontend already maps (see `src/lib/tauri.ts`).
fn map_io_error(e: &std::io::Error, path: &str) -> String {
    use std::io::ErrorKind;
    match e.kind() {
        ErrorKind::NotFound => format!("Log file not found: {path}"),
        ErrorKind::PermissionDenied => {
            // Often Controlled Folder Access on Windows — surfaced specially by the UI.
            format!("permission denied (os error 13) reading {path}")
        }
        _ => format!("Failed to open log file: {e}"),
    }
}

/// Result of a full scan: the sessions and the flat list of fights.
pub struct ScanResult {
    pub sessions: Vec<LogSession>,
    pub fights: Vec<FightSummary>,
    pub total_fights: usize,
}

/// Running boundary-detection state, shared by the full-file and chunk scanners.
///
/// Feeding lines (with their absolute byte offsets) drives session and fight
/// detection. Only O(sessions + fights) memory is held; line content is not
/// retained between calls.
#[derive(Default)]
struct Detector {
    sessions: Vec<LogSession>,
    fights: Vec<FightSummary>,
    total_fights: usize,
    fight_collect_limit: Option<usize>,
    session_open: bool,
    session_fight_count: usize,
    /// (start_offset, rel_ms) of an open `BEGIN_COMBAT` awaiting its `END_COMBAT`.
    fight_start: Option<(u64, u64)>,
    pending_zone: Option<String>,
    pending_boss: Option<String>,
    /// Chunk-scan only: absolute offset of the first `BEGIN_LOG` seen *after*
    /// the chunk started in an already-open session (a `/encounterlog`
    /// re-enable mid-tail). Lets the watcher detect a new session without a
    /// file shrink. Unused by the full-file scan.
    new_session_at: Option<u64>,
}

impl Detector {
    fn close_session(&mut self, end_offset: u64) {
        if let Some(last) = self.sessions.last_mut() {
            last.end_offset = end_offset;
            last.fight_count = self.session_fight_count;
            last.size_bytes = end_offset.saturating_sub(last.start_offset);
        }
    }

    /// Feed one line. `offset` is the line's first byte; `next_offset` is the
    /// start of the following line (i.e. one past the line terminator).
    fn feed(&mut self, line: &str, offset: u64, next_offset: u64) {
        match classify(line) {
            LineType::BeginLog => {
                if self.session_open {
                    self.close_session(offset);
                    // A new BEGIN_LOG while already in a session = a fresh
                    // logging session appended to the same file. Record the
                    // first such boundary for the chunk scanner, anchored just
                    // PAST the BEGIN_LOG line so re-reading from it doesn't
                    // re-detect the same boundary (avoids a 1-byte crawl).
                    if self.new_session_at.is_none() {
                        self.new_session_at = Some(next_offset);
                    }
                }
                self.session_open = true;
                self.session_fight_count = 0;
                self.fight_start = None;
                self.pending_zone = None;
                self.pending_boss = None;

                let start_time_ms = field(line, 2)
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                let log_version = field(line, 3).unwrap_or("").to_string();
                let realm = field(line, 4)
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty());

                let index = self.sessions.len();
                self.sessions.push(LogSession {
                    index,
                    start_offset: offset,
                    end_offset: next_offset,
                    start_time_ms,
                    log_version,
                    realm,
                    fight_count: 0,
                    size_bytes: 0,
                });
            }
            LineType::EndLog => {
                if self.session_open {
                    self.close_session(next_offset);
                    self.session_open = false;
                }
            }
            LineType::ZoneChanged => {
                self.pending_zone = field(line, 3)
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty());
            }
            LineType::UnitAdded => {
                // isBoss is field 7 (isLocalPlayer is field 4 — a different
                // "T" flag at a different offset). A bare `,T,` substring
                // search previously matched the local player's own
                // UNIT_ADDED line via its isLocalPlayer="T", wrongly naming
                // fights after the player. Name is field 10; the @-handle
                // filter is kept as defense in depth so only monster names
                // are ever adopted.
                if field(line, 7).is_some_and(|f| f.eq_ignore_ascii_case("T")) {
                    if let Some(name) =
                        field(line, 10).filter(|s| !s.is_empty() && !s.starts_with('@'))
                    {
                        self.pending_boss = Some(name.to_string());
                    }
                }
            }
            LineType::BeginCombat => {
                self.fight_start = Some((offset, parse_rel_ms(line)));
            }
            LineType::EndCombat => {
                if let Some((start_offset, start_ms)) = self.fight_start.take() {
                    let index = self.total_fights;
                    let should_collect = self
                        .fight_collect_limit
                        .map(|limit| self.fights.len() < limit)
                        .unwrap_or(true);
                    if should_collect {
                        self.fights.push(FightSummary {
                            index,
                            start_offset,
                            end_offset: next_offset,
                            start_ms,
                            end_ms: parse_rel_ms(line),
                            zone_name: self.pending_zone.clone(),
                            boss_name: self.pending_boss.clone(),
                        });
                    }
                    self.pending_boss.take();
                    self.total_fights += 1;
                    self.session_fight_count += 1;
                }
            }
            LineType::Other => {}
        }
    }

    fn finish(mut self, eof_offset: u64) -> ScanResult {
        if self.session_open {
            self.close_session(eof_offset);
        }
        ScanResult {
            sessions: self.sessions,
            fights: self.fights,
            total_fights: self.total_fights,
        }
    }
}

/// Strip a trailing `\n` and optional `\r` from a raw line buffer, returning the
/// content byte slice (offsets are computed from the full buffer, not this).
fn line_content(buf: &[u8]) -> &[u8] {
    let mut end = buf.len();
    if end > 0 && buf[end - 1] == b'\n' {
        end -= 1;
    }
    if end > 0 && buf[end - 1] == b'\r' {
        end -= 1;
    }
    &buf[..end]
}

/// Scan an entire log file into sessions and fights in a single streaming pass.
///
/// O(lines) time, O(sessions + fights) memory — each line's content is decoded,
/// classified, and dropped before the next read.
pub fn scan_file(path: &str) -> Result<ScanResult, String> {
    scan_file_with_fight_limit(path, None)
}

/// Scan an entire log file while optionally capping the retained fight summaries.
/// `total_fights` and each session's `fight_count` still count every completed
/// fight, so callers can omit a large IPC payload without losing counts.
pub fn scan_file_with_fight_limit(
    path: &str,
    fight_collect_limit: Option<usize>,
) -> Result<ScanResult, String> {
    let file = File::open(path).map_err(|e| map_io_error(&e, path))?;
    let mut reader = BufReader::with_capacity(1 << 20, file); // 1 MiB buffer

    let mut detector = Detector {
        fight_collect_limit,
        ..Default::default()
    };
    let mut offset: u64 = 0;
    let mut buf: Vec<u8> = Vec::with_capacity(512);

    loop {
        buf.clear();
        // `read_until(b'\n')` makes offsets account for `\r\n` vs `\n`: the byte
        // count `n` always includes the terminator, so `next_offset` is the
        // start of the following line.
        let n = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| format!("Failed to read log: {e}"))?;
        if n == 0 {
            break;
        }
        let next_offset = offset + n as u64;
        let content = line_content(&buf);
        if !content.is_empty() {
            // Lossy decode for field parsing only — offsets came from raw bytes.
            let line = String::from_utf8_lossy(content);
            detector.feed(&line, offset, next_offset);
        }
        offset = next_offset;
    }

    Ok(detector.finish(offset))
}

/// Scan a bounded byte range of a log into sessions and fights.
///
/// `start` must point at a true line boundary (the latest-session fast path passes a
/// `BEGIN_LOG` offset). Returned offsets are still absolute file offsets, so a later
/// split can validate and copy directly from the original log without translation.
pub fn scan_range(path: &str, start: u64, end: u64) -> Result<ScanResult, String> {
    scan_range_with_fight_limit(path, start, end, None)
}

/// Scan a bounded byte range while optionally capping retained fight summaries.
/// Unlike `scan_file`, the bounded/latest-session path treats the trailing
/// unterminated line as incomplete and leaves it unclassified.
pub fn scan_range_with_fight_limit(
    path: &str,
    start: u64,
    end: u64,
    fight_collect_limit: Option<usize>,
) -> Result<ScanResult, String> {
    if end <= start {
        return Err("Empty scan range".into());
    }
    let mut file = File::open(path).map_err(|e| map_io_error(&e, path))?;
    file.seek(SeekFrom::Start(start))
        .map_err(|e| format!("Failed to seek log: {e}"))?;
    let limited = file.take(end - start);
    let mut reader = BufReader::with_capacity(1 << 20, limited);

    let mut detector = Detector {
        fight_collect_limit,
        ..Default::default()
    };
    let mut offset = start;
    let mut buf: Vec<u8> = Vec::with_capacity(512);

    while offset < end {
        buf.clear();
        let n = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| format!("Failed to read log: {e}"))?;
        if n == 0 {
            break;
        }
        let next_offset = offset + n as u64;
        if next_offset >= end && buf.last() != Some(&b'\n') {
            break;
        }
        let content = line_content(&buf);
        if !content.is_empty() {
            let line = String::from_utf8_lossy(content);
            detector.feed(&line, offset, next_offset);
        }
        offset = next_offset;
    }

    Ok(detector.finish(offset))
}

/// Scan only the latest/current logging session: backward-anchor to the newest
/// `BEGIN_LOG`, then scan forward from that byte offset to a single EOF snapshot.
/// Returns the scan plus the whole-file snapshot length used for the range.
pub fn scan_latest_session(path: &str) -> Result<(ScanResult, u64), String> {
    scan_latest_session_with_fight_limit(path, None)
}

/// Scan only the latest/current logging session, optionally capping retained
/// fight summaries while preserving true fight counts.
pub fn scan_latest_session_with_fight_limit(
    path: &str,
    fight_collect_limit: Option<usize>,
) -> Result<(ScanResult, u64), String> {
    let src = Path::new(path);
    let size_bytes = std::fs::metadata(src)
        .map_err(|e| format!("Failed to read file: {e}"))?
        .len();
    let begin = find_latest_session_begin(src, size_bytes)?
        .ok_or_else(|| "No logging sessions found in this file.".to_string())?;
    if size_bytes <= begin {
        return Err("Latest logging session is empty.".into());
    }
    Ok((
        scan_range_with_fight_limit(path, begin, size_bytes, fight_collect_limit)?,
        size_bytes,
    ))
}

/// Result of scanning an incremental chunk for the live watcher.
pub struct ChunkScan {
    /// Fights that *completed* (BEGIN_COMBAT…END_COMBAT) within the chunk, with
    /// absolute byte offsets. `index` is 0 — the caller assigns running indices.
    pub fights: Vec<FightSummary>,
    /// Absolute byte offset of an open `BEGIN_COMBAT` whose `END_COMBAT` was not
    /// in the chunk. The watcher re-anchors here so a fight straddling the read
    /// window is captured on the next pass (rather than being mistaken for an
    /// oversized fight and skipped).
    pub open_fight_start: Option<u64>,
    /// Absolute byte offset just *past* the first mid-chunk `BEGIN_LOG` line (a
    /// `/encounterlog` re-enable). The watcher emits a session reset, re-indexes
    /// from 0, and re-anchors here — past the boundary line, so it isn't
    /// re-detected on the next pass.
    pub new_session_at: Option<u64>,
    /// The zone name still pending (seen via `ZONE_CHANGED` but not yet consumed
    /// by a completed fight) at the END of this chunk. The watcher carries this
    /// into the next pass's `pending_zone` seed so a zone/boss line consumed by
    /// an earlier read isn't lost by the time a later fight closes (E2). Cleared
    /// whenever a `BEGIN_LOG` is fed, mirroring `Detector::feed`'s `BeginLog` arm.
    pub pending_zone: Option<String>,
    /// The boss name still pending at the end of this chunk — same carry
    /// discipline as `pending_zone`.
    pub pending_boss: Option<String>,
}

/// Scan an in-memory raw byte chunk for completed fights, plus the boundary
/// signals the live watcher needs (open-fight start, new-session start).
///
/// Takes raw bytes (not a `&str`) so offsets are tracked from true byte lengths;
/// each line is decoded lossily only for field parsing.
///
/// `session_already_open` distinguishes a chunk read from *inside* an ongoing
/// session (true — a leading `BEGIN_LOG` is a mid-stream `/encounterlog`
/// re-enable → `new_session_at`) from one read at the *start* of a fresh file
/// (false — a leading `BEGIN_LOG` is just the session header, not a re-enable).
/// The watcher passes `false` right after a truncation reset to avoid a spurious
/// second `SessionReset`.
///
/// `pending_zone`/`pending_boss` seed the detector's naming state from the
/// PREVIOUS chunk's [`ChunkScan::pending_zone`]/[`ChunkScan::pending_boss`] (E2):
/// without this, each chunk starts naming state fresh, so a fight whose
/// `ZONE_CHANGED`/boss `UNIT_ADDED` lines were consumed by an earlier read (and
/// thus never re-scanned) would always come out unnamed. A `BEGIN_LOG` fed
/// during this scan still clears the seed exactly like a fresh session would.
pub fn scan_chunk_for_fights(
    chunk: &[u8],
    base: u64,
    session_already_open: bool,
    pending_zone: Option<String>,
    pending_boss: Option<String>,
) -> ChunkScan {
    let mut detector = Detector {
        session_open: session_already_open,
        pending_zone,
        pending_boss,
        ..Default::default()
    };
    let mut offset: u64 = 0;

    for raw_line in chunk.split_inclusive(|b| *b == b'\n') {
        // A trailing segment without a terminating '\n' is a not-yet-complete
        // line (the live read frequently lands mid-line while ESO appends).
        // Feeding it would misclassify a partially-flushed `…,BEGIN_LOG` or
        // `…,END_COMBAT` as a real boundary. Leave it unconsumed — the watcher
        // keeps `consumed` here and re-scans once the newline arrives.
        if raw_line.last() != Some(&b'\n') {
            break;
        }
        let n = raw_line.len() as u64;
        let next_offset = offset + n;
        let content = line_content(raw_line);
        if !content.is_empty() {
            let line = String::from_utf8_lossy(content);
            detector.feed(&line, base + offset, base + next_offset);
        }
        offset = next_offset;
    }

    ChunkScan {
        pending_zone: detector.pending_zone,
        pending_boss: detector.pending_boss,
        fights: detector.fights,
        open_fight_start: detector.fight_start.map(|(off, _)| off),
        new_session_at: detector.new_session_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run the streaming detector over an in-memory string (test helper).
    fn scan_text(text: &str) -> ScanResult {
        let bytes = text.as_bytes();
        let mut detector = Detector::default();
        let mut offset: u64 = 0;
        for raw_line in bytes.split_inclusive(|b| *b == b'\n') {
            let n = raw_line.len() as u64;
            let next_offset = offset + n;
            let content = line_content(raw_line);
            if !content.is_empty() {
                let line = String::from_utf8_lossy(content);
                detector.feed(&line, offset, next_offset);
            }
            offset = next_offset;
        }
        detector.finish(offset)
    }

    #[test]
    fn detects_single_session_and_fight() {
        let log = "0,BEGIN_LOG,1700000000000,15,\"NA Megaserver\",\"en\",\"10.0.0\"\n\
                   100,ZONE_CHANGED,1301,\"Sunspire\",VETERAN\n\
                   200,BEGIN_COMBAT\n\
                   5200,END_COMBAT\n\
                   5300,END_LOG\n";
        let r = scan_text(log);
        assert_eq!(r.sessions.len(), 1);
        assert_eq!(r.sessions[0].fight_count, 1);
        assert_eq!(r.sessions[0].log_version, "15");
        assert_eq!(r.sessions[0].start_time_ms, 1700000000000);
        assert_eq!(r.fights.len(), 1);
        assert_eq!(r.fights[0].zone_name.as_deref(), Some("Sunspire"));
        assert_eq!(r.fights[0].start_ms, 200);
        assert_eq!(r.fights[0].end_ms, 5200);
    }

    #[test]
    fn splits_multiple_sessions_on_begin_log() {
        let log = "0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"10.0\"\n\
                   10,BEGIN_COMBAT\n20,END_COMBAT\n\
                   0,BEGIN_LOG,1700000900000,15,\"NA\",\"en\",\"10.0\"\n\
                   10,BEGIN_COMBAT\n20,END_COMBAT\n30,BEGIN_COMBAT\n40,END_COMBAT\n";
        let r = scan_text(log);
        assert_eq!(r.sessions.len(), 2);
        assert_eq!(r.sessions[0].fight_count, 1);
        assert_eq!(r.sessions[1].fight_count, 2);
        assert_eq!(r.fights.len(), 3);
        // Second session starts exactly where the first ends.
        assert_eq!(r.sessions[0].end_offset, r.sessions[1].start_offset);
    }

    #[test]
    fn scan_range_keeps_absolute_offsets() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Encounter.log");
        let first = "0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"x\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n";
        let second = "0,BEGIN_LOG,2000,15,\"NA\",\"en\",\"x\"\n30,BEGIN_COMBAT\n40,END_COMBAT\n";
        std::fs::write(&path, format!("{first}{second}")).unwrap();

        let start = first.len() as u64;
        let end = start + second.len() as u64;
        let scan = scan_range(path.to_str().unwrap(), start, end).unwrap();

        assert_eq!(scan.sessions.len(), 1);
        assert_eq!(scan.sessions[0].index, 0);
        assert_eq!(scan.sessions[0].start_offset, start);
        assert_eq!(scan.sessions[0].end_offset, end);
        assert_eq!(scan.fights.len(), 1);
        assert_eq!(
            scan.fights[0].start_offset,
            start + second.find("30,BEGIN_COMBAT").unwrap() as u64
        );
    }

    #[test]
    fn scan_latest_session_only_scans_newest_begin_log_range() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Encounter.log");
        let first = "0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"x\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n";
        let second = "0,BEGIN_LOG,2000,15,\"NA\",\"en\",\"x\"\n30,BEGIN_COMBAT\n40,END_COMBAT\n";
        std::fs::write(&path, format!("{first}{second}")).unwrap();

        let (scan, size) = scan_latest_session(path.to_str().unwrap()).unwrap();

        assert_eq!(size, (first.len() + second.len()) as u64);
        assert_eq!(scan.sessions.len(), 1);
        assert_eq!(scan.sessions[0].start_offset, first.len() as u64);
        assert_eq!(scan.sessions[0].start_time_ms, 2000);
        assert_eq!(scan.fights.len(), 1);
        assert_eq!(scan.fights[0].start_ms, 30);
    }

    #[test]
    fn capped_scan_counts_all_fights_but_only_keeps_requested_summaries() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Encounter.log");
        let log = "0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"x\"\n\
                   10,BEGIN_COMBAT\n20,END_COMBAT\n\
                   30,BEGIN_COMBAT\n40,END_COMBAT\n\
                   50,BEGIN_COMBAT\n60,END_COMBAT\n";
        std::fs::write(&path, log).unwrap();

        let scan = scan_file_with_fight_limit(path.to_str().unwrap(), Some(1)).unwrap();

        assert_eq!(scan.total_fights, 3);
        assert_eq!(scan.sessions[0].fight_count, 3);
        assert_eq!(scan.fights.len(), 1);
        assert_eq!(scan.fights[0].index, 0);

        let count_only = scan_file_with_fight_limit(path.to_str().unwrap(), Some(0)).unwrap();
        assert_eq!(count_only.total_fights, 3);
        assert!(count_only.fights.is_empty());
    }

    #[test]
    fn latest_session_scan_ignores_partial_trailing_combat_line() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Encounter.log");
        let partial = "0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"x\"\n\
                       10,BEGIN_COMBAT\n\
                       20,END_COMBAT";
        std::fs::write(&path, partial).unwrap();

        let (scan, _) = scan_latest_session(path.to_str().unwrap()).unwrap();

        assert_eq!(scan.total_fights, 0);
        assert!(scan.fights.is_empty());

        std::fs::write(&path, format!("{partial}\n")).unwrap();
        let (complete, _) = scan_latest_session(path.to_str().unwrap()).unwrap();
        assert_eq!(complete.total_fights, 1);
        assert_eq!(complete.fights.len(), 1);
    }

    #[test]
    fn latest_session_anchor_ignores_partial_trailing_begin_log() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Encounter.log");
        let first = "0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"x\"\n\
                     10,BEGIN_COMBAT\n20,END_COMBAT\n";
        let partial_next = "0,BEGIN_LOG,2000,15,\"NA\",\"en\",\"x\"";
        std::fs::write(&path, format!("{first}{partial_next}")).unwrap();
        let eof = std::fs::metadata(&path).unwrap().len();

        let begin = find_latest_session_begin(&path, eof).unwrap().unwrap();
        let (scan, _) = scan_latest_session(path.to_str().unwrap()).unwrap();

        assert_eq!(begin, 0);
        assert_eq!(scan.sessions.len(), 1);
        assert_eq!(scan.sessions[0].start_time_ms, 1000);
        assert_eq!(scan.total_fights, 1);
    }

    #[test]
    fn byte_offsets_are_contiguous_and_cover_fights() {
        let log = "0,BEGIN_LOG,1,15,\"NA\",\"en\",\"x\"\n50,BEGIN_COMBAT\n99,END_COMBAT\n";
        let r = scan_text(log);
        let f = &r.fights[0];
        // The fight range must lie inside the session range.
        assert!(f.start_offset >= r.sessions[0].start_offset);
        assert!(f.end_offset <= r.sessions[0].end_offset);
    }

    #[test]
    fn chunk_scanner_tracks_raw_byte_offsets_with_invalid_utf8() {
        // A line with an invalid UTF-8 byte (0xFF) before the fight. If offsets
        // were derived from the lossy String (U+FFFD = 3 bytes), the fight
        // offsets would drift; from raw bytes they stay correct.
        let mut chunk: Vec<u8> = Vec::new();
        chunk.extend_from_slice(b"0,ZONE_CHANGED,1,\"Bad");
        chunk.push(0xFF); // invalid byte
        chunk.extend_from_slice(b"name\",NORMAL\n");
        let begin_at = chunk.len() as u64;
        chunk.extend_from_slice(b"10,BEGIN_COMBAT\n");
        chunk.extend_from_slice(b"20,END_COMBAT\n");

        let scan = scan_chunk_for_fights(&chunk, 0, true, None, None);
        assert_eq!(scan.fights.len(), 1);
        // start_offset must equal the true byte position of BEGIN_COMBAT.
        assert_eq!(scan.fights[0].start_offset, begin_at);
        assert_eq!(scan.fights[0].end_offset, chunk.len() as u64);
        assert_eq!(scan.open_fight_start, None);
    }

    #[test]
    fn chunk_scanner_reports_open_fight_start_for_straddling_fight() {
        // A fight whose BEGIN_COMBAT is in the chunk but END_COMBAT is not: the
        // watcher must re-anchor here, not treat it as oversized.
        let chunk = b"5,ZONE_CHANGED,1,\"Cloudrest\",VETERAN\n100,BEGIN_COMBAT\n";
        let begin_at = chunk.iter().position(|&b| b == b'\n').unwrap() as u64 + 1;
        let scan = scan_chunk_for_fights(chunk, 0, true, None, None);
        assert!(scan.fights.is_empty());
        assert_eq!(scan.open_fight_start, Some(begin_at));
    }

    #[test]
    fn chunk_scanner_reports_new_session_past_begin_log_line() {
        // A /encounterlog re-enable appends a fresh BEGIN_LOG to the same file.
        let begin_line = "0,BEGIN_LOG,1700001000000,15,\"NA\",\"en\",\"x\"\n";
        let prefix = "10,BEGIN_COMBAT\n20,END_COMBAT\n";
        let chunk = format!("{prefix}{begin_line}5,BEGIN_COMBAT\n15,END_COMBAT\n").into_bytes();
        // new_session_at is the offset just PAST the BEGIN_LOG line, so the next
        // read starts after it and the boundary isn't re-detected (no 1-byte crawl).
        let expected = (prefix.len() + begin_line.len()) as u64;
        // session_already_open = true: the chunk is read from inside an ongoing
        // session, so the embedded BEGIN_LOG is a mid-stream re-enable.
        let scan = scan_chunk_for_fights(&chunk, 0, true, None, None);
        assert_eq!(scan.new_session_at, Some(expected));
        // Both fights are still detected within the chunk (the watcher uses the
        // boundary to split pre/post-session dispatch).
        assert_eq!(scan.fights.len(), 2);
    }

    #[test]
    fn fresh_file_begin_log_is_header_not_a_new_session() {
        // Reading from the start of a fresh/rotated file: the leading BEGIN_LOG
        // is the session header, not a mid-stream re-enable, so it must NOT
        // report new_session_at (which would cause a spurious second reset).
        let chunk =
            "0,BEGIN_LOG,1700001000000,15,\"NA\",\"en\",\"x\"\n5,BEGIN_COMBAT\n15,END_COMBAT\n"
                .as_bytes();
        let scan = scan_chunk_for_fights(chunk, 0, false, None, None);
        assert_eq!(scan.new_session_at, None);
        assert_eq!(scan.fights.len(), 1);
    }

    #[test]
    fn partial_trailing_line_is_not_classified_as_a_boundary() {
        // The live read often ends mid-line while ESO appends. A partially
        // flushed BEGIN_LOG must NOT fire a (spurious) new-session signal until
        // its terminating newline arrives.
        let chunk = b"10,BEGIN_COMBAT\n20,END_COMBAT\n30,BEGIN_LOG,1700001000000,15";
        let scan = scan_chunk_for_fights(chunk, 0, true, None, None);
        assert_eq!(
            scan.new_session_at, None,
            "partial BEGIN_LOG must be deferred"
        );
        // The complete fight before the partial line is still detected.
        assert_eq!(scan.fights.len(), 1);
    }

    // ── UNIT_ADDED boss naming (E1: isBoss is field 7, not a `,T,` substring) ──

    // The local player's own UNIT_ADDED line has isLocalPlayer="T" (field 4) but
    // isBoss="F" (field 7). A bare `,T,` substring search used to match this line
    // (every session has exactly one) and wrongly name trash fights after the
    // player. With the positional field-7 check, a fight with no real boss unit
    // must come out with no boss name.
    #[test]
    fn unit_added_player_line_does_not_set_boss_name() {
        let log = "0,UNIT_ADDED,1,PLAYER,T,1,0,F,3,9,\"H\",\"@h\",1,50,160,0,PLAYER_ALLY,T\n\
                   10,BEGIN_COMBAT\n\
                   20,END_COMBAT\n";
        let r = scan_text(log);
        assert_eq!(r.fights.len(), 1);
        assert_eq!(r.fights[0].boss_name, None);
    }

    // A monster UNIT_ADDED with isBoss="T" at field 7 must still set the pending
    // boss name (positive case — the fix must not regress real boss detection).
    #[test]
    fn unit_added_boss_monster_line_sets_boss_name() {
        let log = "0,UNIT_ADDED,40,MONSTER,F,0,90001,T,0,0,\"Boss\",\"\",0,50,160,0,HOSTILE,F\n\
                   10,BEGIN_COMBAT\n\
                   20,END_COMBAT\n";
        let r = scan_text(log);
        assert_eq!(r.fights.len(), 1);
        assert_eq!(r.fights[0].boss_name.as_deref(), Some("Boss"));
    }

    // ── naming carry across chunk passes (E2) ─────────────────────────────────

    // Chunk 1 contains only a ZONE_CHANGED line (no fight yet) — the shape of a
    // live pass that reads the zone change but nothing else. Chunk 2, read later,
    // contains a full fight with no zone line of its own. Without carrying
    // chunk 1's `pending_zone` forward into chunk 2's seed, the fight would come
    // out unnamed even though the zone is known.
    #[test]
    fn scan_chunk_carries_pending_zone_across_passes() {
        let chunk1 = b"5,ZONE_CHANGED,1,\"Cloudrest\",VETERAN\n";
        let scan1 = scan_chunk_for_fights(chunk1, 0, true, None, None);
        assert!(scan1.fights.is_empty());
        assert_eq!(scan1.pending_zone.as_deref(), Some("Cloudrest"));
        assert_eq!(scan1.pending_boss, None);

        let base2 = chunk1.len() as u64;
        let chunk2 = b"100,BEGIN_COMBAT\n5200,END_COMBAT\n";
        let scan2 =
            scan_chunk_for_fights(chunk2, base2, true, scan1.pending_zone, scan1.pending_boss);
        assert_eq!(scan2.fights.len(), 1);
        assert_eq!(
            scan2.fights[0].zone_name.as_deref(),
            Some("Cloudrest"),
            "the zone seen in an earlier pass must still name a fight closed in a later pass"
        );
    }

    // A `BEGIN_LOG` — a fresh session or `/encounterlog` re-enable — must clear
    // any carried naming state, mirroring `Detector::feed`'s `BeginLog` arm.
    // Without this, a zone/boss name from the PREVIOUS session could leak onto a
    // fight logged under a brand new one.
    #[test]
    fn scan_chunk_carry_is_cleared_by_begin_log() {
        let chunk1 = b"5,ZONE_CHANGED,1,\"Cloudrest\",VETERAN\n";
        let scan1 = scan_chunk_for_fights(chunk1, 0, true, None, None);
        assert_eq!(scan1.pending_zone.as_deref(), Some("Cloudrest"));

        let base2 = chunk1.len() as u64;
        let chunk2 = b"0,BEGIN_LOG,1700001000000,15,\"NA\",\"en\",\"x\"\n\
                       5,BEGIN_COMBAT\n15,END_COMBAT\n";
        let scan2 =
            scan_chunk_for_fights(chunk2, base2, true, scan1.pending_zone, scan1.pending_boss);
        assert_eq!(scan2.fights.len(), 1);
        assert_eq!(
            scan2.fights[0].zone_name, None,
            "BEGIN_LOG must clear the carried pending_zone, not leak the prior session's zone"
        );
        assert_eq!(
            scan2.pending_zone, None,
            "the outgoing carry after a BEGIN_LOG must also be cleared"
        );
    }

    // ── tail_session_state (native-live readiness probe) ─────────────────────

    // A tail that ends inside an open session (BEGIN_LOG, no END_LOG) with a fight in
    // progress → logging is active, no fresh header coming. Mid-file peek (drop the
    // leading partial), so prefix a junk partial line and pass drop_first_line=true.
    #[test]
    fn tail_state_open_session_with_fight() {
        let chunk = "…partial junk first line\n\
            100,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live\"\n\
            200,BEGIN_COMBAT\n\
            300,COMBAT_EVENT,DAMAGE,FIRE,1,5,0,1,1,1\n";
        let st = tail_session_state(chunk.as_bytes(), true);
        assert!(st.open_session, "BEGIN_LOG with no END_LOG → open session");
        assert!(
            st.fight_in_progress,
            "BEGIN_COMBAT with no END_COMBAT → fight"
        );
        assert!(st.saw_session_boundary);
        assert!(st.saw_combat_boundary);
    }

    // A tail that ends after END_LOG → logging stopped (next start writes a fresh
    // BEGIN_LOG, so native needs no /reloadui).
    #[test]
    fn tail_state_ended_session() {
        let chunk = "drop me\n\
            100,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live\"\n\
            200,BEGIN_COMBAT\n\
            300,END_COMBAT\n\
            400,END_LOG\n";
        let st = tail_session_state(chunk.as_bytes(), true);
        assert!(!st.open_session, "END_LOG → session closed");
        assert!(!st.fight_in_progress);
        assert!(st.saw_session_boundary);
    }

    // An open session whose last fight already ended → active but between pulls.
    #[test]
    fn tail_state_open_session_between_fights() {
        let chunk = "drop\n\
            100,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live\"\n\
            200,BEGIN_COMBAT\n\
            300,END_COMBAT\n";
        let st = tail_session_state(chunk.as_bytes(), true);
        assert!(st.open_session, "no END_LOG → still in the session");
        assert!(!st.fight_in_progress, "last combat boundary was END_COMBAT");
        assert!(st.saw_session_boundary);
    }

    // Combat boundaries but NO session boundary (the BEGIN_LOG scrolled off the top of
    // the peek window) → saw_combat_boundary but NOT saw_session_boundary, so the
    // caller must treat open_session as unknown (Uncertain), not "logging off".
    #[test]
    fn tail_state_combat_only_no_session_boundary() {
        let chunk = "drop\n\
            300,BEGIN_COMBAT\n\
            301,COMBAT_EVENT,DAMAGE,FIRE,1,5,0,1,1,1\n\
            302,END_COMBAT\n";
        let st = tail_session_state(chunk.as_bytes(), true);
        assert!(
            !st.saw_session_boundary,
            "no BEGIN/END_LOG in the peek → session state is unknown"
        );
        assert!(st.saw_combat_boundary, "combat boundaries were seen");
    }

    // A SMALL whole file read from byte 0: the first line is the real BEGIN_LOG and
    // must NOT be dropped (drop_first_line=false), or we'd miss the session header.
    #[test]
    fn tail_state_byte0_keeps_first_line() {
        let chunk = "0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live\"\n\
            200,BEGIN_COMBAT\n";
        let dropped = tail_session_state(chunk.as_bytes(), true);
        assert!(
            !dropped.saw_session_boundary,
            "dropping the first line would miss the only BEGIN_LOG"
        );
        let kept = tail_session_state(chunk.as_bytes(), false);
        assert!(
            kept.open_session && kept.saw_session_boundary,
            "reading from byte 0 keeps the BEGIN_LOG → open session"
        );
    }

    // ── find_current_session_begin (mid-session warm-up anchor) ──────────────────

    /// Write bytes to a uniquely-named temp file and return its path (caller removes).
    fn temp_log(name: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("kalpa-anchor-{name}.log"));
        std::fs::write(&path, content).unwrap();
        path
    }

    // Two sessions in one file (a /reloadui mid-file): the anchor MUST be the SECOND
    // (most-recent) BEGIN_LOG, never the earlier one — we never cross a /reloadui.
    #[test]
    fn find_current_session_begin_returns_most_recent() {
        let content = "\
0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live\"
200,BEGIN_COMBAT
300,END_COMBAT
400,END_LOG
500,BEGIN_LOG,1700000099999,15,\"NA\",\"en\",\"eso.live\"
600,BEGIN_COMBAT
700,COMBAT_EVENT,DAMAGE,FIRE,1,5,0,1,1,1
";
        let path = temp_log("most-recent", content);
        let eof = std::fs::metadata(&path).unwrap().len();
        let anchor = find_current_session_begin(&path, eof).unwrap().unwrap();
        // The second BEGIN_LOG line's absolute offset.
        let want = content.find("500,BEGIN_LOG").unwrap() as u64;
        assert_eq!(
            anchor.begin_log_offset, want,
            "must pick the MOST-RECENT BEGIN_LOG"
        );
        assert!(anchor.open_session, "second session has no END_LOG → open");
        let _ = std::fs::remove_file(&path);
    }

    // A session that ended in END_LOG → closed (the caller will NOT warm up; it tails
    // from EOF and waits for a fresh header).
    #[test]
    fn find_current_session_begin_reports_closed_after_end_log() {
        let content = "\
0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live\"
200,BEGIN_COMBAT
300,END_COMBAT
400,END_LOG
";
        let path = temp_log("closed", content);
        let eof = std::fs::metadata(&path).unwrap().len();
        let anchor = find_current_session_begin(&path, eof).unwrap().unwrap();
        assert_eq!(anchor.begin_log_offset, 0);
        assert!(!anchor.open_session, "END_LOG → session closed");
        let _ = std::fs::remove_file(&path);
    }

    // No BEGIN_LOG anywhere (e.g. only Interface-style/headerless content) → None, so the
    // caller falls back and NEVER synthesizes a wall clock.
    #[test]
    fn find_current_session_begin_none_when_no_header() {
        let content = "100,SOMETHING,1\n200,OTHER,2\n";
        let path = temp_log("noheader", content);
        let eof = std::fs::metadata(&path).unwrap().len();
        assert!(find_current_session_begin(&path, eof).unwrap().is_none());
        let _ = std::fs::remove_file(&path);
    }

    // A BEGIN_LOG that straddles a 256 KiB window seam must still be found (the backward
    // scan overlaps windows). Pad with filler so the header lands across a window edge.
    #[test]
    fn find_current_session_begin_handles_window_seam() {
        let header = "1234567,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live\"\n";
        // Place the BEGIN_LOG so it spans the first window boundary (ANCHOR_WINDOW),
        // counting back from EOF: lead + header + trailer, with header centered on the
        // seam. Use filler lines (valid-ish, ignored by classify).
        let filler_line = "0,COMBAT_EVENT,DAMAGE,FIRE,1,5,0,1,1,1\n"; // 38 bytes
        let win = ANCHOR_WINDOW as usize;
        // Build a trailer so the header's bytes sit across the boundary `eof - win`.
        let trailer_len = win + 13; // push header start to just before the seam
        let mut trailer = String::new();
        while trailer.len() < trailer_len {
            trailer.push_str(filler_line);
        }
        let lead = filler_line.repeat(win / filler_line.len() + 5);
        let content = format!("{lead}{header}{trailer}");
        let path = temp_log("seam", &content);
        let eof = std::fs::metadata(&path).unwrap().len();
        let anchor = find_current_session_begin(&path, eof).unwrap().unwrap();
        let want = content.find("1234567,BEGIN_LOG").unwrap() as u64;
        assert_eq!(
            anchor.begin_log_offset, want,
            "a BEGIN_LOG straddling a window seam must still be found at its true offset"
        );
        assert!(anchor.open_session);
        let _ = std::fs::remove_file(&path);
    }

    // Empty file → None (nothing to anchor to).
    #[test]
    fn find_current_session_begin_empty_file() {
        let path = temp_log("empty", "");
        assert!(find_current_session_begin(&path, 0).unwrap().is_none());
        let _ = std::fs::remove_file(&path);
    }
}
