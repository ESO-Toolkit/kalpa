//! Settings-only ZIP backups. A format marker distinguishes these from the
//! broader migration snapshots stored alongside them.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const MARKER: &str = ".kalpa-backup.json";
const FORMAT: &str = "kalpa-saved-variables";

#[derive(Serialize, Deserialize)]
struct Marker {
    format: String,
    version: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ArchiveStats {
    pub file_count: u32,
    pub total_size: u64,
}

fn member_name(name: &str) -> Result<&str, String> {
    let leaf = name
        .strip_prefix("SavedVariables/")
        .ok_or_else(|| format!("Unexpected backup entry: {name}"))?;
    let stem = leaf.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    if leaf.is_empty()
        || leaf.starts_with('.')
        || leaf.contains("..")
        || leaf
            .chars()
            .any(|c| c.is_control() || "\\/<>:\"|?*".contains(c))
        || leaf.ends_with(['.', ' '])
        || reserved
    {
        return Err(format!("Unsafe backup entry: {name}"));
    }
    Ok(leaf)
}

fn open_settings_archive(path: &Path) -> Result<Option<ZipArchive<File>>, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open backup: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("Invalid backup ZIP: {e}"))?;
    let marker = {
        let mut entry = match archive.by_name(MARKER) {
            Ok(entry) => entry,
            Err(zip::result::ZipError::FileNotFound) => return Ok(None),
            Err(e) => return Err(format!("Failed to read backup format: {e}")),
        };
        if entry.size() > 4096 {
            return Err("Invalid backup format marker.".to_string());
        }
        let mut bytes = Vec::new();
        (&mut entry)
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Failed to read backup format: {e}"))?;
        if bytes.len() > 4096 {
            return Err("Invalid backup format marker.".to_string());
        }
        serde_json::from_slice::<Marker>(&bytes)
            .map_err(|e| format!("Invalid backup format marker: {e}"))?
    };
    if marker.format != FORMAT || marker.version != 1 {
        return Err("Unsupported settings backup format.".to_string());
    }
    Ok(Some(archive))
}

/// Validate the complete directory before allocating or restoring any data.
fn archive_stats(archive: &mut ZipArchive<File>) -> Result<ArchiveStats, String> {
    let mut stats = ArchiveStats {
        file_count: 0,
        total_size: 0,
    };
    let mut names = HashSet::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|e| format!("Invalid backup entry: {e}"))?;
        if entry.name() == MARKER {
            continue;
        }
        let name = member_name(entry.name())?;
        if entry.is_dir() || entry.is_symlink() || !names.insert(name.to_lowercase()) {
            return Err(format!(
                "Invalid or duplicate backup entry: {}",
                entry.name()
            ));
        }
        stats.file_count = stats
            .file_count
            .checked_add(1)
            .ok_or("Too many backup files.")?;
        stats.total_size = stats
            .total_size
            .checked_add(entry.size())
            .ok_or("Backup is too large.")?;
    }
    Ok(stats)
}

/// `None` means this ZIP belongs to a different backup system.
pub fn inspect(path: &Path) -> Result<Option<ArchiveStats>, String> {
    open_settings_archive(path)?
        .map(|mut archive| archive_stats(&mut archive))
        .transpose()
}

/// Buffer and verify every entry (including CRC) before the caller writes live settings.
pub fn read(path: &Path, max_bytes: u64) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut archive = open_settings_archive(path)?.ok_or(
        "This ZIP is not a settings backup. Restore automatic snapshots from Safety Center.",
    )?;
    let stats = archive_stats(&mut archive)?;
    if stats.total_size > max_bytes {
        return Err(
            "Backup is too large to restore safely. Extract it manually instead.".to_string(),
        );
    }
    let mut buffered = Vec::new();
    let mut remaining = max_bytes;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("Invalid backup entry: {e}"))?;
        if entry.name() == MARKER {
            continue;
        }
        let name = member_name(entry.name())?.to_string();
        let expected = entry.size();
        let mut bytes = Vec::new();
        (&mut entry)
            .take(remaining.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Failed to read {name} from backup: {e}"))?;
        if bytes.len() as u64 > remaining || bytes.len() as u64 != expected {
            return Err(format!("Invalid or oversized backup entry: {name}"));
        }
        remaining -= bytes.len() as u64;
        buffered.push((name, bytes));
    }
    Ok(buffered)
}

