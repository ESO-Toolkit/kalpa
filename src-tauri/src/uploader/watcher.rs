//! Live folder/file watcher for streaming uploads during a raid.
//!
//! Tails the active `Encounter.log` by byte offset (the game only appends), so
//! it never re-reads the whole file. New bytes are scanned for completed fights
//! (`BEGIN_COMBAT`…`END_COMBAT`); each completed fight is emitted to the
//! frontend via a Tauri [`Channel`] and queued for the transport.
//!
//! Watching is built on `notify` with a short polling fallback because Windows
//! `ReadDirectoryChangesW` modify events for a single appended file can coalesce
//! or be missed under heavy raid write volume.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::ipc::Channel;

use super::scanner;

/// 64 MiB cap on a single incremental read, guarding against a corrupt size.
const MAX_READ: u64 = 64 * 1024 * 1024;
/// Poll fallback cadence — short enough to feel live, light on IO.
const POLL_INTERVAL: Duration = Duration::from_millis(400);

/// Events streamed to the frontend over the live-session [`Channel`].
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LiveEvent {
    /// Watching started; reports the file and starting byte offset.
    Started { file: String, start_offset: u64 },
    /// A fight finished and is queued for upload.
    FightDetected {
        index: usize,
        zone_name: Option<String>,
        boss_name: Option<String>,
        duration_ms: u64,
    },
    /// A fight's upload changed state.
    FightStatus { index: usize, status: String },
    /// The report code became known (surface the link immediately).
    Report { code: String, url: String },
    /// The log was truncated or a new session began.
    SessionReset,
    /// A non-fatal warning (e.g. transient read failure, retrying).
    Warning { message: String },
    /// Watching stopped (user-initiated or fatal error).
    Stopped { reason: String },
}

