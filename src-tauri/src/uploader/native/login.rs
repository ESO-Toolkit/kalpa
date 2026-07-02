//! In-app ESO Logs login for the native upload session.
//!
//! The `/desktop-client/*` upload endpoints authenticate with a **website
//! session cookie** (Laravel `web` guard), not the OAuth API bearer token Kalpa
//! uses for the GraphQL API (an API bearer is empirically rejected with `401`).
//! The only no-password way to obtain that cookie is to let the user log in on
//! **ESO Logs' own login page inside an embedded webview that Kalpa owns**, then
//! read the resulting `laravel_session` cookie from that webview's cookie jar.
//!
//! Why this is the chosen approach (and why the obvious alternatives can't work):
//! * An external browser cannot hand Kalpa the cookie — `laravel_session` is
//!   `HttpOnly` and esologs.com-scoped, living in a separate process; OS/browser
//!   isolation blocks it (the very property that makes it secure).
//! * `document.cookie` (via webview `eval`) cannot read it either — it is
//!   `HttpOnly`, so it is absent from the DOM cookie string.
//! * Kalpa owns *its* webview's cookie jar, so it can read the cookie after the
//!   user authenticates on the real ESO Logs page. This is the standard embedded
//!   OAuth/login pattern (Spotify/Discord/Steam).
//!
//! Cookie reads use Tauri's runtime cookie store
//! ([`tauri::webview::WebviewWindow::cookies_for_url`]), which explicitly returns
//! `HttpOnly` cookies. On Windows those reads must NOT happen on a synchronous
//! command thread (they deadlock the WebView2), so this module performs them off
//! a blocking task while the login command is `async`.
//!
//! Clean-room: the login URL, the cookie name, and the upload session model are
//! facts about the ESO Logs website; the capture flow here is implemented from
//! scratch.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

use super::session::StoredSessionProvider;

/// Fixed label for the login webview window so a second login attempt reuses /
/// replaces the same window rather than stacking duplicates.
const LOGIN_WINDOW_LABEL: &str = "esologs-login";

/// The real ESO Logs login page. After a successful login the site sets the
/// `laravel_session` cookie for the esologs.com origin in this webview's jar.
const LOGIN_URL: &str = "https://www.esologs.com/login";

/// Origins to scope the cookie read to (only http/https URLs return cookies).
/// Both the canonical `www` host and the apex are read and merged: ESO Logs
/// serves on `www`, but reading both keeps capture correct if a login ever lands
/// on the apex host (which [`url_is_authenticated_view`] also accepts). A guest
/// cookie on either host is still gated out by the authenticated-view check, so
/// widening the read scope adds coverage without weakening the guest guard.
const ESOLOGS_ORIGINS: &[&str] = &["https://www.esologs.com", "https://esologs.com"];

/// The session cookie the upload endpoints authenticate with.
///
/// EMPIRICALLY CONFIRMED (2026-06-18, live login capture): ESO Logs (an RPGLogs /
/// Warcraft Logs platform site) names its web session cookie **`wcl_session`**,
/// NOT `laravel_session` (the earlier assumption — esologs is Laravel-based but
/// uses the `wcl_` prefix). The authenticated jar also carries `XSRF-TOKEN` and a
/// `remember_web_<hash>` persistent-auth cookie; all three are forwarded.
const SESSION_COOKIE_NAME: &str = "wcl_session";

/// Prefix of Laravel's "remember me" persistent-auth cookie (`remember_web_<hash>`).
/// Present in the authenticated jar; forwarded so the session is recognized even if
/// `wcl_session` alone is insufficient. Matched by prefix (the suffix is a hash).
const REMEMBER_COOKIE_PREFIX: &str = "remember_web_";

/// How long to wait for the user to complete the login before giving up.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// How often to poll the cookie jar / webview URL for login completion.
const POLL_INTERVAL: Duration = Duration::from_millis(750);

