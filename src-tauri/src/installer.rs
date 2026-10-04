use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use typed_path::{Utf8WindowsComponent, Utf8WindowsPath};

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
    /// [`CANCELLED`] and the staged changes are discarded before publication.
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

/// Build the complete replacement on the destination volume before publishing.
/// Cancellation, invalid ZIPs and copy failures leave every live byte unchanged.
/// A failed publish restores all folders already swapped; failed rollback retains
/// its originals on disk and names their location in the error.
fn extract_with_rollback(
    zip_path: &Path,
    addons_dir: &Path,
    skip_files: &HashSet<String>,
    hooks: ExtractHooks,
) -> Result<Vec<String>, String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("Failed to open ZIP file: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Failed to read ZIP archive: {e}"))?;
    fs::create_dir_all(addons_dir).map_err(|e| describe_write_error(addons_dir, &e))?;
    // Keep the lock file: unlinking an advisory lock permits two processes to
    // lock different inodes at the same name. Closing this handle releases it.
    let lock_path = addons_dir.join(".kalpa-install.lock");
    reject_link_if_present(&lock_path)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| describe_write_error(&lock_path, &e))?;
    lock.lock()
        .map_err(|e| format!("Failed to lock addon installation: {e}"))?;
    if is_cancelled(&hooks) {
        return Err(CANCELLED.into());
    }
    let wrap_name = flat_archive_wrap_name(&archive);
    let top_level = match wrap_name {
        Some(ref name) => HashSet::from([name.clone()]),
        None => collect_zip_top_folders(&archive),
    };
    if top_level
        .iter()
        .any(|name| name.to_ascii_lowercase().starts_with(".kalpa-install"))
    {
        return Err("ZIP contains a reserved Kalpa installation path.".into());
    }
    let transaction = tempfile::Builder::new()
        .prefix(".kalpa-install-")
        .tempdir_in(addons_dir)
        .map_err(|e| format!("Create installation staging: {e}"))?;
    let staged = transaction.path().join("new");
    let original = transaction.path().join("old");
    fs::create_dir(&staged).map_err(|e| describe_write_error(&staged, &e))?;
    fs::create_dir(&original).map_err(|e| describe_write_error(&original, &e))?;
    for name in &top_level {
        copy_existing_tree(&addons_dir.join(name), &staged.join(name), &hooks)?;
    }
    let folders = extract_addon_zip_inner(
        &mut archive,
        &staged,
        skip_files,
        hooks,
        wrap_name.as_deref(),
    )?;
    if is_cancelled(&hooks) {
        return Err(CANCELLED.into());
    }
    // Publication is deliberately not cancellable between renames. A Stop is
    // observed before this short critical section, never halfway through it.
    let mut names = folders.clone();
    names.sort();
    let mut published: Vec<(String, bool)> = Vec::new();
    let result = (|| {
        for name in names {
            let live = addons_dir.join(&name);
            reject_link_if_present(&live)?;
            let existed = match fs::symlink_metadata(&live) {
                Ok(_) => true,
                Err(e) if e.kind() == io::ErrorKind::NotFound => false,
                Err(e) => return Err(describe_write_error(&live, &e)),
            };
            if existed {
                fs::rename(&live, original.join(&name))
                    .map_err(|e| describe_write_error(&live, &e))?;
            }
            // Record the old move before publishing so even this rename's
            // failure restores the original name.
            published.push((name.clone(), existed));
            fs::rename(staged.join(&name), &live).map_err(|e| describe_write_error(&live, &e))?;
        }
        Ok::<_, String>(())
    })();
    if let Err(error) = result {
        let mut rollback_errors = Vec::new();
        for (name, existed) in published.into_iter().rev() {
            let live = addons_dir.join(&name);
            // If the staged entry still exists, publication failed before it
            // reached the live path: do not delete something we didn't publish.
            let undo = (|| {
                if !staged.join(&name).exists() {
                    let metadata = fs::symlink_metadata(&live)?;
                    if metadata.is_dir() {
                        fs::remove_dir_all(&live)?;
                    } else {
                        fs::remove_file(&live)?;
                    }
                }
                if existed {
                    fs::rename(original.join(&name), &live)?;
                }
                Ok::<_, io::Error>(())
            })();
            if let Err(e) = undo {
                rollback_errors.push(format!("{name}: {e}"));
            }
        }
        if !rollback_errors.is_empty() {
            let recovery = transaction.keep();
            return Err(format!(
                "{error}. Rollback incomplete ({}). Original files retained at {}",
                rollback_errors.join("; "),
                recovery.join("old").display()
            ));
        }
        return Err(error);
    }
    Ok(folders)
}

