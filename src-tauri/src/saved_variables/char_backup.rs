//! Server-scoped, per-character SavedVariables backup & restore via byte-exact
//! subtree surgery.
//!
//! The legacy character backup copied WHOLE SavedVariables files matched by name,
//! so a same-name NA/EU twin shared one backup and restoring one rolled back the
//! other server's (and the account-wide) data. This module instead extracts only
//! a single character's per-character subtree and merges just that subtree back
//! on restore, leaving every other character and all account-wide data untouched.
//!
//! Everything operates on raw bytes (never `String`), so non-UTF8 SavedVariables
//! content — caret keys, addon binary blobs — round-trips losslessly. The
//! navigation reuses the parser's byte primitives (`skip_lua_string`,
//! `skip_lua_comment`) and the same brace/string-aware scanning as
//! [`super::profile`], but every step is scoped to an explicit absolute
//! `[open..=close]` brace window so a key is only ever matched inside the exact
//! ancestor it belongs to (an NA `["Bob"]` is never confused with the EU one,
//! which sits at the same depth elsewhere in the file).
//!
//! ## Layouts handled (mirrors the roster's structural model)
//!
//! * account-keyed: `Default → @account → CharName` (no world; server not
//!   separable — isolation is by character name only)
//! * world-scoped:  `Default → "<World>" → @account → CharName`
//! * pChat world-first: `"<World>" → @account → CharName`
//!
//! When a target megaserver is given (the character's server is a known
//! megaserver), only world-scoped subtrees under THAT world are taken, so NA/EU
//! twins are isolated. Account-keyed subtrees are always taken (they carry no
//! world). When no megaserver is known (an `Unknown`-server recovered character),
//! world-scoped subtrees under ANY world are taken too, so such a character is
//! still backed up (it just can't be server-isolated).

use super::parser::{skip_lua_comment, skip_lua_string};
use super::scrub::WELL_KNOWN_WORLDS;

/// A single character's subtree located within one SavedVariables file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharBlock {
    /// Top-level addon variable name (e.g. `pChatData`).
    pub addon_var: Vec<u8>,
    /// Ancestor keys from the addon variable down to (but excluding) the
    /// character: `[Default, (World,) @account]`, each verbatim as on disk.
    pub path: Vec<Vec<u8>>,
    /// The verbatim character key on disk (`Bob` or e.g. `Bob^Mx`).
    pub char_key: Vec<u8>,
    /// The character's table value, exact bytes INCLUDING the surrounding braces.
    pub value: Vec<u8>,
}

impl CharBlock {
    /// The distinct world-layer key this block was captured under (e.g.
    /// `b"NA Megaserver"`), if any. `None` for an account-keyed block, which
    /// carries no world layer at all. Used to detect when an Unknown-server
    /// backup (which takes world-scoped subtrees from every megaserver, since
    /// it has no single world to filter to) has silently spanned more than one
    /// megaserver — i.e. bundled a same-named twin from another server.
    pub fn world_layer(&self) -> Option<&[u8]> {
        self.path
            .iter()
            .find(|k| is_world_layer(k.as_slice()))
            .map(|k| k.as_slice())
    }
}

#[inline]
fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

/// Strip a raw caret suffix (`Bob^Mx` -> `Bob`) to get the base name used for
/// matching a key on disk.
pub fn char_base(key: &[u8]) -> &[u8] {
    match key.iter().position(|&b| b == b'^') {
        Some(p) => &key[..p],
        None => key,
    }
}

#[inline]
fn is_world_layer(key: &[u8]) -> bool {
    WELL_KNOWN_WORLDS.iter().any(|w| w.as_bytes() == key) || key.contains(&b' ')
}

/// Advance past whitespace and Lua comments, returning the next significant
/// index (bounded by `end`).
fn skip_ws_comments(bytes: &[u8], mut i: usize, end: usize) -> usize {
    while i < end {
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'-' if bytes.get(i + 1) == Some(&b'-') => i = skip_lua_comment(bytes, i),
            _ => break,
        }
    }
    i
}

/// Find the matching `}` for the `{` at absolute index `open`, respecting
/// strings and comments. Returns the absolute index of the closing `}`.
pub fn matching_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
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

