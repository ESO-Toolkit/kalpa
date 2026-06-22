use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Maximum total extracted size (500 MB) to guard against ZIP bombs.
const MAX_EXTRACT_SIZE: u64 = 500 * 1024 * 1024;

/// Turn a filesystem write error into a user-facing message. When the OS
/// reports permission denied (Windows `os error 5` / Unix `PermissionDenied`),
/// the most common cause on Windows is the AddOns folder living under
/// `Documents`, which Windows Defender's **Controlled Folder Access**
/// (ransomware protection) blocks apps from writing to. Surface that
/// explanation with concrete steps instead of a raw "Access is denied".
fn describe_write_error(path: &Path, e: &io::Error) -> String {
    if e.kind() == io::ErrorKind::PermissionDenied {
        format!(
            "Windows blocked Kalpa from writing to your AddOns folder ({path:?}). \
             This is most often Controlled Folder Access (ransomware protection), \
             but can also be a read-only file, restrictive permissions, or antivirus. \
             To fix the common case: open Windows Security → Virus & threat protection → \
             Ransomware protection → Allow an app through Controlled folder access, \
             then add Kalpa. (Underlying error: {e})"
        )
    } else {
        format!("Failed to write {path:?}: {e}")
    }
}

/// Describe an error from streaming a ZIP entry to disk (`io::copy`). A
/// permission denial here is still a blocked write (surface the CFA guidance),
/// but any other failure is most likely a corrupt/truncated archive on the
/// read side — so give extraction context rather than a misleading
/// "failed to write" message.
fn describe_extract_error(path: &Path, e: &io::Error) -> String {
    if e.kind() == io::ErrorKind::PermissionDenied {
        describe_write_error(path, e)
    } else {
        format!("Failed to extract {path:?} (the archive may be corrupt): {e}")
    }
}

/// Cooperative hooks for a long extraction: optional cancellation and per-file
/// progress. Both default to `None` (see [`ExtractHooks::NONE`]) so the common
/// callers stay trivial.
#[derive(Clone, Copy)]
pub struct ExtractHooks<'a> {
    /// Polled before each entry; when it reads `true` the extraction aborts with
    /// [`CANCELLED`] and the caller's rollback removes any newly-created folders.
    pub cancel: Option<&'a AtomicBool>,
    /// Invoked as `(done, total)` at the start of each entry so the UI can render
    /// "Extracting N of M". `total` is the raw archive entry count (includes
    /// directories), close enough for a progress bar.
    pub progress: Option<&'a dyn Fn(usize, usize)>,
}

impl ExtractHooks<'_> {
    /// No cancellation, no progress — the default for callers that need neither.
    pub const NONE: ExtractHooks<'static> = ExtractHooks {
        cancel: None,
        progress: None,
    };
}

/// Error string returned when an extraction is cancelled via
/// [`ExtractHooks::cancel`]. Callers match on this to distinguish a deliberate
/// Stop from a real failure (e.g. to show a neutral "stopped" state).
pub const CANCELLED: &str = "Update cancelled.";

fn report_progress(hooks: &ExtractHooks, done: usize, total: usize) {
    if let Some(cb) = hooks.progress {
        cb(done, total);
    }
}

fn is_cancelled(hooks: &ExtractHooks) -> bool {
    hooks
        .cancel
        .map(|flag| flag.load(Ordering::Relaxed))
        .unwrap_or(false)
}

pub fn extract_addon_zip_selective(
    zip_path: &Path,
    addons_dir: &Path,
    skip_files: &HashSet<String>,
) -> Result<Vec<String>, String> {
    extract_addon_zip_selective_with(zip_path, addons_dir, skip_files, ExtractHooks::NONE)
}

/// Like [`extract_addon_zip_selective`] but with cancellation/progress hooks.
pub fn extract_addon_zip_selective_with(
    zip_path: &Path,
    addons_dir: &Path,
    skip_files: &HashSet<String>,
    hooks: ExtractHooks,
) -> Result<Vec<String>, String> {
    extract_with_rollback(zip_path, addons_dir, skip_files, hooks)
}

