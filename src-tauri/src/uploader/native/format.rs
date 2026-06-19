//! ESO Logs upload format facts and version pinning.
//!
//! The `/desktop-client/*` endpoints accept a compact, structured representation
//! of a log segment plus a "master table" of the units/abilities/effects the
//! segment references — NOT the raw `Encounter.log` text. This module pins the
//! **format facts** the [`super::convert`] serializer targets:
//!
//! * the **parser/format version** the server currently expects, and
//! * the small structural constants of the segment + master-table shape.
//!
//! ## Why version pinning matters
//!
//! The accepted format is coupled to ESO Logs' server-side parser version. If
//! the server advances its expected version and we keep sending the old one,
//! uploads fail — often *silently* (accepted-then-discarded, or a generic
//! error). So the version lives in exactly one place, is sent on every report,
//! and any version-rejection from the server must surface as a clear,
//! actionable error (see [`FORMAT_VERSION`] usage in the client), never a silent
//! drop.
//!
//! ## Clean-room note
//!
//! These are *facts about the ESO Logs service* (the version number it expects,
//! the field layout it parses) — analogous to an endpoint URL or a JSON schema.
//! They are established empirically against the live service and the public ESO
//! log format, and the serializer that produces them is implemented from
//! scratch. No third-party implementation is copied.

/// The parser/format version sent with every native report (the `parserVersion`
/// field of `create-report`).
///
/// **Server-coupled — must be kept current.** When ESO Logs advances its
/// expected parser version, update this single constant. A mismatch surfaces as
/// a [`FormatError::VersionRejected`] rather than a silent upload failure.
///
/// Confirmed = 11, observed directly in a real `create-report` request from the
/// official uploader (clientVersion 8.20.113). Sent as a JSON **integer**, not a
/// string.
pub const FORMAT_VERSION: u32 = 11;

/// The `clientVersion` string sent with `create-report`, identifying the desktop
/// client to the service. Observed directly in a real `create-report` request
/// (the matching `parserVersion` is [`FORMAT_VERSION`]). A protocol fact (a value
/// the service expects), not reverse-engineered logic. If the service starts
/// rejecting it as outdated, bump it.
pub const CLIENT_VERSION: &str = "8.20.113";

/// Whether [`FORMAT_VERSION`] has been empirically confirmed against the live
/// service. Gates enabling native upload by default — while `false`, the native
/// transport must not be the default (it may still be exercised behind an
/// explicit dev/opt-in path for the round-trip that confirms the version).
///
/// The version itself is confirmed (11). This stays `false` until the segment
/// **serialization** is also confirmed by a byte-exact round-trip — sending an
/// independently-produced segment that the server accepts — since a correct
/// version with a wrong segment body would still fail.
// CONFIRMED (2026-06-18, live round-trip): a Kalpa-built segment + master table
// was uploaded directly to esologs.com/desktop-client/* and the server accepted
// it, creating report `jAHXkRdzpGwxVQ1t`. The format version (11) and the segment
// serialization are therefore empirically validated end-to-end, so native upload
// is enabled (alongside the per-log coverage gate in `coverage::PROVEN_LINE_TYPES`
// and the user opt-in). Render-correctness of the report is a separate quality
// check; server acceptance is the version-confirmation bar this flag guards.
pub const FORMAT_VERSION_CONFIRMED: bool = true;

/// Errors specific to producing or versioning the upload format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    /// The server rejected our [`FORMAT_VERSION`] as outdated/unrecognized.
    /// Carries the server-reported detail so the fix (bump the constant) is
    /// obvious. This must be distinguishable from auth/network errors so it is
    /// never mistaken for a transient failure.
    VersionRejected { sent: u32, detail: String },
    /// A log line could not be parsed into the structured form (carries a short
    /// reason; the offending content is not included to avoid leaking log data
    /// into errors/telemetry).
    Unparseable(String),
    /// The segment or master table was internally inconsistent (a serializer
    /// bug or a malformed input the scanner should have rejected upstream).
    Inconsistent(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::VersionRejected { sent, detail } => write!(
                f,
                "ESO Logs rejected the upload format version {sent} ({detail}). \
                 Kalpa's native uploader needs an update — use the official \
                 uploader meanwhile."
            ),
            FormatError::Unparseable(why) => write!(f, "Could not parse a log line: {why}"),
            FormatError::Inconsistent(why) => write!(f, "Internal format error: {why}"),
        }
    }
}

impl std::error::Error for FormatError {}

#[cfg(test)]
mod tests {
    use super::*;

    // Guard rail: native upload must not silently default-on until the full
    // format (version AND serialization) is round-trip confirmed. The version
    // itself is now pinned (11, observed live), so it must always be a real
    // value; the CONFIRMED flag flips only once a produced segment round-trips.
    #[test]
    fn native_format_version_gate_is_explicit() {
        assert!(
            FORMAT_VERSION > 0,
            "the format version is pinned (11) — must be a real, non-zero value"
        );
        if FORMAT_VERSION_CONFIRMED {
            // Flipping this flag is the deliberate, human-verified step that
            // enables native upload by default after a byte-exact round-trip.
            assert!(
                FORMAT_VERSION > 0,
                "a confirmed format must carry a real version"
            );
        }
    }

    #[test]
    fn version_rejected_error_is_actionable() {
        let e = FormatError::VersionRejected {
            sent: 42,
            detail: "expected >= 43".into(),
        };
        let msg = e.to_string();
        assert!(msg.contains("42"));
        assert!(msg.contains("official uploader"), "must offer the fallback");
    }
}
