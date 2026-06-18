//! Secure token storage using Windows Credential Manager.
//!
//! Auth tokens (access_token, refresh_token, expires_at, user_id, user_name)
//! are stored as a JSON string in the OS credential store instead of plaintext
//! in the tauri-plugin-store settings file. Non-sensitive settings remain in the
//! store.

use crate::auth::AuthTokens;

const SERVICE: &str = "kalpa";
/// Legacy single-entry key (raw JSON). Read for back-compat; never written to
/// by the chunked format. New data uses `auth_tokens.count` + `auth_tokens.{N}`.
const USER: &str = "auth_tokens";

// ── Windows implementation (real credential manager) ────────────────────
//
// Windows Credential Manager caps a credential blob at 2560 bytes of UTF-16
// (≈1280 ASCII chars). An ESO Logs access+refresh JWT pair serialized to JSON
// far exceeds that, so a single `set_password` of the whole token JSON fails —
// which broke login, refresh, AND migration (they all funnel through
// `save_tokens`). We base64-encode the JSON (→ pure ASCII, exactly 2 UTF-16
// bytes per char) and split it into fixed-size chunks across multiple
// credential entries, well under the limit.

#[cfg(windows)]
use base64::{engine::general_purpose::STANDARD, Engine};

/// Blob key prefix for the auth tokens: count sentinel at `auth_tokens.count`,
/// chunk N at `auth_tokens.{N}` (the historical layout, preserved exactly).
#[cfg(windows)]
const CHUNK_PREFIX: &str = "auth_tokens";
/// Upper bound on chunks read/swept (sanity cap; ~64 KB of base64 max).
#[cfg(windows)]
const MAX_CHUNKS: usize = 64;
/// Base64 chars per chunk → 2000 UTF-16 bytes, a ~28% margin under 2560.
#[cfg(windows)]
const CHUNK_LEN: usize = 1000;

#[cfg(windows)]
fn entry(user: &str) -> Option<keyring::Entry> {
    keyring::Entry::new(SERVICE, user).ok()
}

// ── Generic chunked blob storage ─────────────────────────────────────────
//
// The credential-manager 2560-byte cap applies to any blob, so the chunked
// fail-closed write/read is factored out here and reused for both the auth
// tokens and the upload-session cookie. A blob is addressed by a `key` prefix:
// the count sentinel lives at `{key}.count` and chunk N at `{key}.{N}`.

/// Write `data` (an arbitrary byte string) under `key` using the fail-closed
/// chunked scheme. Returns false (after logging) if any chunk write failed; on
/// failure the count sentinel is left cleared so a reader fails closed rather
/// than reading a half-written blob.
#[cfg(windows)]
fn save_chunked(key: &str, data: &[u8]) -> bool {
    let count_key = format!("{key}.count");
    // base64 → pure ASCII so each char is exactly 2 UTF-16 bytes; chunk by byte
    // count, which (ASCII) equals char count.
    let b64 = STANDARD.encode(data);
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(CHUNK_LEN).collect();

    // Fail-closed ordering: invalidate the count sentinel FIRST, then write the
    // new chunks (overwriting same-index slots), then flip the count to the new
    // total as the single commit point, then sweep orphans. Invalidating first
    // is what makes a mid-write crash safe: overwriting a slot in place destroys
    // the old chunk there, so a crash between slots would otherwise leave the old
    // count pointing at a base64 splice of old+new chunks — which the loader
    // decodes to garbage. With the sentinel cleared up front, a mid-write crash
    // leaves NO valid count, so the loader cleanly falls back (legacy entry or
    // None → a re-login) instead of reading a corrupt blob. We cannot keep the
    // prior blob recoverable without an atomic multi-key write (the credential
    // store has none); clean fail-closed is the correct minimal guarantee.
    //
    // The whole guarantee rests on the sentinel actually being gone before we
    // touch any chunk, so we VERIFY the deletion rather than ignore its result:
    // a delete that returns anything but success/NoEntry, or a sentinel still
    // readable afterward, means the old count could survive and pair with mixed
    // old/new chunks — so we abort BEFORE writing any chunk (the old blob stays
    // intact and readable; nothing is corrupted).
    let Some(count_entry) = entry(&count_key) else {
        eprintln!("[token_store] failed to open {key} count entry");
        return false;
    };
    match count_entry.delete_credential() {
        Ok(()) => {}
        Err(keyring::Error::NoEntry) => {} // already absent — fine
        Err(err) => {
            eprintln!("[token_store] could not invalidate {key} count sentinel: {err}");
            return false;
        }
    }
    // Confirm it is truly gone before proceeding (a stale, still-readable count is
    // the exact failure mode we must prevent).
    if count_entry.get_password().is_ok() {
        eprintln!("[token_store] {key} count sentinel still present after delete; aborting");
        return false;
    }

    for (i, c) in chunks.iter().enumerate() {
        // base64 output is valid ASCII, so this never fails.
        let s = match std::str::from_utf8(c) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("[token_store] {key} chunk {i} not valid ascii (unexpected)");
                return false;
            }
        };
        let Some(e) = entry(&format!("{key}.{i}")) else {
            eprintln!("[token_store] failed to create keyring entry for {key} chunk {i}");
            return false;
        };
        if let Err(err) = e.set_password(s) {
            // Abort before committing the count. The sentinel was already cleared
            // above, so the loader fails closed (legacy/None → re-login) rather
            // than reading a half-written blob.
            eprintln!("[token_store] failed to save {key} chunk {i}: {err}");
            return false;
        }
    }

    // Commit point: flip the count to the new chunk total. Until this succeeds
    // there is no valid count (it was cleared above), so the loader fails closed.
    // Reuse the verified `count_entry`; a write failure here is a hard failure
    // (the caller must NOT believe the blob was persisted).
    if let Err(err) = count_entry.set_password(&chunks.len().to_string()) {
        eprintln!("[token_store] failed to save {key} count: {err}");
        return false;
    }

    // Post-commit cleanup (best-effort): remove orphan high-index chunks left by
    // a previously-larger blob. Failures here don't affect correctness — the
    // loader only reads chunks 0..count.
    for i in chunks.len()..MAX_CHUNKS {
        if let Some(e) = entry(&format!("{key}.{i}")) {
            let _ = e.delete_credential();
        }
    }
    true
}

