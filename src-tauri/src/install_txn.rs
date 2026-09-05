//! Crash-safe publication of staged addon folders.

use crate::atomic_file;
use crate::transaction_lock::{self, LockOptions};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STAGING_DIR: &str = ".kalpa-staging";
const HASHES_DIR: &str = ".kalpa-hashes";
/// Marks a folder whose hash baseline this transaction promoted when there was
/// no previous one, so a rollback can tell its own work from the user's.
const ABSENT_BASELINE_SUFFIX: &str = ".absent";
const RENAME_ATTEMPTS: usize = 5;
const RENAME_BACKOFF: Duration = Duration::from_millis(40);
const CREATE_ATTEMPTS: usize = 16;
static TXN_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Preparing,
    Staged,
    Swapped,
    Promoted,
}

#[derive(Debug, Deserialize, Serialize)]
struct Journal {
    phase: Phase,
    folders: Vec<String>,
    pre_existing: Vec<String>,
    #[serde(default)]
    hash_folders: Vec<String>,
    #[serde(default)]
    root_files: Vec<String>,
    #[serde(default)]
    pre_existing_root_files: Vec<String>,
}

pub(crate) struct InstallTransaction {
    addons_dir: PathBuf,
    root: PathBuf,
    _lock: transaction_lock::LockGuard,
    finished: bool,
    swap_started: bool,
}

/// Holds the AddOns transaction lock after completing startup recovery.
///
/// Callers that mutate live addon folders outside an install transaction keep
/// this guard alive for the entire mutation so they cannot race publication or
/// recovery in another process.
pub(crate) struct RecoveryGuard {
    _lock: transaction_lock::LockGuard,
}

/// Holds transaction locks for a multi-root operation after recovering every
/// root while the full, deterministically ordered lock set is held.
#[allow(dead_code)] // Used by Tauri; this module is also compiled into the Slint binary.
pub(crate) struct MultiRecoveryGuard {
    _locks: transaction_lock::LockSet,
}

impl InstallTransaction {
    pub(crate) fn begin(addons_dir: &Path, cancel: Option<&AtomicBool>) -> Result<Self, String> {
        let lock = transaction_lock::acquire(
            addons_dir,
            LockOptions {
                timeout: transaction_lock::DEFAULT_TIMEOUT,
                cancel,
            },
        )
        .map_err(lock_error)?;
        recover_staging_locked(addons_dir)?;
        let staging = addons_dir.join(STAGING_DIR);
        fs::create_dir_all(&staging)
            .map_err(|e| format!("Failed to create installer staging directory: {e}"))?;
        let root = create_transaction_root(&staging)?;
        let prepared = ["stage", "tombstone", "hashes", "hash-tombstone"]
            .into_iter()
            .try_for_each(|child| fs::create_dir(root.join(child)))
            .map_err(|e| format!("Failed to prepare installer transaction: {e}"))
            .and_then(|()| {
                write_journal(
                    &root,
                    &Journal {
                        phase: Phase::Preparing,
                        folders: Vec::new(),
                        pre_existing: Vec::new(),
                        hash_folders: Vec::new(),
                        root_files: Vec::new(),
                        pre_existing_root_files: Vec::new(),
                    },
                )
            });
        if let Err(error) = prepared {
            let _ = fs::remove_dir_all(&root);
            cleanup_empty_staging(addons_dir);
            return Err(error);
        }
        Ok(Self {
            addons_dir: addons_dir.to_path_buf(),
            root,
            _lock: lock,
            finished: false,
            swap_started: false,
        })
    }

    pub(crate) fn stage_dir(&self) -> PathBuf {
        self.root.join("stage")
    }

    #[allow(dead_code)]
    pub(crate) fn commit(
        self,
        folders: &[String],
        manifests: &[(String, Vec<u8>)],
    ) -> Result<Vec<String>, String> {
        self.commit_with_root_files(folders, &[], manifests)
    }

