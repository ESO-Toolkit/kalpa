use super::parser;
use super::serializer;
use super::types::{SavedVariableFile, SvFileStamp, SvReadResponse, SvTreeNode};
use crate::metadata;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// Return the SavedVariables directory relative to the AddOns dir.
pub fn saved_variables_dir(addons_dir: &Path) -> std::path::PathBuf {
    addons_dir
        .parent()
        .unwrap_or(addons_dir)
        .join("SavedVariables")
}

/// Return the kalpa-backups directory relative to the AddOns dir.
pub fn backups_dir(addons_dir: &Path) -> std::path::PathBuf {
    addons_dir
        .parent()
        .unwrap_or(addons_dir)
        .join("kalpa-backups")
}

/// Get the file stamp (size + mtime) for overwrite protection.
fn file_stamp(path: &Path) -> Result<SvFileStamp, String> {
    let meta = fs::metadata(path).map_err(|e| format!("Failed to read file metadata: {}", e))?;
    let modified_epoch_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Ok(SvFileStamp {
        size: meta.len(),
        modified_epoch_ms,
    })
}

/// Extract character keys from a SavedVariables .lua file.
/// Tracks brace depth while respecting string literals so that
/// braces inside string values don't corrupt the depth counter.
pub fn extract_character_keys(content: &str) -> Vec<String> {
    static RE_KEY: OnceLock<Regex> = OnceLock::new();
    let re_key = RE_KEY.get_or_init(|| Regex::new(r#"^\["([^"]+)"\]\s*=\s*\{?\s*$"#).unwrap());

    let mut keys: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Track brace depth while skipping string contents.
    // Character keys live at depth 3 in ESO SavedVariables:
    //   TopVar = {                       depth 0→1
    //       ["Default"] = {              depth 1→2
    //           ["@AccountName"] = {     depth 2→3
    //               ["$AccountWide"] = { depth 3→4  ← skip this
    //               ["CharName"] = {     depth 3→4  ← these are character keys
    let mut depth: i32 = 0;

    for line in content.lines() {
        let trimmed = line.trim();

        if depth == 3 {
            if let Some(cap) = re_key.captures(trimmed) {
                let key = cap[1].to_string();
                if key != "$AccountWide" && seen.insert(key.clone()) {
                    keys.push(key);
                }
            }
        }

        // Count braces on this line while skipping string contents
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'"' | b'\'' => {
                    let quote = bytes[i];
                    i += 1;
                    while i < bytes.len() && bytes[i] != quote {
                        if bytes[i] == b'\\' {
                            i += 1; // skip escaped char
                        }
                        i += 1;
                    }
                    i += 1; // skip closing quote
                }
                b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => {
                    break; // rest of line is a comment
                }
                b'{' => {
                    depth += 1;
                    i += 1;
                }
                b'}' => {
                    depth -= 1;
                    i += 1;
                }
                _ => i += 1,
            }
        }
    }

    keys
}

/// Cache entry for character key extraction. Keyed by file path.
/// When the file's mtime+size hasn't changed, we reuse the cached keys.
#[derive(Clone)]
struct CharKeyCacheEntry {
    size: u64,
    modified_secs: u64,
    keys: Vec<String>,
}

static CHAR_KEY_CACHE: OnceLock<Mutex<HashMap<String, CharKeyCacheEntry>>> = OnceLock::new();

fn char_key_cache() -> &'static Mutex<HashMap<String, CharKeyCacheEntry>> {
    CHAR_KEY_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// List all SavedVariables .lua files with metadata.
