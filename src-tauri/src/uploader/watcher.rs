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
/// Give up after this many consecutive stat/read failures (~the active log was
/// deleted/renamed/replaced-by-a-dir, or a permanent AV/CFA/share lock). At a
/// ~400ms cadence this is ~12s of unbroken failure before we tear the session
/// down rather than spinning forever while the UI shows a stuck "LIVE".
const MAX_CONSECUTIVE_FAILURES: u32 = 30;

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

/// Read `[start, end)` from a file into the caller-owned `buf`, bounded by
/// [`MAX_READ`], and return the number of bytes read. `buf` is resized to
/// exactly that length, so the valid bytes are `&buf[..len]`.
///
/// The caller hands in a buffer that lives for the whole tail loop so this
/// never allocates per pass — it grows to the session's high-water mark once
/// and is reused. Offsets are still tracked from true byte lengths (the raw
/// bytes), never from a lossily-decoded string.
fn read_range(path: &Path, start: u64, end: u64, buf: &mut Vec<u8>) -> Result<usize, String> {
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

    // Construct the filesystem watcher SYNCHRONOUSLY, before spawning the thread,
    // so a setup failure (watcher creation or registering the parent dir) returns
    // `Err` to the caller — which settles the history record and never promotes a
    // dead session. Previously this happened inside the thread, where a failure
    // could only emit `LiveEvent::Stopped` while the command had already returned
    // `Ok` and marked the record `Live`, leaving it stale with no watcher.
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        Config::default().with_poll_interval(POLL_INTERVAL),
    )
    .map_err(|e| format!("Could not start file watcher: {e}"))?;
    if let Some(parent) = path.parent() {
        watcher
            .watch(parent, RecursiveMode::NonRecursive)
            .map_err(|e| format!("Could not watch the logs folder: {e}"))?;
    }

    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);

    let _ = channel.send(LiveEvent::Started {
        file: file_path.to_string(),
        start_offset,
    });

    let thread = thread::spawn(move || {
        // Move the already-constructed watcher into the thread so it stays alive
        // for the whole tail loop (dropping it would stop notifications).
        tail_loop(path, start_offset, stop_thread, channel, watcher, rx);
    });

    Ok(LiveWatchHandle {
        stop,
        thread: Mutex::new(Some(thread)),
    })
}

