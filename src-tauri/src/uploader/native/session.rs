//! Authenticated session for the native ESO Logs upload client.
//!
//! The `/desktop-client/*` upload endpoints authenticate with a **website
//! session cookie** (Laravel `web` guard + CSRF), not the OAuth API bearer token
//! Kalpa uses for the GraphQL API — empirically confirmed by a `401` when the
//! bearer is presented. A session is established by logging into the website and
//! persisting the resulting cookie jar, then sending it on every upload request.
//!
//! This module owns the *seam*, not a specific login implementation: the
//! [`SessionProvider`] trait is the single point the rest of the native client
//! depends on. Everything downstream (the protocol client, the transport) is
//! written against this trait, so the concrete login flow can be developed,
//! swapped, or stored differently without touching the upload logic.
//!
//! Cookie persistence reuses the existing secure storage path (Windows
//! Credential Manager via `token_store`), so a session survives restarts without
//! re-login, and is never written to plaintext on disk.

use std::fmt;

/// A handle to an authenticated website session usable for upload requests.
///
/// The concrete value is whatever the login flow produces (a serialized cookie
/// jar); the rest of the client only needs to (a) attach it to a request and
/// (b) know whether it is still usable. Kept deliberately opaque so the upload
/// code never inspects or logs the raw session secret.
#[derive(Clone)]
pub struct Session {
    /// Serialized cookie jar (e.g. the `Cookie` header value) for the esologs
    /// origin. Opaque to callers; never logged or surfaced.
    cookie_header: String,
}

impl Session {
    /// Build a session from a serialized cookie header value. The caller (the
    /// login flow) is responsible for producing a valid jar; this type only
    /// carries it.
    pub fn from_cookie_header(cookie_header: impl Into<String>) -> Self {
        Self {
            cookie_header: cookie_header.into(),
        }
    }

    /// The `Cookie` header value to attach to upload requests. Crate-internal so
    /// only the protocol client reads it. (Consumed by the client's wire-send,
    /// which is pinned to the confirmed format before it is filled in.)
    #[allow(dead_code)]
    pub(crate) fn cookie_header(&self) -> &str {
        &self.cookie_header
    }

    /// Whether the session carries any cookies at all. A *true* result does not
    /// guarantee the server still accepts it (only a request can prove that) —
    /// it only rules out the empty case.
    pub fn is_nonempty(&self) -> bool {
        !self.cookie_header.trim().is_empty()
    }
}

// Never leak the cookie value through Debug (it is a credential).
impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("cookie_header", &"<redacted>")
            .field("nonempty", &self.is_nonempty())
            .finish()
    }
}

/// Why a session could not be provided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    /// No stored session and no way to establish one without user action
    /// (e.g. the user has not completed the website login).
    NotAuthenticated,
    /// A session existed but the server rejected it (expired/invalid); the user
    /// must re-establish it.
    Expired,
    /// The login/refresh attempt failed for an operational reason (network, IO,
    /// storage). Carries a human-readable detail.
    Failed(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::NotAuthenticated => {
                write!(f, "Not signed in to ESO Logs for uploading.")
            }
            SessionError::Expired => {
                write!(f, "Your ESO Logs upload session expired — sign in again.")
            }
            SessionError::Failed(d) => write!(f, "Could not establish an upload session: {d}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// Supplies an authenticated session for upload requests.
///
/// This is the seam the login flow plugs into. The protocol client calls
/// [`SessionProvider::session`] to obtain a usable [`Session`] and, on a server
/// rejection mid-upload, [`SessionProvider::invalidate`] so a stale session is
/// not reused. Implementations are responsible for persistence and refresh; the
/// client makes no assumptions about how the session was obtained.
pub trait SessionProvider: Send + Sync {
    /// Return a currently-usable session, establishing or refreshing one if
    /// necessary. Returns [`SessionError::NotAuthenticated`] when that requires
    /// user action the provider cannot perform headlessly.
    fn session(&self) -> Result<Session, SessionError>;

    /// Mark the current session invalid (e.g. the server returned `401`/`419`
    /// mid-upload) so the next [`SessionProvider::session`] re-establishes it.
    fn invalidate(&self);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_redacts_cookie_in_debug() {
        let s = Session::from_cookie_header("laravel_session=supersecret; XSRF-TOKEN=abc");
        let dbg = format!("{s:?}");
        assert!(
            !dbg.contains("supersecret"),
            "Debug must not leak the cookie secret: {dbg}"
        );
        assert!(dbg.contains("redacted"));
        assert!(dbg.contains("nonempty: true"));
    }

    #[test]
    fn empty_session_is_detected() {
        assert!(!Session::from_cookie_header("   ").is_nonempty());
        assert!(Session::from_cookie_header("laravel_session=x").is_nonempty());
    }

    #[test]
    fn session_error_messages_are_user_facing() {
        assert!(SessionError::NotAuthenticated
            .to_string()
            .contains("Not signed in"));
        assert!(SessionError::Expired.to_string().contains("expired"));
        assert!(SessionError::Failed("io".into()).to_string().contains("io"));
    }
}
