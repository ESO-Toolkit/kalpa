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

/// Whether the bytes the file grew by since preflight (`[from, to)`) contain a
/// new `BEGIN_LOG` — i.e. ESO started a fresh logging session in the active file
/// after the preflight scan. If so the stale session list is incomplete (it
/// would extend the previous session's split to cover the new session's bytes),
/// so the caller must re-scan. Reads only the appended tail, not the whole file.
/// Returns `true` (force a re-scan, the safe choice) on any read error.
fn appended_range_has_new_session(src: &Path, from: u64, to: u64) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    if to <= from {
        return false;
    }
    let Ok(mut f) = File::open(src) else {
        return true;
    };
    // Start one byte before `from` so a `BEGIN_LOG` line that begins exactly at
    // `from` is seen whole (its leading newline is just before `from`). Clamp to 0.
    let start = from.saturating_sub(1);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return true;
    }
    // Bound the work: only the appended span matters, and BEGIN_LOG appears at a
    // line start, so reading the appended bytes in a modest buffer and scanning
    // for the token is enough. Cap the scan so an enormous append can't read GBs;
    // a new session's BEGIN_LOG, if present, is at the very start of the append.
    const MAX_SCAN: u64 = 8 * 1024 * 1024;
    let span = (to - start).min(MAX_SCAN);
    let mut remaining = span as usize;
    let mut buf = vec![0u8; 1 << 20]; // 1 MiB
                                      // Track whether we are at a line start (the byte before the current buffer
                                      // was a newline) so a "BEGIN_LOG" mid-field can't false-positive.
    let mut at_line_start = false; // `start` is mid-line (one byte before `from`)
    while remaining > 0 {
        let want = remaining.min(buf.len());
        let n = match f.read(&mut buf[..want]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => return true,
        };
        remaining -= n;
        let chunk = &buf[..n];
        for line in chunk.split_inclusive(|b| *b == b'\n') {
            if at_line_start {
                // ESO log lines are `<relMs>,<TYPE>,…`; a session header is
                // `…,BEGIN_LOG,…` as the second field. Cheap check: the line
                // contains ",BEGIN_LOG" (the leading comma anchors the field).
                if line
                    .windows(10)
                    .any(|w| w.eq_ignore_ascii_case(b",BEGIN_LOG"))
                {
                    return true;
                }
            }
            at_line_start = line.last() == Some(&b'\n');
        }
    }
    false
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
            // Re-scan for a consistent list unless: the length is still ≥ the max
            // end, the first session's bytes still classify as the same BEGIN_LOG
            // (same start_time_ms) the preflight recorded, AND nothing the file
            // grew by since preflight starts a NEW session. That last check is
            // load-bearing: if ESO appended a fresh `BEGIN_LOG` after preflight,
            // the final session below would be extended to `snapshot_len` and
            // swallow the new session's bytes into the previous session's file —
            // a wrong split. A new BEGIN_LOG in `[max_end, snapshot_len)` forces a
            // re-scan so each session gets its own file.
            let max_end = s.iter().map(|x| x.end_offset).max().unwrap_or(0);
            if snapshot_len < max_end
                || !offsets_still_valid(src, &s[0])
                || appended_range_has_new_session(src, max_end, snapshot_len)
            {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(path: &Path, bytes: &[u8]) {
        let mut f = File::create(path).unwrap();
        f.write_all(bytes).unwrap();
    }

    // A preflight that saw ONE session must not, after the file appends a SECOND
    // session, merge the new session's bytes into the first session's split file.
    // The stale-offset trust path must detect the appended BEGIN_LOG and re-scan.
    #[test]
    fn appended_session_after_preflight_forces_rescan_into_separate_files() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Encounter.log");
        let out = tmp.path().join("out");

        // Session A only (what preflight saw).
        let sess_a = b"0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n\
                       10,BEGIN_COMBAT\n20,END_COMBAT\n";
        write(&log, sess_a);
        let a_end = sess_a.len() as u64;

        // The stale preflight list: one open session ending at the then-EOF.
        let stale = vec![LogSession {
            index: 0,
            start_offset: 0,
            end_offset: a_end,
            start_time_ms: 1000,
            log_version: "15".into(),
            realm: Some("NA".into()),
            fight_count: 1,
            size_bytes: a_end,
        }];

        // Now ESO appends a brand-new logging session B to the same file.
        let sess_b = b"0,BEGIN_LOG,2000,15,\"NA\",\"en\",\"10.0\"\n\
                       10,BEGIN_COMBAT\n20,END_COMBAT\n";
        let mut full = sess_a.to_vec();
        full.extend_from_slice(sess_b);
        write(&log, &full);

        let written =
            split_by_session(log.to_str().unwrap(), out.to_str().unwrap(), Some(stale)).unwrap();

        // Must produce TWO files (one per session), not one merged file.
        assert_eq!(written.len(), 2, "expected a separate file per session");

        // The first session's file must NOT contain session B's BEGIN_LOG bytes.
        let first = std::fs::read(&written[0]).unwrap();
        assert!(
            !first.windows(b"2000".len()).any(|w| w == b"2000"),
            "session A's split leaked session B's content"
        );
    }

    // Sanity: when the file only GREW the final (open) session — no new
    // BEGIN_LOG — the stale list is still trusted (fast path) and the final
    // session extends to include the appended fights.
    #[test]
    fn appended_within_same_session_keeps_fast_path_single_file() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Encounter.log");
        let out = tmp.path().join("out");

        let base = b"0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n";
        write(&log, base);
        let base_end = base.len() as u64;
        let stale = vec![LogSession {
            index: 0,
            start_offset: 0,
            end_offset: base_end,
            start_time_ms: 1000,
            log_version: "15".into(),
            realm: Some("NA".into()),
            fight_count: 1,
            size_bytes: base_end,
        }];

        // Append more fights to the SAME session (no new BEGIN_LOG).
        let mut full = base.to_vec();
        full.extend_from_slice(b"30,BEGIN_COMBAT\n40,END_COMBAT\n");
        write(&log, &full);

        let written =
            split_by_session(log.to_str().unwrap(), out.to_str().unwrap(), Some(stale)).unwrap();
        assert_eq!(written.len(), 1, "one session should stay one file");
        // The single file includes the appended fights (extended to EOF).
        let bytes = std::fs::read(&written[0]).unwrap();
        assert_eq!(bytes.len() as u64, full.len() as u64);
    }
}
