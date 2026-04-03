use super::io;
use super::parser::{skip_lua_comment, skip_lua_string};
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

/// Find the next occurrence of a byte outside string literals.
pub fn find_next_outside_strings(content: &str, start: usize, target: u8) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => i = skip_lua_string(bytes, i),
            b'[' if i + 1 < bytes.len() && (bytes[i + 1] == b'[' || bytes[i + 1] == b'=') => {
                i = skip_lua_string(bytes, i)
            }
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => i = skip_lua_comment(bytes, i),
            b if b == target => return Some(i),
            _ => i += 1,
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

/// Copy a character profile within a SavedVariables file.
pub fn copy_sv_profile_blocking(
    addons_dir: &Path,
    file_name: &str,
    from_key: &str,
    to_key: &str,
) -> Result<(), String> {
    let sv_dir = io::saved_variables_dir(addons_dir);
    let file_path = sv_dir.join(file_name);

    if !file_path.is_file() {
        return Err(format!("File not found: {}", file_name));
    }

    let content =
        fs::read_to_string(&file_path).map_err(|e| format!("Failed to read file: {}", e))?;

    // Create a .bak copy before modifying the file
    let bak_path = file_path.with_extension("lua.bak");
    fs::copy(&file_path, &bak_path)
        .map_err(|e| format!("Failed to create backup before copy: {}", e))?;

    // Find the source key at depth 3 (character keys in ESO SV files).
    let search_pattern = format!("[\"{}\"]\u{0020}=", from_key);
    let _key_start = find_key_at_depth(&content, &search_pattern, 3)
        .ok_or_else(|| format!("Source key \"{}\" not found at expected depth.", from_key))?;

    // If to_key already exists at the same depth, remove the old block first
    let dest_pattern = format!("[\"{}\"]\u{0020}=", to_key);
    let mut content = content;
    if let Some(dest_start) = find_key_at_depth(&content, &dest_pattern, 3) {
        let after_dest = dest_start + dest_pattern.len();
        if let Some(dest_brace_start) = find_next_outside_strings(&content, after_dest, b'{') {
            if let Some(dest_brace_end) = find_matching_brace(&content, dest_brace_start) {
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
                if remove_end < rest.len() && rest[remove_end] == b'\n' {
                    remove_end += 1;
                }
                content = format!("{}{}", &content[..line_start], &content[remove_end..]);
            }
        }
    }

    // Re-search for source key after potential removal (positions may have shifted)
    let key_start = find_key_at_depth(&content, &search_pattern, 3)
        .ok_or_else(|| format!("Source key \"{}\" not found after cleanup.", from_key))?;

    let after_pattern = key_start + search_pattern.len();
    let brace_start = find_next_outside_strings(&content, after_pattern, b'{')
        .ok_or("Could not find opening brace for source key.")?;

    let brace_end =
        find_matching_brace(&content, brace_start).ok_or("Unbalanced braces in source block.")?;

    let value_block = &content[brace_start..=brace_end];

    let insert_pos = brace_end + 1;
    let mut actual_insert = insert_pos;
    let rest = &content.as_bytes()[actual_insert..];
    for &b in rest {
        if b == b',' || b == b' ' || b == b'\t' {
            actual_insert += 1;
        } else {
            break;
        }
    }

    let line_start = content[..key_start].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let indent: String = content[line_start..key_start]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();

    let new_block = format!("\n{}[\"{}\"] = {},", indent, to_key, value_block);

    let mut result = String::with_capacity(content.len() + new_block.len());
    result.push_str(&content[..actual_insert]);
    result.push_str(&new_block);
    result.push_str(&content[actual_insert..]);

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
}
