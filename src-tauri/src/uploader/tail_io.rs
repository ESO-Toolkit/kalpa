//! Shared byte-offset tail primitives for the uploader's live paths.
//!
//! Both the fight-detection watcher ([`super::watcher`]) and the native
//! live-streaming driver ([`super::native::live`]) tail a growing
//! `Encounter.log` by byte offset (the game only appends), reading just the
//! newly-written range each pass so a multi-hour raid never re-reads the whole
//! file. The read primitive and the loop tuning constants live here so there is
//! exactly ONE definition of the read bound, the poll cadence, and the
//! failure-streak teardown threshold — the two tail loops cannot drift apart on
//! these semantics.

use std::path::Path;
use std::time::Duration;

/// 64 MiB cap on a single incremental read, bounding memory per pass.
pub const MAX_READ: u64 = 64 * 1024 * 1024;

/// How much of a log's TAIL the native-live readiness probe reads to judge the
/// current session state (256 KiB is many thousands of lines — far more than enough
/// to find the latest `BEGIN_LOG`/`END_COMBAT` boundary).
pub const TAIL_PEEK: u64 = 256 * 1024;

/// Poll fallback cadence — short enough to feel live, light on IO.
pub const POLL_INTERVAL: Duration = Duration::from_millis(400);

/// Give up after this many consecutive stat/read failures (~the active log was
/// deleted/renamed/replaced-by-a-dir, or a permanent AV/CFA/share lock). At a
/// ~400ms cadence this is ~12s of unbroken failure before we tear the session
/// down rather than spinning forever while the UI shows a stuck "LIVE".
pub const MAX_CONSECUTIVE_FAILURES: u32 = 30;

/// Read `[start, end)` from a file into the caller-owned `buf`, bounded by
/// [`MAX_READ`], and return the number of bytes read. `buf` is resized to
/// exactly that length, so the valid bytes are `&buf[..len]`.
///
/// The caller hands in a buffer that lives for the whole tail loop so this
/// never allocates per pass — it grows to the session's high-water mark once
/// and is reused. Offsets are still tracked from true byte lengths (the raw
/// bytes), never from a lossily-decoded string.
pub fn read_range(path: &Path, start: u64, end: u64, buf: &mut Vec<u8>) -> Result<usize, String> {
    use std::io::{Read, Seek, SeekFrom};
    if end <= start {
        buf.clear();
        return Ok(0);
    }
    let len = end - start;
    if len > MAX_READ {
        return Err(format!("incremental read too large: {len} bytes"));
    }
    let len = len as usize;
    let mut f = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    f.seek(SeekFrom::Start(start))
        .map_err(|e| format!("seek: {e}"))?;
    // `resize` reuses existing capacity; it only zero-fills bytes beyond the
    // current high-water mark, and those are immediately overwritten by the
    // read below.
    buf.resize(len, 0);
    f.read_exact(&mut buf[..len])
        .map_err(|e| format!("read: {e}"))?;
    Ok(len)
}

/// The offset just AFTER the last newline at or before `eof` — i.e. the start of the
/// line that straddles `eof` (or `eof` itself when the byte before `eof` is a newline,
/// meaning `eof` is already a clean line boundary).
///
/// Used to start a live tail on a guaranteed line boundary so a non-atomic append in
/// progress at attach time can't split a line across the warm-up/tail seam (the warm-up
/// replays complete lines up to this boundary; the tail then reads the straddling line
/// whole from its true start). Returns `None` only in the degenerate case where no
/// newline exists within the search window (a single line longer than the window).
pub fn last_line_boundary(path: &Path, eof: u64) -> Option<u64> {
    if eof == 0 {
        return Some(0);
    }
    // A well-formed ESO line is far below `MAX_PARTIAL` (1 MiB); searching the last 1 MiB
    // is bounded and always captures the newline preceding the straddling tail.
    let window = eof.min(1024 * 1024);
    let start = eof - window;
    let mut buf = Vec::new();
    let n = read_range(path, start, eof, &mut buf).ok()?;
    buf[..n]
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|idx| start + idx as u64 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn read_range_reads_only_the_requested_window() {
        let dir = std::env::temp_dir();
        let path = dir.join("kalpa-tail-io-read-range-test.log");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"0123456789").unwrap();
        }
        let mut buf = Vec::new();
        let n = read_range(&path, 2, 6, &mut buf).unwrap();
        assert_eq!(n, 4);
        assert_eq!(&buf[..n], b"2345");
        // An empty window clears the buffer and reads nothing.
        let n0 = read_range(&path, 5, 5, &mut buf).unwrap();
        assert_eq!(n0, 0);
        assert!(buf.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_range_rejects_an_oversized_window() {
        let dir = std::env::temp_dir();
        let path = dir.join("kalpa-tail-io-oversized-test.log");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"x").unwrap();
        }
        // A window larger than MAX_READ is refused before any allocation/read.
        let err = read_range(&path, 0, MAX_READ + 1, &mut Vec::new()).unwrap_err();
        assert!(err.contains("too large"), "{err}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn last_line_boundary_finds_the_line_safe_start() {
        let dir = std::env::temp_dir();
        let path = dir.join("kalpa-tail-io-boundary-test.log");
        // "aaa\nbbb\nccc" — newlines at offsets 3 and 7; "ccc" (8..11) is the straddling tail.
        std::fs::write(&path, b"aaa\nbbb\nccc").unwrap();
        let eof = std::fs::metadata(&path).unwrap().len();
        // EOF mid-line ("ccc") → boundary is the start of "ccc" (just after the \n at 7).
        assert_eq!(last_line_boundary(&path, eof), Some(8));
        // EOF exactly on a line boundary (just after "bbb\n") → that same offset (8 is the
        // byte after the \n at 7; for eof=8 the boundary is 8 itself).
        assert_eq!(last_line_boundary(&path, 8), Some(8));
        // EOF == 0 → 0 (nothing before it).
        assert_eq!(last_line_boundary(&path, 0), Some(0));
        // A file with no newline at all → None (degenerate; caller falls back to eof).
        std::fs::write(&path, b"no-newline-here").unwrap();
        let eof2 = std::fs::metadata(&path).unwrap().len();
        assert_eq!(last_line_boundary(&path, eof2), None);
        let _ = std::fs::remove_file(&path);
    }
}
