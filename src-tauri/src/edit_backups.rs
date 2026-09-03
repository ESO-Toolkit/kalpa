use crate::metadata;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BACKUPS_PER_ADDON: usize = 5;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupManifest {
    #[serde(alias = "addon_folder")]
    pub addon_folder: String,
    #[serde(alias = "backed_up_at")]
    pub backed_up_at: String,
    #[serde(alias = "update_from")]
    pub update_from: String,
    #[serde(alias = "update_to")]
    pub update_to: String,
    pub files: Vec<String>,
    /// Files backed up from the AddOns root rather than an addon folder.
    #[serde(default)]
    pub root_files: Vec<String>,
}

fn backups_dir(addons_dir: &Path) -> std::path::PathBuf {
    addons_dir.join(".kalpa-backups")
}

/// The manifest's `backed_up_at` value and the directory name derived from it.
///
/// Both MUST come from a single instant: [`restore_backup_file`] rebuilds the
/// directory name from the manifest field, so reading the clock twice strands
/// every backup whose copy phase happened to cross a second boundary.
fn backup_timestamps() -> (String, String) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let stamp = metadata::format_timestamp(secs);
    let dir_name = stamp.replace(':', "-");
    (stamp, dir_name)
}

/// Back up user-edited files before they're overwritten by an update.
/// Copies files from the addon folder into `.kalpa-backups/<folder>/<timestamp>/`.
pub fn backup_user_files(
    addons_dir: &Path,
    folder_name: &str,
    files: &[String],
    from_version: &str,
    to_version: &str,
) -> Result<(), String> {
    backup_files(
        addons_dir,
        folder_name,
        files,
        from_version,
        to_version,
        false,
    )
}

/// Back up files written directly under AddOns by a foldered archive.
pub fn backup_root_files(
    addons_dir: &Path,
    owner_folder: &str,
    files: &[String],
    from_version: &str,
    to_version: &str,
) -> Result<(), String> {
    backup_files(
        addons_dir,
        owner_folder,
        files,
        from_version,
        to_version,
        true,
    )
}

fn backup_files(
    addons_dir: &Path,
    folder_name: &str,
    files: &[String],
    from_version: &str,
    to_version: &str,
    root_files: bool,
) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }

    let (backed_up_at, ts) = backup_timestamps();
    let backup_root = backups_dir(addons_dir).join(folder_name).join(&ts);

    fs::create_dir_all(&backup_root)
        .map_err(|e| format!("Failed to create backup directory: {e}"))?;

    let mut backed_up = Vec::new();

    for rel_path in files {
        // Relative paths are forward-slash normalized (see file_hashes::relative_key).
        // Join them verbatim: Windows accepts '/' as a separator, while rewriting to
        // '\' would make the whole path a single literal component on macOS/Linux.
        let src = if root_files {
            addons_dir.join(rel_path)
        } else {
            addons_dir.join(folder_name).join(rel_path)
        };
        if !src.exists() {
            eprintln!(
                "Warning: backup source not found for {folder_name}/{rel_path}, skipping: {rel_path}"
            );
            continue;
        }
        let dest = if root_files {
            backup_root.join("root").join(rel_path)
        } else {
            backup_root.join(rel_path)
        };
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create backup subdirectory: {e}"))?;
        }
        fs::copy(&src, &dest).map_err(|e| format!("Failed to back up {rel_path}: {e}"))?;
        backed_up.push(rel_path.clone());
    }

    let manifest_path = backup_root.join("manifest.json");
    let mut manifest: BackupManifest = if manifest_path.exists() {
        metadata::load_json_with_backup(&manifest_path)
    } else {
        BackupManifest {
            addon_folder: folder_name.to_string(),
            backed_up_at,
            update_from: from_version.to_string(),
            update_to: to_version.to_string(),
            ..Default::default()
        }
    };
    if root_files {
        manifest.root_files.extend(backed_up);
    } else {
        manifest.files.extend(backed_up);
    }
    manifest.files.sort();
    manifest.files.dedup();
    manifest.root_files.sort();
    manifest.root_files.dedup();

    metadata::save_json_with_backup(&manifest_path, &manifest)?;

    prune_old_backups(addons_dir, folder_name);

    Ok(())
}

