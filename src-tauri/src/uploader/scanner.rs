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
use std::io::{BufRead, BufReader};

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
                // A boss unit has isBoss="T"; name is field 10. Skip @handles
                // (player display names) so we only adopt monster names.
                if line.contains(",T,") {
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
                    let index = self.fights.len();
                    self.fights.push(FightSummary {
                        index,
                        start_offset,
                        end_offset: next_offset,
                        start_ms,
                        end_ms: parse_rel_ms(line),
                        zone_name: self.pending_zone.clone(),
                        boss_name: self.pending_boss.take(),
                    });
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
    let file = File::open(path).map_err(|e| map_io_error(&e, path))?;
    let mut reader = BufReader::with_capacity(1 << 20, file); // 1 MiB buffer

    let mut detector = Detector::default();
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
pub fn scan_chunk_for_fights(chunk: &[u8], base: u64, session_already_open: bool) -> ChunkScan {
    let mut detector = Detector {
        session_open: session_already_open,
        ..Default::default()
    };
    let mut offset: u64 = 0;

    for raw_line in chunk.split_inclusive(|b| *b == b'\n') {
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

        let scan = scan_chunk_for_fights(&chunk, 0, true);
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
        let scan = scan_chunk_for_fights(chunk, 0, true);
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
        let scan = scan_chunk_for_fights(&chunk, 0, true);
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
        let scan = scan_chunk_for_fights(chunk, 0, false);
        assert_eq!(scan.new_session_at, None);
        assert_eq!(scan.fights.len(), 1);
    }
}
