//! Auto-learn SavedVariables dropdown choices by scanning an addon's Lua source
//! for LibAddonMenu (LAM) dropdown definitions.
//!
//! Given the SV file stem (`sv_name`), resolve the owning addon folder(s) via
//! their manifests, then scan each `*.lua` file for `type = "dropdown"` control
//! blocks with literal `choices = {...}` (and optional `choicesValues = {...}`),
//! keying each hint to the SV setting name extracted from the `setFunc`/`getFunc`
//! closure. Purely literal — no Lua evaluation. An empty response is graceful
//! degradation, never an error the UI must surface.

use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const MAX_BLOCK: usize = 16 * 1024;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_FILES: usize = 400;

/// A single label/value pair learned from a LAM dropdown's choices list.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LamDropdownChoice {
    pub label: String,
    /// string | number | boolean, mirroring the Lua literal.
    pub value: Value,
}

/// The learned choices for one SV setting key.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LamDropdownHint {
    pub setting_key: String,
    pub choices: Vec<LamDropdownChoice>,
}

/// Response for a single scan. An empty response means "nothing learned" and is
/// returned instead of an error whenever resolution or parsing yields nothing.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LamScanResponse {
    pub hints: Vec<LamDropdownHint>,
    pub scanned_files: u32,
    pub matched_folders: Vec<String>,
}

// ── Cache ───────────────────────────────────────────────────────────────

#[derive(Clone)]
struct LamCacheEntry {
    response: LamScanResponse,
    /// (manifest path, len, mtime secs) for each matched manifest. Addon
    /// installs/updates rewrite manifests, so re-stat'ing them invalidates.
    fingerprint: Vec<(String, u64, u64)>,
}

static LAM_CACHE: OnceLock<Mutex<HashMap<String, LamCacheEntry>>> = OnceLock::new();

fn lam_cache() -> &'static Mutex<HashMap<String, LamCacheEntry>> {
    LAM_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

// ── Public entry ────────────────────────────────────────────────────────

pub fn scan_lam_dropdowns_blocking(
    addons_dir: &Path,
    sv_name: &str,
) -> Result<LamScanResponse, String> {
    let matched = resolve_folders(addons_dir, sv_name);
    if matched.is_empty() {
        return Ok(LamScanResponse::default());
    }

    let fingerprint: Vec<(String, u64, u64)> = matched
        .iter()
        .map(|m| {
            let (len, mtime) = fs_stat(&m.manifest);
            (m.manifest.to_string_lossy().to_string(), len, mtime)
        })
        .collect();

    let cache_key = format!("{}|{}", addons_dir.display(), sv_name);
    if let Some(cached) = lam_cache()
        .lock()
        .ok()
        .and_then(|c| c.get(&cache_key).cloned())
    {
        if cached.fingerprint == fingerprint {
            return Ok(cached.response);
        }
    }

    let response = scan_matched(&matched);

    if let Ok(mut cache) = lam_cache().lock() {
        cache.insert(
            cache_key,
            LamCacheEntry {
                response: response.clone(),
                fingerprint,
            },
        );
    }

    Ok(response)
}

fn fs_stat(path: &Path) -> (u64, u64) {
    let meta = std::fs::metadata(path).ok();
    let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (len, mtime)
}

// ── Folder resolution ───────────────────────────────────────────────────

struct MatchedFolder {
    path: PathBuf,
    name: String,
    manifest: PathBuf,
}

/// An addon folder discovered in the walk (top level + up to 2 sublevels).
struct AddonFolder {
    path: PathBuf,
    name: String,
    manifest: PathBuf,
    declared_svs: Vec<String>,
}

const SV_SUFFIXES: &[&str] = &["SavedVariables", "SavedVars", "SV", "Vars", "Data", "Settings"];

fn resolve_folders(addons_dir: &Path, sv_name: &str) -> Vec<MatchedFolder> {
    let mut addons: Vec<AddonFolder> = Vec::new();
    collect_addon_folders(addons_dir, &mut addons, 0);

    // Pass 1: manifest declares the exact name (case-sensitive).
    let mut matched = pick(&addons, |n| n == sv_name);
    // Pass 2: case-insensitive manifest match.
    if matched.is_empty() {
        matched = pick(&addons, |n| n.eq_ignore_ascii_case(sv_name));
    }
    // Fallback: folder-name heuristics.
    if matched.is_empty() {
        let sv_lower = sv_name.to_ascii_lowercase();
        matched = addons
            .iter()
            .filter(|a| folder_matches(&a.name, sv_name, &sv_lower))
            .map(|a| MatchedFolder {
                path: a.path.clone(),
                name: a.name.clone(),
                manifest: a.manifest.clone(),
            })
            .collect();
    }
    matched
}

fn pick(addons: &[AddonFolder], pred: impl Fn(&str) -> bool) -> Vec<MatchedFolder> {
    addons
        .iter()
        .filter(|a| a.declared_svs.iter().any(|n| pred(n)))
        .map(|a| MatchedFolder {
            path: a.path.clone(),
            name: a.name.clone(),
            manifest: a.manifest.clone(),
        })
        .collect()
}