pub fn list_backups(addons_dir: &Path, folder_name: &str) -> Vec<BackupManifest> {
    let addon_backup_dir = backups_dir(addons_dir).join(folder_name);
    if !addon_backup_dir.is_dir() {
        return Vec::new();
    }

    let mut results = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(&addon_backup_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .collect();

    entries.sort_by_key(|e| e.file_name());
    entries.reverse();

    for entry in entries {
        let manifest_path = entry.path().join("manifest.json");
        if manifest_path.exists() {
            let manifest: BackupManifest = metadata::load_json_with_backup(&manifest_path);
            if !manifest.addon_folder.is_empty() {
                results.push(manifest);
            }
        }
    }

    results
}

pub fn restore_backup_file(
    addons_dir: &Path,
    folder_name: &str,
    backed_up_at: &str,
    relative_path: &str,
) -> Result<(), String> {
    let timestamp_dir = backed_up_at.replace(':', "-");
    let backup_root = backups_dir(addons_dir)
        .join(folder_name)
        .join(&timestamp_dir);

    let manifest_path = backup_root.join("manifest.json");
    let manifest: BackupManifest = if manifest_path.exists() {
        metadata::load_json_with_backup(&manifest_path)
    } else {
        BackupManifest::default()
    };
    let is_root_file = manifest.root_files.iter().any(|path| path == relative_path);
    let backup_file = if is_root_file {
        backup_root.join("root").join(relative_path)
    } else {
        backup_root.join(relative_path)
    };

    if !backup_file.exists() {
        return Err(format!("Backup file not found: {relative_path}"));
    }

    let dest = if is_root_file {
        addons_dir.join(relative_path)
    } else {
        addons_dir.join(folder_name).join(relative_path)
    };

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {e}"))?;
    }

    // Publish through the shared unique-staging helper so concurrent Tauri and
    // Slint restores cannot truncate or clean up one another's temporary file.
    let bytes = fs::read(&backup_file).map_err(|e| format!("Failed to restore file: {e}"))?;
    crate::atomic_file::atomic_write(&dest, &bytes)
        .map_err(|e| format!("Failed to restore file: {e}"))?;

    Ok(())
}

