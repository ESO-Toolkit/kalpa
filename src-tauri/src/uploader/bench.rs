//! Performance benchmark for the oversized-log pipeline: the streaming **split**
//! (byte-range copy on `BEGIN_LOG` boundaries) and the memory-aware **native encode**
//! (raw log → ESO Logs segment + master table).
//!
//! Purpose: quantify Kalpa's oversized-log throughput and peak memory and record
//! where each stage stands against the performance target (a few seconds and low,
//! flat memory for a ~1 GB log).
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
//!
//! RECORDED BASELINE (2026-06-25, `--release`, single-threaded, sunspire_raw.log
//! 927.8 MiB / 0.97 GB; run with `--test-threads=1` so the shared peak-heap counter
//! isn't cross-contaminated by the two tests running in parallel):
//!
//! * SPLIT  : 2.72 s, 357 MB/s, **peak heap 8.0 MiB** (= the 8 MiB copy buffer →
//!   truly O(1) memory) — comfortably inside the target on both axes.
//! * ENCODE : previous baseline, 200 MiB chunk in 8.74 s, 24 MB/s, peak heap
//!   462.7 MiB (~2.3x input) from `read_to_string` + a retained `Vec<&str>` +
//!   rendered segment text. Re-run this benchmark after memory changes to record
//!   the new peak on the same corpus.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const BENCH_NATIVE_UPLOAD_SAMPLE: &str = "sunspire_raw.log";
/// Mirrors the production native finished-upload ceiling. The fallback sample
/// copies the largest single session up to this size so the real-service proof
/// never accidentally exercises a multi-session/too-large input.
const BENCH_NATIVE_UPLOAD_MAX_BYTES: usize = 256 * 1024 * 1024;

/// Resolve a gitignored sample log under the worktree's `.decode-samples/`.
/// `CARGO_MANIFEST_DIR` is `…/src-tauri`, so the corpus sits one level up.
fn sample(name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".decode-samples")
        .join(name);
    p.is_file().then_some(p)
}

fn bench_env(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Some(value),
        _ => bench_env_file_values().get(name).cloned(),
    }
}

fn bench_upload_cookie() -> Option<String> {
    bench_env("KALPA_BENCH_ESOLOGS_COOKIE").or_else(|| {
        crate::token_store::load_upload_session().filter(|cookie| !cookie.trim().is_empty())
    })
}

struct BenchUploadLog {
    path: PathBuf,
    _temp: Option<tempfile::NamedTempFile>,
}

impl BenchUploadLog {
    fn path_str(&self) -> &str {
        self.path
            .to_str()
            .expect("benchmark upload log path must be UTF-8")
    }
}

fn bench_upload_log() -> Option<BenchUploadLog> {
    if let Some(path) = bench_env("KALPA_BENCH_NATIVE_UPLOAD_LOG") {
        return Some(BenchUploadLog {
            path: PathBuf::from(path),
            _temp: None,
        });
    }

    let source = sample(BENCH_NATIVE_UPLOAD_SAMPLE)?;
    let temp = copy_largest_session_prefix_line_aligned(&source, BENCH_NATIVE_UPLOAD_MAX_BYTES);
    let path = temp.path().to_path_buf();
    if let Ok(meta) = std::fs::metadata(&path) {
        eprintln!(
            "[bench] KALPA_BENCH_NATIVE_UPLOAD_LOG unset; using largest single-session prefix \
             from .decode-samples/{BENCH_NATIVE_UPLOAD_SAMPLE} ({:.1} MiB)",
            mib(meta.len())
        );
    }
    Some(BenchUploadLog {
        path,
        _temp: Some(temp),
    })
}

fn bench_env_file_values() -> &'static HashMap<String, String> {
    static VALUES: OnceLock<HashMap<String, String>> = OnceLock::new();
    VALUES.get_or_init(|| {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        bench_env_file_candidates(&repo_root)
            .into_iter()
            .find_map(|path| load_bench_env_file(&path).ok())
            .unwrap_or_default()
    })
}

fn bench_env_file_candidates(repo_root: &Path) -> Vec<PathBuf> {
    let explicit_env_file = std::env::var("KALPA_BENCH_ENV_FILE").ok();
    let main_worktree = git_common_worktree_root(repo_root);
    let cwd = std::env::current_dir().ok();
    bench_env_file_candidates_from(
        repo_root,
        explicit_env_file.as_deref(),
        main_worktree.as_deref(),
        cwd.as_deref(),
    )
}

fn bench_env_file_candidates_from(
    repo_root: &Path,
    explicit_env_file: Option<&str>,
    main_worktree: Option<&Path>,
    cwd: Option<&Path>,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = explicit_env_file {
        let path = path.trim();
        if !path.is_empty() {
            push_unique_path(&mut paths, PathBuf::from(path));
        }
    }

    push_unique_path(&mut paths, repo_root.join(".env.bench.local"));

    if let Some(main_worktree) = main_worktree {
        push_unique_path(&mut paths, main_worktree.join(".env.bench.local"));
    }

    if let Some(cwd) = cwd {
        push_unique_path(&mut paths, cwd.join(".env.bench.local"));
    }

    paths
}

fn git_common_worktree_root(repo_root: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    let common_dir = PathBuf::from(raw.trim());
    common_dir.parent().map(Path::to_path_buf)
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    let key = bench_env_path_key(&path);
    if paths
        .iter()
        .any(|existing| bench_env_path_key(existing) == key)
    {
        return;
    }
    paths.push(path);
}

fn bench_env_path_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

fn load_bench_env_file(path: &Path) -> Result<HashMap<String, String>, std::io::Error> {
    let raw = std::fs::read_to_string(path)?;
    let mut values = HashMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        values.insert(key.to_string(), parse_bench_env_value(value));
    }
    Ok(values)
}

