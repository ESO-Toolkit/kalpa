//! Streaming, bounded-memory scanner that reproduces
//! [`detect_identities_from_tree`] without parsing the whole SavedVariables file
//! into an `SvTreeNode` tree.
//!
//! [`detect_identities_from_tree`]: super::scrub::detect_identities_from_tree
//!
//! The identity-export path ([`crate::commands`]'s `collect_local_identities`)
//! must see EVERY account / world / character-id across all local SV files so an
//! imported `.esopack`'s placeholders can be resolved. The tree-based
//! `detect_identities_from_tree` did this by parsing each `.lua` fully (~10x its
//! source size in memory), and a single SavedVariables file can reach 1–2 GB.
//! This scanner walks the raw bytes with the same state-machine technique as
//! [`roster_stream`](super::roster_stream), holding only `O(nesting depth + one
//! key)` regardless of file size, and emits the SAME identities the tree
//! detector finds.
//!
//! ## Parity with the tree detector
//!
//! The lexer here is byte-for-byte the same structural scanner as
//! `roster_stream` (same comment/string/long-bracket/identifier handling), so it
//! is parser-equivalent on every key, comment, and whitespace position. Only the
//! semantic layer differs — it mirrors the `classify_*` recursion in `scrub.rs`
//! rather than the stricter `roster_*` recursion:
//!
//! * `detect_identities_from_tree` classifies a key by NAME regardless of its
//!   value type (a scalar `["@Foo"] = true` still records the account, and any
//!   non-marker key under an account is a character — it deliberately
//!   over-collects so scrubbing never leaks an identity). So identities are
//!   emitted at **key-commit** time (the `=`), not only when a `{` table opens.
//! * The context stack mirrors which `classify_*` function the tree detector
//!   would apply to a table's direct children:
//!     - `Default → @account → Char/CharId`
//!     - `Default → "<World>" → @account → Char/CharId`
//!     - `"<World>" → @account → Char/CharId`          (world-first, pChat)
//!     - `@account` directly under the addon var        (numeric char-ids only)
//!
//! A parity test asserts this scanner yields the identical `ScrubContext` as
//! `detect_identities_from_tree` on every representative fixture.

use std::io::{self, Read};

use super::scrub::{ScrubContext, WELL_KNOWN_WORLDS};

/// Read buffer size. Chunking is irrelevant to correctness — the state machine
/// fully persists across chunk boundaries.
const READ_CHUNK: usize = 64 * 1024;

/// Cap on a single buffered key. No real ESO structural or identity key comes
/// close. A longer key is pathological; it is treated as unusable and bounds
/// memory. (Unlike the roster scanner this does not surface a warning, because
/// `collect_local_identities` merges best-effort and has no malformed channel.)
const MAX_KEY_BYTES: usize = 4096;

/// Cap on the context-stack depth — a pure memory guard against pathologically
/// nested braces (real ESO data nests ~6 deep). Beyond this we stop pushing real
/// contexts and treat the extra depth as `Ignore`.
const MAX_CTX_DEPTH: usize = 4096;

/// Cap on distinct identities of each kind retained by a single scan. A real
/// account has a few accounts / worlds / dozens of characters; this ceiling is
/// wildly generous yet bounds output against a file stuffed with millions of
/// distinct identity-shaped keys (the only term that would otherwise grow with
/// file size).
const MAX_IDENTITIES_PER_KIND: usize = 100_000;

/// Classification of the table we are currently inside — i.e. how its DIRECT
/// children should be treated. Mirrors which `classify_*` function the tree
/// detector applies to those children.
#[derive(Debug, Clone, Copy)]
enum Ctx {
    /// The synthetic file root: a table opened here from an identifier
    /// assignment is a top-level addon variable, classified as `UnderTop`.
    File,
    /// Direct children of an addon variable (`classify_under_top`).
    UnderTop,
    /// Children of an `@account` sitting directly under the addon variable (no
    /// `Default`): only numeric character-ids are collected here.
    AccountDirectUnderTop,
    /// Children of `Default` (`classify_account_or_world`).
    AccountOrWorld,
    /// Children of a world layer (`classify_world_layer`): only `@account`
    /// children descend.
    WorldLayer,
    /// Children of an `@account` handle (`classify_under_account`): characters
    /// and numeric character-ids.
    UnderAccount,
    /// Anything not on an identity-bearing path — descend but never classify.
    Ignore,
}

