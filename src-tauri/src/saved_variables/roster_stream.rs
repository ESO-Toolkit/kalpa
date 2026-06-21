//! Streaming, bounded-memory extractor for the Characters roster.
//!
//! Replaces the previous "parse the whole file into an `SvTreeNode` tree" roster
//! scan, which had to SKIP any SavedVariables file larger than 64 MiB for memory
//! safety — hiding any character whose only key lived inside a huge file. This
//! module walks the raw bytes with a small state machine, emitting the SAME
//! `(name, world)` set that [`detect_roster_characters_from_tree`] produces
//! (verified by a parity test over the scrub fixtures), while never holding more
//! than `O(nesting depth + one key)` in memory regardless of file size.
//!
//! [`detect_roster_characters_from_tree`]: super::scrub::detect_roster_characters_from_tree
//!
//! ## Why a hand-rolled scanner mirrors the tree detector
//!
//! ESO SavedVariables are machine-generated. The structural keys that matter —
//! `Default`, the `@account` handle, the megaserver world layer, and the
//! character names — are ALWAYS double-quoted string keys (`["..."]`). The
//! contexts below mirror `scrub.rs`'s `roster_*` recursion exactly:
//!
//! * `Default → @account → CharName`              → world `None`
//! * `Default → "<World>" → @account → CharName`   → world = canonical(World)
//! * `"<World>" → @account → CharName` (pChat)     → world = canonical(World)
//!
//! An `@account` sitting directly under the addon variable (no `Default`) is
//! skipped, exactly as `roster_under_top` falls through. A character key is
//! accepted only when its value is a `{` table and it passes
//! [`looks_like_character_name`] (and is not empty / `$`-marker / all-digits).
//!
//! ## Deliberate, documented divergences from the tree path
//!
//! * **Unquoted identifier keys are inert.** ESO never writes an unquoted
//!   structural key, and an identifier key can never equal `Default`/`@…`/a
//!   world or pass `looks_like_character_name`, so ignoring them changes no
//!   real or fixture input.
//! * **Comments** are skipped between top-level entries and in value position
//!   (where the tree parser also skips them); ESO SavedVariables contain none.
//! * **Malformed input is not silently trusted.** The tree path skips a whole
//!   file on any parse error; this scanner keeps going (recovering what it can)
//!   but reports [`RosterScan::malformed`] when it ends mid-token or with
//!   unbalanced braces, so the caller can still warn that the roster may be
//!   incomplete. Parity is therefore defined over inputs that parse cleanly
//!   (every fixture and every real ESO file).

use std::collections::BTreeSet;
use std::io::{self, Read};

use super::scrub::{looks_like_character_name, WELL_KNOWN_WORLDS};

/// Read buffer size. Chunking is irrelevant to correctness — the state machine
/// fully persists across chunk boundaries.
const READ_CHUNK: usize = 64 * 1024;

/// Cap on a single buffered key. Structural keys and character names are short
/// (`looks_like_character_name` itself caps names at 32 chars), so a longer key
/// can be neither a structural level nor a character — it is treated as if no
/// usable key were present. Keeps memory bounded against a pathological key.
const MAX_KEY_BYTES: usize = 512;

/// Cap on the context stack depth. A pure memory guard against pathologically
/// nested braces (real ESO data nests ~6 deep). Beyond this we stop pushing real
/// contexts and treat the extra depth as `Ignore`. Parity with the tree detector
/// is only defined within the parser's own nesting limit anyway.
const MAX_CTX_DEPTH: usize = 4096;

/// Result of a streaming scan: the `(name, world)` set plus whether the scan hit
/// a malformed/truncated structure (unterminated token or unbalanced braces).
#[derive(Debug, Clone, Default)]
pub struct RosterScan {
    pub characters: BTreeSet<(String, Option<String>)>,
    pub malformed: bool,
}

/// Classification of the table we are currently inside — i.e. how its DIRECT
/// children should be treated. Mirrors which `roster_*` function the tree
/// detector would apply to those children.
#[derive(Debug, Clone)]
enum Ctx {
    /// The synthetic file root: any table opened here is a top-level addon
    /// variable, whose children are classified as `UnderTop`.
    File,
    /// Direct children of an addon variable (`roster_under_top`).
    UnderTop,
    /// Children of `Default` (`roster_account_or_world`).
    AccountOrWorld,
    /// Children of a world layer (`roster_world_layer`); only `@account`
    /// children descend, carrying this canonical megaserver (or `None`).
    WorldLayer(Option<String>),
    /// Children of an `@account` handle (`roster_chars_under_account`): the
    /// character keys, attributed to this megaserver (or `None`).
    CharsUnderAccount(Option<String>),
    /// Anything not on a character-bearing path — descend but never emit.
    Ignore,
}