pub fn extract_addon_zip(zip_path: &Path, addons_dir: &Path) -> Result<Vec<String>, String> {
    extract_addon_zip_with(zip_path, addons_dir, ExtractHooks::NONE)
}

/// Like [`extract_addon_zip`] but with cancellation/progress hooks.
pub fn extract_addon_zip_with(
    zip_path: &Path,
    addons_dir: &Path,
    hooks: ExtractHooks,
) -> Result<Vec<String>, String> {
    extract_with_rollback(zip_path, addons_dir, &HashSet::new(), hooks)
}

/// Shared extraction driver: snapshot which top-level folders already exist, run
/// the inner loop, and on ANY error (including a user cancel) remove only the
/// folders that were newly created — never the user's pre-existing addon during a
/// failed/cancelled update. An empty `skip_files` extracts everything.
fn extract_with_rollback(
    zip_path: &Path,
    addons_dir: &Path,
    skip_files: &HashSet<String>,
    hooks: ExtractHooks,
) -> Result<Vec<String>, String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("Failed to open ZIP file: {e}"))?;

    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Failed to read ZIP archive: {e}"))?;

    // Snapshot which top-level addon folders already exist so we only clean up
    // genuinely new directories on failure (or cancel).
    let mut pre_existing: HashSet<String> = HashSet::new();
    for i in 0..archive.len() {
        if let Ok(entry) = archive.by_index(i) {
            if let Some(p) = entry.enclosed_name() {
                if let Some(first) = p.components().next() {
                    let folder = first.as_os_str().to_string_lossy().to_string();
                    if addons_dir.join(&folder).is_dir() {
                        pre_existing.insert(folder);
                    }
                }
            }
        }
    }

    let result = extract_addon_zip_inner(&mut archive, addons_dir, skip_files, hooks);

    if let Err(ref err_msg) = result {
        // Remove only folders that were newly created (not pre-existing) so a
        // failed or cancelled update never destroys the user's existing addon.
        if let Ok(created) = collect_zip_top_folders(&mut archive) {
            for folder in &created {
                if !pre_existing.contains(folder) {
                    let folder_path = addons_dir.join(folder);
                    if folder_path.is_dir() {
                        eprintln!(
                            "Cleaning up partially extracted folder {folder:?} after error: {err_msg}"
                        );
                        let _ = fs::remove_dir_all(&folder_path);
                    }
                }
            }
        }
    }

    result
}

/// Collect top-level folder names from a ZIP archive.
fn collect_zip_top_folders(
    archive: &mut zip::ZipArchive<fs::File>,
) -> Result<HashSet<String>, String> {
    let mut folders = HashSet::new();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry: {e}"))?;
        if let Some(p) = entry.enclosed_name() {
            if let Some(first) = p.components().next() {
                folders.insert(first.as_os_str().to_string_lossy().to_string());
            }
        }
    }
    Ok(folders)
}

