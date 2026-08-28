//! Cross-process transaction locks for short read-modify-write operations.
//!
//! Every caller follows one order: existing process-local mutex first, then
//! these OS locks in canonical sorted order, then read/mutate/atomic publish.
//! Do not acquire a local mutex, perform network/archive/directory-scan work,
//! or invoke UI/plugin callbacks while an OS guard is alive.
//!
//! Lock files are persistent coordination objects. Their bytes and names do
//! not indicate ownership, they are never deleted as "stale", and process
//! death releases the kernel lock automatically when its handle closes.

use fs4::{FileExt, TryLockError};
use std::collections::HashSet;
use std::ffi::OsString;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const LOCK_SUFFIX: &str = ".kalpa.lock";

#[derive(Clone, Copy)]
pub(crate) struct LockOptions<'a> {
    pub timeout: Duration,
    pub cancel: Option<&'a AtomicBool>,
}

impl Default for LockOptions<'_> {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            cancel: None,
        }
    }
}

#[derive(Debug)]
pub(crate) enum LockError {
    Timeout { path: PathBuf, waited: Duration },
    Cancelled { path: PathBuf },
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout { path, waited } => write!(
                f,
                "Another Kalpa window is still saving {}; try again in a moment (waited {} ms).",
                display_name(path),
                waited.as_millis()
            ),
            Self::Cancelled { path } => write!(f, "Saving {} was cancelled.", display_name(path)),
            Self::Io { path, source } => {
                write!(f, "Could not lock {}: {source}", display_name(path))
            }
        }
    }
}

impl std::error::Error for LockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Timeout { .. } | Self::Cancelled { .. } => None,
        }
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[derive(Clone, Debug)]
pub(crate) struct LockKey {
    target: PathBuf,
    lock_path: PathBuf,
    order_key: String,
}

impl PartialEq for LockKey {
    fn eq(&self, other: &Self) -> bool {
        self.order_key == other.order_key
    }
}

impl Eq for LockKey {}

impl LockKey {
    pub(crate) fn for_path(path: impl AsRef<Path>) -> Result<Self, LockError> {
        let requested = path.as_ref();
        let absolute = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|source| LockError::Io {
                    path: requested.to_path_buf(),
                    source,
                })?
                .join(requested)
        };
        // Preserve `..` until the nearest existing ancestor is canonicalized.
        // Lexically collapsing it first is incorrect when a preceding component
        // is a symlink (the filesystem resolves `link/..` from the link target).
        let target = lexical_normalize(&canonicalize_with_missing_tail(&absolute).map_err(
            |source| LockError::Io {
                path: requested.to_path_buf(),
                source,
            },
        )?);
        let parent = target.parent().ok_or_else(|| LockError::Io {
            path: target.clone(),
            source: io::Error::new(
                io::ErrorKind::InvalidInput,
                "target has no parent directory",
            ),
        })?;
        let name = target.file_name().ok_or_else(|| LockError::Io {
            path: target.clone(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "target has no file name"),
        })?;
        let mut lock_name = OsString::from(".");
        lock_name.push(name);
        lock_name.push(LOCK_SUFFIX);
        let lock_path = parent.join(lock_name);
        let mut order_key = lock_path.to_string_lossy().into_owned();
        #[cfg(windows)]
        {
            order_key = order_key.to_lowercase();
        }
        Ok(Self {
            target,
            lock_path,
            order_key,
        })
    }

    #[cfg(test)]
    fn lock_path(&self) -> &Path {
        &self.lock_path
    }
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(component.as_os_str());
                }
            }
            _ => out.push(component.as_os_str()),
        }
    }
    out
}

fn canonicalize_with_missing_tail(path: &Path) -> io::Result<PathBuf> {
    let mut existing = path;
    let mut tail: Vec<OsString> = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no existing ancestor for {}", path.display()),
            ));
        };
        tail.push(name.to_owned());
        existing = existing.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no existing ancestor for {}", path.display()),
            )
        })?;
    }
    let mut canonical = dunce::canonicalize(existing)?;
    for component in tail.into_iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}

struct ProcessLockState {
    held: Mutex<HashSet<String>>,
    released: Condvar,
}

fn process_lock_state() -> &'static ProcessLockState {
    static STATE: OnceLock<ProcessLockState> = OnceLock::new();
    STATE.get_or_init(|| ProcessLockState {
        held: Mutex::new(HashSet::new()),
        released: Condvar::new(),
    })
}

struct ProcessLockGuard {
    keys: Vec<String>,
}

