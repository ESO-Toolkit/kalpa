//! Pack Hub: the `kalpa-pack-hub` Cloudflare Worker client.
//!
//! This module owns everything that talks to the dedicated Pack Hub worker
//! (`https://kalpa-pack-hub.eso-toolkit.workers.dev`) and its sibling share
//! worker: browsing/publishing/voting on packs, install-count tracking,
//! private share codes, `.esopack` file export/import, and the roster-pack
//! lookup used by the `kalpa://install-pack/{id}` deep link.
//!
//! Deliberately NOT here: the ESO Logs OAuth session (`auth_login`,
//! `auth_logout`, `auth_get_user`, `auth_cached_user`, `auth_cancel_login`,
//! and their `AuthState`/token-persistence helpers) in `crate::commands`.
//! That session is shared with the ESO Logs uploader and the header account
//! chip — it is not a Pack Hub concept, even though several commands in this
//! module borrow its bearer token to call the worker as a signed-in user.
//! Those commands reach the shared helpers via
//! `crate::commands::{save_auth_tokens, clear_session_if_rejected,
//! clear_auth_and_upload_sessions}`, which stay `pub(crate)` in `commands.rs`
//! for exactly this purpose.
//!
//! The auth session is not the only borrow, though, and reading it as the only
//! one is how a "pure move" starts drifting. `require_allowed_path`,
//! `validate_name` and `ensure_eso_not_running_for_settings_write` also come
//! from `crate::commands`: they are the AddOns-path gate, the folder-name gate
//! and the running-client refusal that every SavedVariables command shares, so
//! the `.esopack` settings commands here must use the same ones rather than
//! grow private copies that can disagree.
//!
//! Module layout:
//! * [`commands`] — Tauri command handlers, HTTP clients, and IPC types.
//! * [`addon_search`] — client for the worker's ESOUI full-text addon index,
//!   which backs Discover search and falls back to `crate::esoui` when the
//!   index cannot answer.

pub mod addon_search;
pub mod commands;
