//! Native ESO Logs sign-in for the Slint shell.
//!
//! The `/desktop-client/*` upload endpoints authenticate with the ESO Logs
//! website session cookie (`wcl_session`), which is `HttpOnly` and therefore
//! unreadable by an external browser or `document.cookie`. The production Tauri
//! app captures it by letting the user log in on ESO Logs' own page inside a
//! WebView Kalpa owns, then reading the cookie from that WebView's jar.
//!
//! The WebView-less Slint shell has no such surface, so this module opens a
//! self-contained wry (WebView2) sign-in window, polls the cookie jar until the
//! user is on an authenticated esologs page carrying `wcl_session`, builds the
//! same `Cookie:` header the production `native::login` builds, and hands it back.
//! The pure header/URL logic is replicated from
//! `src-tauri/src/uploader/native/login.rs`.
//!
//! A WebView2 event loop hard-faults when spun up alongside Slint's winit loop in
//! one process, so the login runs OUT-OF-PROCESS: the parent spawns
//! `<exe> --esologs-login`, which calls [`run_login_subprocess`] on its own main
//! thread and returns the cookie via stdout. See `run_uploader_sign_in` in
//! `main.rs`.

use std::time::{Duration, Instant};

/// CLI flag that puts the process into sign-in-only mode (see `run_login_subprocess`).
pub const LOGIN_SUBPROCESS_FLAG: &str = "--esologs-login";
/// Marker printed to stdout immediately before the captured cookie header, so the
/// parent can find it even if the WebView emits other stdout noise.
pub const COOKIE_STDOUT_MARKER: &str = "ESOLOGS_COOKIE:";

const LOGIN_URL: &str = "https://www.esologs.com/login";
const COOKIE_ORIGIN: &str = "https://www.esologs.com";
const SESSION_COOKIE_NAME: &str = "wcl_session";
const REMEMBER_COOKIE_PREFIX: &str = "remember_web_";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const POLL_INTERVAL: Duration = Duration::from_millis(750);

/// Outcome of an in-app ESO Logs sign-in attempt.
pub enum LoginOutcome {
    /// Sign-in succeeded; carries the `Cookie:` header for the upload session.
    Success(String),
    /// The user closed the window before finishing.
    Cancelled,
    /// The user did not finish within [`LOGIN_TIMEOUT`].
    TimedOut,
    /// The window/webview could not be created or driven.
    Error(String),
}

/// Build the `Cookie:` header from a captured (name, value) cookie list —
/// `wcl_session` (required) plus `XSRF-TOKEN` and `remember_web_<hash>` when
/// present. Returns `None` until the session cookie appears (the poll signal).
/// Pure — unit-tested without a live webview.
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

/// Whether the webview is on an authenticated (post-login) esologs view — the
/// guard against capturing the guest `wcl_session` issued on the `/login` page
/// itself. Requires an esologs host that is no longer inside an auth flow. Pure.
fn url_is_authenticated_view(current_url: &str) -> bool {
    let Ok(url) = url::Url::parse(current_url) else {
        return false;
    };
    let on_esologs = url
        .host_str()
        .is_some_and(|h| h == "www.esologs.com" || h == "esologs.com");
    if !on_esologs {
        return false;
    }
    let path = url.path();
    let in_auth_flow = path.starts_with("/login")
        || path.starts_with("/register")
        || path.starts_with("/password")
        || path.starts_with("/oauth");
    !in_auth_flow
}