/// Whether the whole buffer's braces balance (and never underflow), skipping
/// strings and comments. A cheap, non-UTF8-safe structural sanity check.
fn braces_balanced(bytes: &[u8]) -> bool {
    let mut depth = 0i32;
    let mut i = 0;
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
                if depth < 0 {
                    return false;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    depth == 0
}

/// Read a `["..."]` key starting at absolute index `pos` (which must point at the
/// `[` of `["`). Returns the verbatim key bytes (escapes retained, exactly like
/// `parse_table_key`) and the absolute index of the closing `"`.
fn read_bracket_key(bytes: &[u8], pos: usize) -> Option<(Vec<u8>, usize)> {
    if bytes.get(pos) != Some(&b'[') || bytes.get(pos + 1) != Some(&b'"') {
        return None;
    }
    let start = pos + 2;
    let mut i = start;
    while i < bytes.len() && bytes[i] != b'"' {
        if bytes[i] == b'\\' {
            i += 1;
        }
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    Some((bytes[start..i].to_vec(), i))
}

/// Skip a scalar value starting at `j` (number/keyword/string/long-string) up to
/// and including the next top-level `,`, or to `end`. Used to step over non-table
/// entries while enumerating a table's children.
fn skip_scalar_value(bytes: &[u8], mut i: usize, end: usize) -> usize {
    while i < end {
        match bytes[i] {
            b'"' | b'\'' => i = skip_lua_string(bytes, i),
            b'[' if i + 1 < bytes.len() && (bytes[i + 1] == b'[' || bytes[i + 1] == b'=') => {
                i = skip_lua_string(bytes, i)
            }
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => i = skip_lua_comment(bytes, i),
            b',' => return i + 1,
            b'}' => return i,
            b'{' => match matching_brace(bytes, i) {
                Some(e) => i = e + 1,
                None => return end,
            },
            _ => i += 1,
        }
    }
    i
}

/// Visit each direct child of the table whose braces span `[open..=close]` that
/// is written as `["key"] = { ...table... }`, calling `f(key, value_open,
/// value_close)` with the verbatim key bytes and the absolute brace span of the
/// child's table value. Scalar-valued children and non-`["..."]` entries
/// (identifier/numeric keys, array elements) are skipped.
fn for_each_child_table<F: FnMut(&[u8], usize, usize)>(
    bytes: &[u8],
    open: usize,
    close: usize,
    mut f: F,
) {
    let mut i = open + 1;
    while i < close {
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' | b',' => i += 1,
            b'-' if bytes.get(i + 1) == Some(&b'-') => i = skip_lua_comment(bytes, i),
            b'"' | b'\'' => i = skip_lua_string(bytes, i), // array string element
            b'{' => match matching_brace(bytes, i) {
                Some(e) => i = e + 1,
                None => break,
            },
            b'[' if bytes.get(i + 1) == Some(&b'[') || bytes.get(i + 1) == Some(&b'=') => {
                i = skip_lua_string(bytes, i) // long-string array element
            }
            b'[' if bytes.get(i + 1) == Some(&b'"') => {
                if let Some((key, quote_end)) = read_bracket_key(bytes, i) {
                    let rb = skip_ws_comments(bytes, quote_end + 1, close);
                    if bytes.get(rb) == Some(&b']') {
                        let eq = skip_ws_comments(bytes, rb + 1, close);
                        if bytes.get(eq) == Some(&b'=') {
                            let v = skip_ws_comments(bytes, eq + 1, close);
                            if v < close && bytes[v] == b'{' {
                                if let Some(vend) = matching_brace(bytes, v) {
                                    f(&key, v, vend);
                                    i = vend + 1;
                                    continue;
                                }
                            }
                            // Scalar value — skip it.
                            i = skip_scalar_value(bytes, v, close);
                            continue;
                        }
                    }
                }
                // Not a parseable key=value; step past the key string.
                i = skip_lua_string(bytes, i + 1);
            }
            _ => i += 1,
        }
    }
}

/// Visit each top-level `identifier = { ...table... }` addon variable, calling
/// `f(name, value_open, value_close)`. Mirrors `parse_sv_file`'s top-level
/// recognition (only identifier-keyed table assignments).
fn for_each_top_level_var<F: FnMut(&[u8], usize, usize)>(bytes: &[u8], mut f: F) {
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        i = skip_ws_comments(bytes, i, n);
        if i >= n {
            break;
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let name = bytes[start..i].to_vec();
            let eq = skip_ws_comments(bytes, i, n);
            if bytes.get(eq) == Some(&b'=') {
                let v = skip_ws_comments(bytes, eq + 1, n);
                if v < n && bytes[v] == b'{' {
                    match matching_brace(bytes, v) {
                        Some(end) => {
                            f(&name, v, end);
                            i = end + 1;
                            continue;
                        }
                        None => break, // unbalanced
                    }
                } else {
                    i = skip_scalar_value(bytes, v, n);
                    continue;
                }
            }
            i = eq;
        } else {
            i += 1;
        }
    }
}