fn prune_old_backups(addons_dir: &Path, folder_name: &str) {
    let addon_backup_dir = backups_dir(addons_dir).join(folder_name);
    if !addon_backup_dir.is_dir() {
        return;
    }

    let mut entries: Vec<_> = fs::read_dir(&addon_backup_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .collect();

    if entries.len() <= MAX_BACKUPS_PER_ADDON {
        return;
    }

    entries.sort_by_key(|e| e.file_name());
    let to_remove = entries.len() - MAX_BACKUPS_PER_ADDON;
    for entry in entries.into_iter().take(to_remove) {
        eprintln!(
            "Pruning old backup for {}: {}",
            folder_name,
            entry.file_name().to_string_lossy()
        );
        if let Err(e) = fs::remove_dir_all(entry.path()) {
            eprintln!(
                "Warning: failed to remove old backup {:?}: {}",
                entry.path(),
                e
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_and_prune() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let addon_path = addons_dir.join("TestAddon");
        fs::create_dir_all(&addon_path).unwrap();
        fs::write(addon_path.join("init.lua"), "original content").unwrap();

        backup_user_files(
            &addons_dir,
            "TestAddon",
            &["init.lua".to_string()],
            "1.0",
            "2.0",
        )
        .unwrap();

        let backup_dir = backups_dir(&addons_dir).join("TestAddon");
        assert!(backup_dir.is_dir());

        let snapshots: Vec<_> = fs::read_dir(&backup_dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().is_dir())
            .collect();
        assert_eq!(snapshots.len(), 1);

        let snapshot_dir = snapshots[0].path();
        assert!(snapshot_dir.join("init.lua").exists());
        assert!(snapshot_dir.join("manifest.json").exists());
    }

    #[test]
    fn backup_skips_missing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let addon_path = addons_dir.join("TestAddon");
        fs::create_dir_all(&addon_path).unwrap();

        let result = backup_user_files(
            &addons_dir,
            "TestAddon",
            &["nonexistent.lua".to_string()],
            "1.0",
            "2.0",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn backup_empty_files_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let result = backup_user_files(tmp.path(), "TestAddon", &[], "1.0", "2.0");
        assert!(result.is_ok());
        assert!(!backups_dir(tmp.path()).exists());
    }

    #[test]
    fn backup_and_restore_roundtrip_for_nested_path() {
        // Relative paths are forward-slash normalized. Rewriting them to '\' made
        // the whole path one literal component on macOS/Linux, so the source was
        // never found and every nested restore failed.
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let addon_path = addons_dir.join("TestAddon");
        fs::create_dir_all(addon_path.join("Libs/LibFoo")).unwrap();
        fs::write(addon_path.join("Libs/LibFoo/LAM.lua"), "user edit").unwrap();

        backup_user_files(
            &addons_dir,
            "TestAddon",
            &["Libs/LibFoo/LAM.lua".to_string()],
            "1.0",
            "2.0",
        )
        .unwrap();

        let manifest = list_backups(&addons_dir, "TestAddon")
            .into_iter()
            .next()
            .expect("a backup manifest");
        assert_eq!(manifest.files, vec!["Libs/LibFoo/LAM.lua".to_string()]);

        // Simulate the update overwriting the user's edit, then restore it.
        fs::write(addon_path.join("Libs/LibFoo/LAM.lua"), "upstream").unwrap();
        restore_backup_file(
            &addons_dir,
            "TestAddon",
            &manifest.backed_up_at,
            "Libs/LibFoo/LAM.lua",
        )
        .unwrap();

        let restored = fs::read_to_string(addon_path.join("Libs/LibFoo/LAM.lua")).unwrap();
        assert_eq!(restored, "user edit");
    }

    #[test]
    fn root_backup_and_restore_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();
        fs::write(addons_dir.join("readme.txt"), "user edit").unwrap();

        backup_root_files(
            &addons_dir,
            "MainAddon",
            &["readme.txt".to_string()],
            "1.0",
            "2.0",
        )
        .unwrap();

        let manifest = list_backups(&addons_dir, "MainAddon")
            .into_iter()
            .next()
            .expect("a root backup manifest");
        assert_eq!(manifest.root_files, vec!["readme.txt"]);
        fs::write(addons_dir.join("readme.txt"), "upstream").unwrap();
        restore_backup_file(
            &addons_dir,
            "MainAddon",
            &manifest.backed_up_at,
            "readme.txt",
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(addons_dir.join("readme.txt")).unwrap(),
            "user edit"
        );
    }

    #[test]
    fn manifest_timestamp_names_the_backup_directory() {
        // restore_backup_file rebuilds the directory name from backed_up_at, so a
        // second clock read would strand any backup crossing a second boundary.
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let addon_path = addons_dir.join("TestAddon");
        fs::create_dir_all(&addon_path).unwrap();
        fs::write(addon_path.join("init.lua"), "content").unwrap();

        backup_user_files(
            &addons_dir,
            "TestAddon",
            &["init.lua".to_string()],
            "1.0",
            "2.0",
        )
        .unwrap();

        let manifest = list_backups(&addons_dir, "TestAddon")
            .into_iter()
            .next()
            .expect("a backup manifest");
        let dir = backups_dir(&addons_dir)
            .join("TestAddon")
            .join(manifest.backed_up_at.replace(':', "-"));
        assert!(
            dir.is_dir(),
            "manifest backed_up_at must name the on-disk backup directory"
        );
    }
}
