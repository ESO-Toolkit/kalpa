use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ── Constants ────────────────────────────────────────────────────────────

/// The website URL that handles the OAuth flow and passes tokens back.
const APP_AUTH_URL: &str = "https://esotk.com/app-auth";

const USER_API: &str = "https://www.esologs.com/api/v2/user";

static OAUTH_ATTEMPT: OnceLock<Mutex<Option<OAuthAttempt>>> = OnceLock::new();
static NEXT_OAUTH_ATTEMPT_ID: AtomicU64 = AtomicU64::new(1);
const OAUTH_CANCEL_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

struct OAuthAttempt {
    id: u64,
    cancel: Arc<AtomicBool>,
}

// ── Types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub user_id: String,
    pub user_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthUser {
    pub user_id: String,
    pub user_name: String,
    /// Whether the session was durably persisted to the OS credential store.
    /// `Some(false)` means the login is **memory-only** (a Credential Manager
    /// failure) and will not survive a restart — the UI should warn the user.
    /// `None`/absent for callers that don't establish a session (back-compat:
    /// existing consumers ignore it).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_persisted: Option<bool>,
}

pub struct AuthState {
    pub tokens: Mutex<Option<AuthTokens>>,
    refresh_lock: Mutex<()>,
}

impl AuthState {
    pub fn new(tokens: Option<AuthTokens>) -> Self {
        Self {
            tokens: Mutex::new(tokens),
            refresh_lock: Mutex::new(()),
        }
    }

    /// Get the current access token, refreshing if expired, without persisting a
    /// refreshed pair. Callers that own a credential store should use
    /// [`AuthState::get_valid_token_persisting`] instead.
    pub fn get_valid_token(&self) -> Result<Option<String>, String> {
        self.get_valid_token_persisting(|_| {})
    }

    /// Get the current access token, refreshing if expired.
    ///
    /// Every step runs under `refresh_lock`, which is what makes concurrent
    /// callers safe: the stored tokens are re-read AFTER the lock is taken, so a
    /// waiter sees the winner's fresh pair and `ensure_valid_token` returns
    /// `Ok(None)` instead of POSTing the same refresh_token a second time (ESO
    /// Logs rotates it, so the loser would get a 4xx and spuriously sign the user
    /// out). `persist` is called with a freshly refreshed pair while the lock is
    /// still held, so two refreshes can never interleave their credential-store
    /// writes; a persistence failure is the caller's to log, not a refresh error.
    pub fn get_valid_token_persisting(
        &self,
        persist: impl FnOnce(&AuthTokens),
    ) -> Result<Option<String>, String> {
        let _refresh_guard = self
            .refresh_lock
            .lock()
            .map_err(|_| "Internal error.".to_string())?;

        let tokens = {
            let guard = self
                .tokens
                .lock()
                .map_err(|_| "Internal error.".to_string())?;
            guard.clone()
        };

        let Some(tokens) = tokens else {
            return Ok(None);
        };

        match ensure_valid_token(&tokens)? {
            Some(new_tokens) => {
                let token = new_tokens.access_token.clone();
                persist(&new_tokens);
                *self
                    .tokens
                    .lock()
                    .map_err(|_| "Internal error.".to_string())? = Some(new_tokens);
                Ok(Some(token))
            }
            None => Ok(Some(tokens.access_token)),
        }
    }
}

