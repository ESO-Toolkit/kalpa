use crate::metadata;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
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

/// Compute the SHA-256 hex digest of a single file, streamed in 8 KiB chunks.
/// Public so callers that edit one file (e.g. the addon file editor) can
/// re-hash just that file instead of walking the whole addon folder.
pub fn hash_file(path: &Path) -> Result<String, String> {
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

/// Prefix marking a cheap size-only signature, as opposed to a 64-hex SHA-256
/// digest. The colon guarantees it can never collide with a hash hex string.
const SIZE_SIG_PREFIX: &str = "size:";

fn size_signature(len: u64) -> String {
    format!("{SIZE_SIG_PREFIX}{len}")
}

fn is_size_signature(value: &str) -> bool {
    value.starts_with(SIZE_SIG_PREFIX)
}

/// File extensions whose CONTENTS we hash (SHA-256) so a user's hand-edit is
/// detected on the next update. Every other file gets a cheap size signature
/// instead.
///
/// Hashing thousands of static binary assets — e.g. the 5,600+ `.dds` textures a
/// media library like LibCustomIcons ships — just to detect edits that never
/// happen is the dominant cost of updating large addons (a single-threaded pass
/// over ~5,600 files measured in minutes). Conflict detection for the files a
/// user can actually edit (Lua/XML/settings/manifests) is unchanged; for binary
/// assets we still flag a size change, only not a same-size in-place edit.
fn hashes_file_contents(key: &str) -> bool {
    let ext = key
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase());
    matches!(
        ext.as_deref(),
        Some(
            "lua"
                | "xml"
                | "txt"
                | "addon"
                | "md"
                | "json"
                | "toc"
                | "def"
                | "lang"
                | "csv"
                | "cfg"
                | "ini"
                | "html"
                | "htm"
        )
    )
}

/// Compute the baseline signature for a single on-disk file: a SHA-256 of its
/// contents for user-editable types, or a size signature for binary assets.
/// `key` is the forward-slash relative path used only to classify by extension.
///
/// Public so the single-file editor save path records the same signature kind
/// this module would, instead of always hashing contents.
pub fn file_signature(key: &str, path: &Path) -> Result<String, String> {
    if hashes_file_contents(key) {
        hash_file(path)
    } else {
        let len = fs::metadata(path)
            .map_err(|e| format!("Failed to read file metadata for signature: {e}"))?
            .len();
        Ok(size_signature(len))
    }
}

