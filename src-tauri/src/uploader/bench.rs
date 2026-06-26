//! Performance benchmark for the oversized-log pipeline: the streaming **split**
//! (byte-range copy on `BEGIN_LOG` boundaries) and the buffered **native encode**
//! (raw log → ESO Logs segment + master table).
//!
//! Purpose: quantify Kalpa against the sheumais/logs reference's published figures
//! (~6 s / ~40 MB RAM for a 1 GB log) and record where each stage stands.
//!
//! This is an `#[ignore]`d test, not a shipping code path: it needs a real
//! multi-GB log (`.decode-samples/sunspire_raw.log`, gitignored/machine-local) and
//! is meant to be run by hand, in `--release`, with peak-heap tracking:
//!
//! ```text
//! cargo test --release --features bench-alloc -p kalpa \
//!     uploader::bench -- --ignored --nocapture
//! ```
//!
//! Without `--features bench-alloc` the timings are still produced; the peak-heap
//! line reports `n/a` (the tracking allocator isn't installed — see `bench_alloc`).
//! When the sample log is absent the benchmark SKIPS (prints a notice and returns)
//! so it never fails CI or another machine.

use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Resolve a gitignored sample log under the worktree's `.decode-samples/`.
/// `CARGO_MANIFEST_DIR` is `…/src-tauri`, so the corpus sits one level up.
fn sample(name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".decode-samples")
        .join(name);
    p.is_file().then_some(p)
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// Throughput in MB/s (decimal MB, matching how disk/encoder speeds are usually
/// quoted), guarding against a divide-by-zero on an absurdly fast run.
fn mb_per_s(bytes: u64, dt: Duration) -> f64 {
    let secs = dt.as_secs_f64();
    if secs <= 0.0 {
        return f64::INFINITY;
    }
    (bytes as f64 / 1_000_000.0) / secs
}

/// Format the peak-heap figure, or `n/a` when the tracking allocator isn't active.
fn peak_heap_line() -> String {
    let peak = crate::bench_alloc::peak();
    if peak == 0 {
        "n/a (re-run with --features bench-alloc for peak heap)".to_string()
    } else {
        format!("{:.1} MiB", mib(peak as u64))
    }
}

/// Benchmark the streaming split of a ~1 GB log: full scan + per-session byte-range
/// copy. Expected to be IO-bound and flat-memory (an 8 MiB copy buffer + a 1 MiB
/// scan buffer, independent of file size) — the axis where Kalpa beats Archon's
/// choke on multi-GB files.
#[test]
#[ignore = "perf benchmark: needs .decode-samples/sunspire_raw.log; run --release"]
fn bench_split_one_gb() {
    let Some(log) = sample("sunspire_raw.log") else {
        eprintln!("SKIP bench_split_one_gb: .decode-samples/sunspire_raw.log not present");
        return;
    };
    let size = std::fs::metadata(&log).unwrap().len();
    let tmp = tempfile::tempdir().unwrap();

    crate::bench_alloc::reset_peak();
    let t = Instant::now();
    let written = super::splitter::split_by_session(
        log.to_str().unwrap(),
        tmp.path().to_str().unwrap(),
        None, // scan internally — measures the full split (scan + copy)
    )
    .expect("split should succeed on a valid log");
    let dt = t.elapsed();
    let peak = peak_heap_line();

    eprintln!("\n=== SPLIT (streaming byte-range copy) ===");
    eprintln!(
        "  input        : {:.1} MiB ({:.2} GB)",
        mib(size),
        size as f64 / 1e9
    );
    eprintln!("  sessions out : {}", written.len());
    eprintln!("  wall time    : {:.2} s", dt.as_secs_f64());
    eprintln!("  throughput   : {:.0} MB/s", mb_per_s(size, dt));
    eprintln!("  peak heap    : {peak}");
    eprintln!("  (sheumais reference for a 1 GB log: ~6 s; split is IO-bound + flat-memory)");

    // Clean up the (potentially multi-GB) split output eagerly; tempdir would too,
    // but a giant log's splits are large enough to want gone the moment we're done.
    drop(tmp);
}

/// Benchmark the buffered native encode (raw → segment + master). Production caps a
/// single native encode at 256 MiB (`MAX_NATIVE_BYTES`) and splits first, so this
/// feeds the leading, BEGIN_LOG-anchored ~200 MiB of a real trial log — a realistic
/// large single-session encode workload. Unlike the split, this path buffers the
/// whole input (read_to_string + `Vec<&str>` + the rendered segment String), so peak
/// heap scales with input size; recording it shows the gap to a streaming encoder.
#[test]
#[ignore = "perf benchmark: needs .decode-samples/sunspire_raw.log; run --release"]
fn bench_encode_chunk() {
    let Some(log) = sample("sunspire_raw.log") else {
        eprintln!("SKIP bench_encode_chunk: .decode-samples/sunspire_raw.log not present");
        return;
    };

    // Read the leading ≤200 MiB (well under the 256 MiB native ceiling), trimmed to
    // the last newline so we feed only whole lines and keep the BEGIN_LOG header.
    const CHUNK: usize = 200 * 1024 * 1024;
    let mut f = std::fs::File::open(&log).unwrap();
    let mut buf = vec![0u8; CHUNK];
    let n = {
        let mut filled = 0;
        loop {
            match f.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(k) => {
                    filled += k;
                    if filled == buf.len() {
                        break;
                    }
                }
                Err(e) => panic!("read sample: {e}"),
            }
        }
        filled
    };
    let end = buf[..n]
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|p| p + 1)
        .unwrap_or(n);
    let contents = String::from_utf8_lossy(&buf[..end]).into_owned();
    drop(buf);
    let chunk_bytes = contents.len() as u64;

    // Measure the full buffered-encode region: line slicing + the multi-pass encode,
    // exactly the work transport::run_native_upload does after read_to_string.
    crate::bench_alloc::reset_peak();
    let t = Instant::now();
    let lines: Vec<&str> = contents.lines().collect();
    let payload = super::native::events::build_native_payload(&lines)
        .expect("encode should not error on a valid log prefix");
    let dt = t.elapsed();
    let peak = peak_heap_line();

    let (seg_len, master_len) = match &payload {
        Some((seg, master)) => (seg.bytes.len(), master.bytes.len()),
        None => (0, 0),
    };

    eprintln!("\n=== ENCODE (buffered raw → segment + master) ===");
    eprintln!(
        "  input chunk  : {:.1} MiB ({} lines)",
        mib(chunk_bytes),
        lines.len()
    );
    eprintln!(
        "  produced     : {}",
        if payload.is_some() {
            "segment + master"
        } else {
            "none (gated/empty)"
        }
    );
    eprintln!("  segment zip  : {:.2} MiB", mib(seg_len as u64));
    eprintln!("  master zip   : {:.2} MiB", mib(master_len as u64));
    eprintln!("  wall time    : {:.2} s", dt.as_secs_f64());
    eprintln!("  throughput   : {:.0} MB/s", mb_per_s(chunk_bytes, dt));
    eprintln!("  peak heap    : {peak}");
    eprintln!("  NOTE: encode buffers the whole input (peak ∝ size) and is capped at");
    eprintln!("        256 MiB in production (split first). A streaming encoder is the");
    eprintln!("        path to matching sheumais's ~40 MB-at-1 GB RAM (future work).");
}