    pub(crate) fn commit_with_root_files(
        mut self,
        folders: &[String],
        root_files: &[String],
        manifests: &[(String, Vec<u8>)],
    ) -> Result<Vec<String>, String> {
        // Debug-only: a tiny addon was observed spending ~3s here, which the
        // single "commit" label could not explain.
        let phase = crate::phase_timer::PhaseTimer::start("commit");

        let mut folders = folders.to_vec();
        folders.sort();
        folders.dedup();
        let mut root_files = root_files.to_vec();
        root_files.sort();
        root_files.dedup();
        for relative in &root_files {
            crate::installer::validate_top_folder_name(relative).map_err(|error| {
                format!("Invalid AddOns-root file in install transaction: {error}")
            })?;
            if !self.root.join("stage").join(relative).is_file() {
                return Err(format!(
                    "Staged AddOns-root file {relative:?} is missing from this install."
                ));
            }
            let live = self.addons_dir.join(relative);
            if live.exists() && !live.is_file() {
                return Err(format!(
                    "Cannot replace AddOns-root file {relative:?} because that path is not a file."
                ));
            }
        }
        let pre_existing: Vec<String> = folders
            .iter()
            .filter(|folder| self.addons_dir.join(folder).exists())
            .cloned()
            .collect();
        let pre_existing_root_files: Vec<String> = root_files
            .iter()
            .filter(|relative| self.addons_dir.join(relative).is_file())
            .cloned()
            .collect();
        let mut hash_folders = Vec::with_capacity(manifests.len());
        for (folder, bytes) in manifests {
            if !folders.contains(folder) {
                return Err(format!(
                    "Hash baseline targets folder {folder}, which is not part of this install."
                ));
            }
            atomic_file::atomic_write(
                &self.root.join("hashes").join(format!("{folder}.json")),
                bytes,
            )
            .map_err(|e| format!("Failed to stage hash baseline for {folder}: {e}"))?;
            hash_folders.push(folder.clone());
        }
        hash_folders.sort();
        hash_folders.dedup();
        let mut journal = Journal {
            phase: Phase::Staged,
            folders: folders.clone(),
            pre_existing,
            hash_folders,
            root_files: root_files.clone(),
            pre_existing_root_files,
        };
        write_journal(&self.root, &journal)?;
        phase.mark("stage journal");

        let mut landed = Vec::new();
        for folder in &folders {
            let live = self.addons_dir.join(folder);
            let stage = self.root.join("stage").join(folder);
            let tombstone = self.root.join("tombstone").join(folder);
            let had_live = live.exists();
            self.swap_started = true;
            if had_live {
                if let Err(error) = rename_with_retries(&live, &tombstone) {
                    let rollback = rollback_landed(&self, &landed);
                    let recovered = rollback.is_ok();
                    let message = swap_error(folder, &error, rollback);
                    if recovered {
                        let _ = fs::remove_dir_all(&self.root);
                        cleanup_empty_staging(&self.addons_dir);
                        self.finished = true;
                    }
                    return Err(message);
                }
            }
            if let Err(error) = rename_with_retries(&stage, &live) {
                let restore = if had_live {
                    rename_with_retries(&tombstone, &live)
                        .map_err(|e| format!("failed to restore {folder}: {e}"))
                } else {
                    Ok(())
                };
                let rollback = rollback_landed(&self, &landed);
                let recovered = restore.is_ok() && rollback.is_ok();
                let recovery = restore.and(rollback);
                let message = swap_error(folder, &error, recovery);
                if recovered {
                    let _ = fs::remove_dir_all(&self.root);
                    cleanup_empty_staging(&self.addons_dir);
                    self.finished = true;
                }
                return Err(message);
            }
            landed.push(folder.clone());
        }

        let mut landed_root_files = Vec::new();
        for relative in &root_files {
            let live = self.addons_dir.join(relative);
            let stage = self.root.join("stage").join(relative);
            let tombstone = self.root.join("tombstone").join(relative);
            let had_live = live.exists();
            self.swap_started = true;
            if had_live {
                if let Err(error) = rename_with_retries(&live, &tombstone) {
                    let rollback = rollback_root_files(&self, &landed_root_files)
                        .and_then(|()| rollback_landed(&self, &landed));
                    let recovered = rollback.is_ok();
                    let message = root_swap_error(relative, &error, rollback);
                    if recovered {
                        let _ = fs::remove_dir_all(&self.root);
                        cleanup_empty_staging(&self.addons_dir);
                        self.finished = true;
                    }
                    return Err(message);
                }
            }
            if let Err(error) = rename_with_retries(&stage, &live) {
                let restore = if had_live {
                    rename_with_retries(&tombstone, &live)
                        .map_err(|e| format!("failed to restore {relative}: {e}"))
                } else {
                    Ok(())
                };
                let rollback = restore
                    .and_then(|()| rollback_root_files(&self, &landed_root_files))
                    .and_then(|()| rollback_landed(&self, &landed));
                let recovered = rollback.is_ok();
                let message = root_swap_error(relative, &error, rollback);
                if recovered {
                    let _ = fs::remove_dir_all(&self.root);
                    cleanup_empty_staging(&self.addons_dir);
                    self.finished = true;
                }
                return Err(message);
            }
            landed_root_files.push(relative.clone());
        }

        journal.phase = Phase::Swapped;
        if let Err(error) = write_journal(&self.root, &journal) {
            let rollback = rollback_root_files(&self, &landed_root_files)
                .and_then(|()| rollback_landed(&self, &landed));
            if rollback.is_ok() {
                let _ = fs::remove_dir_all(&self.root);
                cleanup_empty_staging(&self.addons_dir);
                self.finished = true;
            }
            let rollback = rollback
                .err()
                .map(|e| format!(" Rollback also failed: {e}"))
                .unwrap_or_default();
            return Err(format!(
                "Failed to persist the completed addon-folder swap: {error}.{rollback}"
            ));
        }
        phase.mark("renames");
        if let Err(error) = promote_hashes(&self.addons_dir, &self.root, &journal.hash_folders) {
            let rollback = rollback_hashes(&self.addons_dir, &self.root, &journal.hash_folders)
                .and_then(|()| rollback_root_files(&self, &landed_root_files))
                .and_then(|()| rollback_landed(&self, &landed));
            if rollback.is_ok() {
                let _ = fs::remove_dir_all(&self.root);
                cleanup_empty_staging(&self.addons_dir);
                self.finished = true;
            }
            let rollback = rollback
                .err()
                .map(|e| format!(" Rollback also failed: {e}"))
                .unwrap_or_default();
            return Err(format!(
                "Failed to publish addon hash baselines: {error}.{rollback}"
            ));
        }
        phase.mark("promote hashes");
        journal.phase = Phase::Promoted;
        finish_committed_transaction(&self.addons_dir, &self.root, &journal);
        phase.mark("cleanup (tombstone delete)");
        self.finished = true;
        Ok(folders)
    }
}

fn create_transaction_root(staging: &Path) -> Result<PathBuf, String> {
    for _ in 0..CREATE_ATTEMPTS {
        let counter = TXN_COUNTER.fetch_add(1, Ordering::Relaxed);
        let clock = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(counter);
        hasher.write_u32(std::process::id());
        let nonce = hasher.finish() ^ clock;
        let root = staging.join(format!("{}-{counter}-{nonce:016x}", std::process::id()));
        match fs::create_dir(&root) {
            Ok(()) => return Ok(root),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Failed to create installer transaction directory: {error}"
                ));
            }
        }
    }
    Err("Failed to allocate a unique installer transaction directory.".to_string())
}

impl Drop for InstallTransaction {
    fn drop(&mut self) {
        // Once publication starts, the journal and tombstones are the only
        // durable recovery record. Leave them in place for the next recovery
        // pass if a later journal/hash write fails.
        if !self.finished && !self.swap_started {
            let _ = fs::remove_dir_all(&self.root);
            cleanup_empty_staging(&self.addons_dir);
        }
    }
}

#[cfg(test)]
pub(crate) fn recover_staging(addons_dir: &Path) -> Result<(), String> {
    let _guard = lock_and_recover(addons_dir)?;
    Ok(())
}

