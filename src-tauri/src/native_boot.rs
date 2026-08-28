//! Launch-ID-bound readiness protocol for the native Slint shell.
//!
//! `native-boot.pending` remains the recovery anchor, but it now names one
//! launch.  A child can acknowledge only that launch by publishing the same
//! identity to `native-boot.ready`.  The parent never treats elapsed time or a
//! marker left by another launch as proof that the child is usable.

use fs4::{FileExt, TryLockError};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::RandomState;
use std::fs;
use std::fs::{File, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(crate) const PENDING_FILE: &str = "native-boot.pending";
pub(crate) const READY_FILE: &str = "native-boot.ready";
pub(crate) const ACQUIRED_FILE: &str = "native-boot.acquired";
pub(crate) const LAUNCH_ID_ENV: &str = "KALPA_NATIVE_LAUNCH_ID";
pub(crate) const WEBVIEW_LAUNCH_ID_ENV: &str = "KALPA_WEBVIEW_LAUNCH_ID";
pub(crate) const ACTIVE_FILE: &str = "native-shell.active";
pub(crate) const SHUTDOWN_FILE: &str = "native-shell.shutdown";
const AUTHORITY_FILE: &str = "native-shell.authority";
/// A boot record is republished from a fresh staging path on failure; see
/// `write_record`. Bounded so a genuinely unwritable state dir still fails.
const PUBLISH_ATTEMPTS: usize = 3;
const PUBLISH_BACKOFF: Duration = Duration::from_millis(100);
#[allow(dead_code)] // Used by the Tauri crate; this file is also path-included by Slint.
pub(crate) const READY_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
static LAUNCH_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct BootRecord {
    pub launch_id: String,
    pub parent_pid: u32,
}

#[allow(dead_code)] // Parent-only; shared source also compiles in the child crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChildState {
    Running,
    Exited,
}

#[allow(dead_code)] // Parent-only; shared source also compiles in the child crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WaitOutcome {
    Ready,
    ChildExited,
    TimedOut,
}

pub(crate) enum AuthorityClaim {
    Held(AuthorityGuard),
    AlreadyHeld,
}

pub(crate) struct AuthorityGuard {
    file: File,
    launch_id: String,
    state_dir: PathBuf,
}

impl AuthorityGuard {
    #[allow(dead_code)] // Used by the path-including Slint crate.
    pub(crate) fn launch_id(&self) -> &str {
        &self.launch_id
    }

    pub(crate) fn signal_acquired(&self) -> Result<bool, String> {
        signal_acquired(&self.state_dir, &self.launch_id)
    }
}

impl Drop for AuthorityGuard {
    fn drop(&mut self) {
        // The active record belongs to the lock epoch. Clear it while this
        // guard still excludes a successor; otherwise an old owner can unlock,
        // pause, and then delete the new owner's freshly published record.
        clear_record_if_owned(&self.state_dir.join(ACTIVE_FILE), &self.launch_id);
        let _ = FileExt::unlock(&self.file);
    }
}

pub(crate) fn pending_path(state_dir: &Path) -> PathBuf {
    state_dir.join(PENDING_FILE)
}

pub(crate) fn ready_path(state_dir: &Path) -> PathBuf {
    state_dir.join(READY_FILE)
}

pub(crate) fn acquired_path(state_dir: &Path) -> PathBuf {
    state_dir.join(ACQUIRED_FILE)
}

#[allow(dead_code)] // Used by the Tauri parent crate.
pub(crate) fn authority_path(state_dir: &Path) -> PathBuf {
    state_dir.join(AUTHORITY_FILE)
}

pub(crate) fn new_launch_id() -> String {
    let counter = LAUNCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut first = RandomState::new().build_hasher();
    first.write_u128(clock);
    first.write_u64(counter);
    first.write_u32(std::process::id());
    let mut second = RandomState::new().build_hasher();
    second.write_u64(first.finish());
    second.write_u128(clock.rotate_left(37));
    second.write_u64(counter.rotate_left(19));
    format!("{:016x}{:016x}", first.finish(), second.finish())
}