/// Within the account table `[open..=close]`, find the character whose base name
/// is `base` (matching either the exact `["base"]` key or a raw caret form
/// `["base^..."]`). Returns the verbatim key and the absolute brace span of its
/// table value.
fn find_char_value(
    bytes: &[u8],
    open: usize,
    close: usize,
    base: &[u8],
) -> Option<(Vec<u8>, usize, usize)> {
    let mut found: Option<(Vec<u8>, usize, usize)> = None;
    for_each_child_table(bytes, open, close, |key, vo, vc| {
        if found.is_some() {
            return;
        }
        if char_base(key) == base {
            found = Some((key.to_vec(), vo, vc));
        }
    });
    found
}

/// Find a direct child table by EXACT key within `[open..=close]`, returning its
/// value brace span.
fn find_child_by_key(
    bytes: &[u8],
    open: usize,
    close: usize,
    key: &[u8],
) -> Option<(usize, usize)> {
    let mut found: Option<(usize, usize)> = None;
    for_each_child_table(bytes, open, close, |k, vo, vc| {
        if found.is_none() && k == key {
            found = Some((vo, vc));
        }
    });
    found
}

/// Whether `world_key` should be taken given the target megaserver `target`:
/// `None` (Unknown server) matches any world; a known megaserver matches only its
/// own world layer.
fn world_matches(world_key: &[u8], target: Option<&str>) -> bool {
    match target {
        None => true,
        Some(t) => world_key == t.as_bytes(),
    }
}

/// Extract every per-character subtree for `char_base_name` from `bytes`, scoped
/// by `world` (see module docs). `char_base_name` is the caret-stripped name.
pub fn extract_character_blocks(
    bytes: &[u8],
    char_base_name: &[u8],
    world: Option<&str>,
) -> Vec<CharBlock> {
    let mut out: Vec<CharBlock> = Vec::new();

    for_each_top_level_var(bytes, |addon_var, av_open, av_close| {
        for_each_child_table(bytes, av_open, av_close, |k1, vo1, vc1| {
            if k1 == b"Default" {
                for_each_child_table(bytes, vo1, vc1, |k2, vo2, vc2| {
                    if k2.first() == Some(&b'@') {
                        // Default -> @account -> Char  (account-keyed; always taken)
                        collect(
                            bytes,
                            addon_var,
                            &[b"Default".to_vec(), k2.to_vec()],
                            vo2,
                            vc2,
                            char_base_name,
                            &mut out,
                        );
                    } else if is_world_layer(k2) && world_matches(k2, world) {
                        // Default -> World -> @account -> Char
                        for_each_child_table(bytes, vo2, vc2, |k3, vo3, vc3| {
                            if k3.first() == Some(&b'@') {
                                collect(
                                    bytes,
                                    addon_var,
                                    &[b"Default".to_vec(), k2.to_vec(), k3.to_vec()],
                                    vo3,
                                    vc3,
                                    char_base_name,
                                    &mut out,
                                );
                            }
                        });
                    }
                });
            } else if is_world_layer(k1) && world_matches(k1, world) {
                // pChat world-first: World -> @account -> Char
                for_each_child_table(bytes, vo1, vc1, |k2, vo2, vc2| {
                    if k2.first() == Some(&b'@') {
                        collect(
                            bytes,
                            addon_var,
                            &[k1.to_vec(), k2.to_vec()],
                            vo2,
                            vc2,
                            char_base_name,
                            &mut out,
                        );
                    }
                });
            }
            // An `@account` directly under the addon variable holds addon section
            // keys, not characters — skipped, exactly like the roster.
        });
    });

    out
}

