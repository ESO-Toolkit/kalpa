//! A minimal file log sink, installed before the Tauri builder runs.
//!
//! Release builds are `windows_subsystem = "windows"`, so every `eprintln!` in
//! this app is discarded — and so is every `log::` record its dependencies
//! emit. That second half matters more than it sounds: `tauri-runtime-wry`
//! reports a failed window creation with a bare `log::error!` and then hands
//! the caller an `Ok(DetachedWindow { .. })` anyway, so a window that never
//! came into existence leaves no trace whatsoever. This module gives those
//! records somewhere to land.
//!
//! Deliberately small: append-only, size-capped, no rotation policy, no config.
//! It exists so the next "Kalpa is in my tray but it won't open" report arrives
//! with evidence attached instead of a guess.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const LOG_FILE_NAME: &str = "kalpa.log";

/// A launch writes a handful of lines, so this keeps a long history while
/// staying small enough to paste into a bug report.
const MAX_BYTES: u64 = 512 * 1024;

struct FileLogger {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl FileLogger {
    fn append(&self, line: &str) {
        // Two threads appending concurrently would interleave mid-line.
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Cheap size cap: start over rather than grow without bound. Losing old
        // lines is fine — the interesting ones are always the most recent.
        if fs::metadata(&self.path)
            .map(|meta| meta.len() > MAX_BYTES)
            .unwrap_or(false)
        {
            let _ = fs::remove_file(&self.path);
        }
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        // Warnings and errors from anywhere — that is how a swallowed
        // `log::error!` inside tauri/wry reaches disk — plus this crate's own
        // deliberate breadcrumbs at any level.
        metadata.level() <= log::Level::Warn || metadata.target().starts_with("kalpa")
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        self.append(&format!(
            "{} {:5} [{}] {}\n",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
            record.level(),
            record.target(),
            record.args()
        ));
    }

    fn flush(&self) {}
}

/// Resolve `<app data dir>/kalpa.log` without an `AppHandle`.
///
/// `dirs::data_dir()` is the same base Tauri resolves `app_data_dir()` from, so
/// this lands beside `settings.json` on every platform.
fn log_path() -> Option<PathBuf> {
    let dir = dirs::data_dir()?.join("com.kalpa.desktop");
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join(LOG_FILE_NAME))
}

/// Install the sink.
///
/// Call this as the first statement of `run()`. A sink installed any later — a
/// Tauri plugin's `setup`, or the app's own `.setup()` closure — misses window
/// creation, which is the single event most worth capturing.
pub fn install() {
    let Some(path) = log_path() else {
        return;
    };
    let logger = FileLogger {
        path,
        write_lock: Mutex::new(()),
    };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
        log::info!(
            "kalpa {} starting (pid {})",
            env!("CARGO_PKG_VERSION"),
            std::process::id()
        );
    }
}
