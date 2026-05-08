use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ── Constants ────────────────────────────────────────────────────────────

/// The website URL that handles the OAuth flow and passes tokens back.
const APP_AUTH_URL: &str = "https://esotk.com/app-auth";

const USER_API: &str = "https://www.esologs.com/api/v2/user";

// ── Types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

pub struct AuthState(pub Mutex<Option<AuthTokens>>);

/// Token data received from the website's OAuth proxy.
#[derive(Debug, Deserialize)]
pub(crate) struct CallbackTokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
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
/// 2. Open browser to website's /app-auth?port={port}
/// 3. Website does PKCE OAuth with ESO Logs (using its registered redirect URI)
/// 4. Website sends tokens to http://localhost:{port}/callback?tokens={base64}
/// 5. We receive and decode the tokens
pub fn run_oauth_flow() -> Result<CallbackTokens, String> {
    // Bind to random port
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("Failed to bind port: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed to get port: {}", e))?
        .port();

    // Open browser to the website's app-auth page
    let auth_url = format!("{}?port={}", APP_AUTH_URL, port);

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &auth_url])
            .spawn()
            .map_err(|e| format!("Failed to open browser: {}", e))?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("xdg-open")
            .arg(&auth_url)
            .spawn()
            .map_err(|e| format!("Failed to open browser: {}", e))?;
    }

    // Wait for callback (120s timeout)
    let timeout = Duration::from_secs(120);
    let start = std::time::Instant::now();
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("Failed to set nonblocking: {}", e))?;

    loop {
        if start.elapsed() > timeout {
            return Err("OAuth login timed out. Please try again.".to_string());
        }

        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                let mut buf = Vec::new();
                let mut tmp = [0u8; 16384];
                // Read until we have the full HTTP request headers
                loop {
                    match stream.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                            if buf.len() > 65536 {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let request = String::from_utf8_lossy(&buf);

                if let Some(tokens) = extract_tokens_from_request(&request) {
                    // Send success page
                    let html = r#"<!DOCTYPE html><html><head><style>body{font-family:system-ui;background:#0b1220;color:#e5e7eb;display:flex;align-items:center;justify-content:center;height:100vh;margin:0}div{text-align:center}h1{color:#c4a44a;font-size:1.5rem}p{opacity:0.6}</style></head><body><div><h1>Signed in!</h1><p>You can close this tab and return to Kalpa.</p></div></body></html>"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: https://esotk.com\r\nConnection: close\r\n\r\n{}",
                        html.len(),
                        html
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                    return Ok(tokens);
                } else if request.contains("OPTIONS") {
                    // Handle CORS preflight — only esotk.com needs access to this callback
                    let response = "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: https://esotk.com\r\nAccess-Control-Allow-Methods: GET\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = stream.write_all(response.as_bytes());
                } else {
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
                return Err(format!("Server error: {}", e));
            }
        }
    }
}

fn extract_tokens_from_request(request: &str) -> Option<CallbackTokens> {
    let first_line = request.lines().next()?;
    let path = first_line.split_whitespace().nth(1)?;
    if !path.starts_with("/callback") {
        return None;
    }
    let query = path.split('?').nth(1)?;
    for param in query.split('&') {
        if let Some(value) = param.strip_prefix("tokens=") {
            let decoded_param = urlencoding::decode(value).ok()?;
            let json_bytes = STANDARD.decode(decoded_param.as_bytes()).ok()?;
            let tokens: CallbackTokens = serde_json::from_slice(&json_bytes).ok()?;
            return Some(tokens);
        }
    }
    None
}

// ── User Validation ──────────────────────────────────────────────────────

