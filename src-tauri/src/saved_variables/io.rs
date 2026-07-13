use super::parser;
use super::serializer;
use super::types::{
    SavedVariableFile, SvChange, SvChangeType, SvFileStamp, SvReadResponse, SvTreeNode, SvValueType,
};
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
pub fn file_stamp(path: &Path) -> Result<SvFileStamp, String> {
    let meta = fs::metadata(path).map_err(|e| format!("Failed to read file metadata: {e}"))?;
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

/// Extract character keys from an in-memory SavedVariables .lua string.
///
/// Thin wrapper over [`extract_character_keys_from_reader`] so callers that
/// already hold the whole file as a string (and existing tests) keep working.
// The streaming variant is used on the hot path (list_saved_variables_blocking),
// so this string wrapper currently only has test callers; keep it available as
// public API without tripping dead_code in non-test builds (pub fns in a
// cdylib/staticlib crate are treated as potentially dead unless externally used).
#[cfg_attr(not(test), allow(dead_code))]
pub fn extract_character_keys(content: &str) -> Vec<String> {
    // BufRead over the string's bytes; this cannot error, so unwrap_or reuses
    // whatever keys were collected before an (impossible) I/O error.
    extract_character_keys_from_reader(std::io::Cursor::new(content.as_bytes())).unwrap_or_default()
}

/// Extract character keys by streaming a reader line-by-line.
///
/// Tracks brace depth while respecting string literals so that braces inside
/// string values don't corrupt the depth counter. Reads raw bytes per line and
/// converts each with `String::from_utf8_lossy`, so files containing non-UTF8
/// bytes are tolerated without loading the whole file into memory.
///
/// Stops after `MAX_SCAN_BYTES` have been read as a safety bound against
/// pathological/never-ending input.
pub fn extract_character_keys_from_reader<R: std::io::BufRead>(
    mut reader: R,
) -> std::io::Result<Vec<String>> {
    static RE_KEY: OnceLock<Regex> = OnceLock::new();
    let re_key = RE_KEY.get_or_init(|| Regex::new(r#"^\["([^"]+)"\]\s*=\s*\{?\s*$"#).unwrap());

    // Sanity bound: never scan more than 64 MB, even for a pathological file.
    const MAX_SCAN_BYTES: u64 = 64 * 1024 * 1024;

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
    let mut scanned: u64 = 0;
    let mut raw: Vec<u8> = Vec::new();

    loop {
        if scanned >= MAX_SCAN_BYTES {
            break;
        }
        raw.clear();
        let n = reader.read_until(b'\n', &mut raw)?;
        if n == 0 {
            break; // EOF
        }
        scanned += n as u64;

        // Convert this line, tolerating non-UTF8 bytes.
        let line = String::from_utf8_lossy(&raw);
        // Strip the trailing newline (and any \r) for the key-match regex.
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

    Ok(keys)
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

/// Single-entry cache of the most recently parsed original tree for `preview_save`.
/// The SV editor previews one file at a time, and each keystroke-driven preview
/// otherwise re-reads and re-parses the same unchanged on-disk file. Keyed by
/// `(path, SvFileStamp)`; a differing size or mtime invalidates the entry so a
/// file changed on disk is always re-read.
type PreviewCacheEntry = (std::path::PathBuf, SvFileStamp, SvTreeNode);
static PREVIEW_ORIGINAL_CACHE: OnceLock<Mutex<Option<PreviewCacheEntry>>> = OnceLock::new();

fn preview_original_cache() -> &'static Mutex<Option<PreviewCacheEntry>> {
    PREVIEW_ORIGINAL_CACHE.get_or_init(|| Mutex::new(None))
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
        fs::read_dir(&sv_dir).map_err(|e| format!("Failed to read SavedVariables: {e}"))?;

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
                // Stream the whole file line-by-line rather than capping the
                // read. ESO SavedVariables files are commonly multi-megabyte,
                // so a fixed prefix would hide characters later in the file.
                // The reader-based extractor keeps memory bounded (one line at
                // a time) and tolerates non-UTF8 bytes via lossy conversion.
                let keys = match fs::File::open(&path) {
                    Ok(f) => {
                        let reader = std::io::BufReader::new(f);
                        extract_character_keys_from_reader(reader).unwrap_or_else(|e| {
                            eprintln!("Warning: failed to read {}: {}", path.display(), e);
                            Vec::new()
                        })
                    }
                    Err(e) => {
                        eprintln!("Warning: failed to open {}: {}", path.display(), e);
                        Vec::new()
                    }
                };
                if let Ok(mut c) = cache.lock() {
                    const MAX_CACHE_ENTRIES: usize = 200;
                    if c.len() >= MAX_CACHE_ENTRIES {
                        c.clear();
                    }
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
        return Err(format!("File not found: {file_name}"));
    }

    const MAX_READ_SIZE: u64 = 20 * 1024 * 1024; // 20 MB
    let meta =
        fs::metadata(&file_path).map_err(|e| format!("Failed to read file metadata: {e}"))?;
    if meta.len() > MAX_READ_SIZE {
        return Err(format!(
            "{} is too large to edit ({:.1} MB). Maximum is 20 MB.",
            file_name,
            meta.len() as f64 / (1024.0 * 1024.0)
        ));
    }

    let stamp = file_stamp(&file_path)?;
    let content =
        fs::read_to_string(&file_path).map_err(|e| format!("Failed to read file: {e}"))?;
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
    // before touching the user's file. This is intentionally kept unconditional:
    // the serialized `content` was just bounded by `MAX_WRITE_SIZE` (50 MB) above,
    // so the transient parse tree is bounded too, and this is the last safety net
    // that prevents a serializer bug from overwriting a good file with invalid
    // Lua. The cost is paid only on an explicit user save of an editor-sized file.
    parser::parse_sv_file(&content, file_name)
        .map_err(|e| format!("Serialization validation failed: {e}. Save aborted."))?;

    // Create a .bak copy before overwriting (automatic backup)
    if file_path.is_file() {
        let bak_path = file_path.with_extension("lua.bak");
        fs::copy(&file_path, &bak_path)
            .map_err(|e| format!("Failed to create backup before saving: {e}"))?;
    }

    // Write atomically via temp file + rename. `std::fs::rename` replaces the
    // destination atomically on both Unix (`rename(2)`) and Windows
    // (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`), so the live file is
    // never momentarily absent and a failed write leaves the original intact.
    write_raw_bytes(&sv_dir, file_name, content.as_bytes())?;

    // Return the new stamp so frontend can track for subsequent saves
    file_stamp(&file_path)
}

/// Format an SvTreeNode leaf value for display.
fn format_sv_value(node: &SvTreeNode) -> String {
    match node.value_type {
        SvValueType::Nil => "nil".to_string(),
        SvValueType::Boolean => node
            .value
            .as_ref()
            .and_then(|v| v.as_bool())
            .map(|b| if b { "true" } else { "false" }.to_string())
            .unwrap_or_else(|| "false".to_string()),
        SvValueType::Number => node
            .value
            .as_ref()
            .map(|v| {
                if let Some(n) = v.as_f64() {
                    if n == (n as i64) as f64 {
                        format!("{}", n as i64)
                    } else {
                        format!("{n}")
                    }
                } else {
                    v.to_string()
                }
            })
            .unwrap_or_else(|| "0".to_string()),
        SvValueType::String => node
            .value
            .as_ref()
            .and_then(|v| v.as_str())
            .map(|s| format!("\"{s}\""))
            .unwrap_or_else(|| "\"\"".to_string()),
        SvValueType::Table => {
            let count = node.children.as_ref().map(|c| c.len()).unwrap_or(0);
            format!("{{...}} ({count} entries)")
        }
    }
}

/// Recursively diff two trees and collect changes.
fn diff_trees(
    old: &SvTreeNode,
    new: &SvTreeNode,
    path: &mut Vec<String>,
    changes: &mut Vec<SvChange>,
) {
    match (old.value_type, new.value_type) {
        (SvValueType::Table, SvValueType::Table) => {
            // Build lookup maps by key for children
            let old_children: std::collections::HashMap<&str, &SvTreeNode> = old
                .children
                .as_ref()
                .map(|c| c.iter().map(|n| (n.key.as_str(), n)).collect())
                .unwrap_or_default();
            let new_children: std::collections::HashMap<&str, &SvTreeNode> = new
                .children
                .as_ref()
                .map(|c| c.iter().map(|n| (n.key.as_str(), n)).collect())
                .unwrap_or_default();

            // Check removed and modified
            if let Some(old_c) = &old.children {
                for child in old_c {
                    path.push(child.key.clone());
                    if let Some(new_child) = new_children.get(child.key.as_str()) {
                        diff_trees(child, new_child, path, changes);
                    } else {
                        changes.push(SvChange {
                            path: path.clone(),
                            change_type: SvChangeType::Removed,
                            old_value: Some(format_sv_value(child)),
                            new_value: None,
                        });
                    }
                    path.pop();
                }
            }
            // Check added
            if let Some(new_c) = &new.children {
                for child in new_c {
                    if !old_children.contains_key(child.key.as_str()) {
                        path.push(child.key.clone());
                        changes.push(SvChange {
                            path: path.clone(),
                            change_type: SvChangeType::Added,
                            old_value: None,
                            new_value: Some(format_sv_value(child)),
                        });
                        path.pop();
                    }
                }
            }
        }
        _ => {
            // Leaf comparison — check if type or value changed.
            // serde_json stores i64 and f64 as different internal representations,
            // so 42 (i64 from JS) != 42.0 (f64 from parser) even though they're
            // the same number. Normalize to f64 for comparison.
            let values_equal = match (&old.value, &new.value) {
                (Some(a), Some(b)) if a.is_number() && b.is_number() => a.as_f64() == b.as_f64(),
                (a, b) => a == b,
            };
            if old.value_type != new.value_type || !values_equal {
                changes.push(SvChange {
                    path: path.clone(),
                    change_type: SvChangeType::Modified,
                    old_value: Some(format_sv_value(old)),
                    new_value: Some(format_sv_value(new)),
                });
            }
        }
    }
}

/// Generate a diff preview by comparing the on-disk tree against the edited tree.
pub fn preview_save(
    addons_dir: &Path,
    file_name: &str,
    tree: &SvTreeNode,
) -> Result<Vec<SvChange>, String> {
    let sv_dir = saved_variables_dir(addons_dir);
    let file_path = sv_dir.join(file_name);

    if !file_path.is_file() {
        return Ok(Vec::new());
    }

    // Cache the parsed on-disk tree keyed by (path, size+mtime). Repeated previews
    // of the same unchanged file (the common case while editing) then skip both
    // the read and the parse. The stamp is captured first so a file modified on
    // disk since the last preview is detected and re-read, keeping behaviour —
    // including the read/parse error paths — identical to the uncached version.
    let stamp = file_stamp(&file_path)?;
    let cache = preview_original_cache();
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());

    let hit = matches!(
        guard.as_ref(),
        Some((p, s, _))
            if p == &file_path
                && s.size == stamp.size
                && s.modified_epoch_ms == stamp.modified_epoch_ms
    );

    if !hit {
        let original_content =
            fs::read_to_string(&file_path).map_err(|e| format!("Failed to read file: {e}"))?;
        let original_tree = parser::parse_sv_file(&original_content, file_name)?;
        *guard = Some((file_path.clone(), stamp, original_tree));
    }

    // `guard` now holds a valid entry for this (path, stamp); diff against it
    // while holding the lock (previews are one-file-at-a-time and the diff is
    // cheap, so serializing them is fine and avoids cloning the cached tree).
    let original_tree = &guard.as_ref().expect("cache populated above").2;

    let mut changes = Vec::new();
    let mut path = Vec::new();
    diff_trees(original_tree, tree, &mut path, &mut changes);

    Ok(changes)
}

/// Write raw Lua content (used by copy_sv_profile which manipulates raw text).
pub fn write_raw_content(sv_dir: &Path, file_name: &str, content: &str) -> Result<(), String> {
    write_raw_bytes(sv_dir, file_name, content.as_bytes())
}

/// Write raw bytes to `sv_dir/file_name` with no lossy UTF-8 round-trip. Used by
/// the per-character backup/restore path, whose merged content may contain
/// non-UTF8 SavedVariables bytes (caret keys, addon binary blobs).
///
/// The new content is written to `<file>.tmp` and then `rename`d into place.
/// `std::fs::rename` replaces the destination ATOMICALLY on both Unix
/// (`rename(2)`) and Windows (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`), so
/// the live file is never momentarily absent — there is no remove-then-rename
/// crash window, and a failed write leaves the original file untouched. No
/// `.bak`/`.old` is left behind: the per-character restore already takes a full
/// safety snapshot, so a lingering sidecar in the live folder (which later
/// backups/snapshots would sweep up) is avoided.
pub fn write_raw_bytes(sv_dir: &Path, file_name: &str, content: &[u8]) -> Result<(), String> {
    let file_path = sv_dir.join(file_name);
    let tmp_path = sv_dir.join(format!("{file_name}.tmp"));
    fs::write(&tmp_path, content).map_err(|e| format!("Failed to write temp file: {e}"))?;
    fs::rename(&tmp_path, &file_path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!("Failed to finalize write: {e}")
    })
}

/// Restore a .bak file back to the original .lua file.
pub fn restore_backup_file(addons_dir: &Path, file_name: &str) -> Result<SvFileStamp, String> {
    let sv_dir = saved_variables_dir(addons_dir);
    let file_path = sv_dir.join(file_name);
    let bak_path = file_path.with_extension("lua.bak");

    if !bak_path.is_file() {
        return Err(format!("No backup found for {file_name}"));
    }

    let bytes = fs::read(&bak_path).map_err(|e| format!("Failed to restore backup: {e}"))?;
    write_raw_bytes(&sv_dir, file_name, &bytes)?;

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
    let backup_name = format!("auto-cleanup-{ts}");
    let backup_dir = backups_dir(addons_dir).join(&backup_name);
    fs::create_dir_all(&backup_dir).map_err(|e| format!("Failed to create backup folder: {e}"))?;

    for name in file_names {
        let src = sv_dir.join(name);
        if src.is_file() {
            let dest = backup_dir.join(name);
            fs::copy(&src, &dest).map_err(|e| {
                format!("Backup failed for {name}. No files were deleted. Error: {e}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_raw_bytes_atomically_replaces_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let path = dir.join("x.lua");
        fs::write(&path, b"OLD").unwrap();

        // Non-UTF8 content overwrites the existing file in place.
        let mut new_content = b"NEW ".to_vec();
        new_content.extend_from_slice(&[0xff, 0xfe]);
        write_raw_bytes(dir, "x.lua", &new_content).unwrap();

        assert_eq!(fs::read(&path).unwrap(), new_content);
        // No scratch sidecars are left behind.
        assert!(!dir.join("x.lua.tmp").exists());
        assert!(!dir.join("x.lua.old").exists());
        assert!(!dir.join("x.lua.bak").exists());
    }

    #[test]
    fn extract_character_keys_finds_keys_past_256kb() {
        // Build a file where a character sits well past the old 256 KB prefix
        // cap, so a capped read would miss it. Padding lives inside a string
        // value at character-key depth (3) so it doesn't disturb brace depth.
        let padding = "x".repeat(400 * 1024);
        let content = format!(
            "TopVar =\n{{\n\t[\"Default\"] =\n\t{{\n\t\t[\"@Acct\"] =\n\t\t{{\n\
             \t\t\t[\"EarlyChar\"] =\n\t\t\t{{\n\t\t\t\t[\"pad\"] = \"{padding}\",\n\t\t\t}},\n\
             \t\t\t[\"LateChar\"] =\n\t\t\t{{\n\t\t\t\t[\"level\"] = 50,\n\t\t\t}},\n\
             \t\t}},\n\t}},\n}}\n"
        );
        let keys = extract_character_keys(&content);
        assert!(keys.contains(&"EarlyChar".to_string()));
        assert!(
            keys.contains(&"LateChar".to_string()),
            "character after 256 KB should still be extracted via streaming"
        );
    }

    #[test]
    fn extract_character_keys_from_reader_matches_string_variant() {
        let content = r#"TopVar =
{
	["Default"] =
	{
		["@Acct"] =
		{
			["$AccountWide"] =
			{
			},
			["Baelthor"] =
			{
			},
		},
	},
}
"#;
        let via_str = extract_character_keys(content);
        let via_reader =
            extract_character_keys_from_reader(std::io::Cursor::new(content.as_bytes())).unwrap();
        assert_eq!(via_str, via_reader);
        assert_eq!(via_str, vec!["Baelthor".to_string()]);
    }

    #[test]
    fn write_raw_bytes_creates_new_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_raw_bytes(dir, "new.lua", b"hello").unwrap();
        assert_eq!(fs::read(dir.join("new.lua")).unwrap(), b"hello");
        assert!(!dir.join("new.lua.tmp").exists());
    }

    #[test]
    fn write_saved_variable_replaces_content_and_leaves_no_tmp() {
        // Lay out <tmp>/live/AddOns and <tmp>/live/SavedVariables so the
        // function's addons_dir.parent()/SavedVariables derivation resolves.
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("live").join("AddOns");
        let sv_dir = tmp.path().join("live").join("SavedVariables");
        fs::create_dir_all(&addons_dir).unwrap();
        fs::create_dir_all(&sv_dir).unwrap();

        let file_name = "MyAddon.lua";
        let file_path = sv_dir.join(file_name);

        // Seed an existing file whose content differs from what we'll write back.
        let initial = "MyAddon_SV =\n{\n\t[\"old\"] = 1,\n}\n";
        fs::write(&file_path, initial).unwrap();
        let stamp = file_stamp(&file_path).unwrap();

        // Build the tree to write from a small SV source.
        let new_source = r#"MyAddon_SV =
{
	["Default"] =
	{
		["@Account"] =
		{
			["CharName"] =
			{
				["enabled"] = true,
			},
		},
	},
}
"#;
        let tree = parser::parse_sv_file(new_source, file_name).unwrap();

        let new_stamp =
            write_saved_variable_blocking(&addons_dir, file_name, &tree, &stamp).unwrap();

        // Content was replaced with the serialized tree.
        let written = fs::read_to_string(&file_path).unwrap();
        assert_eq!(written, serializer::serialize_to_lua(&tree));
        assert_ne!(written, initial);
        assert!(written.contains("[\"CharName\"]"));

        // No .tmp sidecar left behind, and a .lua.bak was created.
        assert!(!sv_dir.join(format!("{file_name}.tmp")).exists());
        assert!(file_path.with_extension("lua.bak").is_file());

        // The returned stamp matches the freshly written file.
        let disk_stamp = file_stamp(&file_path).unwrap();
        assert_eq!(new_stamp.size, disk_stamp.size);
        assert_eq!(new_stamp.modified_epoch_ms, disk_stamp.modified_epoch_ms);
    }

    #[test]
    fn extract_character_keys_finds_identifier_safe_key_in_serializer_output() {
        // A character key that is a valid Lua identifier must still be emitted
        // in ["key"] = form so extract_character_keys (which only matches
        // bracket style at depth 3) keeps seeing it after a round-trip save.
        let input = r#"MyAddon_SV =
{
	["Default"] =
	{
		["@Account"] =
		{
			CharName =
			{
				["enabled"] = true,
			},
		},
	},
}
"#;
        let tree = parser::parse_sv_file(input, "MyAddon.lua").unwrap();
        let output = serializer::serialize_to_lua(&tree);
        let keys = extract_character_keys(&output);
        assert!(
            keys.contains(&"CharName".to_string()),
            "expected CharName in {keys:?}"
        );
    }
}
