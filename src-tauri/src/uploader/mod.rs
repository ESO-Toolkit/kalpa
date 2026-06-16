//! ESO Logs uploader.
//!
//! Locates ESO combat logs, understands their structure (sessions and fights)
//! by streaming over byte offsets, can split oversized logs to disk, and hands
//! prepared logs to the **official ESO Logs uploader** for the actual upload —
//! Kalpa never speaks the private `/desktop-client/` protocol itself (see
//! [`transport`] for the rationale).
//!
//! Module layout:
//! * [`types`]     — IPC-serialized data types.
//! * [`discovery`] — find the `Logs` directory and enumerate log files.
//! * [`scanner`]   — streaming session/fight boundary detection.
//! * [`splitter`]  — extract sessions/fights to standalone `.log` files.
//! * [`transport`] — the [`transport::LogUploadTransport`] abstraction.
//! * [`watcher`]   — live tailing + per-fight dispatch.
//! * [`history`]   — persistent upload history.
//! * [`commands`]  — Tauri command handlers and managed state.

pub mod commands;
pub mod discovery;
pub mod history;
/// Debug-only auth spike for a possible future native upload path. Compiled out
/// of release builds; delete once that direction is settled.
#[cfg(debug_assertions)]
pub mod native_probe;
pub mod scanner;
pub mod splitter;
pub mod transport;
pub mod types;
pub mod watcher;