/// Lexer / structural state. All fields are `Copy`, so the whole enum is `Copy`
/// and we can freely match it by value while mutating `self.lex`.
#[derive(Debug, Clone, Copy)]
enum Lex {
    /// Between tokens (structural scanning).
    Normal,
    /// Consumed a `-` in `Normal`; awaiting a second `-` for a comment.
    Dash,
    /// Consumed `--`; deciding line vs block comment.
    CommentStart,
    /// Inside a line comment, until `\n`.
    LineComment,
    /// Consumed `--[` then `eqs` `=`; awaiting `[` to open a block comment.
    BlockOpen { eqs: usize },
    /// Inside a block comment of the given long-bracket `level`; `m` tracks the
    /// close `]` `=`*level `]` progress (`None` = not after a `]`).
    BlockBody { level: usize, m: Option<usize> },
    /// Consumed `[` in `Normal`; deciding long string vs `["..."]`/`[num]` key.
    AfterLBracket,
    /// Consumed `[` then `eqs` `=`; awaiting `[` to open a long-string value.
    LongOpen { eqs: usize },
    /// Inside a long-string VALUE; close-matching like `BlockBody`.
    LongBody { level: usize, m: Option<usize> },
    /// Inside a quoted string VALUE (`"..."` / `'...'`).
    Str { quote: u8, escaped: bool },
    /// Reading a `["..."]` key into `key_buf` (raw bytes, escapes retained).
    KeyStr { escaped: bool },
    /// Reading a `[<digits>]` numeric key into `key_buf`.
    KeyNum,
    /// Consuming a scalar VALUE (number / true / false / nil) in value position.
    Scalar,
    /// After `[` + whitespace; awaiting `"` or a digit/`-` key token.
    SeekKeyToken,
    /// After a key token; skipping whitespace until `]`.
    SeekRBracket,
    /// After `]`; skipping whitespace until `=`.
    SeekEquals,
}

struct Scanner {
    /// Context stack; `last()` classifies the table we are currently inside.
    ctx: Vec<Ctx>,
    /// Brace depth beyond `MAX_CTX_DEPTH` (treated as `Ignore`, not stored).
    overflow: usize,
    /// The confirmed key for the value currently expected (set on `=`). `Some`
    /// also means "value position"; `None` means entry start.
    pending_key: Option<Vec<u8>>,
    /// True after `=` until the value is consumed (value-position indicator,
    /// independent of `pending_key` so an over-long key still tracks position).
    value_expected: bool,
    /// Accumulates the in-progress `["..."]`/`[num]` key bytes.
    key_buf: Vec<u8>,
    /// The in-progress key exceeded `MAX_KEY_BYTES`; treat it as unusable.
    key_overflow: bool,
    lex: Lex,
    out: BTreeSet<(String, Option<String>)>,
    malformed: bool,
}

#[inline]
fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

/// A byte that can appear inside a scalar value token (number or keyword).
/// Permissive on purpose — the value is skipped, not interpreted.
#[inline]
fn is_scalar_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-' | b'_')
}

#[inline]
fn is_world_layer(key: &[u8]) -> bool {
    WELL_KNOWN_WORLDS.iter().any(|w| w.as_bytes() == key) || key.contains(&b' ')
}

#[inline]
fn canonical_world(key: &[u8]) -> Option<String> {
    WELL_KNOWN_WORLDS
        .iter()
        .find(|w| w.as_bytes() == key)
        .map(|w| (*w).to_string())
}

/// Does this decoded key qualify as an ESO character name under an account?
/// Mirrors `roster_chars_under_account` exactly (the value-is-table requirement
/// is enforced by the caller, which only calls this when opening a `{`).
fn name_qualifies(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('$')
        && !name.bytes().all(|b| b.is_ascii_digit())
        && looks_like_character_name(name)
}