#[allow(clippy::too_many_arguments)]
fn collect(
    bytes: &[u8],
    addon_var: &[u8],
    path: &[Vec<u8>],
    acc_open: usize,
    acc_close: usize,
    base: &[u8],
    out: &mut Vec<CharBlock>,
) {
    if let Some((char_key, vo, vc)) = find_char_value(bytes, acc_open, acc_close, base) {
        let block = CharBlock {
            addon_var: addon_var.to_vec(),
            path: path.to_vec(),
            char_key,
            value: bytes[vo..=vc].to_vec(),
        };
        if !out.contains(&block) {
            out.push(block);
        }
    }
}

/// Build a `["key"] = <value>,` entry with one level of indentation.
fn char_entry(indent: &[u8], char_key: &[u8], value: &[u8]) -> Vec<u8> {
    let mut e = Vec::new();
    e.extend_from_slice(indent);
    e.extend_from_slice(b"[\"");
    e.extend_from_slice(char_key);
    e.extend_from_slice(b"\"] = ");
    e.extend_from_slice(value);
    e.push(b',');
    e
}

/// Build the nested `["p0"] = { ["p1"] = { ... ["char"] = value, } }` structure
/// for the remaining `path`, innermost first, at the given starting indent depth.
fn build_nested(path: &[Vec<u8>], char_key: &[u8], value: &[u8], depth: usize) -> Vec<u8> {
    let indent = vec![b'\t'; depth + 1];
    if path.is_empty() {
        let mut out = char_entry(&indent, char_key, value);
        out.push(b'\n');
        return out;
    }
    let inner = build_nested(&path[1..], char_key, value, depth + 1);
    let mut out = Vec::new();
    out.extend_from_slice(&indent);
    out.extend_from_slice(b"[\"");
    out.extend_from_slice(&path[0]);
    out.extend_from_slice(b"\"] =\n");
    out.extend_from_slice(&indent);
    out.extend_from_slice(b"{\n");
    out.extend_from_slice(&inner);
    out.extend_from_slice(&indent);
    out.extend_from_slice(b"},\n");
    out
}

/// Build a complete minimal `<addon_var> = { <path> { char = value } }` file for
/// one block (used when the addon variable is absent in the target buffer).
fn build_var_snippet(block: &CharBlock) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&block.addon_var);
    out.extend_from_slice(b" =\n{\n");
    out.extend_from_slice(&build_nested(&block.path, &block.char_key, &block.value, 0));
    out.extend_from_slice(b"}\n");
    out
}

/// Insert `entry` immediately after the `{` at `brace_open`, preserving every
/// existing byte (siblings stay byte-identical; only their offset shifts).
fn insert_after_brace(live: &[u8], brace_open: usize, entry: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(live.len() + entry.len() + 1);
    buf.extend_from_slice(&live[..=brace_open]);
    buf.push(b'\n');
    buf.extend_from_slice(entry);
    buf.extend_from_slice(&live[brace_open + 1..]);
    buf
}

/// Merge one character subtree into `live`, returning the new file bytes. Other
/// characters and all account-wide data are left byte-identical: the merge is a
/// single splice that either REPLACES the character's existing value block or
/// INSERTS the (possibly partial) path + character. The result is validated
/// structurally before being returned.
pub fn merge_character_block(live: &[u8], block: &CharBlock) -> Result<Vec<u8>, String> {
    let buf = merge_raw(live, block);
    validate_merge(&buf, block)?;
    Ok(buf)
}