/// Inner extraction loop, separated so [`extract_with_rollback`] can clean up on
/// error. Files whose forward-slash key is in `skip_files` are left untouched
/// (conflict "keep mine"); an empty set extracts everything. Between entries it
/// honors the cancel flag and reports progress via `hooks`.
fn extract_addon_zip_inner(
    archive: &mut zip::ZipArchive<fs::File>,
    addons_dir: &Path,
    skip_files: &HashSet<String>,
    hooks: ExtractHooks,
) -> Result<Vec<String>, String> {
    let mut created_folders: HashSet<String> = HashSet::new();
    let mut total_extracted: u64 = 0;
    let total = archive.len();

    for i in 0..total {
        // Cooperative cancellation: abort cleanly between entries (sub-100ms
        // latency) and let extract_with_rollback remove partial output.
        if is_cancelled(&hooks) {
            return Err(CANCELLED.to_string());
        }
        report_progress(&hooks, i, total);

        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry: {e}"))?;

        // Skip symlink entries (check unix mode for symlink bit 0o120000)
        if let Some(mode) = entry.unix_mode() {
            if mode & 0o170000 == 0o120000 {
                continue;
            }
        }

        // Use enclosed_name for path traversal safety
        let relative_path = match entry.enclosed_name() {
            Some(p) => p.to_owned(),
            None => continue,
        };

        // Honor "keep mine" conflict decisions (no-op when skip_files is empty).
        let key = relative_path.to_string_lossy().replace('\\', "/");
        if skip_files.contains(&key) {
            continue;
        }

        let out_path = addons_dir.join(&relative_path);

        // Track top-level folder names
        if let Some(first_component) = relative_path.components().next() {
            let folder = first_component.as_os_str().to_string_lossy().to_string();
            created_folders.insert(folder);
        }

        if entry.is_dir() {
            fs::create_dir_all(&out_path).map_err(|e| describe_write_error(&out_path, &e))?;
        } else {
            // Check declared size against remaining budget before extracting
            let declared_size = entry.size();
            if total_extracted + declared_size > MAX_EXTRACT_SIZE {
                return Err(format!(
                    "ZIP extraction aborted: total size exceeds {} MB limit. Possible ZIP bomb.",
                    MAX_EXTRACT_SIZE / (1024 * 1024)
                ));
            }

            // Ensure parent directory exists
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|e| describe_write_error(parent, &e))?;
            }

            let mut outfile =
                fs::File::create(&out_path).map_err(|e| describe_write_error(&out_path, &e))?;

            let bytes_written = io::copy(&mut entry, &mut outfile)
                .map_err(|e| describe_extract_error(&out_path, &e))?;

            total_extracted += bytes_written;

            // Double-check actual bytes written against budget
            if total_extracted > MAX_EXTRACT_SIZE {
                // Clean up the file we just wrote
                let _ = fs::remove_file(&out_path);
                return Err(format!(
                    "ZIP extraction aborted: total size exceeds {} MB limit. Possible ZIP bomb.",
                    MAX_EXTRACT_SIZE / (1024 * 1024)
                ));
            }
        }
    }

    report_progress(&hooks, total, total);

    if created_folders.is_empty() {
        return Err("ZIP archive contained no addon folders.".to_string());
    }

    Ok(created_folders.into_iter().collect())
}