/// Token data received from the website's OAuth proxy.
#[derive(Debug, Deserialize)]
pub(crate) struct CallbackTokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    /// Echo of the `state` nonce this attempt put in the auth URL. Absent while
    /// esotk.com has not been updated to echo it — see [`state_matches`].
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphQLResponse {
    data: Option<GraphQLData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQLData {
    user_data: Option<UserData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserData {
    current_user: Option<CurrentUser>,
}

#[derive(Debug, Deserialize)]
struct CurrentUser {
    id: serde_json::Value,
    name: String,
}

// ── URL encoding (minimal, no external crate) ───────────────────────────

mod urlencoding {
    pub fn decode(s: &str) -> Result<String, ()> {
        let mut bytes = Vec::new();
        let mut chars = s.bytes();
        while let Some(b) = chars.next() {
            if b == b'%' {
                let hi = chars.next().ok_or(())?;
                let lo = chars.next().ok_or(())?;
                let hex = [hi, lo];
                let s = std::str::from_utf8(&hex).map_err(|_| ())?;
                let byte = u8::from_str_radix(s, 16).map_err(|_| ())?;
                bytes.push(byte);
            } else if b == b'+' {
                bytes.push(b' ');
            } else {
                bytes.push(b);
            }
        }
        String::from_utf8(bytes).map_err(|_| ())
    }
}

// ── Localhost Callback Server ────────────────────────────────────────────

/// Opens browser to the website's /app-auth page which handles the full
/// OAuth flow, then redirects tokens back to our localhost server.
///
/// Flow:
/// 1. Bind localhost server on random port
/// 2. Open browser to website's /app-auth?port={port}&state={nonce}
/// 3. Website does PKCE OAuth with ESO Logs (using its registered redirect URI)
/// 4. Website posts tokens to http://localhost:{port}/callback as JSON
/// 5. We receive and decode the tokens
pub fn run_oauth_flow() -> Result<CallbackTokens, String> {
    let (attempt_id, cancel) = begin_oauth_attempt()?;
    let result = run_oauth_flow_attempt(&cancel);
    finish_oauth_attempt(attempt_id);
    result
}

pub fn cancel_oauth_flow() -> Result<bool, String> {
    let mut guard = oauth_attempts()
        .lock()
        .map_err(|_| "Internal error.".to_string())?;
    let Some(attempt) = guard.take() else {
        return Ok(false);
    };
    attempt.cancel.store(true, Ordering::SeqCst);
    Ok(true)
}

fn oauth_attempts() -> &'static Mutex<Option<OAuthAttempt>> {
    OAUTH_ATTEMPT.get_or_init(|| Mutex::new(None))
}

fn begin_oauth_attempt() -> Result<(u64, Arc<AtomicBool>), String> {
    let drain_started = std::time::Instant::now();
    loop {
        {
            let mut guard = oauth_attempts()
                .lock()
                .map_err(|_| "Internal error.".to_string())?;
            if guard.is_none() {
                let id = NEXT_OAUTH_ATTEMPT_ID.fetch_add(1, Ordering::SeqCst);
                let cancel = Arc::new(AtomicBool::new(false));
                *guard = Some(OAuthAttempt {
                    id,
                    cancel: Arc::clone(&cancel),
                });
                return Ok((id, cancel));
            }

            if let Some(previous) = guard.as_ref() {
                previous.cancel.store(true, Ordering::SeqCst);
            }
        }

        if drain_started.elapsed() >= OAUTH_CANCEL_DRAIN_TIMEOUT {
            return Err(
                "A previous sign-in is still closing. Try again in a moment and use the newest ESO Logs tab; an older tab may be stale."
                    .to_string(),
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn finish_oauth_attempt(id: u64) {
    let Ok(mut guard) = oauth_attempts().lock() else {
        return;
    };
    if guard.as_ref().is_some_and(|attempt| attempt.id == id) {
        *guard = None;
    }
}

fn run_oauth_flow_attempt(cancel: &AtomicBool) -> Result<CallbackTokens, String> {
    // Bind 127.0.0.1 EXPLICITLY, not "localhost".
    //
    // `TcpListener::bind("localhost:0")` resolves the name and binds the first
    // address that accepts. On Windows `localhost` resolves to `::1` first, so
    // the listener ended up IPv6-only — and the browser posting the callback to
    // `http://localhost:{port}` could pick 127.0.0.1 and get connection refused,
    // surfacing as "Could not connect to the desktop application."
    //
    // RFC 8252 §7.3 says native OAuth clients should use the IPv4 loopback
    // literal for exactly this reason. Fall back to `::1` for the rare host with
    // IPv4 loopback disabled, so this is strictly more permissive than before.
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .or_else(|_| TcpListener::bind(("::1", 0)))
        .map_err(|e| format!("Failed to bind port: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed to get port: {e}"))?
        .port();

    // Per-attempt secret binding the callback to THIS flow (RFC 8252 §8.9). The
    // listener is otherwise a bare loopback port that accepts any request which
    // happens to parse as tokens.
    let state = generate_state_nonce();

    // Open browser to the website's app-auth page
    let auth_url = format!("{APP_AUTH_URL}?port={port}&state={state}");

    crate::platform::open_url(&auth_url)?;

    // Wait for callback (120s timeout)
    let timeout = Duration::from_secs(120);
    let start = std::time::Instant::now();
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("Failed to set nonblocking: {e}"))?;

    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("Sign-in was cancelled.".to_string());
        }

        if start.elapsed() > timeout {
            return Err(
                "Sign-in timed out after 120 seconds. Try again and use the newest ESO Logs tab; an older tab may be stale."
                    .to_string(),
            );
        }

        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                let mut buf = Vec::new();
                let mut tmp = [0u8; 16384];
                let mut header_end = None;
                let mut expected_len = None;
                // Read until we have the full HTTP request, including the JSON body
                // used by the current esotk.com app-auth callback.
                loop {
                    match stream.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);

                            if header_end.is_none() {
                                header_end = find_header_end(&buf);
                                if let Some(end) = header_end {
                                    let request_head = String::from_utf8_lossy(&buf[..end]);
                                    expected_len = Some(end + content_length(&request_head));
                                }
                            }

                            if expected_len.is_some_and(|len| buf.len() >= len) {
                                break;
                            }
                            if buf.len() > 65536 {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let header_end = header_end.unwrap_or(buf.len());
                let request = String::from_utf8_lossy(&buf[..header_end]);
                let body = if header_end <= buf.len() {
                    &buf[header_end..]
                } else {
                    &[]
                };

                if request.starts_with("OPTIONS ") {
                    write_preflight_response(&mut stream);
                } else if let Some(tokens) = extract_tokens_from_request(&request, body, &state) {
                    // Send success page
                    let html = r#"<!DOCTYPE html><html><head><style>body{font-family:system-ui;background:#0b1220;color:#e5e7eb;display:flex;align-items:center;justify-content:center;height:100vh;margin:0}div{text-align:center}h1{color:#c4a44a;font-size:1.5rem}p{opacity:0.6}</style></head><body><div><h1>Signed in!</h1><p>You can close this tab and return to Kalpa.</p></div></body></html>"#;
                    write_response(&mut stream, "200 OK", "text/html", html);
                    return Ok(tokens);
                } else {
                    // Unknown callback request.
                    let response =
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = stream.write_all(response.as_bytes());
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => {
                return Err(format!("Server error: {e}"));
            }
        }
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

fn content_length(request_head: &str) -> usize {
    request_head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0)
}

fn write_cors_headers(response: &mut String) {
    response.push_str("Access-Control-Allow-Origin: https://esotk.com\r\n");
    response.push_str("Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n");
    response.push_str("Access-Control-Allow-Headers: Content-Type\r\n");
    response.push_str("Access-Control-Allow-Private-Network: true\r\n");
    response.push_str("Vary: Origin\r\n");
}

fn write_preflight_response(stream: &mut impl Write) {
    let mut response = "HTTP/1.1 204 No Content\r\n".to_string();
    write_cors_headers(&mut response);
    response.push_str("Content-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn write_response(stream: &mut impl Write, status: &str, content_type: &str, body: &str) {
    let mut response = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n");
    write_cors_headers(&mut response);
    response.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    ));
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// A 128-bit value an outside observer cannot predict, for the callback `state`.
///
/// Built from `RandomState`, whose SipHash keys are seeded from the OS CSPRNG
/// and never leave the process — so the digests below are unguessable to the
/// local process or web page this nonce defends against. std-only on purpose:
/// this module is also `#[path]`-included by the Slint sidecar crate, which
/// carries its own (smaller) dependency list.
fn generate_state_nonce() -> String {
    use std::collections::hash_map::RandomState;
    use std::fmt::Write as _;
    use std::hash::{BuildHasher, Hasher};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let mut out = String::with_capacity(32);
    for half in 0u64..2 {
        // A fresh RandomState per half so the two digests use different keys.
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(half);
        hasher.write_u128(nanos);
        let _ = write!(out, "{:016x}", hasher.finish());
    }
    out
}

/// Whether a callback's `state` echo belongs to this attempt.
///
/// A supplied state MUST match. An ABSENT state is still accepted, because
/// esotk.com does not echo the parameter yet and rejecting it would break every
/// sign-in until the site ships that change. Meanwhile the browser vector stays
/// closed by the `Content-Type: application/json` requirement below: it forces a
/// CORS preflight, and the preflight only permits `https://esotk.com`. Once the
/// site echoes `state`, make an absent value a rejection here.
fn state_matches(supplied: Option<&str>, expected: &str) -> bool {
    match supplied {
        Some(s) => s == expected,
        None => true,
    }
}

/// Whether a request head declares a JSON body.
///
/// Required on the POST callback: a cross-origin page can send `text/plain` or
/// form encodings with no preflight, but not `application/json` — so demanding
/// it routes every browser-originated callback through the preflight, which
/// [`write_cors_headers`] answers for `https://esotk.com` alone.
fn is_json_content_type(request_head: &str) -> bool {
    request_head.lines().any(|line| {
        let Some((name, value)) = line.split_once(':') else {
            return false;
        };
        name.eq_ignore_ascii_case("content-type")
            && value
                .split(';')
                .next()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
    })
}

fn extract_tokens_from_request(
    request: &str,
    body: &[u8],
    expected_state: &str,
) -> Option<CallbackTokens> {
    let first_line = request.lines().next()?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next()?;
    let path = parts.next()?;
    if !path.starts_with("/callback") {
        return None;
    }

    if method.eq_ignore_ascii_case("POST") {
        if !is_json_content_type(request) {
            return None;
        }
        let tokens: CallbackTokens = serde_json::from_slice(body).ok()?;
        // The query string is the other place esotk.com could echo the nonce.
        let echoed = path.split('?').nth(1).and_then(query_state);
        let supplied = tokens.state.as_deref().or(echoed.as_deref());
        return state_matches(supplied, expected_state).then_some(tokens);
    }

    if !method.eq_ignore_ascii_case("GET") {
        return None;
    }

    let query = path.split('?').nth(1)?;
    let echoed = query_state(query);
    for param in query.split('&') {
        if let Some(value) = param.strip_prefix("tokens=") {
            let decoded_param = urlencoding::decode(value).ok()?;
            let json_bytes = STANDARD.decode(decoded_param.as_bytes()).ok()?;
            let tokens: CallbackTokens = serde_json::from_slice(&json_bytes).ok()?;
            let supplied = tokens.state.as_deref().or(echoed.as_deref());
            return state_matches(supplied, expected_state).then_some(tokens);
        }
    }
    None
}

/// The decoded `state` query parameter, if present.
fn query_state(query: &str) -> Option<String> {
    query
        .split('&')
        .find_map(|param| param.strip_prefix("state="))
        .and_then(|v| urlencoding::decode(v).ok())
}

// ── User Validation ──────────────────────────────────────────────────────

/// Shared HTTP client for token validation and refresh. Both endpoints use the
/// same flat timeout, so it lives on the client; reusing one client avoids a
/// fresh connection pool + TLS handshake for every auth call (refresh always
/// validates right after, so the calls come in pairs).
fn auth_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("failed to build auth HTTP client")
    })
}

pub fn validate_token(access_token: &str) -> Result<(String, String), String> {
    let client = auth_client();

    let query = r#"{ "query": "{ userData { currentUser { id name } } }" }"#;

    let response = client
        .post(USER_API)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .body(query)
        .send()
        .map_err(|e| format!("User validation failed: {e}"))?;

    if !response.status().is_success() {
        return Err("Token validation failed".to_string());
    }

    let body: GraphQLResponse = response
        .json()
        .map_err(|e| format!("Failed to parse user response: {e}"))?;

    let user = body
        .data
        .and_then(|d| d.user_data)
        .and_then(|u| u.current_user)
        .ok_or_else(|| "Could not retrieve user info".to_string())?;

    let user_id = match &user.id {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };

    Ok((user_id, user.name))
}

// ── Full Login Flow ──────────────────────────────────────────────────────

pub fn login() -> Result<AuthTokens, String> {
    let token_resp = run_oauth_flow()?;
    let (user_id, user_name) = validate_token(&token_resp.access_token)?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let expires_at = now + token_resp.expires_in.unwrap_or(3600);

    Ok(AuthTokens {
        access_token: token_resp.access_token,
        refresh_token: token_resp.refresh_token.unwrap_or_default(),
        expires_at,
        user_id,
        user_name,
    })
}

/// Refresh tokens if expired, returns updated tokens or error.
pub fn ensure_valid_token(tokens: &AuthTokens) -> Result<Option<AuthTokens>, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // Still valid (with 60s buffer)
    if tokens.expires_at > now + 60 {
        return Ok(None);
    }

    // Try refresh via the website's token endpoint
    if tokens.refresh_token.is_empty() {
        return Err("Session expired. Please sign in again.".to_string());
    }

    let token_resp = refresh_token_request(&tokens.refresh_token)?;
    let (user_id, user_name) = validate_token(&token_resp.access_token)?;

    let expires_at = now + token_resp.expires_in.unwrap_or(3600);

    Ok(Some(AuthTokens {
        access_token: token_resp.access_token,
        refresh_token: token_resp
            .refresh_token
            .unwrap_or_else(|| tokens.refresh_token.clone()),
        expires_at,
        user_id,
        user_name,
    }))
}

