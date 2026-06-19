//! ZIP packaging for the `logfile` multipart field.
//!
//! The `/desktop-client/*` upload protocol does not send the segment or
//! master-table **text** directly — each is wrapped in a ZIP archive containing a
//! single entry named `log.txt`, DEFLATE-compressed, and posted as the `logfile`
//! part (filename `blob`). This module is that one transform: a rendered segment /
//! master-table [`String`] → the ZIP bytes the client sends.
//!
//! It is the missing step between [`super::serialize`] (which renders the text)
//! and [`super::client`] (whose [`super::client::Segment`] /
//! [`super::client::MasterTableBytes`] carry *already-ZIP-compressed* bytes).
//!
//! ## Determinism
//!
//! The archive is built with a **fixed** modification time (the ZIP epoch,
//! 1980-01-01) and a single fixed entry name, so the same input always produces
//! byte-identical output. That is what makes [`zip_log_txt`] golden-testable and
//! keeps it free of any wall-clock read (which the encoder must avoid anyway). The
//! server only unzips and re-parses the entry, so the stored timestamp is inert.
//!
//! Clean-room: ZIP is a public container format and DEFLATE a public codec; the
//! "single `log.txt` entry" shape is a protocol fact about the service. The
//! implementation here is from scratch using the `zip` crate.

use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

/// The single entry name every upload archive uses (segment text and master-table
/// text alike), confirmed from the protocol.
const ENTRY_NAME: &str = "log.txt";

/// DEFLATE level for the entry. Level 9 is standard "best" DEFLATE — it maps to
/// the `flate2` backend's maximum (not the slower zopfli path, which the `zip`
/// crate only selects for levels *above* 9), matching the official uploader's
/// "compress as a normal ZIP" behavior.
const DEFLATE_LEVEL: i64 = 9;

/// Wrap a rendered segment / master-table text in the ZIP envelope the upload
/// expects: a single DEFLATE-9 entry named `log.txt`. Returns the archive bytes
/// for the `logfile` multipart part.
///
/// Deterministic: a fixed entry name and a fixed (ZIP-epoch) modification time, so
/// equal input → byte-identical output. The only failure modes are an internal
/// `zip`/IO error (a single in-memory buffer, so these are not expected in
/// practice); they surface as a short message rather than panicking.
pub fn zip_log_txt(text: &str) -> Result<Vec<u8>, String> {
    // In-memory cursor: the whole archive is small relative to the raw log, and
    // the client wants owned bytes for the multipart body.
    let mut writer = ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(DEFLATE_LEVEL))
        // Fixed timestamp → deterministic, reproducible archive bytes (and no
        // wall-clock read). The ZIP epoch (1980-01-01) is `DateTime::default()`.
        .last_modified_time(DateTime::default());

    writer
        .start_file(ENTRY_NAME, options)
        .map_err(|e| format!("zip: start entry failed: {e}"))?;
    writer
        .write_all(text.as_bytes())
        .map_err(|e| format!("zip: write entry failed: {e}"))?;
    let cursor = writer
        .finish()
        .map_err(|e| format!("zip: finalize failed: {e}"))?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// Unzip the single `log.txt` entry back to a string — the inverse used to
    /// prove the round-trip.
    fn unzip_log_txt(bytes: &[u8]) -> String {
        let mut archive =
            zip::ZipArchive::new(Cursor::new(bytes.to_vec())).expect("open zip archive");
        assert_eq!(archive.len(), 1, "archive must hold exactly one entry");
        let mut file = archive.by_index(0).expect("entry 0");
        assert_eq!(file.name(), ENTRY_NAME, "entry must be named log.txt");
        let mut out = String::new();
        file.read_to_string(&mut out).expect("read entry");
        out
    }

    #[test]
    fn round_trips_through_zip() {
        // A realistic fights-segment-shaped payload (header + count + events).
        let text = "15|1\n3\n0|41|1129|Hall|0\n87969|5|1|16|16|A1\n87969|7|1|16|16\n";
        let bytes = zip_log_txt(text).expect("zip");
        assert_eq!(unzip_log_txt(&bytes), text, "unzip must recover the input");
    }

    #[test]
    fn round_trips_empty_and_large_payloads() {
        // Empty (degenerate but must not panic).
        let empty = zip_log_txt("").expect("zip empty");
        assert_eq!(unzip_log_txt(&empty), "");

        // A large payload exercises the multi-block DEFLATE path.
        let big: String = (0..50_000)
            .map(|i| format!("{i}|5|{i}|16|16|A{i}\n"))
            .collect();
        let bytes = zip_log_txt(&big).expect("zip big");
        assert_eq!(unzip_log_txt(&bytes), big);
        // The ZIP must actually compress this highly-repetitive text.
        assert!(
            bytes.len() < big.len(),
            "DEFLATE must shrink repetitive text ({} >= {})",
            bytes.len(),
            big.len()
        );
    }

    #[test]
    fn output_is_deterministic() {
        // Equal input → byte-identical archives (fixed entry name + timestamp).
        let text = "15|1\n0\n";
        let a = zip_log_txt(text).expect("a");
        let b = zip_log_txt(text).expect("b");
        assert_eq!(a, b, "the archive must be byte-stable for equal input");
    }

    #[test]
    fn entry_is_named_log_txt() {
        let bytes = zip_log_txt("x").expect("zip");
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("open");
        assert_eq!(archive.by_index(0).unwrap().name(), "log.txt");
    }
}
