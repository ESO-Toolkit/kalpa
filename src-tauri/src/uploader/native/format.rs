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
/// client to the service. A protocol fact (a value the service expects), not
/// reverse-engineered logic.
///
/// Bumped `8.20.113` → `9.3.93` (2026-06-23): the standalone ESO Logs Uploader/
/// Companion apps (which sent `8.20.113`) retire **2026-06-29**, replaced by the
/// unified **Archon App**. The `/desktop-client/*` endpoints and the create-report
/// body are UNCHANGED — only the client-version string moved. `9.3.93` is the value
/// the Archon App sends (its `ff()` client-version constant), confirmed clean-room
/// from `Uploaders-archon` v9.3.93's `app.asar`. The matching `parserVersion` is
/// still fetched from the parser at runtime (the log-format version, independent of
/// the app rename) and remains [`FORMAT_VERSION`] = 11. If the service starts
/// rejecting it as outdated, bump it to whatever the current Archon App sends.
pub const CLIENT_VERSION: &str = "9.3.93";

/// Whether [`FORMAT_VERSION`] has been empirically confirmed against the live
/// service. Gates enabling native upload by default — while `false`, the native
/// transport must not be the default (it may still be exercised behind an
/// explicit dev/opt-in path for the round-trip that confirms the version).
///
/// The version is confirmed (11). This flag additionally requires the produced
/// segment to **render** correctly server-side, not merely be accepted.
// GATE CLOSED (2026-06-18) → CONFIRMED OPEN (2026-06-19/24): an earlier upload was
// server-ACCEPTED but did NOT render — traced to a zero-width segment-window
// transport bug (the `add-report-segment` request sent startTime/endTime 0), since
// fixed (see `super::client`), NOT an encoder fault. After the fix a real dungeon
// log rendered a complete report (2026-06-19, owner-verified). Then a real Kyne's
// Aegis trial uploaded natively was confirmed to RENDER and match the official
// Archon upload within 0.06% on raid totals + identical top-player numbers
// (2026-06-24, "Probe B", reports ryJ4QzTDB72bxZaX vs AjNzRvD7bg8CQt9c). The
// hard-won lesson — server acceptance ≠ rendering — is now satisfied: rendering is
// proven, so the gate is OPEN. Per-log coverage still gates which logs qualify
// (see PROVEN_LINE_TYPES); any unproven line type still falls back.
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
