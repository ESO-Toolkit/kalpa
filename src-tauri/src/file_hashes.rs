use crate::metadata;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn is_zero(v: &u32) -> bool {
    *v == 0
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HashManifest {
    pub addon_folder: String,
    /// Canonical list of ESOUI IDs that ship files into this folder.
    #[serde(default)]
    pub esoui_ids: Vec<u32>,
    /// Legacy single-ID field kept for deserializing manifests written before
    /// esoui_ids was introduced. Migrated to esoui_ids on load; never written.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub esoui_id: u32,
    pub recorded_at: String,
    pub installed_version: String,
    pub files: HashMap<String, String>,
    #[serde(default)]
    pub modified_files: Vec<String>,
}

fn hashes_dir(addons_dir: &Path) -> std::path::PathBuf {
    addons_dir.join(".kalpa-hashes")
}

fn manifest_path(addons_dir: &Path, folder_name: &str) -> std::path::PathBuf {
    hashes_dir(addons_dir).join(format!("{}.json", folder_name))
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|e| format!("Failed to open file for hashing: {}", e))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("Failed to read file for hashing: {}", e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect())
}

fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Walk an addon folder and compute SHA-256 hashes for every file.
/// Keys are forward-slash-normalized relative paths within the addon folder.
pub fn compute_addon_hashes(addon_path: &Path) -> Result<HashMap<String, String>, String> {
    let mut hashes = HashMap::new();

    fn walk(
        base: &Path,
        current: &Path,
        hashes: &mut HashMap<String, String>,
    ) -> Result<(), String> {
        let entries = fs::read_dir(current)
            .map_err(|e| format!("Failed to read directory {:?}: {}", current, e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read dir entry: {}", e))?;
            let path = entry.path();

            if path.is_dir() {
                walk(base, &path, hashes)?;
            } else if path.is_file() {
                let relative = path
                    .strip_prefix(base)
                    .map_err(|e| format!("Path prefix error: {}", e))?;
                let key = relative
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .collect::<Vec<_>>()
                    .join("/");
                let hash = hash_file(&path)?;
                hashes.insert(key, hash);
            }
        }
        Ok(())
    }

    if !addon_path.is_dir() {
        return Err(format!("Addon path is not a directory: {:?}", addon_path));
    }

    walk(addon_path, addon_path, &mut hashes)?;
    Ok(hashes)
}

/// Hash files inside a ZIP that belong to a specific addon folder, without extracting.
/// Keys are forward-slash-normalized relative paths (excluding the top-level folder prefix).
pub fn hash_zip_entries(
    zip_path: &Path,
    folder_name: &str,
) -> Result<HashMap<String, String>, String> {
    let file =
        fs::File::open(zip_path).map_err(|e| format!("Failed to open ZIP for hashing: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Failed to read ZIP archive: {}", e))?;

    let prefix = format!("{}/", folder_name);
    let mut hashes = HashMap::new();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry: {}", e))?;

        if entry.is_dir() {
            continue;
        }

        let name = match entry.enclosed_name() {
            Some(p) => p.to_string_lossy().replace('\\', "/"),
            None => continue,
        };

        let relative = match name.strip_prefix(&prefix) {
            Some(r) if !r.is_empty() => r.to_string(),
            _ => continue,
        };

        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut buf)
            .map_err(|e| format!("Failed to read ZIP entry {}: {}", name, e))?;

        hashes.insert(relative, sha256_bytes(&buf));
    }

    Ok(hashes)
}

pub fn save_hash_manifest(addons_dir: &Path, manifest: &HashManifest) -> Result<(), String> {
    let dir = hashes_dir(addons_dir);
    if !dir.exists() {
        fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create .kalpa-hashes directory: {}", e))?;
    }
    let path = manifest_path(addons_dir, &manifest.addon_folder);
    metadata::save_json_with_backup(&path, manifest)
}

pub fn load_hash_manifest(addons_dir: &Path, folder_name: &str) -> Option<HashManifest> {
    let path = manifest_path(addons_dir, folder_name);
    if !path.exists() {
        return None;
    }
    let mut manifest: HashManifest = metadata::load_json_with_backup(&path);
    // Migrate manifests written before esoui_ids was introduced.
    if manifest.esoui_ids.is_empty() && manifest.esoui_id != 0 {
        manifest.esoui_ids = vec![manifest.esoui_id];
    }
    Some(manifest)
}