fn parse_bench_env_value(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 {
        let first = value.as_bytes()[0];
        let last = value.as_bytes()[value.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

fn count_lines(path: &Path) -> usize {
    let file = std::fs::File::open(path).expect("open line-count input");
    let mut reader = std::io::BufReader::with_capacity(64 * 1024, file);
    let mut line = String::new();
    let mut count = 0usize;
    loop {
        line.clear();
        let read =
            std::io::BufRead::read_line(&mut reader, &mut line).expect("read line-count input");
        if read == 0 {
            break;
        }
        count += 1;
    }
    count
}

fn copy_largest_session_prefix_line_aligned(
    source: &Path,
    target_bytes: usize,
) -> tempfile::NamedTempFile {
    let scan = super::scanner::scan_file(source.to_str().expect("sample path must be UTF-8"))
        .expect("scan prefix source");
    let session = scan
        .sessions
        .iter()
        .max_by_key(|session| session.size_bytes)
        .expect("sample must contain at least one session");
    let mut src = std::fs::File::open(source).expect("open prefix source");
    std::io::Seek::seek(&mut src, std::io::SeekFrom::Start(session.start_offset))
        .expect("seek prefix source to session");
    let mut reader = std::io::BufReader::with_capacity(64 * 1024, src);
    let mut out = tempfile::NamedTempFile::new().expect("create prefix temp file");
    let mut copied = 0usize;
    let session_bytes = session.size_bytes as usize;
    let target_bytes = target_bytes.min(session_bytes);
    let mut line = String::new();
    loop {
        line.clear();
        let read = std::io::BufRead::read_line(&mut reader, &mut line).expect("read prefix source");
        if read == 0 {
            break;
        }
        if copied > 0 && copied.saturating_add(read) > target_bytes {
            break;
        }
        if copied.saturating_add(read) > session_bytes {
            break;
        }
        std::io::Write::write_all(&mut out, line.as_bytes()).expect("write prefix temp file");
        copied = copied.saturating_add(read);
        if copied >= target_bytes || copied >= session_bytes {
            break;
        }
    }
    std::io::Write::flush(&mut out).expect("flush prefix temp file");
    out
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

fn minutes(ms: u64) -> f64 {
    ms as f64 / 60_000.0
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(feature = "bench-alloc")]
const PEAK_TARGET_BYTES: usize = 50 * 1024 * 1024;

#[cfg(feature = "bench-alloc")]
const OLD_ENCODE_BASELINE_PEAK_BYTES: usize = 485_176_115; // 462.7 MiB

/// Format the peak-heap figure, or `n/a` when the tracking allocator isn't active.
fn peak_heap_line() -> String {
    let peak = crate::bench_alloc::peak();
    if peak == 0 {
        "n/a (re-run with --features bench-alloc for peak heap)".to_string()
    } else {
        format!("{:.1} MiB", mib(peak as u64))
    }
}

#[cfg(feature = "bench-alloc")]
fn peak_delta_from(baseline: usize) -> usize {
    crate::bench_alloc::peak().saturating_sub(baseline)
}

struct EnvSession(String);

impl super::native::session::SessionProvider for EnvSession {
    fn session(
        &self,
    ) -> Result<super::native::session::Session, super::native::session::SessionError> {
        Ok(super::native::session::Session::from_cookie_header(
            self.0.clone(),
        ))
    }

    fn invalidate(&self) {}
}

/// Real-service validation hook for the native finished-upload path. This creates
/// an actual ESO Logs report, so it is ignored and triple-gated:
///
/// ```text
/// $env:KALPA_BENCH_NATIVE_UPLOAD_LOG = "C:\path\to\prepared.log"
/// # If omitted, the benchmark uses the largest single-session prefix from
/// # .decode-samples/sunspire_raw.log when that local corpus exists.
/// $env:KALPA_BENCH_ESOLOGS_COOKIE = "wcl_session=...; XSRF-TOKEN=..."
/// # Or sign in to ESO Logs inside Kalpa; the benchmark can reuse that stored upload session.
/// $env:KALPA_BENCH_NATIVE_UPLOAD_CONFIRM = "upload"
/// # Optional browser readiness timing after upload completion:
/// # $env:KALPA_BENCH_REPORT_READY_BROWSER = "1"
/// # Optional comparison against an external baseline, e.g. official uploader:
/// # $env:KALPA_BENCH_BASELINE_FULLY_LOADED_MS = "120000"
/// # $env:KALPA_BENCH_REQUIRE_10X = "1"
/// # Optional JSON proof artifact; defaults to src-tauri/target/native-upload-proof-<code>.json
/// # $env:KALPA_BENCH_PROOF_JSON = "C:\path\to\native-upload-proof.json"
/// cargo test --release uploader::bench::bench_real_native_upload_finished_log -- --ignored --nocapture
/// ```
///
/// The log must pass the same production native-routing preflight. The cookie is never
/// printed. The report is created as Private by default. Browser readiness timing
/// requires `npm install` and `npx playwright install chromium` in the repo root.
#[test]
#[ignore = "real ESO Logs upload; requires env-gated credentials and confirmation"]
fn bench_real_native_upload_finished_log() {
    if bench_env("KALPA_BENCH_NATIVE_UPLOAD_CONFIRM").as_deref() != Some("upload") {
        eprintln!(
            "SKIP bench_real_native_upload_finished_log: set \
             KALPA_BENCH_NATIVE_UPLOAD_CONFIRM=upload"
        );
        return;
    }
    let Some(cookie) = bench_upload_cookie() else {
        eprintln!(
            "SKIP bench_real_native_upload_finished_log: set KALPA_BENCH_ESOLOGS_COOKIE \
             or sign in to ESO Logs inside Kalpa"
        );
        return;
    };
    let Some(upload_log) = bench_upload_log() else {
        eprintln!(
            "SKIP bench_real_native_upload_finished_log: set KALPA_BENCH_NATIVE_UPLOAD_LOG \
             or stage .decode-samples/{BENCH_NATIVE_UPLOAD_SAMPLE}"
        );
        return;
    };
    let log_path = upload_log.path_str();
    let browser_ready_enabled =
        bench_env("KALPA_BENCH_REPORT_READY_BROWSER").as_deref() == Some("1");
    let baseline_env = bench_env("KALPA_BENCH_BASELINE_FULLY_LOADED_MS");
    validate_real_upload_10x_gate(
        bench_env("KALPA_BENCH_REQUIRE_10X").as_deref() == Some("1"),
        browser_ready_enabled,
        baseline_env.as_deref(),
    )
    .expect("10x proof gate is misconfigured");

    let full_started = Instant::now();
    let routing_started = Instant::now();
    let routing = super::transport::assess_native_routing(log_path, true);
    let routing_dt = routing_started.elapsed();
    if !matches!(routing, super::transport::NativeRouting::Native) {
        eprintln!(
            "SKIP bench_real_native_upload_finished_log: log does not route to native upload"
        );
        return;
    }
    if browser_ready_enabled
        && !browser_ready_candidate_or_skip(log_path, "bench_real_native_upload_finished_log")
    {
        return;
    }

    let input_bytes = std::fs::metadata(log_path)
        .expect("benchmark log metadata")
        .len();

    let build_started = Instant::now();
    let payloads =
        super::native::live::build_finished_payloads_from_file(std::path::Path::new(&log_path))
            .expect("build native payloads")
            .expect("benchmark log must contain a valid native session");
    let build_dt = build_started.elapsed();
    let payload_proof = real_upload_payload_proof(
        &payloads,
        super::native::live::FINISHED_UPLOAD_SEGMENT_RAW_BYTE_TARGET,
    );

    let mut segments = Vec::with_capacity(payloads.len());
    let mut masters = Vec::with_capacity(payloads.len());
    for payload in payloads {
        segments.push(payload.segment);
        masters.push(payload.master);
    }

    let opts = super::types::UploadOptions {
        visibility: super::types::Visibility::Private,
        description: Some("Kalpa native uploader perf benchmark".into()),
        ..super::types::UploadOptions::default()
    };
    let session = EnvSession(cookie);
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let upload = super::native::client::NativeUpload::new(&session, &opts, cancel);
    let no_progress = |_p: super::native::client::UploadProgress| {};

    let result = upload
        .upload_finished_measured(&segments, &masters, &no_progress)
        .expect("real native upload should complete");
    let report_url = format!("https://www.esologs.com/reports/{}", result.code.0);
    let m = result.metrics;
    let mut proof = RealUploadProof {
        schema: "kalpa.native-upload-proof",
        schema_version: 1,
        measured_unix_ms: unix_now_ms(),
        report_url: report_url.clone(),
        input_bytes,
        payloads: payload_proof,
        timings: RealUploadTimingProof {
            routing_assess_ms: routing_dt.as_millis().min(u128::from(u64::MAX)) as u64,
            local_map_ms: 0,
            local_build_ms: build_dt.as_millis().min(u128::from(u64::MAX)) as u64,
            session_ms: m.session_ms,
            create_report_ms: m.create_report_ms,
            set_master_table_ms: m.set_master_table_ms,
            add_segment_ms: m.add_segment_ms,
            terminate_report_ms: m.terminate_report_ms,
            upload_total_ms: m.upload_total_ms,
        },
        browser: None,
        baseline: None,
    };

    eprintln!("\n=== REAL NATIVE UPLOAD (ESO Logs) ===");
    eprintln!("  report       : {report_url}");
    eprintln!("  input        : {:.1} MiB", mib(input_bytes));
    eprintln!(
        "  payloads     : {} segments, {} requests, {:.2}+{:.2} MiB zipped",
        m.segments_total,
        m.requests_total,
        mib(m.segment_zip_bytes as u64),
        mib(m.master_zip_bytes as u64)
    );
    eprintln!(
        "  raw chunks   : target {:.1} MiB, max {:.1} MiB, {} over target",
        mib(proof.payloads.raw_target_bytes as u64),
        mib(proof.payloads.max_raw_segment_bytes as u64),
        proof.payloads.segments_over_raw_target
    );
    eprintln!("  routing scan : {} ms", routing_dt.as_millis());
    eprintln!("  local input  : streamed from disk (no mmap)");
    eprintln!(
        "  local build  : {:.2} s, {:.0} MB/s",
        build_dt.as_secs_f64(),
        mb_per_s(input_bytes, build_dt)
    );
    eprintln!(
        "  remote post  : {} ms total (session {}, create {}, master {}, segment {}, terminate {})",
        m.upload_total_ms,
        m.session_ms,
        m.create_report_ms,
        m.set_master_table_ms,
        m.add_segment_ms,
        m.terminate_report_ms
    );
    if browser_ready_enabled {
        let browser = measure_report_ready(&report_url, full_started);
        let native_fully_loaded_ms = browser.native_fully_loaded_ms;
        eprintln!("\n=== REAL NATIVE FULLY LOADED ===");
        eprintln!(
            "  report ready : {} ms ({})",
            browser.ready_ms, browser.ready_source
        );
        eprintln!(
            "  browser wall : {} ms including process startup",
            browser.browser_wall_ms
        );
        eprintln!("  total native : {native_fully_loaded_ms} ms from native build start");
        proof.browser = Some(browser);
        proof.baseline = maybe_report_real_upload_ratio(native_fully_loaded_ms);
    } else {
        eprintln!(
            "  next step    : run `npm run measure:esologs-report -- {report_url}` \
             to time browser readiness"
        );
    }
    write_real_upload_proof(&result.code.0, &proof);
}

/// Real-service A/B proof hook for choosing the fastest native report-ready
/// protocol on the same finished log. This uploads the same locally-built payloads
/// twice:
///
/// * `finished` uses the production manual path (`isLiveLog:false`,
///   `isRealTime:false`, terminate after all segments).
/// * `liveStyle` uses the already-verified live request envelopes
///   (`isLiveLog:true`, `isRealTime:true`) and terminates immediately after the
///   final finished-log segment.
///
/// It is deliberately separate from `bench_real_native_upload_finished_log` because
/// it creates two private reports. Run only when you are ready to compare protocol
/// behavior against the real service:
///
/// ```text
/// $env:KALPA_BENCH_NATIVE_UPLOAD_LOG = "C:\path\to\prepared.log"
/// # If omitted, the benchmark uses the largest single-session prefix from
/// # .decode-samples/sunspire_raw.log when that local corpus exists.
/// $env:KALPA_BENCH_ESOLOGS_COOKIE = "wcl_session=...; XSRF-TOKEN=..."
/// # Or sign in to ESO Logs inside Kalpa; the benchmark can reuse that stored upload session.
/// $env:KALPA_BENCH_NATIVE_UPLOAD_CONFIRM = "upload"
/// $env:KALPA_BENCH_REPORT_READY_BROWSER = "1"
/// $env:KALPA_BENCH_PROTOCOL_COMPARE = "1"
/// cargo test --release uploader::bench::bench_real_native_upload_protocol_compare -- --ignored --nocapture
/// ```
#[test]
#[ignore = "real ESO Logs protocol A/B; creates two private reports"]
fn bench_real_native_upload_protocol_compare() {
    if bench_env("KALPA_BENCH_NATIVE_UPLOAD_CONFIRM").as_deref() != Some("upload") {
        eprintln!(
            "SKIP bench_real_native_upload_protocol_compare: set \
             KALPA_BENCH_NATIVE_UPLOAD_CONFIRM=upload"
        );
        return;
    }
    if bench_env("KALPA_BENCH_PROTOCOL_COMPARE").as_deref() != Some("1") {
        eprintln!(
            "SKIP bench_real_native_upload_protocol_compare: set KALPA_BENCH_PROTOCOL_COMPARE=1"
        );
        return;
    }
    if bench_env("KALPA_BENCH_REPORT_READY_BROWSER").as_deref() != Some("1") {
        eprintln!(
            "SKIP bench_real_native_upload_protocol_compare: set \
             KALPA_BENCH_REPORT_READY_BROWSER=1"
        );
        return;
    }
    let Some(cookie) = bench_upload_cookie() else {
        eprintln!(
            "SKIP bench_real_native_upload_protocol_compare: set KALPA_BENCH_ESOLOGS_COOKIE \
             or sign in to ESO Logs inside Kalpa"
        );
        return;
    };
    let Some(upload_log) = bench_upload_log() else {
        eprintln!(
            "SKIP bench_real_native_upload_protocol_compare: set KALPA_BENCH_NATIVE_UPLOAD_LOG \
             or stage .decode-samples/{BENCH_NATIVE_UPLOAD_SAMPLE}"
        );
        return;
    };
    let log_path = upload_log.path_str();

    let routing_started = Instant::now();
    let routing = super::transport::assess_native_routing(log_path, true);
    let routing_dt = routing_started.elapsed();
    if !matches!(routing, super::transport::NativeRouting::Native) {
        eprintln!(
            "SKIP bench_real_native_upload_protocol_compare: log does not route to native upload"
        );
        return;
    }
    if !browser_ready_candidate_or_skip(log_path, "bench_real_native_upload_protocol_compare") {
        return;
    }
    let input_bytes = std::fs::metadata(log_path)
        .expect("benchmark log metadata")
        .len();
    let build_started = Instant::now();
    let payloads =
        super::native::live::build_finished_payloads_from_file(std::path::Path::new(&log_path))
            .expect("build native payloads")
            .expect("benchmark log must contain a valid native session");
    let build_dt = build_started.elapsed();
    let local_elapsed_ms = duration_ms(routing_dt).saturating_add(duration_ms(build_dt));
    let payload_proof = real_upload_payload_proof(
        &payloads,
        super::native::live::FINISHED_UPLOAD_SEGMENT_RAW_BYTE_TARGET,
    );

    let opts = super::types::UploadOptions {
        visibility: super::types::Visibility::Private,
        description: Some("Kalpa native protocol comparison".into()),
        ..super::types::UploadOptions::default()
    };

    let mut segments = Vec::with_capacity(payloads.len());
    let mut masters = Vec::with_capacity(payloads.len());
    for payload in &payloads {
        segments.push(payload.segment.clone());
        masters.push(payload.master.clone());
    }

    let manual_session = EnvSession(cookie.clone());
    let manual_upload = super::native::client::NativeUpload::new(
        &manual_session,
        &opts,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );
    let no_progress = |_p: super::native::client::UploadProgress| {};
    let manual_result = manual_upload
        .upload_finished_measured(&segments, &masters, &no_progress)
        .expect("finished protocol upload should complete");
    let manual_url = format!("https://www.esologs.com/reports/{}", manual_result.code.0);
    let manual_elapsed_before_browser_ms =
        local_elapsed_ms.saturating_add(manual_result.metrics.upload_total_ms);
    let manual_browser =
        measure_report_ready_after_elapsed(&manual_url, manual_elapsed_before_browser_ms);

    let live_session = std::sync::Arc::new(EnvSession(cookie));
    let live_result = upload_finished_live_style_for_bench(live_session, &opts, &payloads)
        .expect("live-style protocol upload should complete");
    let live_url = format!("https://www.esologs.com/reports/{}", live_result.code.0);
    let live_elapsed_before_browser_ms =
        local_elapsed_ms.saturating_add(live_result.timings.upload_total_ms);
    let live_browser =
        measure_report_ready_after_elapsed(&live_url, live_elapsed_before_browser_ms);

    let proof = ProtocolCompareProof {
        schema: "kalpa.native-protocol-compare-proof",
        schema_version: 1,
        measured_unix_ms: unix_now_ms(),
        input_bytes,
        payloads: payload_proof,
        finished: ProtocolRunProof {
            report_url: manual_url,
            timings: RealUploadTimingProof {
                routing_assess_ms: duration_ms(routing_dt),
                local_map_ms: 0,
                local_build_ms: duration_ms(build_dt),
                session_ms: manual_result.metrics.session_ms,
                create_report_ms: manual_result.metrics.create_report_ms,
                set_master_table_ms: manual_result.metrics.set_master_table_ms,
                add_segment_ms: manual_result.metrics.add_segment_ms,
                terminate_report_ms: manual_result.metrics.terminate_report_ms,
                upload_total_ms: manual_result.metrics.upload_total_ms,
            },
            browser: manual_browser,
        },
        live_style: ProtocolRunProof {
            report_url: live_url,
            timings: RealUploadTimingProof {
                routing_assess_ms: duration_ms(routing_dt),
                local_map_ms: 0,
                local_build_ms: duration_ms(build_dt),
                ..live_result.timings
            },
            browser: live_browser,
        },
    };

    eprintln!("\n=== REAL NATIVE PROTOCOL COMPARE (ESO Logs) ===");
    eprintln!("  input        : {:.1} MiB", mib(input_bytes));
    eprintln!(
        "  payloads     : {} segments, {} requests, {:.2}+{:.2} MiB zipped",
        proof.payloads.segments_total,
        proof.payloads.requests_total,
        mib(proof.payloads.segment_zip_bytes as u64),
        mib(proof.payloads.master_zip_bytes as u64)
    );
    eprintln!(
        "  raw chunks   : target {:.1} MiB, max {:.1} MiB, {} over target",
        mib(proof.payloads.raw_target_bytes as u64),
        mib(proof.payloads.max_raw_segment_bytes as u64),
        proof.payloads.segments_over_raw_target
    );
    eprintln!("  routing scan : {} ms", duration_ms(routing_dt));
    eprintln!(
        "  local build  : {:.2} s, {:.0} MB/s",
        build_dt.as_secs_f64(),
        mb_per_s(input_bytes, build_dt)
    );
    eprintln!(
        "  finished     : {} ms fully loaded ({}) {}",
        proof.finished.browser.native_fully_loaded_ms,
        proof.finished.browser.ready_source,
        proof.finished.report_url
    );
    eprintln!(
        "  live-style   : {} ms fully loaded ({}) {}",
        proof.live_style.browser.native_fully_loaded_ms,
        proof.live_style.browser.ready_source,
        proof.live_style.report_url
    );
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("native-upload-protocol-compare.json");
    let json = serde_json::to_string_pretty(&proof).expect("serialize protocol compare proof");
    std::fs::write(&path, format!("{json}\n")).expect("write protocol compare proof");
    eprintln!("  proof json   : {}", path.display());
}

/// Real-service sweep for the completed-upload segment raw-byte target. This is
/// separate from the main 10x gate because it creates one private report per
/// candidate. Use it when the native path is correct but report-ready latency
/// needs tuning against ESO Logs' real parser/render pipeline:
///
/// ```text
/// $env:KALPA_BENCH_NATIVE_UPLOAD_LOG = "C:\path\to\prepared.log"
/// # If omitted, the benchmark uses the largest single-session prefix from
/// # .decode-samples/sunspire_raw.log when that local corpus exists.
/// $env:KALPA_BENCH_ESOLOGS_COOKIE = "wcl_session=...; XSRF-TOKEN=..."
/// # Or sign in to ESO Logs inside Kalpa; the benchmark can reuse that stored upload session.
/// $env:KALPA_BENCH_NATIVE_UPLOAD_CONFIRM = "upload"
/// $env:KALPA_BENCH_REPORT_READY_BROWSER = "1"
/// $env:KALPA_BENCH_SEGMENT_TARGET_SWEEP = "1"
/// # Optional; defaults to 16,64,96,192
/// # $env:KALPA_BENCH_SEGMENT_TARGETS_MIB = "8,16,24,32"
/// cargo test --release uploader::bench::bench_real_native_upload_segment_target_sweep -- --ignored --nocapture
/// ```
#[test]
#[ignore = "real ESO Logs segment target sweep; creates one private report per target"]
fn bench_real_native_upload_segment_target_sweep() {
    if bench_env("KALPA_BENCH_NATIVE_UPLOAD_CONFIRM").as_deref() != Some("upload") {
        eprintln!(
            "SKIP bench_real_native_upload_segment_target_sweep: set \
             KALPA_BENCH_NATIVE_UPLOAD_CONFIRM=upload"
        );
        return;
    }
    if bench_env("KALPA_BENCH_SEGMENT_TARGET_SWEEP").as_deref() != Some("1") {
        eprintln!(
            "SKIP bench_real_native_upload_segment_target_sweep: set \
             KALPA_BENCH_SEGMENT_TARGET_SWEEP=1"
        );
        return;
    }
    if bench_env("KALPA_BENCH_REPORT_READY_BROWSER").as_deref() != Some("1") {
        eprintln!(
            "SKIP bench_real_native_upload_segment_target_sweep: set \
             KALPA_BENCH_REPORT_READY_BROWSER=1"
        );
        return;
    }
    let Some(cookie) = bench_upload_cookie() else {
        eprintln!(
            "SKIP bench_real_native_upload_segment_target_sweep: set KALPA_BENCH_ESOLOGS_COOKIE \
             or sign in to ESO Logs inside Kalpa"
        );
        return;
    };
    let Some(upload_log) = bench_upload_log() else {
        eprintln!(
            "SKIP bench_real_native_upload_segment_target_sweep: set KALPA_BENCH_NATIVE_UPLOAD_LOG \
             or stage .decode-samples/{BENCH_NATIVE_UPLOAD_SAMPLE}"
        );
        return;
    };
    let log_path = upload_log.path_str();

    let targets = segment_target_mib_candidates().expect("segment target sweep env is invalid");
    let routing_started = Instant::now();
    let routing = super::transport::assess_native_routing(log_path, true);
    let routing_dt = routing_started.elapsed();
    if !matches!(routing, super::transport::NativeRouting::Native) {
        eprintln!(
            "SKIP bench_real_native_upload_segment_target_sweep: log does not route to native upload"
        );
        return;
    }
    if !browser_ready_candidate_or_skip(log_path, "bench_real_native_upload_segment_target_sweep") {
        return;
    }
    let input_bytes = std::fs::metadata(log_path)
        .expect("benchmark log metadata")
        .len();
    let opts = super::types::UploadOptions {
        visibility: super::types::Visibility::Private,
        description: Some("Kalpa native segment target sweep".into()),
        ..super::types::UploadOptions::default()
    };

    let mut runs = Vec::with_capacity(targets.len());
    eprintln!("\n=== REAL NATIVE SEGMENT TARGET SWEEP (ESO Logs) ===");
    eprintln!("  input        : {:.1} MiB", mib(input_bytes));
    eprintln!("  routing scan : {} ms", duration_ms(routing_dt));
    eprintln!(
        "  target | segments | requests | max raw | over | build s | upload ms | ready ms | fully loaded | report"
    );
    for raw_target_mib in targets {
        let raw_target_bytes = raw_target_mib
            .checked_mul(1024 * 1024)
            .expect("segment target MiB is too large");
        let build_started = Instant::now();
        let payloads = super::native::live::build_finished_payloads_from_file_limited(
            std::path::Path::new(&log_path),
            super::native::live::FINISHED_UPLOAD_FIGHTS_PER_SEGMENT,
            raw_target_bytes,
        )
        .expect("build native payloads")
        .expect("benchmark log must contain a valid native session");
        let build_dt = build_started.elapsed();

        let payload_proof = real_upload_payload_proof(&payloads, raw_target_bytes);
        let mut segments = Vec::with_capacity(payloads.len());
        let mut masters = Vec::with_capacity(payloads.len());
        for payload in payloads {
            segments.push(payload.segment);
            masters.push(payload.master);
        }

        let session = EnvSession(cookie.clone());
        let upload = super::native::client::NativeUpload::new(
            &session,
            &opts,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        let no_progress = |_p: super::native::client::UploadProgress| {};
        let result = upload
            .upload_finished_measured(&segments, &masters, &no_progress)
            .expect("segment target sweep upload should complete");
        let report_url = format!("https://www.esologs.com/reports/{}", result.code.0);
        let elapsed_before_browser_ms = duration_ms(routing_dt)
            .saturating_add(duration_ms(build_dt))
            .saturating_add(result.metrics.upload_total_ms);
        let browser = measure_report_ready_after_elapsed(&report_url, elapsed_before_browser_ms);

        eprintln!(
            "  {:>6} | {:>8} | {:>8} | {:>7.1} | {:>4} | {:>7.2} | {:>9} | {:>8} | {:>12} | {}",
            raw_target_mib,
            payload_proof.segments_total,
            payload_proof.requests_total,
            mib(payload_proof.max_raw_segment_bytes as u64),
            payload_proof.segments_over_raw_target,
            build_dt.as_secs_f64(),
            result.metrics.upload_total_ms,
            browser.ready_ms,
            browser.native_fully_loaded_ms,
            report_url
        );
        runs.push(SegmentTargetRunProof {
            raw_target_mib,
            report_url,
            payloads: payload_proof,
            timings: RealUploadTimingProof {
                routing_assess_ms: duration_ms(routing_dt),
                local_map_ms: 0,
                local_build_ms: duration_ms(build_dt),
                session_ms: result.metrics.session_ms,
                create_report_ms: result.metrics.create_report_ms,
                set_master_table_ms: result.metrics.set_master_table_ms,
                add_segment_ms: result.metrics.add_segment_ms,
                terminate_report_ms: result.metrics.terminate_report_ms,
                upload_total_ms: result.metrics.upload_total_ms,
            },
            browser,
        });
    }

    let proof = SegmentTargetSweepProof {
        schema: "kalpa.native-segment-target-sweep-proof",
        schema_version: 1,
        measured_unix_ms: unix_now_ms(),
        input_bytes,
        runs,
    };
    let path = bench_env("KALPA_BENCH_SEGMENT_SWEEP_PROOF_JSON")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("native-upload-segment-target-sweep.json")
        });
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).expect("create segment target sweep proof directory");
    }
    let json = serde_json::to_string_pretty(&proof).expect("serialize segment target sweep proof");
    std::fs::write(&path, format!("{json}\n")).expect("write segment target sweep proof");
    eprintln!("  proof json   : {}", path.display());
}

/// Experimental real-service sweep for splitting a completed long fight into
/// live-style in-progress fragments. This is deliberately NOT production behavior:
/// it exists to prove whether ESO Logs accepts and renders mid-fight fragments before
/// we consider using that protocol to reduce full-load time for single very long
/// fights.
///
/// ```text
/// $env:KALPA_BENCH_NATIVE_UPLOAD_LOG = "C:\path\to\prepared.log"
/// $env:KALPA_BENCH_ESOLOGS_COOKIE = "wcl_session=...; XSRF-TOKEN=..."
/// $env:KALPA_BENCH_NATIVE_UPLOAD_CONFIRM = "upload"
/// $env:KALPA_BENCH_REPORT_READY_BROWSER = "1"
/// $env:KALPA_BENCH_MIDFIGHT_LIVE_STYLE_SWEEP = "1"
/// # Optional; defaults to 16,64,96,192
/// # $env:KALPA_BENCH_SEGMENT_TARGETS_MIB = "8,16,24,32"
/// cargo test --release uploader::bench::bench_real_native_upload_midfight_live_style_sweep -- --ignored --nocapture
/// ```
#[test]
#[ignore = "experimental real ESO Logs mid-fight live-style sweep; creates one private report per target"]
fn bench_real_native_upload_midfight_live_style_sweep() {
    if bench_env("KALPA_BENCH_NATIVE_UPLOAD_CONFIRM").as_deref() != Some("upload") {
        eprintln!(
            "SKIP bench_real_native_upload_midfight_live_style_sweep: set \
             KALPA_BENCH_NATIVE_UPLOAD_CONFIRM=upload"
        );
        return;
    }
    if bench_env("KALPA_BENCH_MIDFIGHT_LIVE_STYLE_SWEEP").as_deref() != Some("1") {
        eprintln!(
            "SKIP bench_real_native_upload_midfight_live_style_sweep: set \
             KALPA_BENCH_MIDFIGHT_LIVE_STYLE_SWEEP=1"
        );
        return;
    }
    if bench_env("KALPA_BENCH_REPORT_READY_BROWSER").as_deref() != Some("1") {
        eprintln!(
            "SKIP bench_real_native_upload_midfight_live_style_sweep: set \
             KALPA_BENCH_REPORT_READY_BROWSER=1"
        );
        return;
    }
    let Some(cookie) = bench_upload_cookie() else {
        eprintln!(
            "SKIP bench_real_native_upload_midfight_live_style_sweep: set \
             KALPA_BENCH_ESOLOGS_COOKIE or sign in to ESO Logs inside Kalpa"
        );
        return;
    };
    let Some(upload_log) = bench_upload_log() else {
        eprintln!(
            "SKIP bench_real_native_upload_midfight_live_style_sweep: set \
             KALPA_BENCH_NATIVE_UPLOAD_LOG or stage .decode-samples/{BENCH_NATIVE_UPLOAD_SAMPLE}"
        );
        return;
    };
    let log_path = upload_log.path_str();
    let targets =
        segment_target_mib_candidates().expect("mid-fight segment target sweep env is invalid");
    let routing_started = Instant::now();
    let routing = super::transport::assess_native_routing(log_path, true);
    let routing_dt = routing_started.elapsed();
    if !matches!(routing, super::transport::NativeRouting::Native) {
        eprintln!(
            "SKIP bench_real_native_upload_midfight_live_style_sweep: log does not route to native upload"
        );
        return;
    }
    if !browser_ready_candidate_or_skip(
        log_path,
        "bench_real_native_upload_midfight_live_style_sweep",
    ) {
        return;
    }
    let input_bytes = std::fs::metadata(log_path)
        .expect("benchmark log metadata")
        .len();
    let opts = super::types::UploadOptions {
        visibility: super::types::Visibility::Private,
        description: Some("Kalpa native mid-fight live-style experiment".into()),
        ..super::types::UploadOptions::default()
    };

    let mut runs = Vec::with_capacity(targets.len());
    eprintln!("\n=== REAL NATIVE MID-FIGHT LIVE-STYLE SWEEP (ESO Logs) ===");
    eprintln!("  input        : {:.1} MiB", mib(input_bytes));
    eprintln!("  routing scan : {} ms", duration_ms(routing_dt));
    eprintln!(
        "  target | segments | in-prog | max raw | build s | upload ms | ready ms | fully loaded | report"
    );
    for raw_target_mib in targets {
        let raw_target_bytes = raw_target_mib
            .checked_mul(1024 * 1024)
            .expect("segment target MiB is too large");
        let build_started = Instant::now();
        let payloads = build_midfight_live_style_payloads_from_file_for_bench(
            std::path::Path::new(&log_path),
            raw_target_bytes,
        )
        .expect("build mid-fight live-style payloads")
        .expect("benchmark log must contain a valid native session");
        let build_dt = build_started.elapsed();

        let payload_proof = real_upload_payload_proof(&payloads, raw_target_bytes);
        let in_progress_segments = payloads
            .iter()
            .filter(|payload| payload.in_progress_event_count > 0)
            .count();
        let max_in_progress_event_count = payloads
            .iter()
            .map(|payload| payload.in_progress_event_count)
            .max()
            .unwrap_or(0);
        let session = std::sync::Arc::new(EnvSession(cookie.clone()));
        let result = upload_finished_live_style_for_bench(session, &opts, &payloads)
            .expect("mid-fight live-style upload should complete");
        let report_url = format!("https://www.esologs.com/reports/{}", result.code.0);
        let elapsed_before_browser_ms = duration_ms(routing_dt)
            .saturating_add(duration_ms(build_dt))
            .saturating_add(result.timings.upload_total_ms);
        let browser = measure_report_ready_after_elapsed(&report_url, elapsed_before_browser_ms);

        eprintln!(
            "  {:>6} | {:>8} | {:>7} | {:>7.1} | {:>7.2} | {:>9} | {:>8} | {:>12} | {}",
            raw_target_mib,
            payload_proof.segments_total,
            in_progress_segments,
            mib(payload_proof.max_raw_segment_bytes as u64),
            build_dt.as_secs_f64(),
            result.timings.upload_total_ms,
            browser.ready_ms,
            browser.native_fully_loaded_ms,
            report_url
        );
        runs.push(MidfightLiveStyleRunProof {
            raw_target_mib,
            report_url,
            payloads: payload_proof,
            in_progress_segments,
            max_in_progress_event_count,
            timings: RealUploadTimingProof {
                routing_assess_ms: duration_ms(routing_dt),
                local_map_ms: 0,
                local_build_ms: duration_ms(build_dt),
                ..result.timings
            },
            browser,
        });
    }

    let proof = MidfightLiveStyleSweepProof {
        schema: "kalpa.native-midfight-live-style-sweep-proof",
        schema_version: 1,
        measured_unix_ms: unix_now_ms(),
        input_bytes,
        runs,
    };
    let path = bench_env("KALPA_BENCH_MIDFIGHT_SWEEP_PROOF_JSON")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("native-upload-midfight-live-style-sweep.json")
        });
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).expect("create mid-fight sweep proof directory");
    }
    let json = serde_json::to_string_pretty(&proof).expect("serialize mid-fight sweep proof");
    std::fs::write(&path, format!("{json}\n")).expect("write mid-fight sweep proof");
    eprintln!("  proof json   : {}", path.display());
}

