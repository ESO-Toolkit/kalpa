use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, Seek};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tempfile::NamedTempFile;

// ── ESOUI filedetails JSON API ──────────────────────────────────────────────

/// Response from `api.mmoui.com/v4/game/ESO/filedetails/{id}.json`.
/// The API wraps the result in a single-element array.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiFileDetail {
    id: u32,
    title: String,
    version: String,
    author: String,
    description: String,
    last_update: u64,
    checksum: String,
    download_uri: String,
    downloads: u64,
    downloads_monthly: u64,
    favorites: u64,
    /// Full version history as one BBCode blob, newest first. The API omits it
    /// on some entries and sends the literal string "None" on others, so it is
    /// defaulted here and normalised in [`fetch_addon_detail`].
    #[serde(default)]
    change_log: String,
    #[serde(default)]
    images: Vec<ApiImage>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiImage {
    image_url: String,
}

/// How long a `filedetails` response stays reusable.
///
/// Sized to collapse the resolve-then-install pair — the Discover flow calls
/// `resolve_esoui_addon` and then `install_addon` seconds apart, and both need
/// the same detail record, so without this an install costs two identical
/// uncached round trips. Short enough that nothing has to invalidate it: the
/// only consumers are download URLs and checksums for an install happening right
/// now, while update DETECTION reads the separate filelist cache, which keeps
/// its own invalidation. A minute-old detail record cannot mislead either.
const DETAIL_TTL: Duration = Duration::from_secs(60);

static DETAIL_CACHE: OnceLock<Mutex<HashMap<u32, (Instant, ApiFileDetail)>>> = OnceLock::new();

fn detail_cache() -> &'static Mutex<HashMap<u32, (Instant, ApiFileDetail)>> {
    DETAIL_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A still-fresh cached record, or `None`. A poisoned lock reads as a miss so a
/// caller always degrades to a live fetch rather than to an error.
fn cached_file_detail(id: u32) -> Option<ApiFileDetail> {
    let cache = detail_cache().lock().ok()?;
    let (fetched_at, detail) = cache.get(&id)?;
    (fetched_at.elapsed() < DETAIL_TTL).then(|| detail.clone())
}

/// Store a successful lookup, dropping expired entries on the way in so the map
/// stays bounded by "addons fetched in the last minute" rather than growing for
/// the life of the process as a user browses.
fn store_file_detail(id: u32, detail: &ApiFileDetail) {
    let Ok(mut cache) = detail_cache().lock() else {
        return;
    };
    cache.retain(|_, (fetched_at, _)| fetched_at.elapsed() < DETAIL_TTL);
    cache.insert(id, (Instant::now(), detail.clone()));
}

/// Fetch addon details from the ESOUI filedetails JSON API, reusing a recent
/// response when one is available. Only successes are cached — an error must
/// stay retryable immediately.
fn fetch_file_detail(client: &reqwest::blocking::Client, id: u32) -> Result<ApiFileDetail, String> {
    if let Some(hit) = cached_file_detail(id) {
        return Ok(hit);
    }
    let url = format!("https://api.mmoui.com/v4/game/ESO/filedetails/{id}.json");
    let response = fetch_with_retry(client, &url).map_err(|e| {
        if e.contains("HTTP 404") {
            "Addon not found on ESOUI. It may have been removed.".to_string()
        } else if e.contains("HTTP 429") {
            "Too many requests to ESOUI. Please wait a moment and try again.".to_string()
        } else {
            e
        }
    })?;

    let entries: Vec<ApiFileDetail> = response
        .json()
        .map_err(|e| format!("Failed to parse ESOUI API response: {e}"))?;

    let detail = entries.into_iter().next().ok_or_else(|| {
        format!("ESOUI API returned empty response for addon {id}. It may have been removed.")
    })?;
    store_file_detail(id, &detail);
    Ok(detail)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EsouiAddonInfo {
    pub id: u32,
    pub title: String,
    pub version: String,
    pub download_url: String,
    pub updated: String,
    /// Publication marker from the same filedetails response as the checksum
    /// and download URL. It lets update metadata distinguish a stale filelist
    /// entry from a genuinely newer artifact.
    #[serde(skip_serializing)]
    pub(crate) last_update: u64,
    /// MD5 the filedetails API reports for `download_url`. Pass it to
    /// [`download_addon`] as `expected_md5` so the existing verification runs —
    /// without it, a corrupt-but-structurally-valid ZIP installs silently.
    pub checksum: String,
}

pub fn parse_esoui_input(input: &str) -> Result<u32, String> {
    let input = input.trim();

    // Bare numeric ID
    if let Ok(id) = input.parse::<u32>() {
        return Ok(id);
    }

    // URL with info{id} pattern: /downloads/info123 or /downloads/info123-Name.html
    static RE_INFO: OnceLock<Regex> = OnceLock::new();
    let re_info = RE_INFO.get_or_init(|| Regex::new(r"info(\d+)").unwrap());
    if let Some(caps) = re_info.captures(input) {
        if let Ok(id) = caps[1].parse::<u32>() {
            return Ok(id);
        }
    }

    // URL with id= query parameter: fileinfo.php?id=123
    static RE_ID: OnceLock<Regex> = OnceLock::new();
    let re_id = RE_ID.get_or_init(|| Regex::new(r"[?&]id=(\d+)").unwrap());
    if let Some(caps) = re_id.captures(input) {
        if let Ok(id) = caps[1].parse::<u32>() {
            return Ok(id);
        }
    }

    Err(format!("Could not parse ESOUI addon ID from: {input}"))
}

fn http_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .user_agent(user_agent())
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("failed to build HTTP client")
    })
}

/// User-agent shared by every ESOUI client.
fn user_agent() -> String {
    format!(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Kalpa/{}",
        env!("CARGO_PKG_VERSION")
    )
}

/// Dedicated client for addon ZIP downloads.
///
/// [`http_client`]'s 30-second `timeout` is a TOTAL deadline that covers the
/// whole body read, so a large addon aborts mid-download at exactly ~30s no
/// matter how healthily it was progressing — a 100 MB map/voice pack needs a
/// sustained ~27 Mbit/s just to finish in time, making those addons
/// uninstallable on slow links. This client therefore sets no total deadline.
///
/// reqwest's blocking builder has no idle-read timeout, so a dropped connection
/// is caught by TCP keepalive probes instead: after 30s of silence, probes every
/// 10s, connection failed after 4 unanswered (~70s) rather than hanging forever.
///
/// Do NOT try to add `read_timeout` here. It exists on reqwest's ASYNC builder
/// and a `blocking::ClientBuilder` can be built `From` an async one, so wiring it
/// up compiles cleanly — and then panics at runtime on the first response body:
/// `ReadTimeoutBody::poll_frame` calls `tokio::time::sleep`, and the blocking
/// client polls bodies on the caller's thread, where there is no reactor
/// ("there is no reactor running, must be called from the context of a Tokio 1.x
/// runtime"). The omission from the blocking builder is a guard rail, not an
/// oversight. This was shipped once and broke every download; the tests missed it
/// because they drive `stream_download_body` with in-memory readers and never
/// make a real request.
fn download_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .user_agent(user_agent())
            .connect_timeout(Duration::from_secs(30))
            .tcp_keepalive(Duration::from_secs(30))
            .tcp_keepalive_interval(Duration::from_secs(10))
            .tcp_keepalive_retries(4u32)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("failed to build download HTTP client")
    })
}

fn fetch_page(
    client: &reqwest::blocking::Client,
    url: &str,
    query: Option<&[(&str, &str)]>,
) -> Result<String, String> {
    fetch_page_with_url(client, url, query).map(|(_, body)| body)
}

/// Like [`fetch_page`] but also returns the FINAL URL after any redirects.
/// ESOUI redirects a precise-name search straight to the addon detail page
/// (e.g. `search.php?search=LuiData` → `info4373-LuiData.html`), so the caller
/// needs the landing URL to recover the addon id from it.
fn fetch_page_with_url(
    client: &reqwest::blocking::Client,
    url: &str,
    query: Option<&[(&str, &str)]>,
) -> Result<(String, String), String> {
    let mut builder = client.get(url);
    if let Some(q) = query {
        builder = builder.query(q);
    }

    let response = builder.send().map_err(|e| {
        if e.is_connect() || e.is_timeout() {
            "Could not connect to ESOUI. Check your internet connection.".to_string()
        } else {
            format!("Network error: {e}")
        }
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            404 => "Addon not found on ESOUI. It may have been removed.".to_string(),
            429 => "Too many requests to ESOUI. Please wait a moment and try again.".to_string(),
            500..=599 => "ESOUI is currently unavailable. Try again later.".to_string(),
            _ => format!("ESOUI returned an error (HTTP {status})"),
        });
    }

    const MAX_PAGE_SIZE: u64 = 5 * 1024 * 1024; // 5 MB
    if let Some(len) = response.content_length() {
        if len > MAX_PAGE_SIZE {
            return Err("ESOUI response too large.".to_string());
        }
    }

    // Capture the final URL (after redirects) before the body consumes `response`.
    let final_url = response.url().to_string();
    let body = response
        .text()
        .map_err(|e| format!("Failed to read response: {e}"))?;
    Ok((final_url, body))
}

