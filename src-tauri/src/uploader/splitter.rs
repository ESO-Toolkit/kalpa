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

    let sessions = match sessions {
        Some(s) if !s.is_empty() => s,
        _ => scanner::scan_file(source_path)?.sessions,
    };
    if sessions.is_empty() {
        return Err("No logging sessions found in this file.".into());
    }

    // The active Encounter.log may still be growing as ESO appends. Snapshot the
    // length and clamp every copy to it, so a session whose `end_offset` reached
    // the (moving) EOF is copied only up to bytes that definitely existed —
    // never a torn read past the snapshot.
    let snapshot_len = std::fs::metadata(src)
        .map_err(|e| format!("Failed to stat source: {e}"))?
        .len();

    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Encounter");

    let mut written = Vec::with_capacity(sessions.len());
    for session in &sessions {
        let end = session.end_offset.min(snapshot_len);
        if end <= session.start_offset {
            continue; // session lies entirely past the snapshot (shouldn't happen)
        }
        let dst = out.join(session_file_name(stem, session));
        copy_range(src, &dst, session.start_offset, end)?;
        written.push(dst.to_string_lossy().into_owned());
    }
    Ok(written)
}