/// Derive the context for a table being opened, given its parent context and the
/// key the table is bound to (`None` for a keyless/array-element table or an
/// unusable over-long key). Mirrors the `roster_*` descent decisions.
fn derive(parent: &Ctx, key: Option<&[u8]>) -> Ctx {
    // A top-level table is always an addon variable, regardless of key.
    if let Ctx::File = parent {
        return Ctx::UnderTop;
    }
    // A keyless table (array element) never matches a structural transition —
    // the tree parser gives it a synthetic numeric key that no `roster_*` arm
    // accepts, so it is effectively `Ignore`.
    let Some(k) = key else {
        return Ctx::Ignore;
    };
    match parent {
        Ctx::UnderTop => {
            if k == b"Default" {
                Ctx::AccountOrWorld
            } else if is_world_layer(k) {
                Ctx::WorldLayer(canonical_world(k))
            } else {
                Ctx::Ignore
            }
        }
        Ctx::AccountOrWorld => {
            if k.first() == Some(&b'@') {
                Ctx::CharsUnderAccount(None)
            } else if is_world_layer(k) {
                Ctx::WorldLayer(canonical_world(k))
            } else {
                Ctx::Ignore
            }
        }
        Ctx::WorldLayer(world) => {
            if k.first() == Some(&b'@') {
                Ctx::CharsUnderAccount(world.clone())
            } else {
                Ctx::Ignore
            }
        }
        Ctx::CharsUnderAccount(_) | Ctx::Ignore => Ctx::Ignore,
        Ctx::File => unreachable!("File handled above"),
    }
}

impl Scanner {
    fn new() -> Self {
        Scanner {
            ctx: vec![Ctx::File],
            overflow: 0,
            pending_key: None,
            value_expected: false,
            key_buf: Vec::new(),
            key_overflow: false,
            lex: Lex::Normal,
            out: BTreeSet::new(),
            malformed: false,
        }
    }

    #[inline]
    fn clear_pending(&mut self) {
        self.pending_key = None;
        self.value_expected = false;
    }

    #[inline]
    fn push_key(&mut self, b: u8) {
        if self.key_buf.len() < MAX_KEY_BYTES {
            self.key_buf.push(b);
        } else {
            self.key_overflow = true;
        }
    }

    /// Open a table: emit a character if applicable, then push the child context.
    fn open_table(&mut self) {
        if self.overflow > 0 {
            self.overflow += 1;
            self.clear_pending();
            return;
        }
        let key = self.pending_key.take();
        let parent = self.ctx.last().cloned().unwrap_or(Ctx::Ignore);

        if let Ctx::CharsUnderAccount(world) = &parent {
            if let Some(k) = &key {
                let name = String::from_utf8_lossy(k);
                if name_qualifies(&name) {
                    self.out.insert((name.into_owned(), world.clone()));
                }
            }
        }

        let child = derive(&parent, key.as_deref());
        if self.ctx.len() >= MAX_CTX_DEPTH {
            self.overflow += 1;
        } else {
            self.ctx.push(child);
        }
        self.value_expected = false;
    }

    fn close_table(&mut self) {
        if self.overflow > 0 {
            self.overflow -= 1;
        } else if self.ctx.len() > 1 {
            self.ctx.pop();
        } else {
            // `}` with nothing open: unbalanced input.
            self.malformed = true;
        }
        self.clear_pending();
    }

    /// Feed one byte, re-processing it after any non-consuming state change.
    #[inline]
    fn feed(&mut self, b: u8) {
        while !self.step(b) {}
    }