/// Token refresh — this calls ESO Logs directly since refresh doesn't
/// require a registered redirect_uri.
fn refresh_token_request(refresh_token: &str) -> Result<CallbackTokens, String> {
    let client = auth_client();

    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", "9fd28ffc-300a-44ce-8a0e-6167db47a7e1"),
    ];

    let response = client
        .post("https://www.esologs.com/oauth/token")
        .form(&params)
        .send()
        .map_err(|e| format!("Token refresh failed: {e}"))?;

    if !response.status().is_success() {
        return Err("Session expired. Please sign in again.".to_string());
    }

    response
        .json::<CallbackTokens>()
        .map_err(|e| format!("Failed to parse refresh response: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const POST_HEAD: &str = concat!(
        "POST /callback HTTP/1.1\r\n",
        "Host: localhost:12345\r\n",
        "Content-Type: application/json\r\n",
        "Content-Length: 69\r\n",
        "\r\n"
    );

    #[test]
    fn extracts_tokens_from_post_json_callback() {
        let body = br#"{"access_token":"access","refresh_token":"refresh","expires_in":3600}"#;

        let tokens = extract_tokens_from_request(POST_HEAD, body, "nonce").expect("tokens");

        assert_eq!(tokens.access_token, "access");
        assert_eq!(tokens.refresh_token.as_deref(), Some("refresh"));
        assert_eq!(tokens.expires_in, Some(3600));
    }

    #[test]
    fn extracts_tokens_from_legacy_get_callback() {
        let encoded = STANDARD
            .encode(br#"{"access_token":"access","refresh_token":"refresh","expires_in":3600}"#);
        let request = format!("GET /callback?tokens={encoded} HTTP/1.1\r\n\r\n");

        let tokens = extract_tokens_from_request(&request, &[], "nonce").expect("tokens");

        assert_eq!(tokens.access_token, "access");
        assert_eq!(tokens.refresh_token.as_deref(), Some("refresh"));
        assert_eq!(tokens.expires_in, Some(3600));
    }

    #[test]
    fn rejects_a_callback_whose_state_belongs_to_another_attempt() {
        let body = br#"{"access_token":"evil","state":"someone-elses-nonce"}"#;
        assert!(extract_tokens_from_request(POST_HEAD, body, "nonce").is_none());

        let encoded = STANDARD.encode(br#"{"access_token":"evil"}"#);
        let request = format!("GET /callback?tokens={encoded}&state=wrong HTTP/1.1\r\n\r\n");
        assert!(extract_tokens_from_request(&request, &[], "nonce").is_none());
    }

    #[test]
    fn accepts_a_callback_echoing_this_attempts_state() {
        let body = br#"{"access_token":"access","state":"nonce"}"#;
        let tokens = extract_tokens_from_request(POST_HEAD, body, "nonce").expect("tokens");
        assert_eq!(tokens.access_token, "access");

        let encoded = STANDARD.encode(br#"{"access_token":"access"}"#);
        let request = format!("GET /callback?tokens={encoded}&state=nonce HTTP/1.1\r\n\r\n");
        assert!(extract_tokens_from_request(&request, &[], "nonce").is_some());
    }

    /// A cross-origin page can POST `text/plain` with no preflight; demanding
    /// JSON forces the preflight, which only `https://esotk.com` passes.
    #[test]
    fn rejects_a_post_callback_that_is_not_json() {
        let body = br#"{"access_token":"evil"}"#;
        let request = concat!(
            "POST /callback HTTP/1.1\r\n",
            "Host: localhost:12345\r\n",
            "Content-Type: text/plain\r\n",
            "Content-Length: 23\r\n",
            "\r\n"
        );
        assert!(extract_tokens_from_request(request, body, "nonce").is_none());
    }

    #[test]
    fn json_content_type_accepts_a_charset_parameter() {
        assert!(is_json_content_type(
            "POST /callback HTTP/1.1\r\ncontent-type: application/json; charset=utf-8\r\n\r\n"
        ));
        assert!(!is_json_content_type("POST /callback HTTP/1.1\r\n\r\n"));
    }

    #[test]
    fn state_nonces_are_unique_and_url_safe() {
        let a = generate_state_nonce();
        let b = generate_state_nonce();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn parses_content_length_case_insensitively() {
        let request = "POST /callback HTTP/1.1\r\ncontent-length: 42\r\n\r\n";

        assert_eq!(content_length(request), 42);
    }

    #[test]
    fn auth_user_session_persisted_serializes_camelcase_and_omits_when_none() {
        // Present → camelCase `sessionPersisted` field carries the bool.
        let with = AuthUser {
            user_id: "1".into(),
            user_name: "n".into(),
            session_persisted: Some(false),
        };
        let j = serde_json::to_value(&with).unwrap();
        assert_eq!(j["sessionPersisted"], serde_json::json!(false));

        // Absent → field is OMITTED entirely (back-compat: old consumers and
        // status responses see no extra key).
        let without = AuthUser {
            user_id: "1".into(),
            user_name: "n".into(),
            session_persisted: None,
        };
        let j2 = serde_json::to_value(&without).unwrap();
        assert!(
            j2.get("sessionPersisted").is_none(),
            "session_persisted: None must be omitted, got {j2}"
        );
    }
}
