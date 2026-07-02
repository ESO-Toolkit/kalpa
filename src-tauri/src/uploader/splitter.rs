//! Splitting an oversized `Encounter.log` into per-session files on disk.
//!
//! ESO appends every play session to one file; ESO Logs' own uploader chokes on
//! multi-GB files. Splitting on `BEGIN_LOG` boundaries yields self-contained,
//! individually-uploadable logs (each session already starts with its own
//! `BEGIN_LOG` header), which is the cleanest way to make a giant file usable.
//!
//! Copies are streamed in fixed buffers so memory stays flat regardless of size.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::scanner;
use super::types::{FightSummary, LogSession};

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

fn latest_session_file_name(stem: &str, start_time_ms: u64) -> String {
    if start_time_ms == 0 {
        format!("{stem}-latest-session.log")
    } else {
        format!("{stem}-latest-session-{start_time_ms}.log")
    }
}

fn begin_log_start_time_at(src: &Path, offset: u64) -> Result<u64, String> {
    let mut reader = BufReader::new(File::open(src).map_err(|e| format!("Open source: {e}"))?);
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|e| format!("Seek: {e}"))?;
    let mut line = Vec::with_capacity(256);
    reader
        .read_until(b'\n', &mut line)
        .map_err(|e| format!("Read BEGIN_LOG: {e}"))?;
    if line.ends_with(b"\n") {
        line.pop();
    }
    if line.ends_with(b"\r") {
        line.pop();
    }
    let text = String::from_utf8_lossy(&line);
    let mut fields = text.split(',');
    let _rel = fields.next();
    if !matches!(fields.next(), Some(name) if name.eq_ignore_ascii_case("BEGIN_LOG")) {
        return Err("Latest session anchor was not a BEGIN_LOG line.".into());
    }
    Ok(fields
        .next()
        .and_then(|ts| ts.trim().parse::<u64>().ok())
        .unwrap_or(0))
}

/// Split only the latest/current logging session without scanning the whole file.
///
/// This is the fast path for the common "upload the one instance I just ran" workflow:
/// scan backward to the most recent `BEGIN_LOG`, then stream-copy `[BEGIN_LOG, EOF)` at
/// a single file-length snapshot. It deliberately does not build the full sessions/fights
/// index; users who need older sessions or per-fight carving can still run preflight and
/// open the full workbench.
pub fn split_latest_session(source_path: &str, out_dir: &str) -> Result<Vec<String>, String> {
    let src = Path::new(source_path);
    if !src.is_file() {
        return Err(format!("Source log not found: {source_path}"));
    }
    let out = PathBuf::from(out_dir);
    std::fs::create_dir_all(&out).map_err(|e| format!("Create output dir: {e}"))?;

    let snapshot_len = std::fs::metadata(src)
        .map_err(|e| format!("Failed to stat source: {e}"))?
        .len();
    let begin_log_offset = scanner::find_latest_session_begin(src, snapshot_len)?
        .ok_or_else(|| "No logging sessions found in this file.".to_string())?;
    if snapshot_len <= begin_log_offset {
        return Err("Latest logging session is empty.".into());
    }

    let start_time_ms = begin_log_start_time_at(src, begin_log_offset)?;
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Encounter");
    let dst = out.join(latest_session_file_name(stem, start_time_ms));
    copy_range(src, &dst, begin_log_offset, snapshot_len)?;
    Ok(vec![dst.to_string_lossy().into_owned()])
}

/// Sanitize a user-supplied split name into a safe, single-segment file stem.
///
/// The UI lets users name each split (e.g. "core-prog lucent hm"). That string
/// reaches the filesystem, so it must never enable path traversal or produce an
/// invalid name. We keep only a conservative allowlist (alphanumerics, space,
/// `-`, `_`, `.`), collapse whitespace to single `-`, strip leading/trailing
/// separators and dots, cap the length, and reject anything that reduces to
/// empty — the caller then falls back to the stable auto name. The `.log`
/// extension is appended by the caller, never taken from user input.
pub fn sanitize_split_stem(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut last_was_sep = false;
    for ch in raw.trim().chars() {
        let keep = if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
            last_was_sep = false;
            Some(ch)
        } else if ch == '-' || ch == '/' || ch == '\\' || ch.is_whitespace() {
            // Collapse runs of separators/whitespace/path-separators into a single
            // '-'. Treating `/` and `\` as word separators (not dropping them)
            // keeps names predictable ("lucent/hm" → "lucent-hm") while still
            // preventing any real path segment from surviving.
            if last_was_sep {
                None
            } else {
                last_was_sep = true;
                Some('-')
            }
        } else {
            // Drop anything else (colons, control chars, unicode punctuation, …).
            None
        };
        if let Some(c) = keep {
            out.push(c);
        }
        // Cap the stem so a pathological name can't blow past filesystem limits.
        if out.len() >= 80 {
            break;
        }
    }
    // Trim separators/dots from the ends so we never produce ".", "..", a hidden
    // dotfile, or a trailing-dot name (invalid on Windows).
    let trimmed = out.trim_matches(|c| c == '-' || c == '.' || c == '_');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// A user's choice for one session in the split workbench: which session
