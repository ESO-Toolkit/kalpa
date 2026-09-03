//! Tauri command handlers, HTTP clients, and IPC types for the Pack Hub.
//!
//! Talks to the dedicated `kalpa-pack-hub` Cloudflare Worker (and its sibling
//! share worker) over `reqwest::blocking` inside `spawn_blocking`, mirroring
//! the async-command-wraps-blocking-IO convention used throughout
//! `commands.rs`. Several commands here borrow the ESO Logs OAuth bearer
//! token to call the worker as a signed-in user, but that session itself
//! (`AuthState`, token persistence, `auth_login`/`auth_logout`/`auth_get_user`)
//! is owned by `crate::commands` — it is shared with the uploader and the
//! header account chip, not a Pack Hub concept. The handful of auth helpers
//! this module needs are re-used from there via `pub(crate)` visibility
//! rather than duplicated.

use crate::auth::AuthState;
use crate::commands::{
    clear_auth_and_upload_sessions, clear_session_if_rejected, require_allowed_path,
    save_auth_tokens, validate_name,
};
use crate::saved_variables::io as sv_io;
use crate::uploader::native::session::StoredSessionProvider;
use crate::AllowedAddonsPath;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

// ── Pack Hub API (kalpa-pack-hub) ──────────────────────────────────────────

/// Base URL for the dedicated Pack Hub worker.
fn pack_hub_url() -> &'static str {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| {
        std::env::var("PACK_HUB_API_URL")
            .unwrap_or_else(|_| "https://kalpa-pack-hub.eso-toolkit.workers.dev".to_string())
    })
}

/// Validate a pack ID to prevent path traversal in URL interpolation.
fn validate_pack_id(id: &str) -> Result<(), String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap());
    if id.is_empty() || id.len() > 100 || !re.is_match(id) {
        return Err("Invalid pack ID.".to_string());
    }
    Ok(())
}

fn pack_hub_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .user_agent(format!("Kalpa/{}", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("failed to build pack hub HTTP client")
    })
}

// ── Pack structs (matching kalpa-pack-hub response) ───────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackAddonEntry {
    pub esoui_id: u32,
    pub name: String,
    #[serde(default = "default_true")]
    pub required: bool,
    pub note: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Full pack object returned by kalpa-pack-hub.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HubPack {
    pub id: String,
    #[serde(default)]
    pub author_id: String,
    pub author_name: String,
    pub is_anonymous: bool,
    pub title: String,
    pub description: String,
    pub pack_type: String,
    pub addons: serde_json::Value, // JSON string from D1 or parsed array
    pub vote_count: i64,
    #[serde(default)]
    pub install_count: i64,
    pub created_at: String,
    pub updated_at: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub user_voted: Option<bool>,
    #[serde(default)]
    pub status: Option<String>,
}

/// Frontend-friendly pack struct sent to the webview.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pack {
    pub id: String,
    pub author_id: String,
    pub title: String,
    pub description: String,
    pub pack_type: String,
    pub author_name: String,
    pub is_anonymous: bool,
    pub vote_count: i64,
    pub install_count: i64,
    pub user_voted: bool,
    pub tags: Vec<String>,
    pub addons: Vec<PackAddonEntry>,
    pub created_at: String,
    pub updated_at: String,
    pub status: String,
}