pub(crate) fn lock_and_recover(addons_dir: &Path) -> Result<RecoveryGuard, String> {
    let lock = transaction_lock::acquire(addons_dir, LockOptions::default()).map_err(lock_error)?;
    recover_staging_locked(addons_dir)?;
    Ok(RecoveryGuard { _lock: lock })
}

#[allow(dead_code)] // Used by Tauri; this module is also compiled into the Slint binary.
pub(crate) fn lock_many_and_recover(addons_dirs: &[&Path]) -> Result<MultiRecoveryGuard, String> {
    let locks =
        transaction_lock::acquire_many(addons_dirs, LockOptions::default()).map_err(lock_error)?;
    for addons_dir in addons_dirs {
        recover_staging_locked(addons_dir)?;
    }
    Ok(MultiRecoveryGuard { _locks: locks })
}

fn lock_error(error: transaction_lock::LockError) -> String {
    match error {
        transaction_lock::LockError::Cancelled { .. } => crate::installer::CANCELLED.to_string(),
        transaction_lock::LockError::Timeout { .. } =>
            "Another Kalpa process is installing or updating addons. Close it or wait for it to finish, then try again.".to_string(),
        other => other.to_string(),
    }
}

fn recover_staging_locked(addons_dir: &Path) -> Result<(), String> {
    let staging = addons_dir.join(STAGING_DIR);
    let entries = match fs::read_dir(&staging) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "Failed to inspect abandoned installer transactions: {error}"
            ));
        }
    };
    for entry in entries {
        let entry =
            entry.map_err(|e| format!("Failed to inspect abandoned installer transaction: {e}"))?;
        let root = entry.path();
        let metadata = fs::symlink_metadata(&root)
            .map_err(|e| format!("Failed to inspect abandoned installer transaction: {e}"))?;
        if !metadata.file_type().is_dir() {
            return Err(format!(
                "Unexpected entry in installer staging directory: {}",
                root.display()
            ));
        }
        let journal = fs::read(root.join("journal.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Journal>(&bytes).ok());
        match journal {
            Some(journal) if journal.phase == Phase::Swapped => {
                validate_journal(&journal)?;
                // The caller writes install metadata only after commit returns.
                // A crash or hash-publication failure in this phase therefore
                // must restore the old folders *and* old hash baselines rather
                // than silently completing an install the caller saw as failed.
                rollback_hashes(addons_dir, &root, &journal.hash_folders)?;
                rollback_journal(addons_dir, &root, &journal)?;
            }
            Some(journal) if journal.phase == Phase::Staged => {
                validate_journal(&journal)?;
                rollback_journal(addons_dir, &root, &journal)?;
            }
            Some(journal) if journal.phase == Phase::Promoted => {
                validate_journal(&journal)?;
            }
            _ => rollback_unjournaled_tombstones(addons_dir, &root)?,
        }
        fs::remove_dir_all(&root)
            .map_err(|e| format!("Failed to clean abandoned installer transaction: {e}"))?;
    }
    cleanup_empty_staging(addons_dir);
    Ok(())
}

fn rollback_unjournaled_tombstones(addons_dir: &Path, root: &Path) -> Result<(), String> {
    let tombstone_dir = root.join("tombstone");
    let entries = match fs::read_dir(&tombstone_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "Failed to inspect installer recovery tombstones: {error}"
            ));
        }
    };
    let mut folders = Vec::new();
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("Failed to inspect installer recovery tombstone: {error}"))?;
        let folder = entry
            .file_name()
            .into_string()
            .map_err(|_| "Invalid non-Unicode installer recovery tombstone name.".to_string())?;
        crate::installer::validate_top_folder_name(&folder)
            .map_err(|error| format!("Invalid folder in installer recovery tombstone: {error}"))?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
            format!("Failed to inspect installer recovery tombstone {folder}: {error}")
        })?;
        if !metadata.file_type().is_dir() {
            return Err(format!(
                "Unexpected installer recovery tombstone: {}",
                entry.path().display()
            ));
        }
        folders.push(folder);
    }
    folders.sort();
    folders.reverse();
    for folder in folders {
        let live = addons_dir.join(&folder);
        let tombstone = tombstone_dir.join(&folder);
        if live.exists() {
            fs::remove_dir_all(&live)
                .map_err(|error| format!("Failed to remove incomplete {folder}: {error}"))?;
        }
        rename_with_retries(&tombstone, &live)
            .map_err(|error| format!("Failed to restore {folder}: {error}"))?;
    }
    Ok(())
}

fn validate_journal(journal: &Journal) -> Result<(), String> {
    for folder in journal
        .folders
        .iter()
        .chain(&journal.pre_existing)
        .chain(&journal.hash_folders)
    {
        crate::installer::validate_top_folder_name(folder).map_err(|error| {
            format!("Invalid folder in abandoned installer transaction: {error}")
        })?;
    }
    for relative in journal
        .root_files
        .iter()
        .chain(&journal.pre_existing_root_files)
    {
        crate::installer::validate_top_folder_name(relative).map_err(|error| {
            format!("Invalid root file in abandoned installer transaction: {error}")
        })?;
    }
    Ok(())
}

