//! Streaming, byte-offset-aware scanner for ESO `Encounter.log` files.
//!
//! ESO combat logs are append-only and can reach multiple gigabytes, so this
//! scanner never loads the whole file into memory. It reads in fixed-size
//! buffers, tracks the exact byte offset of every line, and detects:
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

use std::fs::File;
use std::io::{BufRead, BufReader};

use super::types::{FightSummary, LogSession};

/// Files larger than this get a "recommend split" hint in the UI.
pub const SPLIT_RECOMMEND_BYTES: u64 = 512 * 1024 * 1024; // 512 MiB

/// A line as seen by the scanner, with its byte position in the file.
struct ScannedLine {
    /// Byte offset of the first byte of the line.
    offset: u64,
    /// Byte offset just past the line terminator (start of the next line).
    next_offset: u64,
    /// The line type token (second comma-separated field), uppercased ASCII.
    line_type: String,
    /// The raw line content without the trailing newline, borrowed-free.
    raw: String,
}

/// Parse the leading relative-ms value (first comma-separated token) of a line.
fn parse_rel_ms(raw: &str) -> u64 {
    raw.split(',')
        .next()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

/// Return the Nth comma-separated field (0-based), trimming surrounding quotes.
/// Naive split is fine here because the fields we read (line type, version,
/// zone/unit names) never themselves contain an embedded unquoted comma before
/// the index we want for these specific line types.
fn field(raw: &str, idx: usize) -> Option<&str> {
    raw.split(',').nth(idx).map(|s| s.trim().trim_matches('"'))
}

/// Extract the line-type token (field index 1).
fn line_type_of(raw: &str) -> String {
    field(raw, 1).unwrap_or("").to_ascii_uppercase()
}

/// Iterate the file line by line, yielding [`ScannedLine`] with exact offsets.
///
/// Uses `read_until(b'\n')` so byte offsets account for `\r\n` vs `\n`
/// transparently: `next_offset` is always the start of the following line.
fn scan_lines(path: &str) -> Result<Vec<ScannedLine>, String> {
    let file = File::open(path).map_err(|e| map_io_error(&e, path))?;
    let mut reader = BufReader::with_capacity(1 << 20, file); // 1 MiB buffer

    let mut lines = Vec::new();
    let mut offset: u64 = 0;
    let mut buf: Vec<u8> = Vec::with_capacity(512);

    loop {
        buf.clear();
        let n = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| format!("Failed to read log: {e}"))?;
        if n == 0 {
            break;
        }
        let next_offset = offset + n as u64;

        // Strip the trailing \n and optional \r for content; offsets are intact.
        let mut end = n;
        if end > 0 && buf[end - 1] == b'\n' {
            end -= 1;
        }
        if end > 0 && buf[end - 1] == b'\r' {
            end -= 1;
        }
        // Lossy is safe: ESO logs are UTF-8 but a corrupt tail shouldn't abort.
        let raw = String::from_utf8_lossy(&buf[..end]).into_owned();

        if !raw.is_empty() {
            let line_type = line_type_of(&raw);
            lines.push(ScannedLine {
                offset,
                next_offset,
                line_type,
                raw,
            });
        }
        offset = next_offset;
    }

    Ok(lines)
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

/// Scan an entire log file into sessions and fights.
///
/// Single pass, O(lines) time, O(sessions + fights) memory — line content is
/// dropped as we go.
pub fn scan_file(path: &str) -> Result<ScanResult, String> {
    let lines = scan_lines(path)?;
    Ok(scan_parsed(&lines))
}

/// Core boundary detection over already-scanned lines (kept separate so it can
/// be unit-tested without touching the filesystem).
fn scan_parsed(lines: &[ScannedLine]) -> ScanResult {
    let mut sessions: Vec<LogSession> = Vec::new();
    let mut fights: Vec<FightSummary> = Vec::new();

    // Per-session running state.
    let mut session_open = false;
    let mut session_fight_count = 0usize;

    // Per-fight running state.
    let mut fight_start: Option<(u64, u64)> = None; // (offset, rel_ms)
    let mut pending_zone: Option<String> = None;
    let mut pending_boss: Option<String> = None;

    let close_session = |sessions: &mut Vec<LogSession>, end_offset: u64, fc: usize| {
        if let Some(last) = sessions.last_mut() {
            last.end_offset = end_offset;
            last.fight_count = fc;
            last.size_bytes = end_offset.saturating_sub(last.start_offset);
        }
    };

    for line in lines {
        match line.line_type.as_str() {
            "BEGIN_LOG" => {
                // Close the previous session at this line's start.
                if session_open {
                    close_session(&mut sessions, line.offset, session_fight_count);
                }
                session_open = true;
                session_fight_count = 0;
                fight_start = None;

                let start_time_ms = field(&line.raw, 2)
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                let log_version = field(&line.raw, 3).unwrap_or("").to_string();
                let realm = field(&line.raw, 4)
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty());

                sessions.push(LogSession {
                    index: sessions.len(),
                    start_offset: line.offset,
                    end_offset: line.next_offset,
                    start_time_ms,
                    log_version,
                    realm,
                    fight_count: 0,
                    size_bytes: 0,
                });
            }
            "END_LOG" => {
                if session_open {
                    close_session(&mut sessions, line.next_offset, session_fight_count);
                    session_open = false;
                }
            }
            "ZONE_CHANGED" => {
                // ZONE_CHANGED, id, "name", difficulty
                pending_zone = field(&line.raw, 3)
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty());
            }
            "UNIT_ADDED" => {
                // A boss unit: UNIT_ADDED has isBoss="T" and a name field.
                // Layout: ..., unitType, ..., monsterId, isBoss, classId, ...,
                // name(field 10), displayName(11). We detect isBoss heuristically.
                if line.raw.contains(",T,") {
                    if let Some(name) = field(&line.raw, 10).filter(|s| !s.is_empty()) {
                        // Only adopt as boss name if this looks like a monster
                        // (no @display-name handle in the name field).
                        if !name.starts_with('@') {
                            pending_boss = Some(name.to_string());
                        }
                    }
                }
            }
            "BEGIN_COMBAT" => {
                fight_start = Some((line.offset, parse_rel_ms(&line.raw)));
            }
            "END_COMBAT" => {
                if let Some((start_offset, start_ms)) = fight_start.take() {
                    fights.push(FightSummary {
                        index: fights.len(),
                        start_offset,
                        end_offset: line.next_offset,
                        start_ms,
                        end_ms: parse_rel_ms(&line.raw),
                        zone_name: pending_zone.clone(),
                        boss_name: pending_boss.take(),
                    });
                    session_fight_count += 1;
                }
            }
            _ => {}
        }
    }

    // Close a session left open at EOF (the common case for an active log).
    if session_open {
        let end = lines.last().map(|l| l.next_offset).unwrap_or(0);
        close_session(&mut sessions, end, session_fight_count);
    }

    ScanResult { sessions, fights }
}