/// Publish only a finished, synced archive. Failures leave no visible partial
/// backup, and publication never replaces an existing file.
pub fn create(sv_dir: &Path, destination: &Path) -> Result<ArchiveStats, String> {
    create_inner(sv_dir, destination, false)
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConsolidationOutcome {
    Copied,
    AlreadyPresent,
}

/// Keep legacy folders untouched. A retry verifies an existing copy instead of
/// overwriting it or adding another archive. Verification streams file contents
/// so consolidation does not require a second in-memory copy of the backup.
pub fn consolidate_folder(
    source: &Path,
    destination: &Path,
) -> Result<ConsolidationOutcome, String> {
    if destination.exists() {
        verify_folder_archive(source, destination, folder_files(source)?)
            .map_err(|e| format!("An existing ZIP could not be verified against this folder: {e} No files were replaced."))?;
        return Ok(ConsolidationOutcome::AlreadyPresent);
    }
    create_inner(source, destination, true)?;
    Ok(ConsolidationOutcome::Copied)
}

fn folder_files(source: &Path) -> Result<BTreeMap<String, fs::Metadata>, String> {
    let root = fs::symlink_metadata(source)
        .map_err(|e| format!("Failed to inspect backup folder: {e}"))?;
    if !root.is_dir() || root.file_type().is_symlink() {
        return Err("Linked backup folders cannot be consolidated safely.".to_string());
    }
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(source).map_err(|e| format!("Failed to read backup folder: {e}"))? {
        let entry = entry.map_err(|e| format!("Failed to read backup entry: {e}"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Unsupported backup filename.")?;
        if name.starts_with('.') {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|e| format!("Failed to inspect {name}: {e}"))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(format!(
                "{name} is a subfolder or linked file. Keep using the original folder."
            ));
        }
        member_name(&format!("SavedVariables/{name}"))?;
        files.insert(name, metadata);
    }
    Ok(files)
}

fn verify_folder_archive(
    source: &Path,
    archive_path: &Path,
    files: BTreeMap<String, fs::Metadata>,
) -> Result<(), String> {
    let mut archive = open_settings_archive(archive_path)?
        .ok_or("The existing ZIP belongs to another backup system.")?;
    let stats = archive_stats(&mut archive)?;
    if stats.file_count as usize != files.len() {
        return Err("The folder and ZIP contain different files.".to_string());
    }
    let mut archive_buffer = [0; 64 * 1024];
    let mut source_buffer = [0; 64 * 1024];
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("Failed to verify ZIP: {e}"))?;
        if entry.name() == MARKER {
            continue;
        }
        let name = member_name(entry.name())?.to_string();
        let metadata = files
            .get(&name)
            .ok_or_else(|| format!("The folder no longer contains {name}."))?;
        if metadata.len() != entry.size() {
            return Err(format!("{name} differs from the ZIP."));
        }
        let mut original =
            File::open(source.join(&name)).map_err(|e| format!("Failed to verify {name}: {e}"))?;
        loop {
            let count = entry
                .read(&mut archive_buffer)
                .map_err(|e| format!("ZIP verification failed for {name}: {e}"))?;
            if count == 0 {
                break;
            }
            original
                .read_exact(&mut source_buffer[..count])
                .map_err(|e| format!("Backup folder changed while verifying {name}: {e}"))?;
            if archive_buffer[..count] != source_buffer[..count] {
                return Err(format!(
                    "{name} differs from the ZIP. The original folder was kept."
                ));
            }
        }
        if original
            .read(&mut source_buffer[..1])
            .map_err(|e| format!("Failed to verify {name}: {e}"))?
            != 0
        {
            return Err(format!("{name} changed during verification."));
        }
    }
    let after = folder_files(source)?;
    if files.len() != after.len()
        || files.iter().any(|(name, before)| {
            after.get(name).is_none_or(|now| {
                before.len() != now.len() || before.modified().ok() != now.modified().ok()
            })
        })
    {
        return Err(
            "The backup folder changed during verification. Retry after it stops changing."
                .to_string(),
        );
    }
    Ok(())
}