/// Tolerate this many *consecutive* cookie-read failures before giving up. Early
/// in a webview's life `cookies_for_url` can transiently fail before the page /
/// cookie jar settles; a single blip should not abort an otherwise-fine login.
const MAX_CONSECUTIVE_READ_ERRORS: u32 = 5;

/// Result of an in-app ESO Logs login attempt, shaped like [`crate::auth::AuthUser`]'s
/// persistence half so the frontend can reuse the same "session not persisted"
/// warning UX. We do not surface the ESO Logs account identity here — this login
/// establishes only the upload session cookie, and the user/name already comes
/// from the OAuth identity (`auth_get_user`).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadLoginResult {
    /// Whether the captured session cookie was durably persisted to the OS
    /// credential store. `false` means it is memory-only (a credential-store
    /// failure) and will not survive a restart — the UI warns, mirroring
    /// `AuthUser.sessionPersisted`.
    pub session_persisted: bool,
}

/// Why an in-app login attempt did not establish a session.
#[derive(Debug)]
pub enum LoginError {
    /// The login webview could not be created.
    WindowCreation(String),
    /// A login is already in progress (the sign-in window is already open); the
    /// existing window was focused instead of opening a second one.
    AlreadyInProgress,
    /// The user closed the login window before completing sign-in.
    WindowClosed,
    /// The login did not complete within [`LOGIN_TIMEOUT`].
    TimedOut,
    /// Reading the webview cookie jar failed.
    CookieRead(String),
}

impl std::fmt::Display for LoginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoginError::WindowCreation(d) => {
                write!(f, "Could not open the ESO Logs sign-in window: {d}")
            }
            LoginError::AlreadyInProgress => write!(
                f,
                "A sign-in window is already open — finish signing in there."
            ),
            LoginError::WindowClosed => write!(
                f,
                "Sign-in was cancelled — the ESO Logs window was closed before signing in."
            ),
            LoginError::TimedOut => write!(
                f,
                "Sign-in timed out. Open the ESO Logs sign-in window and try again."
            ),
            LoginError::CookieRead(d) => write!(f, "Could not read the ESO Logs session: {d}"),
        }
    }
}

impl std::error::Error for LoginError {}

/// Build the `Cookie:` header value from the cookies in the esologs jar.
///
/// The upload requests authenticate on the web session cookie
/// ([`SESSION_COOKIE_NAME`] = `wcl_session`). We also forward `XSRF-TOKEN` (CSRF)
/// and the `remember_web_<hash>` persistent-auth cookie when present — together
/// they are the authenticated jar the official uploader rides. Returns `None` if
/// the session cookie is not present yet (the user has not finished logging in),
/// which is the signal the poll loop keeps waiting on.
///
/// Pure over a cookie list so it is unit-testable without a live webview.
fn cookie_header_from(cookies: &[(String, String)]) -> Option<String> {
    let mut session: Option<&str> = None;
    let mut xsrf: Option<&str> = None;
    let mut remember: Option<(&str, &str)> = None;
    for (name, value) in cookies {
        if value.is_empty() {
            continue;
        }
        if name == SESSION_COOKIE_NAME {
            session = Some(value);
        } else if name == "XSRF-TOKEN" {
            xsrf = Some(value);
        } else if name.starts_with(REMEMBER_COOKIE_PREFIX) {
            remember = Some((name, value));
        }
    }
    // The session cookie is the required signal that login completed.
    let session = session?;
    let mut header = format!("{SESSION_COOKIE_NAME}={session}");
    if let Some(x) = xsrf {
        header.push_str("; XSRF-TOKEN=");
        header.push_str(x);
    }
    if let Some((name, value)) = remember {
        header.push_str("; ");
        header.push_str(name);
        header.push('=');
        header.push_str(value);
    }
    Some(header)
}