pub fn remove_addon(addons_dir: &Path, folder_name: &str) -> Result<(), String> {
    // Validate folder name — no path traversal
    if folder_name.contains("..")
        || folder_name.contains('/')
        || folder_name.contains('\\')
        || folder_name.is_empty()
    {
        return Err("Invalid addon folder name.".to_string());
    }

    let addon_path = addons_dir.join(folder_name);

    if !addon_path.is_dir() {
        return Err(format!("Addon folder not found: {folder_name}"));
    }

    // Verify the folder is actually inside the addons directory
    let canonical_addons = addons_dir
        .canonicalize()
        .map_err(|e| format!("Failed to resolve addons path: {e}"))?;
    let canonical_addon = addon_path
        .canonicalize()
        .map_err(|e| format!("Failed to resolve addon path: {e}"))?;

    if !canonical_addon.starts_with(&canonical_addons) {
        return Err("Addon path is outside the AddOns directory.".to_string());
    }

    fs::remove_dir_all(&addon_path)
        .map_err(|e| format!("Failed to remove addon {folder_name}: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    #[test]
    fn permission_denied_mentions_controlled_folder_access() {
        let err = io::Error::from(io::ErrorKind::PermissionDenied);
        let msg = describe_write_error(Path::new("C:/Users/x/Documents/AddOns/Foo"), &err);
        assert!(msg.contains("Controlled Folder Access"));
        assert!(msg.contains("Allow an app"));
    }

    #[test]
    fn other_write_errors_stay_generic() {
        let err = io::Error::from(io::ErrorKind::NotFound);
        let msg = describe_write_error(Path::new("/tmp/x"), &err);
        assert!(msg.starts_with("Failed to write"));
        assert!(!msg.contains("Controlled Folder Access"));
    }

    #[test]
    fn extract_permission_denied_still_explains_cfa() {
        let err = io::Error::from(io::ErrorKind::PermissionDenied);
        let msg = describe_extract_error(Path::new("C:/x/Foo"), &err);
        assert!(msg.contains("Controlled Folder Access"));
    }

    #[test]
    fn extract_non_permission_errors_mention_corruption() {
        let err = io::Error::from(io::ErrorKind::UnexpectedEof);
        let msg = describe_extract_error(Path::new("/tmp/x"), &err);
        assert!(msg.contains("Failed to extract"));
        assert!(msg.contains("corrupt"));
        assert!(!msg.contains("Controlled Folder Access"));
    }

    /// Create a simple valid ZIP with one folder and one file.
    fn create_test_zip(dir: &Path, zip_name: &str, folder: &str, file_content: &str) -> PathBuf {
        let zip_path = dir.join(zip_name);
        let file = fs::File::create(&zip_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);

        let options = zip::write::SimpleFileOptions::default();
        archive
            .start_file(format!("{folder}/test.txt"), options)
            .unwrap();
        archive.write_all(file_content.as_bytes()).unwrap();
        archive.finish().unwrap();

        zip_path
    }

    #[test]
    fn extracts_valid_zip() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        let zip_path = create_test_zip(tmp.path(), "test.zip", "TestAddon", "hello");
        let folders = extract_addon_zip(&zip_path, &addons_dir).unwrap();

        assert_eq!(folders, vec!["TestAddon".to_string()]);
        assert!(addons_dir.join("TestAddon/test.txt").exists());
        assert_eq!(
            fs::read_to_string(addons_dir.join("TestAddon/test.txt")).unwrap(),
            "hello"
        );
    }

    #[test]
    fn rejects_empty_zip() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        // Create an empty ZIP
        let zip_path = tmp.path().join("empty.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let archive = zip::ZipWriter::new(file);
        archive.finish().unwrap();

        let result = extract_addon_zip(&zip_path, &addons_dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no addon folders"));
    }

    #[test]
    fn remove_addon_rejects_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        assert!(remove_addon(&addons_dir, "..").is_err());
        assert!(remove_addon(&addons_dir, "../etc").is_err());
        assert!(remove_addon(&addons_dir, "foo/bar").is_err());
        assert!(remove_addon(&addons_dir, "foo\\bar").is_err());
        assert!(remove_addon(&addons_dir, "").is_err());
    }

    #[test]
    fn remove_addon_rejects_nonexistent() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        let result = remove_addon(&addons_dir, "NoSuchAddon");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn removes_addon_successfully() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let addon_path = addons_dir.join("TestAddon");
        fs::create_dir_all(&addon_path).unwrap();
        fs::write(addon_path.join("test.txt"), "data").unwrap();

        assert!(addon_path.exists());
        remove_addon(&addons_dir, "TestAddon").unwrap();
        assert!(!addon_path.exists());
    }

    #[test]
    fn tracks_multiple_top_level_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        let zip_path = tmp.path().join("multi.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();

        archive.start_file("AddonA/init.lua", options).unwrap();
        archive.write_all(b"-- lua").unwrap();
        archive.start_file("AddonB/init.lua", options).unwrap();
        archive.write_all(b"-- lua").unwrap();
        archive.finish().unwrap();

        let mut folders = extract_addon_zip(&zip_path, &addons_dir).unwrap();
        folders.sort();
        assert_eq!(folders, vec!["AddonA".to_string(), "AddonB".to_string()]);
    }

    // ── Cancellation, progress, and selective extraction ─────────────────

    fn create_multi_file_zip(dir: &Path, name: &str, folder: &str, count: usize) -> PathBuf {
        let zip_path = dir.join(name);
        let file = fs::File::create(&zip_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for n in 0..count {
            archive
                .start_file(format!("{folder}/file{n}.lua"), options)
                .unwrap();
            archive.write_all(b"-- lua").unwrap();
        }
        archive.finish().unwrap();
        zip_path
    }

    #[test]
    fn cancel_midway_removes_newly_created_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();
        let zip_path = create_multi_file_zip(tmp.path(), "big.zip", "NewAddon", 10);

        let flag = AtomicBool::new(false);
        // Trip the cancel flag once a couple of files are in.
        let cb = |done: usize, _total: usize| {
            if done >= 2 {
                flag.store(true, Ordering::Relaxed);
            }
        };
        let hooks = ExtractHooks {
            cancel: Some(&flag),
            progress: Some(&cb),
        };

        let result = extract_addon_zip_with(&zip_path, &addons_dir, hooks);
        assert_eq!(result.unwrap_err(), CANCELLED);
        assert!(
            !addons_dir.join("NewAddon").exists(),
            "a cancelled fresh install must clean up the partially-written folder"
        );
    }

    #[test]
    fn cancel_midway_preserves_pre_existing_addon_files() {
        // Cancelling midway through an IN-PLACE update (the folder already
        // exists) must never delete the user's addon — even though it is left
        // partially updated. Data safety: no file is removed, only some are
        // overwritten with the new version's bytes.
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let existing = addons_dir.join("MyAddon");
        fs::create_dir_all(&existing).unwrap();
        for n in 0..10 {
            fs::write(existing.join(format!("file{n}.lua")), "OLD").unwrap();
        }

        // ZIP that overwrites every file with new bytes.
        let zip_path = create_multi_file_zip(tmp.path(), "u.zip", "MyAddon", 10);

        let flag = AtomicBool::new(false);
        let cb = |done: usize, _total: usize| {
            if done >= 2 {
                flag.store(true, Ordering::Relaxed);
            }
        };
        let hooks = ExtractHooks {
            cancel: Some(&flag),
            progress: Some(&cb),
        };

        let result = extract_addon_zip_with(&zip_path, &addons_dir, hooks);
        assert_eq!(result.unwrap_err(), CANCELLED);
        // The pre-existing folder and ALL its files must still be present.
        assert!(
            existing.is_dir(),
            "pre-existing addon must survive a midway cancel"
        );
        for n in 0..10 {
            assert!(
                existing.join(format!("file{n}.lua")).exists(),
                "cancel must not delete the user's existing files"
            );
        }
    }

    #[test]
    fn cancel_preserves_pre_existing_addon() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let existing = addons_dir.join("MyAddon");
        fs::create_dir_all(&existing).unwrap();
        fs::write(existing.join("keep.lua"), "user data").unwrap();

        let zip_path = create_test_zip(tmp.path(), "u.zip", "MyAddon", "new");
        let flag = AtomicBool::new(true); // cancel before the first entry
        let hooks = ExtractHooks {
            cancel: Some(&flag),
            progress: None,
        };

        let result = extract_addon_zip_with(&zip_path, &addons_dir, hooks);
        assert_eq!(result.unwrap_err(), CANCELLED);
        assert!(
            existing.join("keep.lua").exists(),
            "cancelling an update must not destroy the user's pre-existing addon"
        );
    }

    #[test]
    fn progress_callback_fires_during_extraction() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();
        let zip_path = create_multi_file_zip(tmp.path(), "p.zip", "TestAddon", 5);

        let max_done = std::sync::atomic::AtomicUsize::new(0);
        let saw_total = AtomicBool::new(false);
        let cb = |done: usize, total: usize| {
            max_done.fetch_max(done, Ordering::Relaxed);
            if done == total && total > 0 {
                saw_total.store(true, Ordering::Relaxed);
            }
        };
        let hooks = ExtractHooks {
            cancel: None,
            progress: Some(&cb),
        };

        extract_addon_zip_with(&zip_path, &addons_dir, hooks).unwrap();
        assert!(
            max_done.load(Ordering::Relaxed) >= 1,
            "progress should advance"
        );
        assert!(
            saw_total.load(Ordering::Relaxed),
            "progress should reach completion (done == total)"
        );
    }

    #[test]
    fn selective_skips_listed_file() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        let zip_path = tmp.path().join("s.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("MyAddon/a.lua", options).unwrap();
        archive.write_all(b"a").unwrap();
        archive.start_file("MyAddon/b.lua", options).unwrap();
        archive.write_all(b"b").unwrap();
        archive.finish().unwrap();

        let mut skip = HashSet::new();
        skip.insert("MyAddon/b.lua".to_string());
        extract_addon_zip_selective(&zip_path, &addons_dir, &skip).unwrap();

        assert!(addons_dir.join("MyAddon/a.lua").exists());
        assert!(
            !addons_dir.join("MyAddon/b.lua").exists(),
            "a skipped (keep-mine) file must not be overwritten"
        );
    }
}