/// Where a comment returns to once it ends — the whitespace-skipping context it
/// interrupted.
#[derive(Debug, Clone, Copy)]
enum Resume {
    Normal,
    KeyToken,
    RBracket,
    Equals,
    IdentEquals,
}

impl Resume {
    fn state(self) -> Lex {
        match self {
            Resume::Normal => Lex::Normal,
            Resume::KeyToken => Lex::SeekKeyToken,
            Resume::RBracket => Lex::SeekRBracket,
            Resume::Equals => Lex::SeekEquals,
            Resume::IdentEquals => Lex::SeekIdentEquals,
        }
    }
}

/// Lexer / structural state. Identical to `roster_stream`'s lexer.
#[derive(Debug, Clone, Copy)]
enum Lex {
    Normal,
    Dash,
    SeekDash {
        resume: Resume,
    },
    CommentStart {
        resume: Resume,
    },
    LineComment {
        resume: Resume,
    },
    BlockOpen {
        eqs: usize,
        resume: Resume,
    },
    BlockBody {
        level: usize,
        m: Option<usize>,
        resume: Resume,
    },
    AfterLBracket,
    LongOpen {
        eqs: usize,
    },
    LongBody {
        level: usize,
        m: Option<usize>,
    },
    Str {
        quote: u8,
        escaped: bool,
    },
    KeyStr {
        escaped: bool,
    },
    KeyNum,
    Ident,
    Scalar {
        is_number: bool,
    },
    ScalarDash {
        is_number: bool,
    },
    SeekKeyToken,
    SeekRBracket,
    SeekEquals,
    SeekIdentEquals,
}

struct Scanner {
    ctx: Vec<Ctx>,
    overflow: usize,
    pending_key: Option<Vec<u8>>,
    pending_ident: bool,
    value_expected: bool,
    key_buf: Vec<u8>,
    key_overflow: bool,
    lex: Lex,
    accounts: std::collections::BTreeSet<String>,
    characters: std::collections::BTreeSet<String>,
    character_ids: std::collections::BTreeSet<String>,
    extra_worlds: std::collections::BTreeSet<String>,
}

#[inline]
fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

#[inline]
fn continues_scalar(b: u8, is_number: bool) -> bool {
    if is_number {
        b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+')
    } else {
        b.is_ascii_lowercase()
    }
}

/// Mirrors the tree detector's world test: a canonical megaserver name, or any
/// key containing a space (`classify_*`'s `key.contains(' ')`).
#[inline]
fn is_world_layer(key: &[u8]) -> bool {
    WELL_KNOWN_WORLDS.iter().any(|w| w.as_bytes() == key) || key.contains(&b' ')
}

#[inline]
fn is_well_known_world(key: &[u8]) -> bool {
    WELL_KNOWN_WORLDS.iter().any(|w| w.as_bytes() == key)
}

/// A non-empty, all-ASCII-digit key of at least 10 digits — the tree detector's
/// numeric character-id rule (`classify_under_account` / the `@`-under-top arm).
#[inline]
fn is_character_id(key: &[u8]) -> bool {
    !key.is_empty() && key.len() >= 10 && key.iter().all(|b| b.is_ascii_digit())
}

/// Decode a raw key exactly as the parser stores it (`from_utf8_lossy`), so the
/// emitted strings compare equal to the tree detector's.
#[inline]
fn decode(key: &[u8]) -> String {
    String::from_utf8_lossy(key).into_owned()
}