/// Whether the webview is showing an **authenticated** view (login complete),
/// inferred from its current URL having navigated away from the login page.
///
/// This is the critical guard against premature capture: Laravel issues a
/// `laravel_session` cookie to *anonymous* visitors on the `/login` page load
/// itself, so the mere presence of the cookie is NOT proof of login. ESO Logs
/// (like most web apps) redirects a freshly-authenticated user away from
/// `/login` to the site (dashboard/home/referrer). So we only accept the cookie
/// once the webview is on the esologs host AND no longer on a `/login` path — at
/// which point the `laravel_session` in the jar is the post-login (regenerated)
/// session, not the guest one.
///
/// Pure over a URL string so it is unit-testable. We require the host to still be
/// an esologs host (don't accept a cookie if the user navigated to an unrelated
/// site) and the path to not be the login (or register/password) flow.
fn url_is_authenticated_view(current_url: &str) -> bool {
    let Ok(url) = current_url.parse::<tauri::webview::Url>() else {
        return false;
    };
    let on_esologs = url
        .host_str()
        .is_some_and(|h| h == "www.esologs.com" || h == "esologs.com");
    if !on_esologs {
        return false;
    }
    // Still inside an auth flow (login / register / password reset) → not yet in.
    let path = url.path();
    let in_auth_flow = path.starts_with("/login")
        || path.starts_with("/register")
        || path.starts_with("/password")
        || path.starts_with("/oauth");
    !in_auth_flow
}

/// Subfolder (under the app data dir) holding the login webview's dedicated WebView2
/// profile. Kept in one const so both call sites agree.
const LOGIN_WEBVIEW_PROFILE_DIR: &str = "login-webview";

/// Dedicated WebView2 `data_directory` for the login window (B1).
///
/// The login window MUST NOT share the DEFAULT WebView2 profile with the main app
/// window. No `data_directory` is set on the main window, so absent this the login
/// window inherits the shared default profile — and [`clear_login_webview_data`]'s
/// `clear_all_browsing_data()` (explicit sign-out) clears the WHOLE profile it runs
/// against. On the shared profile that wipes the MAIN app's `localStorage` (the theme
/// pre-paint mirror → a one-time theme flash on the next launch, plus uploader prefs)
/// on every sign-out. Giving the login window its OWN profile scopes both the login
/// cookies and the sign-out clear to just that profile, leaving the main app untouched.
///
/// [`run_login`] and [`clear_login_webview_data`] MUST pass the SAME path so the window
/// that stores the cookies and the window whose profile is cleared are the same profile.
/// Returns `None` if the app data dir cannot be resolved; callers then handle that
/// explicitly (login falls back to the shared default so sign-in still works; sign-out
/// SKIPS the clear rather than wipe the shared profile).
///
/// Windows-only in effect: `WebviewWindowBuilder::data_directory` is a no-op on other
/// platforms, but Kalpa ships Windows only.
fn login_webview_data_dir<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .ok()?
        .join(LOGIN_WEBVIEW_PROFILE_DIR);
    // Best-effort create so the path exists before WebView2 opens it; WebView2 would
    // create it too, but doing it here keeps both call sites consistent.
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