    /// Process one byte. Returns `true` if the byte was consumed, `false` if the
    /// state changed and the same byte should be re-processed (always resolves
    /// in at most one extra pass, since the fallback state consumes every byte).
    fn step(&mut self, b: u8) -> bool {
        match self.lex {
            Lex::Normal => self.step_normal(b),
            Lex::Dash => {
                if b == b'-' {
                    self.lex = Lex::CommentStart;
                    true
                } else {
                    // A lone `-`: the start of a (possibly negative) scalar.
                    self.lex = Lex::Scalar;
                    false
                }
            }
            Lex::CommentStart => {
                match b {
                    b'[' => self.lex = Lex::BlockOpen { eqs: 0 },
                    b'\n' => self.lex = Lex::Normal,
                    _ => self.lex = Lex::LineComment,
                }
                true
            }
            Lex::LineComment => {
                if b == b'\n' {
                    self.lex = Lex::Normal;
                }
                true
            }
            Lex::BlockOpen { eqs } => {
                match b {
                    b'=' => self.lex = Lex::BlockOpen { eqs: eqs + 1 },
                    b'[' => {
                        self.lex = Lex::BlockBody {
                            level: eqs,
                            m: None,
                        }
                    }
                    b'\n' => self.lex = Lex::Normal, // was a line comment
                    _ => self.lex = Lex::LineComment,
                }
                true
            }
            Lex::BlockBody { level, m } => {
                match long_close_step(level, m, b) {
                    Some(newm) => self.lex = Lex::BlockBody { level, m: newm },
                    None => self.lex = Lex::Normal, // comment closed
                }
                true
            }
            Lex::AfterLBracket => self.step_after_lbracket(b),
            Lex::LongOpen { eqs } => {
                match b {
                    b'=' => self.lex = Lex::LongOpen { eqs: eqs + 1 },
                    b'[' => {
                        self.lex = Lex::LongBody {
                            level: eqs,
                            m: None,
                        }
                    }
                    _ => {
                        // Not a valid long bracket (e.g. `[==x`): malformed.
                        self.malformed = true;
                        self.lex = Lex::Normal;
                        return false;
                    }
                }
                true
            }
            Lex::LongBody { level, m } => {
                match long_close_step(level, m, b) {
                    Some(newm) => self.lex = Lex::LongBody { level, m: newm },
                    None => {
                        // Long-string value closed.
                        if self.value_expected {
                            self.clear_pending();
                        }
                        self.lex = Lex::Normal;
                    }
                }
                true
            }
            Lex::Str { quote, escaped } => {
                if escaped {
                    self.lex = Lex::Str {
                        quote,
                        escaped: false,
                    };
                } else if b == b'\\' {
                    self.lex = Lex::Str {
                        quote,
                        escaped: true,
                    };
                } else if b == quote {
                    if self.value_expected {
                        self.clear_pending();
                    }
                    self.lex = Lex::Normal;
                }
                true
            }
            Lex::KeyStr { escaped } => {
                if escaped {
                    self.push_key(b);
                    self.lex = Lex::KeyStr { escaped: false };
                } else if b == b'\\' {
                    // Retain the backslash verbatim, exactly like parse_table_key.
                    self.push_key(b'\\');
                    self.lex = Lex::KeyStr { escaped: true };
                } else if b == b'"' {
                    self.lex = Lex::SeekRBracket;
                } else {
                    self.push_key(b);
                }
                true
            }
            Lex::KeyNum => {
                if b.is_ascii_digit() {
                    self.push_key(b);
                    true
                } else {
                    self.lex = Lex::SeekRBracket;
                    false
                }
            }
            Lex::Scalar => {
                if is_scalar_char(b) {
                    true
                } else {
                    self.clear_pending();
                    self.lex = Lex::Normal;
                    false
                }
            }
            Lex::SeekKeyToken => {
                if is_ws(b) {
                    true
                } else {
                    match b {
                        b'"' => {
                            self.key_buf.clear();
                            self.key_overflow = false;
                            self.lex = Lex::KeyStr { escaped: false };
                            true
                        }
                        b'0'..=b'9' | b'-' => {
                            self.key_buf.clear();
                            self.key_overflow = false;
                            self.push_key(b);
                            self.lex = Lex::KeyNum;
                            true
                        }
                        _ => {
                            self.lex = Lex::Normal;
                            false
                        }
                    }
                }
            }
            Lex::SeekRBracket => {
                if is_ws(b) {
                    true
                } else if b == b']' {
                    self.lex = Lex::SeekEquals;
                    true
                } else {
                    // Not a key after all.
                    self.key_buf.clear();
                    self.key_overflow = false;
                    self.lex = Lex::Normal;
                    false
                }
            }
            Lex::SeekEquals => {
                if is_ws(b) {
                    true
                } else if b == b'=' {
                    self.value_expected = true;
                    self.pending_key = if self.key_overflow {
                        None
                    } else {
                        Some(std::mem::take(&mut self.key_buf))
                    };
                    self.key_buf.clear();
                    self.key_overflow = false;
                    self.lex = Lex::Normal;
                    true
                } else {
                    self.key_buf.clear();
                    self.key_overflow = false;
                    self.lex = Lex::Normal;
                    false
                }
            }
        }
    }