/// Derive the context for a NON-`File` table being opened, given its parent
/// context and the key it is bound to (`None` for a keyless/array-element table
/// or an unusable over-long key). Mirrors the `classify_*` descent decisions.
fn derive(parent: Ctx, key: Option<&[u8]>) -> Ctx {
    let Some(k) = key else {
        return Ctx::Ignore;
    };
    match parent {
        Ctx::File => Ctx::UnderTop, // caller (`open_table`) already checked from_ident
        Ctx::UnderTop => {
            if k == b"Default" {
                Ctx::AccountOrWorld
            } else if k.first() == Some(&b'@') {
                Ctx::AccountDirectUnderTop
            } else if is_world_layer(k) {
                Ctx::WorldLayer
            } else {
                Ctx::Ignore
            }
        }
        Ctx::AccountDirectUnderTop => Ctx::Ignore,
        Ctx::AccountOrWorld => {
            if k.first() == Some(&b'@') {
                Ctx::UnderAccount
            } else if is_world_layer(k) {
                Ctx::WorldLayer
            } else {
                Ctx::Ignore
            }
        }
        Ctx::WorldLayer => {
            if k.first() == Some(&b'@') {
                Ctx::UnderAccount
            } else {
                Ctx::Ignore
            }
        }
        Ctx::UnderAccount | Ctx::Ignore => Ctx::Ignore,
    }
}

impl Scanner {
    fn new() -> Self {
        Scanner {
            ctx: vec![Ctx::File],
            overflow: 0,
            pending_key: None,
            pending_ident: false,
            value_expected: false,
            key_buf: Vec::new(),
            key_overflow: false,
            lex: Lex::Normal,
            accounts: std::collections::BTreeSet::new(),
            characters: std::collections::BTreeSet::new(),
            character_ids: std::collections::BTreeSet::new(),
            extra_worlds: std::collections::BTreeSet::new(),
        }
    }

    #[inline]
    fn clear_pending(&mut self) {
        self.pending_key = None;
        self.pending_ident = false;
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

    /// Commit the buffered key as the pending key for the value after `=`, and
    /// classify it against the current container context (the tree detector
    /// classifies by key name regardless of the value's type). An over-long key
    /// is treated as unusable (`None`) and contributes no identity.
    fn commit_key(&mut self, is_ident: bool) {
        self.value_expected = true;
        self.pending_ident = is_ident;
        self.pending_key = if self.key_overflow {
            None
        } else {
            Some(std::mem::take(&mut self.key_buf))
        };
        self.key_buf.clear();
        self.key_overflow = false;
        self.lex = Lex::Normal;

        // Classify the committed key. Skipped at overflow depth, where the
        // container context is beyond the tracked stack (treated as `Ignore`).
        if self.overflow == 0 {
            if let Some(key) = self.pending_key.clone() {
                self.classify_committed(&key);
            }
        }
    }

    /// Record an identity for `key` based on the container context, mirroring the
    /// `classify_*` functions in `scrub.rs`.
    fn classify_committed(&mut self, key: &[u8]) {
        let parent = self.ctx.last().copied().unwrap_or(Ctx::Ignore);
        match parent {
            Ctx::UnderTop => {
                if key == b"Default" {
                    // Structural; descent handled at table open.
                } else if key.first() == Some(&b'@') {
                    self.insert_account(key);
                } else if is_world_layer(key) {
                    self.insert_extra_world(key);
                }
            }
            Ctx::AccountDirectUnderTop => {
                if is_character_id(key) {
                    self.insert_character_id(key);
                }
            }
            Ctx::AccountOrWorld => {
                if key.first() == Some(&b'@') {
                    self.insert_account(key);
                } else if is_world_layer(key) {
                    self.insert_extra_world(key);
                }
            }
            Ctx::WorldLayer => {
                if key.first() == Some(&b'@') {
                    self.insert_account(key);
                }
            }
            Ctx::UnderAccount => {
                if key.first() == Some(&b'$') {
                    // `$AccountWide` and friends — markers, not identities.
                } else if !key.is_empty() && key.iter().all(|b| b.is_ascii_digit()) {
                    if key.len() >= 10 {
                        self.insert_character_id(key);
                    }
                } else if !key.is_empty() {
                    self.insert_character(key);
                }
            }
            Ctx::File | Ctx::Ignore => {}
        }
    }

    #[inline]
    fn insert_account(&mut self, key: &[u8]) {
        if self.accounts.len() < MAX_IDENTITIES_PER_KIND {
            self.accounts.insert(decode(key));
        }
    }

    #[inline]
    fn insert_extra_world(&mut self, key: &[u8]) {
        // Only NON-canonical world names go into `extra_worlds`, matching
        // `classify_world_layer` (which skips `WELL_KNOWN_WORLDS`).
        if !is_well_known_world(key) && self.extra_worlds.len() < MAX_IDENTITIES_PER_KIND {
            self.extra_worlds.insert(decode(key));
        }
    }

    #[inline]
    fn insert_character(&mut self, key: &[u8]) {
        if self.characters.len() < MAX_IDENTITIES_PER_KIND {
            self.characters.insert(decode(key));
        }
    }

    #[inline]
    fn insert_character_id(&mut self, key: &[u8]) {
        if self.character_ids.len() < MAX_IDENTITIES_PER_KIND {
            self.character_ids.insert(decode(key));
        }
    }

    /// Open a table: push the child context. Identities are recorded at
    /// key-commit time, not here, since the tree detector classifies by key name
    /// regardless of value type.
    fn open_table(&mut self) {
        if self.overflow > 0 {
            self.overflow += 1;
            self.clear_pending();
            return;
        }
        let key = self.pending_key.take();
        let from_ident = self.pending_ident;
        self.pending_ident = false;
        let parent = self.ctx.last().copied().unwrap_or(Ctx::Ignore);

        let child = match parent {
            // The top level only becomes an addon variable when the table was
            // opened from an identifier assignment (`Foo = { ... }`), exactly as
            // `parse_sv_file` recognizes top-level vars.
            Ctx::File => {
                if key.is_some() && from_ident {
                    Ctx::UnderTop
                } else {
                    Ctx::Ignore
                }
            }
            _ => derive(parent, key.as_deref()),
        };
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
        }
        self.clear_pending();
    }