pub(crate) fn try_claim_authority(
    state_dir: &Path,
    launch_id: &str,
) -> Result<AuthorityClaim, String> {
    fs::create_dir_all(state_dir)
        .map_err(|error| format!("Could not create native state directory: {error}"))?;
    let path = state_dir.join(AUTHORITY_FILE);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| format!("Could not open native UI authority lock: {error}"))?;
    match FileExt::try_lock(&file) {
        Ok(()) => {
            let guard = AuthorityGuard {
                file,
                launch_id: launch_id.to_string(),
                state_dir: state_dir.to_path_buf(),
            };
            write_record(
                &state_dir.join(ACTIVE_FILE),
                &BootRecord {
                    launch_id: launch_id.to_string(),
                    parent_pid: std::process::id(),
                },
            )?;
            let _ = fs::remove_file(state_dir.join(SHUTDOWN_FILE));
            Ok(AuthorityClaim::Held(guard))
        }
        Err(TryLockError::WouldBlock) => Ok(AuthorityClaim::AlreadyHeld),
        Err(TryLockError::Error(error)) => {
            Err(format!("Could not acquire native UI authority: {error}"))
        }
    }
}

pub(crate) fn request_active_shutdown(state_dir: &Path) -> Result<bool, String> {
    let Some(active) = read_record(&state_dir.join(ACTIVE_FILE)) else {
        return Ok(false);
    };
    write_record(&state_dir.join(SHUTDOWN_FILE), &active)?;
    eprintln!(
        "[native-shell] requested authority release launch_id={}",
        active.launch_id
    );
    Ok(true)
}

#[allow(dead_code)] // Used by the path-including Slint crate.
pub(crate) fn native_authority_is_active(state_dir: &Path) -> bool {
    read_record(&state_dir.join(ACTIVE_FILE))
        .map(|record| !record.launch_id.starts_with("webview-"))
        .unwrap_or(false)
}

#[allow(dead_code)] // Used by the path-including Slint crate.
pub(crate) fn webview_authority_is_active(state_dir: &Path) -> bool {
    read_record(&state_dir.join(ACTIVE_FILE))
        .map(|record| record.launch_id.starts_with("webview-"))
        .unwrap_or(false)
}

/// Positive proof that a native active record is backed by a live process
/// holding the OS authority lock. A crash can leave `native-shell.active`
/// behind, so the record alone is never sufficient for duplicate acceptance.
#[allow(dead_code)] // Used by the Tauri parent crate.
pub(crate) fn live_native_authority_exists(state_dir: &Path) -> bool {
    if !native_authority_is_active(state_dir) {
        return false;
    }
    let Ok(file) = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(state_dir.join(AUTHORITY_FILE))
    else {
        return false;
    };
    match FileExt::try_lock(&file) {
        Err(TryLockError::WouldBlock) => true,
        Ok(()) => {
            let _ = FileExt::unlock(&file);
            false
        }
        Err(TryLockError::Error(_)) => false,
    }
}

#[allow(dead_code)] // Used by the path-including Slint crate.
pub(crate) fn shutdown_requested(state_dir: &Path, launch_id: &str) -> bool {
    read_record(&state_dir.join(SHUTDOWN_FILE))
        .map(|record| record.launch_id == launch_id)
        .unwrap_or(false)
}