/// Read a chunked blob written under `key`. Returns the raw bytes, or `None` if
/// the count sentinel is missing/invalid or any chunk is missing (fail closed).
#[cfg(windows)]
fn load_chunked(key: &str) -> Option<Vec<u8>> {
    let count_entry = entry(&format!("{key}.count"))?;
    let count_str = count_entry.get_password().ok()?;
    let n: usize = count_str.trim().parse().ok()?;
    if n == 0 || n > MAX_CHUNKS {
        return None;
    }
    let mut b64 = String::new();
    for i in 0..n {
        // Any missing chunk = corrupt/partial → fail closed.
        let part = entry(&format!("{key}.{i}"))?.get_password().ok()?;
        b64.push_str(&part);
    }
    STANDARD.decode(b64.as_bytes()).ok()
}

/// Remove all chunks + the count sentinel for `key` (best-effort).
#[cfg(windows)]
fn clear_chunked(key: &str) {
    for i in 0..MAX_CHUNKS {
        if let Some(e) = entry(&format!("{key}.{i}")) {
            let _ = e.delete_credential();
        }
    }
    if let Some(e) = entry(&format!("{key}.count")) {
        let _ = e.delete_credential();
    }
}

#[cfg(windows)]
pub fn save_tokens(tokens: &AuthTokens) {
    let json = match serde_json::to_string(tokens) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("[token_store] failed to serialize tokens: {e}");
            return;
        }
    };
    // save_chunked(CHUNK_PREFIX) writes exactly the historical
    // auth_tokens.count / auth_tokens.{N} layout.
    if !save_chunked(CHUNK_PREFIX, json.as_bytes()) {
        return;
    }
    // Sweep the legacy single-entry blob (pre-chunking plaintext).
    if let Some(e) = entry(USER) {
        let _ = e.delete_credential();
    }
}

/// Load tokens from the chunked format ONLY (count sentinel + chunks), with no
/// legacy single-entry fallback. Used by migration to confirm the chunked save
/// actually committed — `load_tokens` would otherwise satisfy a verify by
/// falling back to the very legacy plaintext entry the migration is about to
/// delete, even when the chunked write failed.
#[cfg(windows)]
fn load_chunked_tokens() -> Option<AuthTokens> {
    let bytes = load_chunked(CHUNK_PREFIX)?;
    let json = String::from_utf8(bytes).ok()?;
    serde_json::from_str(&json).ok()
}