/// Compare current files on disk against stored hashes.
/// Returns the list of relative paths that have been modified.
/// Also updates the `modified_files` cache in the manifest on disk.
pub fn detect_modifications(addons_dir: &Path, folder_name: &str) -> Result<Vec<String>, String> {
    let mut manifest = match load_hash_manifest(addons_dir, folder_name) {
        Some(m) => m,
        None => return Ok(Vec::new()),
    };

    let addon_path = addons_dir.join(folder_name);
    let current_hashes = compute_addon_hashes(&addon_path)?;

    let mut modified = Vec::new();
    for (path, stored_hash) in &manifest.files {
        match current_hashes.get(path) {
            Some(disk_hash) if disk_hash != stored_hash => {
                modified.push(path.clone());
            }
            None => {
                // File was deleted by user — treat as modified
                modified.push(path.clone());
            }
            _ => {}
        }
    }

    // Detect files that exist on disk but were not part of the recorded baseline.
    // These may conflict with new files added by an upstream update.
    for path in current_hashes.keys() {
        if !manifest.files.contains_key(path) {
            modified.push(path.clone());
        }
    }

    modified.sort();
    modified.dedup();
    manifest.modified_files = modified.clone();
    save_hash_manifest(addons_dir, &manifest)?;

    Ok(modified)
}

/// Record hashes for all folders extracted from an addon install/update.
/// Called after `extract_addon_zip` to create the initial hash baseline.
pub fn record_hashes_for_folders(
    addons_dir: &Path,
    installed_folders: &[String],
    esoui_id: u32,
    version: &str,
) {
    record_hashes_for_folders_with_overrides(
        addons_dir,
        installed_folders,
        esoui_id,
        version,
        None,
    );
}