/// Fetch basic addon info (title, version, download URL) from ESOUI JSON API.
pub fn fetch_addon_info(id: u32) -> Result<EsouiAddonInfo, String> {
    let client = http_client();
    let detail = fetch_file_detail(client, id)?;

    Ok(EsouiAddonInfo {
        id: detail.id,
        title: detail.title,
        version: detail.version,
        download_url: detail.download_uri,
        updated: String::new(), // Not needed by callers — metadata uses last_update epoch
        last_update: detail.last_update,
        checksum: detail.checksum,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EsouiAddonDetail {
    pub id: u32,
    pub title: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub compatibility: String,
    pub md5: String,
    pub total_downloads: String,
    pub monthly_downloads: String,
    pub favorites: String,
    pub updated: String,
    pub created: String,
    pub screenshots: Vec<String>,
    pub download_url: String,
    /// Cleaned version history, or empty when the author published none.
    pub change_log: String,
    /// Upload dates for past releases, newest first, scraped from the same
    /// fileinfo page the compatibility/created fields come from — so this
    /// costs no extra request. Empty when the author archives nothing.
    pub archived_versions: Vec<ArchivedVersion>,
}

/// One row of the ESOUI "Archived Files" table: a past release and the date it
/// was uploaded. The current version is never in this table — its date is the
/// API's `lastUpdate`, already exposed as [`EsouiAddonDetail::updated`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedVersion {
    pub version: String,
    /// As ESOUI renders it, e.g. `04/23/26 01:16 PM`.
    pub date: String,
}

fn clean_description(s: &str) -> String {
    let decoded = decode_html_entities(s);

    // Replace [*] list bullets with newlines so items don't run together
    static RE_BULLET: OnceLock<Regex> = OnceLock::new();
    let re_bullet = RE_BULLET.get_or_init(|| Regex::new(r"\[\*\]").unwrap());
    let with_newlines = re_bullet.replace_all(&decoded, "\n• ");

    static RE_BBCODE: OnceLock<Regex> = OnceLock::new();
    let re_bb = RE_BBCODE.get_or_init(|| Regex::new(r"\[/?[A-Za-z*]+[^\]]*\]").unwrap());
    let no_bbcode = re_bb.replace_all(&with_newlines, "");

    static RE_HTML: OnceLock<Regex> = OnceLock::new();
    let re_html = RE_HTML.get_or_init(|| Regex::new(r"</?[A-Za-z][^>]*>").unwrap());
    re_html.replace_all(&no_bbcode, "").trim().to_string()
}

/// Normalise the API's `changeLog` field. ESOUI sends the literal string
/// "None" rather than an empty value when an author published no changelog, so
/// both collapse to `""` — the single "no changelog" signal the UI checks.
fn clean_change_log(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return String::new();
    }
    clean_description(trimmed)
}

fn decode_html_entities(s: &str) -> String {
    static RE_ENTITY: OnceLock<Regex> = OnceLock::new();
    let re = RE_ENTITY.get_or_init(|| Regex::new(r"&(#(\d+)|#[xX]([0-9a-fA-F]+)|(\w+));").unwrap());
    re.replace_all(s, |caps: &regex::Captures| {
        if let Some(decimal) = caps.get(2) {
            if let Some(ch) = decimal
                .as_str()
                .parse::<u32>()
                .ok()
                .and_then(char::from_u32)
            {
                return ch.to_string();
            }
        } else if let Some(hex) = caps.get(3) {
            if let Some(ch) = u32::from_str_radix(hex.as_str(), 16)
                .ok()
                .and_then(char::from_u32)
            {
                return ch.to_string();
            }
        } else if let Some(name) = caps.get(4) {
            return match name.as_str() {
                "amp" => "&",
                "lt" => "<",
                "gt" => ">",
                "quot" => "\"",
                "apos" => "'",
                "nbsp" => " ",
                _ => return caps[0].to_string(),
            }
            .to_string();
        }
        caps[0].to_string()
    })
    .into_owned()
}

/// Format a number with comma separators (e.g., 1234567 → "1,234,567").
fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    result
}

/// Format an epoch-millisecond timestamp as "MM/DD/YY HH:MM AM/PM".
fn format_epoch_millis(millis: u64) -> String {
    if millis == 0 {
        return String::new();
    }
    // Use the metadata module's timestamp formatter for date portion
    let secs = millis / 1000;
    // Simple date format matching ESOUI's display: "MM/DD/YY HH:MM AM/PM"
    // We'll use chrono-free approach: just format the epoch
    let days = secs / 86400;
    let day_secs = secs % 86400;
    let mut hours = (day_secs / 3600) as u32;
    let minutes = ((day_secs % 3600) / 60) as u32;
    let ampm = if hours >= 12 { "PM" } else { "AM" };
    if hours == 0 {
        hours = 12;
    } else if hours > 12 {
        hours -= 12;
    }

    // Convert days since epoch to date
    let mut y: u32 = 1970;
    let mut d = days;
    loop {
        let leap = y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
        let year_days: u64 = if leap { 366 } else { 365 };
        if d < year_days {
            break;
        }
        d -= year_days;
        y += 1;
        if y > 3000 {
            return String::new();
        }
    }
    let leap = y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m: u32 = 0;
    for &md in &month_days {
        if d < md {
            break;
        }
        d -= md;
        m += 1;
    }

    format!(
        "{:02}/{:02}/{:02} {:02}:{:02} {}",
        m + 1,
        d + 1,
        y % 100,
        hours,
        minutes,
        ampm
    )
}

