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

/// The offset of the first byte of the line containing `at` — i.e. one past the
/// nearest `\n` at or before `at`, or 0 if there is none. Lets the appended-tail
/// scan begin at a true line start even when `at` (a preflight `end_offset`)
/// landed mid-line, which the full-file scanner can do: it feeds an unterminated
/// trailing line and `finish()` sets the final session end to EOF, so `max_end`
/// can sit inside a partially-flushed `0,BEGIN_LOG,…` header. Scanning from a
/// real line start re-reads that line once it completes.
fn line_start_at_or_before(src: &Path, at: u64) -> Option<u64> {
    use std::io::{Read, Seek, SeekFrom};
    if at == 0 {
        return Some(0);
    }
    let mut f = File::open(src).ok()?;
    // Walk backwards in modest windows looking for the last `\n` strictly before
    // `at`; the line starts one byte after it.
    const WIN: u64 = 64 * 1024;
    let mut end = at;
    let mut buf = vec![0u8; WIN as usize];
    while end > 0 {
        let start = end.saturating_sub(WIN);
        let len = (end - start) as usize;
        f.seek(SeekFrom::Start(start)).ok()?;
        f.read_exact(&mut buf[..len]).ok()?;
        if let Some(pos) = buf[..len].iter().rposition(|b| *b == b'\n') {
            return Some(start + pos as u64 + 1);
        }
        end = start;
    }
    Some(0) // no newline before `at` → the first line starts at 0
}

/// Whether the bytes the file grew by since preflight (`[from, to)`) contain a
/// new `BEGIN_LOG` — i.e. ESO started a fresh logging session in the active file
/// after the preflight scan. If so the stale session list is incomplete (the
/// final session would be extended over the new session's bytes), so the caller
/// must re-scan. Reads only the appended tail, line by line at constant memory.
/// Returns `true` (force a re-scan, the safe choice) on any read error.
fn appended_range_has_new_session(src: &Path, from: u64, to: u64) -> bool {
    use std::io::{BufRead, Seek, SeekFrom};
    if to <= from {
        return false;
    }
    let Ok(f) = File::open(src) else {
        return true;
    };
    let mut reader = BufReader::with_capacity(1 << 20, f);
    // `from` is the previous final session's `end_offset`. That is USUALLY a line
    // boundary, but the full-file scanner can set it to EOF mid-line when preflight
    // caught a partially-flushed trailing line (e.g. `0,BEG` of a forming
    // `0,BEGIN_LOG,…`). So begin at the true start of the line containing `from`,
    // not at `from` itself — otherwise we'd read the completed header's tail
    // (`IN_LOG,…`), miss the `,BEGIN_LOG` token, and wrongly trust the stale list.
    // Re-reading that one line is harmless: it was already part of the prior
    // session's range, so detecting its (now-complete) BEGIN_LOG correctly forces
    // a re-scan. We scan the ENTIRE tail (no cap) line-by-line at constant memory,
    // early-exiting on the first BEGIN_LOG.
    let scan_from = line_start_at_or_before(src, from).unwrap_or(0);
    if reader.seek(SeekFrom::Start(scan_from)).is_err() {
        return true;
    }
    let mut line: Vec<u8> = Vec::with_capacity(256);
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(_) => return true, // read error → fail closed (force re-scan)
        }
        // Each read_until call yields exactly one whole line (even across the
        // buffer boundary), so a BEGIN_LOG can't be torn. ESO log lines are
        // `<relMs>,<TYPE>,…`; a session header has `BEGIN_LOG` as the second
        // field, so the line contains the `,BEGIN_LOG` token (the leading comma
        // anchors it to a field start, avoiding a substring false-positive).
        if line
            .windows(10)
            .any(|w| w.eq_ignore_ascii_case(b",BEGIN_LOG"))
        {
            return true;
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

    // The new BEGIN_LOG can appear FAR into the append (the same session can grow
    // many MiB before the user re-toggles logging). The appended-tail scan must
    // not stop early and miss it — a bounded scan would wrongly trust the stale
    // list and merge the new session. Append >8 MiB of same-session data before
    // the new BEGIN_LOG and assert two separate files.
    #[test]
    fn new_session_deep_in_append_still_forces_rescan() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Encounter.log");
        let out = tmp.path().join("out");

        let sess_a =
            b"0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n";
        write(&log, sess_a);
        let a_end = sess_a.len() as u64;
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

        // >8 MiB of same-session filler lines (past the old MAX_SCAN cap), THEN a
        // new session's BEGIN_LOG.
        let mut full = sess_a.to_vec();
        let filler_line = b"30,COMBAT_EVENT,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15\n";
        while full.len() < a_end as usize + 9 * 1024 * 1024 {
            full.extend_from_slice(filler_line);
        }
        full.extend_from_slice(
            b"0,BEGIN_LOG,2000,15,\"NA\",\"en\",\"10.0\"\n40,BEGIN_COMBAT\n50,END_COMBAT\n",
        );
        write(&log, &full);

        let written =
            split_by_session(log.to_str().unwrap(), out.to_str().unwrap(), Some(stale)).unwrap();
        assert_eq!(
            written.len(),
            2,
            "a new BEGIN_LOG deep in the append must still split into two files"
        );
        // Session A's file must not contain session B's bytes.
        let first = std::fs::read(&written[0]).unwrap();
        assert!(
            !first.windows(4).any(|w| w == b"2000"),
            "session A's split leaked session B's content"
        );
    }

    // Preflight can land while ESO has flushed only PART of the next session's
    // header (e.g. `0,BEG`), so the full-file scanner sets the final session's
    // end_offset mid-token (to EOF). After the line completes to a real
    // `0,BEGIN_LOG,…`, the appended scan must still detect it — by starting at the
    // line boundary, not at the mid-token offset. Otherwise it reads `IN_LOG,…`,
    // sees no `,BEGIN_LOG`, and merges the new session into session A's file.
    #[test]
    fn partial_begin_log_at_preflight_eof_still_forces_rescan() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Encounter.log");
        let out = tmp.path().join("out");

        let sess_a =
            b"0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n";
        // Preflight caught a partially-flushed next header: file was sess_a + "0,BEG".
        let partial = b"0,BEG";
        let preflight_eof = sess_a.len() as u64 + partial.len() as u64;
        // The scanner sets the open final session's end_offset to that EOF — i.e.
        // mid-token, inside the forming BEGIN_LOG line.
        let stale = vec![LogSession {
            index: 0,
            start_offset: 0,
            end_offset: preflight_eof,
            start_time_ms: 1000,
            log_version: "15".into(),
            realm: Some("NA".into()),
            fight_count: 1,
            size_bytes: preflight_eof,
        }];

        // Now the line completes into a real new session B and more is appended.
        let mut full = sess_a.to_vec();
        full.extend_from_slice(
            b"0,BEGIN_LOG,2000,15,\"NA\",\"en\",\"10.0\"\n40,BEGIN_COMBAT\n50,END_COMBAT\n",
        );
        write(&log, &full);

        let written =
            split_by_session(log.to_str().unwrap(), out.to_str().unwrap(), Some(stale)).unwrap();
        assert_eq!(
            written.len(),
            2,
            "a BEGIN_LOG torn at the preflight EOF must still split into two files"
        );
        let first = std::fs::read(&written[0]).unwrap();
        assert!(
            !first.windows(4).any(|w| w == b"2000"),
            "session A's split leaked session B's content"
        );
    }
}
