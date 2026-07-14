//! Upload-session cookie storage — a Tauri-free subset of the production
//! `src-tauri/src/token_store.rs`, kept BYTE-COMPATIBLE with it.
//!
//! The native ESO Logs uploader authenticates its `/desktop-client/*` calls with
//! a website session cookie (Laravel `web` guard). The production app stores that
//! cookie in the Windows Credential Manager under service `kalpa`, key
//! `upload_session`, in a fixed chunked+base64 layout. This module reproduces the
//! SAME service name, key, chunk size, and encoding so a cookie the production
//! Kalpa app persisted is readable here (and vice-versa) — that is what lets the
//! Slint shell upload natively for a user who signed in via the main app.
//!
//! Only the upload-session functions are ported (the native uploader needs no
//! auth tokens), so this drops the `crate::auth` + `tauri` dependencies the full
//! file carries.

const SERVICE: &str = "kalpa";

#[cfg(windows)]
use base64::{engine::general_purpose::STANDARD, Engine};

/// Upper bound on chunks read/swept (sanity cap; ~64 KB of base64 max).
#[cfg(windows)]
const MAX_CHUNKS: usize = 64;
/// Base64 chars per chunk → 2000 UTF-16 bytes, a ~28% margin under the 2560 cap.
#[cfg(windows)]
const CHUNK_LEN: usize = 1000;
/// Credential key for the upload-session cookie blob.
#[cfg(windows)]
const UPLOAD_SESSION_KEY: &str = "upload_session";

#[cfg(windows)]
fn entry(user: &str) -> Option<keyring::Entry> {
    keyring::Entry::new(SERVICE, user).ok()
}

/// Write `data` under `key` using the same fail-closed chunked scheme as
/// production (`{key}.count` sentinel + `{key}.{N}` chunks, base64 payload).
#[cfg(windows)]
fn save_chunked(key: &str, data: &[u8]) -> bool {
    let count_key = format!("{key}.count");
    let b64 = STANDARD.encode(data);
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(CHUNK_LEN).collect();

    let Some(count_entry) = entry(&count_key) else {
        return false;
    };
    match count_entry.delete_credential() {
        Ok(()) => {}
        Err(keyring::Error::NoEntry) => {}
        Err(_) => return false,
    }
    if count_entry.get_password().is_ok() {
        return false;
    }

    for (i, c) in chunks.iter().enumerate() {
        let Ok(s) = std::str::from_utf8(c) else {
            return false;
        };
        let Some(e) = entry(&format!("{key}.{i}")) else {
            return false;
        };
        if e.set_password(s).is_err() {
            return false;
        }
    }

    if count_entry.set_password(&chunks.len().to_string()).is_err() {
        return false;
    }

    for i in chunks.len()..MAX_CHUNKS {
        if let Some(e) = entry(&format!("{key}.{i}")) {
            let _ = e.delete_credential();
        }
    }
    true
}

/// Read a chunked blob written under `key` (fail-closed on any missing chunk).
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

/// Persist the upload-session cookie header. Returns `true` only if committed.
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

/// Remove the persisted upload-session cookie (sign-out / `401`).
#[cfg(windows)]
pub fn clear_upload_session() {
    clear_chunked(UPLOAD_SESSION_KEY);
}

// ── Non-Windows stubs (parity with production's cfg layout) ─────────────────

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