fn folder_matches(folder: &str, sv_name: &str, sv_lower: &str) -> bool {
    if folder.eq_ignore_ascii_case(sv_name) {
        return true;
    }
    for suffix in SV_SUFFIXES {
        let suf_lower = suffix.to_ascii_lowercase();
        if sv_lower.len() > suf_lower.len() && sv_lower.ends_with(&suf_lower) {
            let stem = &sv_name[..sv_name.len() - suffix.len()];
            if folder.eq_ignore_ascii_case(stem) {
                return true;
            }
        }
    }
    false
}

/// Walk mirroring `build_installed_index`: top level plus up to 2 sublevels,
/// skipping `.disabled` folders. A folder counts as an addon when it holds a
/// `<folder>/<folder>.txt|.addon` manifest.
fn collect_addon_folders(dir: &Path, out: &mut Vec<AddonFolder>, depth: usize) {
    if depth > 2 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.ends_with(".disabled") {
            continue;
        }
        if let Some(manifest) = find_manifest_in(&path, name) {
            out.push(AddonFolder {
                path: path.clone(),
                name: name.to_string(),
                manifest: manifest.clone(),
                declared_svs: parse_declared_svs(&manifest),
            });
        }
        collect_addon_folders(&path, out, depth + 1);
    }
}

fn find_manifest_in(dir: &Path, base_name: &str) -> Option<PathBuf> {
    let txt = dir.join(format!("{base_name}.txt"));
    if txt.exists() {
        return Some(txt);
    }
    let addon = dir.join(format!("{base_name}.addon"));
    if addon.exists() {
        return Some(addon);
    }
    None
}

/// Collect `## SavedVariables:` directive values (space-separated), tolerant of
/// surrounding whitespace and a leading UTF-8 BOM.
fn parse_declared_svs(manifest: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(manifest) else {
        return Vec::new();
    };
    let raw = String::from_utf8_lossy(&bytes);
    let content = raw.strip_prefix('\u{FEFF}').unwrap_or(&raw);
    let mut names = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("## ").or_else(|| line.strip_prefix("##")) else {
            continue;
        };
        let Some((key, value)) = rest.split_once(':') else {
            continue;
        };
        if key.trim() == "SavedVariables" {
            names.extend(value.split_whitespace().map(|s| s.to_string()));
        }
    }
    names
}

// ── Scan ────────────────────────────────────────────────────────────────

fn scan_matched(matched: &[MatchedFolder]) -> LamScanResponse {
    let mut lua_files: Vec<PathBuf> = Vec::new();
    for m in matched {
        collect_lua_files(&m.path, &mut lua_files);
        if lua_files.len() >= MAX_FILES {
            break;
        }
    }

    // key -> Some(choices) while consistent, None once a conflict is seen.
    let mut acc: HashMap<String, Option<Vec<LamDropdownChoice>>> = HashMap::new();
    let mut scanned = 0u32;

    for path in &lua_files {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        scanned += 1;
        let content = String::from_utf8_lossy(&bytes);
        for (key, choices) in extract_dropdowns(content.as_ref()) {
            match acc.entry(key) {
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert(Some(choices));
                }
                std::collections::hash_map::Entry::Occupied(mut o) => {
                    let conflict = matches!(o.get(), Some(existing) if *existing != choices);
                    if conflict {
                        o.insert(None);
                    }
                }
            }
        }
    }

    let mut hints: Vec<LamDropdownHint> = acc
        .into_iter()
        .filter_map(|(setting_key, choices)| {
            choices.map(|choices| LamDropdownHint {
                setting_key,
                choices,
            })
        })
        .collect();
    hints.sort_by(|a, b| a.setting_key.cmp(&b.setting_key));

    let mut matched_folders: Vec<String> = matched.iter().map(|m| m.name.clone()).collect();
    matched_folders.sort();
    matched_folders.dedup();

    LamScanResponse {
        hints,
        scanned_files: scanned,
        matched_folders,
    }
}

fn collect_lua_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue; // hidden dirs/files, including .git
        }
        if path.is_dir() {
            collect_lua_files(&path, out);
        } else if name.ends_with(".lua") {
            let too_big = std::fs::metadata(&path)
                .map(|m| m.len() > MAX_FILE_BYTES)
                .unwrap_or(true);
            if too_big {
                continue;
            }
            out.push(path);
        }
    }
}

// ── Per-file extraction ─────────────────────────────────────────────────