/// (`index`, matching [`LogSession::index`]) and an optional custom name. Only
/// sessions present in the selection are written, so the UI can drop empty or
/// unwanted sessions.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitSelection {
    /// The [`LogSession::index`] this selection refers to.
    pub index: usize,
    /// A user-supplied name (sanitized before use); falls back to the auto name.
    pub name: Option<String>,
    /// Absolute byte offset of the session's `BEGIN_LOG` line. When present, this
    /// is preferred over `index` so latest-session-only preflights (whose indices
    /// are local to that bounded scan) still resolve correctly after a full rescan.
    pub start_offset: Option<u64>,
    /// The session's `start_time_ms` at selection time. When present, it is
    /// verified against the resolved session so a rescan (the log was
    /// truncated/rotated between preflight and split) that shifted indices is
    /// caught instead of silently writing a different session under this name.
    pub start_time_ms: Option<u64>,
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

    let (sessions, snapshot_len) = resolve_sessions(src, source_path, sessions, snapshot_len)?;

    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Encounter");

    let last_index = sessions.len() - 1;
    let mut written = Vec::with_capacity(sessions.len());
    for (i, session) in sessions.iter().enumerate() {
        let end = clamped_session_end(session, i == last_index, snapshot_len);
        if end <= session.start_offset {
            continue; // session lies entirely past the snapshot (shouldn't happen)
        }
        let dst = out.join(session_file_name(stem, session));
        copy_range(src, &dst, session.start_offset, end)?;
        written.push(dst.to_string_lossy().into_owned());
    }
    Ok(written)
}

/// Resolve the trustworthy session list (re-scanning if the stale preflight
/// offsets can't be trusted) plus the length snapshot the copies are clamped to.
/// Shared by [`split_by_session`] and [`split_selected`] so both apply the exact
/// same correctness gate.
fn resolve_sessions(
    src: &Path,
    source_path: &str,
    sessions: Option<Vec<LogSession>>,
    snapshot_len: u64,
) -> Result<(Vec<LogSession>, u64), String> {
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
    Ok((sessions, snapshot_len))
}

/// The byte offset a session's copy ends at, clamped to the snapshot. Completed
/// sessions end at a real BEGIN_LOG/END_LOG boundary that never moves, so their
/// stale `end_offset` is correct. The FINAL (possibly still-open) session is
/// extended to the current EOF so fights ESO appended since preflight are
/// included, then clamped so no copy reads past the snapshot.
fn clamped_session_end(session: &LogSession, is_last: bool, snapshot_len: u64) -> u64 {
    let raw = if is_last {
        session.end_offset.max(snapshot_len)
    } else {
        session.end_offset
    };
    raw.min(snapshot_len)
}

/// Split only the sessions the user selected (in the split workbench), naming
/// each from the user's sanitized custom name where given. Unlike
/// [`split_by_session`], a session not present in `selections` is skipped — so a
/// user can drop empty/unwanted sessions and keep just the ones worth uploading.
///
/// A custom name that sanitizes to empty (or collides with another written file)
/// falls back to the stable auto name, so the result is always valid and
/// collision-free. Returns the written paths in selection order.
pub fn split_selected(
    source_path: &str,
    out_dir: &str,
    sessions: Option<Vec<LogSession>>,
    selections: Vec<SplitSelection>,
) -> Result<Vec<String>, String> {
    let src = Path::new(source_path);
    if !src.is_file() {
        return Err(format!("Source log not found: {source_path}"));
    }
    if selections.is_empty() {
        return Err("No sessions were selected to split.".into());
    }
    // Bound the work: a real log has at most a few dozen sessions. A list far
    // larger than that is a bug or abuse — refuse rather than copy unbounded
    // multi-GB ranges into app data.
    const MAX_SELECTIONS: usize = 256;
    if selections.len() > MAX_SELECTIONS {
        return Err("Too many sessions selected.".into());
    }
    // De-duplicate by session index (first occurrence wins) so a repeated index
    // can't write the same multi-GB range twice.
    let selections: Vec<SplitSelection> = {
        let mut seen = std::collections::HashSet::new();
        selections
            .into_iter()
            .filter(|s| seen.insert(s.index))
            .collect()
    };
    let out = PathBuf::from(out_dir);
    std::fs::create_dir_all(&out).map_err(|e| format!("Create output dir: {e}"))?;

    let snapshot_len = std::fs::metadata(src)
        .map_err(|e| format!("Failed to stat source: {e}"))?
        .len();
    let (sessions, snapshot_len) = resolve_sessions(src, source_path, sessions, snapshot_len)?;
    let last_index = sessions.len() - 1;

    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Encounter");

    // Track used file names so two custom names (or a custom name colliding with
    // an auto name) never overwrite each other; a clash gets a `-2`, `-3`, … suffix.
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut written = Vec::with_capacity(selections.len());

    for sel in &selections {
        // Find the session this selection refers to. Prefer the absolute offset
        // when supplied; a latest-session-only preflight has local indices, while
        // a fallback full rescan has file-global indices.
        let found = sessions.iter().enumerate().find(|(_, s)| {
            sel.start_offset
                .map(|offset| s.start_offset == offset)
                .unwrap_or(s.index == sel.index)
        });
        let Some((pos, session)) = found else {
            continue;
        };
        // If the caller pinned the session's identity, verify it still matches the
        // resolved session. A mismatch means the log was truncated/rotated between
        // preflight and split and indices shifted — fail loudly so we never write a
        // different session under the user's chosen name.
        if let Some(expected) = sel.start_time_ms {
            if session.start_time_ms != expected {
                return Err(
                    "The log changed since it was scanned. Re-select it and try the split again."
                        .into(),
                );
            }
        }
        let end = clamped_session_end(session, pos == last_index, snapshot_len);
        if end <= session.start_offset {
            continue;
        }

        // Resolve the file name: sanitized custom name if usable, else the auto
        // name; then de-duplicate against names already written this run.
        let base = sel
            .name
            .as_deref()
            .and_then(sanitize_split_stem)
            .map(|s| format!("{s}.log"))
            .unwrap_or_else(|| session_file_name(stem, session));
        let name = unique_name(&mut used, base);

        let dst = out.join(&name);
        copy_range(src, &dst, session.start_offset, end)?;
        written.push(dst.to_string_lossy().into_owned());
    }

    if written.is_empty() {
        return Err("None of the selected sessions could be written.".into());
    }
    Ok(written)
}

