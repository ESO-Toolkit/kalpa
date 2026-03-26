use regex::Regex;
use scraper::{Html, Selector};
use serde::Serialize;
use std::io::Write;
use tempfile::NamedTempFile;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EsouiAddonInfo {
    pub id: u32,
    pub title: String,
    pub download_url: String,
}

pub fn parse_esoui_input(input: &str) -> Result<u32, String> {
    let input = input.trim();

    // Bare numeric ID
    if let Ok(id) = input.parse::<u32>() {
        return Ok(id);
    }

    // URL with info{id} pattern: /downloads/info123 or /downloads/info123-Name.html
    let re_info = Regex::new(r"info(\d+)").unwrap();
    if let Some(caps) = re_info.captures(input) {
        if let Ok(id) = caps[1].parse::<u32>() {
            return Ok(id);
        }
    }

    // URL with id= query parameter: fileinfo.php?id=123
    let re_id = Regex::new(r"[?&]id=(\d+)").unwrap();
    if let Some(caps) = re_id.captures(input) {
        if let Ok(id) = caps[1].parse::<u32>() {
            return Ok(id);
        }
    }

    Err(format!(
        "Could not parse ESOUI addon ID from: {}",
        input
    ))
}

fn http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent("ESOAddonManager/0.1.0")
        .build()
        .expect("failed to build HTTP client")
}

pub fn fetch_addon_info(id: u32) -> Result<EsouiAddonInfo, String> {
    let url = format!("https://www.esoui.com/downloads/info{}", id);
    let client = http_client();

    let response = client
        .get(&url)
        .send()
        .map_err(|e| format!("Failed to fetch ESOUI page: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "ESOUI returned status {} for addon {}",
            response.status(),
            id
        ));
    }

    let body = response
        .text()
        .map_err(|e| format!("Failed to read ESOUI response: {}", e))?;

    let document = Html::parse_document(&body);

    // Extract title from <meta property="og:title" content="...">
    let meta_sel = Selector::parse(r#"meta[property="og:title"]"#).unwrap();
    let title = document
        .select(&meta_sel)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("Addon #{}", id));

    // Extract download URL from link to cdn.esoui.com
    let a_sel = Selector::parse("a[href]").unwrap();
    let download_url = document
        .select(&a_sel)
        .filter_map(|el| el.value().attr("href"))
        .find(|href| href.contains("cdn.esoui.com"))
        .map(|s| s.to_string())
        .ok_or_else(|| {
            "Could not find download link on ESOUI page. The addon may have been removed.".to_string()
        })?;

    Ok(EsouiAddonInfo {
        id,
        title,
        download_url,
    })
}

pub fn download_addon(url: &str) -> Result<NamedTempFile, String> {
    let client = http_client();

    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("Failed to download addon: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status {}", response.status()));
    }

    let bytes = response
        .bytes()
        .map_err(|e| format!("Failed to read download: {}", e))?;

    let mut tmp = NamedTempFile::new().map_err(|e| format!("Failed to create temp file: {}", e))?;

    tmp.write_all(&bytes)
        .map_err(|e| format!("Failed to write temp file: {}", e))?;

    Ok(tmp)
}