    #[inline]
    fn feed(&mut self, b: u8) {
        while !self.step(b) {}
    }

    fn step(&mut self, b: u8) -> bool {
        match self.lex {
            Lex::Normal => self.step_normal(b),
            Lex::Dash => {
                if b == b'-' {
                    self.lex = Lex::CommentStart {
                        resume: Resume::Normal,
                    };
                    true
                } else {
                    self.lex = Lex::Scalar { is_number: true };
                    false
                }
            }
            Lex::SeekDash { resume } => {
                if b == b'-' {
                    self.lex = Lex::CommentStart { resume };
                    true
                } else {
                    self.lex = resume.state();
                    false
                }
            }
            Lex::CommentStart { resume } => {
                match b {
                    b'[' => self.lex = Lex::BlockOpen { eqs: 0, resume },
                    b'\n' => self.lex = resume.state(),
                    _ => self.lex = Lex::LineComment { resume },
                }
                true
            }
            Lex::LineComment { resume } => {
                if b == b'\n' {
                    self.lex = resume.state();
                }
                true
            }
            Lex::BlockOpen { eqs, resume } => {
                match b {
                    b'=' => {
                        self.lex = Lex::BlockOpen {
                            eqs: eqs + 1,
                            resume,
                        }
                    }
                    b'[' => {
                        self.lex = Lex::BlockBody {
                            level: eqs,
                            m: None,
                            resume,
                        }
                    }
                    b'\n' => self.lex = resume.state(),
                    _ => self.lex = Lex::LineComment { resume },
                }
                true
            }
            Lex::BlockBody { level, m, resume } => {
                match long_close_step(level, m, b) {
                    Some(newm) => {
                        self.lex = Lex::BlockBody {
                            level,
                            m: newm,
                            resume,
                        }
                    }
                    None => self.lex = resume.state(),
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
            Lex::Ident => {
                if b.is_ascii_alphanumeric() || b == b'_' {
                    self.push_key(b);
                    true
                } else {
                    self.lex = Lex::SeekIdentEquals;
                    false
                }
            }
            Lex::Scalar { is_number } => {
                if b == b'-' {
                    self.lex = Lex::ScalarDash { is_number };
                    true
                } else if continues_scalar(b, is_number) {
                    true
                } else {
                    self.clear_pending();
                    self.lex = Lex::Normal;
                    false
                }
            }
            Lex::ScalarDash { is_number } => {
                if b == b'-' {
                    self.clear_pending();
                    self.lex = Lex::CommentStart {
                        resume: Resume::Normal,
                    };
                    true
                } else {
                    self.lex = Lex::Scalar { is_number };
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
                        b'0'..=b'9' => {
                            self.key_buf.clear();
                            self.key_overflow = false;
                            self.push_key(b);
                            self.lex = Lex::KeyNum;
                            true
                        }
                        b'-' => {
                            self.lex = Lex::SeekDash {
                                resume: Resume::KeyToken,
                            };
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
                } else if b == b'-' {
                    self.lex = Lex::SeekDash {
                        resume: Resume::RBracket,
                    };
                    true
                } else {
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
                    self.commit_key(false);
                    true
                } else if b == b'-' {
                    self.lex = Lex::SeekDash {
                        resume: Resume::Equals,
                    };
                    true
                } else {
                    self.key_buf.clear();
                    self.key_overflow = false;
                    self.lex = Lex::Normal;
                    false
                }
            }
            Lex::SeekIdentEquals => {
                if is_ws(b) {
                    true
                } else if b == b'=' {
                    self.commit_key(true);
                    true
                } else if b == b'-' {
                    self.lex = Lex::SeekDash {
                        resume: Resume::IdentEquals,
                    };
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
            b'=' => true,
            _ => {
                if self.value_expected {
                    let is_number = b.is_ascii_digit() || b == b'.';
                    self.lex = Lex::Scalar { is_number };
                    false
                } else if b.is_ascii_alphabetic() || b == b'_' {
                    self.key_buf.clear();
                    self.key_overflow = false;
                    self.push_key(b);
                    self.lex = Lex::Ident;
                    true
                } else {
                    true
                }
            }
        }
    }

    fn step_after_lbracket(&mut self, b: u8) -> bool {
        if self.value_expected {
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
                    self.lex = Lex::Normal;
                    false
                }
            }
        } else {
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
                b'0'..=b'9' => {
                    self.key_buf.clear();
                    self.key_overflow = false;
                    self.push_key(b);
                    self.lex = Lex::KeyNum;
                    true
                }
                b'-' => {
                    self.lex = Lex::SeekDash {
                        resume: Resume::KeyToken,
                    };
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

    fn into_context(self) -> ScrubContext {
        ScrubContext {
            accounts: self.accounts.into_iter().collect(),
            characters: self.characters.into_iter().collect(),
            character_ids: self.character_ids.into_iter().collect(),
            extra_worlds: self.extra_worlds.into_iter().collect(),
        }
    }
}

/// Advance the long-bracket close matcher for one body byte. Identical to
/// `roster_stream`'s.
#[inline]
fn long_close_step(level: usize, m: Option<usize>, b: u8) -> Option<Option<usize>> {
    match b {
        b']' => {
            if m == Some(level) {
                None
            } else {
                Some(Some(0))
            }
        }
        b'=' => match m {
            Some(n) => Some(Some(n + 1)),
            None => Some(None),
        },
        _ => Some(None),
    }
}

/// Stream `reader` and extract the identity `ScrubContext` with bounded memory.
/// Emits the same accounts / worlds / characters / character-ids as
/// [`detect_identities_from_tree`](super::scrub::detect_identities_from_tree)
/// on any cleanly-parsing SavedVariables content. Only fails on an underlying
/// I/O error; a malformed/truncated file contributes whatever it could recover.
pub fn detect_identities_streaming<R: Read>(mut reader: R) -> io::Result<ScrubContext> {
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
    Ok(scanner.into_context())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saved_variables::parser::parse_sv_file;
    use crate::saved_variables::scrub::detect_identities_from_tree;

    fn stream_ctx(lua: &str) -> ScrubContext {
        detect_identities_streaming(std::io::Cursor::new(lua.as_bytes().to_vec())).unwrap()
    }

    /// Assert the streaming scanner and the tree detector agree exactly.
    fn assert_parity(lua: &str) {
        let tree = parse_sv_file(lua, "test.lua").expect("fixture parses");
        let tree_ctx = detect_identities_from_tree(&tree);
        let stream = stream_ctx(lua);
        assert_eq!(
            stream.accounts, tree_ctx.accounts,
            "accounts diverged for:\n{lua}"
        );
        assert_eq!(
            stream.characters, tree_ctx.characters,
            "characters diverged for:\n{lua}"
        );
        assert_eq!(
            stream.character_ids, tree_ctx.character_ids,
            "character_ids diverged for:\n{lua}"
        );
        assert_eq!(
            stream.extra_worlds, tree_ctx.extra_worlds,
            "extra_worlds diverged for:\n{lua}"
        );
    }

    #[test]
    fn parity_standard_account_layout() {
        assert_parity(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = { ["enabled"] = true },
                        ["Mainchar"] = { ["x"] = 1 },
                        ["Alttank"] = { ["x"] = 2 },
                        ["123456789012345"] = { ["y"] = 3 },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_world_scoped_layout() {
        assert_parity(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["NA Megaserver"] = {
                        ["@Author"] = {
                            ["$AccountWide"] = { ["v"] = 1 },
                            ["Mainchar"] = { ["x"] = 1 },
                        },
                    },
                    ["EU Megaserver"] = {
                        ["@Author"] = {
                            ["Euchar"] = { ["x"] = 2 },
                        },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_world_first_pchat_layout() {
        assert_parity(
            r#"PCHAT_OPTS = {
                ["NA Megaserver"] = {
                    ["@Author"] = {
                        ["Mainchar"] = { ["x"] = 1 },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_account_direct_under_top() {
        // IIfA-style: account key directly under the addon var (no Default). Only
        // numeric character-ids are collected there; section keys are ignored.
        assert_parity(
            r#"IIfA_Data = {
                ["Default"] = { ["@Primary"] = { ["Mainchar"] = { ["a"] = 1 } } },
                ["@Secondary"] = {
                    ["settings"] = { ["x"] = 1 },
                    ["987654321012"] = { ["y"] = 2 },
                    ["1234"] = { ["z"] = 3 },
                },
            }"#,
        );
    }

    #[test]
    fn parity_custom_world_extra_worlds() {
        assert_parity(
            r#"Addon = {
                ["Default"] = {
                    ["Custom Realm"] = {
                        ["@Author"] = { ["Mainchar"] = { ["x"] = 1 } },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_scalar_valued_identity_keys() {
        // The tree detector classifies by key NAME regardless of value type: a
        // scalar '@'-key is still an account, and a scalar non-marker key under an
        // account is still a (over-collected) character.
        assert_parity(
            r#"Addon = {
                ["Default"] = {
                    ["@Author"] = {
                        ["version"] = 3,
                        ["Mainchar"] = { ["x"] = 1 },
                    },
                },
            }"#,
        );
    }

    #[test]
    fn parity_multiple_top_level_vars_and_comments() {
        assert_parity(
            r#"-- a comment
            First_SV = {
                ["Default"] = { ["@A1"] = { ["CharA"] = { ["x"] = 1 } } },
            }
            Second_SV = {
                ["Default"] = { ["@A2"] = { ["CharB"] = { ["y"] = 2 } } }, -- trailing
            }"#,
        );
    }

    #[test]
    fn parity_non_identity_config_only() {
        assert_parity(
            r#"Addon = {
                ["settings"] = { ["foo"] = 1 },
                ["Default"] = { ["notaccount"] = { ["x"] = 1 } },
            }"#,
        );
    }

    #[test]
    fn parity_empty_and_no_vars() {
        assert_parity("");
        assert_parity("-- just a comment\n");
        assert_parity(r#"Addon = { ["Default"] = { } }"#);
    }
}