/// Reserve a unique file name, appending `-2`, `-3`, … before the extension on
/// collision so two splits never clobber one another. The reservation key is
/// LOWERCASED: on Windows the output directory is case-insensitive, so `Raid.log`
/// and `raid.log` resolve to the same path and the second copy would overwrite
/// the first. We compare case-insensitively while keeping the user's casing in
/// the actual file name.
fn unique_name(used: &mut std::collections::HashSet<String>, candidate: String) -> String {
    if used.insert(candidate.to_lowercase()) {
        return candidate;
    }
    let (stem, ext) = match candidate.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (candidate.clone(), String::new()),
    };
    for n in 2..1000 {
        let next = format!("{stem}-{n}{ext}");
        if used.insert(next.to_lowercase()) {
            return next;
        }
    }
    // Pathological fallback (1000 collisions): use the original, accepting overwrite.
    candidate
}

/// Copy a set of byte `segments` (each `[start, end)`) from `src` into a single
/// new file at `dst`, concatenated in order. Used to assemble a single-fight log
/// from the session preamble plus one fight's combat block, skipping the byte
/// ranges of the other fights. Streams in fixed buffers so memory stays flat, and —
/// like [`copy_range`] — fails loudly (removing the partial output) if the source
/// shrank under a segment, so a truncated log never yields a silently-corrupt file.
fn copy_ranges(src: &Path, dst: &Path, segments: &[(u64, u64)]) -> Result<(), String> {
    let mut reader = BufReader::new(File::open(src).map_err(|e| format!("Open source: {e}"))?);
    let mut writer = BufWriter::new(File::create(dst).map_err(|e| format!("Create output: {e}"))?);
    let mut buf = vec![0u8; COPY_BUF];
    for &(start, end) in segments {
        if end <= start {
            continue;
        }
        reader
            .seek(SeekFrom::Start(start))
            .map_err(|e| format!("Seek: {e}"))?;
        let mut remaining = end - start;
        while remaining > 0 {
            let want = remaining.min(COPY_BUF as u64) as usize;
            let n = reader
                .read(&mut buf[..want])
                .map_err(|e| format!("Read: {e}"))?;
            if n == 0 {
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
    }
    writer.flush().map_err(|e| format!("Flush: {e}"))?;
    Ok(())
}

/// A user's choice for one FIGHT in the per-fight split workbench: which fight
/// (`index`, matching [`FightSummary::index`]) and an optional custom name. Only
/// fights present in the selection are written — one self-contained `.log` each.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FightSelection {
    /// The [`FightSummary::index`] this selection refers to.
    pub index: usize,
    /// A user-supplied name (sanitized before use); falls back to the auto name.
    pub name: Option<String>,
    /// Absolute byte offset of the fight's `BEGIN_COMBAT` line. When present, this
    /// is preferred over `index` so latest-session-only preflights can survive a
    /// fallback full-file rescan where global fight indices differ.
    pub start_offset: Option<u64>,
    /// The fight's `start_ms` at selection time. When present, it is verified
    /// against the resolved fight so a rescan (the log changed between preflight
    /// and split) that shifted indices is caught instead of mislabeling/mis-slicing.
    pub start_ms: Option<u64>,
}

/// A stable, filesystem-safe fallback name for a single fight's split file.
/// Anchored on the file-global fight index + the fight's relative start time so
/// names are unique and sortable even when a fight has no boss/zone name.
fn fight_file_name(stem: &str, fight: &FightSummary) -> String {
    format!("{stem}-fight{:02}-{}.log", fight.index + 1, fight.start_ms)
}

/// Resolve a trustworthy `(sessions, fights, snapshot_len)` triple for a per-fight
/// split: trust the caller's preflight lists only when the file still matches them
/// (the SAME trust gate as [`resolve_sessions`]); otherwise re-scan for a
/// consistent pair. Returning BOTH from one scan keeps fight indices aligned with
/// the sessions they fall in. A pinned per-fight `start_ms` (verified by the caller)
/// then catches the case where a re-scan shifted indices.
fn resolve_scan(
    src: &Path,
    source_path: &str,
    sessions: Option<Vec<LogSession>>,
    fights: Option<Vec<FightSummary>>,
    snapshot_len: u64,
) -> Result<(Vec<LogSession>, Vec<FightSummary>, u64), String> {
    let trust = match (&sessions, &fights) {
        (Some(s), Some(_)) if !s.is_empty() => {
            let max_end = s.iter().map(|x| x.end_offset).max().unwrap_or(0);
            snapshot_len >= max_end
                && offsets_still_valid(src, &s[0])
                && !appended_range_has_new_session(src, max_end, snapshot_len)
        }
        _ => false,
    };
    if trust {
        // Both are `Some` and trusted (the match guard above proved it).
        Ok((sessions.unwrap(), fights.unwrap(), snapshot_len))
    } else {
        let scan = scanner::scan_file(source_path)?;
        if scan.sessions.is_empty() {
            return Err("No logging sessions found in this file.".into());
        }
        Ok((scan.sessions, scan.fights, snapshot_len))
    }
}

/// Split selected FIGHTS out of `source_path` into `out_dir`, writing ONE
/// self-contained `.log` per selected fight.
///
/// Each output file is a valid single-fight session log: the enclosing session's
/// preamble (its `BEGIN_LOG` header plus every line up to the fight that is NOT
/// inside an earlier fight's `BEGIN_COMBAT`…`END_COMBAT` block) followed by the
/// selected fight's own combat block. Earlier fights in the same session are
/// dropped, so the report isolates exactly one fight while keeping the session
/// header and the zone/unit/ability definitions that precede it (so ESO Logs can
/// parse it). Definitions emitted *inside* an earlier fight (lazily, on first use)
/// are not carried over — a deliberate tradeoff: at worst an ability reused from an
/// earlier fight shows as "Unknown" in the report; damage/healing numbers, which
/// come from the combat events themselves, stay correct.
///
/// Like [`split_selected`], a custom name that sanitizes to empty or collides falls
/// back to a stable auto name, and the written paths are returned in selection order.
pub fn split_selected_fights(
    source_path: &str,
    out_dir: &str,
    sessions: Option<Vec<LogSession>>,
    fights: Option<Vec<FightSummary>>,
    selections: Vec<FightSelection>,
) -> Result<Vec<String>, String> {
    let src = Path::new(source_path);
    if !src.is_file() {
        return Err(format!("Source log not found: {source_path}"));
    }
    if selections.is_empty() {
        return Err("No fights were selected to split.".into());
    }
    // A real night has at most a few hundred fights; refuse a list far larger than
    // that (a bug or abuse) rather than write unbounded copies into app data.
    const MAX_SELECTIONS: usize = 1024;
    if selections.len() > MAX_SELECTIONS {
        return Err("Too many fights selected.".into());
    }
    // De-duplicate by fight index (first occurrence wins) so a repeated index can't
    // write the same fight twice.
    let selections: Vec<FightSelection> = {
        let mut seen = std::collections::HashSet::new();
        selections
            .into_iter()
            .filter(|s| seen.insert(s.index))
            .collect()
    };

    let out = PathBuf::from(out_dir);
    std::fs::create_dir_all(&out).map_err(|e| format!("Create output dir: {e}"))?;

    let snapshot_len = std::fs::metadata(src)
        .map_err(|e| format!("Failed to stat source: {e}"))?
        .len();
    let (sessions, all_fights, snapshot_len) =
        resolve_scan(src, source_path, sessions, fights, snapshot_len)?;
    if all_fights.is_empty() {
        return Err("No fights were found in this file.".into());
    }
    let last_session_index = sessions.len() - 1;

    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Encounter");

    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut written = Vec::with_capacity(selections.len());

    for sel in &selections {
        // Resolve the target fight. Prefer the absolute offset when supplied; a
        // latest-session-only preflight has local indices, while a fallback full
        // rescan has file-global indices.
        let found = all_fights.iter().find(|f| {
            sel.start_offset
                .map(|offset| f.start_offset == offset)
                .unwrap_or(f.index == sel.index)
        });
        let Some(fight) = found else {
            continue;
        };
        // If the caller pinned the fight identity, verify it still matches the
        // resolved fight; a mismatch means the log changed and indices shifted.
        if let Some(expected) = sel.start_ms {
            if fight.start_ms != expected {
                return Err(
                    "The log changed since it was scanned. Re-select it and try the split again."
                        .into(),
                );
            }
        }

        // Find the session containing this fight (its byte range encloses the
        // fight's start). The preamble we keep is anchored at this session's start.
        let Some((pos, session)) = sessions.iter().enumerate().find(|(_, s)| {
            fight.start_offset >= s.start_offset && fight.start_offset < s.end_offset
        }) else {
            continue; // orphan fight (shouldn't happen) — skip rather than mislabel
        };

        // The session's usable end, clamped to the snapshot (the final session may
        // still be open/growing — extend it, then clamp like the per-session path).
        let session_end = clamped_session_end(session, pos == last_session_index, snapshot_len);
        // Cap the target fight's end to what definitely exists on disk.
        let fight_end = fight.end_offset.min(session_end).min(snapshot_len);
        if fight_end <= fight.start_offset {
            continue; // the fight lies past the snapshot — nothing safe to copy
        }

        // Earlier fights in the SAME session become "holes": [session.start, fight_end)
        // MINUS each earlier fight's combat block, so only this fight's combat
        // survives while every preceding definition/zone line is kept.
        let mut earlier: Vec<(u64, u64)> = all_fights
            .iter()
            .filter(|g| {
                g.index != fight.index
                    && g.start_offset >= session.start_offset
                    && g.start_offset < fight.start_offset
            })
            .map(|g| (g.start_offset, g.end_offset.min(fight.start_offset)))
            .collect();
        earlier.sort_by_key(|&(s, _)| s);

        let mut segments: Vec<(u64, u64)> = Vec::with_capacity(earlier.len() + 1);
        let mut cursor = session.start_offset;
        for (gs, ge) in earlier {
            if gs > cursor {
                segments.push((cursor, gs));
            }
            cursor = cursor.max(ge);
        }
        if fight_end > cursor {
            segments.push((cursor, fight_end));
        }
        if segments.is_empty() {
            continue;
        }

        // Resolve the destination name: sanitized custom name if usable, else the
        // stable auto name; de-duplicate against names already written this run.
        let base = sel
            .name
            .as_deref()
            .and_then(sanitize_split_stem)
            .map(|s| format!("{s}.log"))
            .unwrap_or_else(|| fight_file_name(stem, fight));
        let name = unique_name(&mut used, base);

        let dst = out.join(&name);
        copy_ranges(src, &dst, &segments)?;
        written.push(dst.to_string_lossy().into_owned());
    }

    if written.is_empty() {
        return Err("None of the selected fights could be written.".into());
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

    #[test]
    fn split_latest_session_writes_only_newest_session() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Encounter.log");
        let out = tmp.path().join("out");

        let first = b"0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n\
                      10,BEGIN_COMBAT\n11,COMBAT_EVENT,OLD\n20,END_COMBAT\n30,END_LOG\n";
        let latest = b"0,BEGIN_LOG,2000,15,\"NA\",\"en\",\"10.0\"\n\
                       10,BEGIN_COMBAT\n11,COMBAT_EVENT,LATEST\n20,END_COMBAT\n";
        let mut full = first.to_vec();
        full.extend_from_slice(latest);
        write(&log, &full);

        let written = split_latest_session(log.to_str().unwrap(), out.to_str().unwrap()).unwrap();

        assert_eq!(written.len(), 1);
        assert!(
            written[0].ends_with("Encounter-latest-session-2000.log"),
            "got {}",
            written[0]
        );
        let bytes = std::fs::read(&written[0]).unwrap();
        assert_eq!(bytes, latest);
    }

    #[test]
    fn split_latest_session_errors_without_begin_log() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Interface.log");
        let out = tmp.path().join("out");
        write(&log, b"plain ui log line\nanother line\n");

        let err = split_latest_session(log.to_str().unwrap(), out.to_str().unwrap())
            .expect_err("a log without BEGIN_LOG must fail");

        assert!(
            err.contains("No logging sessions"),
            "unexpected error: {err}"
        );
    }

    // ── split_selected_fights (per-fight extraction) ─────────────────────────

    /// A three-fight session with a pre-combat definition block and an inter-fight
    /// definition line, so we can prove the per-fight extract keeps the session
    /// header + definitions but only the SELECTED fight's combat.
    fn three_fight_session() -> Vec<u8> {
        // Markers: AAA/BBB/CCC identify each fight's events; "Some Ability" is a
        // definition emitted BETWEEN fight 0 and fight 1 (must survive a fight-1
        // extract); "Sunspire"/"BossName" are the session preamble.
        b"0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n\
5,ZONE_CHANGED,1,\"Sunspire\",VETERAN\n\
6,UNIT_ADDED,1,MONSTER,T,1,1,1,1,1,1,\"BossName\",\"\"\n\
10,BEGIN_COMBAT\n\
11,COMBAT_EVENT,AAA\n\
20,END_COMBAT\n\
21,ABILITY_INFO,12345,\"Some Ability\"\n\
30,BEGIN_COMBAT\n\
31,COMBAT_EVENT,BBB\n\
40,END_COMBAT\n\
50,BEGIN_COMBAT\n\
51,COMBAT_EVENT,CCC\n\
60,END_COMBAT\n\
70,END_LOG\n"
            .to_vec()
    }

    // Extracting a MIDDLE fight (index 1) must yield: the session header + the
    // pre-combat preamble + the inter-fight definition, plus ONLY fight 1's combat —
    // never fight 0's or fight 2's events.
    #[test]
    fn split_fights_extracts_only_the_selected_fight_with_preamble() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Encounter.log");
        let out = tmp.path().join("out");
        let full = three_fight_session();
        write(&log, &full);
        let scan = scanner::scan_file(log.to_str().unwrap()).unwrap();
        assert_eq!(scan.fights.len(), 3, "fixture has three fights");

        let written = split_selected_fights(
            log.to_str().unwrap(),
            out.to_str().unwrap(),
            Some(scan.sessions.clone()),
            Some(scan.fights.clone()),
            vec![FightSelection {
                index: 1,
                name: Some("kynes-prog".into()),
                start_offset: None,
                start_ms: Some(scan.fights[1].start_ms),
            }],
        )
        .unwrap();
        assert_eq!(written.len(), 1);
        assert!(written[0].ends_with("kynes-prog.log"), "got {}", written[0]);

        let bytes = std::fs::read(&written[0]).unwrap();
        let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        // Session header + preamble + inter-fight definition are kept.
        assert!(has(b"BEGIN_LOG"), "session header must be kept");
        assert!(has(b"Sunspire"), "zone preamble must be kept");
        assert!(has(b"BossName"), "unit preamble must be kept");
        assert!(has(b"Some Ability"), "inter-fight definition must be kept");
        // Only the selected fight's combat survives.
        assert!(has(b"BBB"), "the selected fight's events must be present");
        assert!(!has(b"AAA"), "an earlier fight's events must be dropped");
        assert!(!has(b"CCC"), "a later fight's events must be dropped");
    }

    // Extracting the FIRST fight (index 0) keeps the header/preamble and only that
    // fight; nothing from fights 1/2 leaks in.
    #[test]
    fn split_fights_first_fight_keeps_only_preamble_and_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Encounter.log");
        let out = tmp.path().join("out");
        write(&log, &three_fight_session());
        let scan = scanner::scan_file(log.to_str().unwrap()).unwrap();

        let written = split_selected_fights(
            log.to_str().unwrap(),
            out.to_str().unwrap(),
            Some(scan.sessions.clone()),
            Some(scan.fights.clone()),
            vec![FightSelection {
                index: 0,
                name: None, // exercise the auto fallback name
                start_offset: None,
                start_ms: None,
            }],
        )
        .unwrap();
        let bytes = std::fs::read(&written[0]).unwrap();
        let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        assert!(has(b"BEGIN_LOG") && has(b"Sunspire") && has(b"AAA"));
        assert!(!has(b"BBB") && !has(b"CCC"));
        // Auto name carries the file stem + fight number + relative start ms.
        assert!(
            written[0].ends_with("-fight01-10.log"),
            "got {}",
            written[0]
        );
    }

    // Selecting several fights writes one file each, in selection order, with
    // colliding custom names de-duplicated.
    #[test]
    fn split_fights_writes_one_file_per_fight_and_dedupes_names() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Encounter.log");
        let out = tmp.path().join("out");
        write(&log, &three_fight_session());
        let scan = scanner::scan_file(log.to_str().unwrap()).unwrap();

        let written = split_selected_fights(
            log.to_str().unwrap(),
            out.to_str().unwrap(),
            Some(scan.sessions.clone()),
            Some(scan.fights.clone()),
            vec![
                FightSelection {
                    index: 0,
                    name: Some("pull".into()),
                    start_offset: None,
                    start_ms: None,
                },
                FightSelection {
                    index: 2,
                    name: Some("pull".into()),
                    start_offset: None,
                    start_ms: None,
                },
            ],
        )
        .unwrap();
        assert_eq!(written.len(), 2);
        assert!(written[0].ends_with("pull.log"));
        assert!(written[1].ends_with("pull-2.log"), "got {}", written[1]);
        // The second file is fight 2 (CCC), not fight 0.
        let second = std::fs::read(&written[1]).unwrap();
        assert!(second.windows(3).any(|w| w == b"CCC"));
        assert!(!second.windows(3).any(|w| w == b"AAA"));
    }

    // A pinned start_ms that no longer matches the resolved fight (the log changed
    // since preflight) must fail loudly rather than extract the wrong fight.
    #[test]
    fn split_fights_rejects_stale_fingerprint() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Encounter.log");
        let out = tmp.path().join("out");
        write(&log, &three_fight_session());
        let scan = scanner::scan_file(log.to_str().unwrap()).unwrap();
        let res = split_selected_fights(
            log.to_str().unwrap(),
            out.to_str().unwrap(),
            Some(scan.sessions.clone()),
            Some(scan.fights.clone()),
            vec![FightSelection {
                index: 0,
                name: None,
                start_offset: None,
                start_ms: Some(999_999), // does not match fight 0's real start_ms
            }],
        );
        assert!(res.is_err(), "a mismatched fight fingerprint must fail");
    }

    #[test]
    fn split_fights_resolves_latest_session_local_index_by_offset_after_rescan() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Encounter.log");
        let out = tmp.path().join("out");

        let old = b"0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n\
                    10,BEGIN_COMBAT\n11,COMBAT_EVENT,OLD\n20,END_COMBAT\n";
        let latest = b"0,BEGIN_LOG,2000,15,\"NA\",\"en\",\"10.0\"\n\
                       10,BEGIN_COMBAT\n11,COMBAT_EVENT,LATEST\n20,END_COMBAT\n";
        let mut full = old.to_vec();
        full.extend_from_slice(latest);
        write(&log, &full);

        // Latest-session preflight sees one local fight at index 0, but its byte
        // offsets remain absolute in the original file.
        let (latest_scan, _) = scanner::scan_latest_session(log.to_str().unwrap()).unwrap();
        assert_eq!(latest_scan.fights[0].index, 0);

        // ESO appends a new session after that preflight, forcing the split path
        // to distrust the bounded scan and fall back to a full-file scan.
        full.extend_from_slice(
            b"0,BEGIN_LOG,3000,15,\"NA\",\"en\",\"10.0\"\n\
              10,BEGIN_COMBAT\n11,COMBAT_EVENT,NEW\n20,END_COMBAT\n",
        );
        write(&log, &full);

        let target = &latest_scan.fights[0];
        let written = split_selected_fights(
            log.to_str().unwrap(),
            out.to_str().unwrap(),
            Some(latest_scan.sessions.clone()),
            Some(latest_scan.fights.clone()),
            vec![FightSelection {
                index: target.index,
                name: Some("latest".into()),
                start_offset: Some(target.start_offset),
                start_ms: Some(target.start_ms),
            }],
        )
        .unwrap();

        let bytes = std::fs::read(&written[0]).unwrap();
        assert!(bytes.windows(6).any(|w| w == b"LATEST"));
        assert!(!bytes.windows(3).any(|w| w == b"OLD"));
        assert!(!bytes.windows(3).any(|w| w == b"NEW"));
    }

    // A fight in the SECOND session must be extracted with the SECOND session's
    // header/preamble — never the first session's.
    #[test]
    fn split_fights_uses_the_enclosing_session_preamble() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Encounter.log");
        let out = tmp.path().join("out");
        // Two sessions; session 2 has a distinct zone + a fight (DDD).
        let full = b"0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n\
5,ZONE_CHANGED,1,\"Sunspire\",VETERAN\n\
10,BEGIN_COMBAT\n11,COMBAT_EVENT,AAA\n20,END_COMBAT\n\
0,BEGIN_LOG,2000,15,\"NA\",\"en\",\"10.0\"\n\
5,ZONE_CHANGED,2,\"Cloudrest\",VETERAN\n\
10,BEGIN_COMBAT\n11,COMBAT_EVENT,DDD\n20,END_COMBAT\n"
            .to_vec();
        write(&log, &full);
        let scan = scanner::scan_file(log.to_str().unwrap()).unwrap();
        assert_eq!(scan.fights.len(), 2);
        // The second fight (index 1) lives in session 2.
        let written = split_selected_fights(
            log.to_str().unwrap(),
            out.to_str().unwrap(),
            Some(scan.sessions.clone()),
            Some(scan.fights.clone()),
            vec![FightSelection {
                index: 1,
                name: None,
                start_offset: None,
                start_ms: None,
            }],
        )
        .unwrap();
        let bytes = std::fs::read(&written[0]).unwrap();
        let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        assert!(
            has(b"Cloudrest") && has(b"DDD"),
            "session-2 preamble + fight"
        );
        assert!(
            !has(b"Sunspire") && !has(b"AAA"),
            "session-1 content must not leak"
        );
        // The header carried is session 2's (start time 2000).
        assert!(has(b"2000") && !has(b"1000"));
    }

    // The split-name sanitizer must never let a crafted name escape the output
    // directory or produce an invalid file name. Path separators, traversal, and
    // exotic characters are stripped; a name that reduces to nothing falls back.
    #[test]
    fn sanitize_split_stem_blocks_traversal_and_separators() {
        // Traversal / separators collapse to a single inner '-' and trim to a
        // safe stem (never "..", a slash, or a leading dot).
        assert_eq!(
            sanitize_split_stem("../../etc/passwd").as_deref(),
            Some("etc-passwd")
        );
        assert_eq!(sanitize_split_stem("a/b\\c").as_deref(), Some("a-b-c"));
        assert_eq!(
            sanitize_split_stem("  core prog  ").as_deref(),
            Some("core-prog")
        );
        assert_eq!(
            sanitize_split_stem("lucent--hm__farm").as_deref(),
            Some("lucent-hm__farm")
        );
        // Reduces to empty → None (caller uses the stable auto name).
        assert_eq!(sanitize_split_stem("../"), None);
        assert_eq!(sanitize_split_stem("..."), None);
        assert_eq!(sanitize_split_stem("   "), None);
        assert_eq!(sanitize_split_stem(""), None);
        // No leading dot (hidden file) or trailing dot (invalid on Windows).
        assert_eq!(sanitize_split_stem(".hidden").as_deref(), Some("hidden"));
        assert_eq!(sanitize_split_stem("name.").as_deref(), Some("name"));
        // Length is capped.
        assert!(sanitize_split_stem(&"x".repeat(500)).unwrap().len() <= 80);
    }

    // split_selected writes only the chosen sessions, names them from the
    // sanitized custom name, and de-duplicates colliding names.
    #[test]
    fn split_selected_writes_only_chosen_sessions_with_custom_names() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Encounter.log");
        let out = tmp.path().join("out");

        // Two sessions.
        let a = b"0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n";
        let b = b"0,BEGIN_LOG,2000,15,\"NA\",\"en\",\"10.0\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n";
        let mut full = a.to_vec();
        full.extend_from_slice(b);
        write(&log, &full);
        let a_end = a.len() as u64;

        let sessions = vec![
            LogSession {
                index: 0,
                start_offset: 0,
                end_offset: a_end,
                start_time_ms: 1000,
                log_version: "15".into(),
                realm: Some("NA".into()),
                fight_count: 1,
                size_bytes: a_end,
            },
            LogSession {
                index: 1,
                start_offset: a_end,
                end_offset: full.len() as u64,
                start_time_ms: 2000,
                log_version: "15".into(),
                realm: Some("NA".into()),
                fight_count: 1,
                size_bytes: b.len() as u64,
            },
        ];

        // Select only session 1 (index 1), with a custom name.
        let written = split_selected(
            log.to_str().unwrap(),
            out.to_str().unwrap(),
            Some(sessions),
            vec![SplitSelection {
                index: 1,
                name: Some("core prog/hm".into()),
                start_offset: None,
                start_time_ms: None,
            }],
        )
        .unwrap();

        assert_eq!(written.len(), 1, "only the selected session is written");
        // Custom name was sanitized (slash → '-') and used.
        assert!(
            written[0].ends_with("core-prog-hm.log"),
            "got {}",
            written[0]
        );
        // The written file is session B's bytes (starts with its BEGIN_LOG ts).
        let bytes = std::fs::read(&written[0]).unwrap();
        assert!(bytes.windows(4).any(|w| w == b"2000"));
        assert!(!bytes.windows(4).any(|w| w == b"1000"));
    }

    // Two selections with the same custom name must not clobber each other.
    #[test]
    fn split_selected_dedupes_colliding_names() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Encounter.log");
        let out = tmp.path().join("out");
        let a = b"0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n";
        let b = b"0,BEGIN_LOG,2000,15,\"NA\",\"en\",\"10.0\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n";
        let mut full = a.to_vec();
        full.extend_from_slice(b);
        write(&log, &full);
        let a_end = a.len() as u64;
        let sessions = vec![
            LogSession {
                index: 0,
                start_offset: 0,
                end_offset: a_end,
                start_time_ms: 1000,
                log_version: "15".into(),
                realm: Some("NA".into()),
                fight_count: 1,
                size_bytes: a_end,
            },
            LogSession {
                index: 1,
                start_offset: a_end,
                end_offset: full.len() as u64,
                start_time_ms: 2000,
                log_version: "15".into(),
                realm: Some("NA".into()),
                fight_count: 1,
                size_bytes: b.len() as u64,
            },
        ];
        let written = split_selected(
            log.to_str().unwrap(),
            out.to_str().unwrap(),
            Some(sessions),
            vec![
                SplitSelection {
                    index: 0,
                    name: Some("raid".into()),
                    start_offset: None,
                    start_time_ms: None,
                },
                SplitSelection {
                    index: 1,
                    name: Some("raid".into()),
                    start_offset: None,
                    start_time_ms: None,
                },
            ],
        )
        .unwrap();
        assert_eq!(written.len(), 2);
        assert!(written[0].ends_with("raid.log"));
        assert!(written[1].ends_with("raid-2.log"), "got {}", written[1]);
    }

    // A pinned start_time_ms that doesn't match the resolved session (the log
    // changed/rescanned since preflight, shifting indices) must fail loudly rather
    // than write a different session under the user's chosen name.
    #[test]
    fn split_selected_rejects_stale_session_fingerprint() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Encounter.log");
        let out = tmp.path().join("out");
        let a = b"0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n";
        write(&log, a);
        let sessions = vec![LogSession {
            index: 0,
            start_offset: 0,
            end_offset: a.len() as u64,
            start_time_ms: 1000,
            log_version: "15".into(),
            realm: Some("NA".into()),
            fight_count: 1,
            size_bytes: a.len() as u64,
        }];
        // Selection pins start_time_ms=9999, but the resolved session is 1000.
        let res = split_selected(
            log.to_str().unwrap(),
            out.to_str().unwrap(),
            Some(sessions),
            vec![SplitSelection {
                index: 0,
                name: Some("raid".into()),
                start_offset: None,
                start_time_ms: Some(9999),
            }],
        );
        assert!(res.is_err(), "a mismatched session fingerprint must fail");
    }

    #[test]
    fn split_selected_resolves_latest_session_local_index_by_offset_after_rescan() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("Encounter.log");
        let out = tmp.path().join("out");

        let old = b"0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n";
        let latest =
            b"0,BEGIN_LOG,2000,15,\"NA\",\"en\",\"10.0\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n";
        let mut full = old.to_vec();
        full.extend_from_slice(latest);
        write(&log, &full);

        let (latest_scan, _) = scanner::scan_latest_session(log.to_str().unwrap()).unwrap();
        assert_eq!(latest_scan.sessions[0].index, 0);

        full.extend_from_slice(
            b"0,BEGIN_LOG,3000,15,\"NA\",\"en\",\"10.0\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n",
        );
        write(&log, &full);

        let target = &latest_scan.sessions[0];
        let written = split_selected(
            log.to_str().unwrap(),
            out.to_str().unwrap(),
            Some(latest_scan.sessions.clone()),
            vec![SplitSelection {
                index: target.index,
                name: Some("latest-session".into()),
                start_offset: Some(target.start_offset),
                start_time_ms: Some(target.start_time_ms),
            }],
        )
        .unwrap();

        let bytes = std::fs::read(&written[0]).unwrap();
        assert!(bytes.windows(4).any(|w| w == b"2000"));
        assert!(!bytes.windows(4).any(|w| w == b"1000"));
        assert!(!bytes.windows(4).any(|w| w == b"3000"));
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
