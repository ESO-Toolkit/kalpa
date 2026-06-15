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

/// Sentinel: decimal chunk count. Its presence marks the chunked format.
#[cfg(windows)]
const COUNT_KEY: &str = "auth_tokens.count";
/// Chunk N is stored under `auth_tokens.{N}`.
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

#[cfg(windows)]
pub fn save_tokens(tokens: &AuthTokens) {
    let json = match serde_json::to_string(tokens) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("[token_store] failed to serialize tokens: {e}");
            return;
        }
    };
    // base64 → pure ASCII so each char is exactly 2 UTF-16 bytes; chunk by byte
    // count, which (ASCII) equals char count.
    let b64 = STANDARD.encode(json.as_bytes());
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(CHUNK_LEN).collect();

    // Fail-closed ordering: invalidate the count sentinel FIRST, then write the
    // new chunks (overwriting same-index slots), then flip the count to the new
    // total as the single commit point, then sweep orphans. Invalidating first
    // is what makes a mid-write crash safe: overwriting a slot in place destroys
    // the old chunk there, so a crash between slots would otherwise leave the old
    // count pointing at a base64 splice of old+new chunks — which `load_tokens`
    // decodes to garbage. With the sentinel cleared up front, a mid-write crash
    // leaves NO valid count, so `load_tokens` cleanly falls back (legacy entry or
    // None → a re-login) instead of reading a corrupt token set. We cannot keep
    // the prior set recoverable without an atomic multi-key write (the credential
    // store has none); clean fail-closed is the correct minimal guarantee.
    if let Some(e) = entry(COUNT_KEY) {
        let _ = e.delete_credential();
    }
    for (i, c) in chunks.iter().enumerate() {
        // base64 output is valid ASCII, so this never fails.
        let s = match std::str::from_utf8(c) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("[token_store] chunk {i} not valid ascii (unexpected)");
                return;
            }
        };
        let Some(e) = entry(&format!("{CHUNK_PREFIX}.{i}")) else {
            eprintln!("[token_store] failed to create keyring entry for chunk {i}");
            return;
        };
        if let Err(err) = e.set_password(s) {
            // Abort before committing the count. The sentinel was already cleared
            // above, so load_tokens fails closed (legacy/None → re-login) rather
            // than reading a half-written set.
            eprintln!("[token_store] failed to save chunk {i}: {err}");
            return;
        }
    }

    // Commit point: flip the count to the new chunk total. Until this succeeds
    // there is no valid count (it was cleared above), so load_tokens fails closed.
    if let Some(e) = entry(COUNT_KEY) {
        if let Err(err) = e.set_password(&chunks.len().to_string()) {
            eprintln!("[token_store] failed to save token count: {err}");
            return;
        }
    }

    // Post-commit cleanup (best-effort): remove orphan high-index chunks left by
    // a previously-larger token set, plus the legacy single-entry blob. Failures
    // here don't affect correctness — load_tokens only reads chunks 0..count.
    for i in chunks.len()..MAX_CHUNKS {
        if let Some(e) = entry(&format!("{CHUNK_PREFIX}.{i}")) {
            let _ = e.delete_credential();
        }
    }
    if let Some(e) = entry(USER) {
        let _ = e.delete_credential();
    }
}

#[cfg(windows)]
pub fn load_tokens() -> Option<AuthTokens> {
    // New chunked format: keyed on the presence of the count sentinel.
    if let Some(count_entry) = entry(COUNT_KEY) {
        if let Ok(count_str) = count_entry.get_password() {
            let n: usize = count_str.trim().parse().ok()?;
            if n == 0 || n > MAX_CHUNKS {
                return None;
            }
            let mut b64 = String::new();
            for i in 0..n {
                // Any missing chunk = corrupt/partial → fail closed.
                let part = entry(&format!("{CHUNK_PREFIX}.{i}"))?.get_password().ok()?;
                b64.push_str(&part);
            }
            let bytes = STANDARD.decode(b64.as_bytes()).ok()?;
            let json = String::from_utf8(bytes).ok()?;
            return serde_json::from_str(&json).ok();
        }
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
    for i in 0..MAX_CHUNKS {
        if let Some(e) = entry(&format!("{CHUNK_PREFIX}.{i}")) {
            let _ = e.delete_credential();
        }
    }
    if let Some(e) = entry(COUNT_KEY) {
        let _ = e.delete_credential();
    }
    if let Some(e) = entry(USER) {
        // legacy single-entry
        let _ = e.delete_credential();
    }
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
    // the 2560-byte credential limit and left plaintext behind). Verify the
    // round-trip via load_tokens before discarding the plaintext copy, so a
    // failed write never loses the user's tokens.
    save_tokens(&tokens);
    if load_tokens().is_some() {
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