impl Drop for ProcessLockGuard {
    fn drop(&mut self) {
        let state = process_lock_state();
        let mut held = state.held.lock().unwrap_or_else(|error| error.into_inner());
        for key in &self.keys {
            held.remove(key);
        }
        state.released.notify_all();
    }
}

struct OsLockGuard {
    file: File,
}

impl Drop for OsLockGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

pub(crate) struct LockGuard {
    // Drop the kernel lock before publishing this process's release. Keeping
    // the fields in this order preserves that handoff ordering.
    _os: OsLockGuard,
    _process: ProcessLockGuard,
}

#[allow(dead_code)]
pub(crate) struct LockSet {
    _guards: Vec<OsLockGuard>,
    _process: ProcessLockGuard,
}

pub(crate) fn acquire(
    target: impl AsRef<Path>,
    options: LockOptions<'_>,
) -> Result<LockGuard, LockError> {
    let key = LockKey::for_path(target)?;
    let deadline = Instant::now() + options.timeout;
    let process = acquire_process_keys(
        std::slice::from_ref(&key),
        deadline,
        options.timeout,
        options.cancel,
    )?;
    let os = acquire_key(&key, deadline, options.timeout, options.cancel)?;
    Ok(LockGuard {
        _os: os,
        _process: process,
    })
}

#[allow(dead_code)]
pub(crate) fn acquire_many<P: AsRef<Path>>(
    targets: &[P],
    options: LockOptions<'_>,
) -> Result<LockSet, LockError> {
    let mut keys = targets
        .iter()
        .map(LockKey::for_path)
        .collect::<Result<Vec<_>, _>>()?;
    keys.sort_by(|a, b| a.order_key.cmp(&b.order_key));
    keys.dedup_by(|a, b| a.order_key == b.order_key);

    let deadline = Instant::now() + options.timeout;
    let process = acquire_process_keys(&keys, deadline, options.timeout, options.cancel)?;
    let mut guards = Vec::with_capacity(keys.len());
    for key in &keys {
        guards.push(acquire_key(key, deadline, options.timeout, options.cancel)?);
    }
    Ok(LockSet {
        _guards: guards,
        _process: process,
    })
}

fn acquire_process_keys(
    keys: &[LockKey],
    deadline: Instant,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<ProcessLockGuard, LockError> {
    if keys.is_empty() {
        return Ok(ProcessLockGuard { keys: Vec::new() });
    }
    let state = process_lock_state();
    let mut held = state.held.lock().unwrap_or_else(|error| error.into_inner());
    loop {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err(LockError::Cancelled {
                path: keys[0].target.clone(),
            });
        }
        if keys.iter().all(|key| !held.contains(&key.order_key)) {
            let claimed = keys
                .iter()
                .map(|key| key.order_key.clone())
                .collect::<Vec<_>>();
            held.extend(claimed.iter().cloned());
            return Ok(ProcessLockGuard { keys: claimed });
        }
        let now = Instant::now();
        if now >= deadline {
            let blocked = keys
                .iter()
                .find(|key| held.contains(&key.order_key))
                .unwrap_or(&keys[0]);
            return Err(LockError::Timeout {
                path: blocked.target.clone(),
                waited: timeout,
            });
        }
        let wait = POLL_INTERVAL.min(deadline.saturating_duration_since(now));
        let result = state.released.wait_timeout(held, wait);
        held = match result {
            Ok((guard, _)) => guard,
            Err(error) => error.into_inner().0,
        };
    }
}

