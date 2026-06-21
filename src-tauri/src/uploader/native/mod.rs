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

pub mod a_counter;
pub mod client;
pub mod convert;
pub mod coverage;
pub mod differential;
pub mod encode;
pub mod events;
pub mod format;
/// Incremental live-streaming index maps (the L7 perf fix) — maintains the
/// cumulative actor/ability index maps in O(1)/line instead of re-walking the whole
/// buffer. Proven byte-identical to the [`encode`] re-walk oracle.
pub mod incremental;
/// Native live-streaming upload driver: holds one report open and pushes
/// fights-segments incrementally. Gated ON only when the user opts in, the format is
/// confirmed, and a session exists (see `transport::assess_native_live_routing`);
/// otherwise the official-uploader handoff runs. The debug round-trip command + the
/// synthetic `FileTail`/`ScriptedTail` test seam inside stay `#[cfg(debug_assertions)]`.
/// See `docs/native-live-streaming-spike-FINDINGS.md`.
pub mod live;
pub mod login;
/// Crash-recovery for unterminated native live reports (L2): persists a
/// `{reportCode, segmentId}` breadcrumb and best-effort terminates leftover codes on
/// next launch.
pub mod orphans;
pub mod serialize;
pub mod session;
pub mod zip_segment;