/// Scrape compatibility, created and the archived-version dates from the ESOUI
/// fileinfo HTML page. Best-effort: returns empties on any failure, because a
/// detail view without these is still useful.
fn scrape_fileinfo_page(
    client: &reqwest::blocking::Client,
    id: u32,
) -> (String, String, Vec<ArchivedVersion>) {
    let url = format!("https://www.esoui.com/downloads/fileinfo.php?id={id}");
    let body = match fetch_page(client, &url, None) {
        Ok(b) => b,
        Err(_) => return (String::new(), String::new(), Vec::new()),
    };
    let document = Html::parse_document(&body);

    let td_sel = Selector::parse("td").unwrap();
    let div_sel = Selector::parse("div").unwrap();
    let cells: Vec<ElementRef> = document.select(&td_sel).collect();

    let mut compatibility = String::new();
    let mut created = String::new();

    let mut i = 0;
    while i < cells.len() {
        let label = cells[i].text().collect::<String>();
        let label = label.trim();

        if label == "Compatibility:" {
            if let Some(next) = cells.get(i + 1) {
                // Value lives inside a child <div>
                compatibility = next
                    .select(&div_sel)
                    .next()
                    .map(|d| d.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();
            }
            i += 2;
            continue;
        }

        if label == "Created:" {
            if let Some(next) = cells.get(i + 1) {
                created = next.text().collect::<String>().trim().to_string();
            }
            i += 2;
            continue;
        }

        i += 1;
    }

    (compatibility, created, scrape_archived_versions(&document))
}

/// Parse the "Archived Files" table into version/date pairs.
///
/// Anchored on the `#other_t` section that wraps the table, with a shape check
/// on every row (five cells, a date-looking last cell) so a markup change
/// degrades to "no dates" rather than to rows of nonsense.
fn scrape_archived_versions(document: &Html) -> Vec<ArchivedVersion> {
    let Ok(row_sel) = Selector::parse("div#other_t tr") else {
        return Vec::new();
    };
    let Ok(cell_sel) = Selector::parse("td") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for row in document.select(&row_sel) {
        let cells: Vec<String> = row
            .select(&cell_sel)
            .map(|c| c.text().collect::<String>().trim().to_string())
            .collect();

        // File Name | Version | Size | Uploader | Date
        if cells.len() < 5 {
            continue;
        }
        let version = cells[1].trim();
        let date = cells[4].trim();
        if version.is_empty() || !looks_like_esoui_date(date) {
            continue;
        }
        out.push(ArchivedVersion {
            version: version.to_string(),
            date: date.to_string(),
        });
    }
    out
}

/// ESOUI renders archive dates as `MM/DD/YY HH:MM AM`. Checking the shape keeps
/// the header row and any future column shuffle out of the results.
fn looks_like_esoui_date(s: &str) -> bool {
    let head = s.as_bytes();
    head.len() >= 8
        && head[0].is_ascii_digit()
        && head[1].is_ascii_digit()
        && head[2] == b'/'
        && head[3].is_ascii_digit()
        && head[4].is_ascii_digit()
        && head[5] == b'/'
        && head[6].is_ascii_digit()
        && head[7].is_ascii_digit()
}

/// Fetch full addon details from the ESOUI JSON API.
pub fn fetch_addon_detail(id: u32) -> Result<EsouiAddonDetail, String> {
    let client = http_client();
    let detail = fetch_file_detail(client, id)?;

    let description = clean_description(&detail.description);
    let screenshots: Vec<String> = detail.images.into_iter().map(|img| img.image_url).collect();
    let updated = format_epoch_millis(detail.last_update);
    let change_log = clean_change_log(&detail.change_log);
    let (compatibility, created, archived_versions) = scrape_fileinfo_page(client, detail.id);

    Ok(EsouiAddonDetail {
        id: detail.id,
        title: detail.title,
        version: detail.version,
        author: detail.author,
        description,
        compatibility,
        md5: detail.checksum,
        total_downloads: format_number(detail.downloads),
        monthly_downloads: format_number(detail.downloads_monthly),
        favorites: format_number(detail.favorites),
        updated,
        created,
        screenshots,
        download_url: detail.download_uri,
        change_log,
        archived_versions,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EsouiSearchResult {
    pub id: u32,
    pub title: String,
    pub author: String,
    pub category: String,
    pub downloads: String,
    pub updated: String,
}

/// Body length above which an ESOUI listing page is a real, rendered page rather
/// than an error stub — so parsing nothing out of it means the markup changed,
/// not that the listing is empty.
const MIN_SUBSTANTIAL_PAGE: usize = 2048;

/// The error a listing parser returns when it recognises markup drift. Silently
/// returning `Ok(vec![])` instead renders as "no results", so a scraper
/// regression is indistinguishable from an empty listing and ships unnoticed.
fn markup_drift_error(what: &str) -> String {
    format!(
        "ESOUI's page format changed and Kalpa could not read the {what}. \
         This needs a Kalpa update — please report it."
    )
}

/// Search ESOUI and return rich results with metadata.
pub fn search_esoui(query: &str) -> Result<Vec<EsouiSearchResult>, String> {
    let client = http_client();
    let (final_url, body) = fetch_page_with_url(
        client,
        "https://www.esoui.com/downloads/search.php",
        Some(&[("search", query), ("se_search", "files")]),
    )?;

    // ESOUI redirects a unique-enough query straight to the addon page, which
    // has no result rows — so the list scraping below would come back empty.
    // Recover the id from the landing URL and synthesize a single result from
    // the addon detail so an exact-name search isn't silently empty.
    if let Some(id) = id_from_info_url(&final_url, query) {
        // The landing page has no result rows — this addon is the only result.
        // Enrich via the lightweight JSON detail API (one request, no extra
        // HTML scrape). If enrichment is momentarily unavailable (e.g. a 429),
        // still return the addon with what we know rather than blanking the
        // search or turning it into an error toast; selecting it re-fetches the
        // full detail. Category is not in the JSON API, so it's left empty for
        // this synthesized result.
        let result = match fetch_file_detail(client, id) {
            Ok(d) => EsouiSearchResult {
                id: d.id,
                title: d.title,
                author: d.author,
                category: String::new(),
                downloads: format_number(d.downloads),
                updated: format_epoch_millis(d.last_update),
            },
            Err(_) => EsouiSearchResult {
                id,
                title: query.to_string(),
                author: String::new(),
                category: String::new(),
                downloads: String::new(),
                updated: String::new(),
            },
        };
        return Ok(vec![result]);
    }

    let (results, addon_link_rows) = parse_search_rows(&body);

    if results.is_empty() && addon_link_rows > 0 {
        return Err(markup_drift_error("search results"));
    }

    Ok(results)
}

/// Parse ESOUI's search-results table out of a page body.
///
/// Returns the parsed rows plus the number of rows that LOOKED like results
/// (multi-cell, carrying an addon link) whether or not the heuristics below
/// accepted them — the caller uses that count to tell a genuinely empty search
/// apart from markup drift.
///
/// Pure and body-only on purpose: the cell-count guard and the `title_idx + N`
/// field offsets are the fragile part of this scrape, and welded to a network
/// fetch they could only be regression-tested by hitting ESOUI.
fn parse_search_rows(body: &str) -> (Vec<EsouiSearchResult>, usize) {
    let document = Html::parse_document(body);

    static RE_SEARCH_ID: OnceLock<Regex> = OnceLock::new();
    let re_id = RE_SEARCH_ID.get_or_init(|| Regex::new(r"[?&]id=(\d+)").unwrap());
    let row_sel = Selector::parse("tr").unwrap();
    let td_sel = Selector::parse("td").unwrap();
    let a_sel = Selector::parse("a[href]").unwrap();

    let mut results: Vec<EsouiSearchResult> = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    let mut addon_link_rows = 0usize;

    for row in document.select(&row_sel) {
        let cells: Vec<_> = row.select(&td_sel).collect();
        if cells.len() >= 2
            && cells.iter().any(|cell| {
                cell.select(&a_sel).any(|a| {
                    a.value()
                        .attr("href")
                        .is_some_and(|h| h.contains("fileinfo.php") && re_id.is_match(h))
                })
            })
        {
            addon_link_rows += 1;
        }
        // Real result rows have 5-6 cells; skip the search summary header (~21 cells)
        if cells.len() < 5 || cells.len() > 10 {
            continue;
        }

        // Find which cell contains the fileinfo.php link (title cell)
        let mut title_idx = None;
        let mut title = String::new();
        let mut id: u32 = 0;

        for (i, cell) in cells.iter().enumerate() {
            if let Some(a) = cell.select(&a_sel).find(|a| {
                a.value()
                    .attr("href")
                    .is_some_and(|h| h.contains("fileinfo.php"))
            }) {
                let href = a.value().attr("href").unwrap_or("");
                if let Some(caps) = re_id.captures(href) {
                    if let Ok(parsed_id) = caps[1].parse::<u32>() {
                        title = a.text().collect::<String>().trim().to_string();
                        id = parsed_id;
                        title_idx = Some(i);
                        break;
                    }
                }
            }
        }

        let title_idx = match title_idx {
            Some(i) => i,
            None => continue,
        };

        let author = cells
            .get(title_idx + 1)
            .map(|c| c.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let category = cells
            .get(title_idx + 2)
            .map(|c| c.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let downloads = cells
            .get(title_idx + 3)
            .map(|c| c.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let updated = cells
            .get(title_idx + 4)
            .map(|c| c.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        if !seen_ids.insert(id) {
            continue;
        }

        results.push(EsouiSearchResult {
            id,
            title,
            author,
            category,
            downloads,
            updated,
        });
    }

    (results, addon_link_rows)
}

/// Minimal percent-decoding for URL slugs (e.g. `%20` → space). Any malformed
/// escape is left as-is.
fn percent_decode(s: &str) -> String {
    fn hex(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Lowercase, percent-decoded, alphanumeric-only key for comparing an ESOUI URL
/// slug against an addon name regardless of separators (`-`, `.`, space, `%20`).
fn slug_key(s: &str) -> String {
    percent_decode(s)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Recover an addon id from an ESOUI detail URL such as
/// `https://www.esoui.com/downloads/info4373-LuiData.html`. Only the final path
/// segment is considered (the query string is dropped) and the slug must
/// resemble `name`, so neither a `search.php?search=info1-x.html` query nor a
/// redirect to an unrelated page can resolve to the wrong addon.
fn id_from_info_url(url: &str, name: &str) -> Option<u32> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    static RE_INFO_SLUG: OnceLock<Regex> = OnceLock::new();
    let re = RE_INFO_SLUG.get_or_init(|| Regex::new(r"(?i)/info(\d+)-([^/]+)\.html$").unwrap());
    let caps = re.captures(path)?;
    let id = caps[1].parse::<u32>().ok()?;
    let slug = slug_key(&caps[2]);
    let name_key = slug_key(name);
    if slug.is_empty() || name_key.is_empty() {
        return None;
    }
    (slug == name_key || slug.contains(&name_key) || name_key.contains(&slug)).then_some(id)
}

/// Search ESOUI for an addon by name, return the best-matching ESOUI ID.
/// Matches results by title, or recovers the id directly when ESOUI redirects a
/// precise query straight to the addon page.
pub fn search_addon_by_name(name: &str) -> Result<Option<u32>, String> {
    let client = http_client();
    let (final_url, body) = fetch_page_with_url(
        client,
        "https://www.esoui.com/downloads/search.php",
        Some(&[("search", name), ("se_search", "files")]),
    )?;

    // ESOUI redirects a unique-enough search straight to the addon page
    // (e.g. "LuiData" → info4373-LuiData.html), which carries no result-list
    // `fileinfo.php?id=` links. Recover the id from the landing URL first;
    // otherwise fall back to scraping the multi-result list below.
    if let Some(id) = id_from_info_url(&final_url, name) {
        return Ok(Some(id));
    }

    let document = Html::parse_document(&body);

    // Search results have links like: <a href="fileinfo.php?s=...&id=7">LibAddonMenu-2.0</a>
    let a_sel = Selector::parse("a[href]").unwrap();
    static RE_NAME_ID: OnceLock<Regex> = OnceLock::new();
    let re_id = RE_NAME_ID.get_or_init(|| Regex::new(r"[?&]id=(\d+)").unwrap());

    let name_lower = name.to_lowercase();

    for element in document.select(&a_sel) {
        let href = match element.value().attr("href") {
            Some(h) if h.contains("fileinfo.php") => h,
            _ => continue,
        };

        let link_text = element.text().collect::<String>();
        let link_text_lower = link_text.trim().to_lowercase();

        // Exact match on the link text
        if link_text_lower == name_lower {
            if let Some(caps) = re_id.captures(href) {
                if let Ok(id) = caps[1].parse::<u32>() {
                    return Ok(Some(id));
                }
            }
        }
    }

    // No exact match found — try a looser match (link text contains the name)
    for element in document.select(&a_sel) {
        let href = match element.value().attr("href") {
            Some(h) if h.contains("fileinfo.php") => h,
            _ => continue,
        };

        let link_text = element.text().collect::<String>();
        let link_text_lower = link_text.trim().to_lowercase();

        if link_text_lower.contains(&name_lower) || name_lower.contains(&link_text_lower) {
            if let Some(caps) = re_id.captures(href) {
                if let Ok(id) = caps[1].parse::<u32>() {
                    return Ok(Some(id));
                }
            }
        }
    }

    Ok(None)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EsouiCategory {
    pub id: u32,
    pub name: String,
    pub depth: u32,
}

/// Fetch the full category list from ESOUI search page.
pub fn fetch_categories() -> Result<Vec<EsouiCategory>, String> {
    let client = http_client();
    let body = fetch_page(client, "https://www.esoui.com/downloads/search.php", None)?;
    let document = Html::parse_document(&body);

    let option_sel = Selector::parse("option[value]").unwrap();
    let mut categories: Vec<EsouiCategory> = Vec::new();

    for el in document.select(&option_sel) {
        let value = el.value().attr("value").unwrap_or("0");
        let id = match value.parse::<u32>() {
            Ok(id) if id > 0 => id,
            _ => continue,
        };
        let text = el.text().collect::<String>();
        let name = text.trim().to_string();
        if name.is_empty() {
            continue;
        }

        let depth = if name.starts_with("--") {
            2
        } else if name.starts_with('-') {
            1
        } else {
            0
        };
        let clean_name = name.trim_start_matches('-').trim().to_string();

        categories.push(EsouiCategory {
            id,
            name: clean_name,
            depth,
        });
    }

    // The search page always renders the category <select>; "no categories" is
    // not a state ESOUI can legitimately be in.
    if categories.is_empty() && body.len() >= MIN_SUBSTANTIAL_PAGE {
        return Err(markup_drift_error("category list"));
    }

    Ok(categories)
}

/// Browse addons in a specific ESOUI category.
pub fn browse_category(
    category_id: u32,
    page: u32,
    sort_by: &str,
) -> Result<Vec<EsouiSearchResult>, String> {
    let client = http_client();

    let sb = match sort_by {
        "downloads" => "dec_hits",
        "newest" => "dec_date",
        "name" => "dec_title",
        _ => "dec_hits",
    };

    // ESOUI uses 1-based `page=` for paginated category listings
    let esoui_page = page + 1;
    let url = format!(
        "https://www.esoui.com/downloads/index.php?cid={category_id}&sb={sb}&so=desc&pt=f&page={esoui_page}"
    );

    let body = fetch_page(client, &url, None)?;
    let document = Html::parse_document(&body);

    static RE_FILE_ID: OnceLock<Regex> = OnceLock::new();
    let re_id = RE_FILE_ID.get_or_init(|| Regex::new(r"file_(\d+)").unwrap());
    static RE_DL_COUNT: OnceLock<Regex> = OnceLock::new();
    let re_dl = RE_DL_COUNT.get_or_init(|| Regex::new(r"^[\d,]+").unwrap());
    let file_sel = Selector::parse("div.file").unwrap();
    let title_sel = Selector::parse("a[href*='fileinfo']").unwrap();
    let author_sel = Selector::parse("div.author").unwrap();
    let dl_sel = Selector::parse("div.downloads").unwrap();
    let updated_sel = Selector::parse("div.updated").unwrap();

    let mut results: Vec<EsouiSearchResult> = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    for file_el in document.select(&file_sel) {
        let file_id_attr = file_el.value().attr("id").unwrap_or("");
        let id = match re_id.captures(file_id_attr) {
            Some(caps) => match caps[1].parse::<u32>() {
                Ok(id) => id,
                Err(_) => continue,
            },
            None => continue,
        };

        if !seen_ids.insert(id) {
            continue;
        }

        let title = file_el
            .select(&title_sel)
            .next()
            .map(|a| a.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        if title.is_empty() {
            continue;
        }

        let author = file_el
            .select(&author_sel)
            .next()
            .map(|el| {
                el.text()
                    .collect::<String>()
                    .trim()
                    .trim_start_matches("By:")
                    .trim()
                    .to_string()
            })
            .unwrap_or_default();

        // "6,443,139 Downloads (71,750 Monthly)" → "6,443,139"
        let downloads = file_el
            .select(&dl_sel)
            .next()
            .and_then(|el| {
                let text = el.text().collect::<String>();
                re_dl.find(text.trim()).map(|m| m.as_str().to_string())
            })
            .unwrap_or_default();

        // "Updated 04/25/26 07:49 AM" → "04/25/26 07:49 AM"
        let updated = file_el
            .select(&updated_sel)
            .next()
            .map(|el| {
                el.text()
                    .collect::<String>()
                    .trim()
                    .trim_start_matches("Updated")
                    .trim()
                    .to_string()
            })
            .unwrap_or_default();

        results.push(EsouiSearchResult {
            id,
            title,
            author,
            category: String::new(),
            downloads,
            updated,
        });
    }

    // Only the FIRST page is guaranteed non-empty: later pages legitimately run
    // past the end of a category, and erroring there would break infinite scroll.
    if results.is_empty() && page == 0 && body.len() >= MIN_SUBSTANTIAL_PAGE {
        return Err(markup_drift_error("category listing"));
    }

    Ok(results)
}

/// Return type for `browse_popular`: results plus an explicit pagination signal.
///
/// `has_more` reflects whether the **upstream** page was full before library
/// filtering. This is important because post-fetch filtering reduces the result
/// count below `PAGE_SIZE` even when more pages exist, so callers must not
/// infer pagination from `results.len()` alone.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowsePopularPage {
    pub results: Vec<EsouiSearchResult>,
    /// True when the upstream page returned a full set of results, meaning
    /// additional pages are likely available regardless of how many entries
    /// survive the library filter.
    pub has_more: bool,
}

const POPULAR_PAGE_SIZE: usize = 25;

fn format_download_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Browse ESOUI's global listing sorted by popularity/newest.
///
/// Uses the ESOUI filelist JSON API for accurate sorting across all addons,
/// with in-memory pagination. Libraries are excluded from results.
pub fn browse_popular(page: u32, sort_by: &str) -> Result<BrowsePopularPage, String> {
    ensure_filelist_cache(false)?;

    let guard = filelist_cache().lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard
        .as_ref()
        .ok_or_else(|| "ESOUI addon list unavailable.".to_string())?;

    let mut entries: Vec<&ApiFileEntry> = cache.entries.iter().filter(|e| !e.library).collect();

    match sort_by {
        "newest" => entries.sort_by_key(|e| std::cmp::Reverse(e.last_update)),
        _ => entries.sort_by_key(|e| std::cmp::Reverse(e.downloads)),
    }

    let start = page as usize * POPULAR_PAGE_SIZE;
    let results: Vec<EsouiSearchResult> = entries
        .iter()
        .skip(start)
        .take(POPULAR_PAGE_SIZE)
        .map(|e| EsouiSearchResult {
            id: e.id,
            title: e.title.clone(),
            author: e.author.clone(),
            category: String::new(),
            downloads: format_download_count(e.downloads),
            updated: format_epoch_millis(e.last_update),
        })
        .collect();

    let has_more = start + POPULAR_PAGE_SIZE < entries.len();

    Ok(BrowsePopularPage { results, has_more })
}

/// Cancellation + progress hooks for [`download_addon_with`], mirroring
/// [`crate::installer::ExtractHooks`] on the extraction side. Both default to
/// `None` (see [`DownloadHooks::NONE`]) so the common callers stay trivial.
#[derive(Clone, Copy)]
pub struct DownloadHooks<'a> {
    /// Polled between body chunks; when it reads `true` the download aborts with
    /// [`crate::installer::CANCELLED`] — deliberately the same sentinel the
    /// extract loop returns, so `cancel_update` callers keep matching one string
    /// whichever phase the Stop lands in.
    pub cancel: Option<&'a AtomicBool>,
    /// Invoked as `(bytes_done, total)` while the body streams in, so the UI can
    /// render "Downloading 4.2 / 19.1 MB". `total` is the server's
    /// `Content-Length`; `None` on the rare response without one, which the UI
    /// renders as an indeterminate bar. Callers are expected to throttle their
    /// own emissions — this fires once per chunk.
    pub progress: Option<&'a dyn Fn(u64, Option<u64>)>,
}

impl DownloadHooks<'_> {
    /// No cancellation, no progress — the default for callers that need neither.
    pub const NONE: DownloadHooks<'static> = DownloadHooks {
        cancel: None,
        progress: None,
    };
}

/// Body chunk size for the streaming download. Large enough that a 19 MB
/// library costs a few hundred reads rather than thousands, small enough that a
/// Stop request is observed promptly (the cancel flag is polled per chunk).
const DOWNLOAD_CHUNK_BYTES: usize = 64 * 1024;

/// How often a download that is receiving no data re-checks the cancel flag.
const BODY_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Pumps a blocking body reader on a dedicated thread so the consuming side
/// can keep observing cancellation while the underlying `read()` is stalled.
///
/// The blocking client cannot take an idle-read timeout (see
/// [`download_client`] for why wiring one up panics), so a read into a
/// connection that stopped sending blocks until TCP keepalive gives up
/// (~70s) — and a Stop click used to go unobserved for that whole window.
/// Here the stalled read blocks the pump thread instead; `read` on this
/// adapter returns `WouldBlock` every [`BODY_POLL_INTERVAL`] with no data,
/// which [`stream_download_body`] treats as "poll cancel and retry".
///
/// After the consumer walks away (cancel, error), the pump thread lingers in
/// its blocked read until keepalive fails it (≤70s), notices the dropped
/// receiver, and exits — bounded, and it holds nothing but the response.
struct ThreadedBodyReader {
    rx: std::sync::mpsc::Receiver<io::Result<Vec<u8>>>,
    /// Chunk delivered by the pump but not yet fully consumed by `read`.
    pending: Vec<u8>,
    pending_pos: usize,
}

impl ThreadedBodyReader {
    fn spawn<R: io::Read + Send + 'static>(mut body: R) -> Self {
        // Bounded: a fast network with a slow disk must not buffer the whole
        // download in memory. 4 chunks = 256 KB in flight.
        let (tx, rx) = std::sync::mpsc::sync_channel::<io::Result<Vec<u8>>>(4);
        std::thread::spawn(move || {
            let mut buf = vec![0u8; DOWNLOAD_CHUNK_BYTES];
            loop {
                match body.read(&mut buf) {
                    // EOF: drop tx, the consumer reads it as end of body.
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(Ok(buf[..n].to_vec())).is_err() {
                            break; // consumer gone (cancelled or failed)
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        break;
                    }
                }
            }
        });
        Self {
            rx,
            pending: Vec::new(),
            pending_pos: 0,
        }
    }
}

impl io::Read for ThreadedBodyReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pending_pos >= self.pending.len() {
            match self.rx.recv_timeout(BODY_POLL_INTERVAL) {
                Ok(Ok(chunk)) => {
                    self.pending = chunk;
                    self.pending_pos = 0;
                }
                Ok(Err(e)) => return Err(e),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "no data within the poll interval",
                    ));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(0),
            }
        }
        let n = buf.len().min(self.pending.len() - self.pending_pos);
        buf[..n].copy_from_slice(&self.pending[self.pending_pos..self.pending_pos + n]);
        self.pending_pos += n;
        Ok(n)
    }
}

fn report_download_progress(hooks: &DownloadHooks, done: u64, total: Option<u64>) {
    if let Some(cb) = hooks.progress {
        cb(done, total);
    }
}

fn download_is_cancelled(hooks: &DownloadHooks) -> bool {
    hooks
        .cancel
        .map(|flag| flag.load(Ordering::Relaxed))
        .unwrap_or(false)
}

/// Map an I/O failure raised while READING the response body — the network side.
///
/// A timeout here is the network giving up mid-body, not a disk problem —
/// "Failed to write download to temp file" sent users looking at their drive
/// instead of their connection. Every other read failure is the network too: a
/// socket reset mid-body is the CDN dropping us, and naming the temp file
/// misdirects in exactly the way the timeout wording exists to prevent. The old
/// single `io::copy` could not tell the two sides apart, so every failure
/// inherited the disk wording; streaming the body ourselves is what makes the
/// split possible.
fn map_download_read_error(e: &io::Error) -> String {
    if matches!(
        e.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        "Download stalled — the connection stopped sending data. Check your internet connection and try again.".to_string()
    } else {
        format!(
            "Download failed while receiving data: {e}. Check your internet connection and try again."
        )
    }
}

/// Map an I/O failure raised while WRITING the body to the temp file — the disk
/// side, the one place the "check your drive" reading is actually correct (a
/// full disk, a denied temp directory).
fn map_download_write_error(e: &io::Error) -> String {
    format!("Failed to write download to temp file: {e}")
}

/// Stream `reader` into `writer` in one pass, folding the MD5 verification into
/// the same loop instead of re-reading the finished file from disk, and
/// reporting `(bytes_done, total)` as it goes.
///
/// Returns the number of bytes written. The checksum is compared here, inside
/// the single pass, so a corrupt body is rejected with the same message the
/// old seek-and-re-read block produced. Generic over `Read`/`Write` so the
/// progress accounting, cancellation and hashing are testable without a
/// network response.
fn stream_download_body<R: io::Read, W: io::Write>(
    reader: &mut R,
    writer: &mut W,
    total: Option<u64>,
    expected_md5: Option<&str>,
    hooks: DownloadHooks,
) -> Result<u64, String> {
    use md5::{Digest, Md5};

    // An empty checksum means ESOUI published none for this artifact; skip the
    // hashing work entirely rather than compare against "".
    let expected = expected_md5
        .map(|s| s.to_lowercase())
        .filter(|s| !s.is_empty());
    // md-5 0.11 (digest 0.11) dropped the `io::Write` impl on the hasher, so it
    // is fed in chunks via `update` rather than through `io::copy`.
    let mut hasher = expected.as_ref().map(|_| Md5::new());

    let mut buf = vec![0u8; DOWNLOAD_CHUNK_BYTES];
    let mut written: u64 = 0;
    // Report zero up front so the UI can swap to a determinate bar (and show the
    // total size) before the first chunk lands, rather than after it.
    report_download_progress(&hooks, 0, total);

    loop {
        if download_is_cancelled(&hooks) {
            return Err(crate::installer::CANCELLED.to_string());
        }
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            // A `ThreadedBodyReader` tick: no data within its poll window.
            // Loop back to the cancel check instead of failing — this is what
            // makes Stop responsive while a stalled read sits in TCP
            // keepalive's ~70s window on the pump thread.
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(map_download_read_error(&e)),
        };
        if n == 0 {
            break;
        }
        writer
            .write_all(&buf[..n])
            .map_err(|e| map_download_write_error(&e))?;
        if let Some(h) = hasher.as_mut() {
            h.update(&buf[..n]);
        }
        written += n as u64;
        report_download_progress(&hooks, written, total);
    }
    writer.flush().map_err(|e| map_download_write_error(&e))?;

    if let (Some(expected), Some(hasher)) = (expected, hasher) {
        // digest 0.11 output no longer implements `LowerHex`; hex-encode by hand.
        let digest = hasher.finalize();
        let mut actual = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write as _;
            let _ = write!(actual, "{byte:02x}");
        }
        if actual != expected {
            return Err(
                "Download checksum mismatch — the file may be corrupt. Try again.".to_string(),
            );
        }
    }

    Ok(written)
}

pub fn download_addon(url: &str, expected_md5: Option<&str>) -> Result<NamedTempFile, String> {
    download_addon_with(url, expected_md5, DownloadHooks::NONE)
}

/// Like [`download_addon`] but with cancellation/progress hooks.
pub fn download_addon_with(
    url: &str,
    expected_md5: Option<&str>,
    hooks: DownloadHooks,
) -> Result<NamedTempFile, String> {
    if !url.starts_with("https://cdn.esoui.com/") && !url.starts_with("https://www.esoui.com/") {
        return Err("Invalid download URL: only ESOUI download links are allowed.".to_string());
    }

    let client = download_client();

    // Retry loop for transient HTTP errors (429, 502, 503, 504)
    const MAX_RETRIES: u32 = 2;
    let mut last_err;
    let response = 'retry: {
        last_err = String::new();
        for attempt in 0..=MAX_RETRIES {
            let resp = client.get(url).send().map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    "Download failed. Check your internet connection.".to_string()
                } else {
                    format!("Download failed: {e}")
                }
            })?;

            let final_url = resp.url().as_str();
            if !final_url.starts_with("https://cdn.esoui.com/")
                && !final_url.starts_with("https://www.esoui.com/")
            {
                return Err("Download was redirected to an untrusted host.".to_string());
            }

            let status = resp.status();
            if status.is_success() {
                break 'retry resp;
            }

            if is_transient_status(status) && attempt < MAX_RETRIES {
                last_err = format!("HTTP {status}");
                let delay = Duration::from_millis(500 * (1 << attempt));
                std::thread::sleep(delay);
                continue;
            }

            return Err(format!(
                "Download failed (HTTP {status}). The file may have been removed from ESOUI.",
            ));
        }
        return Err(format!("Download failed after retries: {last_err}"));
    };

    let expected_size = response.content_length();

    let mut tmp = NamedTempFile::new().map_err(|e| format!("Failed to create temp file: {e}"))?;

    // One pass: body → temp file, MD5 folded in, progress reported per chunk.
    // The file is never re-read to hash it, which is what made a 19 MB /
    // 5,642-file library pay for its own bytes twice. The body is pumped on a
    // dedicated thread (see `ThreadedBodyReader`) so a Stop lands within
    // ~250ms even while a stalled connection blocks the actual read.
    let mut body = ThreadedBodyReader::spawn(response);
    let written = stream_download_body(&mut body, &mut tmp, expected_size, expected_md5, hooks)?;

    if let Some(expected) = expected_size {
        if written != expected {
            return Err(format!(
                "Download incomplete: received {written} bytes, expected {expected}. Try again."
            ));
        }
    }

    // Verify the file is a valid ZIP archive
    tmp.as_file()
        .seek(io::SeekFrom::Start(0))
        .map_err(|e| format!("Failed to seek: {e}"))?;
    zip::ZipArchive::new(tmp.as_file()).map_err(|_| {
        "Downloaded file is not a valid ZIP archive. It may be corrupt — try again.".to_string()
    })?;
    tmp.as_file()
        .seek(io::SeekFrom::Start(0))
        .map_err(|e| format!("Failed to seek: {e}"))?;

    Ok(tmp)
}