fn reject_link_if_present(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("Inspect {}: {e}", path.display())),
    };
    let link = metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            // FILE_ATTRIBUTE_REPARSE_POINT
            return Err(format!("Refusing linked addon path: {}", path.display()));
        }
    }
    if link {
        return Err(format!("Refusing linked addon path: {}", path.display()));
    }
    Ok(())
}

fn copy_existing_tree(source: &Path, target: &Path, hooks: &ExtractHooks) -> Result<(), String> {
    if is_cancelled(hooks) {
        return Err(CANCELLED.into());
    }
    reject_link_if_present(source)?;
    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("Inspect {}: {e}", source.display())),
    };
    if metadata.is_file() {
        // A fresh file breaks existing hard links: replacing an addon must not
        // change an unrelated file that happens to share its old inode.
        fs::copy(source, target).map_err(|e| describe_write_error(target, &e))?;
    } else if metadata.is_dir() {
        fs::create_dir(target).map_err(|e| describe_write_error(target, &e))?;
        for entry in fs::read_dir(source).map_err(|e| format!("Read {}: {e}", source.display()))? {
            let entry = entry.map_err(|e| format!("Read {}: {e}", source.display()))?;
            copy_existing_tree(&entry.path(), &target.join(entry.file_name()), hooks)?;
        }
    } else {
        return Err(format!("Unsupported addon file: {}", source.display()));
    }
    Ok(())
}

/// The first path component of a ZIP entry name, or `None` when the name is
/// unsafe. Replicates `ZipFile::enclosed_name` exactly — including its
/// `Utf8WindowsPath` parsing, which on every platform splits on `\` as well as
/// `/`, strips a *leading* drive prefix or root, and rejects the whole name on
/// NUL bytes, a mid-path prefix/root, or `..` traversal that escapes the
/// archive root — then takes the first component of the simplified path. The
/// extraction loop derives its created-folder names from `enclosed_name`, so
/// any divergence here could make rollback miss (or mistarget) a folder the
/// extractor actually created.
///
/// Used by the rollback passes, which only need top-level folder names and can
/// therefore read them straight from the central directory (`file_names`)
/// instead of paying a per-entry local-header read via `by_index`.
fn enclosed_top_component(name: &str) -> Option<String> {
    enclosed_components(name)?.into_iter().next()
}

/// Every normalized component of a contained archive path, or `None` when the
/// name escapes its root. Shared with [`enclosed_top_component`] so the depth
/// rules that keep extraction contained have exactly one definition.
fn enclosed_components(name: &str) -> Option<Vec<String>> {
    if name.contains('\0') {
        return None;
    }
    let mut depth = 0usize;
    let mut components: Vec<&str> = Vec::new();
    for component in Utf8WindowsPath::new(name).components() {
        match component {
            Utf8WindowsComponent::Prefix(_) | Utf8WindowsComponent::RootDir => {
                if depth > 0 {
                    return None;
                }
            }
            Utf8WindowsComponent::ParentDir => {
                depth = depth.checked_sub(1)?;
                components.pop();
            }
            Utf8WindowsComponent::Normal(s) => {
                depth += 1;
                components.push(s);
            }
            Utf8WindowsComponent::CurDir => (),
        }
    }
    Some(components.into_iter().map(|s| s.to_string()).collect())
}