fn rollback_journal(addons_dir: &Path, root: &Path, journal: &Journal) -> Result<(), String> {
    let mut first_error = None;
    for relative in journal.root_files.iter().rev() {
        let live = addons_dir.join(relative);
        let stage = root.join("stage").join(relative);
        let tombstone = root.join("tombstone").join(relative);
        if !stage.exists() && live.is_file() {
            if journal.pre_existing_root_files.contains(relative) && tombstone.is_file() {
                if let Err(error) = fs::remove_file(&live) {
                    first_error.get_or_insert_with(|| {
                        format!("Failed to remove incomplete root file {relative}: {error}")
                    });
                    continue;
                }
                if let Err(error) = rename_with_retries(&tombstone, &live) {
                    first_error.get_or_insert_with(|| {
                        format!("Failed to restore root file {relative}: {error}")
                    });
                }
            } else if !journal.pre_existing_root_files.contains(relative) {
                if let Err(error) = fs::remove_file(&live) {
                    first_error.get_or_insert_with(|| {
                        format!("Failed to remove incomplete root file {relative}: {error}")
                    });
                }
            }
        } else if !live.exists() && tombstone.is_file() {
            if let Err(error) = rename_with_retries(&tombstone, &live) {
                first_error.get_or_insert_with(|| {
                    format!("Failed to restore root file {relative}: {error}")
                });
            }
        }
    }
    for folder in journal.folders.iter().rev() {
        let live = addons_dir.join(folder);
        let stage = root.join("stage").join(folder);
        let tombstone = root.join("tombstone").join(folder);
        if !stage.exists() && live.exists() {
            if journal.pre_existing.contains(folder) && tombstone.exists() {
                if let Err(error) = fs::remove_dir_all(&live) {
                    first_error.get_or_insert_with(|| {
                        format!("Failed to remove incomplete {folder}: {error}")
                    });
                    continue;
                }
                if let Err(error) = rename_with_retries(&tombstone, &live) {
                    first_error
                        .get_or_insert_with(|| format!("Failed to restore {folder}: {error}"));
                }
            } else if !journal.pre_existing.contains(folder) {
                if let Err(error) = fs::remove_dir_all(&live) {
                    first_error.get_or_insert_with(|| {
                        format!("Failed to remove incomplete {folder}: {error}")
                    });
                }
            }
        } else if !live.exists() && tombstone.exists() {
            if let Err(error) = rename_with_retries(&tombstone, &live) {
                first_error.get_or_insert_with(|| format!("Failed to restore {folder}: {error}"));
            }
        }
    }
    if let Some(error) = first_error {
        Err(error)
    } else {
        Ok(())
    }
}

fn rollback_landed(txn: &InstallTransaction, landed: &[String]) -> Result<(), String> {
    let mut first_error = None;
    for folder in landed.iter().rev() {
        let live = txn.addons_dir.join(folder);
        let tombstone = txn.root.join("tombstone").join(folder);
        if live.exists() {
            if let Err(error) = fs::remove_dir_all(&live) {
                first_error.get_or_insert_with(|| {
                    format!("failed to remove replacement {folder}: {error}")
                });
                continue;
            }
        }
        if tombstone.exists() {
            if let Err(error) = rename_with_retries(&tombstone, &live) {
                first_error.get_or_insert_with(|| format!("failed to restore {folder}: {error}"));
            }
        }
    }
    if let Some(error) = first_error {
        Err(error)
    } else {
        Ok(())
    }
}