fn is_transient_error(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect() || err.is_request()
}

fn is_transient_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 502 | 503 | 504)
}

/// Fetch with up to 3 retries on transient failures (timeouts, 5xx, connection errors).
fn fetch_with_retry(
    client: &reqwest::blocking::Client,
    url: &str,
) -> Result<reqwest::blocking::Response, String> {
    let mut last_err = String::new();
    for attempt in 0u32..3 {
        if attempt > 0 {
            let delay = Duration::from_millis(500 * (1 << (attempt - 1)));
            std::thread::sleep(delay);
        }
        match client.get(url).send() {
            Ok(resp) if resp.status().is_success() => return Ok(resp),
            Ok(resp) if is_transient_status(resp.status()) => {
                last_err = format!("HTTP {}", resp.status());
                continue;
            }
            Ok(resp) => {
                return Err(format!("HTTP {}", resp.status()));
            }
            Err(e) if is_transient_error(&e) && attempt < 2 => {
                last_err = e.to_string();
                continue;
            }
            Err(e) => {
                return Err(if e.is_connect() || e.is_timeout() {
                    "Could not reach ESOUI. Check your internet connection.".to_string()
                } else {
                    format!("Request failed: {e}")
                });
            }
        }
    }
    Err(format!("Request failed after retries: {last_err}"))
}