#[allow(dead_code)] // Used by the Tauri crate; this file is also path-included by Slint.
pub(crate) fn claim_webview_after_shutdown(
    state_dir: &Path,
    timeout: Duration,
) -> Result<AuthorityGuard, String> {
    let launch_id = format!("webview-{}", new_launch_id());
    let deadline = Instant::now() + timeout;
    let mut requested = false;
    let mut last_request = None;
    loop {
        match try_claim_authority(state_dir, &launch_id)? {
            AuthorityClaim::Held(guard) => {
                eprintln!("[native-shell] WebView acquired UI authority");
                return Ok(guard);
            }
            AuthorityClaim::AlreadyHeld => {
                // Re-read and re-target periodically: a crashed/relaunched native
                // owner can replace the active launch ID while this WebView waits.
                // A one-shot request would remain bound to the dead owner forever.
                if last_request
                    .map(|instant: Instant| instant.elapsed() >= Duration::from_millis(250))
                    .unwrap_or(true)
                {
                    requested = request_active_shutdown(state_dir)?;
                    last_request = Some(Instant::now());
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(if requested {
                "Timed out waiting for the native shell to release UI authority.".to_string()
            } else {
                "Native UI authority is held but its active record is unavailable.".to_string()
            });
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Wait for the current UI owner to complete a launch-bound handoff, without
/// asking it to shut down. The ready parent owns that release decision.
#[allow(dead_code)] // Used by the path-including Slint crate.
pub(crate) fn claim_after_ready_release(
    state_dir: &Path,
    launch_id: &str,
    timeout: Duration,
) -> Result<AuthorityGuard, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match try_claim_authority(state_dir, launch_id)? {
            AuthorityClaim::Held(guard) => return Ok(guard),
            AuthorityClaim::AlreadyHeld => {}
        }
        if Instant::now() >= deadline {
            return Err("Timed out waiting for the ready parent to release UI authority.".into());
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[allow(dead_code)] // Used by the Tauri parent crate.
pub(crate) fn prepare(state_dir: &Path, launch_id: &str) -> Result<(), String> {
    fs::create_dir_all(state_dir)
        .map_err(|error| format!("Could not create native state directory: {error}"))?;
    let _ = fs::remove_file(ready_path(state_dir));
    let _ = fs::remove_file(acquired_path(state_dir));
    write_record(
        &pending_path(state_dir),
        &BootRecord {
            launch_id: launch_id.to_string(),
            parent_pid: std::process::id(),
        },
    )
}

#[allow(dead_code)] // Used by the path-including Slint crate.
pub(crate) fn signal_ready(state_dir: &Path, launch_id: &str) -> Result<bool, String> {
    let Some(pending) = read_record(&pending_path(state_dir)) else {
        return Ok(false);
    };
    if pending.launch_id != launch_id {
        return Ok(false);
    }
    write_record(&ready_path(state_dir), &pending)?;
    Ok(true)
}

#[allow(dead_code)] // Used by the Tauri parent crate.
pub(crate) fn ready_matches(state_dir: &Path, launch_id: &str) -> bool {
    read_record(&ready_path(state_dir))
        .map(|record| record.launch_id == launch_id)
        .unwrap_or(false)
}

pub(crate) fn signal_acquired(state_dir: &Path, launch_id: &str) -> Result<bool, String> {
    let pending_matches = read_record(&pending_path(state_dir))
        .map(|record| record.launch_id == launch_id)
        .unwrap_or(false);
    let active_matches = read_record(&state_dir.join(ACTIVE_FILE))
        .map(|record| record.launch_id == launch_id)
        .unwrap_or(false);
    if !pending_matches || !active_matches {
        return Ok(false);
    }
    write_record(
        &acquired_path(state_dir),
        &BootRecord {
            launch_id: launch_id.to_string(),
            parent_pid: std::process::id(),
        },
    )?;
    Ok(true)
}

pub(crate) fn acquired_matches(state_dir: &Path, launch_id: &str) -> bool {
    read_record(&acquired_path(state_dir))
        .map(|record| record.launch_id == launch_id)
        .unwrap_or(false)
}

#[allow(dead_code)] // Used by the Tauri parent crate.
pub(crate) fn clear_owned(state_dir: &Path, launch_id: &str) {
    for path in [
        pending_path(state_dir),
        ready_path(state_dir),
        acquired_path(state_dir),
    ] {
        if read_record(&path)
            .map(|record| record.launch_id == launch_id)
            .unwrap_or(false)
        {
            let _ = fs::remove_file(path);
        }
    }
}

#[allow(dead_code)] // Used by the Tauri crate; this file is also path-included by Slint.
pub(crate) fn has_pending(state_dir: &Path) -> bool {
    pending_path(state_dir).is_file()
}

#[allow(dead_code)] // Used by the Tauri parent crate.
pub(crate) fn wait_for_ready(
    state_dir: &Path,
    launch_id: &str,
    timeout: Duration,
    mut child_state: impl FnMut() -> Result<ChildState, String>,
) -> Result<WaitOutcome, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if ready_matches(state_dir, launch_id) {
            return Ok(if child_state()? == ChildState::Running {
                WaitOutcome::Ready
            } else {
                WaitOutcome::ChildExited
            });
        }
        if child_state()? == ChildState::Exited {
            return Ok(WaitOutcome::ChildExited);
        }
        if Instant::now() >= deadline {
            return Ok(WaitOutcome::TimedOut);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[allow(dead_code)] // Used by both parent crates.
pub(crate) fn wait_for_acquired(
    state_dir: &Path,
    launch_id: &str,
    timeout: Duration,
    mut child_state: impl FnMut() -> Result<ChildState, String>,
) -> Result<WaitOutcome, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if acquired_matches(state_dir, launch_id) {
            return Ok(if child_state()? == ChildState::Running {
                WaitOutcome::Ready
            } else {
                WaitOutcome::ChildExited
            });
        }
        if child_state()? == ChildState::Exited {
            return Ok(WaitOutcome::ChildExited);
        }
        if Instant::now() >= deadline {
            return Ok(WaitOutcome::TimedOut);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[allow(dead_code)] // Used by both parent crates.
pub(crate) fn terminate_and_reap_child(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Result<(), String> {
    if child
        .try_wait()
        .map_err(|error| format!("Could not inspect child process: {error}"))?
        .is_some()
    {
        return Ok(());
    }
    let kill_error = child.kill().err();
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL_INTERVAL),
            Ok(None) => {
                return Err(kill_error.map_or_else(
                    || "Timed out reaping terminated child process.".to_string(),
                    |error| format!("Could not terminate child process: {error}"),
                ))
            }
            Err(error) => return Err(format!("Could not reap child process: {error}")),
        }
    }
}

fn read_record(path: &Path) -> Option<BootRecord> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn clear_record_if_owned(path: &Path, launch_id: &str) {
    if read_record(path)
        .map(|record| record.launch_id == launch_id)
        .unwrap_or(false)
    {
        let _ = fs::remove_file(path);
    }
}

/// Publish a boot record, retrying the *whole* atomic write rather than just
/// the rename. A scanner or indexer holding the freshly created staging file
/// cannot be waited out in place; only a new `create_new` staging path escapes
/// it, so the handshake does not inherit the publisher's rename budget.
fn write_record(path: &Path, record: &BootRecord) -> Result<(), String> {
    let bytes = serde_json::to_vec(record)
        .map_err(|error| format!("Could not encode native boot record: {error}"))?;
    let mut last = None;
    for attempt in 0..PUBLISH_ATTEMPTS {
        match crate::atomic_file::atomic_write(path, &bytes) {
            Ok(()) => return Ok(()),
            Err(error) => {
                eprintln!(
                    "[native-shell] publish attempt {} for {} failed: {error} (os error {:?})",
                    attempt + 1,
                    path.display(),
                    error.raw_os_error()
                );
                last = Some(error);
                if attempt + 1 < PUBLISH_ATTEMPTS {
                    std::thread::sleep(PUBLISH_BACKOFF);
                }
            }
        }
    }
    Err(format!(
        "Could not publish {}: {}",
        path.display(),
        last.expect("publish loop records a failure before exhausting attempts")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn matching_child_ready_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        prepare(dir.path(), "launch-a").unwrap();
        assert!(signal_ready(dir.path(), "launch-a").unwrap());
        assert_eq!(
            wait_for_ready(dir.path(), "launch-a", Duration::ZERO, || Ok(
                ChildState::Running
            ))
            .unwrap(),
            WaitOutcome::Ready
        );
    }

    #[test]
    fn stale_or_wrong_ready_marker_cannot_confirm_launch() {
        let dir = tempfile::tempdir().unwrap();
        prepare(dir.path(), "old-launch").unwrap();
        signal_ready(dir.path(), "old-launch").unwrap();
        prepare(dir.path(), "new-launch").unwrap();
        write_record(
            &ready_path(dir.path()),
            &BootRecord {
                launch_id: "old-launch".into(),
                parent_pid: 1,
            },
        )
        .unwrap();
        assert_eq!(
            wait_for_ready(dir.path(), "new-launch", Duration::ZERO, || Ok(
                ChildState::Running
            ))
            .unwrap(),
            WaitOutcome::TimedOut
        );
        assert!(!signal_ready(dir.path(), "old-launch").unwrap());
    }

    #[test]
    fn child_exit_wins_before_readiness() {
        let dir = tempfile::tempdir().unwrap();
        prepare(dir.path(), "launch-a").unwrap();
        assert_eq!(
            wait_for_ready(dir.path(), "launch-a", Duration::from_secs(1), || Ok(
                ChildState::Exited
            ))
            .unwrap(),
            WaitOutcome::ChildExited
        );
    }

    #[test]
    fn timeout_is_bounded_and_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        prepare(dir.path(), "launch-a").unwrap();
        let probes = AtomicUsize::new(0);
        assert_eq!(
            wait_for_ready(dir.path(), "launch-a", Duration::from_millis(30), || {
                probes.fetch_add(1, Ordering::Relaxed);
                Ok(ChildState::Running)
            })
            .unwrap(),
            WaitOutcome::TimedOut
        );
        assert!(probes.load(Ordering::Relaxed) >= 1);
    }

    #[test]
    fn rapid_relaunch_cleanup_never_removes_new_launch() {
        let dir = tempfile::tempdir().unwrap();
        prepare(dir.path(), "first").unwrap();
        prepare(dir.path(), "second").unwrap();
        clear_owned(dir.path(), "first");
        assert_eq!(
            read_record(&pending_path(dir.path())).unwrap().launch_id,
            "second"
        );
    }

    #[test]
    fn ready_marker_does_not_hide_a_child_that_already_exited() {
        let dir = tempfile::tempdir().unwrap();
        prepare(dir.path(), "launch-a").unwrap();
        signal_ready(dir.path(), "launch-a").unwrap();
        assert_eq!(
            wait_for_ready(dir.path(), "launch-a", Duration::from_secs(1), || Ok(
                ChildState::Exited
            ))
            .unwrap(),
            WaitOutcome::ChildExited
        );
    }

    #[test]
    fn authority_shutdown_is_bound_to_the_active_launch() {
        let dir = tempfile::tempdir().unwrap();
        let guard = match try_claim_authority(dir.path(), "native-a").unwrap() {
            AuthorityClaim::Held(guard) => guard,
            AuthorityClaim::AlreadyHeld => panic!("fresh directory was already locked"),
        };
        assert!(native_authority_is_active(dir.path()));
        assert!(live_native_authority_exists(dir.path()));
        assert!(request_active_shutdown(dir.path()).unwrap());
        assert!(shutdown_requested(dir.path(), "native-a"));
        assert!(!shutdown_requested(dir.path(), "native-b"));
        assert!(matches!(
            try_claim_authority(dir.path(), "webview-b").unwrap(),
            AuthorityClaim::AlreadyHeld
        ));
        drop(guard);
        assert!(!live_native_authority_exists(dir.path()));
        assert!(matches!(
            try_claim_authority(dir.path(), "webview-b").unwrap(),
            AuthorityClaim::Held(_)
        ));
    }

    #[test]
    fn invalid_or_unwritable_state_fails_closed() {
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(prepare(file.path(), "launch-a").is_err());
    }

    #[test]
    fn authority_publication_failure_cannot_become_ready() {
        let dir = tempfile::tempdir().unwrap();
        prepare(dir.path(), "launch-a").unwrap();
        fs::create_dir(dir.path().join(ACTIVE_FILE)).unwrap();

        assert!(try_claim_authority(dir.path(), "launch-a").is_err());
        assert!(!ready_matches(dir.path(), "launch-a"));

        // The failed active-record publication drops/unlocks its provisional
        // guard, so a corrected retry can deterministically acquire authority.
        fs::remove_dir(dir.path().join(ACTIVE_FILE)).unwrap();
        assert!(matches!(
            try_claim_authority(dir.path(), "launch-b").unwrap(),
            AuthorityClaim::Held(_)
        ));
    }

    #[test]
    fn ready_child_acquires_authority_only_after_parent_releases() {
        let dir = tempfile::tempdir().unwrap();
        let parent = match try_claim_authority(dir.path(), "webview-parent").unwrap() {
            AuthorityClaim::Held(guard) => guard,
            AuthorityClaim::AlreadyHeld => panic!("fresh directory was already locked"),
        };
        let state_dir = dir.path().to_path_buf();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            drop(parent);
        });

        let child = claim_after_ready_release(&state_dir, "native-child", Duration::from_secs(1))
            .expect("ready child acquires after parent release");
        assert_eq!(child.launch_id(), "native-child");
        release.join().unwrap();
    }

    #[test]
    fn blocked_ready_path_cannot_create_false_readiness() {
        let dir = tempfile::tempdir().unwrap();
        prepare(dir.path(), "launch-a").unwrap();
        fs::create_dir(ready_path(dir.path())).unwrap();
        assert!(signal_ready(dir.path(), "launch-a").is_err());
        assert!(!ready_matches(dir.path(), "launch-a"));
    }

    #[test]
    fn child_process_helper() {
        let Ok(mode) = std::env::var("KALPA_BOOT_TEST_CHILD") else {
            return;
        };
        let state_dir = PathBuf::from(std::env::var_os("KALPA_NATIVE_STATE_DIR").unwrap());
        let launch_id = std::env::var(LAUNCH_ID_ENV).unwrap();
        match mode.as_str() {
            "ready" => {
                assert!(signal_ready(&state_dir, &launch_id).unwrap());
                std::thread::sleep(Duration::from_secs(2));
            }
            "crash" => std::process::exit(91),
            // Claim UI authority, prove readiness, then die without unwinding.
            // `process::exit` runs no destructors, so only the kernel releases
            // the lock - exactly what a real crash leaves behind.
            "authority-ready-crash" => match try_claim_authority(&state_dir, &launch_id).unwrap() {
                AuthorityClaim::Held(_guard) => {
                    assert!(signal_ready(&state_dir, &launch_id).unwrap());
                    std::process::exit(92);
                }
                AuthorityClaim::AlreadyHeld => std::process::exit(93),
            },
            "authority-hold" => match try_claim_authority(&state_dir, &launch_id).unwrap() {
                AuthorityClaim::Held(_guard) => {
                    assert!(signal_ready(&state_dir, &launch_id).unwrap());
                    std::thread::sleep(Duration::from_secs(30));
                }
                AuthorityClaim::AlreadyHeld => std::process::exit(93),
            },
            "ready-then-crash-before-authority" => {
                assert!(signal_ready(&state_dir, &launch_id).unwrap());
                let deadline = Instant::now() + Duration::from_secs(10);
                while !state_dir.join("crash-before-authority").is_file()
                    && Instant::now() < deadline
                {
                    std::thread::sleep(POLL_INTERVAL);
                }
                std::process::exit(96);
            }
            "ready-without-authority-hold" => {
                assert!(signal_ready(&state_dir, &launch_id).unwrap());
                std::thread::sleep(Duration::from_secs(30));
            }
            "ready-acquire-hold" => {
                assert!(signal_ready(&state_dir, &launch_id).unwrap());
                let guard =
                    claim_after_ready_release(&state_dir, &launch_id, Duration::from_secs(10))
                        .unwrap();
                assert!(guard.signal_acquired().unwrap());
                std::thread::sleep(Duration::from_secs(30));
            }
            "authority-reclaim" => {
                let deadline = Instant::now() + Duration::from_secs(10);
                let _guard = loop {
                    match try_claim_authority(&state_dir, &launch_id).unwrap() {
                        AuthorityClaim::Held(guard) => break guard,
                        AuthorityClaim::AlreadyHeld if Instant::now() < deadline => {
                            std::thread::sleep(POLL_INTERVAL);
                        }
                        AuthorityClaim::AlreadyHeld => std::process::exit(94),
                    }
                };
                fs::write(state_dir.join("reclaimer.claimed"), b"claimed").unwrap();
                let deadline = Instant::now() + Duration::from_secs(10);
                while !state_dir.join("reclaimer.release").is_file() && Instant::now() < deadline {
                    std::thread::sleep(POLL_INTERVAL);
                }
                if !state_dir.join("reclaimer.release").is_file() {
                    std::process::exit(95);
                }
            }
            "hang" => std::thread::sleep(Duration::from_secs(2)),
            _ => panic!("unknown child mode"),
        }
    }

    fn spawn_protocol_child(state_dir: &Path, launch_id: &str, mode: &str) -> std::process::Child {
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "native_boot::tests::child_process_helper"])
            .env("KALPA_BOOT_TEST_CHILD", mode)
            .env("KALPA_NATIVE_STATE_DIR", state_dir)
            .env(LAUNCH_ID_ENV, launch_id)
            .spawn()
            .unwrap()
    }

    /// Duplicate acceptance keys on the held OS lock alone. These two tests are
    /// why the `ready_matches` conjunct was safe to drop: a published marker
    /// proves the child *reached* readiness, never that it is still alive.
    #[test]
    fn a_live_owner_is_detected_across_processes() {
        let root = tempfile::tempdir().unwrap();
        let state_dir = root.path().join("state");
        prepare(&state_dir, "live-owner").unwrap();
        let mut child = spawn_protocol_child(&state_dir, "live-owner", "authority-hold");
        let outcome = wait_for_ready(&state_dir, "live-owner", Duration::from_secs(10), || {
            Ok(if child.try_wait().unwrap().is_some() {
                ChildState::Exited
            } else {
                ChildState::Running
            })
        })
        .unwrap();
        assert_eq!(outcome, WaitOutcome::Ready);
        assert!(live_native_authority_exists(&state_dir));
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn a_crashed_child_that_published_ready_is_not_a_live_owner() {
        let root = tempfile::tempdir().unwrap();
        let state_dir = root.path().join("state");
        prepare(&state_dir, "crash-after-ready").unwrap();
        let mut child =
            spawn_protocol_child(&state_dir, "crash-after-ready", "authority-ready-crash");
        assert_eq!(child.wait().unwrap().code(), Some(92));
        // It really did publish readiness before dying...
        assert!(ready_matches(&state_dir, "crash-after-ready"));
        // ...but the kernel released its authority lock, so it is not a live
        // owner and must never be accepted as one.
        assert!(!live_native_authority_exists(&state_dir));
    }

    #[test]
    fn ready_then_crash_before_authority_never_completes_handoff() {
        let root = tempfile::tempdir().unwrap();
        let state_dir = root.path().join("state");
        prepare(&state_dir, "successor").unwrap();
        let parent = match try_claim_authority(&state_dir, "parent").unwrap() {
            AuthorityClaim::Held(guard) => guard,
            AuthorityClaim::AlreadyHeld => panic!("fresh directory was already locked"),
        };
        let mut child =
            spawn_protocol_child(&state_dir, "successor", "ready-then-crash-before-authority");
        assert_eq!(
            wait_for_ready(&state_dir, "successor", Duration::from_secs(10), || {
                Ok(if child.try_wait().unwrap().is_some() {
                    ChildState::Exited
                } else {
                    ChildState::Running
                })
            })
            .unwrap(),
            WaitOutcome::Ready
        );
        drop(parent);
        fs::write(state_dir.join("crash-before-authority"), b"crash").unwrap();
        assert_eq!(
            wait_for_acquired(&state_dir, "successor", Duration::from_secs(10), || {
                Ok(if child.try_wait().unwrap().is_some() {
                    ChildState::Exited
                } else {
                    ChildState::Running
                })
            })
            .unwrap(),
            WaitOutcome::ChildExited
        );
        assert!(!acquired_matches(&state_dir, "successor"));
    }

    #[test]
    fn handoff_completes_only_after_successor_proves_authority() {
        let root = tempfile::tempdir().unwrap();
        let state_dir = root.path().join("state");
        prepare(&state_dir, "successor").unwrap();
        let parent = match try_claim_authority(&state_dir, "parent").unwrap() {
            AuthorityClaim::Held(guard) => guard,
            AuthorityClaim::AlreadyHeld => panic!("fresh directory was already locked"),
        };
        let mut child = spawn_protocol_child(&state_dir, "successor", "ready-acquire-hold");
        assert_eq!(
            wait_for_ready(&state_dir, "successor", Duration::from_secs(10), || {
                Ok(if child.try_wait().unwrap().is_some() {
                    ChildState::Exited
                } else {
                    ChildState::Running
                })
            })
            .unwrap(),
            WaitOutcome::Ready
        );
        assert!(!acquired_matches(&state_dir, "successor"));
        drop(parent);
        assert_eq!(
            wait_for_acquired(&state_dir, "successor", Duration::from_secs(10), || {
                Ok(if child.try_wait().unwrap().is_some() {
                    ChildState::Exited
                } else {
                    ChildState::Running
                })
            })
            .unwrap(),
            WaitOutcome::Ready
        );
        assert!(live_native_authority_exists(&state_dir));
        terminate_and_reap_child(&mut child, Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn ready_then_block_before_authority_times_out_and_is_reaped() {
        let root = tempfile::tempdir().unwrap();
        let state_dir = root.path().join("state");
        prepare(&state_dir, "successor").unwrap();
        let parent = match try_claim_authority(&state_dir, "parent").unwrap() {
            AuthorityClaim::Held(guard) => guard,
            AuthorityClaim::AlreadyHeld => panic!("fresh directory was already locked"),
        };
        let mut child =
            spawn_protocol_child(&state_dir, "successor", "ready-without-authority-hold");
        assert_eq!(
            wait_for_ready(&state_dir, "successor", Duration::from_secs(10), || {
                Ok(if child.try_wait().unwrap().is_some() {
                    ChildState::Exited
                } else {
                    ChildState::Running
                })
            })
            .unwrap(),
            WaitOutcome::Ready
        );
        drop(parent);
        assert_eq!(
            wait_for_acquired(&state_dir, "successor", Duration::from_millis(100), || {
                Ok(if child.try_wait().unwrap().is_some() {
                    ChildState::Exited
                } else {
                    ChildState::Running
                })
            })
            .unwrap(),
            WaitOutcome::TimedOut
        );
        terminate_and_reap_child(&mut child, Duration::from_secs(1)).unwrap();
        assert!(child.try_wait().unwrap().is_some());
        assert!(!acquired_matches(&state_dir, "successor"));
    }

    #[test]
    fn terminate_and_reap_accepts_a_child_that_won_the_exit_race() {
        let root = tempfile::tempdir().unwrap();
        let state_dir = root.path().join("state");
        prepare(&state_dir, "already-exited").unwrap();
        let mut child = spawn_protocol_child(&state_dir, "already-exited", "crash");
        std::thread::sleep(Duration::from_millis(100));

        terminate_and_reap_child(&mut child, Duration::from_secs(1)).unwrap();
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn pre_ready_cancellation_terminates_and_reaps_spawned_child() {
        let root = tempfile::tempdir().unwrap();
        let state_dir = root.path().join("state");
        prepare(&state_dir, "cancelled-child").unwrap();
        let mut child = spawn_protocol_child(&state_dir, "cancelled-child", "hang");
        let mut cancel = true;

        let outcome = wait_for_ready(
            &state_dir,
            "cancelled-child",
            Duration::from_secs(3),
            || {
                if cancel {
                    cancel = false;
                    terminate_and_reap_child(&mut child, Duration::from_secs(1))?;
                    return Ok(ChildState::Exited);
                }
                Ok(if child.try_wait().unwrap().is_some() {
                    ChildState::Exited
                } else {
                    ChildState::Running
                })
            },
        )
        .unwrap();

        assert_eq!(outcome, WaitOutcome::ChildExited);
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn successor_active_record_survives_authority_release_and_reclaim() {
        let root = tempfile::tempdir().unwrap();
        let state_dir = root.path().join("state");
        let old = match try_claim_authority(&state_dir, "old-owner").unwrap() {
            AuthorityClaim::Held(guard) => guard,
            AuthorityClaim::AlreadyHeld => panic!("fresh directory was already locked"),
        };
        let mut successor = spawn_protocol_child(&state_dir, "new-owner", "authority-reclaim");

        drop(old);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !state_dir.join("reclaimer.claimed").is_file() && Instant::now() < deadline {
            std::thread::sleep(POLL_INTERVAL);
        }
        assert_eq!(
            read_record(&state_dir.join(ACTIVE_FILE))
                .expect("successor publishes its active record")
                .launch_id,
            "new-owner"
        );

        fs::write(state_dir.join("reclaimer.release"), b"release").unwrap();
        assert!(successor.wait().unwrap().success());
    }

    #[test]
    fn real_child_success_exit_and_timeout_are_distinguished() {
        let root = tempfile::tempdir().unwrap();
        let state_dir = root.path().join("state with spaces");

        prepare(&state_dir, "ready-child").unwrap();
        let mut ready = spawn_protocol_child(&state_dir, "ready-child", "ready");
        assert_eq!(
            wait_for_ready(&state_dir, "ready-child", Duration::from_secs(3), || {
                Ok(if ready.try_wait().unwrap().is_some() {
                    ChildState::Exited
                } else {
                    ChildState::Running
                })
            })
            .unwrap(),
            WaitOutcome::Ready
        );
        ready.kill().unwrap();
        ready.wait().unwrap();

        prepare(&state_dir, "crashed-child").unwrap();
        let mut crashed = spawn_protocol_child(&state_dir, "crashed-child", "crash");
        assert_eq!(
            wait_for_ready(&state_dir, "crashed-child", Duration::from_secs(1), || Ok(
                if crashed.try_wait().unwrap().is_some() {
                    ChildState::Exited
                } else {
                    ChildState::Running
                }
            ))
            .unwrap(),
            WaitOutcome::ChildExited
        );

        prepare(&state_dir, "hung-child").unwrap();
        let mut hung = spawn_protocol_child(&state_dir, "hung-child", "hang");
        assert_eq!(
            wait_for_ready(&state_dir, "hung-child", Duration::from_millis(75), || Ok(
                if hung.try_wait().unwrap().is_some() {
                    ChildState::Exited
                } else {
                    ChildState::Running
                }
            ))
            .unwrap(),
            WaitOutcome::TimedOut
        );
        hung.kill().unwrap();
        hung.wait().unwrap();
    }
}