fn create_inner(
    sv_dir: &Path,
    destination: &Path,
    verify_source: bool,
) -> Result<ArchiveStats, String> {
    // Refuse unsupported trees before staging; never silently omit their data.
    let source_files = verify_source.then(|| folder_files(sv_dir)).transpose()?;
    let root = destination.parent().ok_or("Invalid backup destination.")?;
    fs::create_dir_all(root).map_err(|e| format!("Failed to create backups folder: {e}"))?;
    let mut staging = tempfile::Builder::new()
        .prefix(".tmp-manual-")
        .suffix(".zip")
        .tempfile_in(root)
        .map_err(|e| format!("Failed to stage backup: {e}"))?;
    let mut stats = ArchiveStats {
        file_count: 0,
        total_size: 0,
    };
    let mut names = HashSet::new();
    {
        let mut writer = ZipWriter::new(staging.as_file_mut());
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(6));
        writer
            .start_file(MARKER, options)
            .map_err(|e| format!("Failed to start backup: {e}"))?;
        let marker = Marker {
            format: FORMAT.to_string(),
            version: 1,
        };
        writer
            .write_all(&serde_json::to_vec(&marker).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Failed to write backup format: {e}"))?;
        for entry in
            fs::read_dir(sv_dir).map_err(|e| format!("Failed to read SavedVariables: {e}"))?
        {
            let entry = entry.map_err(|e| format!("Failed to read SavedVariables entry: {e}"))?;
            if !entry
                .file_type()
                .map_err(|e| format!("Failed to inspect settings file: {e}"))?
                .is_file()
            {
                continue;
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "Unsupported settings filename.")?;
            if name.starts_with('.') {
                continue;
            }
            let archive_name = format!("SavedVariables/{name}");
            member_name(&archive_name)?;
            if !names.insert(name.to_lowercase()) {
                return Err(format!("Duplicate settings filename: {name}"));
            }
            let mut source =
                File::open(entry.path()).map_err(|e| format!("Failed to open {name}: {e}"))?;
            writer
                .start_file(archive_name, options)
                .map_err(|e| format!("Failed to archive {name}: {e}"))?;
            let copied = std::io::copy(&mut source, &mut writer)
                .map_err(|e| format!("Failed to archive {name}: {e}"))?;
            stats.file_count += 1;
            stats.total_size += copied;
        }
        writer
            .finish()
            .map_err(|e| format!("Failed to finish backup: {e}"))?;
    }
    if let Some(source_files) = source_files {
        verify_folder_archive(sv_dir, staging.path(), source_files)?;
        let modified = fs::metadata(sv_dir)
            .and_then(|m| m.modified())
            .map_err(|e| format!("Failed to preserve backup date: {e}"))?;
        staging
            .as_file()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .map_err(|e| format!("Failed to preserve backup date: {e}"))?;
    }
    staging
        .as_file()
        .sync_all()
        .map_err(|e| format!("Failed to sync backup: {e}"))?;
    staging
        .persist_noclobber(destination)
        .map_err(|e| format!("Failed to publish backup: {}", e.error))?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked_archive(path: &Path, entries: &[(&str, &[u8])]) {
        let mut writer = ZipWriter::new(File::create(path).unwrap());
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.start_file(MARKER, options).unwrap();
        writer
            .write_all(br#"{"format":"kalpa-saved-variables","version":1}"#)
            .unwrap();
        for (name, bytes) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn settings_zip_roundtrips_raw_bytes_and_compresses() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("SavedVariables");
        fs::create_dir(&source).unwrap();
        let contents = vec![b'x'; 64 * 1024];
        fs::write(source.join("Example.lua"), &contents).unwrap();
        fs::write(source.join("Example.lua.bak"), b"previous").unwrap();
        fs::write(source.join(".internal"), b"metadata").unwrap();
        fs::create_dir(source.join("subfolder")).unwrap();
        let archive = temp.path().join("KalpaBackups/manual.zip");
        let stats = create(&source, &archive).unwrap();
        assert_eq!(stats.file_count, 2);
        assert_eq!(stats.total_size, contents.len() as u64 + 8);
        assert!(archive.metadata().unwrap().len() < stats.total_size);
        let inspected = inspect(&archive).unwrap().unwrap();
        assert_eq!(inspected.total_size, stats.total_size);
        let mut restored = read(&archive, stats.total_size).unwrap();
        restored.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            restored,
            vec![
                ("Example.lua".into(), contents),
                ("Example.lua.bak".into(), b"previous".to_vec())
            ]
        );
        assert!(read(&archive, stats.total_size - 1).is_err());
    }

    #[test]
    fn failed_creation_leaves_no_partial_backup_and_never_overwrites() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("SavedVariables");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("Example.lua"), b"original").unwrap();
        let root = temp.path().join("KalpaBackups");
        let archive = root.join("manual.zip");
        create(&source, &archive).unwrap();
        let original_archive = fs::read(&archive).unwrap();
        fs::write(source.join("Example.lua"), b"changed").unwrap();
        assert!(create(&source, &archive).is_err());
        assert_eq!(fs::read(&archive).unwrap(), original_archive);
        // Valid on every supported filesystem, but not a valid archive member.
        fs::write(source.join("bad..lua"), b"invalid").unwrap();
        assert!(create(&source, &root.join("failed.zip")).is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }

    #[test]
    fn consolidation_verifies_copies_preserves_dates_and_is_repeatable() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("kalpa-backups/Before changes");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("Example.lua"), vec![b'x'; 128 * 1024 + 3]).unwrap();
        fs::write(source.join("Example.lua.bak"), b"previous").unwrap();
        let original_date = fs::metadata(&source).unwrap().modified().unwrap();
        let destination = temp.path().join("KalpaBackups/Before changes.zip");
        assert_eq!(
            consolidate_folder(&source, &destination).unwrap(),
            ConsolidationOutcome::Copied
        );
        assert_eq!(
            fs::metadata(&destination).unwrap().modified().unwrap(),
            original_date
        );
        assert_eq!(
            consolidate_folder(&source, &destination).unwrap(),
            ConsolidationOutcome::AlreadyPresent
        );
        let archive_bytes = fs::read(&destination).unwrap();
        fs::write(source.join("Example.lua.bak"), b"modified").unwrap();
        assert!(consolidate_folder(&source, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), archive_bytes);
        assert_eq!(
            fs::read(source.join("Example.lua.bak")).unwrap(),
            b"modified"
        );
    }

    #[test]
    fn consolidation_refuses_nested_data_without_publishing_a_partial_zip() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("legacy");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("Example.lua"), b"settings").unwrap();
        fs::write(source.join("nested/Extra.lua"), b"nested data").unwrap();
        let destination = temp.path().join("shared/backup.zip");
        assert!(consolidate_folder(&source, &destination).is_err());
        assert!(!destination.exists());
        assert!(source.join("nested/Extra.lua").is_file());
    }

    #[test]
    fn consolidation_detects_source_changes_since_before_compression() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("legacy");
        fs::create_dir(&source).unwrap();
        let settings = source.join("Example.lua");
        fs::write(&settings, b"settings").unwrap();
        let before = folder_files(&source).unwrap();
        let original_date = before["Example.lua"].modified().unwrap();
        let archive = temp.path().join("backup.zip");
        create(&source, &archive).unwrap();
        // Even identical bytes must not hide a source modification during the copy.
        File::options()
            .write(true)
            .open(&settings)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(original_date + std::time::Duration::from_secs(60)),
            )
            .unwrap();
        assert!(verify_folder_archive(&source, &archive, before)
            .unwrap_err()
            .contains("changed"));
        assert_eq!(fs::read(settings).unwrap(), b"settings");
    }

    #[test]
    fn settings_reader_rejects_unsafe_members_and_duplicate_windows_names() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("test.zip");
        for name in [
            "../Example.lua",
            "SavedVariables/../Example.lua",
            "SavedVariables/sub/Example.lua",
            "SavedVariables/C:\\Example.lua",
            "SavedVariables/Example.lua:stream",
            "SavedVariables/CON.lua",
            "SavedVariables/Example.lua.",
            "SavedVariables/.metadata",
        ] {
            marked_archive(&archive, &[(name, b"bad")]);
            assert!(read(&archive, 1024).is_err(), "accepted {name}");
        }
        marked_archive(
            &archive,
            &[
                ("SavedVariables/Example.lua", b"one"),
                ("SavedVariables/example.lua", b"two"),
            ],
        );
        assert!(read(&archive, 1024).is_err());
    }

    #[test]
    fn automatic_snapshot_is_not_a_settings_backup() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("automatic.zip");
        let mut writer = ZipWriter::new(File::create(&archive).unwrap());
        writer
            .start_file("SavedVariables/Example.lua", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"settings").unwrap();
        writer.finish().unwrap();
        assert!(inspect(&archive).unwrap().is_none());
        assert!(read(&archive, 1024).unwrap_err().contains("Safety Center"));
    }

    #[test]
    fn settings_reader_checks_crc_for_every_file_before_returning_data() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("corrupt.zip");
        marked_archive(
            &archive,
            &[
                ("SavedVariables/Good.lua", b"good"),
                ("SavedVariables/Bad.lua", b"corrupt-me"),
            ],
        );
        let mut raw = fs::read(&archive).unwrap();
        let offset = raw
            .windows(10)
            .position(|bytes| bytes == b"corrupt-me")
            .unwrap();
        raw[offset] ^= 1;
        fs::write(&archive, raw).unwrap();
        assert_eq!(inspect(&archive).unwrap().unwrap().file_count, 2);
        assert!(read(&archive, 1024).is_err());
    }
}