/// Whether a freshly-computed signature represents the SAME file state as a
/// stored baseline value. Use this instead of `==` everywhere a live signature
/// is compared against a manifest entry.
///
/// Besides exact equality, this handles the migration from the old
/// hash-everything manifests: a binary file recorded as a 64-hex SHA-256 in an
/// older manifest will not equal its new `size:` signature, but that mismatch is
/// a format difference, not a user edit. A stored content hash vs a fresh size
/// signature (or the reverse) is therefore treated as a match; the next baseline
/// write replaces the legacy value with a size signature and normal comparison
/// resumes. Two values of the SAME kind that differ are a real change.
///
/// Accepted limitation: during that single migration window, a stored content
/// hash can't be compared against a size signature, so a user's hand-edit to a
/// non-content-hashed binary file (e.g. a swapped `.dds`/`.ttf`) is treated as
/// unchanged even if it changed the file's size — that one update may overwrite
/// it without a conflict prompt. We accept this rather than the alternatives,
/// which each defeat a core goal of the size-signature design: comparing the
/// disk size against the upstream/ZIP size instead would resurface the
/// thousands-of-textures conflict storm whenever an upstream update legitimately
/// changes binary sizes, and re-hashing the binaries to compare exactly would
/// reintroduce the multi-minute hashing cost this change exists to remove.
/// Binary assets inside addons are effectively never hand-edited, and after this
/// one update the baseline self-heals to size signatures so future size-changing
/// edits are detected normally.
pub fn signatures_match(stored: &str, current: &str) -> bool {
    if stored == current {
        return true;
    }
    // Mixed kinds (one size signature, one content hash) only arise from a
    // pre-migration manifest entry for a binary file — assume unchanged.
    is_size_signature(stored) != is_size_signature(current)
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

/// Walk an addon folder and compute a baseline signature for every file (a
/// SHA-256 for editable types, a size signature for binary assets — see
/// [`file_signature`]). Keys are forward-slash-normalized relative paths.
///
/// The walk is sequential (directory I/O), but the per-file signatures are
/// computed in parallel with rayon: on a large media addon the previous
/// single-threaded pass over thousands of files was the dominant update cost.
pub fn compute_addon_hashes(addon_path: &Path) -> Result<HashMap<String, String>, String> {
    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    walk_addon_files(addon_path, |key, path| {
        entries.push((key, path.to_path_buf()));
        Ok(())
    })?;
    entries
        .into_par_iter()
        .map(|(key, path)| {
            let sig = file_signature(&key, &path)?;
            Ok((key, sig))
        })
        .collect()
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
    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    walk_addon_files(addon_path, |key, path| {
        entries.push((key, path.to_path_buf()));
        Ok(())
    })?;
    entries
        .into_par_iter()
        .map(|(key, path)| {
            let value = match zip_hashes.get(&key) {
                Some(h) => h.clone(),
                None => file_signature(&key, &path)?,
            };
            Ok((key, value))
        })
        .collect()
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

    let prefix = format!("{folder_name}/");
    let mut hashes = HashMap::new();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry: {e}"))?;

        if entry.is_dir() {
            continue;
        }

        // Skip symlink entries, exactly as `extract_addon_zip*` does. Otherwise a
        // symlink entry would be hashed here but never written to disk, so a
        // symlink whose path collides with a real on-disk file would record the
        // symlink's payload hash as that file's baseline — a false mismatch on the
        // next scan.
        if let Some(mode) = entry.unix_mode() {
            if mode & 0o170000 == 0o120000 {
                continue;
            }
        }

        let name = match entry.enclosed_name() {
            Some(p) => p.to_string_lossy().replace('\\', "/"),
            None => continue,
        };

        let relative = match name.strip_prefix(&prefix) {
            Some(r) if !r.is_empty() => r.to_string(),
            _ => continue,
        };

        // Binary assets get a size signature without decompressing the entry —
        // `entry.size()` is the uncompressed size, identical to the file's size
        // on disk after extraction, so it matches the disk-side signature in
        // [`file_signature`]. Editable files are still content-hashed.
        let value = if hashes_file_contents(&relative) {
            stream_sha256(&mut entry)
                .map_err(|e| format!("Failed to read ZIP entry {name}: {e}"))?
        } else {
            size_signature(entry.size())
        };
        hashes.insert(relative, value);
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
            Some(disk_hash) if !signatures_match(stored_hash, disk_hash) => {
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
    // For kept files, record the upstream ZIP hash as the baseline so the user's
    // change stays detectable on the next update. This is inserted
    // UNCONDITIONALLY: a kept file the user *deleted* (auto-kept because upstream
    // didn't touch it) is absent from `files`, but it still needs its upstream
    // hash stored — otherwise the next scan finds no baseline entry, treats the
    // path as untracked, and silently re-extracts the file the user removed.
    if let Some(overrides) = hash_overrides {
        for (path, upstream_hash) in overrides {
            files.insert(path.clone(), upstream_hash.clone());
        }
    }

    // Every kept file is, by definition, a user modification (edit or deletion).
    let modified_files: Vec<String> = match hash_overrides {
        Some(ov) => {
            let mut m: Vec<String> = ov.keys().cloned().collect();
            m.sort();
            m
        }
        None => Vec::new(),
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
    fn hash_file_matches_compute_addon_hashes_entry() {
        // The addon file editor relies on hash_file(single) producing the same
        // digest that compute_addon_hashes(folder) records for that file.
        let tmp = tempfile::tempdir().unwrap();
        let content = "local greeting = 'hello'";
        let addon_path = create_addon_dir(tmp.path(), "Editable", &[("core.lua", content)]);

        let folder_hashes = compute_addon_hashes(&addon_path).unwrap();
        let single = hash_file(&addon_path.join("core.lua")).unwrap();

        assert_eq!(single.len(), 64);
        assert_eq!(single, folder_hashes["core.lua"]);
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

    #[test]
    fn record_with_zip_baseline_kept_deletion_stores_upstream_hash() {
        // A user DELETED a file that upstream still ships. The conflict scan
        // auto-keeps the deletion and selective extraction skips re-creating it,
        // so the file is absent from disk during baseline recording. The override
        // must still be stored (upstream hash) so the next scan keeps detecting
        // the deletion — otherwise the file would be silently re-extracted.
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        // settings.lua is NOT on disk (user deleted it; extraction skipped it).
        create_addon_dir(&addons_dir, "MyAddon", &[("init.lua", "code")]);
        let zip = create_test_zip(
            tmp.path(),
            "u.zip",
            "MyAddon",
            &[("init.lua", "code"), ("settings.lua", "UPSTREAM")],
        );
        let zip_hashes = hash_zip_entries(&zip, "MyAddon").unwrap();

        // The kept deletion's override carries the upstream hash.
        let mut overrides = HashMap::new();
        overrides.insert(
            "settings.lua".to_string(),
            zip_hashes["settings.lua"].clone(),
        );

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
        // The deleted-but-kept file IS in the baseline with the upstream hash...
        assert_eq!(m.files.get("settings.lua"), Some(&sha256_hex("UPSTREAM")));
        // ...and is flagged as a user modification so the deletion stays tracked.
        assert!(m.modified_files.contains(&"settings.lua".to_string()));
        // The on-disk file is still recorded normally.
        assert_eq!(m.files["init.lua"], sha256_hex("code"));
    }

    #[test]
    fn hash_zip_entries_skips_symlink_entries() {
        // A symlink ZIP entry must not be hashed: the extractor skips writing it,
        // so hashing it would record a baseline for a path whose real on-disk
        // bytes differ, producing a false "modified" flag next scan.
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("sym.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();

        // A normal file...
        archive.start_file("MyAddon/real.lua", opts).unwrap();
        archive.write_all(b"real").unwrap();

        // ...and a real symlink entry (sets the 0o120000 symlink mode bits).
        archive
            .add_symlink("MyAddon/link.lua", "../target", opts)
            .unwrap();
        archive.finish().unwrap();

        let hashes = hash_zip_entries(&zip_path, "MyAddon").unwrap();
        assert!(hashes.contains_key("real.lua"));
        assert!(
            !hashes.contains_key("link.lua"),
            "symlink entry must be skipped, like extraction does"
        );
    }

    // ── Binary-asset size signatures + legacy-manifest migration ─────────

    #[test]
    fn binary_files_get_size_signature_not_sha256() {
        // A .dds texture must record a cheap `size:N` signature, while a .lua
        // file in the same folder still gets a 64-hex SHA-256.
        let tmp = tempfile::tempdir().unwrap();
        let addon = create_addon_dir(
            tmp.path(),
            "MediaAddon",
            &[("icons/a.dds", "BINARYBYTES"), ("core.lua", "local x = 1")],
        );
        let hashes = compute_addon_hashes(&addon).unwrap();

        assert_eq!(hashes["icons/a.dds"], size_signature(11)); // "BINARYBYTES".len()
        assert!(is_size_signature(&hashes["icons/a.dds"]));
        assert_eq!(hashes["core.lua"].len(), 64); // SHA-256 hex
        assert!(!is_size_signature(&hashes["core.lua"]));
    }

    #[test]
    fn binary_signature_matches_between_disk_and_zip() {
        // The disk-side and ZIP-side signatures for a binary file must be equal
        // (both size-based) so a clean update produces no spurious conflict.
        let tmp = tempfile::tempdir().unwrap();
        let content = "texture-payload-bytes";
        let addon = create_addon_dir(tmp.path(), "MediaAddon", &[("icons/a.dds", content)]);
        let zip = create_test_zip(
            tmp.path(),
            "u.zip",
            "MediaAddon",
            &[("icons/a.dds", content)],
        );

        let disk = compute_addon_hashes(&addon).unwrap();
        let zip_hashes = hash_zip_entries(&zip, "MediaAddon").unwrap();
        assert_eq!(disk["icons/a.dds"], zip_hashes["icons/a.dds"]);
        assert!(is_size_signature(&zip_hashes["icons/a.dds"]));
        // The ZIP-baseline merge agrees too.
        let baseline = compute_baseline_with_zip(&addon, &zip_hashes).unwrap();
        assert_eq!(baseline, disk);
    }

    #[test]
    fn signatures_match_bridges_legacy_hash_and_size_sig() {
        let sha = sha256_hex("anything");
        let sig = size_signature(123);
        // Legacy SHA-256 vs new size signature (either order) = treated as a match.
        assert!(signatures_match(&sha, &sig));
        assert!(signatures_match(&sig, &sha));
        // Same kind, equal = match.
        assert!(signatures_match(&sig, &size_signature(123)));
        assert!(signatures_match(&sha, &sha));
        // Same kind, different = real change.
        assert!(!signatures_match(&sig, &size_signature(456)));
        assert!(!signatures_match(&sha, &sha256_hex("other")));
    }

    #[test]
    fn detect_modifications_flags_binary_size_change_but_not_legacy_format() {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        create_addon_dir(&addons_dir, "MediaAddon", &[("icons/a.dds", "AAAA")]);

        // Baseline records a size signature for the .dds.
        record_hashes_for_folders(&addons_dir, &["MediaAddon".to_string()], 1, "1.0").unwrap();
        let baseline = load_hash_manifest(&addons_dir, "MediaAddon").unwrap();
        assert!(is_size_signature(&baseline.files["icons/a.dds"]));

        // Same size, different bytes → NOT flagged (acceptable tradeoff).
        fs::write(addons_dir.join("MediaAddon/icons/a.dds"), "BBBB").unwrap();
        assert!(detect_modifications(&addons_dir, "MediaAddon")
            .unwrap()
            .is_empty());

        // Different size → flagged.
        fs::write(addons_dir.join("MediaAddon/icons/a.dds"), "BBBBB").unwrap();
        assert_eq!(
            detect_modifications(&addons_dir, "MediaAddon").unwrap(),
            vec!["icons/a.dds"]
        );
    }

    #[test]
    fn detect_modifications_ignores_legacy_sha_for_unchanged_binary() {
        // A manifest written by the OLD code stores a SHA-256 for a .dds. The
        // file is unchanged on disk, but the new code computes a size signature.
        // The mismatch must NOT be flagged as a user modification.
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        create_addon_dir(&addons_dir, "MediaAddon", &[("icons/a.dds", "TEXTURE")]);

        let hashes_dir = addons_dir.join(".kalpa-hashes");
        fs::create_dir_all(&hashes_dir).unwrap();
        let legacy = format!(
            r#"{{"addon_folder":"MediaAddon","esoui_ids":[1],"recorded_at":"2025-01-01T00-00-00Z","installed_version":"1.0","files":{{"icons/a.dds":"{}"}}}}"#,
            sha256_hex("TEXTURE")
        );
        fs::write(hashes_dir.join("MediaAddon.json"), legacy).unwrap();

        let modified = detect_modifications(&addons_dir, "MediaAddon").unwrap();
        assert!(
            modified.is_empty(),
            "legacy SHA vs new size signature must not be flagged, got: {modified:?}"
        );
    }

    #[test]
    fn record_pass_self_heals_legacy_binary_sha_to_size_signature() {
        // The migration's correctness guarantee is that the lenient mixed-kind
        // bridge applies only for ONE update: a record pass must rewrite a legacy
        // 64-hex SHA for a binary file into a `size:` signature, so normal exact
        // comparison resumes afterward. This pins that self-heal end-to-end.
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        create_addon_dir(&addons_dir, "MediaAddon", &[("icons/a.dds", "TEXTURE")]);

        let hashes_dir = addons_dir.join(".kalpa-hashes");
        fs::create_dir_all(&hashes_dir).unwrap();
        let legacy_sha = sha256_hex("TEXTURE");
        let legacy = format!(
            r#"{{"addon_folder":"MediaAddon","esoui_ids":[1],"recorded_at":"2025-01-01T00-00-00Z","installed_version":"1.0","files":{{"icons/a.dds":"{legacy_sha}"}}}}"#,
        );
        fs::write(hashes_dir.join("MediaAddon.json"), legacy).unwrap();

        // A record pass (the same one every update performs) rebuilds the manifest
        // from freshly computed signatures.
        record_hashes_for_folders(&addons_dir, &["MediaAddon".to_string()], 1, "2.0").unwrap();

        let healed = load_hash_manifest(&addons_dir, "MediaAddon").unwrap();
        let entry = &healed.files["icons/a.dds"];
        assert!(
            is_size_signature(entry),
            "binary entry must self-heal to a size signature, got: {entry}"
        );
        assert_ne!(entry, &legacy_sha, "the legacy SHA must be replaced");
    }
}