/// Uses an in-memory cache for character key extraction so unchanged files
/// don't need to be re-read on subsequent calls.
pub fn list_saved_variables_blocking(addons_dir: &Path) -> Result<Vec<SavedVariableFile>, String> {
    let sv_dir = saved_variables_dir(addons_dir);
    if !sv_dir.is_dir() {
        return Ok(Vec::new());
    }

    let entries =
        fs::read_dir(&sv_dir).map_err(|e| format!("Failed to read SavedVariables: {}", e))?;

    let mut files: Vec<SavedVariableFile> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !file_name.ends_with(".lua") {
            continue;
        }

        let addon_name = file_name
            .strip_suffix(".lua")
            .unwrap_or(&file_name)
            .to_string();

        let meta = fs::metadata(&path).ok();
        let size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let last_modified = meta
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let secs = t
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                metadata::format_timestamp(secs)
            })
            .unwrap_or_default();

        // Extract character keys, using cache when file hasn't changed
        let modified_secs = fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let cache_key = path.to_string_lossy().to_string();
        let character_keys = {
            let cache = char_key_cache();
            let cached = cache
                .lock()
                .ok()
                .and_then(|c| c.get(&cache_key).cloned())
                .filter(|e| e.size == size_bytes && e.modified_secs == modified_secs);

            if let Some(entry) = cached {
                entry.keys
            } else {
                let keys = match fs::File::open(&path) {
                    Ok(mut f) => {
                        use std::io::Read;
                        let read_limit = 256 * 1024;
                        let mut buf = vec![0u8; read_limit.min(size_bytes as usize)];
                        let n = f.read(&mut buf).unwrap_or(0);
                        buf.truncate(n);
                        let content = String::from_utf8_lossy(&buf);
                        extract_character_keys(&content)
                    }
                    Err(_) => Vec::new(),
                };
                if let Ok(mut c) = cache.lock() {
                    c.insert(
                        cache_key,
                        CharKeyCacheEntry {
                            size: size_bytes,
                            modified_secs,
                            keys: keys.clone(),
                        },
                    );
                }
                keys
            }
        };

        files.push(SavedVariableFile {
            file_name,
            addon_name,
            last_modified,
            size_bytes,
            character_keys,
        });
    }

    files.sort_by(|a, b| {
        a.addon_name
            .to_lowercase()
            .cmp(&b.addon_name.to_lowercase())
    });
    Ok(files)
}

/// Read and parse a SavedVariables file, returning the tree and a file stamp
/// for overwrite protection.
pub fn read_saved_variable_blocking(
    addons_dir: &Path,
    file_name: &str,
) -> Result<SvReadResponse, String> {
    let sv_dir = saved_variables_dir(addons_dir);
    let file_path = sv_dir.join(file_name);

    if !file_path.is_file() {
        return Err(format!("File not found: {}", file_name));
    }

    const MAX_READ_SIZE: u64 = 20 * 1024 * 1024; // 20 MB
    let meta =
        fs::metadata(&file_path).map_err(|e| format!("Failed to read file metadata: {}", e))?;
    if meta.len() > MAX_READ_SIZE {
        return Err(format!(
            "{} is too large to edit ({:.1} MB). Maximum is 20 MB.",
            file_name,
            meta.len() as f64 / (1024.0 * 1024.0)
        ));
    }

    let stamp = file_stamp(&file_path)?;
    let content =
        fs::read_to_string(&file_path).map_err(|e| format!("Failed to read file: {}", e))?;
    let tree = parser::parse_sv_file(&content, file_name)?;

    Ok(SvReadResponse { tree, stamp })
}

/// Write a modified tree back to a SavedVariables file.
///
/// Performs overwrite protection by comparing the current file stamp against
/// the stamp captured at read time. If the file changed on disk since it was
/// loaded, the write is rejected.
///
/// Also performs a validation pass: the serialized Lua is re-parsed to ensure
/// it is syntactically valid before touching the original file.
pub fn write_saved_variable_blocking(
    addons_dir: &Path,
    file_name: &str,
    tree: &SvTreeNode,
    expected_stamp: &SvFileStamp,
) -> Result<SvFileStamp, String> {
    let sv_dir = saved_variables_dir(addons_dir);
    let file_path = sv_dir.join(file_name);

    // Overwrite protection: check that the file hasn't changed since we read it
    if file_path.is_file() {
        let current_stamp = file_stamp(&file_path)?;
        if current_stamp.modified_epoch_ms != expected_stamp.modified_epoch_ms
            || current_stamp.size != expected_stamp.size
        {
            return Err("File has been modified externally since you loaded it. \
                 Please reload the file before saving."
                .to_string());
        }
    }

    // Serialize the tree to Lua
    let content = serializer::serialize_to_lua(tree);

    // Size limit
    const MAX_WRITE_SIZE: usize = 50 * 1024 * 1024; // 50 MB
    if content.len() > MAX_WRITE_SIZE {
        return Err(format!(
            "Content is too large ({:.1} MB). Maximum write size is 50 MB.",
            content.len() as f64 / (1024.0 * 1024.0)
        ));
    }

    // Validation pass: re-parse the serialized output to ensure it's valid
    parser::parse_sv_file(&content, file_name)
        .map_err(|e| format!("Serialization validation failed: {}. Save aborted.", e))?;

    // Create a .bak copy before overwriting (automatic backup)
    if file_path.is_file() {
        let bak_path = file_path.with_extension("lua.bak");
        let _ = fs::copy(&file_path, &bak_path);
    }

    // Write atomically via temp file + rename
    let tmp_path = sv_dir.join(format!("{}.tmp", file_name));
    fs::write(&tmp_path, &content).map_err(|e| format!("Failed to write temp file: {}", e))?;
    fs::rename(&tmp_path, &file_path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!("Failed to finalize write: {}", e)
    })?;

    // Return the new stamp so frontend can track for subsequent saves
    file_stamp(&file_path)
}

