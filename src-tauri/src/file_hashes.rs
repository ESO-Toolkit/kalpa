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
    hashes_dir(addons_dir).join(format!("{folder_name}.json"))
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|e| format!("Failed to open file for hashing: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("Failed to read file for hashing: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

const MAX_HASH_BYTES: u64 = 500 * 1024 * 1024;

fn stream_sha256(reader: &mut impl Read) -> Result<String, std::io::Error> {
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_HASH_BYTES {
            return Err(std::io::Error::other("entry exceeds maximum hashable size"));
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

const MAX_WALK_DEPTH: u32 = 32;

/// Compute the forward-slash-normalized relative key for a file under `base`.
fn relative_key(base: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(base)
        .map_err(|e| format!("Path prefix error: {e}"))?;
    Ok(relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/"))
}

/// Walk an addon folder, invoking `on_file(key, path)` for every real (non-symlink)
/// file with its forward-slash relative key. Symlinks/junctions are skipped to avoid
/// loops, exactly mirroring [`compute_addon_hashes`].
fn walk_addon_files<F>(addon_path: &Path, mut on_file: F) -> Result<(), String>
where
    F: FnMut(String, &Path) -> Result<(), String>,
{
    fn walk<F>(base: &Path, current: &Path, depth: u32, on_file: &mut F) -> Result<(), String>
    where
        F: FnMut(String, &Path) -> Result<(), String>,
    {
        if depth > MAX_WALK_DEPTH {
            return Err("Directory tree too deep (> 32 levels).".to_string());
        }

        let entries = fs::read_dir(current)
            .map_err(|e| format!("Failed to read directory {current:?}: {e}"))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read dir entry: {e}"))?;
            let path = entry.path();

            // Skip symlinks/junctions to avoid loops
            if let Ok(meta) = path.symlink_metadata() {
                if meta.file_type().is_symlink() {
                    continue;
                }
            }

            if path.is_dir() {
                walk(base, &path, depth + 1, on_file)?;
            } else if path.is_file() {
                let key = relative_key(base, &path)?;
                on_file(key, &path)?;
            }
        }
        Ok(())
    }

    if !addon_path.is_dir() {
        return Err(format!("Addon path is not a directory: {addon_path:?}"));
    }

    walk(addon_path, addon_path, 0, &mut on_file)
}

/// Walk an addon folder and compute SHA-256 hashes for every file.
/// Keys are forward-slash-normalized relative paths within the addon folder.
pub fn compute_addon_hashes(addon_path: &Path) -> Result<HashMap<String, String>, String> {
    let mut hashes = HashMap::new();
    walk_addon_files(addon_path, |key, path| {
        hashes.insert(key, hash_file(path)?);
        Ok(())
    })?;
    Ok(hashes)
}

/// Build a hash baseline for a just-extracted addon folder by reusing a ZIP-derived
/// hash map (`zip_hashes`, keyed folder-relative) for every file the ZIP provided,
/// and hashing from disk ONLY the files the ZIP did not cover.
///
/// This is the optimized equivalent of [`compute_addon_hashes`] for the
/// install/update path. The result is identical to hashing the whole folder from
/// disk, because extraction writes the ZIP's bytes verbatim — so a covered file's
/// disk hash equals its ZIP hash. The only files actually read from disk here are
/// the ones the ZIP did not contain: files removed in the new version that linger,
/// and user-added files. (For kept/skipped files the disk holds the user's bytes,
/// but the caller overlays the upstream ZIP hash afterwards, and the ZIP hash is
/// already what this function recorded — so the overlay is a safe no-op there.)
///
/// Driving the merge from a DISK walk (rather than starting from `zip_hashes`)
/// guarantees the baseline only ever contains keys that actually exist on disk.
/// A ZIP entry that extraction skips structurally — a symlink entry, which
/// `hash_zip_entries` still hashes but `extract_addon_zip*` does not write — is
/// therefore never recorded, matching `compute_addon_hashes` and avoiding a
/// spurious "file deleted" flag on the next modification scan.
pub fn compute_baseline_with_zip(
    addon_path: &Path,
    zip_hashes: &HashMap<String, String>,
) -> Result<HashMap<String, String>, String> {
    let mut hashes = HashMap::new();
    walk_addon_files(addon_path, |key, path| {
        let hash = match zip_hashes.get(&key) {
            Some(h) => h.clone(),
            None => hash_file(path)?,
        };
        hashes.insert(key, hash);
        Ok(())
    })?;
    Ok(hashes)
}

/// Strip a leading `<folder>/` prefix from a forward-slash ZIP entry path,
/// matching the folder segment CASE-INSENSITIVELY but returning the remainder
/// with its ORIGINAL casing.
///
/// ESO addon folders are created on disk verbatim from the ZIP's top-level
/// folder name, so disk and ZIP casing normally agree. They can diverge only if
/// an upstream author changes the folder's casing between releases, while the
/// caller still passes the previously-installed (disk) folder name. Matching the
/// folder segment case-insensitively keeps conflict detection and kept-file
/// overrides working across such a rename; preserving the remainder's casing
/// keeps the relative keys identical to what the disk walk produces (Windows is
/// case-insensitive, so a file can't exist under two casings at once).
fn strip_folder_prefix_ci<'a>(name: &'a str, folder_name: &str) -> Option<&'a str> {
    let slash = name.find('/')?;
    let (zip_folder, rest) = (&name[..slash], &name[slash + 1..]);
    if rest.is_empty() || !zip_folder.eq_ignore_ascii_case(folder_name) {
        return None;
    }
    Some(rest)
}

/// Hash files inside a ZIP that belong to a specific addon folder, without extracting.
/// Keys are forward-slash-normalized relative paths (excluding the top-level folder prefix).
pub fn hash_zip_entries(
    zip_path: &Path,
    folder_name: &str,
) -> Result<HashMap<String, String>, String> {
    let file =
        fs::File::open(zip_path).map_err(|e| format!("Failed to open ZIP for hashing: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Failed to read ZIP archive: {e}"))?;

    let mut hashes = HashMap::new();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry: {e}"))?;

        if entry.is_dir() {
            continue;
        }

        let name = match entry.enclosed_name() {
            Some(p) => p.to_string_lossy().replace('\\', "/"),
            None => continue,
        };

        let relative = match strip_folder_prefix_ci(&name, folder_name) {
            Some(r) => r.to_string(),
            None => continue,
        };

        let hash = stream_sha256(&mut entry)
            .map_err(|e| format!("Failed to read ZIP entry {name}: {e}"))?;
        hashes.insert(relative, hash);
    }

    Ok(hashes)
}