#[cfg(windows)]
pub fn load_tokens() -> Option<AuthTokens> {
    // New chunked format: keyed on the presence of the count sentinel.
    if let Some(tokens) = load_chunked_tokens() {
        return Some(tokens);
    }

    // Legacy single-entry format (raw JSON, pre-chunking). Lets already-migrated
    // small-token users keep working; they heal to chunked on the next save.
    let json = entry(USER)?.get_password().ok()?;
    serde_json::from_str(&json).ok()
}

#[cfg(windows)]
pub fn clear_tokens() {
    // Best-effort sweep regardless of the recorded count, so orphan chunks from
    // a previously-larger token set are removed too. delete_credential on a
    // nonexistent entry returns Err — ignored.
    clear_chunked(CHUNK_PREFIX);
    if let Some(e) = entry(USER) {
        // legacy single-entry
        let _ = e.delete_credential();
    }
}

// ── Upload-session cookie storage ────────────────────────────────────────
//
// The native uploader's `/desktop-client/*` calls authenticate with a website
// session cookie (Laravel `web` guard), a DIFFERENT credential from the OAuth
// API tokens above. It is obtained via the in-app ESO Logs login webview and
// persisted here (encrypted in Credential Manager) so uploads survive restarts
// without re-login. Stored under its own `upload_session` key so it is managed
// independently of the auth tokens (clearing one never touches the other).

/// Credential key prefix for the upload-session cookie blob.
#[cfg(windows)]
const UPLOAD_SESSION_KEY: &str = "upload_session";

/// Persist the upload-session cookie header (the `Cookie:` value for esologs).
/// Returns `true` only if the cookie was actually committed to the credential
/// store; `false` means the caller must treat the session as non-durable (it
/// will not survive a restart). The fail-closed write never leaves a corrupt
/// blob, so a `false` here is safe — it just is not persisted.
#[cfg(windows)]
pub fn save_upload_session(cookie_header: &str) -> bool {
    save_chunked(UPLOAD_SESSION_KEY, cookie_header.as_bytes())
}

/// Load the persisted upload-session cookie header, if any.
#[cfg(windows)]
pub fn load_upload_session() -> Option<String> {
    let bytes = load_chunked(UPLOAD_SESSION_KEY)?;
    String::from_utf8(bytes).ok()
}

/// Remove the persisted upload-session cookie (e.g. on sign-out or `401`).
#[cfg(windows)]
pub fn clear_upload_session() {
    clear_chunked(UPLOAD_SESSION_KEY);
}

#[cfg(windows)]
pub fn migrate_from_store(app: &tauri::AppHandle) {
    use tauri_plugin_store::StoreExt;

    let store = match app.store("settings.json") {
        Ok(s) => s,
        Err(_) => return,
    };

    let val = match store.get("auth_tokens") {
        Some(v) => v,
        None => return, // nothing to migrate
    };

    let tokens: AuthTokens = match serde_json::from_value(val.clone()) {
        Ok(t) => t,
        Err(_) => return, // corrupt data, skip
    };

    // Use the chunked save path (the old inline single-entry write overflowed
    // the 2560-byte credential limit and left plaintext behind). Verify via the
    // CHUNKED loader (not load_tokens) before discarding the plaintext copy:
    // load_tokens would fall back to the legacy plaintext entry that still
    // exists here and report success even if the chunked write failed (save
    // clears the count sentinel before writing chunks), which would delete the
    // plaintext while no committed copy exists. load_chunked_tokens confirms the
    // chunked set actually committed, so a failed write never loses the tokens.
    save_tokens(&tokens);
    if load_chunked_tokens().is_some() {
        let _ = store.delete("auth_tokens");
    } else {
        eprintln!("[token_store] migration: verify failed, leaving plaintext intact");
    }
}

// ── Non-Windows stubs ───────────────────────────────────────────────────
// Kalpa is Windows-only; implement platform keychain here if needed.

#[cfg(not(windows))]
pub fn save_tokens(_tokens: &AuthTokens) {}

#[cfg(not(windows))]
pub fn load_tokens() -> Option<AuthTokens> {
    None
}

#[cfg(not(windows))]
pub fn clear_tokens() {}

#[cfg(not(windows))]
pub fn migrate_from_store(_app: &tauri::AppHandle) {}

#[cfg(not(windows))]
pub fn save_upload_session(_cookie_header: &str) -> bool {
    false
}

#[cfg(not(windows))]
pub fn load_upload_session() -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn clear_upload_session() {}