/// Handle controlling a running live watcher; dropping it stops the thread.
pub struct LiveWatchHandle {
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl LiveWatchHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Ok(mut guard) = self.thread.lock() {
            if let Some(handle) = guard.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for LiveWatchHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Read `[start, end)` from a file into a UTF-8 string, bounded by [`MAX_READ`].
fn read_range(path: &Path, start: u64, end: u64) -> Result<String, String> {
    use std::io::{Read, Seek, SeekFrom};
    if end <= start {
        return Ok(String::new());
    }
    let len = end - start;
    if len > MAX_READ {
        return Err(format!("incremental read too large: {len} bytes"));
    }
    let mut f = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    f.seek(SeekFrom::Start(start))
        .map_err(|e| format!("seek: {e}"))?;
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf).map_err(|e| format!("read: {e}"))?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Callback the watcher invokes for each completed fight: given the byte range
/// in the source file, the implementor extracts + uploads it. Returns an
/// optional report code once known.
pub type OnFight = Box<dyn Fn(FightRange) -> Result<Option<String>, String> + Send + 'static>;

/// A completed fight located in the source file, ready to extract & upload.
#[derive(Debug, Clone)]
pub struct FightRange {
    pub index: usize,
    pub start_offset: u64,
    pub end_offset: u64,
    pub zone_name: Option<String>,
    pub boss_name: Option<String>,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Start tailing `file_path`. `start_offset` is where to begin (0 to include
/// the whole existing file, or its current length to only catch new fights).
///
/// For each completed fight, `on_fight` is called (off the event-detection hot
/// path is not required — fights are infrequent) and `channel` receives a
/// [`LiveEvent`]. Returns a handle to stop watching.
pub fn start_live_watch(
    file_path: &str,
    start_offset: u64,
    channel: Channel<LiveEvent>,
    on_fight: OnFight,
) -> Result<LiveWatchHandle, String> {
    let path = PathBuf::from(file_path);
    if !path.is_file() {
        return Err(format!("File does not exist: {file_path}"));
    }

    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let file_string = file_path.to_string();

    let _ = channel.send(LiveEvent::Started {
        file: file_string.clone(),
        start_offset,
    });

    let thread = thread::spawn(move || {
        tail_loop(path, start_offset, stop_thread, channel, on_fight);
    });

    Ok(LiveWatchHandle {
        stop,
        thread: Mutex::new(Some(thread)),
    })
}

/// The tailing loop: wait for change → read new bytes → scan for completed
/// fights in the accumulated buffer → emit & dispatch each.
fn tail_loop(
    path: PathBuf,
    start_offset: u64,
    stop: Arc<AtomicBool>,
    channel: Channel<LiveEvent>,
    on_fight: OnFight,
) {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = match RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        Config::default().with_poll_interval(POLL_INTERVAL),
    ) {
        Ok(w) => w,
        Err(e) => {
            let _ = channel.send(LiveEvent::Stopped {
                reason: format!("Could not start file watcher: {e}"),
            });
            return;
        }
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = watcher.watch(parent, RecursiveMode::NonRecursive) {
            let _ = channel.send(LiveEvent::Stopped {
                reason: format!("Could not watch the logs folder: {e}"),
            });
            return;
        }
    }

    // `consumed` is the byte offset of everything we've fully turned into fights.
    // We re-scan from `consumed` each time so a fight split across reads is found
    // once its END_COMBAT arrives. Completed fights advance `consumed` past them.
    let mut consumed = start_offset;
    let mut next_index = 0usize;
    let mut last_poll = Instant::now();

    loop {
        if stop.load(Ordering::SeqCst) {
            let _ = channel.send(LiveEvent::Stopped {
                reason: "Live logging stopped.".into(),
            });
            return;
        }

        // Block briefly for an FS event, else fall through to poll.
        match rx.recv_timeout(POLL_INTERVAL) {
            Ok(Ok(ev)) => {
                if !matches!(ev.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                    continue;
                }
                if !ev.paths.iter().any(|p| p == &path) {
                    continue;
                }
            }
            Ok(Err(_)) => continue,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last_poll.elapsed() < POLL_INTERVAL {
                    continue;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        last_poll = Instant::now();

        let size = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(_) => continue,
        };

        // Truncation / new session detection.
        if size < consumed {
            let _ = channel.send(LiveEvent::SessionReset);
            consumed = 0;
            next_index = 0;
            continue;
        }
        if size == consumed {
            continue;
        }

        // Cap each pass to MAX_READ; a very large catch-up will progress over
        // multiple iterations.
        let read_end = size.min(consumed + MAX_READ);
        let chunk = match read_range(&path, consumed, read_end) {
            Ok(c) => c,
            Err(e) => {
                // Transient (e.g. sharing violation while ESO flushes) — retry.
                let _ = channel.send(LiveEvent::Warning {
                    message: format!("Read retry: {e}"),
                });
                continue;
            }
        };

        // Find completed fights within the chunk and dispatch each. We compute
        // absolute offsets by adding `consumed` to the chunk-relative offsets.
        // `next_index` is the index of the next *successfully uploaded* fight.
        // Re-scanning from `consumed` after a failure re-surfaces the same
        // fight with the same index, so a retry updates the existing UI row
        // rather than appending a duplicate.
        let completed = scan_chunk_fights(&chunk, consumed);
        let mut advanced_to = consumed;
        for fr in completed {
            let mut fr = fr;
            fr.index = next_index;

            let _ = channel.send(LiveEvent::FightDetected {
                index: fr.index,
                zone_name: fr.zone_name.clone(),
                boss_name: fr.boss_name.clone(),
                duration_ms: fr.end_ms.saturating_sub(fr.start_ms),
            });
            let _ = channel.send(LiveEvent::FightStatus {
                index: fr.index,
                status: "uploading".into(),
            });

            let end_offset = fr.end_offset;
            match on_fight(fr.clone()) {
                Ok(report_code) => {
                    let _ = channel.send(LiveEvent::FightStatus {
                        index: fr.index,
                        status: "uploaded".into(),
                    });
                    if let Some(code) = report_code {
                        let _ = channel.send(LiveEvent::Report {
                            url: report_url(&code),
                            code,
                        });
                    }
                    advanced_to = end_offset;
                    next_index += 1;
                }
                Err(e) => {
                    let _ = channel.send(LiveEvent::FightStatus {
                        index: fr.index,
                        status: format!("failed: {e}"),
                    });
                    // Don't advance past a failed fight or bump the index; it
                    // will be retried (same index) on the next change once
                    // conditions improve. Advancing only on success also avoids
                    // a tight retry loop.
                    break;
                }
            }
        }
        // Advance `consumed` only past dispatched fights. Trailing partial data
        // (an in-progress fight) is re-scanned next pass.
        if advanced_to > consumed {
            consumed = advanced_to;
        } else if read_end == size {
            // No completed fight and we've caught up to EOF — keep `consumed`
            // where it is so the in-progress fight is re-scanned with more data.
        }
    }
}

/// Scan a freshly-read chunk for *completed* fights, returning absolute byte
/// ranges (chunk offsets shifted by `base`). Reuses the same boundary logic as
/// the full-file scanner via a tiny inline pass to avoid duplicating offsets.
fn scan_chunk_fights(chunk: &str, base: u64) -> Vec<FightRange> {
    let result = scanner::scan_chunk_for_fights(chunk, base);
    result
        .into_iter()
        .map(|f| FightRange {
            index: 0,
            start_offset: f.start_offset,
            end_offset: f.end_offset,
            zone_name: f.zone_name,
            boss_name: f.boss_name,
            start_ms: f.start_ms,
            end_ms: f.end_ms,
        })
        .collect()
}

/// Build the public report URL from a report code.
pub fn report_url(code: &str) -> String {
    format!("https://www.esologs.com/reports/{code}")
}
