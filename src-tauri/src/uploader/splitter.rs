//! Splitting an oversized `Encounter.log` into per-session files on disk.
//!
//! ESO appends every play session to one file; ESO Logs' own uploader chokes on
//! multi-GB files. Splitting on `BEGIN_LOG` boundaries yields self-contained,
//! individually-uploadable logs (each session already starts with its own
//! `BEGIN_LOG` header), which is the cleanest way to make a giant file usable.
//!
//! Copies are streamed in fixed buffers so memory stays flat regardless of size.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::scanner;
use super::types::LogSession;

/// 8 MiB copy buffer — large enough to keep IO efficient, small enough to stay
/// off the stack and bounded in memory.
const COPY_BUF: usize = 8 * 1024 * 1024;

/// Copy a byte range `[start, end)` from `src` into a new file at `dst`.
fn copy_range(src: &Path, dst: &Path, start: u64, end: u64) -> Result<(), String> {
    if end <= start {
        return Err("Empty byte range".into());
    }
    let mut reader = BufReader::new(File::open(src).map_err(|e| format!("Open source: {e}"))?);
    reader
        .seek(SeekFrom::Start(start))
        .map_err(|e| format!("Seek: {e}"))?;

    let mut writer = BufWriter::new(File::create(dst).map_err(|e| format!("Create output: {e}"))?);

    let mut remaining = end - start;
    let mut buf = vec![0u8; COPY_BUF];
    while remaining > 0 {
        let want = remaining.min(COPY_BUF as u64) as usize;
        let n = reader
            .read(&mut buf[..want])
            .map_err(|e| format!("Read: {e}"))?;
        if n == 0 {
            // The source shrank/rotated mid-copy (e.g. /reloadui+relog on the
            // active log). A short copy would be a silently-truncated, corrupt
            // session file — fail loudly and remove the partial output rather
            // than report success.
            let _ = writer.flush();
            drop(writer);
            let _ = std::fs::remove_file(dst);
            return Err(format!(
                "Source log shrank during copy ({remaining} bytes missing) — \
                 it may have been rotated. Try again."
            ));
        }
        writer
            .write_all(&buf[..n])
            .map_err(|e| format!("Write: {e}"))?;
        remaining -= n as u64;
    }
    writer.flush().map_err(|e| format!("Flush: {e}"))?;
    Ok(())
}

/// Cheaply verify that the supplied preflight offsets still describe THIS file
/// (not a different one written after a truncate-and-regrow). Reads the first
/// session's leading bytes and confirms they are a `BEGIN_LOG` line carrying the
/// same `start_time_ms` the preflight recorded. A truncate-to-0-then-regrow can
/// leave `snapshot_len >= max_end` (so the length check passes) while the byte
/// boundaries now point at unrelated data — this catches that case. Returns
/// false on any read error or mismatch so the caller falls back to a re-scan.
fn offsets_still_valid(src: &Path, first: &LogSession) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = File::open(src) else {
        return false;
    };
    if f.seek(SeekFrom::Start(first.start_offset)).is_err() {
        return false;
    }
    // A BEGIN_LOG line is short; 512 bytes is ample to cover the timestamp field.
    let mut buf = [0u8; 512];
    let n = match f.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };
    let head = &buf[..n];
    // First line only (BEGIN_LOG lines are `0,BEGIN_LOG,<start_time_ms>,…`).
    let line_end = head.iter().position(|b| *b == b'\n').unwrap_or(head.len());
    let Ok(line) = std::str::from_utf8(&head[..line_end]) else {
        return false;
    };
    let mut fields = line.split(',');
    // field[0] is a relative time offset, field[1] the event name.
    if fields.next().is_none() {
        return false;
    }
    if !matches!(fields.next(), Some(name) if name.eq_ignore_ascii_case("BEGIN_LOG")) {
        return false;
    }
    // field[2] is the absolute start time; it must match the preflight value.
    matches!(fields.next(), Some(ts) if ts.trim() == first.start_time_ms.to_string())
}

/// Produce a stable, filesystem-safe name for a session's split file.
fn session_file_name(stem: &str, session: &LogSession) -> String {
    // Anchor on the absolute start time so names are sortable and unique.
    format!(
        "{stem}-session{:02}-{}.log",
        session.index + 1,
        session.start_time_ms
    )
}

/// Split `source_path` into one file per logging session inside `out_dir`.
///
/// `sessions` may be supplied from a prior preflight scan to avoid a second full
/// pass over a multi-GB file; pass `None` to scan here.
///
/// Returns the paths of the files written, in session order. Sessions with no
/// fights are still written (they may contain useful context), but the caller
/// can filter on the [`LogSession::fight_count`] it already has from a preflight.
pub fn split_by_session(
    source_path: &str,
    out_dir: &str,
    sessions: Option<Vec<LogSession>>,
) -> Result<Vec<String>, String> {
    let src = Path::new(source_path);
    if !src.is_file() {
        return Err(format!("Source log not found: {source_path}"));
    }
    let out = PathBuf::from(out_dir);
    std::fs::create_dir_all(&out).map_err(|e| format!("Create output dir: {e}"))?;

    // The active Encounter.log may still be growing as ESO appends. Snapshot the
    // length and clamp every copy to it, so a session whose `end_offset` reached
    // the (moving) EOF is copied only up to bytes that definitely existed —
    // never a torn read past the snapshot.
    let snapshot_len = std::fs::metadata(src)
        .map_err(|e| format!("Failed to stat source: {e}"))?
        .len();

    let sessions = match sessions {
        Some(s) if !s.is_empty() => {
            // Caller-supplied offsets are from preflight time. They're only safe
            // to trust if the file hasn't been rewritten since:
            //  - shorter than the sessions' max end  → it shrank/rotated; OR
            //  - a truncate-to-0-then-regrow past max_end leaves the length check
            //    passing while the byte boundaries now point at unrelated data.
            // Re-scan for a consistent list unless the length is still ≥ the max
            // end AND the first session's bytes still classify as the same
            // BEGIN_LOG (same start_time_ms) the preflight recorded.
            let max_end = s.iter().map(|x| x.end_offset).max().unwrap_or(0);
            if snapshot_len < max_end || !offsets_still_valid(src, &s[0]) {
                scanner::scan_file(source_path)?.sessions
            } else {
                s
            }
        }
        _ => scanner::scan_file(source_path)?.sessions,
    };
    if sessions.is_empty() {
        return Err("No logging sessions found in this file.".into());
    }

    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Encounter");

    let last_index = sessions.len() - 1;
    let mut written = Vec::with_capacity(sessions.len());
    for (i, session) in sessions.iter().enumerate() {
        // Completed sessions end at a real BEGIN_LOG/END_LOG boundary that never
        // moves, so their stale `end_offset` is correct (clamped to the snapshot
        // for safety). The FINAL session may still be open and growing: its
        // preflight `end_offset` was the EOF at scan time, so anything ESO
        // appended since would be silently dropped. Extend it to the current EOF
        // (snapshot_len) so those later fights are included (L3).
        let end = if i == last_index {
            session.end_offset.max(snapshot_len)
        } else {
            session.end_offset
        }
        .min(snapshot_len);
        if end <= session.start_offset {
            continue; // session lies entirely past the snapshot (shouldn't happen)
        }
        let dst = out.join(session_file_name(stem, session));
        copy_range(src, &dst, session.start_offset, end)?;
        written.push(dst.to_string_lossy().into_owned());
    }
    Ok(written)
}
