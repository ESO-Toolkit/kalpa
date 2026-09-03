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
//! Those commands reach the shared helpers via `crate::commands::{
//! save_auth_tokens, is_session_rejection, clear_session_if_rejected,
//! clear_auth_and_upload_sessions}`, which stay `pub(crate)` in
//! `commands.rs` for exactly this purpose.
//!
//! Module layout:
//! * [`commands`] — Tauri command handlers, HTTP clients, and IPC types.

pub mod commands;
