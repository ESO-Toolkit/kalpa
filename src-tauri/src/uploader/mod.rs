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
/// Native `/desktop-client/*` upload client (opt-in; official handoff is the
/// fallback). Built clean-room from protocol facts. See [`native`].
pub mod native;
pub mod scanner;
pub mod splitter;
/// Shared byte-offset tail primitives (`read_range` + loop tuning constants)
/// used by both [`watcher`] and the native live-streaming driver.
pub mod tail_io;
pub mod transport;
pub mod types;
pub mod watcher;
