use super::io;
use super::parser::{self, skip_lua_comment, skip_lua_string};
use std::fs;
use std::path::Path;

/// Find a substring at a specific brace depth, skipping string literals and comments.
pub fn find_key_at_depth(content: &str, pattern: &str, target_depth: i32) -> Option<usize> {
    let bytes = content.as_bytes();
    let pat_bytes = pattern.as_bytes();
    let mut i = 0;
    let mut depth: i32 = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => i = skip_lua_string(bytes, i),
            b'[' if i + 1 < bytes.len() && (bytes[i + 1] == b'[' || bytes[i + 1] == b'=') => {
                i = skip_lua_string(bytes, i)
            }
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => i = skip_lua_comment(bytes, i),
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                i += 1;
            }
            _ => {
                if depth == target_depth
                    && i + pat_bytes.len() <= bytes.len()
                    && &bytes[i..i + pat_bytes.len()] == pat_bytes
                {
                    return Some(i);
                }
                i += 1;
            }
        }
    }
    None
}

/// Find the matching closing brace for an opening brace, respecting strings.
pub fn find_matching_brace(content: &str, open_pos: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut depth = 0i32;
    let mut i = open_pos;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => i = skip_lua_string(bytes, i),
            b'[' if i + 1 < bytes.len() && (bytes[i + 1] == b'[' || bytes[i + 1] == b'=') => {
                i = skip_lua_string(bytes, i)
            }
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => i = skip_lua_comment(bytes, i),
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

/// Find the innermost enclosing open-brace position for a byte `offset`.
///
/// Scans from the start of `content` tracking a stack of positions of
/// still-unclosed `{`, respecting string literals and comments (like the other
/// helpers).  Returns the position of the innermost `{` that is still open when
/// `offset` is reached — i.e. the enclosing account table's `{` when `offset`
/// points at a character key inside it.  Character keys that are direct children
/// of that table sit at brace depth 1 relative to the returned brace.
pub fn find_enclosing_brace(content: &str, offset: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut stack: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < bytes.len() && i < offset {
        match bytes[i] {
            b'"' | b'\'' => i = skip_lua_string(bytes, i),
            b'[' if i + 1 < bytes.len() && (bytes[i + 1] == b'[' || bytes[i + 1] == b'=') => {
                i = skip_lua_string(bytes, i)
            }
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => i = skip_lua_comment(bytes, i),
            b'{' => {
                stack.push(i);
                i += 1;
            }
            b'}' => {
                stack.pop();
                i += 1;
            }
            _ => i += 1,
        }
    }
    stack.last().copied()
}

/// Starting just past a `["key"] =` match, skip only ASCII whitespace and return
/// the position of the opening `{` of a table value.  Returns `None` if any
/// non-whitespace byte other than `{` intervenes — i.e. the value is a scalar
/// (`= 5,`) rather than a table.  This prevents grabbing the `{` that belongs to
/// a *following* entry when the matched key has a scalar value.
fn skip_ws_to_brace(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'{' => return Some(i),
            _ => return None,
        }
    }
    None
}