// ── ESOUI REST API (api.mmoui.com) ──────────────────────────────────────────

/// A single addon entry from the ESOUI filelist API.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ApiFileEntry {
    pub id: u32,
    pub category_id: u32,
    pub version: String,
    pub last_update: u64, // epoch millis
    pub title: String,
    pub author: String,
    pub file_info_uri: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub downloads_monthly: u64,
    #[serde(default)]
    pub favorites: u64,
    #[serde(default)]
    pub library: bool,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub addons: Vec<ApiAddonPath>,
}

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::deserialize(deserializer)?.unwrap_or_default())
}

/// Sub-addon path entry within an ESOUI file listing.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiAddonPath {
    pub path: String,
}

/// Lookup entry for a resolved addon from the API.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiAddonLookup {
    pub esoui_id: u32,
    pub title: String,
    pub version: String,
    pub author: String,
    pub last_update: u64, // epoch millis
    pub file_info_uri: String,
}

struct FilelistCache {
    entries: Vec<ApiFileEntry>,
    lookup: Arc<HashMap<String, Arc<ApiAddonLookup>>>,
    fetched_at: Instant,
}

static FILELIST_CACHE: OnceLock<Mutex<Option<FilelistCache>>> = OnceLock::new();

fn filelist_cache() -> &'static Mutex<Option<FilelistCache>> {
    FILELIST_CACHE.get_or_init(|| Mutex::new(None))
}