struct LiveStyleUploadResult {
    code: super::native::client::ReportCode,
    timings: RealUploadTimingProof,
}

fn upload_finished_live_style_for_bench(
    session: std::sync::Arc<dyn super::native::session::SessionProvider>,
    opts: &super::types::UploadOptions,
    payloads: &[super::native::live::LiveSegmentPayload],
) -> Result<LiveStyleUploadResult, super::native::client::UploadError> {
    use super::native::client::{
        create_report_body_for, desktop_client_base, parse_next_segment_id, parse_report_code,
        LiveSender, OwnedLiveRequest,
    };

    let started = Instant::now();
    let mut timings = RealUploadTimingProof {
        routing_assess_ms: 0,
        local_map_ms: 0,
        local_build_ms: 0,
        session_ms: 0,
        create_report_ms: 0,
        set_master_table_ms: 0,
        add_segment_ms: 0,
        terminate_report_ms: 0,
        upload_total_ms: 0,
    };
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let session_started = Instant::now();
    let _session = session.session()?;
    timings.session_ms = elapsed_ms(session_started);
    let sender = LiveSender::new(session);
    let base = desktop_client_base();

    let create_started = Instant::now();
    let code_body = sender.send_create_cancellable(
        &format!("{base}/create-report"),
        OwnedLiveRequest::CreateReport {
            body: create_report_body_for(opts),
        },
        &cancel,
    )?;
    timings.create_report_ms = elapsed_ms(create_started);
    let code = parse_report_code(&code_body)?;

    let result = (|| {
        let mut segment_id = 1u64;
        for (i, payload) in payloads.iter().enumerate() {
            let set_master_started = Instant::now();
            sender.send_cancellable(
                &format!("{base}/set-report-master-table/{}", code.0),
                OwnedLiveRequest::MasterTable {
                    segment_id,
                    bytes: payload.master.bytes.clone(),
                },
                &cancel,
            )?;
            timings.set_master_table_ms = timings
                .set_master_table_ms
                .saturating_add(elapsed_ms(set_master_started));

            let add_started = Instant::now();
            let body = sender.send_cancellable(
                &format!("{base}/add-report-segment/{}", code.0),
                OwnedLiveRequest::AddSegment {
                    segment_id,
                    bytes: payload.segment.bytes.clone(),
                    start_time: payload.segment.start_time,
                    end_time: payload.segment.end_time,
                    in_progress_event_count: payload.in_progress_event_count,
                },
                &cancel,
            )?;
            timings.add_segment_ms = timings
                .add_segment_ms
                .saturating_add(elapsed_ms(add_started));
            let next = parse_next_segment_id(&body)?;
            let is_last = i + 1 == payloads.len();
            if next == 0 && !is_last {
                return Err(super::native::client::UploadError::Server {
                    status: 0,
                    detail: format!(
                        "server returned terminal nextSegmentId=0 after segment {} of {}",
                        i + 1,
                        payloads.len()
                    ),
                });
            }
            if !is_last {
                segment_id = next;
            }
        }
        Ok(())
    })();

    let terminate_started = Instant::now();
    let terminate = sender.send_cancellable(
        &format!("{base}/terminate-report/{}", code.0),
        OwnedLiveRequest::Terminate,
        &cancel,
    );
    timings.terminate_report_ms = elapsed_ms(terminate_started);
    result?;
    terminate?;
    timings.upload_total_ms = elapsed_ms(started);
    Ok(LiveStyleUploadResult { code, timings })
}

