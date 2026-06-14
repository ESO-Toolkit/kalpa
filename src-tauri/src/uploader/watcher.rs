//! Live fight detector for the uploader's live dashboard.
//!
//! Tails the active `Encounter.log` by byte offset (the game only appends), so
//! it never re-reads the whole file, and emits a [`LiveEvent`] to the frontend
//! whenever a fight (`BEGIN_COMBAT`…`END_COMBAT`) completes. This drives the
//! per-fight timeline in the UI **only** — it does not upload.
//!
//! The actual live upload is performed once per session by handing the whole
//! `Encounter.log` to the official ESO Logs uploader with real-time uploading
//! enabled (see `commands::uploader_start_live`). A single fight slice has no
//! `BEGIN_LOG` header or actor/ability context, so it is not independently
//! uploadable; the official uploader tails the full file itself. Decoupling
//! detection from upload keeps this loop cheap and correct.
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

/// 64 MiB cap on a single incremental read, bounding memory per pass.
const MAX_READ: u64 = 64 * 1024 * 1024;
/// Poll fallback cadence — short enough to feel live, light on IO.
const POLL_INTERVAL: Duration = Duration::from_millis(400);

/// Events streamed to the frontend over the live-session [`Channel`].
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LiveEvent {
    /// Watching started; reports the file and starting byte offset.
    Started { file: String, start_offset: u64 },
    /// A fight finished (UI timeline entry).
    FightDetected {
        index: usize,
        zone_name: Option<String>,
        boss_name: Option<String>,
        duration_ms: u64,
    },
    /// The log was truncated or a new session began.
    SessionReset,
    /// A fight was too large to track in the timeline and was skipped (the full
    /// log still uploads). Distinct from `Warning` so the UI can surface this
    /// one meaningful event without being flooded by transient read retries.
    FightSkipped { reason: String },
    /// A non-fatal warning (e.g. transient read retry). The UI may log but
    /// should not toast these, as they can recur frequently.
    Warning { message: String },
    /// Watching stopped (user-initiated or fatal error).
    Stopped { reason: String },
}

/// Handle controlling a running live watcher. Both [`stop`](Self::stop) and
/// `Drop` signal the thread and join it, so the notify watcher and file handle
/// are released deterministically.
pub struct LiveWatchHandle {
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl LiveWatchHandle {
    fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Ok(mut guard) = self.thread.lock() {
            if let Some(handle) = guard.take() {
                let _ = handle.join();
            }
        }
    }

    /// Signal the watcher to stop and wait for its thread to exit.
    pub fn stop(&self) {
        self.shutdown();
    }
}

impl Drop for LiveWatchHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Read `[start, end)` from a file into a raw byte buffer, bounded by
/// [`MAX_READ`]. Returns raw bytes so callers track offsets from true byte
/// lengths (never from a lossily-decoded string).
fn read_range(path: &Path, start: u64, end: u64) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    if end <= start {
        return Ok(Vec::new());
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
    Ok(buf)
}

/// Start tailing `file_path` for fight detection. `start_offset` is where to
/// begin (0 to enumerate fights already in the file, or its current length to
/// only surface fights logged after Start).
///
/// Each completed fight emits a [`LiveEvent::FightDetected`] on `channel`.
/// Returns a handle that stops and joins the thread on drop.
pub fn start_live_watch(
    file_path: &str,
    start_offset: u64,
    channel: Channel<LiveEvent>,
) -> Result<LiveWatchHandle, String> {
    let path = PathBuf::from(file_path);
    if !path.is_file() {
        return Err(format!("File does not exist: {file_path}"));
    }

    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);

    let _ = channel.send(LiveEvent::Started {
        file: file_path.to_string(),
        start_offset,
    });

    let thread = thread::spawn(move || {
        tail_loop(path, start_offset, stop_thread, channel);
    });

    Ok(LiveWatchHandle {
        stop,
        thread: Mutex::new(Some(thread)),
    })
}

/// The tailing loop: wait for a change → read new bytes → scan for completed
/// fights → emit each. `consumed` advances only past dispatched fights so a
/// fight spanning two reads is found once its `END_COMBAT` arrives.
fn tail_loop(path: PathBuf, start_offset: u64, stop: Arc<AtomicBool>, channel: Channel<LiveEvent>) {
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

        // Cap each pass to MAX_READ; catch-up progresses over multiple passes.
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

        // Detect completed fights and boundary signals in the chunk.
        let scan = scanner::scan_chunk_for_fights(&chunk, consumed);

        // A mid-chunk BEGIN_LOG (a /encounterlog re-enable) starts a new session
        // in the same growing file. Dispatch any fights that completed *before*
        // the boundary (they belong to the closing session) so they aren't lost
        // from the timeline, then reset the UI, re-index from 0, and re-anchor
        // just past the boundary (new_session_at is already its next_offset).
        if let Some(new_at) = scan.new_session_at {
            for fr in scan.fights.iter().filter(|f| f.end_offset <= new_at) {
                let _ = channel.send(LiveEvent::FightDetected {
                    index: next_index,
                    zone_name: fr.zone_name.clone(),
                    boss_name: fr.boss_name.clone(),
                    duration_ms: fr.end_ms.saturating_sub(fr.start_ms),
                });
                next_index += 1;
            }
            let _ = channel.send(LiveEvent::SessionReset);
            next_index = 0;
            consumed = new_at.max(consumed + 1);
            continue;
        }

        let mut advanced_to = consumed;
        for fr in scan.fights {
            let _ = channel.send(LiveEvent::FightDetected {
                index: next_index,
                zone_name: fr.zone_name.clone(),
                boss_name: fr.boss_name.clone(),
                duration_ms: fr.end_ms.saturating_sub(fr.start_ms),
            });
            next_index += 1;
            advanced_to = fr.end_offset;
        }

        if advanced_to > consumed {
            // Advance past the last completed fight. Any open fight after it is
            // re-scanned next pass from the new `consumed`.
            consumed = advanced_to;
        } else if read_end == size {
            // Caught up to EOF with an in-progress (unterminated) fight — keep
            // `consumed` so the partial fight is re-scanned once more arrives.
        } else if let Some(open_start) = scan.open_fight_start {
            if open_start > consumed {
                // A fight began partway through the window but its END_COMBAT
                // landed past the read cap. Re-anchor to its BEGIN_COMBAT so the
                // next (fresh) window can capture the whole fight — this is the
                // common case, NOT an oversized fight.
                consumed = open_start;
            } else {
                // The open fight started at/before `consumed` and a full
                // MAX_READ window still held no END_COMBAT: the fight body alone
                // exceeds the read cap. Skip it (the official uploader still
                // streams the whole file) and report it distinctly so the UI can
                // surface this one meaningful skip without spamming.
                let skip_to = match chunk.iter().rposition(|b| *b == b'\n') {
                    Some(nl) => consumed + nl as u64 + 1,
                    None => read_end,
                };
                let _ = channel.send(LiveEvent::FightSkipped {
                    reason: "A single fight was too large to track in the live \
                             timeline; the full log still uploads."
                        .into(),
                });
                consumed = skip_to.max(consumed + 1);
            }
        } else {
            // Full non-EOF window with no fight and no open boundary at all —
            // genuinely unparseable; advance to the last newline to make progress.
            let skip_to = match chunk.iter().rposition(|b| *b == b'\n') {
                Some(nl) => consumed + nl as u64 + 1,
                None => read_end,
            };
            consumed = skip_to.max(consumed + 1);
        }
    }
}

/// Build the public report URL from a report code.
pub fn report_url(code: &str) -> String {
    format!("https://www.esologs.com/reports/{code}")
}