/// Copy a character profile within a SavedVariables file.
///
/// Captures a file stamp before reading, then re-checks it before writing to
/// detect concurrent modifications.  The generated Lua is re-parsed before
/// writing to ensure it is syntactically valid.
///
/// The source key and its enclosing account table are located first; any
/// existing destination key is then removed *only from within that same account
/// table* and the copy is inserted into it, so a destination that happens to
/// exist under a different account (or a different top-level SV table) is never
/// touched.
pub fn copy_sv_profile_blocking(
    addons_dir: &Path,
    file_name: &str,
    from_key: &str,
    to_key: &str,
) -> Result<(), String> {
    let sv_dir = io::saved_variables_dir(addons_dir);
    let file_path = sv_dir.join(file_name);

    if !file_path.is_file() {
        return Err(format!("File not found: {file_name}"));
    }

    // Size guard: mirror read_saved_variable_blocking so we never slurp a
    // pathologically large file into memory.
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

    // Capture stamp before reading for overwrite protection
    let read_stamp = io::file_stamp(&file_path)?;

    let mut content =
        fs::read_to_string(&file_path).map_err(|e| format!("Failed to read file: {e}"))?;

    // Find the source key at depth 3 (character keys in ESO SV files).
    let search_pattern = format!("[\"{from_key}\"]\u{0020}=");
    let key_start = find_key_at_depth(&content, &search_pattern, 3)
        .ok_or_else(|| format!("Source key \"{from_key}\" not found at expected depth."))?;

    // Determine the byte span of the source key's enclosing account table so all
    // destination handling stays scoped to it.
    let acct_open = find_enclosing_brace(&content, key_start)
        .ok_or_else(|| format!("Source key \"{from_key}\" is not inside a table."))?;
    let acct_close = find_matching_brace(&content, acct_open)
        .ok_or("Unbalanced braces around the source's account table.")?;

    // If to_key already exists WITHIN THE SAME ACCOUNT TABLE, remove the old
    // block first.  Relative to the account table's `{`, character keys are at
    // depth 1, so the search is naturally confined to this account.
    let dest_pattern = format!("[\"{to_key}\"]\u{0020}=");
    let dest_hit = {
        let span = &content[acct_open..=acct_close];
        find_key_at_depth(span, &dest_pattern, 1).map(|rel| acct_open + rel)
    };
    if let Some(dest_start) = dest_hit {
        let after_dest = dest_start + dest_pattern.len();
        // A destination that exists but holds a scalar value must be an error,
        // not a mangled removal of the following entry's block.
        let dest_brace_start = skip_ws_to_brace(content.as_bytes(), after_dest).ok_or_else(|| {
            format!("Destination key \"{to_key}\" has a non-table value; refusing to overwrite.")
        })?;
        let dest_brace_end = find_matching_brace(&content, dest_brace_start)
            .ok_or("Unbalanced braces in destination block.")?;
        let line_start = content[..dest_start]
            .rfind('\n')
            .map(|p| p + 1)
            .unwrap_or(dest_start);
        let mut remove_end = dest_brace_end + 1;
        let rest = content.as_bytes();
        while remove_end < rest.len()
            && (rest[remove_end] == b','
                || rest[remove_end] == b' '
                || rest[remove_end] == b'\t')
        {
            remove_end += 1;
        }
        // Consume the trailing line break, handling CRLF (`\r\n`) so no stray
        // blank line is left behind on Windows-authored files.
        if remove_end < rest.len() && rest[remove_end] == b'\r' {
            remove_end += 1;
        }
        if remove_end < rest.len() && rest[remove_end] == b'\n' {
            remove_end += 1;
        }
        content = format!("{}{}", &content[..line_start], &content[remove_end..]);
    }

    // Re-search for the source key after potential removal (positions may have
    // shifted) and recompute its account-table span so the insertion also lands
    // inside that same table.
    let key_start = find_key_at_depth(&content, &search_pattern, 3)
        .ok_or_else(|| format!("Source key \"{from_key}\" not found after cleanup."))?;
    let acct_open = find_enclosing_brace(&content, key_start)
        .ok_or_else(|| format!("Source key \"{from_key}\" is not inside a table."))?;
    let acct_close = find_matching_brace(&content, acct_open)
        .ok_or("Unbalanced braces around the source's account table.")?;

    let after_pattern = key_start + search_pattern.len();
    // Require a table value: only whitespace may sit between `=` and `{`.
    let brace_start = skip_ws_to_brace(content.as_bytes(), after_pattern)
        .ok_or_else(|| format!("Source key \"{from_key}\" does not have a table value."))?;

    let brace_end =
        find_matching_brace(&content, brace_start).ok_or("Unbalanced braces in source block.")?;

    let value_block = &content[brace_start..=brace_end];

    // Advance past the source block's trailing comma/whitespace to find the
    // insertion point, tracking whether a comma was actually consumed.  Stay
    // within the account table's closing brace.
    let mut actual_insert = brace_end + 1;
    let mut comma_seen = false;
    let bytes = content.as_bytes();
    while actual_insert <= acct_close && actual_insert < bytes.len() {
        match bytes[actual_insert] {
            b',' => {
                comma_seen = true;
                actual_insert += 1;
            }
            b' ' | b'\t' => actual_insert += 1,
            _ => break,
        }
    }

    let line_start = content[..key_start].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let indent: String = content[line_start..key_start]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();

    // If the source block was the last entry with no trailing comma, prefix the
    // inserted text with a comma so the resulting Lua stays valid.
    let new_block = if comma_seen {
        format!("\n{indent}[\"{to_key}\"] = {value_block},")
    } else {
        format!(",\n{indent}[\"{to_key}\"] = {value_block}")
    };

    let mut result = String::with_capacity(content.len() + new_block.len());
    result.push_str(&content[..actual_insert]);
    result.push_str(&new_block);
    result.push_str(&content[actual_insert..]);

    // Validation: re-parse the generated Lua to ensure it's syntactically valid
    parser::parse_sv_file(&result, file_name)
        .map_err(|e| format!("Copy produced invalid Lua: {e}. Operation aborted."))?;

    // Overwrite protection: reject if the file changed while we were computing
    let pre_write_stamp = io::file_stamp(&file_path)?;
    if pre_write_stamp.modified_epoch_ms != read_stamp.modified_epoch_ms
        || pre_write_stamp.size != read_stamp.size
    {
        return Err("File was modified while the copy was being prepared. \
             Please try again."
            .to_string());
    }

    // Create a .bak copy immediately before writing — after all validation and
    // the stamp re-check — so a copy that aborts early never clobbers the
    // previous good backup.
    let bak_path = file_path.with_extension("lua.bak");
    fs::copy(&file_path, &bak_path)
        .map_err(|e| format!("Failed to create backup before copy: {e}"))?;

    io::write_raw_content(&sv_dir, file_name, &result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_key_at_depth_finds_depth_2_key() {
        let content = r#"MyAddon_SV =
{
    ["Default"] =
    {
        ["@User"] =
        {
            ["CharName^NA"] =
            {
            },
        },
    },
}"#;
        assert!(find_key_at_depth(content, "[\"@User\"]", 2).is_some());
    }

    #[test]
    fn find_key_at_depth_ignores_wrong_depth() {
        let content = r#"MyAddon_SV =
{
    ["Default"] =
    {
        ["@User"] =
        {
        },
    },
}"#;
        assert!(find_key_at_depth(content, "[\"@User\"]", 1).is_none());
    }

    // ---- copy_sv_profile_blocking integration tests -------------------------

    /// Build a tempdir laid out as `<tmp>/AddOns` + `<tmp>/SavedVariables`,
    /// write `content` to `Test.lua`, and return the addons dir and file path.
    fn setup(content: &str) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let addons_dir = tmp.path().join("AddOns");
        let sv_dir = tmp.path().join("SavedVariables");
        fs::create_dir_all(&addons_dir).unwrap();
        fs::create_dir_all(&sv_dir).unwrap();
        let file = sv_dir.join("Test.lua");
        fs::write(&file, content).unwrap();
        (tmp, addons_dir, file)
    }

    const BASE: &str = r#"Test_SV =
{
    ["Default"] =
    {
        ["@Acct"] =
        {
            ["$AccountWide"] =
            {
                ["v"] = 1,
            },
            ["Alpha"] =
            {
                ["setting"] = "a",
            },
            ["Beta"] =
            {
                ["setting"] = "b",
            },
        },
    },
}
"#;

    fn count(hay: &str, needle: &str) -> usize {
        hay.matches(needle).count()
    }

    #[test]
    fn basic_copy_to_new_key() {
        let (_tmp, addons, file) = setup(BASE);
        copy_sv_profile_blocking(&addons, "Test.lua", "Alpha", "Gamma").unwrap();
        let out = fs::read_to_string(&file).unwrap();
        // Valid Lua and both source and new key present.
        parser::parse_sv_file(&out, "Test.lua").unwrap();
        assert_eq!(count(&out, "[\"Alpha\"]"), 1);
        assert_eq!(count(&out, "[\"Gamma\"]"), 1);
        // The copy carries the source's value.
        let gamma = &out[out.find("[\"Gamma\"]").unwrap()..];
        assert!(gamma.contains("[\"setting\"] = \"a\""));
    }

    #[test]
    fn copy_overwriting_existing_dest() {
        let (_tmp, addons, file) = setup(BASE);
        copy_sv_profile_blocking(&addons, "Test.lua", "Alpha", "Beta").unwrap();
        let out = fs::read_to_string(&file).unwrap();
        parser::parse_sv_file(&out, "Test.lua").unwrap();
        // Beta still appears exactly once, now with Alpha's value.
        assert_eq!(count(&out, "[\"Beta\"]"), 1);
        let beta = &out[out.find("[\"Beta\"]").unwrap()..];
        assert!(beta.contains("[\"setting\"] = \"a\""));
        // No stray "b" survives (old Beta block removed).
        assert!(!out.contains("[\"setting\"] = \"b\""));
    }

    #[test]
    fn source_not_found_error() {
        let (_tmp, addons, file) = setup(BASE);
        let before = fs::read_to_string(&file).unwrap();
        let err = copy_sv_profile_blocking(&addons, "Test.lua", "Nope", "X").unwrap_err();
        assert!(err.contains("not found"), "unexpected error: {err}");
        // File untouched.
        assert_eq!(fs::read_to_string(&file).unwrap(), before);
    }

    #[test]
    fn from_equals_to_called_directly_is_safe() {
        // The command wrapper rejects from==to, but the blocking fn must not
        // corrupt the file if invoked directly with equal keys.
        let (_tmp, addons, file) = setup(BASE);
        let before = fs::read_to_string(&file).unwrap();
        let res = copy_sv_profile_blocking(&addons, "Test.lua", "Alpha", "Alpha");
        assert!(res.is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), before);
    }

    #[test]
    fn dest_before_source_position_shift() {
        // Alpha (dest) appears before Beta (source); removing Alpha shifts
        // Beta's position, which must be re-located correctly.
        let (_tmp, addons, file) = setup(BASE);
        copy_sv_profile_blocking(&addons, "Test.lua", "Beta", "Alpha").unwrap();
        let out = fs::read_to_string(&file).unwrap();
        parser::parse_sv_file(&out, "Test.lua").unwrap();
        assert_eq!(count(&out, "[\"Alpha\"]"), 1);
        assert_eq!(count(&out, "[\"Beta\"]"), 1);
        // Alpha now carries Beta's value.
        let alpha = &out[out.find("[\"Alpha\"]").unwrap()..];
        assert!(alpha.contains("[\"setting\"] = \"b\""));
    }

    #[test]
    fn values_with_braces_and_quotes_in_strings() {
        let content = r#"Test_SV =
{
    ["Default"] =
    {
        ["@Acct"] =
        {
            ["Alpha"] =
            {
                ["msg"] = "he said \"hi\" and {not a brace}",
                ["tbl"] =
                {
                    ["n"] = 1,
                },
            },
            ["Beta"] =
            {
                ["setting"] = "b",
            },
        },
    },
}
"#;
        let (_tmp, addons, file) = setup(content);
        copy_sv_profile_blocking(&addons, "Test.lua", "Alpha", "Gamma").unwrap();
        let out = fs::read_to_string(&file).unwrap();
        parser::parse_sv_file(&out, "Test.lua").unwrap();
        assert_eq!(count(&out, "[\"Gamma\"]"), 1);
        // The tricky string survived intact in the copy, and Beta is untouched.
        assert_eq!(count(&out, "{not a brace}"), 2);
        assert_eq!(count(&out, "[\"Beta\"]"), 1);
    }

    #[test]
    fn last_entry_without_trailing_comma() {
        // Beta is the last entry and has NO trailing comma.
        let content = r#"Test_SV =
{
    ["Default"] =
    {
        ["@Acct"] =
        {
            ["Alpha"] =
            {
                ["setting"] = "a",
            },
            ["Beta"] =
            {
                ["setting"] = "b",
            }
        },
    },
}
"#;
        let (_tmp, addons, file) = setup(content);
        copy_sv_profile_blocking(&addons, "Test.lua", "Beta", "Gamma").unwrap();
        let out = fs::read_to_string(&file).unwrap();
        // Result must be valid Lua despite the missing trailing comma.
        parser::parse_sv_file(&out, "Test.lua").unwrap();
        assert_eq!(count(&out, "[\"Gamma\"]"), 1);
        let gamma = &out[out.find("[\"Gamma\"]").unwrap()..];
        assert!(gamma.contains("[\"setting\"] = \"b\""));
    }

    #[test]
    fn scalar_valued_dest_errors_and_leaves_file_untouched() {
        // The destination key exists but holds a scalar, not a table.
        let content = r#"Test_SV =
{
    ["Default"] =
    {
        ["@Acct"] =
        {
            ["Alpha"] =
            {
                ["setting"] = "a",
            },
            ["Beta"] = 5,
        },
    },
}
"#;
        let (_tmp, addons, file) = setup(content);
        let before = fs::read_to_string(&file).unwrap();
        let err = copy_sv_profile_blocking(&addons, "Test.lua", "Alpha", "Beta").unwrap_err();
        assert!(
            err.contains("non-table value"),
            "unexpected error: {err}"
        );
        // Nothing was written; the scalar Beta is intact.
        assert_eq!(fs::read_to_string(&file).unwrap(), before);
    }

    #[test]
    fn dest_key_under_other_account_is_untouched() {
        // "Gamma" exists under @Acct2; the source "Alpha" lives under @Acct1.
        // The copy must land under @Acct1 and @Acct2's Gamma must be untouched.
        let content = r#"Test_SV =
{
    ["Default"] =
    {
        ["@Acct1"] =
        {
            ["Alpha"] =
            {
                ["setting"] = "a",
            },
        },
        ["@Acct2"] =
        {
            ["Gamma"] =
            {
                ["setting"] = "other",
            },
        },
    },
}
"#;
        let (_tmp, addons, file) = setup(content);
        copy_sv_profile_blocking(&addons, "Test.lua", "Alpha", "Gamma").unwrap();
        let out = fs::read_to_string(&file).unwrap();
        parser::parse_sv_file(&out, "Test.lua").unwrap();

        // Two Gamma keys now exist: the untouched one under @Acct2 and the new
        // copy under @Acct1.
        assert_eq!(count(&out, "[\"Gamma\"]"), 2);
        // @Acct2's Gamma value ("other") is preserved.
        assert!(out.contains("[\"setting\"] = \"other\""));

        // The new Gamma under @Acct1 sits before @Acct2 and carries Alpha's "a".
        let acct1 = out.find("[\"@Acct1\"]").unwrap();
        let acct2 = out.find("[\"@Acct2\"]").unwrap();
        let acct1_block = &out[acct1..acct2];
        assert!(acct1_block.contains("[\"Gamma\"]"));
        assert!(acct1_block.contains("[\"setting\"] = \"a\""));
    }
}