/// Record hashes with optional overrides for files the user kept during a
/// conflict resolution. For "keep_mine" files, we store the *upstream* hash
/// (from the ZIP) so the next update still detects the user's edit.
pub fn record_hashes_for_folders_with_overrides(
    addons_dir: &Path,
    installed_folders: &[String],
    esoui_id: u32,
    version: &str,
    hash_overrides: Option<&HashMap<String, String>>,
) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let timestamp = metadata::format_timestamp(now).replace(':', "-");

    for folder in installed_folders {
        let addon_path = addons_dir.join(folder);
        let mut files = match compute_addon_hashes(&addon_path) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("Warning: failed to hash addon {}: {}", folder, e);
                continue;
            }
        };

        // For kept files, replace disk hash with the upstream ZIP hash
        // so the user's edit remains detectable on the next update.
        if let Some(overrides) = hash_overrides {
            for (path, upstream_hash) in overrides {
                if files.contains_key(path) {
                    files.insert(path.clone(), upstream_hash.clone());
                }
            }
        }

        let modified_files: Vec<String> = if let Some(ov) = hash_overrides {
            ov.keys()
                .filter(|k| files.contains_key(*k))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        let manifest = HashManifest {
            addon_folder: folder.clone(),
            esoui_ids: vec![esoui_id],
            recorded_at: timestamp.clone(),
            installed_version: version.to_string(),
            files,
            modified_files,
            ..Default::default()
        };

        if let Err(e) = save_hash_manifest(addons_dir, &manifest) {
            eprintln!(
                "Warning: failed to save hash manifest for {}: {}",
                folder, e
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn create_addon_dir(base: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let addon_path = base.join(name);
        for (rel_path, content) in files {
            let file_path = addon_path.join(rel_path);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&file_path, content).unwrap();
        }
        addon_path
    }

    fn create_test_zip(
        dir: &Path,
        zip_name: &str,
        folder: &str,
        files: &[(&str, &str)],
    ) -> PathBuf {
        let zip_path = dir.join(zip_name);
        let file = fs::File::create(&zip_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();

        for (rel_path, content) in files {
            archive
                .start_file(format!("{}/{}", folder, rel_path), options)
                .unwrap();
            archive.write_all(content.as_bytes()).unwrap();
        }
        archive.finish().unwrap();
        zip_path
    }

    #[test]
    fn compute_hashes_produces_correct_keys() {
        let tmp = tempfile::tempdir().unwrap();
        create_addon_dir(
            tmp.path(),
            "TestAddon",
            &[
                ("TestAddon.lua", "local x = 1"),
                ("modules/combat.lua", "-- combat"),
            ],
        );

        let hashes = compute_addon_hashes(&tmp.path().join("TestAddon")).unwrap();
        assert_eq!(hashes.len(), 2);
        assert!(hashes.contains_key("TestAddon.lua"));
        assert!(hashes.contains_key("modules/combat.lua"));
        // SHA-256 hex should be 64 chars
        for hash in hashes.values() {
            assert_eq!(hash.len(), 64);
        }
    }

    #[test]
    fn hash_zip_entries_matches_folder_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        let content = "local x = 42";

        // Create a file on disk
        create_addon_dir(tmp.path(), "MyAddon", &[("init.lua", content)]);
        let disk_hashes = compute_addon_hashes(&tmp.path().join("MyAddon")).unwrap();

        // Create a ZIP with the same content
        let zip_path = create_test_zip(tmp.path(), "test.zip", "MyAddon", &[("init.lua", content)]);
        let zip_hashes = hash_zip_entries(&zip_path, "MyAddon").unwrap();

        assert_eq!(disk_hashes["init.lua"], zip_hashes["init.lua"]);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        let mut files = HashMap::new();
        files.insert("test.lua".to_string(), "abc123".repeat(10));

        let manifest = HashManifest {
            addon_folder: "TestAddon".to_string(),
            esoui_ids: vec![42],
            recorded_at: "2026-05-03T12:00:00Z".to_string(),
            installed_version: "1.0.0".to_string(),
            files,
            modified_files: Vec::new(),
            ..Default::default()
        };

        save_hash_manifest(&addons_dir, &manifest).unwrap();

        let loaded = load_hash_manifest(&addons_dir, "TestAddon").unwrap();
        assert_eq!(loaded.addon_folder, "TestAddon");
        assert_eq!(loaded.esoui_ids, vec![42]);
        assert_eq!(loaded.files.len(), 1);
    }

    #[test]
    fn load_returns_none_for_missing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load_hash_manifest(tmp.path(), "NoSuchAddon").is_none());
    }

    #[test]
    fn detect_modifications_finds_changed_file() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        create_addon_dir(&addons_dir, "MyAddon", &[("init.lua", "original")]);

        // Record initial hashes
        record_hashes_for_folders(&addons_dir, &["MyAddon".to_string()], 1, "1.0");

        // Modify the file
        fs::write(addons_dir.join("MyAddon/init.lua"), "modified content").unwrap();

        let modified = detect_modifications(&addons_dir, "MyAddon").unwrap();
        assert_eq!(modified, vec!["init.lua"]);
    }

    #[test]
    fn detect_modifications_finds_deleted_file() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        create_addon_dir(
            &addons_dir,
            "MyAddon",
            &[("init.lua", "code"), ("extra.lua", "more code")],
        );

        record_hashes_for_folders(&addons_dir, &["MyAddon".to_string()], 1, "1.0");

        // Delete one file
        fs::remove_file(addons_dir.join("MyAddon/extra.lua")).unwrap();

        let modified = detect_modifications(&addons_dir, "MyAddon").unwrap();
        assert_eq!(modified, vec!["extra.lua"]);
    }

    #[test]
    fn detect_modifications_empty_when_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        create_addon_dir(&addons_dir, "MyAddon", &[("init.lua", "unchanged")]);

        record_hashes_for_folders(&addons_dir, &["MyAddon".to_string()], 1, "1.0");

        let modified = detect_modifications(&addons_dir, "MyAddon").unwrap();
        assert!(modified.is_empty());
    }

    #[test]
    fn detect_modifications_returns_empty_without_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let modified = detect_modifications(tmp.path(), "NoManifest").unwrap();
        assert!(modified.is_empty());
    }

    #[test]
    fn record_hashes_creates_manifest_for_each_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        create_addon_dir(&addons_dir, "AddonA", &[("a.lua", "aaa")]);
        create_addon_dir(&addons_dir, "AddonB", &[("b.lua", "bbb")]);

        record_hashes_for_folders(
            &addons_dir,
            &["AddonA".to_string(), "AddonB".to_string()],
            99,
            "2.0",
        );

        let a = load_hash_manifest(&addons_dir, "AddonA").unwrap();
        assert_eq!(a.esoui_ids, vec![99]);
        assert!(a.files.contains_key("a.lua"));

        let b = load_hash_manifest(&addons_dir, "AddonB").unwrap();
        assert!(b.files.contains_key("b.lua"));
    }

    #[test]
    fn detect_modifications_finds_new_untracked_file() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        create_addon_dir(&addons_dir, "MyAddon", &[("init.lua", "original")]);

        record_hashes_for_folders(&addons_dir, &["MyAddon".to_string()], 1, "1.0");

        // Simulate a new file appearing on disk (e.g., added by upstream update or user)
        fs::write(addons_dir.join("MyAddon/new_module.lua"), "new content").unwrap();

        let modified = detect_modifications(&addons_dir, "MyAddon").unwrap();
        assert!(
            modified.contains(&"new_module.lua".to_string()),
            "expected new_module.lua to be flagged, got: {:?}",
            modified
        );
    }

    #[test]
    fn load_manifest_migrates_legacy_esoui_id() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        fs::create_dir_all(&addons_dir).unwrap();

        // Write a manifest in the old format with esoui_id (singular)
        let old_json = r#"{"addon_folder":"LegacyAddon","esoui_id":77,"recorded_at":"2025-01-01T00:00:00Z","installed_version":"1.0","files":{}}"#;
        let hashes_dir = addons_dir.join(".kalpa-hashes");
        fs::create_dir_all(&hashes_dir).unwrap();
        fs::write(hashes_dir.join("LegacyAddon.json"), old_json).unwrap();

        let loaded = load_hash_manifest(&addons_dir, "LegacyAddon").unwrap();
        assert_eq!(loaded.esoui_ids, vec![77]);
    }

    #[test]
    fn hash_zip_ignores_other_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("multi.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();

        archive.start_file("AddonA/init.lua", options).unwrap();
        archive.write_all(b"aaa").unwrap();
        archive.start_file("AddonB/init.lua", options).unwrap();
        archive.write_all(b"bbb").unwrap();
        archive.finish().unwrap();

        let hashes = hash_zip_entries(&zip_path, "AddonA").unwrap();
        assert_eq!(hashes.len(), 1);
        assert!(hashes.contains_key("init.lua"));
    }
}
