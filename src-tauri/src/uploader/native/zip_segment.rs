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

/// DEFLATE level for the entry. Level 3 keeps the standard DEFLATE ZIP envelope
/// but avoids spending user-visible upload time chasing the final few percent of
/// compression ratio. On representative segment text it is ~2.5x faster than
/// level 9 while staying much smaller than level 1.
const DEFLATE_LEVEL: i64 = 3;

/// Wrap a rendered segment / master-table text in the ZIP envelope the upload
/// expects: a single DEFLATE entry named `log.txt`. Returns the archive bytes
/// for the `logfile` multipart part.
///
/// Deterministic: a fixed entry name and a fixed (ZIP-epoch) modification time, so
/// equal input → byte-identical output. The only failure modes are an internal
/// `zip`/IO error (a single in-memory buffer, so these are not expected in
/// practice); they surface as a short message rather than panicking.
pub fn zip_log_txt(text: &str) -> Result<Vec<u8>, String> {
    zip_log_txt_from_writer(|entry| {
        entry
            .write_all(text.as_bytes())
            .map_err(|e| format!("zip: write entry failed: {e}"))
    })
}

/// Streaming variant of [`zip_log_txt`]. The caller writes the `log.txt` entry
/// directly into the ZIP encoder, avoiding a large intermediate rendered string
/// when the final upload only needs compressed bytes.
pub fn zip_log_txt_from_writer<F>(write_entry: F) -> Result<Vec<u8>, String>
where
    F: FnOnce(&mut dyn Write) -> Result<(), String>,
{
    zip_log_txt_from_writer_with_level(DEFLATE_LEVEL, write_entry)
}

fn zip_log_txt_from_writer_with_level<F>(
    deflate_level: i64,
    write_entry: F,
) -> Result<Vec<u8>, String>
where
    F: FnOnce(&mut dyn Write) -> Result<(), String>,
{
    // In-memory cursor: the whole archive is small relative to the raw log, and
    // the client wants owned bytes for the multipart body.
    let mut writer = ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(deflate_level))
        // Fixed timestamp → deterministic, reproducible archive bytes (and no
        // wall-clock read). The ZIP epoch (1980-01-01) is `DateTime::default()`.
        .last_modified_time(DateTime::default());

    writer
        .start_file(ENTRY_NAME, options)
        .map_err(|e| format!("zip: start entry failed: {e}"))?;
    write_entry(&mut writer)?;
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

    #[test]
    #[ignore = "perf benchmark: compares ZIP deflate levels"]
    fn bench_deflate_levels() {
        let mut text = String::with_capacity(32 * 1024 * 1024);
        for i in 0..1_000_000 {
            use std::fmt::Write as _;
            let a = (i % 20_000) + 1;
            let b = (i % 4) * 16;
            let c = (i % 17) + 1;
            writeln!(
                text,
                "{}|5|{}.{}.{}|{}|{}|A{}",
                i * 83,
                a,
                b,
                c,
                16 + (i % 3) * 16,
                16 + (i % 5) * 16,
                c
            )
            .unwrap();
        }
        eprintln!("\n=== ZIP DEFLATE LEVELS ===");
        eprintln!(
            "  input text   : {:.1} MiB",
            text.len() as f64 / (1024.0 * 1024.0)
        );
        for level in [1, 3, 6, 9] {
            let t = std::time::Instant::now();
            let bytes = zip_log_txt_from_writer_with_level(level, |entry| {
                entry
                    .write_all(text.as_bytes())
                    .map_err(|e| format!("zip: write entry failed: {e}"))
            })
            .expect("zip");
            let dt = t.elapsed();
            eprintln!(
                "  level {level:<2}    : {:>6.2} s, {:>6.1} MiB, ratio {:.2}%",
                dt.as_secs_f64(),
                bytes.len() as f64 / (1024.0 * 1024.0),
                100.0 * bytes.len() as f64 / text.len() as f64
            );
        }
    }
}
