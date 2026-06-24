//! Secure token storage using Windows Credential Manager.
//!
//! Auth tokens (access_token, refresh_token, expires_at, user_id, user_name)
//! are stored as a JSON string in the OS credential store instead of plaintext
//! in the tauri-plugin-store settings file. Non-sensitive settings remain in the
//! store.

use crate::auth::AuthTokens;

const SERVICE: &str = "kalpa";
const USER: &str = "auth_tokens";

// ── Windows implementation (real credential manager) ────────────────────

#[cfg(windows)]
pub fn save_tokens(tokens: &AuthTokens) {
    let json = match serde_json::to_string(tokens) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("[token_store] failed to serialize tokens: {e}");
            return;
        }
    };
    let entry = match keyring::Entry::new(SERVICE, USER) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[token_store] failed to create keyring entry: {e}");
            return;
        }
    };
    if let Err(e) = entry.set_password(&json) {
        eprintln!("[token_store] failed to save tokens: {e}");
    }
}

#[cfg(windows)]
pub fn load_tokens() -> Option<AuthTokens> {
    let entry = keyring::Entry::new(SERVICE, USER).ok()?;
    let json = entry.get_password().ok()?;
    serde_json::from_str(&json).ok()
}

#[cfg(windows)]
pub fn clear_tokens() {
    let entry = match keyring::Entry::new(SERVICE, USER) {
        Ok(e) => e,
        Err(_) => return,
    };
    // delete_credential returns Err if no credential exists — that is fine
    let _ = entry.delete_credential();
}

#[cfg(windows)]
pub fn migrate_from_store(app: &tauri::AppHandle) {
    use tauri_plugin_store::StoreExt;

    // This is the FIRST opener of settings.json (runs in the setup hook, before
    // the webview loads). Register the store with autosave OFF so it stays off for
    // the whole app lifetime: plugin-store caches stores by path and ignores the
    // options passed by later openers, so the frontend reuses THIS instance. The
    // theme-migration writes there rely on fully explicit, atomic saves — a
    // debounced autosave could otherwise flush a partial multi-key batch and
    // strand a user mid-migration.
    let store = match app
        .store_builder("settings.json")
        .disable_auto_save()
        .build()
    {
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

    // Write to credential manager first; only delete from store on success.
    let json = match serde_json::to_string(&tokens) {
        Ok(j) => j,
        Err(_) => return,
    };
    let entry = match keyring::Entry::new(SERVICE, USER) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[token_store] migration: failed to create keyring entry: {e}");
            return; // leave store intact so tokens are not lost
        }
    };
    if let Err(e) = entry.set_password(&json) {
        eprintln!("[token_store] migration: failed to save tokens: {e}");
        return; // leave store intact
    }

    // Credential manager write succeeded — remove plaintext copy. autosave is off,
    // so persist the deletion explicitly — and atomically (crash-safe), via the
    // same path the rest of the app uses, instead of the plugin's truncate-write.
    let _ = store.delete("auth_tokens");
    let _ = crate::settings_store::flush(app);
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