impl Pack {
    fn from_hub(hub: HubPack) -> Self {
        let addons: Vec<PackAddonEntry> = match &hub.addons {
            serde_json::Value::String(s) => serde_json::from_str(s).unwrap_or_else(|e| {
                eprintln!(
                    "Warning: failed to parse addons JSON string for pack {}: {}",
                    hub.id, e
                );
                Vec::new()
            }),
            serde_json::Value::Array(_) => serde_json::from_value(hub.addons.clone())
                .unwrap_or_else(|e| {
                    eprintln!(
                        "Warning: failed to parse addons array for pack {}: {}",
                        hub.id, e
                    );
                    Vec::new()
                }),
            _ => {
                eprintln!(
                    "Warning: unexpected addons type for pack {}: {}",
                    hub.id, hub.addons
                );
                Vec::new()
            }
        };
        Self {
            id: hub.id,
            author_id: hub.author_id,
            title: hub.title,
            description: hub.description,
            pack_type: hub.pack_type,
            author_name: if hub.is_anonymous {
                "Anonymous".to_string()
            } else {
                hub.author_name
            },
            is_anonymous: hub.is_anonymous,
            vote_count: hub.vote_count,
            install_count: hub.install_count,
            user_voted: hub.user_voted.unwrap_or(false),
            tags: hub.tags,
            addons,
            created_at: hub.created_at,
            updated_at: hub.updated_at,
            status: hub.status.unwrap_or_else(|| "published".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackListResponse {
    pub packs: Vec<HubPack>,
    pub page: i64,
    pub sort: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackSingleResponse {
    pub pack: HubPack,
}

/// Response sent to the frontend with packs and the current page number.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackPage {
    pub packs: Vec<Pack>,
    pub page: i64,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn list_packs(
    app: tauri::AppHandle,
    pack_type: Option<String>,
    tag: Option<String>,
    query: Option<String>,
    sort: Option<String>,
    page: Option<i64>,
    author: Option<String>,
    status: Option<String>,
) -> Result<PackPage, String> {
    let access_token = pack_hub_read_token(&app).await;

    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/packs");

        let mut query_params: Vec<(&str, String)> = Vec::new();
        if let Some(t) = &pack_type {
            query_params.push(("type", t.clone()));
        }
        if let Some(t) = &tag {
            query_params.push(("tag", t.clone()));
        }
        if let Some(q) = &query {
            query_params.push(("q", q.clone()));
        }
        if let Some(s) = &sort {
            query_params.push(("sort", s.clone()));
        }
        if let Some(p) = &page {
            query_params.push(("page", p.to_string()));
        }
        if let Some(a) = &author {
            query_params.push(("author", a.clone()));
        }
        if let Some(st) = &status {
            query_params.push(("status", st.clone()));
        }

        let mut req = client.get(&url).query(&query_params);
        if let Some(token) = &access_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let response = req.send().map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                "Could not connect to Pack Hub. Check your internet connection.".to_string()
            } else {
                format!("Network error: {e}")
            }
        })?;

        if !response.status().is_success() {
            return Err(format!("Pack Hub returned HTTP {}", response.status()));
        }

        let body: PackListResponse = response
            .json()
            .map_err(|e| format!("Failed to parse packs response: {e}"))?;

        Ok(PackPage {
            packs: body.packs.into_iter().map(Pack::from_hub).collect(),
            page: body.page,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn get_pack(app: tauri::AppHandle, id: String) -> Result<Pack, String> {
    validate_pack_id(&id)?;
    let access_token = pack_hub_read_token(&app).await;

    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/packs/{id}");

        let mut req = client.get(&url);
        if let Some(token) = &access_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let response = req.send().map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                "Could not connect to Pack Hub. Check your internet connection.".to_string()
            } else {
                format!("Network error: {e}")
            }
        })?;

        match response.status().as_u16() {
            200 => {}
            404 => return Err(format!("Pack \"{id}\" not found.")),
            status => return Err(format!("Pack Hub returned HTTP {status}")),
        }

        let body: PackSingleResponse = response
            .json()
            .map_err(|e| format!("Failed to parse pack response: {e}"))?;

        Ok(Pack::from_hub(body.pack))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Resolve the access token for an authenticated Pack Hub / share call.
///
/// Refreshing goes through [`AuthState::get_valid_token_persisting`], which
/// holds `refresh_lock` across the token re-read, the refresh and the
/// credential-store write: two commands issued while the token is expired can
/// therefore neither POST the same (server-rotated) refresh_token twice nor
/// interleave their chunked credential writes. `not_signed_in` is returned when
/// there is no session at all.
async fn authed_pack_hub_token(
    app: &tauri::AppHandle,
    not_signed_in: &'static str,
) -> Result<String, String> {
    let app = app.clone();
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AuthState>();
        match state.get_valid_token_persisting(|tokens| {
            // Persistence failure is logged in the helper and keeps the
            // refreshed token working in-memory; don't fail the refresh.
            let _ = save_auth_tokens(&app, tokens);
        }) {
            Ok(Some(token)) => Ok(token),
            Ok(None) => Err(not_signed_in.to_string()),
            Err(e) => {
                clear_session_if_rejected(&state, &e);
                Err(e)
            }
        }
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// The access token for a Pack Hub *read*, refreshed when it has expired.
///
/// The worker treats an expired bearer as an anonymous viewer instead of
/// answering 401, so sending a stale token silently drops the caller's drafts
/// and own anonymous packs out of My Packs. Reads stay best-effort: with no
/// session — or when the refresh cannot be completed — the request goes out
/// anonymously rather than failing.
async fn pack_hub_read_token(app: &tauri::AppHandle) -> Option<String> {
    let app = app.clone();
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AuthState>();
        match state.get_valid_token_persisting(|tokens| {
            let _ = save_auth_tokens(&app, tokens);
        }) {
            Ok(token) => token,
            Err(e) => {
                clear_session_if_rejected(&state, &e);
                eprintln!("[auth] pack hub read continuing as anonymous: {e}");
                None
            }
        }
    })
    .await
    .unwrap_or(None)
}

// ── Vote response from the hub API ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoteResponse {
    pub voted: bool,
    pub vote_count: i64,
}

#[tauri::command]
pub async fn vote_pack(app: tauri::AppHandle, pack_id: String) -> Result<VoteResponse, String> {
    validate_pack_id(&pack_id)?;
    let access_token = authed_pack_hub_token(&app, "Sign in to vote on packs.").await?;

    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/packs/{pack_id}/vote");

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    "Could not connect to Pack Hub. Check your internet connection.".to_string()
                } else {
                    format!("Network error: {e}")
                }
            })?;

        match response.status().as_u16() {
            200 => {}
            401 => return Err("Session expired. Please sign in again.".to_string()),
            404 => return Err("Pack not found.".to_string()),
            429 => return Err("Too many votes. Please wait a moment.".to_string()),
            status => {
                let body = response.text().unwrap_or_default();
                return Err(format!("Pack Hub returned HTTP {status} — {body}"));
            }
        }

        let body: VoteResponse = response
            .json()
            .map_err(|e| format!("Failed to parse vote response: {e}"))?;

        Ok(body)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// ── Track pack install ──────────────────────────────────────────────────