fn rollback_root_files(txn: &InstallTransaction, landed: &[String]) -> Result<(), String> {
    let mut first_error = None;
    for relative in landed.iter().rev() {
        let live = txn.addons_dir.join(relative);
        let tombstone = txn.root.join("tombstone").join(relative);
        if live.is_file() {
            if let Err(error) = fs::remove_file(&live) {
                first_error.get_or_insert_with(|| {
                    format!("failed to remove replacement root file {relative}: {error}")
                });
                continue;
            }
        }
        if tombstone.is_file() {
            if let Err(error) = rename_with_retries(&tombstone, &live) {
                first_error.get_or_insert_with(|| {
                    format!("failed to restore root file {relative}: {error}")
                });
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn promote_hashes(addons_dir: &Path, root: &Path, hash_folders: &[String]) -> Result<(), String> {
    if hash_folders.is_empty() {
        return Ok(());
    }
    let source_dir = root.join("hashes");
    let tombstone_dir = root.join("hash-tombstone");
    let destination = addons_dir.join(HASHES_DIR);
    fs::create_dir_all(&destination)
        .map_err(|e| format!("Failed to create hash baseline directory: {e}"))?;
    for folder in hash_folders {
        let file_name = format!("{folder}.json");
        let source = source_dir.join(&file_name);
        let live = destination.join(&file_name);
        let tombstone = tombstone_dir.join(&file_name);
        let absent = tombstone_dir.join(format!("{folder}{ABSENT_BASELINE_SUFFIX}"));

        // Renames preserve both the old and new baseline until the transaction
        // is finalized. This makes a partially promoted set fully reversible.
        if source.is_file() {
            if tombstone.is_file() || absent.is_file() {
                if live.exists() {
                    return Err(format!(
                        "Conflicting live and tombstoned hash baselines for {folder}."
                    ));
                }
            } else if live.is_file() {
                rename_with_retries(&live, &tombstone).map_err(|e| {
                    format!("Failed to preserve the previous hash baseline for {folder}: {e}")
                })?;
            } else if live.exists() {
                return Err(format!("Hash baseline path for {folder} is not a file."));
            } else {
                // Nothing to preserve, but the rollback still has to be able to
                // tell "this transaction promoted the file now sitting at `live`"
                // from "the staged file went missing and `live` is the user's
                // existing baseline". Without a durable marker those look
                // identical, and rollback would withdraw a baseline it never
                // wrote — see `rollback_hashes`. Write the marker before the
                // rename so a crash between the two still leaves the promotion
                // attributable.
                fs::write(&absent, b"").map_err(|e| {
                    format!("Failed to record the absent hash baseline for {folder}: {e}")
                })?;
            }
            rename_with_retries(&source, &live)
                .map_err(|e| format!("Failed to promote hash baseline for {folder}: {e}"))?;
        } else if !live.is_file() {
            return Err(format!("Staged hash baseline for {folder} is missing."));
        }
    }
    Ok(())
}

fn rollback_hashes(addons_dir: &Path, root: &Path, hash_folders: &[String]) -> Result<(), String> {
    let source_dir = root.join("hashes");
    let tombstone_dir = root.join("hash-tombstone");
    let destination = addons_dir.join(HASHES_DIR);
    let mut first_error = None;

    for folder in hash_folders.iter().rev() {
        let file_name = format!("{folder}.json");
        let source = source_dir.join(&file_name);
        let live = destination.join(&file_name);
        let tombstone = tombstone_dir.join(&file_name);
        let absent = tombstone_dir.join(format!("{folder}{ABSENT_BASELINE_SUFFIX}"));

        // Only withdraw a live baseline this transaction actually published.
        // `promote_hashes` leaves one of the two markers behind whenever it
        // promotes: the displaced previous baseline, or the empty marker saying
        // there was none. Neither being present means the staged file went
        // missing before promotion and `live` is the user's own baseline, still
        // untouched — withdrawing it would move it into the transaction root,
        // which recovery then deletes. Losing a baseline is not cosmetic: the
        // next update sees none, reads the user's edits as absent, and
        // overwrites them.
        let this_transaction_promoted_it = tombstone.is_file() || absent.is_file();
        if !source.is_file() && live.is_file() && this_transaction_promoted_it {
            if let Err(error) = rename_with_retries(&live, &source) {
                first_error.get_or_insert_with(|| {
                    format!("Failed to withdraw new hash baseline for {folder}: {error}")
                });
                continue;
            }
        }
        if tombstone.is_file() && !live.exists() {
            if let Err(error) = rename_with_retries(&tombstone, &live) {
                first_error.get_or_insert_with(|| {
                    format!("Failed to restore hash baseline for {folder}: {error}")
                });
            }
        }
    }

    first_error.map_or(Ok(()), Err)
}

fn write_journal(root: &Path, journal: &Journal) -> Result<(), String> {
    let bytes = serde_json::to_vec(journal)
        .map_err(|e| format!("Failed to encode installer journal: {e}"))?;
    atomic_file::atomic_write(&root.join("journal.json"), &bytes)
        .map_err(|e| format!("Failed to persist installer journal: {e}"))
}

fn finish_committed_transaction(addons_dir: &Path, root: &Path, journal: &Journal) {
    finish_committed_transaction_with(
        || write_journal(root, journal),
        || fs::remove_dir_all(root),
        || cleanup_empty_staging(addons_dir),
    );
}

fn finish_committed_transaction_with(
    persist_promoted: impl FnOnce() -> Result<(), String>,
    cleanup_root: impl FnOnce() -> io::Result<()>,
    cleanup_staging: impl FnOnce(),
) {
    if let Err(error) = persist_promoted() {
        eprintln!(
            "Warning: install publication committed, but its recovery journal could not be finalized: {error}"
        );
        return;
    }
    if let Err(error) = cleanup_root() {
        eprintln!(
            "Warning: install publication committed, but staging cleanup was deferred: {error}"
        );
        return;
    }
    cleanup_staging();
}

fn is_transient_rename_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::PermissionDenied
        || matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

fn rename_with_retries(from: &Path, to: &Path) -> io::Result<()> {
    retry_transient_rename(
        || fs::rename(from, to),
        || std::thread::sleep(RENAME_BACKOFF),
    )
}

fn retry_transient_rename(
    mut operation: impl FnMut() -> io::Result<()>,
    mut wait: impl FnMut(),
) -> io::Result<()> {
    for attempt in 0..RENAME_ATTEMPTS {
        match operation() {
            Ok(()) => return Ok(()),
            Err(error) if attempt + 1 < RENAME_ATTEMPTS && is_transient_rename_error(&error) => {
                wait()
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

fn swap_error(folder: &str, error: &io::Error, rollback: Result<(), String>) -> String {
    let rollback = rollback
        .err()
        .map(|e| format!(" Rollback also failed: {e}"))
        .unwrap_or_default();
    format!("Could not replace addon folder {folder}: {error}. Close ESO and any editor or antivirus window using files in the AddOns folder, then try again.{rollback}")
}

fn root_swap_error(relative: &str, error: &io::Error, rollback: Result<(), String>) -> String {
    let rollback = rollback
        .err()
        .map(|e| format!(" Rollback also failed: {e}"))
        .unwrap_or_default();
    format!(
        "Could not replace AddOns-root file {relative}: {error}. Close ESO and any editor or antivirus window using files in the AddOns folder, then try again.{rollback}"
    )
}

fn cleanup_empty_staging(addons_dir: &Path) {
    let staging = addons_dir.join(STAGING_DIR);
    if fs::read_dir(&staging)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(false)
    {
        let _ = fs::remove_dir(&staging);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_addon(folder: &Path, contents: &str) {
        fs::create_dir_all(folder).expect("create addon folder");
        fs::write(folder.join("main.lua"), contents).expect("write addon file");
    }

    fn prepare_root(addons_dir: &Path, name: &str) -> PathBuf {
        let root = addons_dir.join(STAGING_DIR).join(name);
        for child in ["stage", "tombstone", "hashes", "hash-tombstone"] {
            fs::create_dir_all(root.join(child)).expect("create transaction directory");
        }
        root
    }

    #[test]
    fn rollback_keeps_a_baseline_this_transaction_never_promoted() {
        // The staged hash file vanishing mid-transaction (antivirus, a cleanup
        // tool, the user) leaves `promote_hashes` looking at a missing source
        // and a live baseline, which it treats as already-promoted and skips.
        // Rollback then used to withdraw that live file into the transaction
        // root, which recovery deletes — destroying the user's own baseline.
        // That is not a cosmetic loss: the next update sees no baseline, reads
        // the user's edits as absent, and overwrites them.
        let temp = tempfile::tempdir().expect("tempdir");
        let addons_dir = temp.path().join("AddOns");
        let root = prepare_root(&addons_dir, "txn");
        let hashes_dir = addons_dir.join(HASHES_DIR);
        fs::create_dir_all(&hashes_dir).expect("create hashes dir");
        let live = hashes_dir.join("Example.json");
        fs::write(&live, br#"{"the-users":"baseline"}"#).expect("write baseline");
        // No staged source, and no tombstone or marker: this transaction never
        // promoted anything for this folder.

        promote_hashes(&addons_dir, &root, &["Example".to_string()]).expect("promote");
        rollback_hashes(&addons_dir, &root, &["Example".to_string()]).expect("rollback");

        assert_eq!(
            fs::read(&live).expect("baseline must survive"),
            br#"{"the-users":"baseline"}"#,
            "rollback withdrew a baseline it never published"
        );
    }

    #[test]
    fn rollback_withdraws_a_first_baseline_this_transaction_did_promote() {
        // The mirror case: no previous baseline, so promotion writes the absent
        // marker and publishes. Rollback must undo that, leaving the folder with
        // no baseline exactly as it found it.
        let temp = tempfile::tempdir().expect("tempdir");
        let addons_dir = temp.path().join("AddOns");
        let root = prepare_root(&addons_dir, "txn");
        fs::write(
            root.join("hashes").join("Example.json"),
            br#"{"new":"baseline"}"#,
        )
        .expect("stage baseline");
        let live = addons_dir.join(HASHES_DIR).join("Example.json");

        promote_hashes(&addons_dir, &root, &["Example".to_string()]).expect("promote");
        assert!(
            live.is_file(),
            "promotion should publish the staged baseline"
        );

        rollback_hashes(&addons_dir, &root, &["Example".to_string()]).expect("rollback");

        assert!(
            !live.exists(),
            "a first baseline this transaction published must be withdrawn again"
        );
        assert!(
            root.join("hashes").join("Example.json").is_file(),
            "the withdrawn baseline belongs back in the transaction"
        );
    }

    #[test]
    fn rollback_restores_the_displaced_previous_baseline() {
        let temp = tempfile::tempdir().expect("tempdir");
        let addons_dir = temp.path().join("AddOns");
        let root = prepare_root(&addons_dir, "txn");
        let hashes_dir = addons_dir.join(HASHES_DIR);
        fs::create_dir_all(&hashes_dir).expect("create hashes dir");
        let live = hashes_dir.join("Example.json");
        fs::write(&live, br#"{"old":"baseline"}"#).expect("write baseline");
        fs::write(
            root.join("hashes").join("Example.json"),
            br#"{"new":"baseline"}"#,
        )
        .expect("stage baseline");

        promote_hashes(&addons_dir, &root, &["Example".to_string()]).expect("promote");
        assert_eq!(fs::read(&live).unwrap(), br#"{"new":"baseline"}"#);

        rollback_hashes(&addons_dir, &root, &["Example".to_string()]).expect("rollback");

        assert_eq!(
            fs::read(&live).expect("previous baseline must come back"),
            br#"{"old":"baseline"}"#
        );
    }

    #[test]
    fn hash_rollback_is_idempotent() {
        // Recovery can itself be interrupted and retried.
        let temp = tempfile::tempdir().expect("tempdir");
        let addons_dir = temp.path().join("AddOns");
        let root = prepare_root(&addons_dir, "txn");
        let hashes_dir = addons_dir.join(HASHES_DIR);
        fs::create_dir_all(&hashes_dir).expect("create hashes dir");
        let live = hashes_dir.join("Example.json");
        fs::write(&live, br#"{"old":"baseline"}"#).expect("write baseline");
        fs::write(
            root.join("hashes").join("Example.json"),
            br#"{"new":"baseline"}"#,
        )
        .expect("stage baseline");

        promote_hashes(&addons_dir, &root, &["Example".to_string()]).expect("promote");
        for attempt in 0..3 {
            rollback_hashes(&addons_dir, &root, &["Example".to_string()])
                .unwrap_or_else(|e| panic!("rollback attempt {attempt} failed: {e}"));
            assert_eq!(fs::read(&live).unwrap(), br#"{"old":"baseline"}"#);
        }
    }

    #[test]
    fn commit_replaces_folder_and_promotes_hashes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let addons_dir = temp.path().join("AddOns");
        write_addon(&addons_dir.join("Example"), "old");

        let transaction = InstallTransaction::begin(&addons_dir, None).expect("begin transaction");
        write_addon(&transaction.stage_dir().join("Example"), "new");
        let installed = transaction
            .commit(
                &["Example".to_string()],
                &[("Example".to_string(), br#"{"version":"2"}"#.to_vec())],
            )
            .expect("commit transaction");

        assert_eq!(installed, ["Example"]);
        assert_eq!(
            fs::read_to_string(addons_dir.join("Example/main.lua")).expect("read live addon"),
            "new"
        );
        assert_eq!(
            fs::read(addons_dir.join(".kalpa-hashes/Example.json")).expect("read hash baseline"),
            br#"{"version":"2"}"#
        );
        assert!(!addons_dir.join(STAGING_DIR).exists());
    }

    #[test]
    fn commit_replaces_addons_root_files_with_the_folder() {
        let temp = tempfile::tempdir().expect("tempdir");
        let addons_dir = temp.path().join("AddOns");
        write_addon(&addons_dir.join("Example"), "old");
        fs::write(addons_dir.join("README.txt"), "root-old").expect("write old root file");

        let transaction = InstallTransaction::begin(&addons_dir, None).expect("begin transaction");
        write_addon(&transaction.stage_dir().join("Example"), "new");
        fs::write(transaction.stage_dir().join("README.txt"), "root-new")
            .expect("write staged root file");
        transaction
            .commit_with_root_files(&["Example".to_string()], &["README.txt".to_string()], &[])
            .expect("commit transaction");

        assert_eq!(
            fs::read_to_string(addons_dir.join("Example/main.lua")).expect("read live addon"),
            "new"
        );
        assert_eq!(
            fs::read_to_string(addons_dir.join("README.txt")).expect("read root file"),
            "root-new"
        );
        assert!(!addons_dir.join(STAGING_DIR).exists());
    }

    #[test]
    fn recovery_restores_a_partially_swapped_root_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let addons_dir = temp.path().join("AddOns");
        let root = prepare_root(&addons_dir, "partial-root");
        fs::write(addons_dir.join("README.txt"), "root-new").expect("write published root file");
        fs::write(root.join("tombstone/README.txt"), "root-old").expect("write root tombstone");
        write_journal(
            &root,
            &Journal {
                phase: Phase::Staged,
                folders: Vec::new(),
                pre_existing: Vec::new(),
                hash_folders: Vec::new(),
                root_files: vec!["README.txt".to_string()],
                pre_existing_root_files: vec!["README.txt".to_string()],
            },
        )
        .expect("write journal");

        recover_staging(&addons_dir).expect("recover transaction");

        assert_eq!(
            fs::read_to_string(addons_dir.join("README.txt")).expect("read restored root file"),
            "root-old"
        );
        assert!(!addons_dir.join(STAGING_DIR).exists());
    }

    #[test]
    fn recovery_rolls_back_a_partially_swapped_staged_transaction() {
        let temp = tempfile::tempdir().expect("tempdir");
        let addons_dir = temp.path().join("AddOns");
        let root = prepare_root(&addons_dir, "partial");
        write_addon(&addons_dir.join("Example"), "old");
        write_addon(&addons_dir.join("Second"), "second-old");
        write_addon(&root.join("stage/Example"), "new");
        write_addon(&root.join("stage/Second"), "second-new");
        fs::rename(addons_dir.join("Example"), root.join("tombstone/Example"))
            .expect("tombstone old addon");
        fs::rename(root.join("stage/Example"), addons_dir.join("Example"))
            .expect("publish replacement");
        write_journal(
            &root,
            &Journal {
                phase: Phase::Staged,
                folders: vec!["Example".to_string(), "Second".to_string()],
                pre_existing: vec!["Example".to_string(), "Second".to_string()],
                hash_folders: Vec::new(),
                root_files: Vec::new(),
                pre_existing_root_files: Vec::new(),
            },
        )
        .expect("write journal");

        recover_staging(&addons_dir).expect("recover transaction");

        assert_eq!(
            fs::read_to_string(addons_dir.join("Example/main.lua")).expect("read restored addon"),
            "old"
        );
        assert_eq!(
            fs::read_to_string(addons_dir.join("Second/main.lua")).expect("read second addon"),
            "second-old"
        );
        assert!(!addons_dir.join(STAGING_DIR).exists());
    }

    #[test]
    fn recovery_restores_the_only_addon_copy_when_the_journal_cannot_describe_the_swap() {
        for journal_state in ["preparing", "missing", "corrupt"] {
            let temp = tempfile::tempdir().expect("tempdir");
            let addons_dir = temp.path().join("AddOns");
            let root = prepare_root(&addons_dir, journal_state);
            write_addon(&root.join("tombstone/Example"), "old");
            match journal_state {
                "preparing" => write_journal(
                    &root,
                    &Journal {
                        phase: Phase::Preparing,
                        folders: Vec::new(),
                        pre_existing: Vec::new(),
                        hash_folders: Vec::new(),
                        root_files: Vec::new(),
                        pre_existing_root_files: Vec::new(),
                    },
                )
                .expect("write preparing journal"),
                "corrupt" => fs::write(root.join("journal.json"), b"not-json")
                    .expect("write corrupt journal"),
                "missing" => {}
                _ => unreachable!(),
            }

            recover_staging(&addons_dir).expect("recover transaction");

            assert_eq!(
                fs::read_to_string(addons_dir.join("Example/main.lua"))
                    .expect("read restored addon"),
                "old",
                "journal state: {journal_state}"
            );
            assert!(!addons_dir.join(STAGING_DIR).exists());
        }
    }

    #[test]
    fn commit_rolls_back_all_folders_when_a_later_swap_fails() {
        let temp = tempfile::tempdir().expect("tempdir");
        let addons_dir = temp.path().join("AddOns");
        write_addon(&addons_dir.join("First"), "first-old");
        write_addon(&addons_dir.join("Second"), "second-old");
        let transaction = InstallTransaction::begin(&addons_dir, None).expect("begin transaction");
        write_addon(&transaction.stage_dir().join("First"), "first-new");

        let error = transaction
            .commit(&["First".to_string(), "Second".to_string()], &[])
            .expect_err("missing second staged folder must abort the commit");

        assert!(error.contains("Could not replace addon folder Second"));
        assert_eq!(
            fs::read_to_string(addons_dir.join("First/main.lua")).expect("read first addon"),
            "first-old"
        );
        assert_eq!(
            fs::read_to_string(addons_dir.join("Second/main.lua")).expect("read second addon"),
            "second-old"
        );
        assert!(!addons_dir.join(STAGING_DIR).exists());
    }

    #[test]
    fn recovery_rolls_back_hashes_after_all_swaps_landed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let addons_dir = temp.path().join("AddOns");
        let root = prepare_root(&addons_dir, "swapped");
        write_addon(&addons_dir.join("Example"), "new");
        write_addon(&root.join("tombstone/Example"), "old");
        fs::create_dir_all(addons_dir.join(".kalpa-hashes")).expect("create hashes directory");
        fs::write(
            root.join("hash-tombstone/Example.json"),
            br#"{"version":"1"}"#,
        )
        .expect("write old hash tombstone");
        fs::write(
            addons_dir.join(".kalpa-hashes/Example.json"),
            br#"{"version":"2"}"#,
        )
        .expect("write promoted hash baseline");
        write_journal(
            &root,
            &Journal {
                phase: Phase::Swapped,
                folders: vec!["Example".to_string()],
                pre_existing: vec!["Example".to_string()],
                hash_folders: vec!["Example".to_string()],
                root_files: Vec::new(),
                pre_existing_root_files: Vec::new(),
            },
        )
        .expect("write journal");

        recover_staging(&addons_dir).expect("recover transaction");

        assert_eq!(
            fs::read_to_string(addons_dir.join("Example/main.lua")).expect("read live addon"),
            "old"
        );
        assert_eq!(
            fs::read(addons_dir.join(".kalpa-hashes/Example.json")).expect("read hash baseline"),
            br#"{"version":"1"}"#
        );
        assert!(!addons_dir.join(STAGING_DIR).exists());
    }

    #[test]
    fn recovery_rolls_back_a_swapped_journal_when_a_stage_folder_remains() {
        let temp = tempfile::tempdir().expect("tempdir");
        let addons_dir = temp.path().join("AddOns");
        let root = prepare_root(&addons_dir, "lost-rename");
        write_addon(&root.join("stage/Example"), "new");
        write_addon(&root.join("tombstone/Example"), "old");
        write_journal(
            &root,
            &Journal {
                phase: Phase::Swapped,
                folders: vec!["Example".to_string()],
                pre_existing: vec!["Example".to_string()],
                hash_folders: Vec::new(),
                root_files: Vec::new(),
                pre_existing_root_files: Vec::new(),
            },
        )
        .expect("write journal");

        recover_staging(&addons_dir).expect("recover transaction");

        assert_eq!(
            fs::read_to_string(addons_dir.join("Example/main.lua")).expect("read restored addon"),
            "old"
        );
        assert!(!addons_dir.join(STAGING_DIR).exists());
    }

    #[test]
    fn recovery_rolls_back_a_swapped_journal_when_the_live_folder_is_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let addons_dir = temp.path().join("AddOns");
        let root = prepare_root(&addons_dir, "missing-live");
        write_addon(&root.join("tombstone/Example"), "old");
        fs::write(root.join("hashes/Example.json"), br#"{"version":"2"}"#)
            .expect("write staged hash baseline");
        write_journal(
            &root,
            &Journal {
                phase: Phase::Swapped,
                folders: vec!["Example".to_string()],
                pre_existing: vec!["Example".to_string()],
                hash_folders: vec!["Example".to_string()],
                root_files: Vec::new(),
                pre_existing_root_files: Vec::new(),
            },
        )
        .expect("write journal");

        recover_staging(&addons_dir).expect("recover transaction");

        assert_eq!(
            fs::read_to_string(addons_dir.join("Example/main.lua")).expect("read restored addon"),
            "old"
        );
        assert!(!addons_dir.join(".kalpa-hashes/Example.json").exists());
        assert!(!addons_dir.join(STAGING_DIR).exists());
    }

    #[test]
    fn recovery_rejects_invalid_journal_folder_names() {
        let temp = tempfile::tempdir().expect("tempdir");
        let addons_dir = temp.path().join("AddOns");
        let root = prepare_root(&addons_dir, "invalid-folder");
        write_journal(
            &root,
            &Journal {
                phase: Phase::Staged,
                folders: vec!["../Outside".to_string()],
                pre_existing: Vec::new(),
                hash_folders: Vec::new(),
                root_files: Vec::new(),
                pre_existing_root_files: Vec::new(),
            },
        )
        .expect("write journal");

        let error = recover_staging(&addons_dir).expect_err("invalid journal must fail closed");

        assert!(error.contains("Invalid folder in abandoned installer transaction"));
        assert!(root.exists(), "failed recovery must retain its evidence");
    }

    #[test]
    fn recovery_rolls_back_a_swapped_install_with_missing_hash_baseline() {
        let temp = tempfile::tempdir().expect("tempdir");
        let addons_dir = temp.path().join("AddOns");
        let root = prepare_root(&addons_dir, "missing-hash");
        write_addon(&addons_dir.join("Example"), "new");
        write_addon(&root.join("tombstone/Example"), "old");
        write_journal(
            &root,
            &Journal {
                phase: Phase::Swapped,
                folders: vec!["Example".to_string()],
                pre_existing: vec!["Example".to_string()],
                hash_folders: vec!["Example".to_string()],
                root_files: Vec::new(),
                pre_existing_root_files: Vec::new(),
            },
        )
        .expect("write journal");

        recover_staging(&addons_dir).expect("recover transaction");
        assert_eq!(
            fs::read_to_string(addons_dir.join("Example/main.lua")).expect("read live addon"),
            "old"
        );
        assert!(!root.exists());
        assert!(!addons_dir.join(".kalpa-hashes/Example.json").exists());
    }

    #[test]
    fn transient_rename_failures_are_retried() {
        let attempts = std::cell::Cell::new(0);
        let waits = std::cell::Cell::new(0);

        retry_transient_rename(
            || {
                let attempt = attempts.get() + 1;
                attempts.set(attempt);
                if attempt < 3 {
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                } else {
                    Ok(())
                }
            },
            || waits.set(waits.get() + 1),
        )
        .expect("transient rename should eventually succeed");

        assert_eq!(attempts.get(), 3);
        assert_eq!(waits.get(), 2);
    }

    #[test]
    fn committed_publication_defers_bookkeeping_failures() {
        let staging_cleanups = std::cell::Cell::new(0);
        finish_committed_transaction_with(
            || Err("journal unavailable".to_string()),
            || panic!("cleanup must wait when the promoted journal is not durable"),
            || staging_cleanups.set(staging_cleanups.get() + 1),
        );
        assert_eq!(staging_cleanups.get(), 0);

        finish_committed_transaction_with(
            || Ok(()),
            || Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            || staging_cleanups.set(staging_cleanups.get() + 1),
        );
        assert_eq!(staging_cleanups.get(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn locked_live_file_fails_cleanly_without_changing_the_addon() {
        use std::os::windows::fs::OpenOptionsExt;

        let temp = tempfile::tempdir().expect("tempdir");
        let addons_dir = temp.path().join("AddOns");
        write_addon(&addons_dir.join("Example"), "old");
        let held_file = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(addons_dir.join("Example/main.lua"))
            .expect("hold addon file without delete sharing");
        let transaction = InstallTransaction::begin(&addons_dir, None).expect("begin transaction");
        write_addon(&transaction.stage_dir().join("Example"), "new");

        let error = transaction
            .commit(&["Example".to_string()], &[])
            .expect_err("locked addon must not be replaced");
        drop(held_file);

        assert!(error.contains("Close ESO and any editor or antivirus"));
        assert_eq!(
            fs::read_to_string(addons_dir.join("Example/main.lua")).expect("read live addon"),
            "old"
        );
        assert!(!addons_dir.join(STAGING_DIR).exists());
    }
}