pub fn save_hash_manifest(addons_dir: &Path, manifest: &HashManifest) -> Result<(), String> {
    let dir = hashes_dir(addons_dir);
    if !dir.exists() {
        fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create .kalpa-hashes directory: {e}"))?;
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
///
/// Returns `Err` if any folder's baseline could not be hashed or persisted; the
/// remaining folders are still attempted first. A missing/stale manifest is
/// silent-data-loss territory (the next update would see no baseline and treat
/// user edits as absent), so callers should surface this rather than ignore it.
pub fn record_hashes_for_folders(
    addons_dir: &Path,
    installed_folders: &[String],
    esoui_id: u32,
    version: &str,
) -> Result<(), String> {
    record_hashes_for_folders_with_overrides(addons_dir, installed_folders, esoui_id, version, None)
}

/// Record hashes with optional overrides for files the user kept during a
/// conflict resolution. For "keep_mine" files, we store the *upstream* hash
/// (from the ZIP) so the next update still detects the user's edit.
///
/// This variant hashes every folder from disk. Prefer
/// [`record_hashes_with_zip_baseline`] on the conflict-aware update paths, where
/// the ZIP has already been hashed once for conflict detection and that map can
/// be reused instead of re-walking the whole folder.
///
/// Every folder is attempted; the first per-folder failure is reported via the
/// returned `Err` (later folders still get their manifests written).
pub fn record_hashes_for_folders_with_overrides(
    addons_dir: &Path,
    installed_folders: &[String],
    esoui_id: u32,
    version: &str,
    hash_overrides: Option<&HashMap<String, String>>,
) -> Result<(), String> {
    let timestamp = manifest_timestamp();

    record_each_folder(installed_folders, |folder| {
        let addon_path = addons_dir.join(folder);
        let files = compute_addon_hashes(&addon_path)?;
        write_folder_manifest(
            addons_dir,
            folder,
            files,
            esoui_id,
            version,
            &timestamp,
            hash_overrides,
        )
    })
}

/// Record hash baselines after a conflict-aware install/update, reusing an
/// already-computed ZIP hash map instead of re-hashing each folder from disk.
///
/// `primary_zip_hashes` is the ZIP hash map for `primary_folder_name` that the
/// caller already produced (e.g. during conflict detection). For the primary
/// folder it is reused directly; for any *secondary* folders a multi-folder ZIP
/// extracted (bundled libraries), this function hashes that folder's ZIP entries
/// itself. Each folder's baseline is built with [`compute_baseline_with_zip`],
/// which only reads from disk the files the ZIP did not provide.
///
/// `hash_overrides` (kept "keep_mine" files → upstream hash) are applied ONLY to
/// the primary folder: selective extraction skips are always primary-folder
/// paths, so secondary folders are fully extracted and never have kept files.
///
/// Every folder in `installed_folders` is guaranteed an attempt at a manifest:
/// if reusing the ZIP map fails for any folder it falls back to a full disk hash
/// pass, and only a folder that fails *both* contributes an `Err`. Returns `Err`
/// (after attempting all folders) if any folder could not be recorded, so the
/// caller can fail the update instead of leaving metadata pointing at a folder
/// with no hash baseline.
#[allow(clippy::too_many_arguments)]
pub fn record_hashes_with_zip_baseline(
    addons_dir: &Path,
    zip_path: &Path,
    installed_folders: &[String],
    primary_folder_name: &str,
    primary_zip_hashes: &HashMap<String, String>,
    esoui_id: u32,
    version: &str,
    hash_overrides: Option<&HashMap<String, String>>,
) -> Result<(), String> {
    let timestamp = manifest_timestamp();

    record_each_folder(installed_folders, |folder| {
        let addon_path = addons_dir.join(folder);
        let is_primary = folder == primary_folder_name;

        // Build the folder's baseline from the ZIP map where possible, always
        // falling back to a full disk hash so a ZIP-side failure (e.g. an entry
        // exceeding the streaming hash cap) still produces a correct manifest.
        let files = if is_primary {
            compute_baseline_with_zip(&addon_path, primary_zip_hashes)
                .or_else(|_| compute_addon_hashes(&addon_path))?
        } else {
            match hash_zip_entries(zip_path, folder) {
                Ok(zip_hashes) => compute_baseline_with_zip(&addon_path, &zip_hashes),
                Err(e) => {
                    eprintln!(
                        "Warning: failed to hash ZIP for {folder}, falling back to disk: {e}"
                    );
                    compute_addon_hashes(&addon_path)
                }
            }
            .or_else(|_| compute_addon_hashes(&addon_path))?
        };

        // Overrides only ever describe primary-folder kept files.
        let folder_overrides = if is_primary { hash_overrides } else { None };

        write_folder_manifest(
            addons_dir,
            folder,
            files,
            esoui_id,
            version,
            &timestamp,
            folder_overrides,
        )
    })
}

/// Run `record` for every folder, attempting all of them and returning the first
/// error encountered (logging each). Centralizes the "attempt all, report any
/// failure" policy both record paths share.
fn record_each_folder<F>(installed_folders: &[String], mut record: F) -> Result<(), String>
where
    F: FnMut(&str) -> Result<(), String>,
{
    let mut first_err: Option<String> = None;
    for folder in installed_folders {
        if let Err(e) = record(folder) {
            eprintln!("Warning: failed to record hash manifest for {folder}: {e}");
            if first_err.is_none() {
                first_err = Some(format!("{folder}: {e}"));
            }
        }
    }
    match first_err {
        Some(e) => Err(format!("Failed to record hash baseline ({e})")),
        None => Ok(()),
    }
}

/// Timestamp string for a freshly recorded manifest (`:` → `-` for filesystem
/// safety, matching the existing manifest format).
fn manifest_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    metadata::format_timestamp(now).replace(':', "-")
}

/// Apply kept-file overrides, derive the `modified_files` cache, and persist one
/// folder's manifest. Shared by the disk-only and ZIP-baseline record paths so
/// they produce byte-identical manifests for the same `files` map. Returns the
/// `save_hash_manifest` error so callers can treat a failed write as fatal.
fn write_folder_manifest(
    addons_dir: &Path,
    folder: &str,
    mut files: HashMap<String, String>,
    esoui_id: u32,
    version: &str,
    timestamp: &str,
    hash_overrides: Option<&HashMap<String, String>>,
) -> Result<(), String> {
    // For kept files, replace the recorded hash with the upstream ZIP hash so
    // the user's edit remains detectable on the next update.
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
        addon_folder: folder.to_string(),
        esoui_ids: vec![esoui_id],
        recorded_at: timestamp.to_string(),
        installed_version: version.to_string(),
        files,
        modified_files,
        ..Default::default()
    };

    save_hash_manifest(addons_dir, &manifest)
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
                .start_file(format!("{folder}/{rel_path}"), options)
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
        record_hashes_for_folders(&addons_dir, &["MyAddon".to_string()], 1, "1.0").unwrap();

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

        record_hashes_for_folders(&addons_dir, &["MyAddon".to_string()], 1, "1.0").unwrap();

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

        record_hashes_for_folders(&addons_dir, &["MyAddon".to_string()], 1, "1.0").unwrap();

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
        )
        .unwrap();

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

        record_hashes_for_folders(&addons_dir, &["MyAddon".to_string()], 1, "1.0").unwrap();

        // Simulate a new file appearing on disk (e.g., added by upstream update or user)
        fs::write(addons_dir.join("MyAddon/new_module.lua"), "new content").unwrap();

        let modified = detect_modifications(&addons_dir, "MyAddon").unwrap();
        assert!(
            modified.contains(&"new_module.lua".to_string()),
            "expected new_module.lua to be flagged, got: {modified:?}"
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

    #[test]
    fn hash_zip_entries_matches_folder_case_insensitively() {
        // Upstream re-cased the top-level folder ("myAddon/") but the caller
        // still passes the previously-installed name ("MyAddon"). The folder
        // segment must match case-insensitively; the relative key keeps its
        // original casing.
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = create_test_zip(
            tmp.path(),
            "u.zip",
            "myAddon",
            &[("init.lua", "x"), ("Sub/Mixed.lua", "y")],
        );

        let hashes = hash_zip_entries(&zip_path, "MyAddon").unwrap();
        assert_eq!(hashes.len(), 2);
        assert_eq!(hashes["init.lua"], sha256_hex("x"));
        // Remainder casing is preserved (only the folder segment is folded).
        assert_eq!(hashes["Sub/Mixed.lua"], sha256_hex("y"));
    }

    #[test]
    fn strip_folder_prefix_ci_behaviors() {
        assert_eq!(
            strip_folder_prefix_ci("MyAddon/init.lua", "myaddon"),
            Some("init.lua")
        );
        assert_eq!(
            strip_folder_prefix_ci("myaddon/Sub/F.lua", "MyAddon"),
            Some("Sub/F.lua")
        );
        // Folder-only entry (no remainder) is rejected.
        assert_eq!(strip_folder_prefix_ci("MyAddon/", "MyAddon"), None);
        // Different folder is rejected.
        assert_eq!(strip_folder_prefix_ci("Other/init.lua", "MyAddon"), None);
        // No slash at all is rejected.
        assert_eq!(strip_folder_prefix_ci("init.lua", "MyAddon"), None);
    }

    // ── ZIP-baseline recording (the perf refactor) ──────────────────────
    //
    // Each test computes the OLD-CODE baseline the slow way (compute_addon_hashes
    // over the extracted folder, then overlay overrides) as an oracle, then asserts
    // the manifest written by record_hashes_with_zip_baseline matches it exactly.

    /// Build the manifest `files` map the OLD code path would have produced:
    /// hash the whole folder from disk, then overlay kept-file overrides.
    fn oracle_baseline(
        addon_path: &Path,
        overrides: Option<&HashMap<String, String>>,
    ) -> HashMap<String, String> {
        let mut files = compute_addon_hashes(addon_path).unwrap();
        if let Some(ov) = overrides {
            for (k, v) in ov {
                if files.contains_key(k) {
                    files.insert(k.clone(), v.clone());
                }
            }
        }
        files
    }

    fn sha256_hex(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn compute_baseline_with_zip_matches_disk_for_covered_files() {
        // When every disk file is covered by the ZIP map, the baseline equals
        // the ZIP hashes — and equals a full disk hash pass (ZIP bytes == disk
        // bytes after extraction).
        let tmp = tempfile::tempdir().unwrap();
        let addon = create_addon_dir(
            tmp.path(),
            "MyAddon",
            &[("init.lua", "a"), ("sub/x.lua", "b")],
        );
        let zip_hashes = compute_addon_hashes(&addon).unwrap();

        let baseline = compute_baseline_with_zip(&addon, &zip_hashes).unwrap();
        assert_eq!(baseline, oracle_baseline(&addon, None));
        assert_eq!(baseline, zip_hashes);
    }

    #[test]
    fn compute_baseline_hashes_disk_only_files_from_disk() {
        // A file on disk but NOT in the ZIP map (lingering from an old version,
        // or user-added) must still be hashed — from disk.
        let tmp = tempfile::tempdir().unwrap();
        let addon = create_addon_dir(
            tmp.path(),
            "MyAddon",
            &[("init.lua", "fresh"), ("legacy.lua", "old-leftover")],
        );
        // ZIP only provides init.lua; legacy.lua is disk-only.
        let mut zip_hashes = HashMap::new();
        zip_hashes.insert("init.lua".to_string(), sha256_hex("fresh"));

        let baseline = compute_baseline_with_zip(&addon, &zip_hashes).unwrap();
        assert_eq!(baseline, oracle_baseline(&addon, None));
        assert_eq!(baseline["legacy.lua"], sha256_hex("old-leftover"));
        assert_eq!(baseline["init.lua"], sha256_hex("fresh"));
    }

    #[test]
    fn compute_baseline_excludes_zip_keys_not_on_disk() {
        // C1 guard: a ZIP key with no corresponding on-disk file (e.g. a symlink
        // entry the extractor skipped) must NOT appear in the baseline, or the
        // next scan would falsely flag it as "deleted".
        let tmp = tempfile::tempdir().unwrap();
        let addon = create_addon_dir(tmp.path(), "MyAddon", &[("init.lua", "x")]);
        let mut zip_hashes = HashMap::new();
        zip_hashes.insert("init.lua".to_string(), sha256_hex("x"));
        zip_hashes.insert("phantom.lua".to_string(), "deadbeef".repeat(8));

        let baseline = compute_baseline_with_zip(&addon, &zip_hashes).unwrap();
        assert!(baseline.contains_key("init.lua"));
        assert!(
            !baseline.contains_key("phantom.lua"),
            "ZIP key with no on-disk file must not be recorded"
        );
        assert_eq!(baseline, oracle_baseline(&addon, None));
    }

    #[test]
    fn record_with_zip_baseline_overwritten_files_match_oracle() {
        // No prior conflict (no kept files): the recorded manifest equals a full
        // disk hash of the extracted folder.
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let addon = create_addon_dir(
            &addons_dir,
            "MyAddon",
            &[("init.lua", "v2"), ("data.lua", "d")],
        );
        let zip = create_test_zip(
            tmp.path(),
            "u.zip",
            "MyAddon",
            &[("init.lua", "v2"), ("data.lua", "d")],
        );
        let zip_hashes = hash_zip_entries(&zip, "MyAddon").unwrap();

        record_hashes_with_zip_baseline(
            &addons_dir,
            &zip,
            &["MyAddon".to_string()],
            "MyAddon",
            &zip_hashes,
            7,
            "2.0",
            None,
        )
        .unwrap();

        let m = load_hash_manifest(&addons_dir, "MyAddon").unwrap();
        assert_eq!(m.files, oracle_baseline(&addon, None));
        assert_eq!(m.esoui_ids, vec![7]);
        assert_eq!(m.installed_version, "2.0");
        assert!(m.modified_files.is_empty());
    }

    #[test]
    fn record_with_zip_baseline_kept_file_stores_upstream_hash() {
        // A kept ("keep_mine") file: the user's bytes stay on disk, but the
        // baseline must store the UPSTREAM (ZIP) hash so the edit is detectable
        // next update. Oracle = disk hash with the override overlaid.
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        // On disk: user's edited init.lua (NOT overwritten because it was skipped).
        let addon = create_addon_dir(
            &addons_dir,
            "MyAddon",
            &[("init.lua", "USER EDIT"), ("other.lua", "o")],
        );
        // ZIP has the upstream init.lua.
        let zip = create_test_zip(
            tmp.path(),
            "u.zip",
            "MyAddon",
            &[("init.lua", "UPSTREAM"), ("other.lua", "o")],
        );
        let zip_hashes = hash_zip_entries(&zip, "MyAddon").unwrap();

        let mut overrides = HashMap::new();
        overrides.insert("init.lua".to_string(), zip_hashes["init.lua"].clone());

        record_hashes_with_zip_baseline(
            &addons_dir,
            &zip,
            &["MyAddon".to_string()],
            "MyAddon",
            &zip_hashes,
            7,
            "2.0",
            Some(&overrides),
        )
        .unwrap();

        let m = load_hash_manifest(&addons_dir, "MyAddon").unwrap();
        assert_eq!(m.files, oracle_baseline(&addon, Some(&overrides)));
        // init.lua baseline = upstream hash, NOT the user's on-disk hash.
        assert_eq!(m.files["init.lua"], sha256_hex("UPSTREAM"));
        assert_ne!(m.files["init.lua"], sha256_hex("USER EDIT"));
        assert_eq!(m.modified_files, vec!["init.lua".to_string()]);
    }

    #[test]
    fn record_with_zip_baseline_keeps_disk_only_files() {
        // File on disk but absent from the ZIP (removed in the new version) must
        // survive in the baseline, hashed from disk — matching old behavior.
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let addon = create_addon_dir(
            &addons_dir,
            "MyAddon",
            &[("init.lua", "new"), ("removed_upstream.lua", "lingering")],
        );
        // ZIP only ships init.lua.
        let zip = create_test_zip(tmp.path(), "u.zip", "MyAddon", &[("init.lua", "new")]);
        let zip_hashes = hash_zip_entries(&zip, "MyAddon").unwrap();

        record_hashes_with_zip_baseline(
            &addons_dir,
            &zip,
            &["MyAddon".to_string()],
            "MyAddon",
            &zip_hashes,
            7,
            "2.0",
            None,
        )
        .unwrap();

        let m = load_hash_manifest(&addons_dir, "MyAddon").unwrap();
        assert_eq!(m.files, oracle_baseline(&addon, None));
        assert_eq!(m.files["removed_upstream.lua"], sha256_hex("lingering"));
    }

    #[test]
    fn record_with_zip_baseline_handles_nested_paths() {
        // Path-separator normalization: nested dirs hash to forward-slash keys
        // identically whether the hash came from the ZIP map or a disk walk.
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let addon = create_addon_dir(
            &addons_dir,
            "MyAddon",
            &[
                ("init.lua", "a"),
                ("controls/slider.lua", "b"),
                ("libs/inner/deep.lua", "c"),
            ],
        );
        let zip = create_test_zip(
            tmp.path(),
            "u.zip",
            "MyAddon",
            &[
                ("init.lua", "a"),
                ("controls/slider.lua", "b"),
                ("libs/inner/deep.lua", "c"),
            ],
        );
        let zip_hashes = hash_zip_entries(&zip, "MyAddon").unwrap();

        record_hashes_with_zip_baseline(
            &addons_dir,
            &zip,
            &["MyAddon".to_string()],
            "MyAddon",
            &zip_hashes,
            7,
            "2.0",
            None,
        )
        .unwrap();

        let m = load_hash_manifest(&addons_dir, "MyAddon").unwrap();
        assert_eq!(m.files, oracle_baseline(&addon, None));
        assert!(m.files.contains_key("controls/slider.lua"));
        assert!(m.files.contains_key("libs/inner/deep.lua"));
        for k in m.files.keys() {
            assert!(!k.contains('\\'), "key {k:?} must use forward slashes");
        }
    }

    #[test]
    fn record_with_zip_baseline_handles_multi_folder_zip() {
        // A ZIP that extracts two top-level folders (bundled library): EACH
        // folder gets its own correct manifest. The primary folder reuses the
        // passed map; the secondary is hashed from the ZIP internally. Overrides
        // apply only to the primary.
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let primary = create_addon_dir(&addons_dir, "AddonX", &[("x.lua", "xx")]);
        let secondary = create_addon_dir(&addons_dir, "LibFoo", &[("foo.lua", "ff")]);

        // One ZIP carrying both folders.
        let zip_path = tmp.path().join("bundle.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("AddonX/x.lua", options).unwrap();
        archive.write_all(b"xx").unwrap();
        archive.start_file("LibFoo/foo.lua", options).unwrap();
        archive.write_all(b"ff").unwrap();
        archive.finish().unwrap();

        let primary_zip_hashes = hash_zip_entries(&zip_path, "AddonX").unwrap();

        record_hashes_with_zip_baseline(
            &addons_dir,
            &zip_path,
            &["AddonX".to_string(), "LibFoo".to_string()],
            "AddonX",
            &primary_zip_hashes,
            42,
            "1.0",
            None,
        )
        .unwrap();

        let mx = load_hash_manifest(&addons_dir, "AddonX").unwrap();
        let ml = load_hash_manifest(&addons_dir, "LibFoo").unwrap();
        assert_eq!(mx.files, oracle_baseline(&primary, None));
        assert_eq!(ml.files, oracle_baseline(&secondary, None));
        assert_eq!(mx.files["x.lua"], sha256_hex("xx"));
        assert_eq!(ml.files["foo.lua"], sha256_hex("ff"));
        // Secondary manifest must NOT inherit primary's keys.
        assert!(!ml.files.contains_key("x.lua"));
    }

    #[test]
    fn record_with_zip_baseline_equivalent_to_old_path_no_manifest() {
        // "No prior manifest" case (fresh-feeling update): record_hashes_with_zip_baseline
        // produces the same manifest as the old record_hashes_for_folders path.
        let tmp = tempfile::tempdir().unwrap();

        // Old path.
        let old_dir = tmp.path().join("Old");
        create_addon_dir(&old_dir, "MyAddon", &[("init.lua", "z"), ("m/n.lua", "q")]);
        record_hashes_for_folders(&old_dir, &["MyAddon".to_string()], 5, "3.0").unwrap();
        let old_m = load_hash_manifest(&old_dir, "MyAddon").unwrap();

        // New path with the same extracted content.
        let new_dir = tmp.path().join("New");
        create_addon_dir(&new_dir, "MyAddon", &[("init.lua", "z"), ("m/n.lua", "q")]);
        let zip = create_test_zip(
            tmp.path(),
            "u.zip",
            "MyAddon",
            &[("init.lua", "z"), ("m/n.lua", "q")],
        );
        let zip_hashes = hash_zip_entries(&zip, "MyAddon").unwrap();
        record_hashes_with_zip_baseline(
            &new_dir,
            &zip,
            &["MyAddon".to_string()],
            "MyAddon",
            &zip_hashes,
            5,
            "3.0",
            None,
        )
        .unwrap();
        let new_m = load_hash_manifest(&new_dir, "MyAddon").unwrap();

        assert_eq!(new_m.files, old_m.files);
        assert_eq!(new_m.modified_files, old_m.modified_files);
        assert_eq!(new_m.esoui_ids, old_m.esoui_ids);
    }

    #[test]
    fn record_with_zip_baseline_secondary_falls_back_on_bad_zip_path() {
        // If a secondary folder's ZIP hashing fails (bad/missing zip path), it
        // must fall back to a full disk hash pass rather than skip the manifest.
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let primary = create_addon_dir(&addons_dir, "AddonX", &[("x.lua", "xx")]);
        let secondary = create_addon_dir(&addons_dir, "LibFoo", &[("foo.lua", "ff")]);

        let mut primary_zip_hashes = HashMap::new();
        primary_zip_hashes.insert("x.lua".to_string(), sha256_hex("xx"));

        // Nonexistent zip path: secondary hash_zip_entries fails → disk fallback.
        let bad_zip = tmp.path().join("does-not-exist.zip");

        record_hashes_with_zip_baseline(
            &addons_dir,
            &bad_zip,
            &["AddonX".to_string(), "LibFoo".to_string()],
            "AddonX",
            &primary_zip_hashes,
            42,
            "1.0",
            None,
        )
        .unwrap();

        // Primary uses the passed map; secondary fell back to disk — both correct.
        let mx = load_hash_manifest(&addons_dir, "AddonX").unwrap();
        let ml = load_hash_manifest(&addons_dir, "LibFoo").unwrap();
        assert_eq!(mx.files, oracle_baseline(&primary, None));
        assert_eq!(ml.files, oracle_baseline(&secondary, None));
        assert_eq!(ml.files["foo.lua"], sha256_hex("ff"));
    }

    #[test]
    fn record_with_zip_baseline_primary_falls_back_to_disk() {
        // If the primary ZIP map is unusable, the primary folder must still get a
        // manifest via a full disk hash pass (not be silently skipped).
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let primary = create_addon_dir(&addons_dir, "MyAddon", &[("init.lua", "real")]);
        let zip = create_test_zip(tmp.path(), "u.zip", "MyAddon", &[("init.lua", "real")]);

        // Empty primary map: compute_baseline_with_zip still succeeds by hashing
        // every file from disk, so the manifest matches the full-disk oracle.
        let empty: HashMap<String, String> = HashMap::new();
        record_hashes_with_zip_baseline(
            &addons_dir,
            &zip,
            &["MyAddon".to_string()],
            "MyAddon",
            &empty,
            7,
            "2.0",
            None,
        )
        .unwrap();

        let m = load_hash_manifest(&addons_dir, "MyAddon").unwrap();
        assert_eq!(m.files, oracle_baseline(&primary, None));
        assert_eq!(m.files["init.lua"], sha256_hex("real"));
    }

    #[test]
    fn record_with_zip_baseline_errors_when_folder_unhashable() {
        // A folder in installed_folders that doesn't exist on disk can't be
        // hashed by either the ZIP-map path or the disk fallback. The function
        // must (a) still write manifests for the folders it CAN hash, and
        // (b) return Err so the caller can fail the update instead of recording
        // metadata for a folder with no baseline.
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let good = create_addon_dir(&addons_dir, "GoodAddon", &[("a.lua", "aa")]);
        // "GhostAddon" is intentionally never created on disk.
        let zip = create_test_zip(tmp.path(), "u.zip", "GoodAddon", &[("a.lua", "aa")]);
        let good_zip_hashes = hash_zip_entries(&zip, "GoodAddon").unwrap();

        let result = record_hashes_with_zip_baseline(
            &addons_dir,
            &zip,
            &["GoodAddon".to_string(), "GhostAddon".to_string()],
            "GoodAddon",
            &good_zip_hashes,
            7,
            "2.0",
            None,
        );

        // The good folder's manifest is still written...
        let mg = load_hash_manifest(&addons_dir, "GoodAddon").unwrap();
        assert_eq!(mg.files, oracle_baseline(&good, None));
        // ...the ghost folder has no manifest...
        assert!(load_hash_manifest(&addons_dir, "GhostAddon").is_none());
        // ...and the overall call reports failure.
        assert!(
            result.is_err(),
            "expected Err when a folder can't be hashed"
        );
        assert!(result.unwrap_err().contains("GhostAddon"));
    }

    #[test]
    fn record_hashes_for_folders_errors_when_folder_unhashable() {
        // The disk-only path has the same contract: report Err if any folder
        // can't be recorded, after writing the ones that can.
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        create_addon_dir(&addons_dir, "GoodAddon", &[("a.lua", "aa")]);

        let result = record_hashes_for_folders(
            &addons_dir,
            &["GoodAddon".to_string(), "GhostAddon".to_string()],
            7,
            "2.0",
        );

        assert!(load_hash_manifest(&addons_dir, "GoodAddon").is_some());
        assert!(load_hash_manifest(&addons_dir, "GhostAddon").is_none());
        assert!(result.is_err());
    }
}
