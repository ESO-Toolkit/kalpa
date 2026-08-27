//! Crash-safe publication of one complete file.
//!
//! This module guarantees atomic publication of a single file. It does **not**
//! serialize read-modify-write transactions across threads or processes: two
//! callers can both read version 1, independently write versions 2 and 2', and
//! lose one update. P0-A2 adds the store lock required around those transactions.
//!
//! After [`AtomicFile::commit`] returns `Ok`, readers observe either the complete
//! old bytes or the complete new bytes, and the new bytes were fsynced before
//! publication. On Unix, the parent-directory sync is best-effort. Windows has
//! no portable directory fsync here, so power loss may lose the rename and make
//! the old complete file reappear; this module promises old-or-new, never torn.

use std::collections::hash_map::RandomState;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CREATE_ATTEMPTS: usize = 16;
const RENAME_ATTEMPTS: usize = 5;
const RENAME_BACKOFF: Duration = Duration::from_millis(40);
pub const STAGING_INFIX: &str = ".tmp-";

static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);
/// Windows can transiently reject simultaneous replacements of one destination
/// even when every source staging path is unique. Serialize only the final
/// publication step within this process. This is not the P0-A2 transaction lock:
/// callbacks and all caller reads/mutations still occur outside it, and each
/// process/crate has its own instance.
static PROCESS_PUBLISH_LOCK: Mutex<()> = Mutex::new(());

fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut value: OsString = path.as_os_str().to_owned();
    value.push(suffix);
    PathBuf::from(value)
}

fn staging_candidate(target: &Path) -> PathBuf {
    let counter = STAGING_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(counter);
    hasher.write_u32(std::process::id());
    let clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let nonce = hasher.finish() ^ clock;
    suffixed(
        target,
        &format!(
            "{STAGING_INFIX}{}-{counter}-{nonce:016x}",
            std::process::id()
        ),
    )
}

fn is_transient_rename_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::PermissionDenied
        || matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

fn rename_with_retries(from: &Path, to: &Path) -> io::Result<()> {
    for attempt in 0..RENAME_ATTEMPTS {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) if attempt + 1 < RENAME_ATTEMPTS && is_transient_rename_error(&error) => {
                std::thread::sleep(RENAME_BACKOFF);
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("rename retry loop always returns")
}

#[cfg(unix)]
fn sync_parent_best_effort(target: &Path) {
    if let Some(parent) = target.parent() {
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
    }
}

#[cfg(not(unix))]
fn sync_parent_best_effort(_target: &Path) {}

/// An owned sibling staging file for a single atomic replacement.
///
/// Dropping without committing, or failing to commit, removes only the exact
/// staging path this value created with `create_new`; it never scans or removes
/// another writer's staging file.
pub struct AtomicFile {
    target: PathBuf,
    staging: Option<PathBuf>,
    file: Option<File>,
}

impl AtomicFile {
    pub fn create(target: &Path) -> io::Result<Self> {
        if let Some(parent) = target.parent().filter(|path| !path.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }

        for _ in 0..CREATE_ATTEMPTS {
            let staging = staging_candidate(target);
            match OpenOptions::new()
                .write(true)
                .read(true)
                .create_new(true)
                .open(&staging)
            {
                Ok(file) => {
                    return Ok(Self {
                        target: target.to_path_buf(),
                        staging: Some(staging),
                        file: Some(file),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }

        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a unique atomic staging file",
        ))
    }

    #[cfg(test)]
    fn staging_path(&self) -> &Path {
        self.staging
            .as_deref()
            .expect("uncommitted AtomicFile has a staging path")
    }

    pub fn commit(self) -> io::Result<()> {
        self.commit_with(|_| Ok(()))
    }

    /// Flush and fsync the replacement, run `before_rename`, then publish it.
    ///
    /// The callback is for semantics that must happen after replacement bytes
    /// are durable but before publication, such as preserving a prior primary
    /// as a backup. It must not rename or remove the supplied staging path.
    pub fn commit_with(
        mut self,
        before_rename: impl FnOnce(&Path) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut file = self.file.take().expect("AtomicFile can only commit once");
        file.flush()?;
        file.sync_all()?;
        drop(file);

        let staging = self
            .staging
            .as_deref()
            .expect("uncommitted AtomicFile has a staging path");
        before_rename(staging)?;
        let _publish_guard = PROCESS_PUBLISH_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        rename_with_retries(staging, &self.target)?;
        self.staging = None;
        sync_parent_best_effort(&self.target);
        Ok(())
    }
}

impl Write for AtomicFile {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.file
            .as_mut()
            .expect("uncommitted AtomicFile has an open file")
            .write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file
            .as_mut()
            .expect("uncommitted AtomicFile has an open file")
            .flush()
    }
}

impl Read for AtomicFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.file
            .as_mut()
            .expect("uncommitted AtomicFile has an open file")
            .read(buffer)
    }
}

impl Seek for AtomicFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file
            .as_mut()
            .expect("uncommitted AtomicFile has an open file")
            .seek(position)
    }
}

impl Drop for AtomicFile {
    fn drop(&mut self) {
        self.file.take();
        if let Some(staging) = self.staging.take() {
            let _ = fs::remove_file(staging);
        }
    }
}

pub fn atomic_write(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut replacement = AtomicFile::create(target)?;
    replacement.write_all(bytes)?;
    replacement.commit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn staging_paths_are_unique_and_drop_cleans_only_the_owner() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("state.json");
        let first = AtomicFile::create(&target).unwrap();
        let second = AtomicFile::create(&target).unwrap();
        let first_path = first.staging_path().to_path_buf();
        let second_path = second.staging_path().to_path_buf();

        assert_ne!(first_path, second_path);
        drop(first);
        assert!(!first_path.exists());
        assert!(second_path.exists());
        drop(second);
        assert!(!second_path.exists());
    }

    #[test]
    fn failed_rename_cleans_only_its_own_staging() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("state.json");
        fs::create_dir(&target).unwrap();
        let mut blocked = AtomicFile::create(&target).unwrap();
        blocked.write_all(b"replacement").unwrap();
        let blocked_path = blocked.staging_path().to_path_buf();
        let other = AtomicFile::create(&target).unwrap();
        let other_path = other.staging_path().to_path_buf();

        assert!(blocked.commit().is_err());
        assert!(!blocked_path.exists());
        assert!(other_path.exists());
        assert!(target.is_dir());
    }

    #[test]
    fn concurrent_writes_always_publish_a_complete_payload() {
        let temp = tempfile::tempdir().unwrap();
        let target = Arc::new(temp.path().join("state.json"));
        let start = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|writer| {
                let target = target.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    for iteration in 0..100 {
                        let payload = format!(r#"{{"writer":{writer},"iteration":{iteration}}}"#);
                        atomic_write(target.as_ref(), payload.as_bytes()).unwrap();
                    }
                })
            })
            .collect();

        for thread in threads {
            thread.join().unwrap();
        }
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(target.as_ref()).unwrap()).unwrap();
        assert!(value.get("writer").is_some());
        assert!(value.get("iteration").is_some());
        assert!(fs::read_dir(temp.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(STAGING_INFIX)
        }));
    }
}