/// The tailing loop: wait for a change → read new bytes → scan for completed
/// fights → emit each. `consumed` advances only past dispatched fights so a
/// fight spanning two reads is found once its `END_COMBAT` arrives.
fn tail_loop(
    path: PathBuf,
    start_offset: u64,
    stop: Arc<AtomicBool>,
    channel: Channel<LiveEvent>,
    // The watcher is constructed in `start_live_watch` (so setup failures surface
    // synchronously) and moved in here purely to keep it alive — dropping it ends
    // notifications. `_watcher` is intentionally unused beyond that.
    _watcher: RecommendedWatcher,
    rx: mpsc::Receiver<notify::Result<Event>>,
) {
    let mut consumed = start_offset;
    let mut next_index = 0usize;
    let mut last_poll = Instant::now();
    // Consecutive stat/read failures; reset on any successful pass. Past
    // MAX_CONSECUTIVE_FAILURES we stop instead of zombie-looping (L1).
    let mut consecutive_failures: u32 = 0;
    // Whether the bytes before `consumed` are inside an open session. Starting
    // at offset 0 (include-entire-file) the first chunk begins with the session
    // header, so it's NOT yet open; tailing from EOF we're already mid-session.
    let mut session_open = start_offset != 0;
    // Read buffer reused across every pass: it grows to the session's
    // high-water mark once instead of allocating a fresh Vec per read, so a
    // multi-hour raid does no per-pass heap churn (and the worst-case 64 MiB
    // catch-up window is allocated at most once).
    let mut read_buf: Vec<u8> = Vec::new();

    loop {
        if stop.load(Ordering::SeqCst) {
            let _ = channel.send(LiveEvent::Stopped {
                reason: "Live logging stopped.".into(),
            });
            return;
        }

        // Wait for an FS event or the poll interval. We watch the parent dir
        // non-recursively, so events arrive for sibling files too. Read when an
        // event names our file, OR whenever the poll deadline has elapsed — the
        // latter keeps the fallback authoritative even when sibling-file churn
        // (OneDrive, AV, other logs) keeps waking us with non-matching events
        // while our own Modify event was coalesced/dropped (the exact Windows
        // case the poll exists to cover).
        let event_for_us = match rx.recv_timeout(POLL_INTERVAL) {
            Ok(Ok(ev)) => {
                matches!(ev.kind, EventKind::Modify(_) | EventKind::Create(_))
                    && ev.paths.iter().any(|p| p == &path)
            }
            Ok(Err(_)) => false,
            Err(mpsc::RecvTimeoutError::Timeout) => false,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // The notify backend's sender dropped — its worker thread
                // terminated, typically because the watched logs folder was
                // deleted/renamed or lost access. No further FS events can ever
                // arrive, so this session is dead. Emit `Stopped` (like the other
                // fatal exits below) rather than a silent `break`: otherwise the
                // frontend never learns, and the backend's `Running` slot + the
                // `Live` history record stay stuck until the next-launch reconcile
                // (this process already ran `reconcile_stale_once`). The reason is
                // worded to NOT end in "stopped" so the UI surfaces it (the
                // frontend suppresses only the plain `…stopped.` user-stop text).
                let _ = channel.send(LiveEvent::Stopped {
                    reason: "Lost connection to the folder watcher (stopped watching).".into(),
                });
                return;
            }
        };
        // Re-check the stop flag after the (up to POLL_INTERVAL) wait and before
        // a potentially large read, so a stop requested while backfilling a big
        // static file is observed within one window, not after a full read+scan.
        if stop.load(Ordering::SeqCst) {
            let _ = channel.send(LiveEvent::Stopped {
                reason: "Live logging stopped.".into(),
            });
            return;
        }
        if !event_for_us && last_poll.elapsed() < POLL_INTERVAL {
            continue;
        }
        last_poll = Instant::now();

        let size = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(e) => {
                consecutive_failures += 1;
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    let _ = channel.send(LiveEvent::Stopped {
                        reason: format!("Lost access to the log file (stopped watching): {e}"),
                    });
                    return;
                }
                continue;
            }
        };
        // A successful stat means the file is reachable; clear any failure streak
        // (an idle file that never grows must not eventually trip the limit).
        consecutive_failures = 0;

        // Truncation / new session detection. After a reset the file starts
        // fresh, so the next chunk's leading BEGIN_LOG is the session header,
        // not a mid-stream re-enable — flag it so we don't double-emit a reset.
        if size < consumed {
            let _ = channel.send(LiveEvent::SessionReset);
            consumed = 0;
            next_index = 0;
            session_open = false;
            continue;
        }
        if size == consumed {
            continue;
        }

        // Cap each pass to MAX_READ; catch-up progresses over multiple passes.
        let read_end = size.min(consumed + MAX_READ);
        let n = match read_range(&path, consumed, read_end, &mut read_buf) {
            Ok(n) => n,
            Err(e) => {
                consecutive_failures += 1;
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    let _ = channel.send(LiveEvent::Stopped {
                        reason: format!(
                            "Could not keep reading the log file (stopped watching): {e}"
                        ),
                    });
                    return;
                }
                // Transient (e.g. sharing violation while ESO flushes) — retry.
                let _ = channel.send(LiveEvent::Warning {
                    message: format!("Read retry: {e}"),
                });
                continue;
            }
        };
        // The valid bytes are the prefix `read_buf` was just resized to; the
        // buffer's spare capacity past `n` holds stale bytes from prior passes.
        let chunk = &read_buf[..n];

        // Detect completed fights and boundary signals in the chunk. Once we've
        // read any data, subsequent chunks are mid-session.
        let scan = scanner::scan_chunk_for_fights(chunk, consumed, session_open);
        session_open = true;

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