fn extract_dropdowns(content: &str) -> Vec<(String, Vec<LamDropdownChoice>)> {
    static DROPDOWN_RE: OnceLock<Regex> = OnceLock::new();
    let re = DROPDOWN_RE
        .get_or_init(|| Regex::new(r#"type\s*=\s*["']dropdown["']"#).unwrap());

    let bytes = content.as_bytes();
    let mask = build_mask(bytes);
    let mut out = Vec::new();

    for m in re.find_iter(content) {
        let pos = m.start();
        if mask[pos] {
            continue; // inside a comment or string
        }
        let Some(open) = find_enclosing_open(bytes, &mask, pos) else {
            continue;
        };
        let Some(close) = find_matching_close(bytes, &mask, open) else {
            continue;
        };
        if close - open > MAX_BLOCK {
            continue;
        }
        if let Some(hit) = extract_from_block(bytes, &mask, open, close) {
            out.push(hit);
        }
    }
    out
}

fn extract_from_block(
    bytes: &[u8],
    mask: &[bool],
    open: usize,
    close: usize,
) -> Option<(String, Vec<LamDropdownChoice>)> {
    let fields = scan_top_fields(bytes, mask, open, close);

    let mut choices_text: Option<&str> = None;
    let mut choices_is_table = false;
    let mut choices_values_text: Option<&str> = None;
    let mut choices_values_is_table = false;
    let mut set_func: Option<&str> = None;
    let mut get_func: Option<&str> = None;

    for f in &fields {
        let slice = std::str::from_utf8(&bytes[f.val_start..f.val_end]).ok()?;
        match f.name.as_str() {
            "choices" => {
                choices_text = Some(slice);
                choices_is_table = f.first_char == b'{';
            }
            "choicesValues" => {
                choices_values_text = Some(slice);
                choices_values_is_table = f.first_char == b'{';
            }
            "setFunc" => set_func = Some(slice),
            "getFunc" => get_func = Some(slice),
            _ => {}
        }
    }

    // choices must be a literal table.
    let choices_text = choices_text?;
    if !choices_is_table {
        return None;
    }
    let labels = parse_table_literal(choices_text)?;
    if labels.is_empty() {
        return None;
    }

    // Setting key from setFunc assignment, falling back to getFunc return.
    let mut key = None;
    if let Some(sf) = set_func {
        key = first_assignment_key(sf);
    }
    if key.is_none() {
        if let Some(gf) = get_func {
            key = return_key(gf);
        }
    }
    let key = key?;

    let choices = if let Some(cv_text) = choices_values_text {
        if !choices_values_is_table {
            return None;
        }
        let values = parse_table_literal(cv_text)?;
        if values.len() != labels.len() {
            return None;
        }
        labels
            .iter()
            .zip(values.into_iter())
            .map(|(label, value)| LamDropdownChoice {
                label: value_label(label),
                value,
            })
            .collect()
    } else {
        labels
            .into_iter()
            .map(|v| LamDropdownChoice {
                label: value_label(&v),
                value: v,
            })
            .collect()
    };

    Some((key, choices))
}

struct Field {
    name: String,
    val_start: usize,
    val_end: usize,
    first_char: u8,
}

/// Sequentially parse `key = value` fields at brace-depth 1 of the block. Values
/// are delimited by the next top-level comma or the block's closing brace, so
/// `function ... end` bodies (no braces) and nested tables are captured whole.
fn scan_top_fields(bytes: &[u8], mask: &[bool], open: usize, close: usize) -> Vec<Field> {
    let mut fields = Vec::new();
    let mut i = open + 1;
    while i < close {
        if mask[i] || bytes[i].is_ascii_whitespace() || bytes[i] == b',' || bytes[i] == b';' {
            i += 1;
            continue;
        }
        if is_ident_start(bytes[i]) {
            let ks = i;
            while i < close && !mask[i] && is_ident_cont(bytes[i]) {
                i += 1;
            }
            let name = String::from_utf8_lossy(&bytes[ks..i]).to_string();
            let mut j = i;
            while j < close && (mask[j] || bytes[j].is_ascii_whitespace()) {
                j += 1;
            }
            if j < close
                && bytes[j] == b'='
                && !mask[j]
                && !(j + 1 < close && bytes[j + 1] == b'=')
            {
                let mut v = j + 1;
                while v < close && (mask[v] || bytes[v].is_ascii_whitespace()) {
                    v += 1;
                }
                let first_char = if v < close { bytes[v] } else { b' ' };
                let val_end = skip_value(bytes, mask, v, close);
                fields.push(Field {
                    name,
                    val_start: v,
                    val_end,
                    first_char,
                });
                i = val_end;
            } else {
                i = skip_to_top_comma(bytes, mask, i, close);
            }
        } else {
            i = skip_to_top_comma(bytes, mask, i, close);
        }
    }
    fields
}

fn skip_value(bytes: &[u8], mask: &[bool], start: usize, close: usize) -> usize {
    let mut depth = 0i32;
    let mut i = start;
    while i < close {
        if !mask[i] {
            match bytes[i] {
                b'{' | b'(' | b'[' => depth += 1,
                b'}' | b')' | b']' => {
                    if depth == 0 {
                        return i;
                    }
                    depth -= 1;
                }
                b',' if depth == 0 => return i,
                _ => {}
            }
        }
        i += 1;
    }
    close
}

fn skip_to_top_comma(bytes: &[u8], mask: &[bool], start: usize, close: usize) -> usize {
    let end = skip_value(bytes, mask, start, close);
    // skip_value stops *on* the comma/close; advance past a comma so the caller
    // makes progress.
    if end < close && !mask[end] && bytes[end] == b',' {
        end + 1
    } else {
        end
    }
}

// ── Brace matching ──────────────────────────────────────────────────────

fn find_enclosing_open(bytes: &[u8], mask: &[bool], from: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = from;
    let limit = from.saturating_sub(MAX_BLOCK);
    while i > limit {
        i -= 1;
        if mask[i] {
            continue;
        }
        match bytes[i] {
            b'}' => depth += 1,
            b'{' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

fn find_matching_close(bytes: &[u8], mask: &[bool], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    let cap = (open + MAX_BLOCK).min(bytes.len());
    while i < cap {
        if !mask[i] {
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

// ── Comment / string mask ───────────────────────────────────────────────

/// Build a byte-mask where `true` marks bytes inside a Lua comment or string
/// literal (short/long forms). Single linear scan.
fn build_mask(bytes: &[u8]) -> Vec<bool> {
    let n = bytes.len();
    let mut mask = vec![false; n];
    let mut i = 0;
    while i < n {
        let b = bytes[i];
        // Comments.
        if b == b'-' && i + 1 < n && bytes[i + 1] == b'-' {
            if let Some(level) = long_bracket_level(bytes, i + 2) {
                let open_end = i + 2 + 1 + level + 1;
                let end = find_long_close(bytes, open_end, level).unwrap_or(n);
                fill(&mut mask, i, end);
                i = end;
            } else {
                let mut j = i;
                while j < n && bytes[j] != b'\n' {
                    j += 1;
                }
                fill(&mut mask, i, j);
                i = j;
            }
            continue;
        }
        // Short strings.
        if b == b'"' || b == b'\'' {
            let mut j = i + 1;
            while j < n && bytes[j] != b {
                if bytes[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(n);
            fill(&mut mask, i, end);
            i = end;
            continue;
        }
        // Long-bracket strings.
        if b == b'[' {
            if let Some(level) = long_bracket_level(bytes, i) {
                let open_end = i + 1 + level + 1;
                let end = find_long_close(bytes, open_end, level).unwrap_or(n);
                fill(&mut mask, i, end);
                i = end;
                continue;
            }
        }
        i += 1;
    }
    mask
}

fn fill(mask: &mut [bool], from: usize, to: usize) {
    let end = to.min(mask.len());
    for m in mask.iter_mut().take(end).skip(from) {
        *m = true;
    }
}

/// If `bytes[pos]` opens a long bracket (`[`, `=`* , `[`), return the `=` count.
fn long_bracket_level(bytes: &[u8], pos: usize) -> Option<usize> {
    if pos >= bytes.len() || bytes[pos] != b'[' {
        return None;
    }
    let mut j = pos + 1;
    let mut level = 0;
    while j < bytes.len() && bytes[j] == b'=' {
        level += 1;
        j += 1;
    }
    if j < bytes.len() && bytes[j] == b'[' {
        Some(level)
    } else {
        None
    }
}

/// Find the index just past a long-bracket close (`]`, `=`{level}, `]`).
fn find_long_close(bytes: &[u8], from: usize, level: usize) -> Option<usize> {
    let mut j = from;
    while j < bytes.len() {
        if bytes[j] == b']' {
            let mut k = j + 1;
            let mut cnt = 0;
            while k < bytes.len() && bytes[k] == b'=' {
                cnt += 1;
                k += 1;
            }
            if cnt == level && k < bytes.len() && bytes[k] == b']' {
                return Some(k + 1);
            }
        }
        j += 1;
    }
    None
}

// ── Key extraction from setFunc / getFunc ───────────────────────────────

/// First assignment `LHS = expr` in the body (ignoring `==`/`~=`/`<=`/`>=` and
/// any `=` inside a string/comment), returning the last segment of the LHS chain.
fn first_assignment_key(func: &str) -> Option<String> {
    let bytes = func.as_bytes();
    let mask = build_mask(bytes);
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if !mask[i] && bytes[i] == b'=' {
            let next_eq = i + 1 < n && bytes[i + 1] == b'=';
            let prev = if i > 0 { bytes[i - 1] } else { 0 };
            let comparison = next_eq || matches!(prev, b'=' | b'~' | b'<' | b'>');
            if !comparison {
                if let Some(key) = lhs_before(bytes, &mask, i) {
                    return Some(key);
                }
            }
        }
        i += 1;
    }
    None
}

/// `return <chain>` fallback, returning the last segment of the chain.
fn return_key(func: &str) -> Option<String> {
    static RETURN_RE: OnceLock<Regex> = OnceLock::new();
    let re = RETURN_RE.get_or_init(|| Regex::new(r"\breturn\b").unwrap());
    let bytes = func.as_bytes();
    let mask = build_mask(bytes);
    for m in re.find_iter(func) {
        if mask[m.start()] {
            continue; // "return" inside a string/comment
        }
        if let Some(key) = chain_after(bytes, &mask, m.end()) {
            return Some(key);
        }
    }
    None
}

/// Walk the LHS chain backwards from the `=` at `eq` and return its last segment.
fn lhs_before(bytes: &[u8], mask: &[bool], eq: usize) -> Option<String> {
    if eq == 0 {
        return None;
    }
    // Skip whitespace immediately before `=`.
    let mut e = eq - 1;
    while e > 0 && !mask[e] && bytes[e].is_ascii_whitespace() {
        e -= 1;
    }
    if mask[e] || bytes[e].is_ascii_whitespace() {
        return None;
    }

    let mut start = e + 1; // exclusive until we record a chain byte
    let mut s = e as isize;
    while s >= 0 {
        let idx = s as usize;
        if mask[idx] {
            break; // a masked byte not reached via a bracket group ends the chain
        }
        let c = bytes[idx];
        if is_ident_cont(c) || c == b'.' {
            start = idx;
            s -= 1;
            continue;
        }
        if c == b']' {
            // Jump to the matching `[`, skipping the (masked) key string.
            let mut depth = 1i32;
            let mut k = idx as isize - 1;
            while k >= 0 {
                let ku = k as usize;
                if !mask[ku] {
                    if bytes[ku] == b']' {
                        depth += 1;
                    } else if bytes[ku] == b'[' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
                k -= 1;
            }
            if k < 0 {
                break; // unmatched bracket
            }
            start = k as usize;
            s = k - 1;
            continue;
        }
        break;
    }
    if start > e {
        return None;
    }
    last_segment(std::str::from_utf8(&bytes[start..=e]).ok()?)
}

/// Parse an lvalue chain forward from `start` (mask-aware) and return its last
/// segment. Used for `return` expressions.
fn chain_after(bytes: &[u8], mask: &[bool], start: usize) -> Option<String> {
    let n = bytes.len();
    let mut i = start;
    while i < n && (mask[i] || bytes[i].is_ascii_whitespace()) {
        i += 1;
    }
    if i >= n || !is_ident_start(bytes[i]) {
        return None;
    }
    let begin = i;
    while i < n && !mask[i] && is_ident_cont(bytes[i]) {
        i += 1;
    }
    loop {
        let mut j = i;
        while j < n && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j < n && !mask[j] && bytes[j] == b'.' {
            j += 1;
            while j < n && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let ks = j;
            while j < n && !mask[j] && is_ident_cont(bytes[j]) {
                j += 1;
            }
            if j == ks {
                break;
            }
            i = j;
            continue;
        }
        if j < n && !mask[j] && bytes[j] == b'[' {
            let mut depth = 1i32;
            let mut k = j + 1;
            while k < n {
                if !mask[k] {
                    if bytes[k] == b'[' {
                        depth += 1;
                    } else if bytes[k] == b']' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
                k += 1;
            }
            if k >= n {
                break;
            }
            i = k + 1;
            continue;
        }
        break;
    }
    last_segment(std::str::from_utf8(&bytes[begin..i]).ok()?)
}

/// Last segment of an lvalue chain: text after the final `.`, or the unescaped
/// string inside the final `["..."]`. Non-string bracket keys yield no key.
fn last_segment(lhs: &str) -> Option<String> {
    let t = lhs.trim();
    if t.ends_with(']') {
        static BRACKET_RE: OnceLock<Regex> = OnceLock::new();
        let re = BRACKET_RE.get_or_init(|| {
            Regex::new(r#"\[\s*(?:"((?:\\.|[^"\\])*)"|'((?:\\.|[^'\\])*)')\s*\]\s*$"#).unwrap()
        });
        let caps = re.captures(t)?;
        let inner = caps.get(1).or_else(|| caps.get(2))?.as_str();
        Some(unescape_lua(inner))
    } else {
        static IDENT_RE: OnceLock<Regex> = OnceLock::new();
        let re = IDENT_RE.get_or_init(|| Regex::new(r#"([A-Za-z_]\w*)\s*$"#).unwrap());
        let caps = re.captures(t)?;
        Some(caps.get(1)?.as_str().to_string())
    }
}

// ── Table-literal parsing ───────────────────────────────────────────────

/// Parse a `{...}` slice into literal values. Returns `None` if any entry is a
/// non-literal (identifier, call, nested table, `[k]=v`, concatenation, nil…).
fn parse_table_literal(text: &str) -> Option<Vec<Value>> {
    let t = text.trim();
    if !t.starts_with('{') || !t.ends_with('}') {
        return None;
    }
    let inner = &t[1..t.len() - 1];
    let mut entries = split_entries(inner);
    if entries.last().map(|e| e.trim().is_empty()).unwrap_or(false) {
        entries.pop(); // trailing comma
    }
    if entries.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::with_capacity(entries.len());
    for e in &entries {
        out.push(parse_entry(e)?);
    }
    Some(out)
}

fn split_entries(inner: &str) -> Vec<String> {
    let b = inner.as_bytes();
    let mut res = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let mut i = 0;
    let mut in_str: Option<u8> = None;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = in_str {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == q {
                in_str = None;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' | b'\'' => in_str = Some(c),
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => depth -= 1,
            b',' if depth == 0 => {
                res.push(inner[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    res.push(inner[start..].to_string());
    res
}

fn parse_entry(entry: &str) -> Option<Value> {
    let t = entry.trim();
    if t.is_empty() {
        return None;
    }
    if t == "true" {
        return Some(Value::Bool(true));
    }
    if t == "false" {
        return Some(Value::Bool(false));
    }
    if let Some(s) = as_single_string(t) {
        return Some(Value::String(s));
    }
    parse_number(t)
}

/// If `t` is exactly one quoted string (nothing after the closing quote but
/// whitespace), return its unescaped content.
fn as_single_string(t: &str) -> Option<String> {
    let b = t.as_bytes();
    if b.is_empty() {
        return None;
    }
    let q = b[0];
    if q != b'"' && q != b'\'' {
        return None;
    }
    let mut i = 1;
    while i < b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == q {
            if t[i + 1..].trim().is_empty() {
                return Some(unescape_lua(&t[1..i]));
            }
            return None;
        }
        i += 1;
    }
    None
}

fn parse_number(t: &str) -> Option<Value> {
    let (neg, body) = if let Some(r) = t.strip_prefix('-') {
        (true, r)
    } else if let Some(r) = t.strip_prefix('+') {
        (false, r)
    } else {
        (false, t)
    };
    let body = body.trim();

    if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let v = i64::from_str_radix(hex, 16).ok()?;
        return Some(Value::from(if neg { -v } else { v }));
    }

    if body.is_empty() {
        return None;
    }
    if body.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(v) = body.parse::<i64>() {
            return Some(Value::from(if neg { -v } else { v }));
        }
    }
    // Float: guard against "inf"/"nan" by restricting the character set.
    if !body
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        return None;
    }
    let f: f64 = t.parse().ok()?;
    if !f.is_finite() {
        return None;
    }
    Some(Value::from(f))
}

fn unescape_lua(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            let c = b[i + 1];
            match c {
                b'n' => {
                    out.push('\n');
                    i += 2;
                }
                b't' => {
                    out.push('\t');
                    i += 2;
                }
                b'r' => {
                    out.push('\r');
                    i += 2;
                }
                b'\\' => {
                    out.push('\\');
                    i += 2;
                }
                b'"' => {
                    out.push('"');
                    i += 2;
                }
                b'\'' => {
                    out.push('\'');
                    i += 2;
                }
                b'0'..=b'9' => {
                    let mut j = i + 1;
                    let mut num = 0u32;
                    let mut cnt = 0;
                    while j < b.len() && cnt < 3 && b[j].is_ascii_digit() {
                        num = num * 10 + (b[j] - b'0') as u32;
                        j += 1;
                        cnt += 1;
                    }
                    if let Some(ch) = char::from_u32(num) {
                        out.push(ch);
                    }
                    i = j;
                }
                _ => {
                    out.push(c as char);
                    i += 2;
                }
            }
        } else {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

// ── Label formatting ────────────────────────────────────────────────────

fn value_label(v: &Value) -> String {
    match v {
        Value::String(s) => strip_markup(s),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Strip ESO rich-text markup: `|cRRGGBB` color codes, `|r` resets, `|t…|t`
/// texture tags, `|u…:|u` hyperlinks.
fn strip_markup(s: &str) -> String {
    static MARKUP_RE: OnceLock<Regex> = OnceLock::new();
    let re = MARKUP_RE.get_or_init(|| {
        Regex::new(r"(?i)\|c[0-9a-f]{6}|\|r|\|t[^|]*\|t|\|t|\|u[^|]*:\|u").unwrap()
    });
    re.replace_all(s, "").trim().to_string()
}

// ── Small helpers ───────────────────────────────────────────────────────

fn is_ident_start(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphabetic()
}

fn is_ident_cont(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn keys(content: &str) -> Vec<(String, Vec<LamDropdownChoice>)> {
        extract_dropdowns(content)
    }

    fn find<'a>(
        hits: &'a [(String, Vec<LamDropdownChoice>)],
        key: &str,
    ) -> Option<&'a Vec<LamDropdownChoice>> {
        hits.iter().find(|(k, _)| k == key).map(|(_, c)| c)
    }

    #[test]
    fn basic_string_choices() {
        let src = r#"
        {
            type = "dropdown",
            choices = {"Small", "Medium", "Large"},
            setFunc = function(value) sv.textSize = value end,
        }
        "#;
        let hits = keys(src);
        let c = find(&hits, "textSize").expect("textSize");
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].label, "Small");
        assert_eq!(c[0].value, json!("Small"));
        assert_eq!(c[2].value, json!("Large"));
    }

    #[test]
    fn numeric_choices_values() {
        let src = r#"
        {
            type = "dropdown",
            choices = {"One", "Two", "Three"},
            choicesValues = {1, 2, 3},
            setFunc = function(v) db.mode = v end,
        }
        "#;
        let hits = keys(src);
        let c = find(&hits, "mode").unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].label, "One");
        assert_eq!(c[0].value, json!(1));
        assert_eq!(c[2].value, json!(3));
    }

    #[test]
    fn single_quotes_escapes_trailing_comma_multiline() {
        let src = "
        {
            type = 'dropdown',
            choices = {
                'Don\\'t',
                'Line\\nBreak',
                'Last',
            },
            setFunc = function(value) sv.opt = value end,
        }
        ";
        let hits = keys(src);
        let c = find(&hits, "opt").unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].value, json!("Don't"));
        assert_eq!(c[1].value, json!("Line\nBreak"));
        assert_eq!(c[2].value, json!("Last"));
    }

    #[test]
    fn hex_negative_bool_values() {
        let src = r#"
        {
            type = "dropdown",
            choices = {"A", "B", "C", "D"},
            choicesValues = {0x10, -5, true, false},
            setFunc = function(v) sv.flags = v end,
        }
        "#;
        let hits = keys(src);
        let c = find(&hits, "flags").unwrap();
        assert_eq!(c[0].value, json!(16));
        assert_eq!(c[1].value, json!(-5));
        assert_eq!(c[2].value, json!(true));
        assert_eq!(c[3].value, json!(false));
    }

    #[test]
    fn setfunc_nested_table_key() {
        let src = r#"
        {
            type = "dropdown",
            choices = {"X"},
            setFunc = function(value) FooBar.savedVars.profile.fontName = value end,
        }
        "#;
        let hits = keys(src);
        assert!(find(&hits, "fontName").is_some());
    }

    #[test]
    fn setfunc_bracket_string_key() {
        let src = r#"
        {
            type = "dropdown",
            choices = {"X"},
            setFunc = function(v) FooSV["some key"] = v end,
        }
        "#;
        let hits = keys(src);
        assert!(find(&hits, "some key").is_some());
    }

    #[test]
    fn setfunc_ignores_equality_before_assignment() {
        let src = r#"
        {
            type = "dropdown",
            choices = {"X"},
            setFunc = function(v)
                if v == 1 then return end
                sv.realKey = v
            end,
        }
        "#;
        let hits = keys(src);
        assert!(find(&hits, "realKey").is_some());
        assert!(find(&hits, "v").is_none());
    }

    #[test]
    fn getfunc_fallback_when_setfunc_has_no_assignment() {
        let src = r#"
        {
            type = "dropdown",
            choices = {"X"},
            getFunc = function() return sv.selected end,
            setFunc = function(v) DoSomething(v) end,
        }
        "#;
        let hits = keys(src);
        assert!(find(&hits, "selected").is_some());
    }

    #[test]
    fn skip_non_literal_choices_variable() {
        let src = r#"
        {
            type = "dropdown",
            choices = myChoicesTable,
            setFunc = function(v) sv.x = v end,
        }
        "#;
        assert!(keys(src).is_empty());
    }

    #[test]
    fn skip_getstring_choices() {
        let src = r#"
        {
            type = "dropdown",
            choices = { GetString(SI_A), GetString(SI_B) },
            setFunc = function(v) sv.x = v end,
        }
        "#;
        assert!(keys(src).is_empty());
    }

    #[test]
    fn skip_nested_table_entry() {
        let src = r#"
        {
            type = "dropdown",
            choices = { "A", {1, 2}, "B" },
            setFunc = function(v) sv.x = v end,
        }
        "#;
        assert!(keys(src).is_empty());
    }

    #[test]
    fn skip_length_mismatch() {
        let src = r#"
        {
            type = "dropdown",
            choices = {"A", "B", "C"},
            choicesValues = {1, 2},
            setFunc = function(v) sv.x = v end,
        }
        "#;
        assert!(keys(src).is_empty());
    }

    #[test]
    fn skip_no_key() {
        let src = r#"
        {
            type = "dropdown",
            choices = {"A", "B"},
            setFunc = function(v) DoThing(v) end,
        }
        "#;
        assert!(keys(src).is_empty());
    }

    #[test]
    fn comment_and_string_immunity() {
        let src = "
        -- type = \"dropdown\"
        local note = \"type = \\\"dropdown\\\" with { braces } inside\"
        local real = {
            type = \"dropdown\",
            choices = {\"A\", \"B\"},
            setFunc = function(v) sv.only = v end,
        }
        ";
        let hits = keys(src);
        assert_eq!(hits.len(), 1);
        assert!(find(&hits, "only").is_some());
    }

    #[test]
    fn label_markup_stripped() {
        let src = r#"
        {
            type = "dropdown",
            choices = {"|c00FF00Green|r", "|cFF0000Red|r"},
            setFunc = function(v) sv.color = v end,
        }
        "#;
        let hits = keys(src);
        let c = find(&hits, "color").unwrap();
        assert_eq!(c[0].label, "Green");
        // value retains the raw literal.
        assert_eq!(c[0].value, json!("|c00FF00Green|r"));
        assert_eq!(c[1].label, "Red");
    }

    #[test]
    fn multiple_dropdowns_one_file() {
        let src = r#"
        local a = {
            type = "dropdown",
            choices = {"A", "B"},
            setFunc = function(v) sv.first = v end,
        }
        local b = {
            type = "dropdown",
            choices = {"C", "D"},
            setFunc = function(v) sv.second = v end,
        }
        "#;
        let hits = keys(src);
        assert!(find(&hits, "first").is_some());
        assert!(find(&hits, "second").is_some());
    }

    #[test]
    fn dedupe_conflicting_dropped_identical_kept() {
        let dir = tempfile::tempdir().unwrap();
        let addon = dir.path().join("MyAddon");
        fs::create_dir_all(&addon).unwrap();
        fs::write(
            addon.join("MyAddon.txt"),
            "## Title: MyAddon\n## SavedVariables: MyAddonSV\n",
        )
        .unwrap();
        // conflicting key `dup`, plus a stable key `same` defined twice identically.
        fs::write(
            addon.join("a.lua"),
            r#"
            {
                type = "dropdown",
                choices = {"A", "B"},
                setFunc = function(v) sv.dup = v end,
            }
            {
                type = "dropdown",
                choices = {"X", "Y"},
                setFunc = function(v) sv.same = v end,
            }
            "#,
        )
        .unwrap();
        fs::write(
            addon.join("b.lua"),
            r#"
            {
                type = "dropdown",
                choices = {"C", "D"},
                setFunc = function(v) sv.dup = v end,
            }
            {
                type = "dropdown",
                choices = {"X", "Y"},
                setFunc = function(v) sv.same = v end,
            }
            "#,
        )
        .unwrap();

        let resp = scan_lam_dropdowns_blocking(dir.path(), "MyAddonSV").unwrap();
        let has_dup = resp.hints.iter().any(|h| h.setting_key == "dup");
        let same: Vec<_> = resp.hints.iter().filter(|h| h.setting_key == "same").collect();
        assert!(!has_dup, "conflicting key should be dropped");
        assert_eq!(same.len(), 1, "identical key kept once");
    }

    #[test]
    fn end_to_end_manifest_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let addon = dir.path().join("MyAddon");
        fs::create_dir_all(&addon).unwrap();
        fs::write(
            addon.join("MyAddon.txt"),
            "## Title: MyAddon\n## SavedVariables: MyAddonSV\n",
        )
        .unwrap();
        fs::write(
            addon.join("settings.lua"),
            r#"
            local panel = {
                type = "dropdown",
                choices = {"Low", "High"},
                setFunc = function(value) MyAddon.sv.quality = value end,
            }
            "#,
        )
        .unwrap();

        let resp = scan_lam_dropdowns_blocking(dir.path(), "MyAddonSV").unwrap();
        assert_eq!(resp.matched_folders, vec!["MyAddon".to_string()]);
        assert!(resp.scanned_files >= 1);
        assert!(resp.hints.iter().any(|h| h.setting_key == "quality"));

        // Unknown sv_name → empty.
        let empty = scan_lam_dropdowns_blocking(dir.path(), "NoSuchSV").unwrap();
        assert!(empty.hints.is_empty());
        assert_eq!(empty.scanned_files, 0);
    }

    #[test]
    fn nested_subfolder_addon_found_and_disabled_skipped() {
        let dir = tempfile::tempdir().unwrap();
        // Nested one sublevel deep under a wrapper folder.
        let nested = dir.path().join("Libs").join("NestedAddon");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            nested.join("NestedAddon.txt"),
            "## Title: NestedAddon\n## SavedVariables: NestedSV\n",
        )
        .unwrap();
        fs::write(
            nested.join("n.lua"),
            r#"
            {
                type = "dropdown",
                choices = {"A", "B"},
                setFunc = function(v) sv.nestedKey = v end,
            }
            "#,
        )
        .unwrap();

        // A disabled folder that also declares the SV must be ignored.
        let disabled = dir.path().join("Other.disabled");
        fs::create_dir_all(&disabled).unwrap();
        fs::write(
            disabled.join("Other.txt"),
            "## Title: Other\n## SavedVariables: NestedSV\n",
        )
        .unwrap();
        fs::write(
            disabled.join("d.lua"),
            r#"
            {
                type = "dropdown",
                choices = {"Z"},
                setFunc = function(v) sv.shouldNotAppear = v end,
            }
            "#,
        )
        .unwrap();

        let resp = scan_lam_dropdowns_blocking(dir.path(), "NestedSV").unwrap();
        assert_eq!(resp.matched_folders, vec!["NestedAddon".to_string()]);
        assert!(resp.hints.iter().any(|h| h.setting_key == "nestedKey"));
        assert!(!resp.hints.iter().any(|h| h.setting_key == "shouldNotAppear"));
    }

    #[test]
    fn folder_name_suffix_fallback() {
        let dir = tempfile::tempdir().unwrap();
        // Manifest declares nothing; folder "MyAddon" + suffix "SavedVars".
        let addon = dir.path().join("MyAddon");
        fs::create_dir_all(&addon).unwrap();
        fs::write(addon.join("MyAddon.txt"), "## Title: MyAddon\n").unwrap();
        fs::write(
            addon.join("s.lua"),
            r#"
            {
                type = "dropdown",
                choices = {"A"},
                setFunc = function(v) sv.viaFallback = v end,
            }
            "#,
        )
        .unwrap();

        let resp = scan_lam_dropdowns_blocking(dir.path(), "MyAddonSavedVars").unwrap();
        assert!(resp.hints.iter().any(|h| h.setting_key == "viaFallback"));
    }

    #[test]
    fn braces_inside_strings_do_not_break_block() {
        let src = r#"
        {
            type = "dropdown",
            choices = {"a}b", "c{d"},
            setFunc = function(v) sv.tricky = v end,
        }
        "#;
        let hits = keys(src);
        let c = find(&hits, "tricky").unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].value, json!("a}b"));
        assert_eq!(c[1].value, json!("c{d"));
    }
}
