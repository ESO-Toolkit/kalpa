//! Native ESO Logs upload client.
//!
//! Speaks the `/desktop-client/*` upload protocol directly so Kalpa owns the
//! whole upload lifecycle (start, progress, clean in-process stop) instead of
//! handing off to the official uploader. This is opt-in and sits behind the
//! [`super::transport`] seam alongside the official-uploader fallback.
//!
//! Layering (each depends only on the ones above it):
//!
//! * [`format`]  — pinned format facts + version, and format-specific errors.
//! * [`convert`] — raw log lines → the structured segment + master-table form.
//! * [`session`] — the [`session::SessionProvider`] auth seam (cookie session).
//! * [`client`]  — the report lifecycle calls (create / add-segment /
//!   set-master-table / terminate), cancellable, driven by a session provider.
//!
//! The converter is the bulk of the work and is built test-first against golden
//! files (`convert` + `testdata/`). The client and session pieces are thin by
//! comparison. Built clean-room from protocol facts; no third-party code copied.

pub mod client;
pub mod convert;
pub mod coverage;
pub mod differential;
pub mod encode;
pub mod format;
pub mod serialize;
pub mod session;