/// Open the sign-in window and block until the user authenticates, closes the
/// window, or the timeout elapses. Runs its own tao event loop, so it must own
/// the thread's message pump — call it on the (sub)process main thread, never
/// alongside Slint's event loop in the same process.
#[cfg(windows)]
pub fn run_login_blocking() -> LoginOutcome {
    use tao::event::{Event, StartCause, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::platform::run_return::EventLoopExtRunReturn;
    use tao::platform::windows::EventLoopBuilderExtWindows;
    use tao::window::WindowBuilder;
    use wry::WebViewBuilder;

    let mut event_loop = EventLoopBuilder::new().with_any_thread(true).build();

    let window = match WindowBuilder::new()
        .with_title("Sign in to ESO Logs")
        .with_inner_size(tao::dpi::LogicalSize::new(520.0, 720.0))
        .build(&event_loop)
    {
        Ok(w) => w,
        Err(e) => return LoginOutcome::Error(format!("window: {e}")),
    };

    let webview = match WebViewBuilder::new().with_url(LOGIN_URL).build(&window) {
        Ok(w) => w,
        Err(e) => return LoginOutcome::Error(format!("webview: {e}")),
    };

    let start = Instant::now();
    let mut outcome: Option<LoginOutcome> = None;

    event_loop.run_return(|event, _target, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + POLL_INTERVAL);

        match event {
            Event::NewEvents(StartCause::Init)
            | Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if start.elapsed() > LOGIN_TIMEOUT {
                    outcome = Some(LoginOutcome::TimedOut);
                    *control_flow = ControlFlow::Exit;
                    return;
                }
                // Only capture once the webview has navigated off the login flow.
                let current = webview.url().unwrap_or_default();
                if url_is_authenticated_view(&current) {
                    if let Ok(cookies) = webview.cookies_for_url(COOKIE_ORIGIN) {
                        let pairs: Vec<(String, String)> = cookies
                            .iter()
                            .map(|c| (c.name().to_string(), c.value().to_string()))
                            .collect();
                        if let Some(header) = cookie_header_from(&pairs) {
                            outcome = Some(LoginOutcome::Success(header));
                            *control_flow = ControlFlow::Exit;
                        }
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if outcome.is_none() {
                    outcome = Some(LoginOutcome::Cancelled);
                }
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });

    outcome.unwrap_or(LoginOutcome::Cancelled)
}

#[cfg(not(windows))]
pub fn run_login_blocking() -> LoginOutcome {
    LoginOutcome::Error("In-app sign-in is only available on Windows.".into())
}

/// Sign-in subprocess entry point: run the login window on this process's main
/// thread, print the captured cookie header to stdout (prefixed with
/// [`COOKIE_STDOUT_MARKER`]) on success, and return a process exit code
/// (0 = success, 2 = cancelled, 3 = timed out, 4 = error).
pub fn run_login_subprocess() -> i32 {
    match run_login_blocking() {
        LoginOutcome::Success(header) => {
            println!("{COOKIE_STDOUT_MARKER}{header}");
            0
        }
        LoginOutcome::Cancelled => 2,
        LoginOutcome::TimedOut => 3,
        LoginOutcome::Error(error) => {
            eprintln!("esologs-login error: {error}");
            4
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_requires_session_cookie() {
        assert!(cookie_header_from(&[("XSRF-TOKEN".into(), "abc".into())]).is_none());
        assert!(cookie_header_from(&[(SESSION_COOKIE_NAME.into(), "".into())]).is_none());
    }

    #[test]
    fn header_forwards_session_xsrf_and_remember() {
        let cookies = vec![
            ("wcl_session".to_string(), "sess123".to_string()),
            ("XSRF-TOKEN".to_string(), "tok456".to_string()),
            ("remember_web_deadbeef".to_string(), "rem789".to_string()),
            ("unrelated".to_string(), "x".to_string()),
        ];
        let header = cookie_header_from(&cookies).expect("session present");
        assert!(header.starts_with("wcl_session=sess123"));
        assert!(header.contains("; XSRF-TOKEN=tok456"));
        assert!(header.contains("; remember_web_deadbeef=rem789"));
        assert!(!header.contains("unrelated"));
    }

    #[test]
    fn authenticated_view_gate() {
        // Guest still on the login flow → not authenticated.
        assert!(!url_is_authenticated_view("https://www.esologs.com/login"));
        assert!(!url_is_authenticated_view(
            "https://www.esologs.com/oauth/authorize"
        ));
        // Navigated away on an esologs host → authenticated.
        assert!(url_is_authenticated_view("https://www.esologs.com/"));
        assert!(url_is_authenticated_view(
            "https://www.esologs.com/dashboard"
        ));
        // A different site never counts.
        assert!(!url_is_authenticated_view("https://example.com/"));
    }
}
