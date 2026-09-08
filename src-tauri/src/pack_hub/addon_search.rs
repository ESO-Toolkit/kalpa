//! Client for the Pack Hub worker's ESOUI addon full-text index.
//!
//! Kalpa's Discover search calls `esoui.com/downloads/search.php`, which matches
//! addon TITLES. Descriptions are never searched, because the bulk filelist API
//! does not carry them — so a question phrased the way players actually think
//! ("something that shows when I'm flagged in combat") finds nothing, even
//! though several addons say exactly that in their description text.
//!
//! The worker indexes title + author + category + description in D1/FTS5 and
//! serves BM25 results from `GET /addons/search`. This module is the client for
//! that route and for `POST /ask`, the natural-language assistant built on the
//! same index.
//!
//! **The index is treated as an enhancement, never a dependency.** If the
//! binding is unconfigured, the worker is unreachable, or the query returns
//! nothing, [`search_addon_index`] falls back to the existing scraper in
//! `crate::esoui`. Search is therefore never worse than it is today, and the
//! feature degrades to current behaviour when offline instead of breaking.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::esoui::EsouiSearchResult;

/// Matches `POPULAR_PAGE_SIZE` in `esoui.rs` so Discover's pagination feels the
/// same whichever backend answered.
const PAGE_SIZE: u32 = 25;

/// Which backend produced a result set. Surfaced so the UI can tell the user
/// when it is showing older title-only results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchSource {
    /// The worker's full-text index.
    Index,
    /// The ESOUI website scraper, used when the index could not answer.
    Esoui,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonSearchPage {
    pub results: Vec<EsouiSearchResult>,
    pub has_more: bool,
    pub source: SearchSource,
}

// ── Worker response shapes (snake_case, matching addon-index.ts) ───────────

#[derive(Debug, Clone, Deserialize)]
struct HubAddonHit {
    esoui_id: u32,
    title: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    last_update: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct HubSearchResponse {
    #[serde(default)]
    hits: Vec<HubAddonHit>,
}

impl From<HubAddonHit> for EsouiSearchResult {
    fn from(hit: HubAddonHit) -> Self {
        EsouiSearchResult {
            id: hit.esoui_id,
            title: hit.title,
            author: hit.author,
            category: hit.category,
            downloads: crate::esoui::format_download_count(hit.downloads),
            updated: crate::esoui::format_epoch_millis(hit.last_update),
        }
    }
}

fn addon_index_url() -> &'static str {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| {
        std::env::var("PACK_HUB_API_URL")
            .unwrap_or_else(|_| "https://kalpa-pack-hub.eso-toolkit.workers.dev".to_string())
    })
}

/// Dedicated client with a short timeout.
///
/// Deliberately tighter than the 15s Pack Hub client: this call sits directly
/// under a search box, and a user who typed a query and got nothing for fifteen
/// seconds is worse off than one who fell back to the scraper after four.
fn search_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .user_agent(format!("Kalpa/{}", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(4))
            .build()
            .expect("failed to build addon search HTTP client")
    })
}

/// Query the worker index. `Ok(None)` means "the index could not answer" —
/// unconfigured, unreachable, or zero hits — which is the caller's cue to fall
/// back rather than to show an error.
fn query_index(query: &str, page: u32) -> Option<Vec<EsouiSearchResult>> {
    let url = format!("{}/addons/search", addon_index_url());
    let offset = page.saturating_mul(PAGE_SIZE);

    let response = search_client()
        .get(&url)
        .query(&[
            ("q", query.to_string()),
            ("limit", PAGE_SIZE.to_string()),
            ("offset", offset.to_string()),
        ])
        .send()
        .ok()?;

    // 503 is the documented "index not configured on this deployment" answer.
    // Anything non-2xx is treated the same way: fall back, do not surface it.
    if !response.status().is_success() {
        return None;
    }

    let body: HubSearchResponse = response.json().ok()?;
    if body.hits.is_empty() {
        return None;
    }
    Some(body.hits.into_iter().map(EsouiSearchResult::from).collect())
}

