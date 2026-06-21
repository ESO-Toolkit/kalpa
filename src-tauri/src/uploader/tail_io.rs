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
}