/// Drive an in-app ESO Logs login: open (or reuse) the login webview, wait for
/// the user to authenticate, capture the `laravel_session` cookie, and persist
/// it via the shared [`StoredSessionProvider`]. Returns once a session cookie is
/// captured (success), the window is closed (cancel), or the timeout elapses.
///
/// `app` is the Tauri app handle (to create the window and read cookies);
/// `provider` is the shared, managed session provider the upload path reads from
/// — capturing here makes the cookie immediately usable for an upload.
pub async fn run_login<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    provider: &StoredSessionProvider,
) -> Result<UploadLoginResult, LoginError> {
    // Concurrency guard: if a login window already exists (a prior attempt still
    // running, or a double-invoke), don't build a second one — the fixed label
    // would make `build()` error and the two poll loops would race the same
    // cookie jar. Focus the existing window and let the in-flight login own it.
    if let Some(existing) = app.get_webview_window(LOGIN_WINDOW_LABEL) {
        let _ = existing.set_focus();
        return Err(LoginError::AlreadyInProgress);
    }

    let url = WebviewUrl::External(
        LOGIN_URL
            .parse()
            .map_err(|e| LoginError::WindowCreation(format!("bad login URL: {e}")))?,
    );
    let mut builder = WebviewWindowBuilder::new(&app, LOGIN_WINDOW_LABEL, url)
        .title("Sign in to ESO Logs")
        .inner_size(520.0, 720.0)
        .resizable(true)
        .focused(true);
    // Isolate the login webview's WebView2 profile from the main app window's (B1). The
    // cookies land in — and sign-out clears — this dedicated profile, never the shared
    // default (see `login_webview_data_dir`). If the app data dir can't be resolved we
    // fall back to the shared default so sign-in still works (sign-out then can't scope
    // its clear, but a blocked login is worse than a wider clear).
    if let Some(dir) = login_webview_data_dir(&app) {
        builder = builder.data_directory(dir);
    }
    let window = builder
        .build()
        .map_err(|e| LoginError::WindowCreation(e.to_string()))?;

    // Run the poll loop, ensuring the login window is closed on EVERY exit path
    // (success or error) — a left-open window would orphan an authenticated
    // webview and block the next login (the concurrency guard above would see it
    // and refuse). The helper returns the outcome; we close, then propagate.
    let outcome = poll_for_session(&app, &window, provider).await;
    let _ = window.close();
    outcome
}

