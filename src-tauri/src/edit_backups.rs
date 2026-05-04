use crate::metadata;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BACKUPS_PER_ADDON: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub addon_folder: String,
    pub backed_up_at: String,
    pub update_from: String,
    pub update_to: String,
    pub files: Vec<String>,
}

fn backups_dir(addons_dir: &Path) -> std::path::PathBuf {
    addons_dir.join(".kalpa-backups")
}

fn timestamp_string() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    metadata::format_timestamp(secs).replace(':', "-")
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
    if files.is_empty() {
        return Ok(());
    }

    let ts = timestamp_string();
    let backup_root = backups_dir(addons_dir).join(folder_name).join(&ts);

    fs::create_dir_all(&backup_root)
        .map_err(|e| format!("Failed to create backup directory: {}", e))?;

    let addon_path = addons_dir.join(folder_name);
    let mut backed_up = Vec::new();

    for rel_path in files {
        let src = addon_path.join(rel_path.replace('/', "\\"));
        if !src.exists() {
            continue;
        }
        let dest = backup_root.join(rel_path.replace('/', "\\"));
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create backup subdirectory: {}", e))?;
        }
        fs::copy(&src, &dest).map_err(|e| format!("Failed to back up {}: {}", rel_path, e))?;
        backed_up.push(rel_path.clone());
    }

    let manifest = BackupManifest {
        addon_folder: folder_name.to_string(),
        backed_up_at: metadata::format_timestamp(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        ),
        update_from: from_version.to_string(),
        update_to: to_version.to_string(),
        files: backed_up,
    };

    let manifest_path = backup_root.join("manifest.json");
    metadata::save_json_with_backup(&manifest_path, &manifest)?;

    prune_old_backups(addons_dir, folder_name);

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
        let _ = fs::remove_dir_all(entry.path());
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
}