/// Generate a diff preview: serialize the tree and return both the current file
/// content and the new serialized content so the frontend can display a diff.
pub fn preview_save(
    addons_dir: &Path,
    file_name: &str,
    tree: &SvTreeNode,
) -> Result<(String, String), String> {
    let sv_dir = saved_variables_dir(addons_dir);
    let file_path = sv_dir.join(file_name);

    let original = if file_path.is_file() {
        fs::read_to_string(&file_path).map_err(|e| format!("Failed to read file: {}", e))?
    } else {
        String::new()
    };

    let serialized = serializer::serialize_to_lua(tree);

    Ok((original, serialized))
}

/// Write raw Lua content (used by copy_sv_profile which manipulates raw text).
pub fn write_raw_content(sv_dir: &Path, file_name: &str, content: &str) -> Result<(), String> {
    let file_path = sv_dir.join(file_name);
    let tmp_path = sv_dir.join(format!("{}.tmp", file_name));
    fs::write(&tmp_path, content).map_err(|e| format!("Failed to write temp file: {}", e))?;
    fs::rename(&tmp_path, &file_path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!("Failed to finalize write: {}", e)
    })
}

/// Restore a .bak file back to the original .lua file.
pub fn restore_backup_file(addons_dir: &Path, file_name: &str) -> Result<SvFileStamp, String> {
    let sv_dir = saved_variables_dir(addons_dir);
    let file_path = sv_dir.join(file_name);
    let bak_path = file_path.with_extension("lua.bak");

    if !bak_path.is_file() {
        return Err(format!("No backup found for {}", file_name));
    }

    fs::copy(&bak_path, &file_path).map_err(|e| format!("Failed to restore backup: {}", e))?;

    file_stamp(&file_path)
}

/// Delete selected SavedVariables files after creating an automatic backup.
pub fn delete_saved_variables_blocking(
    addons_dir: &Path,
    file_names: &[String],
) -> Result<u32, String> {
    let sv_dir = saved_variables_dir(addons_dir);
    if !sv_dir.is_dir() {
        return Err("SavedVariables folder not found.".to_string());
    }

    // Auto-backup: copy files to kalpa-backups/auto-cleanup-{timestamp}/
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let backup_name = format!("auto-cleanup-{}", ts);
    let backup_dir = backups_dir(addons_dir).join(&backup_name);
    fs::create_dir_all(&backup_dir)
        .map_err(|e| format!("Failed to create backup folder: {}", e))?;

    for name in file_names {
        let src = sv_dir.join(name);
        if src.is_file() {
            let dest = backup_dir.join(name);
            fs::copy(&src, &dest).map_err(|e| {
                format!(
                    "Backup failed for {}. No files were deleted. Error: {}",
                    name, e
                )
            })?;
        }
    }

    // Delete files (only reached if all backups succeeded)
    let mut deleted = 0u32;
    for name in file_names {
        let path = sv_dir.join(name);
        if path.is_file() && fs::remove_file(&path).is_ok() {
            deleted += 1;
        }
    }

    Ok(deleted)
}