fn build_midfight_live_style_payloads_from_file_for_bench(
    path: &Path,
    raw_byte_target: usize,
) -> Result<Option<Vec<super::native::live::LiveSegmentPayload>>, String> {
    let source = super::native::encode::FileLineReplay::new(path);
    build_midfight_live_style_payloads_from_replay_for_bench(&source, raw_byte_target)
}

fn build_midfight_live_style_payloads_from_replay_for_bench(
    source: &impl super::native::encode::LineReplay,
    raw_byte_target: usize,
) -> Result<Option<Vec<super::native::live::LiveSegmentPayload>>, String> {
    let mut seg = super::native::live::LiveSegmenter::new();
    let mut out = Vec::new();
    let mut seen_begin_log = false;
    let raw_byte_target = raw_byte_target.max(1);
    let mut segment_raw_bytes = 0usize;

    source.replay_lines(&mut |line| {
        let raw_line_bytes = line.len().saturating_add(1);
        segment_raw_bytes = segment_raw_bytes.saturating_add(raw_line_bytes);

        if let Some(t) = super::native::coverage::unproven_line_type(line) {
            return Err(format!("unproven log line type '{t}'"));
        }

        let is_begin_log = bench_kind_of(line) == Some("BEGIN_LOG");
        if is_begin_log {
            if seen_begin_log {
                return Err(
                    "native mid-fight experiment does not support multi-session logs".into(),
                );
            }
            seen_begin_log = true;
        }

        let _boundary = seg.feed(line);
        if seen_begin_log && segment_raw_bytes >= raw_byte_target && seg.shields_settled() {
            if let Some(mut payload) = seg.build_next_segment()? {
                payload.source_raw_bytes = Some(segment_raw_bytes);
                out.push(payload);
            }
            segment_raw_bytes = 0;
        }
        Ok(())
    })?;

    if !seen_begin_log {
        return Ok(None);
    }

    seg.drain_trailing_shields();
    if let Some(mut payload) = seg.build_next_segment()? {
        payload.source_raw_bytes = Some(segment_raw_bytes);
        out.push(payload);
    }

    if out.is_empty() {
        Ok(None)
    } else {
        Ok(Some(out))
    }
}