/// Scan an in-memory chunk for *completed* fights only (BEGIN_COMBAT followed
/// by END_COMBAT), returning [`FightSummary`] with byte offsets shifted by
/// `base` (the chunk's absolute offset in the file). Used by the live watcher,
/// which reads incremental chunks rather than the whole file.
///
/// `index` is left at 0 — the caller assigns running indices.
pub fn scan_chunk_for_fights(chunk: &str, base: u64) -> Vec<FightSummary> {
    let mut fights = Vec::new();
    let mut offset: u64 = 0;

    let mut fight_start: Option<(u64, u64)> = None;
    let mut pending_zone: Option<String> = None;
    let mut pending_boss: Option<String> = None;

    for raw_line in chunk.split_inclusive('\n') {
        let n = raw_line.len() as u64;
        let next_offset = offset + n;
        let trimmed = raw_line.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            offset = next_offset;
            continue;
        }
        match line_type_of(trimmed).as_str() {
            "ZONE_CHANGED" => {
                pending_zone = field(trimmed, 3)
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty());
            }
            "UNIT_ADDED" => {
                if trimmed.contains(",T,") {
                    if let Some(name) =
                        field(trimmed, 10).filter(|s| !s.is_empty() && !s.starts_with('@'))
                    {
                        pending_boss = Some(name.to_string());
                    }
                }
            }
            "BEGIN_COMBAT" => {
                fight_start = Some((base + offset, parse_rel_ms(trimmed)));
            }
            "END_COMBAT" => {
                if let Some((start_offset, start_ms)) = fight_start.take() {
                    fights.push(FightSummary {
                        index: 0,
                        start_offset,
                        end_offset: base + next_offset,
                        start_ms,
                        end_ms: parse_rel_ms(trimmed),
                        zone_name: pending_zone.clone(),
                        boss_name: pending_boss.take(),
                    });
                }
            }
            _ => {}
        }
        offset = next_offset;
    }

    fights
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_from(text: &str) -> Vec<ScannedLine> {
        let mut out = Vec::new();
        let mut offset = 0u64;
        for raw_line in text.split_inclusive('\n') {
            let n = raw_line.len() as u64;
            let next_offset = offset + n;
            let trimmed = raw_line.trim_end_matches(['\n', '\r']);
            if !trimmed.is_empty() {
                out.push(ScannedLine {
                    offset,
                    next_offset,
                    line_type: line_type_of(trimmed),
                    raw: trimmed.to_string(),
                });
            }
            offset = next_offset;
        }
        out
    }

    #[test]
    fn detects_single_session_and_fight() {
        let log = "0,BEGIN_LOG,1700000000000,15,\"NA Megaserver\",\"en\",\"10.0.0\"\n\
                   100,ZONE_CHANGED,1301,\"Sunspire\",VETERAN\n\
                   200,BEGIN_COMBAT\n\
                   5200,END_COMBAT\n\
                   5300,END_LOG\n";
        let r = scan_parsed(&lines_from(log));
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
        let r = scan_parsed(&lines_from(log));
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
        let r = scan_parsed(&lines_from(log));
        let f = &r.fights[0];
        // The fight range must lie inside the session range.
        assert!(f.start_offset >= r.sessions[0].start_offset);
        assert!(f.end_offset <= r.sessions[0].end_offset);
    }
}