    fn step_normal(&mut self, b: u8) -> bool {
        match b {
            _ if is_ws(b) => true,
            b'{' => {
                self.open_table();
                true
            }
            b'}' => {
                self.close_table();
                true
            }
            b',' => {
                self.clear_pending();
                true
            }
            b'"' | b'\'' => {
                self.lex = Lex::Str {
                    quote: b,
                    escaped: false,
                };
                true
            }
            b'[' => {
                self.lex = Lex::AfterLBracket;
                true
            }
            b'-' => {
                self.lex = Lex::Dash;
                true
            }
            b'=' => {
                // An identifier-key (or top-level addon var) assignment.
                self.value_expected = true;
                true
            }
            _ => {
                if self.value_expected {
                    // Start of a scalar value (number / true / false / nil).
                    self.lex = Lex::Scalar;
                    false
                } else {
                    // Inert: identifier-key bytes / array-element keyword.
                    true
                }
            }
        }
    }

    fn step_after_lbracket(&mut self, b: u8) -> bool {
        if self.value_expected {
            // Value position: `[` must open a long string (`[[` / `[=[`).
            match b {
                b'[' => {
                    self.lex = Lex::LongBody { level: 0, m: None };
                    true
                }
                b'=' => {
                    self.lex = Lex::LongOpen { eqs: 1 };
                    true
                }
                _ => {
                    self.malformed = true;
                    self.lex = Lex::Normal;
                    false
                }
            }
        } else {
            // Entry start: a `["..."]`/`[num]` key, or an array-element long
            // string (`[[` / `[=[`).
            match b {
                b'[' => {
                    self.lex = Lex::LongBody { level: 0, m: None };
                    true
                }
                b'=' => {
                    self.lex = Lex::LongOpen { eqs: 1 };
                    true
                }
                b'"' => {
                    self.key_buf.clear();
                    self.key_overflow = false;
                    self.lex = Lex::KeyStr { escaped: false };
                    true
                }
                b'0'..=b'9' | b'-' => {
                    self.key_buf.clear();
                    self.key_overflow = false;
                    self.push_key(b);
                    self.lex = Lex::KeyNum;
                    true
                }
                _ if is_ws(b) => {
                    self.lex = Lex::SeekKeyToken;
                    true
                }
                _ => {
                    self.lex = Lex::Normal;
                    false
                }
            }
        }
    }

    fn finish(&mut self) {
        // A clean end is `Normal` (or a value/comment that ran to EOF). Any other
        // state means a token was cut off mid-stream.
        let clean = matches!(
            self.lex,
            Lex::Normal | Lex::LineComment | Lex::CommentStart | Lex::Scalar
        );
        if !clean {
            self.malformed = true;
        }
        // Unbalanced braces: tables left open at EOF.
        if self.ctx.len() != 1 || self.overflow != 0 {
            self.malformed = true;
        }
    }
}

/// Advance the long-bracket close matcher for one body byte. `m` is the close
/// progress (`None` = not currently after a `]`, `Some(n)` = saw `]` then `n`
/// `=`). Returns `None` when the close `]` `=`*level `]` completes, or
/// `Some(new_m)` with the updated progress to stay inside the body.
#[inline]
fn long_close_step(level: usize, m: Option<usize>, b: u8) -> Option<Option<usize>> {
    match b {
        b']' => {
            if m == Some(level) {
                None // close sequence complete
            } else {
                Some(Some(0)) // this `]` starts a fresh close candidate
            }
        }
        b'=' => match m {
            Some(n) => Some(Some(n + 1)),
            None => Some(None), // stays None (not currently after a `]`)
        },
        _ => Some(None), // any other byte resets the candidate to None
    }
}