fn bench_kind_of(line: &str) -> Option<&str> {
    line.split(',').nth(1).map(str::trim)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RealUploadProof {
    schema: &'static str,
    schema_version: u8,
    measured_unix_ms: u64,
    report_url: String,
    input_bytes: u64,
    payloads: RealUploadPayloadProof,
    timings: RealUploadTimingProof,
    browser: Option<RealUploadBrowserProof>,
    baseline: Option<RealUploadBaselineProof>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProtocolCompareProof {
    schema: &'static str,
    schema_version: u8,
    measured_unix_ms: u64,
    input_bytes: u64,
    payloads: RealUploadPayloadProof,
    finished: ProtocolRunProof,
    live_style: ProtocolRunProof,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SegmentTargetSweepProof {
    schema: &'static str,
    schema_version: u8,
    measured_unix_ms: u64,
    input_bytes: u64,
    runs: Vec<SegmentTargetRunProof>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SegmentTargetRunProof {
    raw_target_mib: usize,
    report_url: String,
    payloads: RealUploadPayloadProof,
    timings: RealUploadTimingProof,
    browser: RealUploadBrowserProof,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MidfightLiveStyleSweepProof {
    schema: &'static str,
    schema_version: u8,
    measured_unix_ms: u64,
    input_bytes: u64,
    runs: Vec<MidfightLiveStyleRunProof>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MidfightLiveStyleRunProof {
    raw_target_mib: usize,
    report_url: String,
    payloads: RealUploadPayloadProof,
    in_progress_segments: usize,
    max_in_progress_event_count: u64,
    timings: RealUploadTimingProof,
    browser: RealUploadBrowserProof,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProtocolRunProof {
    report_url: String,
    timings: RealUploadTimingProof,
    browser: RealUploadBrowserProof,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RealUploadPayloadProof {
    segments_total: usize,
    requests_total: usize,
    segment_zip_bytes: usize,
    master_zip_bytes: usize,
    raw_target_bytes: usize,
    raw_segment_bytes: usize,
    max_raw_segment_bytes: usize,
    segments_over_raw_target: usize,
}

fn real_upload_payload_proof(
    payloads: &[super::native::live::LiveSegmentPayload],
    raw_target_bytes: usize,
) -> RealUploadPayloadProof {
    let raw_segment_bytes = payloads
        .iter()
        .filter_map(|payload| payload.source_raw_bytes)
        .sum();
    let max_raw_segment_bytes = payloads
        .iter()
        .filter_map(|payload| payload.source_raw_bytes)
        .max()
        .unwrap_or(0);
    let segments_over_raw_target = payloads
        .iter()
        .filter_map(|payload| payload.source_raw_bytes)
        .filter(|&bytes| bytes > raw_target_bytes)
        .count();

    RealUploadPayloadProof {
        segments_total: payloads.len(),
        requests_total: 2 + payloads.len().saturating_mul(2),
        segment_zip_bytes: payloads.iter().map(|p| p.segment.bytes.len()).sum(),
        master_zip_bytes: payloads.iter().map(|p| p.master.bytes.len()).sum(),
        raw_target_bytes,
        raw_segment_bytes,
        max_raw_segment_bytes,
        segments_over_raw_target,
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RealUploadTimingProof {
    routing_assess_ms: u64,
    local_map_ms: u64,
    local_build_ms: u64,
    session_ms: u64,
    create_report_ms: u64,
    set_master_table_ms: u64,
    add_segment_ms: u64,
    terminate_report_ms: u64,
    upload_total_ms: u64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RealUploadBrowserProof {
    ready_ms: u64,
    ready_source: String,
    browser_wall_ms: u64,
    native_fully_loaded_ms: u64,
    title: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RealUploadBaselineProof {
    baseline_fully_loaded_ms: u64,
    native_fully_loaded_ms: u64,
    improvement_ratio: f64,
    require_10x: bool,
    passed_10x: bool,
}

fn measure_report_ready(report_url: &str, full_started: Instant) -> RealUploadBrowserProof {
    measure_report_ready_with_offset(report_url, full_started, 0)
}

fn measure_report_ready_after_elapsed(
    report_url: &str,
    elapsed_before_browser_ms: u64,
) -> RealUploadBrowserProof {
    measure_report_ready_with_offset(report_url, Instant::now(), elapsed_before_browser_ms)
}

fn measure_report_ready_with_offset(
    report_url: &str,
    full_started: Instant,
    elapsed_before_browser_ms: u64,
) -> RealUploadBrowserProof {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("scripts")
        .join("measure-esologs-report-ready.mjs");
    let browser_started = Instant::now();
    let mut command = std::process::Command::new("node");
    command.arg(script).arg(report_url).arg("--json");
    if let Some(cookie) = bench_upload_cookie() {
        command.env("KALPA_BENCH_ESOLOGS_COOKIE", cookie);
    }
    let output = command
        .output()
        .expect("launch report-ready browser measurement");
    assert!(
        output.status.success(),
        "report-ready browser measurement failed with {}:\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let ready_json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("report-ready output must be JSON");
    let ready_ms = ready_json
        .get("readyMs")
        .and_then(|v| v.as_u64())
        .expect("report-ready JSON must include readyMs");
    let ready_source = ready_json
        .get("readySource")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    RealUploadBrowserProof {
        ready_ms,
        ready_source,
        browser_wall_ms: browser_started.elapsed().as_millis() as u64,
        native_fully_loaded_ms: elapsed_before_browser_ms.saturating_add(elapsed_ms(full_started)),
        title: ready_json
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
    }
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn write_real_upload_proof(report_code: &str, proof: &RealUploadProof) {
    let path = match bench_env("KALPA_BENCH_PROOF_JSON") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("native-upload-proof-{report_code}.json")),
    };
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).expect("create native upload proof directory");
    }
    let json = serde_json::to_string_pretty(proof).expect("serialize native upload proof");
    std::fs::write(&path, format!("{json}\n")).expect("write native upload proof JSON");
    eprintln!("  proof json   : {}", path.display());
}

fn maybe_report_real_upload_ratio(native_fully_loaded_ms: u64) -> Option<RealUploadBaselineProof> {
    let Some(baseline) = bench_env("KALPA_BENCH_BASELINE_FULLY_LOADED_MS") else {
        eprintln!(
            "  baseline     : set KALPA_BENCH_BASELINE_FULLY_LOADED_MS to compute the 10x ratio"
        );
        return None;
    };
    let baseline_ms: u64 = baseline
        .parse()
        .expect("KALPA_BENCH_BASELINE_FULLY_LOADED_MS must be an integer millisecond value");
    let ratio = baseline_ms as f64 / native_fully_loaded_ms.max(1) as f64;
    let require_10x = bench_env("KALPA_BENCH_REQUIRE_10X").as_deref() == Some("1");
    let comparison = RealUploadBaselineProof {
        baseline_fully_loaded_ms: baseline_ms,
        native_fully_loaded_ms,
        improvement_ratio: ratio,
        require_10x,
        passed_10x: ratio >= 10.0,
    };
    eprintln!("  baseline     : {baseline_ms} ms");
    eprintln!("  improvement  : {ratio:.2}x");
    if require_10x {
        assert!(
            comparison.passed_10x,
            "native fully-loaded time must improve by at least 10x \
             (baseline={baseline_ms} ms, native={native_fully_loaded_ms} ms, ratio={ratio:.2}x)"
        );
    }
    Some(comparison)
}

fn segment_target_mib_candidates() -> Result<Vec<usize>, String> {
    let raw =
        bench_env("KALPA_BENCH_SEGMENT_TARGETS_MIB").unwrap_or_else(|| "16,64,96,192".to_string());
    parse_segment_target_mib_list(&raw)
}

fn parse_segment_target_mib_list(raw: &str) -> Result<Vec<usize>, String> {
    let mut targets = Vec::new();
    for token in raw.split([',', ';', ' ']).map(str::trim) {
        if token.is_empty() {
            continue;
        }
        let mib = token
            .parse::<usize>()
            .map_err(|_| format!("invalid segment target MiB value '{token}'"))?;
        if !(1..=512).contains(&mib) {
            return Err(format!(
                "segment target MiB value {mib} is outside the allowed 1..=512 range"
            ));
        }
        if !targets.contains(&mib) {
            targets.push(mib);
        }
    }
    if targets.is_empty() {
        return Err("at least one segment target MiB value is required".into());
    }
    Ok(targets)
}

fn browser_ready_candidate_or_skip(log_path: &str, bench_name: &str) -> bool {
    let scan = match super::scanner::scan_file(log_path) {
        Ok(scan) => scan,
        Err(e) => {
            eprintln!("SKIP {bench_name}: failed to scan log before browser-ready proof: {e}");
            return false;
        }
    };
    if scan.fights.is_empty() {
        eprintln!(
            "SKIP {bench_name}: browser-ready proof needs a log with at least one completed \
             BEGIN_COMBAT..END_COMBAT fight; selected log has none"
        );
        return false;
    }
    true
}

fn validate_real_upload_10x_gate(
    require_10x: bool,
    browser_ready_enabled: bool,
    baseline_ms: Option<&str>,
) -> Result<(), String> {
    if !require_10x {
        return Ok(());
    }
    if !browser_ready_enabled {
        return Err("KALPA_BENCH_REQUIRE_10X=1 also requires \
             KALPA_BENCH_REPORT_READY_BROWSER=1 so the benchmark measures a fully loaded report"
            .into());
    }
    let Some(baseline_ms) = baseline_ms else {
        return Err("KALPA_BENCH_REQUIRE_10X=1 also requires \
             KALPA_BENCH_BASELINE_FULLY_LOADED_MS"
            .into());
    };
    match baseline_ms.parse::<u64>() {
        Ok(ms) if ms > 0 => Ok(()),
        _ => Err(
            "KALPA_BENCH_BASELINE_FULLY_LOADED_MS must be a positive integer millisecond value"
                .into(),
        ),
    }
}

#[test]
fn bench_env_file_parser_preserves_cookie_headers_and_windows_paths() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".env.bench.local");
    std::fs::write(
        &path,
        "KALPA_BENCH_NATIVE_UPLOAD_LOG=C:\\logs\\prepared.log\n\
         KALPA_BENCH_ESOLOGS_COOKIE=wcl_session=abc; XSRF-TOKEN=def%3D\n\
         export KALPA_BENCH_NATIVE_UPLOAD_CONFIRM=upload\n",
    )
    .unwrap();

    let values = load_bench_env_file(&path).unwrap();

    assert_eq!(
        values
            .get("KALPA_BENCH_NATIVE_UPLOAD_LOG")
            .map(String::as_str),
        Some("C:\\logs\\prepared.log")
    );
    assert_eq!(
        values.get("KALPA_BENCH_ESOLOGS_COOKIE").map(String::as_str),
        Some("wcl_session=abc; XSRF-TOKEN=def%3D")
    );
    assert_eq!(
        values
            .get("KALPA_BENCH_NATIVE_UPLOAD_CONFIRM")
            .map(String::as_str),
        Some("upload")
    );
}

#[test]
fn bench_env_file_candidates_include_linked_worktree_fallback() {
    let repo = tempfile::tempdir().unwrap();
    let main = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let explicit = repo.path().join("shared-bench.env");
    let paths = bench_env_file_candidates_from(
        repo.path(),
        Some(explicit.to_str().unwrap()),
        Some(main.path()),
        Some(cwd.path()),
    );

    assert_eq!(
        paths,
        vec![
            explicit,
            repo.path().join(".env.bench.local"),
            main.path().join(".env.bench.local"),
            cwd.path().join(".env.bench.local"),
        ]
    );
}

#[test]
fn bench_midfight_live_style_builder_can_cut_inside_open_fight() {
    let raw = include_str!("native/testdata/chunk1_raw.log");
    let payloads = build_midfight_live_style_payloads_from_replay_for_bench(&raw, 512)
        .expect("mid-fight experiment builder should not reject fixture")
        .expect("fixture should produce payloads");

    assert!(
        payloads.len() > 1,
        "tiny raw target should force multiple experimental live-style payloads"
    );
    assert!(
        payloads
            .iter()
            .any(|payload| payload.in_progress_event_count > 0),
        "at least one experimental segment should carry an unfinished fight tail"
    );
    assert!(
        payloads
            .last()
            .map(|payload| !payload.fight_durations_ms.is_empty())
            .unwrap_or(false),
        "final experimental segment should still close the completed fight"
    );
}

#[test]
fn real_upload_proof_json_records_10x_evidence_without_credentials() {
    let proof = RealUploadProof {
        schema: "kalpa.native-upload-proof",
        schema_version: 1,
        measured_unix_ms: 1_797_000_000_000,
        report_url: "https://www.esologs.com/reports/example".to_string(),
        input_bytes: 64 * 1024 * 1024,
        payloads: RealUploadPayloadProof {
            segments_total: 2,
            requests_total: 6,
            segment_zip_bytes: 1_234_567,
            master_zip_bytes: 45_678,
            raw_target_bytes: 16 * 1024 * 1024,
            raw_segment_bytes: 64 * 1024 * 1024,
            max_raw_segment_bytes: 33 * 1024 * 1024,
            segments_over_raw_target: 1,
        },
        timings: RealUploadTimingProof {
            routing_assess_ms: 20,
            local_map_ms: 4,
            local_build_ms: 800,
            session_ms: 100,
            create_report_ms: 200,
            set_master_table_ms: 300,
            add_segment_ms: 400,
            terminate_report_ms: 500,
            upload_total_ms: 1_500,
        },
        browser: Some(RealUploadBrowserProof {
            ready_ms: 2_000,
            ready_source: "encounter-list".to_string(),
            browser_wall_ms: 2_500,
            native_fully_loaded_ms: 4_000,
            title: "Report: example | ESO Logs".to_string(),
        }),
        baseline: Some(RealUploadBaselineProof {
            baseline_fully_loaded_ms: 45_000,
            native_fully_loaded_ms: 4_000,
            improvement_ratio: 11.25,
            require_10x: true,
            passed_10x: true,
        }),
    };

    let json = serde_json::to_value(&proof).expect("proof must serialize");
    assert_eq!(json["schema"], "kalpa.native-upload-proof");
    assert_eq!(json["payloads"]["segmentsTotal"], 2);
    assert_eq!(json["payloads"]["segments_total"], serde_json::Value::Null);
    assert_eq!(json["payloads"]["rawTargetBytes"], 16 * 1024 * 1024);
    assert_eq!(json["payloads"]["maxRawSegmentBytes"], 33 * 1024 * 1024);
    assert_eq!(json["payloads"]["segmentsOverRawTarget"], 1);
    assert_eq!(json["timings"]["routingAssessMs"], 20);
    assert_eq!(json["browser"]["nativeFullyLoadedMs"], 4_000);
    assert_eq!(json["baseline"]["passed10x"], true);
    assert!(json.get("cookie").is_none());
    assert!(json.get("session").is_none());
}

#[test]
fn real_upload_10x_gate_requires_browser_ready_and_baseline() {
    assert!(validate_real_upload_10x_gate(false, false, None).is_ok());
    assert!(validate_real_upload_10x_gate(true, false, Some("120000"))
        .unwrap_err()
        .contains("KALPA_BENCH_REPORT_READY_BROWSER"));
    assert!(validate_real_upload_10x_gate(true, true, None)
        .unwrap_err()
        .contains("KALPA_BENCH_BASELINE_FULLY_LOADED_MS"));
    assert!(validate_real_upload_10x_gate(true, true, Some("0")).is_err());
    assert!(validate_real_upload_10x_gate(true, true, Some("120000")).is_ok());
}

#[test]
fn segment_target_mib_parser_dedupes_and_rejects_bad_values() {
    assert_eq!(
        parse_segment_target_mib_list("8, 16;32 16").unwrap(),
        vec![8, 16, 32]
    );
    assert!(parse_segment_target_mib_list("").is_err());
    assert!(parse_segment_target_mib_list("0").is_err());
    assert!(parse_segment_target_mib_list("513").is_err());
    assert!(parse_segment_target_mib_list("eight").is_err());
}

#[test]
fn segment_target_sweep_proof_serializes_camelcase_without_credentials() {
    let proof = SegmentTargetSweepProof {
        schema: "kalpa.native-segment-target-sweep-proof",
        schema_version: 1,
        measured_unix_ms: 1_797_000_000_000,
        input_bytes: 64 * 1024 * 1024,
        runs: vec![SegmentTargetRunProof {
            raw_target_mib: 16,
            report_url: "https://www.esologs.com/reports/example".to_string(),
            payloads: RealUploadPayloadProof {
                segments_total: 4,
                requests_total: 10,
                segment_zip_bytes: 1_234_567,
                master_zip_bytes: 45_678,
                raw_target_bytes: 16 * 1024 * 1024,
                raw_segment_bytes: 64 * 1024 * 1024,
                max_raw_segment_bytes: 16 * 1024 * 1024,
                segments_over_raw_target: 0,
            },
            timings: RealUploadTimingProof {
                routing_assess_ms: 20,
                local_map_ms: 0,
                local_build_ms: 900,
                session_ms: 100,
                create_report_ms: 200,
                set_master_table_ms: 300,
                add_segment_ms: 400,
                terminate_report_ms: 500,
                upload_total_ms: 1_600,
            },
            browser: RealUploadBrowserProof {
                ready_ms: 2_000,
                ready_source: "encounter-list".to_string(),
                browser_wall_ms: 2_500,
                native_fully_loaded_ms: 4_520,
                title: "Report: example | ESO Logs".to_string(),
            },
        }],
    };

    let json = serde_json::to_value(&proof).expect("segment sweep proof must serialize");
    assert_eq!(json["schema"], "kalpa.native-segment-target-sweep-proof");
    assert_eq!(json["runs"][0]["rawTargetMib"], 16);
    assert_eq!(json["runs"][0]["raw_target_mib"], serde_json::Value::Null);
    assert_eq!(
        json["runs"][0]["payloads"]["segmentsOverRawTarget"],
        serde_json::json!(0)
    );
    assert_eq!(json["runs"][0]["browser"]["nativeFullyLoadedMs"], 4_520);
    assert!(json.get("cookie").is_none());
    assert!(json.get("session").is_none());
}

#[cfg(feature = "bench-alloc")]
fn synthetic_repeated_raw(target_bytes: usize) -> String {
    let fixture = include_str!("native/testdata/sample_raw_encounter.log");
    let mut raw = String::with_capacity(target_bytes + fixture.len());
    while raw.len() < target_bytes {
        raw.push_str(fixture);
        if !raw.ends_with('\n') {
            raw.push('\n');
        }
    }
    raw
}

#[cfg(feature = "bench-alloc")]
fn repeated_single_session_fixture(fixture: &str, target_bytes: usize) -> String {
    let begin = fixture
        .lines()
        .find(|line| line.split(',').nth(1).map(str::trim) == Some("BEGIN_LOG"))
        .expect("fixture must contain BEGIN_LOG");
    let body: Vec<&str> = fixture
        .lines()
        .filter(|line| {
            !matches!(
                line.split(',').nth(1).map(str::trim),
                Some("BEGIN_LOG") | Some("END_LOG")
            )
        })
        .collect();
    let max_ts = body
        .iter()
        .filter_map(|line| line.split(',').next()?.trim().parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    let step = max_ts + 10_000;

    let mut raw = String::with_capacity(target_bytes + fixture.len());
    raw.push_str(begin);
    raw.push('\n');
    let mut offset = 0u64;
    while raw.len() < target_bytes {
        for line in &body {
            push_line_with_ts_offset(&mut raw, line, offset);
            if raw.len() >= target_bytes {
                break;
            }
        }
        offset += step;
    }
    raw.push_str(&format!("{},END_LOG\n", offset + max_ts + 1));
    raw
}

#[cfg(feature = "bench-alloc")]
fn push_line_with_ts_offset(out: &mut String, line: &str, offset: u64) {
    let Some((ts, rest)) = line.split_once(',') else {
        out.push_str(line);
        out.push('\n');
        return;
    };
    let shifted = ts.trim().parse::<u64>().unwrap_or(0) + offset;
    out.push_str(&shifted.to_string());
    out.push(',');
    out.push_str(rest);
    out.push('\n');
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
    eprintln!("  (target for a 1 GB log: ~a few s; split is IO-bound + flat-memory)");

    // Clean up the (potentially multi-GB) split output eagerly; tempdir would too,
    // but a giant log's splits are large enough to want gone the moment we're done.
    drop(tmp);
}

/// Benchmark the native completed-upload encode (raw -> segment/master batches).
/// Production caps a single native upload at 256 MiB (`MAX_NATIVE_BYTES`) and
/// splits first, so this feeds the local corpus through the same file-backed
/// incremental builder the app uses. The raw input is streamed from disk rather
/// than mmap'd or copied into heap, so this tracks process-owned encode memory.
#[test]
#[ignore = "perf benchmark: needs .decode-samples/sunspire_raw.log; run --release"]
fn bench_encode_chunk() {
    let Some(log) = sample("sunspire_raw.log") else {
        eprintln!("SKIP bench_encode_chunk: .decode-samples/sunspire_raw.log not present");
        return;
    };

    let input_bytes = std::fs::metadata(&log).unwrap().len();
    let line_count = count_lines(&log);

    // Measure the full native encode region using the production file-backed path.
    crate::bench_alloc::reset_peak();
    let t = Instant::now();
    let payloads = super::native::live::build_finished_payloads_from_file(&log)
        .expect("encode should not error on a valid log")
        .expect("sample log must contain a valid native session");
    let dt = t.elapsed();
    #[cfg(feature = "bench-alloc")]
    let peak_bytes = crate::bench_alloc::peak();
    let peak = peak_heap_line();

    let seg_len: usize = payloads.iter().map(|p| p.segment.bytes.len()).sum();
    let master_len: usize = payloads.iter().map(|p| p.master.bytes.len()).sum();

    eprintln!("\n=== ENCODE (streamed raw -> segment/master batches) ===");
    eprintln!(
        "  input        : {:.1} MiB ({} lines)",
        mib(input_bytes),
        line_count
    );
    eprintln!("  payloads     : {}", payloads.len());
    eprintln!("  segment zips : {:.2} MiB", mib(seg_len as u64));
    eprintln!("  master zips  : {:.2} MiB", mib(master_len as u64));
    if let Some(max_raw) = payloads.iter().filter_map(|p| p.source_raw_bytes).max() {
        let over_target = payloads
            .iter()
            .filter_map(|p| p.source_raw_bytes)
            .filter(|&bytes| bytes > super::native::live::FINISHED_UPLOAD_SEGMENT_RAW_BYTE_TARGET)
            .count();
        eprintln!(
            "  raw chunks   : max {:.1} MiB, {} over {:.1} MiB target",
            mib(max_raw as u64),
            over_target,
            mib(super::native::live::FINISHED_UPLOAD_SEGMENT_RAW_BYTE_TARGET as u64)
        );
    }
    eprintln!("  wall time    : {:.2} s", dt.as_secs_f64());
    eprintln!("  throughput   : {:.0} MB/s", mb_per_s(input_bytes, dt));
    eprintln!("  peak heap    : {peak}");
    #[cfg(feature = "bench-alloc")]
    {
        let ratio = OLD_ENCODE_BASELINE_PEAK_BYTES as f64 / peak_bytes.max(1) as f64;
        eprintln!("  improvement  : {ratio:.1}x vs 462.7 MiB baseline");
        assert!(
            ratio >= 10.0,
            "completed upload encode should be at least 10x better than the old \
             462.7 MiB peak baseline (peak={peak_bytes}, ratio={ratio:.2}x)"
        );
        assert!(
            peak_bytes <= PEAK_TARGET_BYTES,
            "completed upload encode peak heap should stay under 50 MiB \
             (peak={peak_bytes}, target={PEAK_TARGET_BYTES})"
        );
    }
    eprintln!("  NOTE: raw input is streamed from disk; no mmap or full-log heap copy.");
}

/// Fast local slope benchmark for the production file-backed completed-upload
/// builder. Uses line-aligned prefixes of the real sample so a run finishes quickly
/// while still exercising real ESO log shapes, disk replay, segment batching,
/// master-table rendering, ZIP streaming, and the coverage/multi-session gates.
#[test]
#[ignore = "perf benchmark: needs .decode-samples/sunspire_raw.log; run --release"]
fn bench_finished_file_prefixes() {
    let Some(log) = sample("sunspire_raw.log") else {
        eprintln!(
            "SKIP bench_finished_file_prefixes: .decode-samples/sunspire_raw.log not present"
        );
        return;
    };

    eprintln!("\n=== FINISHED FILE PREFIXES (production file-backed builder) ===");
    eprintln!(
        "  target | actual MiB | lines | segments | max raw | over | wall s | MB/s | seg zip | master zip | peak MiB"
    );
    for target_mib in [16usize, 32, 64] {
        let prefix = copy_largest_session_prefix_line_aligned(&log, target_mib * 1024 * 1024);
        let actual_bytes = std::fs::metadata(prefix.path())
            .expect("prefix metadata")
            .len();
        let line_count = count_lines(prefix.path());

        crate::bench_alloc::reset_peak();
        let t = Instant::now();
        let payloads = super::native::live::build_finished_payloads_from_file(prefix.path())
            .expect("prefix encode should not error")
            .expect("prefix must contain a valid native session");
        let dt = t.elapsed();
        let peak = crate::bench_alloc::peak();
        let seg_len: usize = payloads.iter().map(|p| p.segment.bytes.len()).sum();
        let master_len: usize = payloads.iter().map(|p| p.master.bytes.len()).sum();
        let max_raw = payloads
            .iter()
            .filter_map(|p| p.source_raw_bytes)
            .max()
            .unwrap_or(0);
        let over_target = payloads
            .iter()
            .filter_map(|p| p.source_raw_bytes)
            .filter(|&bytes| bytes > super::native::live::FINISHED_UPLOAD_SEGMENT_RAW_BYTE_TARGET)
            .count();

        eprintln!(
            "  {:>6} | {:>10.1} | {:>5} | {:>8} | {:>7.1} | {:>4} | {:>6.2} | {:>4.0} | {:>7.2} | {:>10.2} | {:>8.1}",
            target_mib,
            mib(actual_bytes),
            line_count,
            payloads.len(),
            mib(max_raw as u64),
            over_target,
            dt.as_secs_f64(),
            mb_per_s(actual_bytes, dt),
            mib(seg_len as u64),
            mib(master_len as u64),
            mib(peak as u64)
        );

        #[cfg(feature = "bench-alloc")]
        assert!(
            peak <= PEAK_TARGET_BYTES,
            "prefix encode peak heap should stay under 50 MiB \
             (target_mib={target_mib}, peak={peak}, target={PEAK_TARGET_BYTES})"
        );
    }
}

/// Local corpus sweep for the completed-upload raw-byte guardrail. This uses
/// `KALPA_BENCH_NATIVE_UPLOAD_LOG` when set, otherwise the staged real sample, so
/// it is the best local signal for request count, local build time, and long-fight
/// target overruns while the final ESO Logs report-ready proof is gated on
/// credentials.
#[test]
#[ignore = "perf benchmark: needs .decode-samples/sunspire_raw.log; run --release"]
fn bench_finished_file_raw_targets() {
    let Some(upload_log) = bench_upload_log() else {
        eprintln!(
            "SKIP bench_finished_file_raw_targets: set KALPA_BENCH_NATIVE_UPLOAD_LOG \
             or stage .decode-samples/{BENCH_NATIVE_UPLOAD_SAMPLE}"
        );
        return;
    };
    let log = upload_log.path;

    let raw_targets_mib = bench_env("KALPA_BENCH_SEGMENT_TARGETS_MIB")
        .map(|raw| parse_segment_target_mib_list(&raw))
        .unwrap_or_else(|| Ok(vec![8, 16, 24, 32, 48, 64, 96]))
        .expect("raw target sweep env is invalid");
    let input_bytes = std::fs::metadata(&log).unwrap().len();

    eprintln!("\n=== FINISHED FILE RAW TARGET SWEEP (production builder) ===");
    eprintln!("  input        : {:.1} MiB", mib(input_bytes));
    eprintln!(
        "  raw MiB | segments | fights | in-prog | max fight | requests | max raw | over | wall s | MB/s | seg zip | master zip | peak MiB"
    );

    for &raw_target_mib in &raw_targets_mib {
        let raw_target_bytes = raw_target_mib
            .checked_mul(1024 * 1024)
            .expect("raw target MiB is too large");

        #[cfg(feature = "bench-alloc")]
        let baseline = crate::bench_alloc::current();
        crate::bench_alloc::reset_peak();
        let t = Instant::now();
        let payloads = super::native::live::build_finished_payloads_from_file_limited(
            &log,
            super::native::live::FINISHED_UPLOAD_FIGHTS_PER_SEGMENT,
            raw_target_bytes,
        )
        .expect("sample encode should not error")
        .expect("sample must contain a valid native session");
        let dt = t.elapsed();
        #[cfg(feature = "bench-alloc")]
        let peak = peak_delta_from(baseline);
        #[cfg(not(feature = "bench-alloc"))]
        let peak = crate::bench_alloc::peak();

        let segments = payloads.len();
        let completed_fights: usize = payloads
            .iter()
            .map(|payload| payload.fight_durations_ms.len())
            .sum();
        let max_fight_ms = payloads
            .iter()
            .flat_map(|payload| payload.fight_durations_ms.iter().copied())
            .max()
            .unwrap_or(0);
        let in_progress_segments = payloads
            .iter()
            .filter(|payload| payload.in_progress_event_count > 0)
            .count();
        let requests = 2 + segments * 2;
        let seg_len: usize = payloads.iter().map(|p| p.segment.bytes.len()).sum();
        let master_len: usize = payloads.iter().map(|p| p.master.bytes.len()).sum();
        let max_raw = payloads
            .iter()
            .filter_map(|p| p.source_raw_bytes)
            .max()
            .unwrap_or(0);
        let over_target = payloads
            .iter()
            .filter_map(|p| p.source_raw_bytes)
            .filter(|&bytes| bytes > raw_target_bytes)
            .count();

        eprintln!(
            "  {:>7} | {:>8} | {:>6} | {:>7} | {:>8.1} | {:>8} | {:>7.1} | {:>4} | {:>6.2} | {:>4.0} | {:>7.2} | {:>10.2} | {:>8.1}",
            raw_target_mib,
            segments,
            completed_fights,
            in_progress_segments,
            minutes(max_fight_ms),
            requests,
            mib(max_raw as u64),
            over_target,
            dt.as_secs_f64(),
            mb_per_s(input_bytes, dt),
            mib(seg_len as u64),
            mib(master_len as u64),
            mib(peak as u64)
        );

        assert!(
            segments > 0,
            "raw target {raw_target_mib} MiB should produce payloads"
        );
        #[cfg(feature = "bench-alloc")]
        assert!(
            peak <= PEAK_TARGET_BYTES,
            "sample target sweep peak heap should stay under 50 MiB \
             (raw_target_mib={raw_target_mib}, peak={peak}, target={PEAK_TARGET_BYTES})"
        );
    }
}

/// Local-only companion to the real mid-fight live-style sweep. It does not upload
/// anything; it just shows whether the experimental builder can turn a single long
/// fight into bounded live-style fragments with acceptable local build cost.
#[test]
#[ignore = "experimental perf benchmark: needs .decode-samples/sunspire_raw.log; run --release"]
fn bench_finished_file_midfight_live_style_raw_targets() {
    let Some(log) = sample("sunspire_raw.log") else {
        eprintln!(
            "SKIP bench_finished_file_midfight_live_style_raw_targets: \
             .decode-samples/sunspire_raw.log not present"
        );
        return;
    };

    const RAW_TARGETS_MIB: &[usize] = &[8, 16, 24, 32, 48, 64, 96];
    let input_bytes = std::fs::metadata(&log).unwrap().len();

    eprintln!("\n=== FINISHED FILE MID-FIGHT LIVE-STYLE SWEEP (experimental builder) ===");
    eprintln!("  input        : {:.1} MiB", mib(input_bytes));
    eprintln!(
        "  raw MiB | segments | in-prog | requests | max raw | wall s | MB/s | seg zip | master zip | peak MiB"
    );

    for &raw_target_mib in RAW_TARGETS_MIB {
        let raw_target_bytes = raw_target_mib
            .checked_mul(1024 * 1024)
            .expect("raw target MiB is too large");

        #[cfg(feature = "bench-alloc")]
        let baseline = crate::bench_alloc::current();
        crate::bench_alloc::reset_peak();
        let t = Instant::now();
        let payloads =
            build_midfight_live_style_payloads_from_file_for_bench(&log, raw_target_bytes)
                .expect("sample mid-fight encode should not error")
                .expect("sample must contain a valid native session");
        let dt = t.elapsed();
        #[cfg(feature = "bench-alloc")]
        let peak = peak_delta_from(baseline);
        #[cfg(not(feature = "bench-alloc"))]
        let peak = crate::bench_alloc::peak();

        let segments = payloads.len();
        let in_progress_segments = payloads
            .iter()
            .filter(|payload| payload.in_progress_event_count > 0)
            .count();
        let requests = 2 + segments * 2;
        let seg_len: usize = payloads.iter().map(|p| p.segment.bytes.len()).sum();
        let master_len: usize = payloads.iter().map(|p| p.master.bytes.len()).sum();
        let max_raw = payloads
            .iter()
            .filter_map(|p| p.source_raw_bytes)
            .max()
            .unwrap_or(0);

        eprintln!(
            "  {:>7} | {:>8} | {:>7} | {:>8} | {:>7.1} | {:>6.2} | {:>4.0} | {:>7.2} | {:>10.2} | {:>8.1}",
            raw_target_mib,
            segments,
            in_progress_segments,
            requests,
            mib(max_raw as u64),
            dt.as_secs_f64(),
            mb_per_s(input_bytes, dt),
            mib(seg_len as u64),
            mib(master_len as u64),
            mib(peak as u64)
        );

        assert!(
            segments > 1,
            "mid-fight target {raw_target_mib} MiB should split the long sample session"
        );
        #[cfg(feature = "bench-alloc")]
        assert!(
            peak <= PEAK_TARGET_BYTES,
            "mid-fight sample sweep peak heap should stay under 50 MiB \
             (raw_target_mib={raw_target_mib}, peak={peak}, target={PEAK_TARGET_BYTES})"
        );
    }
}

/// Local proof benchmark that needs no private corpus. It compares the old
/// slice-backed payload builder (line vec + rendered segment/master text) with the
/// production text-backed streaming builder on the same synthetic raw log.
#[test]
#[ignore = "perf benchmark: run with --features bench-alloc --nocapture"]
fn bench_encode_legacy_vs_streaming_synthetic() {
    #[cfg(not(feature = "bench-alloc"))]
    {
        eprintln!("SKIP bench_encode_legacy_vs_streaming_synthetic: enable --features bench-alloc");
    }

    #[cfg(feature = "bench-alloc")]
    {
        const TARGET: usize = 12 * 1024 * 1024;
        let raw = synthetic_repeated_raw(TARGET);
        let raw_bytes = raw.len() as u64;

        let baseline = crate::bench_alloc::current();
        crate::bench_alloc::reset_peak();
        let t = Instant::now();
        let lines: Vec<&str> = raw.lines().collect();
        let legacy = super::native::events::build_native_payload(&lines)
            .expect("legacy encode should not error")
            .expect("synthetic fixture should contain a valid session");
        let legacy_dt = t.elapsed();
        let legacy_peak = peak_delta_from(baseline);
        drop(lines);

        let baseline = crate::bench_alloc::current();
        crate::bench_alloc::reset_peak();
        let t = Instant::now();
        let streaming = super::native::events::build_native_payload_from_text(&raw)
            .expect("streaming encode should not error")
            .expect("synthetic fixture should contain a valid session");
        let streaming_dt = t.elapsed();
        let streaming_peak = peak_delta_from(baseline);

        assert_eq!(
            streaming.0.bytes, legacy.0.bytes,
            "streaming segment ZIP must match legacy output"
        );
        assert_eq!(
            streaming.1.bytes, legacy.1.bytes,
            "streaming master ZIP must match legacy output"
        );

        let ratio = legacy_peak as f64 / streaming_peak.max(1) as f64;
        eprintln!("\n=== ENCODE MEMORY (legacy vs streaming synthetic) ===");
        eprintln!("  input        : {:.1} MiB", mib(raw_bytes));
        eprintln!(
            "  legacy peak  : {:.1} MiB ({:.2} s)",
            mib(legacy_peak as u64),
            legacy_dt.as_secs_f64()
        );
        eprintln!(
            "  stream peak  : {:.1} MiB ({:.2} s)",
            mib(streaming_peak as u64),
            streaming_dt.as_secs_f64()
        );
        eprintln!("  improvement  : {ratio:.1}x");
        assert!(
            ratio >= 10.0,
            "streaming path peak heap should improve by at least 10x on synthetic log \
             (legacy={legacy_peak}, streaming={streaming_peak}, ratio={ratio:.2}x)"
        );
    }
}

/// Compares the completed-file one-shot encoder with the finished-log live
/// segmenter on a single-session combat workload. The fixture repeats a real raw
/// fight with shifted timestamps, so the live path cuts at realistic END_COMBAT
/// boundaries without the pathological tiny-segment rate in
/// `bench_live_logging_synthetic_peak`.
#[test]
#[ignore = "perf benchmark: run with --features bench-alloc --nocapture"]
fn bench_finished_live_vs_one_shot_synthetic() {
    #[cfg(not(feature = "bench-alloc"))]
    {
        eprintln!("SKIP bench_finished_live_vs_one_shot_synthetic: enable --features bench-alloc");
    }

    #[cfg(feature = "bench-alloc")]
    {
        const TARGET: usize = 50 * 1024 * 1024;
        let raw =
            repeated_single_session_fixture(include_str!("native/testdata/chunk1_raw.log"), TARGET);
        let raw_bytes = raw.len() as u64;

        let baseline = crate::bench_alloc::current();
        crate::bench_alloc::reset_peak();
        let t = Instant::now();
        let one_shot = super::native::events::build_native_payload_from_text(&raw)
            .expect("one-shot encode should not error")
            .expect("synthetic fixture should contain a valid session");
        let one_shot_dt = t.elapsed();
        let one_shot_peak = peak_delta_from(baseline);
        let one_shot_seg = one_shot.0.bytes.len();
        let one_shot_master = one_shot.1.bytes.len();
        drop(one_shot);

        let baseline = crate::bench_alloc::current();
        crate::bench_alloc::reset_peak();
        let t = Instant::now();
        let live = super::native::live::build_finished_payloads_from_text(&raw)
            .expect("finished live encode should not error")
            .expect("synthetic fixture should contain a valid session");
        let live_dt = t.elapsed();
        let live_peak = peak_delta_from(baseline);
        let live_segments = live.len();
        let live_seg_bytes: usize = live.iter().map(|p| p.segment.bytes.len()).sum();
        let live_master_bytes: usize = live.iter().map(|p| p.master.bytes.len()).sum();

        eprintln!("\n=== FINISHED ENCODE (one-shot vs live segmentation synthetic) ===");
        eprintln!("  input        : {:.1} MiB", mib(raw_bytes));
        eprintln!(
            "  one-shot     : {:.2} s, {:.0} MB/s, peak {:.1} MiB, zips {:.2}+{:.2} MiB",
            one_shot_dt.as_secs_f64(),
            mb_per_s(raw_bytes, one_shot_dt),
            mib(one_shot_peak as u64),
            mib(one_shot_seg as u64),
            mib(one_shot_master as u64)
        );
        eprintln!(
            "  live batches : {:.2} s, {:.0} MB/s, peak {:.1} MiB, {} segments, zips {:.2}+{:.2} MiB",
            live_dt.as_secs_f64(),
            mb_per_s(raw_bytes, live_dt),
            mib(live_peak as u64),
            live_segments,
            mib(live_seg_bytes as u64),
            mib(live_master_bytes as u64)
        );

        assert!(
            live_segments > 1,
            "finished-live benchmark should build multiple fight segments"
        );
        assert!(
            live_peak <= PEAK_TARGET_BYTES,
            "finished live encode peak heap should stay under 50 MiB \
             (peak={live_peak}, target={PEAK_TARGET_BYTES})"
        );
    }
}

/// Sweeps completed-upload batch sizes on the same repeated combat fixture. This
/// tracks the local build cost and, more importantly for full-load latency, how
/// many master-table/segment HTTP request pairs a finished native upload would
/// make for the workload.
#[test]
#[ignore = "perf benchmark: run with --features bench-alloc --nocapture"]
fn bench_finished_batch_sizes_synthetic() {
    #[cfg(not(feature = "bench-alloc"))]
    {
        eprintln!("SKIP bench_finished_batch_sizes_synthetic: enable --features bench-alloc");
    }

    #[cfg(feature = "bench-alloc")]
    {
        const TARGET: usize = 50 * 1024 * 1024;
        const BATCHES: &[usize] = &[1, 4, 8, 16, 32, 64, 128, 256];
        let raw =
            repeated_single_session_fixture(include_str!("native/testdata/chunk1_raw.log"), TARGET);
        let raw_bytes = raw.len() as u64;

        eprintln!("\n=== FINISHED BATCH SIZE SWEEP (synthetic) ===");
        eprintln!("  input        : {:.1} MiB", mib(raw_bytes));
        eprintln!(
            "  batch | segments | requests | wall s | MB/s | seg zip | master zip | peak MiB"
        );

        for &batch in BATCHES {
            let baseline = crate::bench_alloc::current();
            crate::bench_alloc::reset_peak();
            let t = Instant::now();
            let payloads =
                super::native::live::build_finished_payloads_from_text_batched(&raw, batch)
                    .expect("finished live encode should not error")
                    .expect("synthetic fixture should contain a valid session");
            let dt = t.elapsed();
            let peak = peak_delta_from(baseline);
            let segments = payloads.len();
            let requests = segments * 2;
            let seg_bytes: usize = payloads.iter().map(|p| p.segment.bytes.len()).sum();
            let master_bytes: usize = payloads.iter().map(|p| p.master.bytes.len()).sum();
            eprintln!(
                "  {:>5} | {:>8} | {:>8} | {:>6.2} | {:>4.0} | {:>7.2} | {:>10.2} | {:>8.1}",
                batch,
                segments,
                requests,
                dt.as_secs_f64(),
                mb_per_s(raw_bytes, dt),
                mib(seg_bytes as u64),
                mib(master_bytes as u64),
                mib(peak as u64)
            );
            assert!(segments > 0, "batch {batch} should produce payloads");
            assert!(
                peak <= PEAK_TARGET_BYTES,
                "finished live encode peak heap should stay under 50 MiB \
                 (batch={batch}, peak={peak}, target={PEAK_TARGET_BYTES})"
            );
        }
    }
}

/// Sweeps the raw-byte guardrail for completed uploads with the fight cap held high.
/// This answers a different question from `bench_finished_batch_sizes_synthetic`:
/// whether a larger server chunk target can collapse request count further without
/// blowing up local heap.
#[test]
#[ignore = "perf benchmark: run with --features bench-alloc --nocapture"]
fn bench_finished_raw_targets_synthetic() {
    #[cfg(not(feature = "bench-alloc"))]
    {
        eprintln!("SKIP bench_finished_raw_targets_synthetic: enable --features bench-alloc");
    }

    #[cfg(feature = "bench-alloc")]
    {
        const TARGET: usize = 64 * 1024 * 1024;
        const RAW_TARGETS_MIB: &[usize] = &[8, 16, 24, 32, 48, 64, 96];
        let raw =
            repeated_single_session_fixture(include_str!("native/testdata/chunk1_raw.log"), TARGET);
        let raw_bytes = raw.len() as u64;

        eprintln!("\n=== FINISHED RAW TARGET SWEEP (synthetic) ===");
        eprintln!("  input        : {:.1} MiB", mib(raw_bytes));
        eprintln!(
            "  raw MiB | segments | requests | wall s | MB/s | seg zip | master zip | peak MiB"
        );

        for &raw_target_mib in RAW_TARGETS_MIB {
            let baseline = crate::bench_alloc::current();
            crate::bench_alloc::reset_peak();
            let t = Instant::now();
            let payloads = super::native::live::build_finished_payloads_from_text_limited(
                &raw,
                usize::MAX,
                raw_target_mib * 1024 * 1024,
            )
            .expect("finished live encode should not error")
            .expect("synthetic fixture should contain a valid session");
            let dt = t.elapsed();
            let peak = peak_delta_from(baseline);
            let segments = payloads.len();
            let requests = segments * 2;
            let seg_bytes: usize = payloads.iter().map(|p| p.segment.bytes.len()).sum();
            let master_bytes: usize = payloads.iter().map(|p| p.master.bytes.len()).sum();
            eprintln!(
                "  {:>7} | {:>8} | {:>8} | {:>6.2} | {:>4.0} | {:>7.2} | {:>10.2} | {:>8.1}",
                raw_target_mib,
                segments,
                requests,
                dt.as_secs_f64(),
                mb_per_s(raw_bytes, dt),
                mib(seg_bytes as u64),
                mib(master_bytes as u64),
                mib(peak as u64)
            );
            assert!(
                segments > 0,
                "raw target {raw_target_mib} MiB should produce payloads"
            );
        }
    }
}

/// Larger version of the raw-target sweep, used to sanity-check peak heap when the
/// candidate 64 MiB guardrail actually fills multiple large completed-upload
/// segments. Kept separate so the default sweep stays quick.
#[test]
#[ignore = "perf benchmark: run with --features bench-alloc --nocapture"]
fn bench_finished_raw_targets_large_synthetic() {
    #[cfg(not(feature = "bench-alloc"))]
    {
        eprintln!("SKIP bench_finished_raw_targets_large_synthetic: enable --features bench-alloc");
    }

    #[cfg(feature = "bench-alloc")]
    {
        const TARGET: usize = 128 * 1024 * 1024;
        const RAW_TARGETS_MIB: &[usize] = &[32, 64];
        let raw =
            repeated_single_session_fixture(include_str!("native/testdata/chunk1_raw.log"), TARGET);
        let raw_bytes = raw.len() as u64;

        eprintln!("\n=== FINISHED RAW TARGET LARGE SWEEP (synthetic) ===");
        eprintln!("  input        : {:.1} MiB", mib(raw_bytes));
        eprintln!(
            "  raw MiB | segments | requests | wall s | MB/s | seg zip | master zip | peak MiB"
        );

        for &raw_target_mib in RAW_TARGETS_MIB {
            let baseline = crate::bench_alloc::current();
            crate::bench_alloc::reset_peak();
            let t = Instant::now();
            let payloads = super::native::live::build_finished_payloads_from_text_limited(
                &raw,
                usize::MAX,
                raw_target_mib * 1024 * 1024,
            )
            .expect("finished live encode should not error")
            .expect("synthetic fixture should contain a valid session");
            let dt = t.elapsed();
            let peak = peak_delta_from(baseline);
            let segments = payloads.len();
            let requests = segments * 2;
            let seg_bytes: usize = payloads.iter().map(|p| p.segment.bytes.len()).sum();
            let master_bytes: usize = payloads.iter().map(|p| p.master.bytes.len()).sum();
            eprintln!(
                "  {:>7} | {:>8} | {:>8} | {:>6.2} | {:>4.0} | {:>7.2} | {:>10.2} | {:>8.1}",
                raw_target_mib,
                segments,
                requests,
                dt.as_secs_f64(),
                mb_per_s(raw_bytes, dt),
                mib(seg_bytes as u64),
                mib(master_bytes as u64),
                mib(peak as u64)
            );
            assert!(
                segments > 0,
                "raw target {raw_target_mib} MiB should produce payloads"
            );
        }
    }
}

/// Measures the live segmenter path with repeated live-correlation fixtures. This
/// simulates the driver behavior that builds and drops payloads at safe cut
/// boundaries, so the measured peak is the live logging steady-state heap rather
/// than an accumulated list of prior uploads.
#[test]
#[ignore = "perf benchmark: run with --features bench-alloc --nocapture"]
fn bench_live_logging_synthetic_peak() {
    #[cfg(not(feature = "bench-alloc"))]
    {
        eprintln!("SKIP bench_live_logging_synthetic_peak: enable --features bench-alloc");
    }

    #[cfg(feature = "bench-alloc")]
    {
        const TARGET: usize = 50 * 1024 * 1024;
        let fixture = include_str!("native/testdata/live_correlation_synthetic.log");

        let baseline = crate::bench_alloc::current();
        crate::bench_alloc::reset_peak();
        let t = Instant::now();
        let mut seg = super::native::live::LiveSegmenter::new();
        let mut payloads = 0usize;
        let mut segment_zip_bytes = 0usize;
        let mut master_zip_bytes = 0usize;
        let mut raw_bytes = 0u64;

        while raw_bytes < TARGET as u64 {
            for line in fixture.lines() {
                raw_bytes += line.len() as u64 + 1;
                if seg.feed(line) && seg.shields_settled() {
                    if let Some(payload) = seg.build_next_segment().expect("live segment builds") {
                        payloads += 1;
                        segment_zip_bytes += payload.segment.bytes.len();
                        master_zip_bytes += payload.master.bytes.len();
                    }
                }
                if raw_bytes >= TARGET as u64 {
                    break;
                }
            }
        }
        seg.drain_trailing_shields();
        if let Some(payload) = seg.build_next_segment().expect("final live segment builds") {
            payloads += 1;
            segment_zip_bytes += payload.segment.bytes.len();
            master_zip_bytes += payload.master.bytes.len();
        }

        let dt = t.elapsed();
        let peak = peak_delta_from(baseline);
        eprintln!("\n=== LIVE LOGGING MEMORY (synthetic) ===");
        eprintln!("  input        : {:.1} MiB", mib(raw_bytes));
        eprintln!("  payloads     : {payloads}");
        eprintln!("  segment zips : {:.2} MiB", mib(segment_zip_bytes as u64));
        eprintln!("  master zips  : {:.2} MiB", mib(master_zip_bytes as u64));
        eprintln!("  wall time    : {:.2} s", dt.as_secs_f64());
        eprintln!("  throughput   : {:.0} MB/s", mb_per_s(raw_bytes, dt));
        eprintln!("  peak heap    : {:.1} MiB", mib(peak as u64));

        assert!(
            payloads > 0,
            "live benchmark must build at least one payload"
        );
        assert!(
            peak <= PEAK_TARGET_BYTES,
            "live logging peak heap should stay under 50 MiB \
             (peak={peak}, target={PEAK_TARGET_BYTES})"
        );
    }
}