/// Collect top-level folder names from a ZIP archive's central directory.
fn collect_zip_top_folders(archive: &zip::ZipArchive<fs::File>) -> HashSet<String> {
    let mut folders = HashSet::new();
    for name in archive.file_names() {
        if let Some(folder) = enclosed_top_component(name) {
            folders.insert(folder);
        }
    }
    folders
}

/// The folder a FLAT archive's contents must be wrapped in, if it is flat.
///
/// ESO loads an addon from `AddOns/<Name>/<Name>.txt`, so an archive whose files
/// sit at its own root has no valid destination as-is. Zipping an addon's
/// *contents* rather than its folder is a common authoring mistake, and extracting
/// one verbatim scatters loose files across the AddOns root and installs something
/// the game will never load. UL_LootLog did exactly this: four files plus a
/// `bindings/` folder landed beside every other addon, and the hash step then
/// reported "Addon path is not a directory" for each loose file.
///
/// The signal is a manifest at the archive root: ESO requires the folder name to
/// equal the manifest name, so a root `<Name>.txt` means the root IS the addon
/// folder's contents. Anything else — including an archive with a stray readme
/// beside a proper addon folder — is left alone, because wrapping there would
/// nest a correct addon one level too deep.
///
/// Exposed to `file_hashes` so conflict detection keys a flat archive's entries
/// the same way extraction lays them out; a mismatch there silently reports zero
/// conflicts and overwrites the user's edits.
pub(crate) fn flat_archive_wrap_name(archive: &zip::ZipArchive<fs::File>) -> Option<String> {
    // A top-level directory that is already a proper addon settles the question:
    // the archive is foldered and its root files are ancillary. `readme.txt`,
    // `Changelog.txt` and `LICENSE.txt` beside a real addon folder are common in
    // author-made ESOUI zips, and treating one as the manifest would bury the
    // addon at AddOns/readme/MyAddon/… and break update tracking permanently.
    if contains_foldered_addon(archive) {
        return None;
    }

    let mut manifest_stems: Vec<String> = Vec::new();
    let mut root_stems: HashSet<String> = HashSet::new();

    for name in archive.file_names() {
        // A trailing separator marks a directory entry; those are not root files.
        if name.ends_with('/') || name.ends_with('\\') {
            continue;
        }
        let Some(components) = enclosed_components(name) else {
            continue;
        };
        // Exactly one component == the entry sits at the archive root.
        let [single] = components.as_slice() else {
            continue;
        };
        let (stem, ext) = match single.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s, e),
            _ => continue,
        };
        root_stems.insert(stem.to_string());
        if ext.eq_ignore_ascii_case("txt") {
            manifest_stems.push(stem.to_string());
        }
    }

    // Require a manifest at the root. It is the only trustworthy evidence that the
    // root IS an addon folder's contents, and ESO requires the destination folder
    // to carry that exact name. Deriving a name from anything else (the archive's
    // own file name, say) risks a versioned folder like "UL_LootLog-1.2" that the
    // game silently refuses to load — worse than not wrapping at all.
    let chosen = match manifest_stems.as_slice() {
        [only] => only.clone(),
        [] => return None,
        many => {
            // Several root .txt files: a manifest plus a readme, most likely. The
            // manifest is the one sharing its name with another root file
            // (`UL_LootLog.txt` beside `UL_LootLog.lua`). If that is still
            // ambiguous, leave the archive alone rather than guess a folder name.
            let mut matched = many
                .iter()
                .filter(|s| root_stems.contains(*s) && has_sibling_with_stem(archive, s));
            let first = matched.next()?;
            if matched.next().is_some() {
                return None;
            }
            first.clone()
        }
    };
    sanitize_wrap_name(&chosen)
}

/// Whether any top-level directory in the archive is itself a proper addon —
/// `<Dir>/<Dir>.txt` among the entries, the exact shape ESO loads.
fn contains_foldered_addon(archive: &zip::ZipArchive<fs::File>) -> bool {
    archive.file_names().any(|name| {
        let Some(components) = enclosed_components(name) else {
            return false;
        };
        let [dir, file] = components.as_slice() else {
            return false;
        };
        matches!(
            file.rsplit_once('.'),
            Some((stem, ext)) if ext.eq_ignore_ascii_case("txt") && stem.eq_ignore_ascii_case(dir)
        )
    })
}