fn merge_raw(live: &[u8], block: &CharBlock) -> Vec<u8> {
    // Locate the addon variable's table.
    let mut av: Option<(usize, usize)> = None;
    for_each_top_level_var(live, |name, open, close| {
        if av.is_none() && name == block.addon_var.as_slice() {
            av = Some((open, close));
        }
    });

    let Some((av_open, av_close)) = av else {
        // Addon variable absent: append a standalone snippet (or be the whole file
        // when the target is blank).
        let snippet = build_var_snippet(block);
        if live.iter().all(|&b| is_ws(b)) {
            return snippet;
        }
        let mut buf = live.to_vec();
        if !buf.ends_with(b"\n") {
            buf.push(b'\n');
        }
        buf.extend_from_slice(&snippet);
        return buf;
    };

    // Descend the path as far as it already exists.
    let mut cur = (av_open, av_close);
    let mut idx = 0;
    while idx < block.path.len() {
        match find_child_by_key(live, cur.0, cur.1, &block.path[idx]) {
            Some(span) => {
                cur = span;
                idx += 1;
            }
            None => break,
        }
    }

    if idx == block.path.len() {
        // Full path exists; act on the character within the account table `cur`.
        // Match the EXACT backed-up key (e.g. `Bob^Mx`) — never a different
        // same-base sibling — so a replace can't splice into the wrong key.
        if let Some((vo, vc)) = find_child_by_key(live, cur.0, cur.1, &block.char_key) {
            // Replace the character's value block in place.
            let mut buf = Vec::with_capacity(live.len() + block.value.len());
            buf.extend_from_slice(&live[..vo]);
            buf.extend_from_slice(&block.value);
            buf.extend_from_slice(&live[vc + 1..]);
            buf
        } else {
            // Insert the character entry into the account table.
            let depth = brace_indent_depth(live, cur.0);
            let indent = vec![b'\t'; depth + 1];
            let entry = char_entry(&indent, &block.char_key, &block.value);
            insert_after_brace(live, cur.0, &entry)
        }
    } else {
        // Insert the missing remaining path + character into the deepest existing
        // ancestor table `cur`.
        let depth = brace_indent_depth(live, cur.0);
        let nested = build_nested(&block.path[idx..], &block.char_key, &block.value, depth);
        // build_nested already indents from `depth+1`; drop its trailing newline's
        // duplicate by inserting as-is (insert_after_brace adds the leading \n).
        let trimmed = nested
            .strip_suffix(b"\n")
            .map(|s| s.to_vec())
            .unwrap_or(nested);
        insert_after_brace(live, cur.0, &trimmed)
    }
}

/// Rough indentation depth for content inside the brace at `brace_open`, derived
/// from the leading tabs/spaces on that brace's line. Cosmetic only.
fn brace_indent_depth(bytes: &[u8], brace_open: usize) -> usize {
    let line_start = bytes[..brace_open]
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|p| p + 1)
        .unwrap_or(0);
    bytes[line_start..brace_open]
        .iter()
        .take_while(|&&b| b == b'\t')
        .count()
}

/// Validate a merged buffer: braces must balance, the character subtree must
/// round-trip byte-exact at its path, and (when the buffer is valid UTF-8) the
/// whole file must re-parse as Lua.
fn validate_merge(buf: &[u8], block: &CharBlock) -> Result<(), String> {
    if !braces_balanced(buf) {
        return Err("merge produced unbalanced braces".to_string());
    }
    let blocks = extract_character_blocks(buf, char_base(&block.char_key), None);
    let round_trips = blocks.iter().any(|b| {
        b.addon_var == block.addon_var
            && b.path == block.path
            && b.char_key == block.char_key
            && b.value == block.value
    });
    if !round_trips {
        return Err("merge did not round-trip the character subtree".to_string());
    }
    // Full re-parse as a final structural confirmation — but only up to the
    // editor's 20 MB cap. `parse_sv_file` builds a complete `SvTreeNode` tree
    // (roughly 10x the source size), so on a large merged buffer (e.g. splicing
    // into a 200 MB live SavedVariables file) that transient allocation would
    // dwarf the merge itself, once per merged block. Above the cap we rely on the
    // two structural validators already run above — balanced braces plus a
    // byte-exact round-trip extraction of the spliced subtree — which together
    // catch a malformed splice. The full parse is a belt-and-braces check that
    // only pays for itself on files small enough to open in the editor anyway.
    const MAX_PARSE_VALIDATE_BYTES: usize = 20 * 1024 * 1024; // matches io.rs editor cap
    if buf.len() <= MAX_PARSE_VALIDATE_BYTES {
        if let Ok(s) = std::str::from_utf8(buf) {
            super::parser::parse_sv_file(s, "merged.lua")
                .map_err(|e| format!("merge produced invalid Lua: {e}"))?;
        }
    }
    Ok(())
}