#[tauri::command]
pub async fn track_pack_install(pack_id: String) -> Result<(), String> {
    validate_pack_id(&pack_id)?;

    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/packs/{pack_id}/install");

        // Fire-and-forget: best-effort tracking, don't block the user
        drop(client.post(&url).send());

        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePackPayload {
    pub title: String,
    pub description: String,
    pub pack_type: String,
    pub addons: Vec<PackAddonEntry>,
    pub tags: Vec<String>,
    pub is_anonymous: bool,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePackPayload {
    pub id: String,
    pub title: String,
    pub description: String,
    pub pack_type: String,
    pub addons: Vec<PackAddonEntry>,
    pub tags: Vec<String>,
    pub is_anonymous: bool,
    pub status: Option<String>,
}

#[tauri::command]
pub async fn create_pack(
    app: tauri::AppHandle,
    payload: CreatePackPayload,
) -> Result<Pack, String> {
    let access_token = authed_pack_hub_token(&app, "Not signed in. Please sign in first.").await?;

    // POST to Pack Hub API
    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/packs");

        let body = serde_json::json!({
            "title": payload.title,
            "description": payload.description,
            "pack_type": payload.pack_type,
            "addons": payload.addons,
            "tags": payload.tags,
            "is_anonymous": payload.is_anonymous,
            "status": payload.status.unwrap_or_else(|| "draft".to_string()),
        });

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    "Could not connect to Pack Hub. Check your internet connection.".to_string()
                } else {
                    format!("Network error: {e}")
                }
            })?;

        match response.status().as_u16() {
            200 | 201 => {}
            401 => return Err("Session expired. Please sign in again.".to_string()),
            429 => {
                return Err("Rate limit reached. Please wait before publishing again.".to_string())
            }
            status => {
                let body = response.text().unwrap_or_default();
                return Err(format!("Pack Hub returned HTTP {status} — {body}"));
            }
        }

        let body: PackSingleResponse = response
            .json()
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        Ok(Pack::from_hub(body.pack))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn update_pack(
    app: tauri::AppHandle,
    payload: UpdatePackPayload,
) -> Result<Pack, String> {
    validate_pack_id(&payload.id)?;
    let access_token = authed_pack_hub_token(&app, "Not signed in. Please sign in first.").await?;

    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{}/packs/{}", base, payload.id);

        let body = serde_json::json!({
            "title": payload.title,
            "description": payload.description,
            "pack_type": payload.pack_type,
            "addons": payload.addons,
            "tags": payload.tags,
            "is_anonymous": payload.is_anonymous,
            "status": payload.status,
        });

        let response = client
            .put(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    "Could not connect to Pack Hub. Check your internet connection.".to_string()
                } else {
                    format!("Network error: {e}")
                }
            })?;

        match response.status().as_u16() {
            200 => {}
            401 => return Err("Session expired. Please sign in again.".to_string()),
            403 => return Err("You can only edit packs you created.".to_string()),
            404 => return Err("Pack not found.".to_string()),
            status => {
                let body = response.text().unwrap_or_default();
                return Err(format!("Pack Hub returned HTTP {status} - {body}"));
            }
        }

        let body: PackSingleResponse = response
            .json()
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        Ok(Pack::from_hub(body.pack))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn delete_pack(app: tauri::AppHandle, id: String) -> Result<(), String> {
    validate_pack_id(&id)?;
    let access_token = authed_pack_hub_token(&app, "Not signed in. Please sign in first.").await?;

    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/packs/{id}");

        let response = client
            .delete(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    "Could not connect to Pack Hub. Check your internet connection.".to_string()
                } else {
                    format!("Network error: {e}")
                }
            })?;

        match response.status().as_u16() {
            200 => Ok(()),
            401 => Err("Session expired. Please sign in again.".to_string()),
            403 => Err("You can only delete packs you created.".to_string()),
            404 => Err("Pack not found.".to_string()),
            status => {
                let body = response.text().unwrap_or_default();
                Err(format!("Pack Hub returned HTTP {status} - {body}"))
            }
        }
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// ── Delete Account Data ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteAccountSummary {
    pub packs: u64,
    pub votes: u64,
    pub shares: u64,
}

#[tauri::command]
pub async fn delete_pack_hub_account(
    state: tauri::State<'_, AuthState>,
    app: tauri::AppHandle,
    upload_session: tauri::State<'_, Arc<StoredSessionProvider>>,
) -> Result<DeleteAccountSummary, String> {
    let access_token = authed_pack_hub_token(&app, "Not signed in. Please sign in first.").await?;

    let result = tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/account");

        let response = client
            .delete(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    "Could not connect to Pack Hub. Check your internet connection.".to_string()
                } else {
                    format!("Network error: {e}")
                }
            })?;

        match response.status().as_u16() {
            200 => {
                #[derive(Deserialize)]
                struct Resp {
                    deleted: DeleteAccountSummary,
                }
                let body: Resp = response
                    .json()
                    .map_err(|e| format!("Invalid response: {e}"))?;

                Ok(body.deleted)
            }
            401 => Err("Session expired. Please sign in again.".to_string()),
            status => {
                let body = response.text().unwrap_or_default();
                Err(format!("Pack Hub returned HTTP {status} - {body}"))
            }
        }
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))??;

    // Sign the user out after successful deletion
    *state
        .tokens
        .lock()
        .map_err(|e| format!("Auth lock poisoned: {e}"))? = None;
    clear_auth_and_upload_sessions(&app, &upload_session);

    Ok(result)
}

// ── Private Sharing ─────────────────────────────────────────────────────────

/// Base URL for the share worker (separate from the pack hub).
fn share_worker_url() -> &'static str {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| {
        std::env::var("SHARE_WORKER_URL")
            .unwrap_or_else(|_| "https://kalpa-pack-hub.eso-toolkit.workers.dev".to_string())
    })
}