/// Whether a root-level file other than `<stem>.txt` also carries `stem` — the
/// signal that `stem` names the addon rather than a stray readme.
fn has_sibling_with_stem(archive: &zip::ZipArchive<fs::File>, stem: &str) -> bool {
    archive.file_names().any(|name| {
        enclosed_components(name)
            .filter(|c| c.len() == 1)
            .and_then(|c| c.into_iter().next())
            .and_then(|f| {
                f.rsplit_once('.')
                    .map(|(s, e)| (s.to_string(), e.to_string()))
            })
            .is_some_and(|(s, e)| s == stem && !e.eq_ignore_ascii_case("txt"))
    })
}

/// A wrap folder name must be a single, ordinary path component. Anything with a
/// separator, a traversal segment, or a NUL would defeat the containment that
/// `enclosed_name` provides for the entries themselves.
fn sanitize_wrap_name(name: &str) -> Option<String> {
    let trimmed = name.trim().trim_end_matches('.');
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains('\0')
        || trimmed.contains(':')
    {
        return None;
    }
    Some(trimmed.to_string())
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
    wrap_name: Option<&str>,
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
        let enclosed = match entry.enclosed_name() {
            Some(p) => p.to_owned(),
            None => continue,
        };

        // A flat archive's entries are re-rooted under the addon's own folder, so
        // its files land at AddOns/<Name>/… instead of loose in the AddOns root.
        // The wrap is applied AFTER enclosed_name, so containment is unaffected —
        // it only ever adds a leading component, and the name itself was validated
        // by `sanitize_wrap_name`.
        let relative_path = match wrap_name {
            Some(name) => Path::new(name).join(&enclosed),
            None => enclosed.clone(),
        };

        // Honor "keep mine" conflict decisions (no-op when skip_files is empty).
        // Callers key skip entries by `<folder>/<folder-relative path>`. For a
        // foldered archive that IS the archive-relative path; for a wrapped flat
        // archive the folder exists only after wrapping, so the caller's key
        // carries a prefix the entry lacks — match the wrapped form too, or every
        // kept file in a flat archive is silently overwritten.
        let key = enclosed.to_string_lossy().replace('\\', "/");
        let wrapped_key = wrap_name.map(|name| format!("{name}/{key}"));
        if skip_files.contains(&key)
            || wrapped_key
                .as_deref()
                .is_some_and(|k| skip_files.contains(k))
        {
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

    /// The rollback passes derive top-level names from central-directory
    /// strings via `enclosed_top_component`, while the extraction loop derives
    /// them from `entry.enclosed_name()`. This proves the two agree for every
    /// shape of entry name, including hostile ones (absolute paths, Windows
    /// drive prefixes, backslash separators, `..` traversal).
    #[test]
    fn enclosed_top_component_matches_enclosed_name_first_component() {
        let names = [
            "MyAddon/file.lua",
            "MyAddon/sub/deep/file.lua",
            "toplevel.lua",
            "MyAddon\\file.lua",
            "C:\\MyAddon\\file.lua",
            "C:/MyAddon/file.lua",
            "/abs/file.lua",
            "../escape.lua",
            "foo/../bar.lua",
            "foo/../../escape.lua",
            "./MyAddon/file.lua",
            "MyAddon/./file.lua",
        ];
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let options = zip::write::SimpleFileOptions::default();
            for n in &names {
                w.start_file(*n, options).unwrap();
            }
            w.finish().unwrap();
        }
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(buf)).unwrap();
        for i in 0..archive.len() {
            let entry = archive.by_index(i).unwrap();
            let name = entry.name().to_string();
            let expected = entry.enclosed_name().and_then(|p| {
                p.components()
                    .next()
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
            });
            assert_eq!(
                enclosed_top_component(&name),
                expected,
                "rollback and extraction disagree on top-level name for {name:?}"
            );
        }
    }

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

    /// Create a ZIP from an explicit list of entry paths, so a test can express a
    /// FLAT archive (files at the archive root) that no folder-based helper can.
    fn create_zip_with_entries(dir: &Path, zip_name: &str, entries: &[&str]) -> PathBuf {
        let zip_path = dir.join(zip_name);
        let file = fs::File::create(&zip_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for e in entries {
            archive.start_file(*e, options).unwrap();
            archive.write_all(b"x").unwrap();
        }
        archive.finish().unwrap();
        zip_path
    }

    /// The UL_LootLog shape: the addon's *contents* were zipped instead of its
    /// folder, so four files and a `bindings/` folder sat at the archive root.
    /// Extracted verbatim they scattered across the AddOns root and the addon
    /// could not load, because ESO only reads AddOns/<Name>/<Name>.txt.
    #[test]
    fn flat_archive_is_wrapped_in_the_addon_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        let zip_path = create_zip_with_entries(
            tmp.path(),
            "UL_LootLog.zip",
            &[
                "UL_LootLog.txt",
                "UL_LootLog.lua",
                "UL_LootLog.xml",
                "bindings/Bindings.lua",
            ],
        );

        let folders = extract_addon_zip(&zip_path, &addons_dir).unwrap();
        assert_eq!(folders, vec!["UL_LootLog".to_string()]);

        assert!(addons_dir.join("UL_LootLog/UL_LootLog.txt").is_file());
        assert!(addons_dir.join("UL_LootLog/UL_LootLog.lua").is_file());
        assert!(addons_dir
            .join("UL_LootLog/bindings/Bindings.lua")
            .is_file());

        // Nothing may be left loose in the AddOns root.
        assert!(!addons_dir.join("UL_LootLog.txt").exists());
        assert!(!addons_dir.join("bindings").exists());
    }

    /// A correctly authored archive must be untouched — wrapping one would nest a
    /// working addon a level too deep and break every install.
    #[test]
    fn a_normal_foldered_archive_is_not_wrapped() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        let zip_path = create_zip_with_entries(
            tmp.path(),
            "Thing.zip",
            &["MyAddon/MyAddon.txt", "MyAddon/core.lua"],
        );

        let folders = extract_addon_zip(&zip_path, &addons_dir).unwrap();
        assert_eq!(folders, vec!["MyAddon".to_string()]);
        assert!(addons_dir.join("MyAddon/MyAddon.txt").is_file());
        assert!(!addons_dir.join("MyAddon/MyAddon").exists());
    }

    /// A stray file beside a proper addon folder is NOT the flat shape: there is no
    /// root manifest naming an addon, so the folder must be left where it is.
    #[test]
    fn a_stray_readme_beside_a_folder_does_not_trigger_wrapping() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        let zip_path = create_zip_with_entries(
            tmp.path(),
            "Thing.zip",
            &["readme.md", "MyAddon/MyAddon.txt"],
        );

        extract_addon_zip(&zip_path, &addons_dir).unwrap();
        assert!(addons_dir.join("MyAddon/MyAddon.txt").is_file());
    }

    /// A root `README.txt`/`Changelog.txt`/`LICENSE.txt` beside a proper addon
    /// folder is a readme, not a manifest. Wrapping on it buried the real addon at
    /// `AddOns/readme/MyAddon/…`, recorded the install under the readme's name, and
    /// broke update tracking permanently.
    #[test]
    fn a_root_readme_txt_beside_a_folder_does_not_trigger_wrapping() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        let zip_path = create_zip_with_entries(
            tmp.path(),
            "Thing.zip",
            &["README.txt", "MyAddon/MyAddon.txt", "MyAddon/core.lua"],
        );

        extract_addon_zip(&zip_path, &addons_dir).unwrap();
        assert!(addons_dir.join("MyAddon/MyAddon.txt").is_file());
        assert!(!addons_dir.join("README").exists());
    }

    /// The same shape with corroboration for the readme stem (`Changelog.txt`
    /// beside `Changelog.md`), which used to satisfy the wrap heuristic outright.
    #[test]
    fn a_changelog_pair_beside_a_folder_does_not_trigger_wrapping() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        let zip_path = create_zip_with_entries(
            tmp.path(),
            "Thing.zip",
            &["Changelog.txt", "Changelog.md", "MyAddon/MyAddon.txt"],
        );

        extract_addon_zip(&zip_path, &addons_dir).unwrap();
        assert!(addons_dir.join("MyAddon/MyAddon.txt").is_file());
        assert!(!addons_dir.join("Changelog").exists());
    }

    /// Several root `.txt` files beside a proper addon folder: the multi-manifest
    /// branch must refuse too, not pick whichever readme has a same-stem sibling.
    #[test]
    fn multiple_root_txt_files_beside_a_folder_do_not_trigger_wrapping() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        let zip_path = create_zip_with_entries(
            tmp.path(),
            "Thing.zip",
            &[
                "README.txt",
                "LICENSE.txt",
                "README.md",
                "MyAddon/MyAddon.txt",
            ],
        );

        extract_addon_zip(&zip_path, &addons_dir).unwrap();
        assert!(addons_dir.join("MyAddon/MyAddon.txt").is_file());
        assert!(!addons_dir.join("README").exists());
        assert!(!addons_dir.join("LICENSE").exists());
    }

    /// "Keep mine" decisions are keyed `<folder>/<path>`, but a flat archive's
    /// entries have no folder component until they are wrapped. The extractor must
    /// match the wrapped key or every kept file in a flat archive is overwritten.
    #[test]
    fn wrapped_flat_archive_honors_folder_prefixed_skip_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();
        fs::create_dir_all(addons_dir.join("UL_LootLog")).unwrap();
        fs::write(addons_dir.join("UL_LootLog/UL_LootLog.lua"), "USER EDIT").unwrap();

        let zip_path = create_zip_with_entries(
            tmp.path(),
            "UL_LootLog.zip",
            &["UL_LootLog.txt", "UL_LootLog.lua"],
        );

        let mut skip = HashSet::new();
        skip.insert("UL_LootLog/UL_LootLog.lua".to_string());
        extract_addon_zip_selective(&zip_path, &addons_dir, &skip).unwrap();

        assert!(addons_dir.join("UL_LootLog/UL_LootLog.txt").is_file());
        assert_eq!(
            fs::read_to_string(addons_dir.join("UL_LootLog/UL_LootLog.lua")).unwrap(),
            "USER EDIT"
        );
    }

    #[test]
    fn a_wrap_name_may_never_escape_the_addons_dir() {
        assert_eq!(sanitize_wrap_name("UL_LootLog"), Some("UL_LootLog".into()));
        for bad in ["..", ".", "", "   ", "a/b", "a\\b", "C:", "x\0y"] {
            assert_eq!(sanitize_wrap_name(bad), None, "must reject {bad:?}");
        }
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
        // A cancellation must preserve the complete old version byte for byte.
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
            assert_eq!(
                fs::read(existing.join(format!("file{n}.lua"))).unwrap(),
                b"OLD"
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

    #[test]
    fn failed_update_preserves_all_existing_bytes_and_no_new_files() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("AddOns");
        fs::create_dir_all(addons.join("A")).unwrap();
        fs::create_dir_all(addons.join("B")).unwrap();
        fs::write(addons.join("A/old.lua"), b"old version").unwrap();
        fs::write(addons.join("B/sub"), b"existing file").unwrap();
        let path = tmp.path().join("update.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for name in ["A/old.lua", "A/new.lua", "B/sub/fail.lua"] {
            zip.start_file(name, options).unwrap();
            zip.write_all(b"new version").unwrap();
        }
        zip.finish().unwrap();
        assert!(extract_addon_zip(&path, &addons).is_err());
        assert_eq!(fs::read(addons.join("A/old.lua")).unwrap(), b"old version");
        assert!(!addons.join("A/new.lua").exists());
        assert_eq!(fs::read(addons.join("B/sub")).unwrap(), b"existing file");
    }

    #[test]
    fn update_breaks_hard_links_without_modifying_external_file() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("AddOns");
        fs::create_dir_all(addons.join("MyAddon")).unwrap();
        let outside = tmp.path().join("outside.lua");
        fs::write(&outside, b"external").unwrap();
        fs::hard_link(&outside, addons.join("MyAddon/file0.lua")).unwrap();
        let archive = create_multi_file_zip(tmp.path(), "update.zip", "MyAddon", 1);
        extract_addon_zip(&archive, &addons).unwrap();
        assert_eq!(fs::read(&outside).unwrap(), b"external");
        assert_eq!(
            fs::read(addons.join("MyAddon/file0.lua")).unwrap(),
            b"-- lua"
        );
    }

    fn link_directory(source: &Path, target: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(source, target).unwrap();
        #[cfg(windows)]
        {
            let result = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(target.to_string_lossy().replace('/', "\\"))
                .arg(source.to_string_lossy().replace('/', "\\"))
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }

    #[test]
    fn update_rejects_nested_linked_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("AddOns");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(addons.join("MyAddon")).unwrap();
        fs::write(outside.join("file0.lua"), b"external").unwrap();
        link_directory(&outside, &addons.join("MyAddon/sub"));
        let archive = create_multi_file_zip(tmp.path(), "update.zip", "MyAddon/sub", 1);
        assert!(extract_addon_zip(&archive, &addons)
            .unwrap_err()
            .contains("linked addon path"));
        assert_eq!(fs::read(outside.join("file0.lua")).unwrap(), b"external");
    }

    #[test]
    fn publication_failure_rolls_back_previous_addons() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("AddOns");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(addons.join("A")).unwrap();
        fs::write(addons.join("A/file.lua"), b"old").unwrap();
        let path = tmp.path().join("update.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for name in ["A/file.lua", "B/file.lua"] {
            zip.start_file(name, options).unwrap();
            zip.write_all(b"new").unwrap();
        }
        zip.finish().unwrap();
        let cb = |done, total| {
            if done == total {
                link_directory(&outside, &addons.join("B"));
            }
        };
        let hooks = ExtractHooks {
            cancel: None,
            progress: Some(&cb),
        };
        assert!(extract_addon_zip_with(&path, &addons, hooks).is_err());
        assert_eq!(fs::read(addons.join("A/file.lua")).unwrap(), b"old");
        assert!(!outside.join("file.lua").exists());
    }

    #[test]
    fn extracted_binary_signature_matches_zip_signature_end_to_end() {
        // The size-signature optimization relies on a ZIP entry's uncompressed
        // size (used as the ZIP-side signature) equalling the byte length written
        // to disk by extraction. Assert that invariant through a real extract,
        // not just two size strings: a clean media update must not flag every
        // texture as size-changed on the next scan.
        use crate::file_hashes::{compute_addon_hashes, hash_zip_entries};

        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        let zip_path = tmp.path().join("media.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        archive
            .start_file("MediaAddon/icons/a.dds", options)
            .unwrap();
        archive
            .write_all(b"\x00\x01\x02texture-bytes\xff\xfe")
            .unwrap();
        archive.finish().unwrap();

        extract_addon_zip_with(&zip_path, &addons_dir, ExtractHooks::NONE).unwrap();

        let disk = compute_addon_hashes(&addons_dir.join("MediaAddon")).unwrap();
        let zip_hashes = hash_zip_entries(&zip_path, "MediaAddon").unwrap();
        assert!(
            zip_hashes["icons/a.dds"].starts_with("size:"),
            "a .dds entry must use a size signature, got: {}",
            zip_hashes["icons/a.dds"]
        );
        assert_eq!(
            disk["icons/a.dds"], zip_hashes["icons/a.dds"],
            "extracted .dds size signature must equal the ZIP-side signature"
        );
    }
}