/// Search addons, preferring the full-text index and falling back to ESOUI.
#[tauri::command]
pub async fn search_addon_index(
    query: String,
    page: Option<u32>,
) -> Result<AddonSearchPage, String> {
    let trimmed = query.trim().to_string();
    if trimmed.is_empty() {
        return Ok(AddonSearchPage {
            results: Vec::new(),
            has_more: false,
            source: SearchSource::Index,
        });
    }
    let page = page.unwrap_or(0);

    tokio::task::spawn_blocking(move || {
        if let Some(results) = query_index(&trimmed, page) {
            let has_more = results.len() as u32 == PAGE_SIZE;
            return Ok(AddonSearchPage {
                results,
                has_more,
                source: SearchSource::Index,
            });
        }

        // The scraper has no paging, so only page 0 can be served this way.
        // Reporting an empty final page is correct: there is genuinely nothing
        // more to show, and pretending otherwise would loop the infinite scroll.
        if page > 0 {
            return Ok(AddonSearchPage {
                results: Vec::new(),
                has_more: false,
                source: SearchSource::Esoui,
            });
        }

        let results = crate::esoui::search_esoui(&trimmed)?;
        Ok(AddonSearchPage {
            results,
            has_more: false,
            source: SearchSource::Esoui,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// ── Ask: natural-language addon assistant ─────────────────────────────────

/// One recommended addon. Mirrors `AskRecommendation` in the worker.
///
/// `file_info_uri` is always rebuilt server-side from the index row — the model
/// never emits a URL — so this can be linked without further validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskRecommendation {
    #[serde(rename = "esoui_id")]
    pub esoui_id: u32,
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub category: String,
    #[serde(rename = "file_info_uri", default)]
    pub file_info_uri: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskResponse {
    /// Prose answer. Empty when `degraded` is set.
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub recommendations: Vec<AskRecommendation>,
    /// Ranked candidates the model did not pick. Free (no extra model call);
    /// shown collapsed so a short answer does not look like it missed things.
    #[serde(rename = "also_considered", default)]
    pub also_considered: Vec<AskRecommendation>,
    #[serde(rename = "no_good_match", default)]
    pub no_good_match: bool,
    /// Ranked candidates shown without model prose, because the model was
    /// unavailable, over its daily budget, or returned ungroundable output.
    #[serde(default)]
    pub degraded: bool,
    #[serde(default)]
    pub cached: bool,
}

#[derive(Serialize)]
struct AskRequest<'a> {
    question: &'a str,
}

/// Longer than the search timeout: a model call is expected to take seconds,
/// and there is no cheaper local fallback to race it against.
const ASK_TIMEOUT_SECS: u64 = 20;

fn ask_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .user_agent(format!("Kalpa/{}", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(ASK_TIMEOUT_SECS))
            .build()
            .expect("failed to build ask HTTP client")
    })
}

/// Ask the assistant a natural-language question about addons.
///
/// Unlike [`search_addon_index`] there is no local fallback — the whole feature
/// lives server-side — so failures surface as errors the UI can explain rather
/// than being silently swallowed.
#[tauri::command]
pub async fn ask_addon_assistant(question: String) -> Result<AskResponse, String> {
    let trimmed = question.trim().to_string();
    if trimmed.is_empty() {
        return Err("Enter a question first.".to_string());
    }

    tokio::task::spawn_blocking(move || {
        let url = format!("{}/ask", addon_index_url());
        let response = ask_client()
            .post(&url)
            .json(&AskRequest { question: &trimmed })
            .send()
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    "Could not reach the addon assistant. Check your internet connection."
                        .to_string()
                } else {
                    format!("Network error: {e}")
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(match status.as_u16() {
                400 => "That question could not be understood. Try rephrasing it.".to_string(),
                429 => "Too many questions right now. Wait a moment and try again.".to_string(),
                503 => "The addon assistant is not available yet.".to_string(),
                other => format!("The addon assistant returned HTTP {other}."),
            });
        }

        response
            .json::<AskResponse>()
            .map_err(|e| format!("Failed to parse assistant response: {e}"))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: u32, downloads: u64) -> HubAddonHit {
        HubAddonHit {
            esoui_id: id,
            title: "CombatIndicator".to_string(),
            author: "Author".to_string(),
            category: "Combat Mods".to_string(),
            downloads,
            last_update: 1_700_000_000_000,
        }
    }

    #[test]
    fn maps_hub_hit_into_the_shape_discover_already_renders() {
        let mapped: EsouiSearchResult = hit(1543, 12_500).into();
        assert_eq!(mapped.id, 1543);
        assert_eq!(mapped.title, "CombatIndicator");
        assert_eq!(mapped.category, "Combat Mods");
        // Counts arrive as integers and are formatted here, matching how
        // browse_popular presents them.
        assert_eq!(mapped.downloads, "12.5K");
        assert!(!mapped.updated.is_empty());
    }

    #[test]
    fn tolerates_a_hit_missing_every_optional_field() {
        let json = r#"{"hits":[{"esoui_id":7,"title":"Bare"}]}"#;
        let parsed: HubSearchResponse = serde_json::from_str(json).expect("should parse");
        let mapped: EsouiSearchResult = parsed.hits.into_iter().next().unwrap().into();
        assert_eq!(mapped.id, 7);
        assert_eq!(mapped.downloads, "0");
    }

    #[test]
    fn parses_a_grounded_ask_response() {
        let json = r#"{
            "answer": "Yes, CombatIndicator does that.",
            "recommendations": [{
                "esoui_id": 1543,
                "title": "CombatIndicator",
                "author": "A",
                "category": "Combat Mods",
                "file_info_uri": "https://www.esoui.com/downloads/info1543.html",
                "reason": "shows a flag while in combat"
            }],
            "no_good_match": false,
            "degraded": false,
            "cached": true
        }"#;
        let parsed: AskResponse = serde_json::from_str(json).expect("should parse");
        assert!(parsed.cached);
        assert!(!parsed.degraded);
        assert_eq!(parsed.recommendations[0].esoui_id, 1543);
        assert_eq!(
            parsed.recommendations[0].file_info_uri,
            "https://www.esoui.com/downloads/info1543.html"
        );
    }

    #[test]
    fn parses_a_degraded_ask_response_with_no_prose() {
        // The model-unavailable path still returns usable candidates.
        let json = r#"{"recommendations":[{"esoui_id":7,"title":"X"}],"degraded":true}"#;
        let parsed: AskResponse = serde_json::from_str(json).expect("should parse");
        assert!(parsed.answer.is_empty());
        assert!(parsed.degraded);
        assert_eq!(parsed.recommendations.len(), 1);
    }

    #[test]
    fn ignores_unknown_fields_so_the_worker_can_add_some() {
        let json =
            r#"{"hits":[{"esoui_id":1,"title":"X","snippet":"s","score":1.5}],"mode":"and"}"#;
        let parsed: HubSearchResponse = serde_json::from_str(json).expect("should parse");
        assert_eq!(parsed.hits.len(), 1);
    }
}