/// Session-level TTL for the filelist cache. One fetch covers the whole
/// session for typical usage; the cache refreshes automatically after this.
const FILELIST_TTL: Duration = Duration::from_secs(900); // 15 minutes

/// How recent a copy a FORCED refresh will still accept. Only wide enough to
/// collapse a burst of Refresh clicks (or a Refresh landing on top of another
/// thread's just-finished fetch) into one download.
const FORCED_REFRESH_COALESCE: Duration = Duration::from_secs(5);

/// Held across the fetch so only one thread downloads the multi-MB filelist.
/// The TTL check releases the cache mutex before fetching, so without this two
/// callers that both observe a stale cache — the on-open update check plus the
/// Browse tab, a routine pairing at launch — each pull all ~4000 entries.
static FILELIST_REFRESH_LOCK: Mutex<()> = Mutex::new(());

fn cache_is_fresh(within: Duration) -> bool {
    filelist_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|cache| cache.fetched_at.elapsed() < within)
}

fn cache_exists() -> bool {
    filelist_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
}

/// Ensure the filelist cache holds usable data.
///
/// `force` bypasses [`FILELIST_TTL`] for the explicit Refresh action: without it
/// a user who reads "addon X updated" on ESOUI and clicks Refresh can be told
/// everything is current for up to 15 minutes with no recourse. The automatic
/// on-open path must keep passing `false` (no background spam).
fn ensure_filelist_cache(force: bool) -> Result<(), String> {
    let acceptable = if force {
        FORCED_REFRESH_COALESCE
    } else {
        FILELIST_TTL
    };

    if cache_is_fresh(acceptable) {
        return Ok(());
    }

    let _refresh_guard = FILELIST_REFRESH_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    // A concurrent refresh may have finished while we waited for the lock.
    if cache_is_fresh(acceptable) {
        return Ok(());
    }

    let entries = match fetch_filelist_entries() {
        Ok(entries) => entries,
        // A transient ESOUI blip must not take a usable cache away from the
        // automatic check — an error toast there is pure noise when the data to
        // answer with is already in memory. An explicit Refresh still fails
        // loudly: the user asked for fresh data and has to know it didn't arrive.
        Err(e) if !force && cache_exists() => {
            eprintln!("[esoui] filelist refresh failed, serving the cached copy: {e}");
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    let lookup = build_filelist_lookup(&entries);

    let mut guard = filelist_cache().lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(FilelistCache {
        entries,
        lookup,
        fetched_at: Instant::now(),
    });

    Ok(())
}

/// Re-fetch the ESOUI filelist now, ignoring the TTL.
///
/// Wire this to the explicit Refresh action ONLY; every automatic path must stay
/// TTL-cached. Returns the fetch error rather than silently serving stale data.
// Not yet called: the Refresh command lives in commands.rs and still goes
// through the TTL-cached path.
#[allow(dead_code)]
pub fn refresh_filelist_cache() -> Result<(), String> {
    ensure_filelist_cache(true)
}

/// Drop a filelist observation after an update successfully installs an
/// artifact described by a newer filedetails response. The next update check
/// must fetch again instead of comparing the installed artifact against the
/// stale pre-download filelist and offering a phantom update.
pub fn invalidate_filelist_cache() {
    // Serialize with the complete fetch-and-publish operation. Clearing only
    // the cache mutex allows a refresh that started before an install to
    // publish its stale, pre-install observation after this invalidation.
    let _refresh_guard = FILELIST_REFRESH_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut guard = filelist_cache().lock().unwrap_or_else(|e| e.into_inner());
    *guard = None;
}

/// Invalidate only when at least one update was actually applied. Batch paths
/// can finish with zero successes and must preserve a still-valid observation.
pub fn invalidate_filelist_cache_if_applied(applied: bool) {
    if applied {
        invalidate_filelist_cache();
    }
}

/// Fetch the full ESOUI filelist and build a lookup map keyed by addon folder path.
///
/// Single HTTP request returns ~4000 addons with all their folder paths,
/// versions, and last-updated timestamps. Result is cached in-memory for
/// `FILELIST_TTL` so repeated update checks within a session don't re-fetch.
pub fn fetch_filelist_lookup() -> Result<Arc<HashMap<String, Arc<ApiAddonLookup>>>, String> {
    ensure_filelist_cache(false)?;
    let guard = filelist_cache().lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard
        .as_ref()
        .ok_or_else(|| "ESOUI addon list unavailable.".to_string())?;
    Ok(Arc::clone(&cache.lookup))
}

fn fetch_filelist_entries() -> Result<Vec<ApiFileEntry>, String> {
    let client = http_client();
    let url = "https://api.mmoui.com/v4/game/ESO/filelist.json";
    let response = fetch_with_retry(client, url)?;

    const MAX_FILELIST_SIZE: u64 = 50 * 1024 * 1024; // 50 MB
    if let Some(len) = response.content_length() {
        if len > MAX_FILELIST_SIZE {
            return Err("ESOUI filelist response too large.".to_string());
        }
    }

    response
        .json()
        .map_err(|e| format!("Failed to parse ESOUI API response: {e}"))
}

fn build_filelist_lookup(entries: &[ApiFileEntry]) -> Arc<HashMap<String, Arc<ApiAddonLookup>>> {
    let mut map = HashMap::new();
    for entry in entries {
        // One shared allocation per file entry: an entry can map many folder
        // paths, and the cache retains every value for the full TTL — cloning
        // the Arc per folder avoids duplicating the four strings each time.
        let lookup = Arc::new(ApiAddonLookup {
            esoui_id: entry.id,
            title: entry.title.clone(),
            version: entry.version.clone(),
            author: entry.author.clone(),
            last_update: entry.last_update,
            file_info_uri: entry.file_info_uri.clone(),
        });
        // Map each addon folder path to its parent file entry
        for addon in &entry.addons {
            // Only use the top-level folder name (before any '/')
            let folder = addon.path.split('/').next().unwrap_or(&addon.path);
            // Don't overwrite if already mapped (first match wins — the primary entry)
            map.entry(folder.to_string())
                .or_insert_with(|| Arc::clone(&lookup));
        }
    }

    Arc::new(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_id_from_redirected_detail_url() {
        // ESOUI redirects a precise search straight to the addon page; the id
        // lives in that URL and must be recovered.
        assert_eq!(
            id_from_info_url(
                "https://www.esoui.com/downloads/info4373-LuiData.html",
                "LuiData"
            ),
            Some(4373)
        );
        assert_eq!(
            id_from_info_url(
                "https://www.esoui.com/downloads/info4374-LuiMedia.html",
                "LuiMedia"
            ),
            Some(4374)
        );
        // Versioned-name slug still resolves.
        assert_eq!(
            id_from_info_url(
                "https://www.esoui.com/downloads/info7-LibAddonMenu-2.0.html",
                "LibAddonMenu-2.0"
            ),
            Some(7)
        );
    }

    #[test]
    fn does_not_resolve_unrelated_or_nonredirected_urls() {
        // A redirect to an unrelated addon must not resolve (wrong-slug guard).
        assert_eq!(
            id_from_info_url(
                "https://www.esoui.com/downloads/info999-SomethingElse.html",
                "LuiData"
            ),
            None
        );
        // Still on the search page (multi-result, no redirect) → no id from URL.
        assert_eq!(
            id_from_info_url(
                "https://www.esoui.com/downloads/search.php?search=LuiData&se_search=files",
                "LuiData"
            ),
            None
        );
        // A query that itself contains an info-URL must NOT be treated as a
        // redirect (the info string is in the query, not the final path).
        assert_eq!(
            id_from_info_url(
                "https://www.esoui.com/downloads/search.php?search=info1-LuiData.html&se_search=files",
                "info1-LuiData.html"
            ),
            None
        );
    }

    #[test]
    fn resolves_id_robustly_across_url_quirks() {
        // Trailing query/fragment on the detail URL.
        assert_eq!(
            id_from_info_url(
                "https://www.esoui.com/downloads/info4373-LuiData.html?x=1#top",
                "LuiData"
            ),
            Some(4373)
        );
        // Case-insensitive scheme/host casing.
        assert_eq!(
            id_from_info_url(
                "https://www.esoui.com/downloads/INFO4373-LuiData.html",
                "LuiData"
            ),
            Some(4373)
        );
        // Percent-encoded space in the slug still matches a spaced name.
        assert_eq!(
            id_from_info_url(
                "https://www.esoui.com/downloads/info123-Foo%20Bar.html",
                "Foo Bar"
            ),
            Some(123)
        );
        // Hyphen-for-space slug matches a spaced name too.
        assert_eq!(
            id_from_info_url(
                "https://www.esoui.com/downloads/info123-Foo-Bar.html",
                "Foo Bar"
            ),
            Some(123)
        );
    }

    /// One ESOUI-shaped search row: the title cell carries the fileinfo link and
    /// the next four cells are author/category/downloads/updated in that order.
    fn search_page(rows: &str) -> String {
        format!("<html><body><table><tr><td colspan=\"6\">Search results</td></tr>{rows}</table></body></html>")
    }

    const REAL_ROW: &str = concat!(
        "<tr>",
        "<td><a href=\"/downloads/fileinfo.php?id=1360\">LibAddonMenu</a></td>",
        "<td>Seerah</td><td>Libraries</td><td>1,234,567</td><td>04/25/26 07:49 AM</td>",
        "<td>&nbsp;</td>",
        "</tr>"
    );

    #[test]
    fn parses_search_row_fields_in_order() {
        let (rows, candidates) = parse_search_rows(&search_page(REAL_ROW));

        assert_eq!(candidates, 1);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.id, 1360);
        assert_eq!(r.title, "LibAddonMenu");
        assert_eq!(r.author, "Seerah");
        assert_eq!(r.category, "Libraries");
        assert_eq!(r.downloads, "1,234,567");
        assert_eq!(r.updated, "04/25/26 07:49 AM");
    }

    #[test]
    fn search_rows_are_deduplicated_by_id() {
        let (rows, _) = parse_search_rows(&search_page(&format!("{REAL_ROW}{REAL_ROW}")));
        assert_eq!(rows.len(), 1);
    }

    /// A genuinely empty search must stay empty (not be reported as drift), while
    /// a result-shaped row the heuristics reject must be counted as a candidate so
    /// `search_esoui` can raise the markup-drift error instead of showing nothing.
    #[test]
    fn distinguishes_no_results_from_markup_drift() {
        let (rows, candidates) =
            parse_search_rows("<html><body><p>No results found.</p></body></html>");
        assert!(rows.is_empty());
        assert_eq!(candidates, 0);

        // Same row, but ESOUI dropped the trailing cells below the 5-cell floor.
        let narrowed = concat!(
            "<tr>",
            "<td><a href=\"/downloads/fileinfo.php?id=1360\">LibAddonMenu</a></td>",
            "<td>Seerah</td>",
            "</tr>"
        );
        let (rows, candidates) = parse_search_rows(&search_page(narrowed));
        assert!(rows.is_empty());
        assert_eq!(candidates, 1, "a rejected result row must still be counted");
    }

    #[test]
    fn decode_cyrillic_numeric_entities() {
        assert_eq!(clean_description("&#1042;&#1055;"), "ВП");
    }

    #[test]
    fn decode_hex_entities() {
        assert_eq!(clean_description("&#x412;"), "В");
    }

    #[test]
    fn decode_named_entities() {
        assert_eq!(clean_description("&amp;"), "&");
        assert_eq!(clean_description("&quot;"), "\"");
        assert_eq!(clean_description("&apos;"), "'");
        assert_eq!(clean_description("&nbsp;"), "");
    }

    #[test]
    fn strip_entity_encoded_bbcode() {
        assert_eq!(clean_description("&#91;b&#93;bold&#91;/b&#93;"), "bold");
    }

    #[test]
    fn strip_entity_encoded_html() {
        assert_eq!(
            clean_description("&lt;br&gt;line&lt;b&gt;bold&lt;/b&gt;"),
            "linebold"
        );
    }

    #[test]
    fn strip_literal_bbcode_with_entities() {
        assert_eq!(clean_description("[b]&#1042;[/b]"), "В");
    }

    #[test]
    fn strip_literal_html_tags() {
        assert_eq!(clean_description("hello<br>world<b>!</b>"), "helloworld!");
    }

    #[test]
    fn invalid_codepoint_passthrough() {
        assert_eq!(clean_description("&#99999999;"), "&#99999999;");
    }

    #[test]
    fn plain_text_passthrough() {
        assert_eq!(clean_description("hello world"), "hello world");
    }

    #[test]
    fn preserve_decoded_angle_brackets_in_text() {
        assert_eq!(clean_description("x &lt; 2 &gt; y"), "x < 2 > y");
        assert_eq!(clean_description("a &lt;= b"), "a <= b");
    }

    #[test]
    fn mixed_cyrillic_description() {
        let input = "If you want to help: PP at GitHub\n\n--RU--- &#1042; &#1087;&#1088;&#1086;&#1094;&#1077;&#1089;&#1089;&#1077; &#1088;&#1072;&#1079;&#1088;&#1072;&#1073;&#1086;&#1090;&#1082;&#1080;!";
        let result = clean_description(input);
        assert!(result.contains("В процессе разработки!"));
        assert!(result.contains("If you want to help"));
    }

    #[test]
    fn archived_versions_parsed_from_fileinfo_table() {
        // The shape ESOUI renders: File Name | Version | Size | Uploader | Date.
        let html = r##"<html><body><div id="other_t">
          <div class="title">Archived Files (2)</div>
          <table>
            <tr><td class="thead"><div>File Name</div></td><td class="thead"><div>Version</div></td>
                <td class="thead"><div>Size</div></td><td class="thead"><div>Uploader</div></td>
                <td class="thead"><div>Date</div></td></tr>
            <tr><td><div><a href="#">AwesomeGuildStore</a></div></td><td><div>1.7.7</div></td>
                <td><div>393kB</div></td><td><div>sirinsidiator</div></td>
                <td><div>04/23/26 01:16 PM</div></td></tr>
            <tr><td><div><a href="#">AwesomeGuildStore</a></div></td><td><div>1.7.6</div></td>
                <td><div>393kB</div></td><td><div>sirinsidiator</div></td>
                <td><div>09/06/25 11:16 AM</div></td></tr>
          </table></div></body></html>"##;
        let out = scrape_archived_versions(&Html::parse_document(html));
        assert_eq!(out.len(), 2, "header row must not become an entry");
        assert_eq!(out[0].version, "1.7.7");
        assert_eq!(out[0].date, "04/23/26 01:16 PM");
        assert_eq!(out[1].version, "1.7.6");
    }

    #[test]
    fn archived_versions_empty_when_section_missing() {
        // No #other_t: the addon archives nothing, or the markup drifted. Both
        // must degrade to "no dates" rather than to rows of nonsense.
        let html = "<html><body><table><tr><td>x</td></tr></table></body></html>";
        assert!(scrape_archived_versions(&Html::parse_document(html)).is_empty());
    }

    #[test]
    fn esoui_date_shape_is_checked() {
        assert!(looks_like_esoui_date("04/23/26 01:16 PM"));
        assert!(!looks_like_esoui_date("Date"));
        assert!(!looks_like_esoui_date("393kB"));
        assert!(!looks_like_esoui_date(""));
    }

    #[test]
    fn change_log_none_sentinel_becomes_empty() {
        // ESOUI sends the literal string "None" rather than an empty value when
        // an author published no changelog. Both must collapse to "", which is
        // the single signal the UI checks before rendering the section.
        assert_eq!(clean_change_log("None"), "");
        assert_eq!(clean_change_log("none"), "");
        assert_eq!(clean_change_log("  None  "), "");
        assert_eq!(clean_change_log(""), "");
        assert_eq!(clean_change_log("   "), "");
    }

    #[test]
    fn change_log_strips_bbcode_and_keeps_versions() {
        let input = "[B]Version 1.7.8[/B][LIST]\r\n[*] Fixed a crash[/LIST]";
        let result = clean_change_log(input);
        assert!(result.contains("Version 1.7.8"));
        assert!(result.contains("Fixed a crash"));
        // The [*] marker becomes a bullet so list items do not run together.
        assert!(result.contains('\u{2022}'));
        assert!(!result.contains("[B]"));
        assert!(!result.contains("[LIST]"));
    }

    #[test]
    fn change_log_keeps_a_real_entry_named_none_in_context() {
        // Only a bare "None" is the sentinel — a changelog that merely mentions
        // the word must survive.
        let result = clean_change_log("None of the old bugs remain");
        assert_eq!(result, "None of the old bugs remain");
    }

    #[test]
    fn download_addon_rejects_non_esoui_urls() {
        let result = download_addon("https://evil.com/malware.zip", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only ESOUI"));
    }

    #[test]
    fn download_addon_rejects_http_esoui() {
        let result = download_addon("http://cdn.esoui.com/addon.zip", None);
        assert!(result.is_err());
    }

    /// MD5 of `b"kalpa"`, computed outside this crate so the streaming-hash
    /// tests assert against an independent value rather than the code's own
    /// output.
    const KALPA_MD5: &str = "cb0fa609383235d6bdb40d38ae0a31c2";

    fn kalpa_body() -> Vec<u8> {
        b"kalpa".to_vec()
    }

    #[test]
    fn stream_download_body_hashes_in_the_same_pass() {
        let mut src = io::Cursor::new(kalpa_body());
        let mut out: Vec<u8> = Vec::new();
        let written = stream_download_body(
            &mut src,
            &mut out,
            Some(5),
            Some(KALPA_MD5),
            DownloadHooks::NONE,
        )
        .expect("matching checksum must verify");
        assert_eq!(written, 5);
        assert_eq!(out, b"kalpa");
    }

    #[test]
    fn stream_download_body_rejects_a_mismatched_checksum() {
        let mut src = io::Cursor::new(kalpa_body());
        let mut out: Vec<u8> = Vec::new();
        let err = stream_download_body(
            &mut src,
            &mut out,
            Some(5),
            Some("00000000000000000000000000000000"),
            DownloadHooks::NONE,
        )
        .expect_err("a wrong checksum must fail the download");
        assert!(err.contains("checksum mismatch"), "unexpected error: {err}");
    }

    #[test]
    fn stream_download_body_accepts_uppercase_and_skips_empty_checksums() {
        let mut src = io::Cursor::new(kalpa_body());
        let mut out: Vec<u8> = Vec::new();
        assert!(stream_download_body(
            &mut src,
            &mut out,
            None,
            Some(&KALPA_MD5.to_uppercase()),
            DownloadHooks::NONE,
        )
        .is_ok());

        // ESOUI publishes no checksum for some artifacts; an empty string must
        // skip verification rather than compare the digest against "".
        let mut src = io::Cursor::new(kalpa_body());
        let mut out: Vec<u8> = Vec::new();
        assert!(
            stream_download_body(&mut src, &mut out, None, Some(""), DownloadHooks::NONE).is_ok()
        );
    }

    #[test]
    fn stream_download_body_reports_monotonic_byte_progress() {
        // Two-and-a-bit chunks, so progress is reported more than once.
        let body = vec![7u8; DOWNLOAD_CHUNK_BYTES * 2 + 17];
        let total = body.len() as u64;
        let ticks = std::sync::Mutex::new(Vec::<(u64, Option<u64>)>::new());
        let record = |done: u64, all: Option<u64>| ticks.lock().unwrap().push((done, all));
        let mut src = io::Cursor::new(body);
        let mut out: Vec<u8> = Vec::new();
        let written = stream_download_body(
            &mut src,
            &mut out,
            Some(total),
            None,
            DownloadHooks {
                cancel: None,
                progress: Some(&record),
            },
        )
        .expect("stream must succeed");

        let ticks = ticks.into_inner().unwrap();
        assert_eq!(written, total);
        // Opens at zero so the UI can render a determinate bar immediately,
        // ends exactly at the total, and never goes backwards in between.
        assert_eq!(ticks.first().copied(), Some((0, Some(total))));
        assert_eq!(ticks.last().copied(), Some((total, Some(total))));
        assert!(ticks.windows(2).all(|w| w[0].0 <= w[1].0));
        assert!(ticks.iter().all(|(_, t)| *t == Some(total)));
        assert!(ticks.len() >= 4, "expected a tick per chunk: {ticks:?}");
    }

    #[test]
    fn stream_download_body_aborts_mid_body_when_cancelled() {
        // A reader that trips the cancel flag after handing over the first
        // chunk, so the abort happens mid-body rather than before the first
        // read — the case a user hits when they Stop a large download.
        struct CancelAfterFirstChunk<'a> {
            flag: &'a AtomicBool,
            reads: usize,
        }
        impl io::Read for CancelAfterFirstChunk<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.reads += 1;
                self.flag.store(true, Ordering::Relaxed);
                let n = buf.len().min(1024);
                buf[..n].fill(1);
                Ok(n)
            }
        }

        let flag = AtomicBool::new(false);
        let mut src = CancelAfterFirstChunk {
            flag: &flag,
            reads: 0,
        };
        let mut out: Vec<u8> = Vec::new();
        let err = stream_download_body(
            &mut src,
            &mut out,
            None,
            None,
            DownloadHooks {
                cancel: Some(&flag),
                progress: None,
            },
        )
        .expect_err("a cancelled download must not report success");
        // The same sentinel the extract loop returns, so the frontend's single
        // "Update cancelled" check covers both phases.
        assert_eq!(err, crate::installer::CANCELLED);
        assert_eq!(src.reads, 1, "must stop reading once the flag is set");
        assert_eq!(out.len(), 1024);
    }

    #[test]
    fn stream_download_body_maps_a_stalled_body_to_the_connection_message() {
        struct Stalled;
        impl io::Read for Stalled {
            fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::TimedOut, "read timed out"))
            }
        }

        let mut out: Vec<u8> = Vec::new();
        let err = stream_download_body(&mut Stalled, &mut out, None, None, DownloadHooks::NONE)
            .expect_err("a timed-out body must fail");
        // Deliberately not the "Failed to write download to temp file" wording:
        // a mid-body timeout is the network, not the user's drive.
        assert!(err.contains("Download stalled"), "unexpected error: {err}");
    }

    /// The Stop-during-stall path: the read yields no data (`WouldBlock`
    /// ticks from `ThreadedBodyReader`), and the loop must bounce back to the
    /// cancel check instead of failing or blocking — here the flag is set
    /// while the body is stalled, and the sentinel must come out.
    #[test]
    fn stream_download_body_observes_cancel_while_the_body_stalls() {
        struct StallAndSetCancel<'a> {
            flag: &'a AtomicBool,
        }
        impl io::Read for StallAndSetCancel<'_> {
            fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
                self.flag.store(true, Ordering::Relaxed);
                Err(io::Error::new(io::ErrorKind::WouldBlock, "stalled"))
            }
        }

        let flag = AtomicBool::new(false);
        let mut src = StallAndSetCancel { flag: &flag };
        let mut out: Vec<u8> = Vec::new();
        let err = stream_download_body(
            &mut src,
            &mut out,
            None,
            None,
            DownloadHooks {
                cancel: Some(&flag),
                progress: None,
            },
        )
        .expect_err("a cancelled stall must not report success");
        assert_eq!(err, crate::installer::CANCELLED);
    }

    #[test]
    fn threaded_body_reader_round_trips_the_body() {
        use std::io::Read as _;
        let payload: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
        let mut reader = ThreadedBodyReader::spawn(io::Cursor::new(payload.clone()));
        let mut out = Vec::new();
        // read_to_end retries on WouldBlock? No — it fails. Pull manually the
        // way stream_download_body does: WouldBlock means try again.
        let mut buf = vec![0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
        assert_eq!(out, payload);
    }

    #[test]
    fn threaded_body_reader_ticks_would_block_while_the_body_is_stalled() {
        use std::io::Read as _;
        // An underlying body blocked in a read (a stalled connection): the
        // adapter must return a WouldBlock tick within its poll interval
        // rather than block the caller.
        struct BlockedUntilDropped(std::sync::mpsc::Receiver<()>);
        impl io::Read for BlockedUntilDropped {
            fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
                let _ = self.0.recv(); // blocks until the sender is dropped
                Ok(0)
            }
        }

        let (release, blocked) = std::sync::mpsc::channel::<()>();
        let mut reader = ThreadedBodyReader::spawn(BlockedUntilDropped(blocked));
        let mut buf = [0u8; 16];
        let err = reader
            .read(&mut buf)
            .expect_err("stall must tick, not block");
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);

        // Unblock the pump: the body EOFs and the adapter reads as done.
        drop(release);
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => panic!("no data was ever sent"),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
    }

    #[test]
    fn threaded_body_reader_propagates_the_body_error_after_its_data() {
        struct DataThenError {
            sent: bool,
        }
        impl io::Read for DataThenError {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.sent {
                    Err(io::Error::new(io::ErrorKind::TimedOut, "keepalive gave up"))
                } else {
                    self.sent = true;
                    buf[..3].copy_from_slice(b"abc");
                    Ok(3)
                }
            }
        }

        use std::io::Read as _;
        let mut reader = ThreadedBodyReader::spawn(DataThenError { sent: false });
        let mut out = Vec::new();
        let mut buf = [0u8; 16];
        let err = loop {
            match reader.read(&mut buf) {
                Ok(0) => panic!("must surface the error, not EOF"),
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => break e,
            }
        };
        assert_eq!(out, b"abc");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn detail_cache_serves_a_fresh_entry_and_expires_a_stale_one() {
        let detail = ApiFileDetail {
            id: 4161,
            title: "LibCustomIcons".to_string(),
            version: "2026-08-31".to_string(),
            author: "m00nyONE".to_string(),
            description: String::new(),
            last_update: 0,
            checksum: "abc".to_string(),
            download_uri: "https://cdn.esoui.com/x.zip".to_string(),
            downloads: 0,
            downloads_monthly: 0,
            favorites: 0,
            change_log: String::new(),
            images: Vec::new(),
        };

        store_file_detail(4161, &detail);
        let hit = cached_file_detail(4161).expect("a just-stored entry must be a hit");
        assert_eq!(hit.checksum, "abc");

        // Backdate past the TTL: the entry must read as a miss, and the next
        // store must evict it rather than let the map grow.
        {
            let mut cache = detail_cache().lock().unwrap();
            let entry = cache.get_mut(&4161).unwrap();
            entry.0 = Instant::now() - DETAIL_TTL - Duration::from_secs(1);
        }
        assert!(
            cached_file_detail(4161).is_none(),
            "an entry past the TTL must not be served"
        );

        store_file_detail(999_999, &detail);
        assert!(
            !detail_cache().lock().unwrap().contains_key(&4161),
            "storing must evict expired entries"
        );
        detail_cache().lock().unwrap().clear();
    }

    #[test]
    fn applied_update_invalidates_stale_filelist_observation() {
        {
            let mut guard = filelist_cache().lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(FilelistCache {
                entries: Vec::new(),
                lookup: Arc::new(HashMap::new()),
                fetched_at: Instant::now(),
            });
        }

        assert!(cache_exists());
        invalidate_filelist_cache();
        assert!(!cache_exists());
    }

    #[test]
    fn invalidation_waits_for_an_in_flight_refresh_to_finish() {
        let refresh_guard = FILELIST_REFRESH_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();

        let invalidator = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            invalidate_filelist_cache();
            finished_tx.send(()).unwrap();
        });

        started_rx.recv().unwrap();
        assert!(finished_rx.recv_timeout(Duration::from_millis(50)).is_err());

        drop(refresh_guard);
        finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        invalidator.join().unwrap();
    }

    #[test]
    fn zero_applied_updates_preserve_filelist_observation() {
        {
            let mut guard = filelist_cache().lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(FilelistCache {
                entries: Vec::new(),
                lookup: Arc::new(HashMap::new()),
                fetched_at: Instant::now(),
            });
        }

        invalidate_filelist_cache_if_applied(false);
        assert!(cache_exists());
        invalidate_filelist_cache();
    }
}