/// The poll loop: wait until the webview is on an authenticated (post-login)
/// view AND carries a `laravel_session`, then capture + persist it. Separated
/// from [`run_login`] so the caller can guarantee window cleanup on all paths.
async fn poll_for_session<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    window: &tauri::WebviewWindow<R>,
    provider: &StoredSessionProvider,
) -> Result<UploadLoginResult, LoginError> {
    // Pre-parse the cookie-read origins once (both www + apex; see ESOLOGS_ORIGINS).
    let origins: Vec<tauri::webview::Url> = ESOLOGS_ORIGINS
        .iter()
        .map(|o| {
            o.parse::<tauri::webview::Url>()
                .map_err(|e| LoginError::CookieRead(format!("bad origin URL: {e}")))
        })
        .collect::<Result<_, _>>()?;

    let start = Instant::now();
    let mut consecutive_read_errors: u32 = 0;
    loop {
        // The user closing the window is a cancel — stop polling a dead webview.
        if app.get_webview_window(LOGIN_WINDOW_LABEL).is_none() {
            return Err(LoginError::WindowClosed);
        }

        // Only accept a cookie once the webview has navigated to an authenticated
        // view (off `/login`). This is the guard against capturing the anonymous
        // guest session the site sets on the login page load itself. `url()` is
        // cheap and safe to call here. If it errors, treat as "not yet ready".
        let authenticated_view = window
            .url()
            .map(|u| url_is_authenticated_view(u.as_str()))
            .unwrap_or(false);

        if authenticated_view {
            // Read cookies off a blocking task: on Windows `cookies_for_url` must
            // not run on a synchronous command/event thread (it deadlocks the
            // WebView2). Cloning the window handle is cheap (an Arc'd dispatcher).
            // Read every origin and merge — a session cookie on either host is
            // accepted.
            let win = window.clone();
            let origins_for_read = origins.clone();
            let read = tauri::async_runtime::spawn_blocking(move || {
                let mut merged: Vec<(String, String)> = Vec::new();
                for origin in &origins_for_read {
                    let cookies = win
                        .cookies_for_url(origin.clone())
                        .map_err(|e| e.to_string())?;
                    merged.extend(
                        cookies
                            .into_iter()
                            .map(|c| (c.name().to_string(), c.value().to_string())),
                    );
                }
                Ok::<_, String>(merged)
            })
            .await;

            match read {
                Ok(Ok(cookies)) => {
                    consecutive_read_errors = 0;
                    if let Some(header) = cookie_header_from(&cookies) {
                        // Captured a post-login session. Persist via the shared
                        // provider so the upload path can use it immediately, and
                        // report durability to the UI.
                        let persisted = provider.store(header);
                        return Ok(UploadLoginResult {
                            session_persisted: persisted,
                        });
                    }
                    // On an authed view but no session cookie yet (jar still
                    // settling) — keep waiting.
                }
                Ok(Err(e)) => {
                    // Tolerate a few transient read failures before giving up;
                    // early in the webview's life the jar may not be ready.
                    consecutive_read_errors += 1;
                    if consecutive_read_errors >= MAX_CONSECUTIVE_READ_ERRORS {
                        return Err(LoginError::CookieRead(e));
                    }
                }
                // The blocking task itself failed (panicked/cancelled) — surface
                // it (not transient).
                Err(e) => return Err(LoginError::CookieRead(format!("cookie task failed: {e}"))),
            }
        }

        // True wall-clock bound: counts time spent in cookie reads too, so a slow
        // WebView2 can't extend the login past the timeout.
        if start.elapsed() >= LOGIN_TIMEOUT {
            return Err(LoginError::TimedOut);
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Clear the login webview's persistent cookie jar on an EXPLICIT sign-out (B1).
///
/// WebView2 persists the login webview's cookies (`wcl_session`, the long-lived
/// `remember_web_*` "remember me" cookie) in the login window's WebView2 profile.
/// Clearing only the stored upload-session copy (`SessionProvider::invalidate`) leaves a
/// valid ESO Logs web session on disk, so the next "Sign in" auto-completes with zero
/// interaction — sign-out would be merely cosmetic. This clears that profile's browsing
/// data so a re-sign-in shows the real login form.
///
/// The clear is scoped to the login window's DEDICATED profile
/// ([`login_webview_data_dir`]): `clear_all_browsing_data()` clears the whole profile it
/// runs against, so the login window must not share the main app window's default
/// profile (else sign-out would wipe the main app's `localStorage` — the theme pre-paint
/// mirror and uploader prefs). Note: users who signed in before this isolation landed
/// have their old session in the shared default profile, so their first sign-in after
/// the update shows the login form again (one-time). Their captured upload token in the
/// OS credential store is a separate copy and is unaffected.
///
/// Get-or-build the login window (built HIDDEN on a blank page on the SAME dedicated
/// profile — no network, no re-login navigation — if it isn't already open), clear all
/// browsing data for that profile, then close it. Best-effort: every failure is
/// swallowed (the stored-session invalidation already happened, which is the load-bearing
/// half). If the dedicated profile path can't be resolved when a rebuild is needed, the
/// clear is SKIPPED rather than run against the shared default profile.
///
/// **Invariant**: this is attached ONLY to the explicit sign-out command. The
/// mid-upload `SessionProvider::invalidate` (a 401/419 during an upload) must NOT reach
/// here — clearing the jar mid-upload would break the reauth pause→re-login UX. This fn
/// has no other caller (see `uploader_logout_esologs`).
pub fn clear_login_webview_data<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let window = match app.get_webview_window(LOGIN_WINDOW_LABEL) {
        Some(w) => w,
        None => {
            // Build a hidden window on `about:blank` purely so we have a handle whose
            // profile we can clear — no navigation to esologs, so nothing can re-set a
            // cookie in the race between build and clear.
            let Ok(url) = "about:blank".parse::<tauri::webview::Url>() else {
                return;
            };
            // Rebuild on the SAME dedicated profile `run_login` used, so the clear scopes
            // to the login cookies only. If that path can't be resolved, SKIP the clear
            // rather than build a shared-default-profile window whose clear would wipe the
            // main app's browsing data (the exact regression this fixes).
            let Some(dir) = login_webview_data_dir(app) else {
                return;
            };
            match WebviewWindowBuilder::new(app, LOGIN_WINDOW_LABEL, WebviewUrl::External(url))
                .visible(false)
                .data_directory(dir)
                .build()
            {
                Ok(w) => w,
                Err(_) => return,
            }
        }
    };
    let _ = window.clear_all_browsing_data();
    let _ = window.close();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_header_requires_session_cookie() {
        // No wcl_session → no header (user not logged in yet). XSRF + remember
        // alone (the anonymous/guest jar) must NOT count as a session.
        assert_eq!(
            cookie_header_from(&[("XSRF-TOKEN".into(), "abc".into())]),
            None
        );
        // Empty session value is treated as absent.
        assert_eq!(
            cookie_header_from(&[(SESSION_COOKIE_NAME.into(), "".into())]),
            None
        );
        // Empty list → none.
        assert_eq!(cookie_header_from(&[]), None);
    }

    #[test]
    fn cookie_header_includes_session_xsrf_and_remember() {
        let header = cookie_header_from(&[
            ("wcl_session".into(), "sess123".into()),
            ("XSRF-TOKEN".into(), "tok456".into()),
            ("remember_web_deadbeef".into(), "rmb789".into()),
            ("_ga".into(), "ignored".into()),
        ])
        .expect("session present");
        assert!(header.contains("wcl_session=sess123"));
        assert!(header.contains("XSRF-TOKEN=tok456"));
        assert!(header.contains("remember_web_deadbeef=rmb789"));
        assert!(!header.contains("ignored"));
    }

    #[test]
    fn cookie_header_session_only_when_no_xsrf() {
        let header = cookie_header_from(&[("wcl_session".into(), "sess123".into())])
            .expect("session present");
        assert_eq!(header, "wcl_session=sess123");
    }

    #[test]
    fn login_page_is_not_an_authenticated_view() {
        // The guest session cookie is set on these pages — capturing there would
        // be the premature-capture bug. None must count as "logged in".
        assert!(!url_is_authenticated_view("https://www.esologs.com/login"));
        assert!(!url_is_authenticated_view(
            "https://www.esologs.com/login?redirect=/"
        ));
        assert!(!url_is_authenticated_view(
            "https://www.esologs.com/register"
        ));
        assert!(!url_is_authenticated_view(
            "https://www.esologs.com/password/reset"
        ));
        assert!(!url_is_authenticated_view(
            "https://www.esologs.com/oauth/authorize"
        ));
    }

    #[test]
    fn post_login_pages_are_authenticated_views() {
        // After login ESO Logs lands the user on a non-auth page on its host.
        assert!(url_is_authenticated_view("https://www.esologs.com/"));
        assert!(url_is_authenticated_view(
            "https://www.esologs.com/user/reports"
        ));
        assert!(url_is_authenticated_view(
            "https://www.esologs.com/character/id/123"
        ));
        // Apex host (no www) also counts as esologs.
        assert!(url_is_authenticated_view("https://esologs.com/dashboard"));
    }

    #[test]
    fn off_site_urls_are_not_authenticated_views() {
        // If the webview is on an unrelated host (e.g. an OAuth provider, or the
        // user navigated away), do not accept a cookie.
        assert!(!url_is_authenticated_view("https://accounts.google.com/"));
        assert!(!url_is_authenticated_view("https://evil.example.com/login"));
        assert!(!url_is_authenticated_view("not a url"));
        // A look-alike host must not match (suffix check is exact, not contains).
        assert!(!url_is_authenticated_view("https://esologs.com.evil.com/"));
        assert!(!url_is_authenticated_view("https://notesologs.com/"));
    }

    #[test]
    fn login_errors_are_user_facing() {
        assert!(LoginError::WindowClosed.to_string().contains("cancelled"));
        assert!(LoginError::TimedOut.to_string().contains("timed out"));
        assert!(LoginError::CookieRead("io".into())
            .to_string()
            .contains("io"));
        assert!(LoginError::WindowCreation("x".into())
            .to_string()
            .contains("open"));
        assert!(LoginError::AlreadyInProgress
            .to_string()
            .contains("already open"));
    }
}
