//! ESOTK report build-evidence sidecar publishing.
//!
//! The native uploader recovers small raw-log build facts that ESOTK can use,
//! but ESO Logs never stores them as report data. This module publishes that
//! compact sidecar to ESOTK's Worker after a successful native upload. Publishing
//! is best-effort and must never make an otherwise-successful ESO Logs upload fail.

use std::sync::OnceLock;

use reqwest::blocking::Client;

use super::types::{KalpaBuildEvidence, Visibility};

const DEFAULT_ESOTK_API_URL: &str = "https://roster-hub-api.eso-toolkit.workers.dev";

pub(crate) fn should_publish_build_evidence(visibility: Visibility) -> bool {
    matches!(visibility, Visibility::Public | Visibility::Unlisted)
}

pub(crate) fn publish_build_evidence(
    report_code: &str,
    evidence: &KalpaBuildEvidence,
    visibility: Visibility,
    access_token: &str,
) -> Result<(), String> {
    if !should_publish_build_evidence(visibility) {
        return Ok(());
    }
    if !valid_report_code(report_code) {
        return Err("Invalid report code for build-evidence sidecar.".into());
    }
    if evidence.players.is_empty() {
        return Ok(());
    }
    if evidence
        .report_code
        .as_deref()
        .is_some_and(|code| code != report_code)
    {
        return Err("Build-evidence report code does not match upload report.".into());
    }
    if access_token.trim().is_empty() {
        return Err("No ESO Logs OAuth token available for sidecar publish.".into());
    }

    let url = format!(
        "{}/reports/{}/build-evidence",
        esotk_api_url().trim_end_matches('/'),
        report_code
    );
    let response = http_client()
        .put(url)
        .bearer_auth(access_token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(evidence)
        .send()
        .map_err(|e| format!("Build-evidence sidecar publish failed: {e}"))?;

    if response.status().is_success() {
        return Ok(());
    }

    let status = response.status();
    let body = response.text().unwrap_or_default();
    Err(format!(
        "Build-evidence sidecar publish returned HTTP {status}: {body}"
    ))
}

fn valid_report_code(report_code: &str) -> bool {
    (8..=32).contains(&report_code.len())
        && report_code.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn esotk_api_url() -> &'static str {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| {
        std::env::var("ESOTK_API_URL").unwrap_or_else(|_| DEFAULT_ESOTK_API_URL.to_string())
    })
}

fn http_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(format!("Kalpa/{}", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to build ESOTK sidecar HTTP client")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishes_only_non_private_reports() {
        assert!(should_publish_build_evidence(Visibility::Public));
        assert!(should_publish_build_evidence(Visibility::Unlisted));
        assert!(!should_publish_build_evidence(Visibility::Private));
    }

    #[test]
    fn validates_plain_report_codes_only() {
        assert!(valid_report_code("NMPAb7mxa8WchCrG"));
        assert!(!valid_report_code("short"));
        assert!(!valid_report_code("NMPAb7mxa8WchCrG/evil"));
        assert!(!valid_report_code("NMPAb7mxa8WchCrG?x=1"));
    }
}