fn share_worker_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .user_agent(format!("Kalpa/{}", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("failed to build share worker HTTP client")
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharePackPayload {
    pub title: String,
    pub description: String,
    pub pack_type: String,
    pub tags: Vec<String>,
    pub addons: Vec<PackAddonEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareCodeResponse {
    pub code: String,
    pub expires_at: String,
    pub deep_link: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedPack {
    pub title: String,
    pub description: String,
    pub pack_type: String,
    pub tags: Vec<String>,
    pub addons: Vec<PackAddonEntry>,
    pub shared_by: String,
    pub shared_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShareResolveResponse {
    pack: ShareResolvedPack,
    shared_by: String,
    shared_at: String,
    expires_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShareResolvedPack {
    title: String,
    description: String,
    pack_type: String,
    tags: Vec<String>,
    addons: Vec<PackAddonEntry>,
}

/// Validate a share code (6 chars from the unambiguous alphabet).
fn validate_share_code(code: &str) -> Result<(), String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^[23456789ABCDEFGHJKMNPQRSTUVWXYZ]{6}$").unwrap());
    if !re.is_match(code) {
        return Err("Invalid share code format.".to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn create_share_code(
    app: tauri::AppHandle,
    payload: SharePackPayload,
) -> Result<ShareCodeResponse, String> {
    let access_token = authed_pack_hub_token(&app, "Not signed in. Please sign in first.").await?;

    tokio::task::spawn_blocking(move || {
        let client = share_worker_client();
        let url = format!("{}/shares", share_worker_url());

        let body = serde_json::json!({
            "title": payload.title,
            "description": payload.description,
            "packType": payload.pack_type,
            "tags": payload.tags,
            "addons": payload.addons,
        });

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    "Could not connect to share service. Check your internet connection."
                        .to_string()
                } else {
                    format!("Network error: {e}")
                }
            })?;

        match response.status().as_u16() {
            200 | 201 => {}
            401 => return Err("Session expired. Please sign in again.".to_string()),
            429 => {
                return Err(
                    "Maximum share codes reached. Wait for existing codes to expire.".to_string(),
                )
            }
            status => {
                let body = response.text().unwrap_or_default();
                return Err(format!("Share service returned HTTP {status} — {body}"));
            }
        }

        let result: ShareCodeResponse = response
            .json()
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        Ok(result)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn resolve_share_code(code: String) -> Result<SharedPack, String> {
    validate_share_code(&code)?;

    tokio::task::spawn_blocking(move || {
        let client = share_worker_client();
        let url = format!("{}/shares/{}", share_worker_url(), code);

        let response = client.get(&url).send().map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                "Could not connect to share service. Check your internet connection.".to_string()
            } else {
                format!("Network error: {e}")
            }
        })?;

        match response.status().as_u16() {
            200 => {}
            400 => return Err("Invalid share code format.".to_string()),
            404 => return Err("Share code not found or expired.".to_string()),
            status => {
                let body = response.text().unwrap_or_default();
                return Err(format!("Share service returned HTTP {status} — {body}"));
            }
        }

        let result: ShareResolveResponse = response
            .json()
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        Ok(SharedPack {
            title: result.pack.title,
            description: result.pack.description,
            pack_type: result.pack.pack_type,
            tags: result.pack.tags,
            addons: result.pack.addons,
            shared_by: result.shared_by,
            shared_at: result.shared_at,
            expires_at: result.expires_at,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}
// ── Pack Export / Import (.esopack files) ───────────────────────────────────

/// Scrubbed SavedVariables for one addon stored in an `.esopack` v2 file.
///
/// `encoding` is always `"lua-text"` for Phase 1. `lua` is the scrubbed Lua
/// source with identity placeholders in place of real names/IDs. `scrub_report`
/// is the full scrub report (drops + templated keys) for user review on import.
/// `detected_identities` captures the `ScrubContext` used during export so the
/// importer knows the placeholder → real-name mapping strategy.
///
/// `original_bytes` — serialized size before any scrubbing.
/// `scrubbed_bytes` — size after identity scrubbing (before per-character strip).
/// `final_bytes`    — actual size of `lua` after the per-character strip; this
///                    is the true exported footprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonSettings {
    pub encoding: String,
    pub lua: String,
    pub original_bytes: usize,
    pub scrubbed_bytes: usize,
    /// Byte length of the exported `lua` string — accurate post-strip size.
    /// Absent in files produced before this field was added; defaults to 0.
    #[serde(default)]
    pub final_bytes: usize,
    #[serde(default)]
    pub scrub_summary: crate::saved_variables::scrub::ScrubSummary,
    #[allow(dead_code)]
    #[serde(default, skip_serializing)]
    pub detected_identities: Option<crate::saved_variables::scrub::ScrubContext>,
    #[allow(dead_code)]
    #[serde(default, skip_serializing)]
    pub scrub_report: Option<crate::saved_variables::scrub::ScrubReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EsoPackFile {
    pub format: String,
    pub version: u32,
    pub pack: EsoPackData,
    pub shared_at: String,
    pub shared_by: String,
    /// v2 only: scrubbed SavedVariables keyed by addon folder name.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub settings: HashMap<String, AddonSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EsoPackData {
    pub title: String,
    pub description: String,
    pub pack_type: String,
    pub tags: Vec<String>,
    pub addons: Vec<PackAddonEntry>,
}

fn pack_export_file_name(title: &str) -> String {
    let safe_name: String = title
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "-_ ".contains(*character))
        .collect();
    let safe_name = safe_name.split_whitespace().collect::<Vec<_>>().join("-");
    format!("{safe_name}.esopack")
}

fn export_pack_to_path(pack: EsoPackFile, file_path: &Path) -> Result<(), String> {
    if file_path.extension().and_then(|e| e.to_str()) != Some("esopack") {
        return Err("Export path must have .esopack extension.".to_string());
    }
    // Canonicalize the parent directory to prevent path traversal
    let parent = file_path
        .parent()
        .ok_or("Invalid file path: no parent directory.")?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|e| format!("Invalid directory: {e}"))?;
    let file_name = file_path
        .file_name()
        .ok_or("Invalid file path: no file name.")?;
    let file_path = canonical_parent.join(file_name);

    let json = serde_json::to_string_pretty(&pack)
        .map_err(|e| format!("Failed to serialize pack: {e}"))?;

    // Atomic write: write to .tmp then replace the destination in one step.
    // `fs::rename` replaces an existing destination atomically on Unix AND on
    // Windows (MoveFileExW with MOVEFILE_REPLACE_EXISTING), so removing the
    // previous export first would only open a window where a crash leaves the
    // user with no .esopack at all.
    crate::atomic_file::atomic_write(&file_path, json.as_bytes())
        .map_err(|error| format!("Failed to write file: {error}"))
}

#[tauri::command]
pub async fn export_pack_file(app: AppHandle, pack: EsoPackFile) -> Result<bool, String> {
    let file_name = pack_export_file_name(&pack.pack.title);
    let Some(selected_path) = app
        .dialog()
        .file()
        .add_filter("ESO Pack", &["esopack"])
        .set_file_name(file_name)
        .blocking_save_file()
    else {
        return Ok(false);
    };
    let file_path = selected_path
        .into_path()
        .map_err(|_| "The save dialog did not return a local file path.".to_string())?;
    export_pack_to_path(pack, &file_path)?;
    Ok(true)
}

fn import_pack_from_path(file_path: &Path) -> Result<EsoPackFile, String> {
    if file_path.extension().and_then(|e| e.to_str()) != Some("esopack") {
        return Err("Only .esopack files can be imported.".to_string());
    }

    // Canonicalize to resolve any traversal components (also verifies existence)
    let file_path = file_path
        .canonicalize()
        .map_err(|_| "File not found.".to_string())?;

    let metadata = fs::metadata(&file_path).map_err(|e| format!("Failed to read file: {e}"))?;
    if metadata.len() > 10 * 1024 * 1024 {
        return Err("File is too large (max 10 MB).".to_string());
    }

    let contents =
        fs::read_to_string(&file_path).map_err(|e| format!("Failed to read file: {e}"))?;

    let pack: EsoPackFile =
        serde_json::from_str(&contents).map_err(|e| format!("Invalid .esopack file: {e}"))?;

    if pack.format != "esopack" {
        return Err("Not a valid .esopack file (wrong format field).".to_string());
    }

    if pack.version != 1 && pack.version != 2 {
        return Err(format!(
            "Unsupported .esopack version {}. Please update the app.",
            pack.version
        ));
    }

    Ok(pack)
}

#[tauri::command]
pub async fn import_pack_file(app: AppHandle) -> Result<Option<EsoPackFile>, String> {
    let Some(selected_path) = app
        .dialog()
        .file()
        .add_filter("ESO Pack", &["esopack"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let file_path = selected_path
        .into_path()
        .map_err(|_| "The open dialog did not return a local file path.".to_string())?;
    import_pack_from_path(&file_path).map(Some)
}
/// Export the SavedVariables settings block for a list of addon folder names.
///
/// For each addon, reads the corresponding `.lua` file from the SavedVariables
/// directory, detects identities, scrubs the tree, and returns an `AddonSettings`
/// map keyed by addon folder name. Only `$AccountWide` subtrees are retained
/// (per-character data is not exported in Phase 1).
///
/// An addon is left out of the map when its file is absent, larger than the
/// export cap, or fails to parse — the map is best-effort, not exhaustive.
///
/// The caller merges this map into an `EsoPackFile` and writes it with
/// `export_pack_file`.
#[tauri::command]
pub async fn export_sv_settings(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    addon_folders: Vec<String>,
) -> Result<HashMap<String, AddonSettings>, String> {
    use crate::saved_variables::parser::parse_sv_file;
    use crate::saved_variables::scrub::{detect_identities_from_tree, scrub};
    use crate::saved_variables::serializer::serialize_to_lua;

    let addons_dir = require_allowed_path(&state, &addons_path)?;

    tokio::task::spawn_blocking(move || {
        let sv_dir = sv_io::saved_variables_dir(&addons_dir);
        let mut result: HashMap<String, AddonSettings> = HashMap::new();

        // Parsing a file into a full `SvTreeNode` tree costs several times the
        // source size, so bound it the way every other SavedVariables read path
        // does instead of letting one bloated addon file (MasterMerchant-class
        // files reach hundreds of MB) become a multi-GB transient.
        const MAX_EXPORT_SIZE: u64 = 20 * 1024 * 1024; // 20 MB

        for folder in &addon_folders {
            // Every sibling SavedVariables command validates the frontend-supplied
            // name before joining it into a path; without this a traversal or
            // absolute value would escape the SavedVariables directory.
            if let Err(e) = validate_name(folder) {
                eprintln!("export_sv_settings: skipping invalid folder name '{folder}': {e}");
                continue;
            }

            let sv_file = sv_dir.join(format!("{folder}.lua"));
            if !sv_file.is_file() {
                continue;
            }

            let file_size = fs::metadata(&sv_file).map(|m| m.len()).unwrap_or(0);
            if file_size > MAX_EXPORT_SIZE {
                eprintln!(
                    "export_sv_settings: skipping {} — {:.1} MB exceeds the {} MB export cap",
                    sv_file.display(),
                    file_size as f64 / (1024.0 * 1024.0),
                    MAX_EXPORT_SIZE / (1024 * 1024)
                );
                continue;
            }

            let bytes = match fs::read(&sv_file) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!(
                        "export_sv_settings: failed to read {}: {}",
                        sv_file.display(),
                        e
                    );
                    continue;
                }
            };
            // SavedVariables files legitimately contain non-UTF8 bytes and the
            // parser tolerates lossy content, so decode lossily rather than
            // dropping the addon from a pack the user believes carries its
            // settings.
            let content = match String::from_utf8(bytes) {
                Ok(s) => s,
                Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
            };

            let file_name = format!("{folder}.lua");
            let tree = match parse_sv_file(&content, &file_name) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("export_sv_settings: failed to parse {file_name}: {e}");
                    continue;
                }
            };

            let ctx = detect_identities_from_tree(&tree);
            // `scrub` consumes `tree` (mutating it in place); `ctx` was already
            // computed from it above, and it is not used afterwards.
            let (scrubbed, report) = scrub(tree, &ctx);

            let account_wide_only = strip_per_character_data(scrubbed);
            let lua = serialize_to_lua(&account_wide_only);
            let final_bytes = lua.len();

            result.insert(
                folder.clone(),
                AddonSettings {
                    encoding: "lua-text".to_string(),
                    lua,
                    original_bytes: report.original_bytes,
                    scrubbed_bytes: report.scrubbed_bytes,
                    final_bytes,
                    scrub_summary: (&report).into(),
                    detected_identities: None,
                    scrub_report: None,
                },
            );
        }

        Ok(result)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

fn strip_per_character_data(
    tree: crate::saved_variables::types::SvTreeNode,
) -> crate::saved_variables::types::SvTreeNode {
    crate::saved_variables::scrub::strip_per_character_data(tree)
}

/// Result of importing SavedVariables settings from a v2 `.esopack` file.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SvImportResult {
    /// Addons whose SV files were successfully written.
    pub applied: Vec<String>,
    /// Addons skipped because their SV file was not in the pack.
    pub skipped: Vec<String>,
    /// Addons where the import failed; contains error messages.
    pub errors: Vec<String>,
}

/// Does `lua` still carry an identity placeholder the importer could not resolve?
///
/// World tokens count. They did not used to: every world collapsed to a single
/// `${WORLD}` that was always substituted, so an unresolved one was impossible.
/// Now that world tokens are mapped one-to-one, a leftover is a NORMAL outcome —
/// a two-megaserver export imported by a one-megaserver player deliberately
/// leaves the extra layer's token alone rather than clobbering a layer the
/// importer does use. Writing that file anyway produced SavedVariables keyed on
/// a literal `${WORLD:1}`, which ESO never reads, while the UI reported the
/// addon as applied. Refusing is the honest answer: the user is told this
/// addon's settings could not be mapped instead of silently getting nothing.
///
/// Re-exported from `saved_variables::scrub` rather than defined here: the
/// Slint sidecar reaches the same writer through the same shared scrub module,
/// and while this guard was a private copy on each side only one of them
/// learned about world tokens.
use crate::saved_variables::scrub::has_unresolved_identity_placeholders;

/// Import SavedVariables settings from a v2 `.esopack` file.
///
/// For each addon in `addon_folders` that has a corresponding entry in the
/// pack's `settings` map, substitutes identity placeholders with the real
/// account/character identities from `ctx`, then writes the resulting Lua to
/// the SavedVariables directory. A `.bak` copy of the existing file is created
/// before each overwrite.
///
/// `ctx` must describe the *importer's* identities (not the exporter's). The
/// substitution maps:
///   `${ACCOUNT}` → `ctx.accounts[0]`
///   `${ACCOUNT:N}` → `ctx.accounts[N]`
///   `${CHAR:N}` → `ctx.characters[N]`
///   `${CHAR_ID:N}` → `ctx.character_ids[N]`
///
/// World tokens do NOT follow that shape and are not documented here on
/// purpose: they are allocated one-to-one by `substitute_placeholders`, whose
/// three rules are the specification. Resolving `${WORLD}` to a canonical name
/// the importer does not play on — which is what this doc-block used to
/// describe — is the exact bug those rules exist to prevent, because it writes
/// EU or PTS settings under `NA Megaserver` where ESO never looks.
///
/// A world token may therefore legitimately survive substitution.
///
/// Placeholder tokens that have no mapping in `ctx` are rejected — the
/// import is skipped and an error is returned for that addon.
#[tauri::command]
pub async fn import_sv_settings(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
    settings: HashMap<String, AddonSettings>,
    ctx: crate::saved_variables::scrub::ScrubContext,
    addon_folders: Vec<String>,
) -> Result<SvImportResult, String> {
    use crate::saved_variables::parser::parse_sv_file;
    use crate::saved_variables::scrub::WELL_KNOWN_WORLDS;

    let addons_dir = require_allowed_path(&state, &addons_path)?;

    tokio::task::spawn_blocking(move || {
        let sv_dir = sv_io::saved_variables_dir(&addons_dir);
        fs::create_dir_all(&sv_dir)
            .map_err(|e| format!("Failed to create SavedVariables directory: {e}"))?;

        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        let mut errors = Vec::new();

        for folder in &addon_folders {
            if let Err(e) = validate_name(folder) {
                errors.push(format!("{folder}: invalid folder name: {e}"));
                continue;
            }

            let entry = match settings.get(folder.as_str()) {
                Some(e) => e,
                None => {
                    skipped.push(folder.clone());
                    continue;
                }
            };

            if entry.encoding != "lua-text" {
                errors.push(format!(
                    "{}: unsupported encoding '{}'",
                    folder, entry.encoding
                ));
                continue;
            }

            let substituted = crate::saved_variables::scrub::substitute_placeholders(
                &entry.lua,
                &ctx,
                WELL_KNOWN_WORLDS,
            );

            // Refuse anything that still carries an unresolved identity
            // placeholder. Writing it would key settings ESO never reads while
            // the UI reported success.
            //
            // A partial import — pruning only the unmappable layers and writing
            // the rest — was implemented here and REVERTED. Substitution runs on
            // the raw Lua text before parsing, so a placeholder-shaped literal in
            // an ordinary key or value is rewritten before any tree-level guard
            // can see it, and world-slot allocation counts those literals too:
            // a stray `${WORLD}` in a value could consume the importer's only
            // world and cause the real identity layer to be dropped, while the
            // file still had content and was written as a partial success. Three
            // successive attempts to guard that at the tree level each missed a
            // shape, one of them overwriting real settings with an empty shell.
            // Doing it safely needs substitution to work on parsed identity KEYS
            // rather than text — tracked as follow-up. Until then, refuse.
            if has_unresolved_identity_placeholders(&substituted) {
                // Shared with the sidecar, which refuses the same files through
                // the same guard — see `unresolved_identity_advice`.
                let advice = crate::saved_variables::scrub::unresolved_identity_advice(&ctx);
                errors.push(format!(
                    "{folder}: settings could not be mapped to your account. {advice}"
                ));
                continue;
            }

            // Validate that the result is a well-formed SavedVariables file
            let file_name = format!("{folder}.lua");
            if let Err(e) = parse_sv_file(&substituted, &file_name) {
                errors.push(format!("{folder}: settings file failed validation: {e}"));
                continue;
            }

            let dest = sv_dir.join(format!("{folder}.lua"));

            // Create .bak before overwriting
            if dest.is_file() {
                let bak = dest.with_extension("lua.bak");
                if let Err(e) = fs::copy(&dest, &bak) {
                    errors.push(format!("{folder}: failed to create backup: {e}"));
                    continue;
                }
            }

            // Atomic write. `fs::rename` replaces the destination atomically on
            // Unix AND on Windows (MoveFileExW with MOVEFILE_REPLACE_EXISTING),
            // so the live file is never removed first — a failure here leaves
            // the previous settings in place instead of no file at all.
            if let Err(e) = write_imported_sv(&dest, substituted.as_bytes()) {
                errors.push(format!("{folder}: failed to write: {e}"));
                continue;
            }

            applied.push(folder.clone());
        }

        Ok(SvImportResult {
            applied,
            skipped,
            errors,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

pub(crate) fn write_imported_sv(destination: &Path, content: &[u8]) -> Result<(), String> {
    crate::atomic_file::atomic_write(destination, content).map_err(|error| error.to_string())
}

/// Walk every SavedVariables `.lua` file and accumulate the merged
/// account/character identities found across all of them. Used by the scrub/
/// import path, which must see every identity, so it is intentionally uncapped.
///
/// Streams each file's raw bytes through
/// [`detect_identities_streaming`](crate::saved_variables::identity_stream), the
/// bounded-memory scanner that emits the SAME identities as the tree-based
/// `detect_identities_from_tree` (verified by a parity test) — instead of
/// parsing every `.lua` into a full `SvTreeNode` tree (~10x the source size),
/// which on a 1–2 GB SavedVariables file was a multi-GB transient. Memory is now
/// `O(nesting depth + one key)` per file regardless of file size, so the export
/// path no longer needs a size cap to stay safe.
///
/// Runs synchronously; callers wrap it in `spawn_blocking`.
fn collect_local_identities(addons_dir: &Path) -> crate::saved_variables::scrub::ScrubContext {
    use crate::saved_variables::identity_stream::detect_identities_streaming;
    use crate::saved_variables::scrub::ScrubContext;

    let sv_dir = sv_io::saved_variables_dir(addons_dir);
    let entries = match fs::read_dir(&sv_dir) {
        Ok(e) => e,
        Err(_) => return ScrubContext::default(),
    };

    let mut merged = ScrubContext::default();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("lua") {
            continue;
        }
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let ctx = match detect_identities_streaming(std::io::BufReader::new(file)) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for acc in ctx.accounts {
            if !merged.accounts.contains(&acc) {
                merged.accounts.push(acc);
            }
        }
        for ch in ctx.characters {
            if !merged.characters.contains(&ch) {
                merged.characters.push(ch);
            }
        }
        for id in ctx.character_ids {
            if !merged.character_ids.contains(&id) {
                merged.character_ids.push(id);
            }
        }
        for w in ctx.extra_worlds {
            if !merged.extra_worlds.contains(&w) {
                merged.extra_worlds.push(w);
            }
        }
    }
    merged
}
/// Detect the account/character identities present in the local SavedVariables
/// directory. Reads any available `.lua` file that parses successfully and
/// accumulates identities across all of them. Returns the merged `ScrubContext`.
///
/// The frontend passes this context to `import_sv_settings` so that
/// placeholder tokens from a v2 `.esopack` file can be substituted with the
/// local player's real names.
#[tauri::command]
pub async fn detect_local_identities(
    state: tauri::State<'_, AllowedAddonsPath>,
    addons_path: String,
) -> Result<crate::saved_variables::scrub::ScrubContext, String> {
    let addons_dir = require_allowed_path(&state, &addons_path)?;

    tokio::task::spawn_blocking(move || -> Result<_, String> {
        Ok(collect_local_identities(&addons_dir))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}
// ── Roster Pack Install (deep link: kalpa://install-pack/{id}) ────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RosterPack {
    pub id: String,
    pub title: String,
    pub addons: Vec<PackAddonEntry>,
}

#[tauri::command]
pub async fn fetch_roster_pack(pack_id: String) -> Result<RosterPack, String> {
    validate_pack_id(&pack_id)?;

    tokio::task::spawn_blocking(move || {
        let client = pack_hub_client();
        let base = pack_hub_url();
        let url = format!("{base}/packs/{pack_id}");

        let response = client.get(&url).send().map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                "Could not connect to Pack Hub. Check your internet connection.".to_string()
            } else {
                format!("Network error: {e}")
            }
        })?;

        match response.status().as_u16() {
            200 => {}
            404 => return Err(format!("Pack \"{pack_id}\" not found.")),
            status => return Err(format!("Pack Hub returned HTTP {status}")),
        }

        let body: PackSingleResponse = response
            .json()
            .map_err(|e| format!("Failed to parse pack response: {e}"))?;

        let pack = Pack::from_hub(body.pack);
        Ok(RosterPack {
            id: pack.id,
            title: pack.title,
            addons: pack.addons,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_pack_id_accepts_valid_ids() {
        assert!(validate_pack_id("trial-essentials").is_ok());
        assert!(validate_pack_id("my_pack_123").is_ok());
        assert!(validate_pack_id("abc").is_ok());
        assert!(validate_pack_id("A-Z_0-9").is_ok());
    }

    #[test]
    fn validate_pack_id_rejects_path_traversal() {
        assert!(validate_pack_id("../admin").is_err());
        assert!(validate_pack_id("..%2Fadmin").is_err());
        assert!(validate_pack_id("foo/bar").is_err());
        assert!(validate_pack_id("foo\\bar").is_err());
    }

    #[test]
    fn validate_pack_id_rejects_empty() {
        assert!(validate_pack_id("").is_err());
    }

    #[test]
    fn validate_pack_id_rejects_special_chars() {
        assert!(validate_pack_id("id with spaces").is_err());
        assert!(validate_pack_id("id&param=1").is_err());
        assert!(validate_pack_id("<script>").is_err());
        assert!(validate_pack_id("id%20encoded").is_err());
    }

    #[test]
    fn validate_pack_id_rejects_over_100_chars() {
        let long_id = "a".repeat(101);
        assert!(validate_pack_id(&long_id).is_err());
        let max_id = "a".repeat(100);
        assert!(validate_pack_id(&max_id).is_ok());
    }

    #[test]
    fn export_pack_file_rejects_non_esopack_extension() {
        let pack = EsoPackFile {
            format: "esopack".to_string(),
            version: 1,
            pack: EsoPackData {
                title: "Test".to_string(),
                description: String::new(),
                pack_type: "addon-pack".to_string(),
                tags: vec![],
                addons: vec![],
            },
            shared_at: String::new(),
            shared_by: String::new(),
            settings: HashMap::new(),
        };
        assert!(export_pack_to_path(pack.clone(), Path::new("C:\\test.json")).is_err());
        assert!(export_pack_to_path(pack, Path::new("C:\\test.exe")).is_err());
    }

    #[test]
    fn export_pack_file_replaces_an_existing_export_by_rename() {
        // The export renames over the destination without removing it first,
        // which only works because fs::rename replaces atomically on Windows
        // too. If that ever stops holding, this fails instead of the user
        // losing a previous export to a crash between remove and rename.
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("pack.esopack");
        fs::write(&dest, b"previous export").unwrap();

        let pack = EsoPackFile {
            format: "esopack".to_string(),
            version: 1,
            pack: EsoPackData {
                title: "Replacement".to_string(),
                description: String::new(),
                pack_type: "addon-pack".to_string(),
                tags: vec![],
                addons: vec![],
            },
            shared_at: String::new(),
            shared_by: String::new(),
            settings: HashMap::new(),
        };
        export_pack_to_path(pack, &dest).unwrap();

        let written = fs::read_to_string(&dest).unwrap();
        assert!(written.contains("Replacement"));
        assert!(
            !tmp.path().join("pack.esopack.tmp").exists(),
            "the temp file must not survive a successful export"
        );
    }

    #[test]
    fn concurrent_pack_exports_never_share_a_staging_file() {
        let temp = tempfile::tempdir().unwrap();
        let destination = std::sync::Arc::new(temp.path().join("pack.esopack"));
        let start = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|writer| {
                let destination = destination.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    let pack = EsoPackFile {
                        format: "esopack".to_string(),
                        version: 1,
                        pack: EsoPackData {
                            title: format!("Writer {writer}"),
                            description: String::new(),
                            pack_type: "addon-pack".to_string(),
                            tags: vec![],
                            addons: vec![],
                        },
                        shared_at: String::new(),
                        shared_by: String::new(),
                        settings: HashMap::new(),
                    };
                    start.wait();
                    export_pack_to_path(pack, &destination).unwrap();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }

        let contents = fs::read_to_string(destination.as_ref()).unwrap();
        serde_json::from_str::<EsoPackFile>(&contents).unwrap();
        assert!(fs::read_dir(temp.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(crate::atomic_file::STAGING_INFIX)
        }));
    }

    /// The worker now returns `user_voted` per viewer; the client's vote button
    /// is a toggle, so absorbing it as `false` makes an "Upvote" click delete
    /// the vote the user already cast.
    #[test]
    fn hub_pack_carries_the_workers_user_voted_through_to_the_frontend() {
        let body = serde_json::json!({
            "id": "raid-starter",
            "author_id": "123",
            "author_name": "Faewynd",
            "is_anonymous": false,
            "title": "Raid Starter",
            "description": "Trials essentials",
            "pack_type": "raid",
            "addons": [
                { "esouiId": 7, "name": "LibAddonMenu", "required": true },
                { "esouiId": 8, "name": "CombatMetrics", "required": false, "note": "optional" }
            ],
            "vote_count": 12,
            "install_count": 3,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-02T00:00:00Z",
            "tags": ["trials"],
            "user_voted": true,
            "status": "published",
        });

        let hub: HubPack = serde_json::from_value(body).expect("hub pack");
        let pack = Pack::from_hub(hub);

        assert!(pack.user_voted);
        assert_eq!(pack.vote_count, 12);
        assert_eq!(pack.addons.len(), 2);
        assert_eq!(pack.addons[0].esoui_id, 7);
        assert!(pack.addons[0].required);
        assert_eq!(pack.addons[1].note.as_deref(), Some("optional"));
    }

    /// The D1 mirror hands `addons` back as a JSON string, and an older worker
    /// omits `user_voted` entirely — both must still deserialize.
    #[test]
    fn hub_pack_accepts_string_addons_and_a_missing_user_voted() {
        let body = serde_json::json!({
            "id": "raid-starter",
            "author_name": "Faewynd",
            "is_anonymous": true,
            "title": "Raid Starter",
            "description": "",
            "pack_type": "raid",
            "addons": "[{\"esouiId\":7,\"name\":\"LibAddonMenu\"}]",
            "vote_count": 0,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "tags": [],
        });

        let hub: HubPack = serde_json::from_value(body).expect("hub pack");
        let pack = Pack::from_hub(hub);

        assert!(!pack.user_voted);
        // An anonymous pack must never carry the author's real name forward.
        assert_eq!(pack.author_name, "Anonymous");
        assert_eq!(pack.author_id, "");
        assert_eq!(pack.status, "published");
        assert_eq!(pack.addons.len(), 1);
        // `required` defaults to true when the worker omits it.
        assert!(pack.addons[0].required);
    }
}