/// Build a self-contained backup file holding every block found in one source
/// SavedVariables file, by merging each block into an initially empty buffer.
pub fn build_backup_file(blocks: &[CharBlock]) -> Result<Vec<u8>, String> {
    let mut buf: Vec<u8> = Vec::new();
    for block in blocks {
        buf = merge_character_block(&buf, block)?;
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn na_eu_acct_file() -> Vec<u8> {
        // NA Bob + EU Bob under world layers, plus account-wide data under the
        // account-keyed layout — the round-trip acceptance fixture.
        concat!(
            "TestAddon =\n{\n",
            "\t[\"Default\"] =\n\t{\n",
            "\t\t[\"NA Megaserver\"] =\n\t\t{\n",
            "\t\t\t[\"@me\"] =\n\t\t\t{\n",
            "\t\t\t\t[\"Bob\"] = { [\"hp\"] = 100, [\"loc\"] = \"NA\" },\n",
            "\t\t\t},\n",
            "\t\t},\n",
            "\t\t[\"EU Megaserver\"] =\n\t\t{\n",
            "\t\t\t[\"@me\"] =\n\t\t\t{\n",
            "\t\t\t\t[\"Bob\"] = { [\"hp\"] = 200, [\"loc\"] = \"EU\" },\n",
            "\t\t\t},\n",
            "\t\t},\n",
            "\t\t[\"@me\"] =\n\t\t{\n",
            "\t\t\t[\"$AccountWide\"] = { [\"gold\"] = 5000 },\n",
            "\t\t},\n",
            "\t},\n}\n"
        )
        .as_bytes()
        .to_vec()
    }

    fn slice<'a>(bytes: &'a [u8], needle: &str) -> &'a [u8] {
        // Return the value-block bytes of the FIRST `["needle"] = { ... }`.
        let pat = format!("[\"{needle}\"] = {{");
        let pos = bytes
            .windows(pat.len())
            .position(|w| w == pat.as_bytes())
            .unwrap_or_else(|| panic!("{needle} not found"));
        let brace = pos + pat.len() - 1;
        let end = matching_brace(bytes, brace).unwrap();
        &bytes[brace..=end]
    }

    #[test]
    fn extract_isolates_na_twin() {
        let file = na_eu_acct_file();
        let blocks = extract_character_blocks(&file, b"Bob", Some("NA Megaserver"));
        assert_eq!(blocks.len(), 1, "only the NA Bob subtree");
        let b = &blocks[0];
        assert_eq!(b.addon_var, b"TestAddon");
        assert_eq!(
            b.path,
            vec![
                b"Default".to_vec(),
                b"NA Megaserver".to_vec(),
                b"@me".to_vec()
            ]
        );
        assert_eq!(b.char_key, b"Bob");
        assert!(b.value.windows(2).any(|w| w == b"NA"));
        assert!(
            !b.value.windows(2).any(|w| w == b"EU"),
            "NA backup must not contain EU data"
        );
    }

    #[test]
    fn build_backup_file_contains_only_target_twin() {
        let file = na_eu_acct_file();
        let blocks = extract_character_blocks(&file, b"Bob", Some("NA Megaserver"));
        let backup = build_backup_file(&blocks).unwrap();
        // The stored backup holds NA Bob but NOT EU Bob's data or $AccountWide.
        assert!(backup.windows(4).any(|w| w == b"\"NA\""));
        assert!(!backup.windows(4).any(|w| w == b"\"EU\""));
        assert!(!backup.windows(12).any(|w| w == b"$AccountWide"));
        // And it is valid, re-extractable Lua.
        let re = extract_character_blocks(&backup, b"Bob", None);
        assert_eq!(re.len(), 1);
        assert_eq!(re[0].value, blocks[0].value);
    }

    #[test]
    fn roundtrip_restore_only_touches_target() {
        let original = na_eu_acct_file();
        // Back up NA Bob.
        let blocks = extract_character_blocks(&original, b"Bob", Some("NA Megaserver"));
        let backup = build_backup_file(&blocks).unwrap();

        // Mutate the live file: change NA Bob, EU Bob, and $AccountWide.
        let mutated = String::from_utf8(original.clone())
            .unwrap()
            .replace("[\"hp\"] = 100", "[\"hp\"] = 1")
            .replace("[\"hp\"] = 200", "[\"hp\"] = 2")
            .replace("[\"gold\"] = 5000", "[\"gold\"] = 1")
            .into_bytes();

        // Capture $AccountWide as it is AFTER mutation (restore must not touch it).
        let acct_mutated = slice(&mutated, "$AccountWide").to_vec();

        // Restore NA Bob: extract from the backup (permissive) and merge each.
        let restore_blocks = extract_character_blocks(&backup, b"Bob", None);
        let mut live = mutated.clone();
        for b in &restore_blocks {
            live = merge_character_block(&live, b).unwrap();
        }

        // NA Bob is back to the original value...
        let na_now = slice(&live, "Bob");
        assert!(na_now.windows(3).any(|w| w == b"100"), "NA Bob restored");
        // ...EU Bob stays mutated (hp=2)...
        let live_str = String::from_utf8(live.clone()).unwrap();
        assert!(live_str.contains("[\"hp\"] = 2"), "EU Bob untouched");
        assert!(!live_str.contains("[\"hp\"] = 200"), "EU Bob not reverted");
        // ...and $AccountWide stays mutated, byte-identical.
        assert_eq!(slice(&live, "$AccountWide"), acct_mutated.as_slice());
    }

    #[test]
    fn restore_into_missing_character_recreates_subtree() {
        let original = na_eu_acct_file();
        let blocks = extract_character_blocks(&original, b"Bob", Some("NA Megaserver"));
        let backup = build_backup_file(&blocks).unwrap();

        // Live file has the path but NO Bob under NA (account table empty-ish).
        let live = concat!(
            "TestAddon =\n{\n",
            "\t[\"Default\"] =\n\t{\n",
            "\t\t[\"NA Megaserver\"] =\n\t\t{\n",
            "\t\t\t[\"@me\"] =\n\t\t\t{\n",
            "\t\t\t\t[\"$AccountWide\"] = { [\"x\"] = 1 },\n",
            "\t\t\t},\n",
            "\t\t},\n",
            "\t},\n}\n"
        )
        .as_bytes()
        .to_vec();

        let restore_blocks = extract_character_blocks(&backup, b"Bob", None);
        let mut merged = live.clone();
        for b in &restore_blocks {
            merged = merge_character_block(&merged, b).unwrap();
        }
        // Bob now exists under NA -> @me, with the backed-up data, and the
        // account-wide sibling is preserved.
        let re = extract_character_blocks(&merged, b"Bob", Some("NA Megaserver"));
        assert_eq!(re.len(), 1);
        assert_eq!(re[0].value, blocks[0].value);
        assert!(String::from_utf8(merged).unwrap().contains("$AccountWide"));
    }

    #[test]
    fn restore_into_missing_file_creates_just_the_subtree() {
        let original = na_eu_acct_file();
        let blocks = extract_character_blocks(&original, b"Bob", Some("NA Megaserver"));
        let backup = build_backup_file(&blocks).unwrap();

        // Empty live "file" (e.g. it didn't exist).
        let restore_blocks = extract_character_blocks(&backup, b"Bob", None);
        let mut merged: Vec<u8> = Vec::new();
        for b in &restore_blocks {
            merged = merge_character_block(&merged, b).unwrap();
        }
        let re = extract_character_blocks(&merged, b"Bob", Some("NA Megaserver"));
        assert_eq!(re.len(), 1);
        assert_eq!(re[0].path, blocks[0].path);
        assert_eq!(re[0].value, blocks[0].value);
        // Nothing else leaked in.
        assert!(!String::from_utf8(merged).unwrap().contains("EU"));
    }

    #[test]
    fn account_keyed_isolates_by_character() {
        // No world layer: Default -> @account -> {Bob, Alice}. Backing up Bob
        // takes only Bob's subtree (server can't be separated, but other chars
        // and account-wide data are untouched).
        let file = concat!(
            "Addon =\n{\n",
            "\t[\"Default\"] =\n\t{\n",
            "\t\t[\"@me\"] =\n\t\t{\n",
            "\t\t\t[\"$AccountWide\"] = { [\"g\"] = 1 },\n",
            "\t\t\t[\"Bob\"] = { [\"hp\"] = 100 },\n",
            "\t\t\t[\"Alice\"] = { [\"hp\"] = 50 },\n",
            "\t\t},\n",
            "\t},\n}\n"
        )
        .as_bytes()
        .to_vec();
        let blocks = extract_character_blocks(&file, b"Bob", None);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].path, vec![b"Default".to_vec(), b"@me".to_vec()]);
        assert!(blocks[0].value.windows(3).any(|w| w == b"100"));
        assert!(!blocks[0].value.windows(2).any(|w| w == b"50"));
    }

    #[test]
    fn caret_key_extracted_and_merged() {
        let file = concat!(
            "Addon =\n{\n",
            "\t[\"Default\"] =\n\t{\n",
            "\t\t[\"@me\"] =\n\t\t{\n",
            "\t\t\t[\"Bob^Mx\"] = { [\"hp\"] = 100 },\n",
            "\t\t},\n",
            "\t},\n}\n"
        )
        .as_bytes()
        .to_vec();
        let blocks = extract_character_blocks(&file, b"Bob", None);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].char_key, b"Bob^Mx");

        let backup = build_backup_file(&blocks).unwrap();
        let re = extract_character_blocks(&backup, b"Bob", None);
        assert_eq!(re[0].char_key, b"Bob^Mx");
        assert_eq!(re[0].value, blocks[0].value);
    }

    #[test]
    fn non_utf8_value_round_trips() {
        // A character value containing invalid UTF-8 bytes must round-trip.
        let mut file: Vec<u8> = concat!(
            "Addon =\n{\n",
            "\t[\"Default\"] =\n\t{\n",
            "\t\t[\"@me\"] =\n\t\t{\n",
            "\t\t\t[\"Bob\"] = { [\"icon\"] = \""
        )
        .as_bytes()
        .to_vec();
        file.extend_from_slice(&[0xff, 0xfe, 0x00, 0x80]);
        file.extend_from_slice(b"\" },\n\t\t},\n\t},\n}\n");

        let blocks = extract_character_blocks(&file, b"Bob", None);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].value.contains(&0xff));

        let backup = build_backup_file(&blocks).unwrap();
        let re = extract_character_blocks(&backup, b"Bob", None);
        assert_eq!(re[0].value, blocks[0].value, "non-UTF8 value preserved");
    }

    #[test]
    fn unknown_server_takes_world_scoped_data() {
        // An Unknown-server character whose only data is world-scoped is still
        // backed up (world = None matches any world layer).
        let file = na_eu_acct_file();
        let blocks = extract_character_blocks(&file, b"Bob", None);
        // Both NA and EU Bob (plus none account-keyed) — Unknown can't isolate.
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn other_addon_vars_left_untouched_on_merge() {
        // Two addon vars in the live file; merging Bob into one must not disturb
        // the other's bytes.
        let live = concat!(
            "AddonA =\n{\n\t[\"Default\"] =\n\t{\n\t\t[\"@me\"] =\n\t\t{\n",
            "\t\t\t[\"Bob\"] = { [\"hp\"] = 1 },\n\t\t},\n\t},\n}\n",
            "AddonB =\n{\n\t[\"keep\"] = { [\"untouched\"] = true },\n}\n"
        )
        .as_bytes()
        .to_vec();
        let block = CharBlock {
            addon_var: b"AddonA".to_vec(),
            path: vec![b"Default".to_vec(), b"@me".to_vec()],
            char_key: b"Bob".to_vec(),
            value: b"{ [\"hp\"] = 999 }".to_vec(),
        };
        let merged = merge_character_block(&live, &block).unwrap();
        let s = String::from_utf8(merged).unwrap();
        assert!(s.contains("[\"hp\"] = 999"), "Bob replaced");
        assert!(
            s.contains("AddonB =\n{\n\t[\"keep\"] = { [\"untouched\"] = true },\n}"),
            "AddonB byte-identical"
        );
    }
}