pub fn validate_token(access_token: &str) -> Result<(String, String), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let query = r#"{ "query": "{ userData { currentUser { id name } } }" }"#;

    let response = client
        .post(USER_API)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .body(query)
        .send()
        .map_err(|e| format!("User validation failed: {}", e))?;

    if !response.status().is_success() {
        return Err("Token validation failed".to_string());
    }

    let body: GraphQLResponse = response
        .json()
        .map_err(|e| format!("Failed to parse user response: {}", e))?;

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
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", "9fd28ffc-300a-44ce-8a0e-6167db47a7e1"),
    ];

    let response = client
        .post("https://www.esologs.com/oauth/token")
        .form(&params)
        .send()
        .map_err(|e| format!("Token refresh failed: {}", e))?;

    if !response.status().is_success() {
        return Err("Session expired. Please sign in again.".to_string());
    }

    response
        .json::<CallbackTokens>()
        .map_err(|e| format!("Failed to parse refresh response: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }

    fn encode_tokens_param(json: &str) -> String {
        STANDARD.encode(json.as_bytes())
    }

    fn http_request_with_query(query: &str) -> String {
        format!(
            "GET /callback?{} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            query
        )
    }

    // ── urlencoding::decode ──────────────────────────────────────────

    #[test]
    fn urldecode_passes_through_plain_ascii() {
        assert_eq!(urlencoding::decode("hello").unwrap(), "hello");
    }

    #[test]
    fn urldecode_translates_plus_to_space() {
        assert_eq!(urlencoding::decode("hello+world").unwrap(), "hello world");
    }

    #[test]
    fn urldecode_decodes_percent_escapes() {
        assert_eq!(urlencoding::decode("a%20b%2Fc").unwrap(), "a b/c");
    }

    #[test]
    fn urldecode_handles_full_byte_range() {
        // %FF must round-trip as 0xFF
        let decoded = urlencoding::decode("%C3%A9").unwrap(); // é (UTF-8)
        assert_eq!(decoded, "é");
    }

    #[test]
    fn urldecode_rejects_truncated_percent_escape() {
        assert!(urlencoding::decode("a%2").is_err());
        assert!(urlencoding::decode("%").is_err());
    }

    #[test]
    fn urldecode_rejects_non_hex_escape() {
        assert!(urlencoding::decode("%ZZ").is_err());
    }

    #[test]
    fn urldecode_rejects_invalid_utf8_sequences() {
        // 0xFF is not valid UTF-8 on its own
        assert!(urlencoding::decode("%FF").is_err());
    }

    // ── extract_tokens_from_request ──────────────────────────────────

    #[test]
    fn extract_tokens_returns_valid_callback_tokens() {
        let payload = r#"{"access_token":"AT","refresh_token":"RT","expires_in":3600}"#;
        let encoded = encode_tokens_param(payload);
        let request = http_request_with_query(&format!("tokens={}", encoded));
        let tokens = extract_tokens_from_request(&request).expect("tokens parsed");
        assert_eq!(tokens.access_token, "AT");
        assert_eq!(tokens.refresh_token.as_deref(), Some("RT"));
        assert_eq!(tokens.expires_in, Some(3600));
    }

    #[test]
    fn extract_tokens_handles_optional_refresh_and_expires() {
        let payload = r#"{"access_token":"AT"}"#;
        let encoded = encode_tokens_param(payload);
        let request = http_request_with_query(&format!("tokens={}", encoded));
        let tokens = extract_tokens_from_request(&request).expect("tokens parsed");
        assert_eq!(tokens.access_token, "AT");
        assert!(tokens.refresh_token.is_none());
        assert!(tokens.expires_in.is_none());
    }

    #[test]
    fn extract_tokens_decodes_percent_encoded_param() {
        let payload = r#"{"access_token":"hello world"}"#;
        let encoded = encode_tokens_param(payload);
        // Replace any '+' (base64 alt char) is irrelevant; force percent-encoding instead.
        let percent_encoded = encoded.replace("=", "%3D");
        let request = http_request_with_query(&format!("tokens={}", percent_encoded));
        let tokens = extract_tokens_from_request(&request).expect("tokens parsed");
        assert_eq!(tokens.access_token, "hello world");
    }

    #[test]
    fn extract_tokens_finds_param_among_others() {
        let payload = r#"{"access_token":"AT"}"#;
        let encoded = encode_tokens_param(payload);
        let request = http_request_with_query(&format!("foo=bar&tokens={}&baz=qux", encoded));
        let tokens = extract_tokens_from_request(&request).expect("tokens parsed");
        assert_eq!(tokens.access_token, "AT");
    }

    #[test]
    fn extract_tokens_rejects_non_callback_path() {
        let payload = r#"{"access_token":"AT"}"#;
        let encoded = encode_tokens_param(payload);
        let request = format!(
            "GET /something-else?tokens={} HTTP/1.1\r\nHost: x\r\n\r\n",
            encoded
        );
        assert!(extract_tokens_from_request(&request).is_none());
    }

    #[test]
    fn extract_tokens_rejects_when_param_missing() {
        let request = "GET /callback?other=1 HTTP/1.1\r\nHost: x\r\n\r\n";
        assert!(extract_tokens_from_request(request).is_none());
    }

    #[test]
    fn extract_tokens_rejects_request_without_query() {
        let request = "GET /callback HTTP/1.1\r\nHost: x\r\n\r\n";
        assert!(extract_tokens_from_request(request).is_none());
    }

    #[test]
    fn extract_tokens_rejects_invalid_base64() {
        let request = http_request_with_query("tokens=!!!not-base64!!!");
        assert!(extract_tokens_from_request(request.as_str()).is_none());
    }

    #[test]
    fn extract_tokens_rejects_invalid_json_after_decoding() {
        let encoded = STANDARD.encode(b"not-json-at-all");
        let request = http_request_with_query(&format!("tokens={}", encoded));
        assert!(extract_tokens_from_request(&request).is_none());
    }

    #[test]
    fn extract_tokens_rejects_empty_request() {
        assert!(extract_tokens_from_request("").is_none());
    }

    // ── ensure_valid_token (offline paths) ────────────────────────────

    fn make_tokens(expires_at: i64, refresh: &str) -> AuthTokens {
        AuthTokens {
            access_token: "at".to_string(),
            refresh_token: refresh.to_string(),
            expires_at,
            user_id: "1".to_string(),
            user_name: "ada".to_string(),
        }
    }

    #[test]
    fn ensure_valid_token_returns_none_when_token_still_valid() {
        let tokens = make_tokens(now_secs() + 3600, "rt");
        let result = ensure_valid_token(&tokens).expect("should not error");
        assert!(result.is_none(), "fresh token must not trigger refresh");
    }

    #[test]
    fn ensure_valid_token_returns_none_at_safety_buffer_boundary() {
        // expires_at = now + 120 → > now + 60, so still valid
        let tokens = make_tokens(now_secs() + 120, "rt");
        assert!(ensure_valid_token(&tokens).unwrap().is_none());
    }

    #[test]
    fn ensure_valid_token_errors_when_expired_and_no_refresh_token() {
        let tokens = make_tokens(now_secs() - 10, "");
        let err = ensure_valid_token(&tokens).expect_err("should fail");
        assert!(err.contains("sign in"), "got unexpected error: {}", err);
    }

    #[test]
    fn ensure_valid_token_errors_when_inside_buffer_and_no_refresh_token() {
        // expires_at = now + 30 → < now + 60, refresh required, but none available
        let tokens = make_tokens(now_secs() + 30, "");
        assert!(ensure_valid_token(&tokens).is_err());
    }
}