/// Stream `reader` and extract the roster `(name, world)` set with bounded
/// memory. Returns the set plus a `malformed` flag for unbalanced/truncated
/// input. Only fails on an underlying I/O error.
pub fn extract_roster_characters_streaming<R: Read>(mut reader: R) -> io::Result<RosterScan> {
    let mut scanner = Scanner::new();
    let mut buf = [0u8; READ_CHUNK];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            scanner.feed(b);
        }
    }
    scanner.finish();
    Ok(RosterScan {
        characters: scanner.out,
        malformed: scanner.malformed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saved_variables::parser::parse_sv_file;
    use crate::saved_variables::scrub::detect_roster_characters_from_tree;

    /// Run the streaming extractor over `lua` bytes.
    fn stream(lua: &[u8]) -> RosterScan {
        extract_roster_characters_streaming(std::io::Cursor::new(lua.to_vec())).unwrap()
    }

    /// The tree detector's `(name, world)` set — the parity oracle.
    fn tree_set(lua: &str) -> BTreeSet<(String, Option<String>)> {
        let tree = parse_sv_file(lua, "test.lua").expect("fixture parses");
        detect_roster_characters_from_tree(&tree)
            .into_iter()
            .map(|c| (c.name, c.world))
            .collect()
    }

    /// Assert the streamer and tree detector agree on a clean-parsing fixture.
    fn assert_parity(lua: &str) {
        let scan = stream(lua.as_bytes());
        assert!(
            !scan.malformed,
            "fixture should scan cleanly:\n{lua}\ngot malformed=true"
        );
        assert_eq!(
            scan.characters,
            tree_set(lua),
            "streaming set != tree set for:\n{lua}"
        );
    }

    // ── Parity over the exact scrub.rs roster fixtures ──────────────────────

    #[test]
    fn parity_account_keyed_characters() {
        assert_parity(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = { ["enabled"] = true },
                        ["Mainchar"] = { ["x"] = 1 },
                        ["Alttank"] = { ["x"] = 2 },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_world_scoped_server() {
        assert_parity(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["EU Megaserver"] = {
                        ["@Author"] = {
                            ["$AccountWide"] = { ["enabled"] = true },
                            ["Mainchar"] = { ["x"] = 1 },
                        },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_world_scoped_config_without_marker() {
        assert_parity(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["NA Megaserver"] = {
                        ["@Author"] = {
                            ["guilds"] = { ["g1"] = true },
                            ["profiles"] = { ["p1"] = true },
                        },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_markerless_character_names() {
        assert_parity(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["Mainchar"] = { ["x"] = 1 },
                        ["Alt Ego"] = { ["x"] = 2 },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_config_siblings_in_marked_account() {
        assert_parity(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = { ["x"] = 1 },
                        ["Mainchar"] = { ["x"] = 1 },
                        ["settings"] = { ["volume"] = 5 },
                        ["profiles"] = { ["p1"] = true },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_capitalized_config_siblings() {
        assert_parity(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = { ["x"] = 1 },
                        ["Mainchar"] = { ["x"] = 1 },
                        ["Settings"] = { ["volume"] = 5 },
                        ["Profile"] = { ["p"] = 1 },
                        ["Servers"] = { ["na"] = true },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_config_section_without_marker() {
        assert_parity(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["settings"] = { ["volume"] = 5 },
                        ["servers"] = { ["NA"] = true },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_scalar_and_numeric_keys() {
        assert_parity(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = { ["enabled"] = true },
                        ["version"] = 3,
                        ["123456789012345"] = { ["x"] = 1 },
                        ["Realchar"] = { ["x"] = 1 },
                    },
                },
            }"#,
        );
    }

    // ── Additional structural fixtures ──────────────────────────────────────

    #[test]
    fn parity_account_directly_under_top_is_skipped() {
        // No `Default` wrapper: the `@account` sits directly under the addon var,
        // so its children are addon sections, not characters.
        assert_parity(
            r#"MyAddon_SV = {
                ["@Author"] = {
                    ["Mainchar"] = { ["x"] = 1 },
                },
            }"#,
        );
    }

    #[test]
    fn parity_pchat_world_first_layout() {
        // World layer directly under the addon var (pChat style).
        assert_parity(
            r#"pChatData = {
                ["NA Megaserver"] = {
                    ["@Author"] = {
                        ["Mainchar"] = { ["x"] = 1 },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_keyless_array_wrapper_emits_nothing() {
        // An array-element table wrapping the account: the tree parser gives it a
        // synthetic numeric key that no structural transition accepts.
        assert_parity(
            r#"MyAddon_SV = {
                ["Default"] = {
                    {
                        ["@Author"] = {
                            ["Mainchar"] = { ["x"] = 1 },
                        },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_caret_suffix_key_preserved() {
        assert_parity(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["Faewynd^Mx"] = { ["x"] = 1 },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_whitespace_around_key_and_equals() {
        // ESO writes `["x"] =` with a space and a newline before the value.
        assert_parity(
            "MyAddon_SV =\n{\n\t[\"Default\"] =\n\t{\n\t\t[ \"@Author\" ] =\n\t\t{\n\t\t\t[\"Mainchar\"]\n\t\t\t= { [\"x\"] = 1 },\n\t\t},\n\t},\n}\n",
        );
    }

    #[test]
    fn parity_scalar_values_dont_leak_onto_next_table() {
        assert_parity(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["lastSeen"] = "Mainchar",
                        ["count"] = 5,
                        ["Realchar"] = { ["x"] = 1 },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_multiple_addon_vars_and_worlds() {
        assert_parity(
            r#"AddonA = {
                ["Default"] = {
                    ["NA Megaserver"] = {
                        ["@Acct"] = { ["Bob"] = { ["x"] = 1 } },
                    },
                    ["EU Megaserver"] = {
                        ["@Acct"] = { ["Bob"] = { ["x"] = 2 } },
                    },
                },
            }
            AddonB = {
                ["Default"] = {
                    ["@Acct"] = { ["Carol"] = { ["y"] = 1 } },
                },
            }"#,
        );
    }

    #[test]
    fn parity_non_canonical_spaced_world_is_unknown() {
        // A custom world key containing a space is a world layer in BOTH paths
        // but contributes world = None.
        assert_parity(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["PC Live"] = {
                        ["@Author"] = {
                            ["Mainchar"] = { ["x"] = 1 },
                        },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_long_string_and_comment_values() {
        // Long-string and commented values must not derail the structure.
        assert_parity(
            "MyAddon_SV = {\n\
             -- a line comment\n\
             [\"Default\"] = {\n\
             --[[ a block comment ]]\n\
             [\"@Author\"] = {\n\
             [\"note\"] = [[a long\nstring with } and ] inside]],\n\
             [\"Mainchar\"] = { [\"x\"] = 1 },\n\
             },\n\
             },\n\
             }\n",
        );
    }

    // ── Key-decoding fidelity (must match parse_table_key byte-for-byte) ─────

    #[test]
    fn key_with_backslash_escape_matches_tree() {
        // parse_table_key keeps the raw backslash in the key; the streamer must
        // too, so the emitted name is byte-identical to the tree's.
        let lua = "MyAddon_SV = {\n  [\"Default\"] = {\n    [\"@Author\"] = {\n      [\"Na\\\"me\"] = { [\"x\"] = 1 },\n    },\n  },\n}\n";
        assert_eq!(stream(lua.as_bytes()).characters, tree_set(lua));
    }

    #[test]
    fn non_utf8_value_bytes_dont_derail_extraction() {
        // Invalid UTF-8 bytes inside a string VALUE: the streamer walks raw bytes
        // and still finds the valid character key after it. (The old tree path
        // read files via `read_to_string` and would have rejected this file
        // outright, so streaming is strictly better here.)
        let mut lua: Vec<u8> = Vec::new();
        lua.extend_from_slice(
            b"MyAddon_SV = {\n[\"Default\"] = {\n[\"@Author\"] = {\n[\"icon\"] = \"",
        );
        lua.extend_from_slice(&[0xff, 0xfe, 0x00, 0x80]); // invalid UTF-8 blob
        lua.extend_from_slice(b"\",\n[\"Faewynd\"] = { [\"x\"] = 1 },\n},\n},\n}\n");

        let scan = stream(&lua);
        assert!(!scan.malformed);
        let names: BTreeSet<String> = scan.characters.iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(names, BTreeSet::from(["Faewynd".to_string()]));
    }

    #[test]
    fn non_utf8_key_bytes_rejected_like_tree() {
        // A character key containing invalid UTF-8 lossy-decodes to U+FFFD, which
        // `looks_like_character_name` rejects — identically in both paths.
        let mut lua: Vec<u8> = Vec::new();
        lua.extend_from_slice(b"MyAddon_SV = {\n[\"Default\"] = {\n[\"@Author\"] = {\n[\"Ka");
        lua.extend_from_slice(&[0xff, 0xfe]); // invalid UTF-8 inside the key
        lua.extend_from_slice(b"l\"] = { [\"x\"] = 1 },\n},\n},\n}\n");

        let lossy = String::from_utf8_lossy(&lua).into_owned();
        assert_eq!(stream(&lua).characters, tree_set(&lossy));
        assert!(stream(&lua).characters.is_empty());
    }

    // ── Chunk-boundary robustness ───────────────────────────────────────────

    #[test]
    fn split_at_every_byte_boundary_is_stable() {
        // Feeding one byte at a time exercises every possible chunk split; the
        // result must be identical to a whole-buffer scan.
        let lua = r#"AddonA = {
            ["Default"] = {
                ["NA Megaserver"] = {
                    ["@Acct"] = {
                        ["$AccountWide"] = { ["k"] = [[long ] ]] = ]] },
                        ["Faewynd^Mx"] = { ["x"] = 1 },
                        ["settings"] = { ["v"] = 1 },
                    },
                },
            },
        }"#;
        let whole = stream(lua.as_bytes());

        let mut scanner = Scanner::new();
        for &b in lua.as_bytes() {
            scanner.feed(b);
        }
        scanner.finish();
        assert_eq!(scanner.out, whole.characters);
        assert!(!whole.malformed);
    }

    #[test]
    fn long_bracket_close_split_across_levels() {
        // `]==]` does not close a level-1 long string; `]=]` does.
        assert_parity(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["blob"] = [=[ contains ]==] and ]] but closes here ]=],
                        ["Mainchar"] = { ["x"] = 1 },
                    },
                },
            }"#,
        );
    }

    // ── Malformed detection ─────────────────────────────────────────────────

    #[test]
    fn truncated_file_flagged_malformed() {
        // Unterminated table at EOF.
        let scan = stream(b"MyAddon_SV = {\n[\"Default\"] = {\n[\"@Author\"] = {\n");
        assert!(scan.malformed);
    }

    #[test]
    fn unterminated_string_flagged_malformed() {
        let scan =
            stream(b"MyAddon_SV = {\n[\"Default\"] = {\n[\"@Author\"] = {\n[\"x\"] = \"oops");
        assert!(scan.malformed);
    }

    #[test]
    fn balanced_file_not_flagged() {
        let scan = stream(
            b"MyAddon_SV = {\n[\"Default\"] = {\n[\"@Author\"] = {\n[\"Bob\"] = { },\n},\n},\n}\n",
        );
        assert!(!scan.malformed);
        assert_eq!(scan.characters.len(), 1);
    }

    // ── Bounded memory: a valid character after a huge block ────────────────

    /// A `Read` that emits `prefix`, then `fill` repeated to `fill_total` bytes,
    /// then `suffix` — all lazily, so the huge middle is never materialized.
    struct HugeReader {
        prefix: Vec<u8>,
        fill: u8,
        fill_total: usize,
        fill_done: usize,
        suffix: Vec<u8>,
        pos: usize, // 0 = prefix, 1 = fill, 2 = suffix
    }

    impl Read for HugeReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.pos == 0 {
                let n = self.prefix.len().min(buf.len());
                buf[..n].copy_from_slice(&self.prefix[..n]);
                self.prefix.drain(..n);
                if self.prefix.is_empty() {
                    self.pos = 1;
                }
                return Ok(n);
            }
            if self.pos == 1 {
                let remaining = self.fill_total - self.fill_done;
                if remaining == 0 {
                    self.pos = 2;
                } else {
                    let n = remaining.min(buf.len());
                    for slot in buf.iter_mut().take(n) {
                        *slot = self.fill;
                    }
                    self.fill_done += n;
                    return Ok(n);
                }
            }
            // suffix
            let n = self.suffix.len().min(buf.len());
            buf[..n].copy_from_slice(&self.suffix[..n]);
            self.suffix.drain(..n);
            Ok(n)
        }
    }

    #[test]
    fn finds_character_after_huge_long_string_past_old_cap() {
        // A valid character key sits AFTER a long-string `$AccountWide` blob that
        // is far larger than the old 64 MiB cap. The blob streams through the
        // no-buffer LongBody path, so memory stays bounded; the character is
        // still found and a config sibling beside it is still rejected.
        let prefix = b"MyAddon_SV = {\n\
            [\"Default\"] = {\n\
            [\"@Author\"] = {\n\
            [\"$AccountWide\"] = { [\"blob\"] = [["
            .to_vec();
        // 80 MiB of filler inside the long string (no `]` so it can't close).
        let fill_total = 80 * 1024 * 1024;
        let suffix = b"]] },\n\
            [\"Realchar\"] = { [\"x\"] = 1 },\n\
            [\"settings\"] = { [\"v\"] = 1 },\n\
            },\n\
            },\n\
            }\n"
        .to_vec();

        let reader = HugeReader {
            prefix,
            fill: b'x',
            fill_total,
            fill_done: 0,
            suffix,
            pos: 0,
        };
        let scan = extract_roster_characters_streaming(reader).unwrap();
        assert!(!scan.malformed);
        let names: BTreeSet<String> = scan.characters.iter().map(|(n, _)| n.clone()).collect();
        assert!(
            names.contains("Realchar"),
            "character after huge blob found"
        );
        assert!(!names.contains("settings"), "config sibling rejected");
        assert_eq!(names.len(), 1);
    }
}
