//! **Debug-only** auth spike for a future native ESO Logs upload path.
//!
//! Kalpa currently prepares logs and hands them to the official ESO Logs
//! uploader for the actual transfer. A native path (Kalpa POSTing report
//! segments itself) would let Kalpa own the whole Start/Stop lifecycle instead
//! of launching a separate program it cannot control. Before committing to that
//! direction, one unknown gates everything: **does the OAuth bearer token Kalpa
//! already holds (the API token from the existing sign-in) authenticate the
//! report-creation endpoint, or does that endpoint require a different
//! credential?**
//!
//! This module answers exactly that and nothing more. It performs a single,
//! reversible probe: take the current access token, attempt to open (and
//! immediately abandon) a report, and classify the server's response into one of
//! a few outcomes. It uploads no log data, writes no history, and changes no UI.
//!
//! It is compiled only under `#[cfg(debug_assertions)]` (release builds never
//! include it), mirroring the existing `dev_scrub_saved_variable` convention.
//! When the native direction is settled this whole file can be deleted.

use std::time::Duration;

use serde::Serialize;

use crate::auth::AuthState;

/// Where report creation lives. A fact about the service, not borrowed code.
const CREATE_REPORT_URL: &str = "https://www.esologs.com/desktop-client/create-report";

/// The probe's classification of the auth attempt, returned to the dev caller.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeAuthProbe {
    /// One of: `accepted`, `unauthorized`, `forbidden`, `not-signed-in`,
    /// `unexpected`, `network-error`. Drives the human-readable verdict.
    pub outcome: String,
    /// The HTTP status the endpoint returned, when a response was received.
    pub http_status: Option<u16>,
    /// A short, human-readable interpretation for the dev console.
    pub verdict: String,
    /// A trimmed snippet of the response body (capped) for diagnosis. Never
    /// contains the token (we send it, the server doesn't echo it).
    pub body_snippet: Option<String>,
}

/// Cap the body snippet so a stray HTML error page can't flood the console.
const SNIPPET_CAP: usize = 600;

fn snippet(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(SNIPPET_CAP).collect())
}

/// **Debug-only.** Probe whether the existing OAuth bearer token is accepted by
/// the report-creation endpoint. Sends a minimal create-report request and
/// classifies the response. Does not upload anything or persist state.
///
/// Returns `Err` only for conditions that prevented the probe from running at
/// all (not signed in, or the HTTP request could not be constructed/sent — the
/// latter is reported as a `network-error` outcome rather than an `Err` so the
/// caller always gets a structured verdict when a request was attempted).
#[tauri::command]
pub async fn uploader_probe_native_auth(
    auth: tauri::State<'_, AuthState>,
) -> Result<NativeAuthProbe, String> {
    // Reuse the existing sign-in: a fresh, refreshed-if-needed access token.
    let token = match auth.get_valid_token()? {
        Some(t) => t,
        None => {
            return Ok(NativeAuthProbe {
                outcome: "not-signed-in".into(),
                http_status: None,
                verdict: "Not signed in to ESO Logs — sign in first, then re-run the probe."
                    .into(),
                body_snippet: None,
            });
        }
    };

    // A minimal, plausible create-report payload. We do NOT proceed past this
    // call — if a report is opened, abandoning it (no segments, no terminate)
    // leaves an empty draft the service expires on its own. The point is solely
    // to read the AUTH verdict from the response status.
    let payload = serde_json::json!({
        "visibility": "private",
        "description": "Kalpa native-auth probe (no data uploaded)",
    });

    // Run the blocking reqwest call off the async runtime so we don't stall it.
    let result = tokio::task::spawn_blocking(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| format!("HTTP client error: {e}"))?;
        let resp = client
            .post(CREATE_REPORT_URL)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        Ok::<(reqwest::StatusCode, String), String>((status, body))
    })
    .await
    .map_err(|e| format!("probe task failed: {e}"))?;

    match result {
        Ok((status, body)) => {
            let code = status.as_u16();
            let outcome = if status.is_success() {
                "accepted"
            } else if code == 401 {
                "unauthorized"
            } else if code == 403 {
                "forbidden"
            } else {
                "unexpected"
            };
            let verdict = match outcome {
                "accepted" => "The existing OAuth bearer token IS accepted by create-report. \
                     Native uploads can reuse the current sign-in with no extra credentials."
                    .to_string(),
                "unauthorized" => "create-report rejected the bearer token (401). The upload \
                     endpoints likely use a different credential than the API token — reusing \
                     the existing sign-in may need a token→session bridge, or a separate login."
                    .to_string(),
                "forbidden" => "create-report returned 403. The token authenticated but lacks \
                     permission/scope for uploading, or the endpoint expects a different client \
                     identity. Reusing the existing sign-in as-is is unlikely."
                    .to_string(),
                _ => format!(
                    "create-report returned an unexpected status {code}. Inspect the body \
                     snippet; the auth verdict is inconclusive."
                ),
            };
            Ok(NativeAuthProbe {
                outcome: outcome.into(),
                http_status: Some(code),
                verdict,
                body_snippet: snippet(&body),
            })
        }
        Err(e) => Ok(NativeAuthProbe {
            outcome: "network-error".into(),
            http_status: None,
            verdict: format!("The probe request could not complete: {e}"),
            body_snippet: None,
        }),
    }
}