fn acquire_key(
    key: &LockKey,
    deadline: Instant,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<OsLockGuard, LockError> {
    // A fresh install may resolve a store below an app-data directory that the
    // framework has not created yet. The transaction owns creation of the
    // sibling coordination object, so it must also make that parent available.
    if let Some(parent) = key.lock_path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| LockError::Io {
            path: key.target.clone(),
            source,
        })?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&key.lock_path)
        .map_err(|source| LockError::Io {
            path: key.target.clone(),
            source,
        })?;

    loop {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err(LockError::Cancelled {
                path: key.target.clone(),
            });
        }
        match FileExt::try_lock(&file) {
            Ok(()) => return Ok(OsLockGuard { file }),
            Err(TryLockError::Error(source)) => {
                return Err(LockError::Io {
                    path: key.target.clone(),
                    source,
                })
            }
            Err(TryLockError::WouldBlock) => {
                let now = Instant::now();
                if now >= deadline {
                    return Err(LockError::Timeout {
                        path: key.target.clone(),
                        waited: timeout,
                    });
                }
                std::thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(now)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::sync::Arc;

    const HELPER_TEST: &str = "transaction_lock::tests::process_helper";

    fn spawn_helper(mode: &str, target: &Path, extra: Option<&Path>) -> Child {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--ignored", "--exact", HELPER_TEST, "--nocapture"])
            .env("KALPA_LOCK_TEST_MODE", mode)
            .env("KALPA_LOCK_TEST_TARGET", target)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        if let Some(extra) = extra {
            command.env("KALPA_LOCK_TEST_EXTRA", extra);
        }
        command.spawn().unwrap()
    }

    fn wait_for(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            assert!(Instant::now() < deadline, "timed out waiting for {path:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[ignore]
    fn process_helper() {
        let mode = std::env::var("KALPA_LOCK_TEST_MODE").expect("helper mode");
        let target = PathBuf::from(std::env::var_os("KALPA_LOCK_TEST_TARGET").unwrap());
        match mode.as_str() {
            "hold" => {
                let ready = PathBuf::from(std::env::var_os("KALPA_LOCK_TEST_EXTRA").unwrap());
                let _guard = acquire(&target, LockOptions::default()).unwrap();
                std::fs::write(ready, b"ready").unwrap();
                std::thread::sleep(Duration::from_secs(30));
            }
            "increment" => {
                for _ in 0..100 {
                    let _guard = acquire(
                        &target,
                        LockOptions {
                            // Test two complete 100-write processes under heavy
                            // Windows CI scheduling without mistaking starvation
                            // for an unbounded wait. Production remains 2s.
                            timeout: Duration::from_secs(10),
                            cancel: None,
                        },
                    )
                    .unwrap();
                    let value = std::fs::read_to_string(&target)
                        .unwrap()
                        .trim()
                        .parse::<u32>()
                        .unwrap();
                    crate::atomic_file::atomic_write(&target, (value + 1).to_string().as_bytes())
                        .unwrap();
                }
            }
            "many" => {
                let other = PathBuf::from(std::env::var_os("KALPA_LOCK_TEST_EXTRA").unwrap());
                for _ in 0..100 {
                    let _guards = acquire_many(
                        &[&target, &other],
                        LockOptions {
                            timeout: Duration::from_secs(5),
                            cancel: None,
                        },
                    )
                    .unwrap();
                    std::thread::yield_now();
                }
            }
            unknown => panic!("unknown helper mode: {unknown}"),
        }
    }

    #[test]
    fn transaction_lock_api_is_available_to_both_crates() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = acquire(dir.path().join("settings.json"), LockOptions::default()).unwrap();
    }

    #[test]
    fn missing_target_aliases_share_one_identity() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let direct = nested.join("settings.json");
        let dotted = nested
            .join(".")
            .join("child")
            .join("..")
            .join("settings.json");
        let a = LockKey::for_path(&direct).unwrap();
        let b = LockKey::for_path(&dotted).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.lock_path(), nested.join(".settings.json.kalpa.lock"));
    }

    #[test]
    fn fresh_store_creates_missing_parent_for_sibling_lock() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("new-app-data").join("settings.json");
        let key = LockKey::for_path(&target).unwrap();

        let _guard = acquire(&target, LockOptions::default()).unwrap();

        assert!(target.parent().unwrap().is_dir());
        assert!(key.lock_path().is_file());
    }

    #[test]
    fn relative_and_absolute_paths_share_one_identity() {
        let absolute = std::env::current_dir()
            .unwrap()
            .join("kalpa-transaction-lock-alias-test.json");
        let relative = PathBuf::from("kalpa-transaction-lock-alias-test.json");
        assert_eq!(
            LockKey::for_path(absolute).unwrap(),
            LockKey::for_path(relative).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_parent_uses_real_parent_identity() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        let alias = dir.path().join("alias");
        std::fs::create_dir(&real).unwrap();
        symlink(&real, &alias).unwrap();
        assert_eq!(
            LockKey::for_path(real.join("missing.json")).unwrap(),
            LockKey::for_path(alias.join("missing.json")).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn parent_after_symlink_uses_filesystem_identity() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let real_parent = dir.path().join("real-parent");
        let real_child = real_parent.join("child");
        std::fs::create_dir_all(&real_child).unwrap();
        let alias = dir.path().join("alias");
        symlink(&real_child, &alias).unwrap();

        let through_alias = LockKey::for_path(alias.join("..").join("missing.json")).unwrap();
        let through_real = LockKey::for_path(real_parent.join("missing.json")).unwrap();
        assert_eq!(through_alias, through_real);
    }

    #[cfg(windows)]
    #[test]
    fn windows_case_and_verbatim_aliases_share_one_identity() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Missing.json");
        let lower = PathBuf::from(target.to_string_lossy().to_lowercase());
        let verbatim = PathBuf::from(format!(r"\\?\{}", target.display()));
        let expected = LockKey::for_path(&target).unwrap();
        assert_eq!(expected, LockKey::for_path(lower).unwrap());
        assert_eq!(expected, LockKey::for_path(verbatim).unwrap());
    }

    #[test]
    fn contention_times_out_with_actionable_error() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("kalpa.json");
        let _owner = acquire(&target, LockOptions::default()).unwrap();
        let started = Instant::now();
        let err = acquire(
            &target,
            LockOptions {
                timeout: Duration::from_millis(100),
                cancel: None,
            },
        )
        .err()
        .expect("contender should time out");
        assert!(matches!(err, LockError::Timeout { .. }));
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(err.to_string().contains("kalpa.json"));
    }

    #[test]
    fn two_threads_read_modify_write_has_no_lost_updates() {
        let dir = tempfile::tempdir().unwrap();
        let target = Arc::new(dir.path().join("counter.txt"));
        std::fs::write(target.as_ref(), b"0").unwrap();
        let start = Arc::new(std::sync::Barrier::new(2));
        let workers = (0..2)
            .map(|_| {
                let target = Arc::clone(&target);
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    for _ in 0..100 {
                        let _guard = acquire(target.as_ref(), LockOptions::default()).unwrap();
                        let value = std::fs::read_to_string(target.as_ref())
                            .unwrap()
                            .trim()
                            .parse::<u32>()
                            .unwrap();
                        crate::atomic_file::atomic_write(
                            target.as_ref(),
                            (value + 1).to_string().as_bytes(),
                        )
                        .unwrap();
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(std::fs::read_to_string(target.as_ref()).unwrap(), "200");
    }

    #[test]
    fn another_process_blocks_then_owner_kill_releases_lock() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("kalpa.json");
        let ready = dir.path().join("ready");
        let mut owner = spawn_helper("hold", &target, Some(&ready));
        wait_for(&ready);

        let error = acquire(
            &target,
            LockOptions {
                timeout: Duration::from_millis(150),
                cancel: None,
            },
        )
        .err()
        .expect("live owner must block contender");
        assert!(matches!(error, LockError::Timeout { .. }));

        owner.kill().unwrap();
        owner.wait().unwrap();
        acquire(
            &target,
            LockOptions {
                timeout: Duration::from_secs(1),
                cancel: None,
            },
        )
        .expect("OS must release the lock when its owner dies");
        assert!(LockKey::for_path(&target).unwrap().lock_path().exists());
    }

    #[test]
    fn two_process_read_modify_write_has_no_lost_updates() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("counter.txt");
        std::fs::write(&target, b"0").unwrap();
        let mut a = spawn_helper("increment", &target, None);
        let mut b = spawn_helper("increment", &target, None);
        assert!(a.wait().unwrap().success());
        assert!(b.wait().unwrap().success());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "200");
    }

    #[test]
    fn opposite_multi_lock_orders_do_not_deadlock_across_processes() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.json");
        let b = dir.path().join("b.json");
        let mut left = spawn_helper("many", &a, Some(&b));
        let mut right = spawn_helper("many", &b, Some(&a));
        assert!(left.wait().unwrap().success());
        assert!(right.wait().unwrap().success());
    }

    #[test]
    fn cancellation_releases_partially_acquired_set() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.json");
        let b = dir.path().join("b.json");
        let _b_owner = acquire(&b, LockOptions::default()).unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let trigger = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(75));
            flag.store(true, Ordering::Relaxed);
        });
        let result = acquire_many(
            &[&a, &b],
            LockOptions {
                timeout: Duration::from_secs(1),
                cancel: Some(&cancelled),
            },
        );
        trigger.join().unwrap();
        assert!(matches!(result, Err(LockError::Cancelled { .. })));
        acquire(
            &a,
            LockOptions {
                timeout: Duration::from_millis(100),
                cancel: None,
            },
        )
        .expect("partial acquisition must release a");
    }

    #[test]
    fn opposite_requested_orders_are_canonicalized() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.json");
        let b = dir.path().join("b.json");
        let left = [
            LockKey::for_path(&a).unwrap(),
            LockKey::for_path(&b).unwrap(),
        ];
        let right = [
            LockKey::for_path(&b).unwrap(),
            LockKey::for_path(&a).unwrap(),
        ];
        let sorted = |mut keys: Vec<LockKey>| {
            keys.sort_by(|x, y| x.order_key.cmp(&y.order_key));
            keys.into_iter()
                .map(|key| key.order_key)
                .collect::<Vec<_>>()
        };
        assert_eq!(sorted(left.to_vec()), sorted(right.to_vec()));
    }
}
