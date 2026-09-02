//! Fetching upstream artifacts for the client-directory manager.
//!
//! Kalpa hosts nothing: ReShade comes from `reshade.me` and shader packages
//! come from their authors' GitHub repositories, fetched at install time. That
//! keeps users on current versions, keeps Kalpa out of the redistribution
//! question entirely, and means there is no Kalpa-owned mirror to become a
//! supply-chain target — which is precisely how DLSS Swapper's community DLL
//! manifest ended up serving malware in 2026.
//!
//! Nothing NVIDIA-authored is ever fetched here. DLSS and Neural Rendering
//! runtimes are supplied by the user from their own machine and verified by
//! Authenticode signer, because they are not licensed for redistribution.
//!
//! The host allowlist is re-checked *after* redirects, mirroring
//! `esoui::download_addon`: an allowed host that 302s to an arbitrary one must
//! not smuggle a payload past the check.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tempfile::NamedTempFile;

/// Hosts this module will fetch from. Checked before the request and again
/// against the final URL after redirects.
pub const ALLOWED_HOSTS: [&str; 5] = [
    "reshade.me",
    "github.com",
    "objects.githubusercontent.com",
    "raw.githubusercontent.com",
    "codeload.github.com",
];

/// Refuse anything larger than this. ReShade's installer and the shader repos
/// are all far smaller; a surprise multi-gigabyte body is a bug or an attack.
pub const MAX_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;

/// Byte-level progress, emitted to the frontend during a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DownloadProgress {
    pub downloaded: u64,
    /// `None` when the server sent no `Content-Length`.
    pub total: Option<u64>,
}

/// What to fetch and how to verify it.
#[derive(Debug, Clone)]
pub struct DownloadSpec {
    pub url: String,
    /// Lowercase hex SHA-256. When set, a mismatch fails the download.
    pub expected_sha256: Option<String>,
    /// Overrides [`MAX_DOWNLOAD_BYTES`] downward for a specific fetch.
    pub max_bytes: Option<u64>,
}

/// Streaming buffer size. Large enough that the syscall overhead is negligible
/// on a fast link, small enough that progress stays responsive on a slow one.
const CHUNK_BYTES: usize = 64 * 1024;

/// Progress throttle: emit at most this often…
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// …or whenever this many bytes have accumulated since the last emit,
/// whichever comes first. Without this a fast download calls back thousands of
/// times a second and the IPC bridge, not the network, becomes the bottleneck.
const PROGRESS_BYTES: u64 = 256 * 1024;

/// Transient statuses worth retrying, matching `esoui::is_transient_status`.
fn is_transient_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 502 | 503 | 504)
}

/// Dedicated download client.
///
/// Deliberately sets **no** total `timeout`: reqwest's blocking `timeout` is a
/// deadline covering the whole body read, so a large artifact on a slow link
/// aborts mid-download no matter how healthily it was progressing (the same
/// trap documented on `esoui::download_client`). A dropped connection is caught
/// by TCP keepalive probes instead — 30s of silence, then probes every 10s,
/// failed after 4 unanswered (~70s) rather than hanging forever.
fn download_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .user_agent(format!(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Kalpa/{}",
                env!("CARGO_PKG_VERSION")
            ))
            .connect_timeout(Duration::from_secs(30))
            .tcp_keepalive(Duration::from_secs(30))
            .tcp_keepalive_interval(Duration::from_secs(10))
            .tcp_keepalive_retries(4u32)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("failed to build client download HTTP client")
    })
}

/// True when `url` parses and its host is in [`ALLOWED_HOSTS`].
///
/// Matches the host exactly or as a subdomain, and requires HTTPS.
///
/// The subdomain match is a dot-anchored suffix, never a bare `ends_with`:
/// `evilgithub.com` shares the suffix `github.com` but is an entirely different
/// registration, and `github.com.evil.tld` is a *subdomain of the attacker*.
/// Both must be rejected. Because the host comes from a parsed URL rather than
/// from string slicing, userinfo tricks (`https://github.com@evil.tld/`) resolve
/// to the real host — `evil.tld` — and are rejected too.
pub fn host_allowed(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    // `Url` already lowercases the host; the trailing-dot form (`github.com.`)
    // is the same name in DNS, so normalize it before comparing.
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    ALLOWED_HOSTS
        .iter()
        .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
}

/// Download `spec` into a temporary file, reporting byte progress.
///
/// Streams through a read/write loop rather than `io::copy` so progress is
/// observable, enforces the size cap while streaming (not after), retries on
/// 429/502/503/504, and verifies length and hash before returning.
pub fn download_to_temp(
    spec: &DownloadSpec,
    on_progress: &dyn Fn(DownloadProgress),
) -> Result<NamedTempFile, String> {
    if !host_allowed(&spec.url) {
        return Err(format!(
            "Refusing to download from an untrusted URL: {}. Downloads are limited to HTTPS on {}.",
            spec.url,
            ALLOWED_HOSTS.join(", ")
        ));
    }

    let cap = spec.max_bytes.unwrap_or(MAX_DOWNLOAD_BYTES);
    let client = download_client();

    const MAX_RETRIES: u32 = 2;
    let mut last_err = String::new();
    let response = 'retry: {
        for attempt in 0..=MAX_RETRIES {
            let resp = client.get(&spec.url).send().map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    "Download failed. Check your internet connection.".to_string()
                } else {
                    format!("Download failed: {e}")
                }
            })?;

            // Re-check AFTER redirects: an allowed host that 302s elsewhere must
            // not smuggle a payload past the pre-flight check.
            if !host_allowed(resp.url().as_str()) {
                return Err(format!(
                    "Download was redirected to an untrusted host: {}.",
                    resp.url()
                ));
            }

            let status = resp.status();
            if status.is_success() {
                break 'retry resp;
            }

            if is_transient_status(status) && attempt < MAX_RETRIES {
                last_err = format!("HTTP {status}");
                std::thread::sleep(Duration::from_millis(500 * (1 << attempt)));
                continue;
            }

            return Err(format!(
                "Download failed (HTTP {status}) for {}. The file may have been moved or removed.",
                spec.url
            ));
        }
        return Err(format!(
            "Download failed after retries ({last_err}). Try again in a moment."
        ));
    };

    let expected_size = response.content_length();

    // Cheap pre-flight: a declared length over the cap saves streaming it.
    if let Some(total) = expected_size {
        if total > cap {
            return Err(format!(
                "Download rejected: {total} bytes exceeds the {cap}-byte limit."
            ));
        }
    }

    let mut response = response;
    stream_to_temp(
        &mut response,
        expected_size,
        cap,
        spec.expected_sha256.as_deref(),
        &spec.url,
        on_progress,
    )
}

/// Stream a body into a temp file: enforce the size cap as bytes arrive, verify
/// the declared length and the optional checksum, and emit throttled progress.
///
/// Split out of [`download_to_temp`] so the cap, length, checksum and progress
/// branches are reachable from tests driving an in-memory reader. Every piece of
/// network work — client construction, the allowlist checks, retries, status
/// handling — stays in the caller. `source_url` only names the artifact in the
/// checksum-mismatch message.
fn stream_to_temp(
    reader: &mut impl Read,
    declared_len: Option<u64>,
    max_bytes: u64,
    expected_sha256: Option<&str>,
    source_url: &str,
    on_progress: &dyn Fn(DownloadProgress),
) -> Result<NamedTempFile, String> {
    let mut tmp = NamedTempFile::new().map_err(|e| format!("Failed to create temp file: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK_BYTES];
    let mut downloaded: u64 = 0;
    let mut last_emit_at = Instant::now();
    let mut last_emit_bytes: u64 = 0;

    on_progress(DownloadProgress {
        downloaded: 0,
        total: declared_len,
    });

    loop {
        let n = reader.read(&mut buf).map_err(|e| {
            // A timeout here is the network giving up mid-body, not a disk
            // problem — pointing users at their drive sends them the wrong way.
            if matches!(
                e.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) {
                "Download stalled — the connection stopped sending data. Check your internet connection and try again.".to_string()
            } else {
                format!("Download failed while reading from the server: {e}")
            }
        })?;
        if n == 0 {
            break;
        }

        // Enforced here, as bytes arrive, so an unbounded body is aborted at the
        // limit rather than after it has already been buffered to disk.
        downloaded += n as u64;
        if downloaded > max_bytes {
            return Err(format!(
                "Download aborted: the file exceeds the {max_bytes}-byte limit."
            ));
        }

        tmp.write_all(&buf[..n])
            .map_err(|e| format!("Failed to write download to temp file: {e}"))?;
        hasher.update(&buf[..n]);

        if downloaded - last_emit_bytes >= PROGRESS_BYTES
            || last_emit_at.elapsed() >= PROGRESS_INTERVAL
        {
            last_emit_at = Instant::now();
            last_emit_bytes = downloaded;
            on_progress(DownloadProgress {
                downloaded,
                total: declared_len,
            });
        }
    }

    tmp.flush()
        .map_err(|e| format!("Failed to write download to temp file: {e}"))?;

    // Always finish on an exact figure, whatever the throttle last emitted.
    on_progress(DownloadProgress {
        downloaded,
        total: declared_len,
    });

    if let Some(expected) = declared_len {
        if downloaded != expected {
            return Err(format!(
                "Download incomplete: received {downloaded} bytes, expected {expected}. Try again."
            ));
        }
    }

    if let Some(expected) = expected_sha256 {
        if !expected.is_empty() {
            // Computed from the stream above, so the file is never re-read.
            let actual = to_hex(&hasher.finalize());
            if actual != expected.to_ascii_lowercase() {
                return Err(format!(
                    "Download checksum mismatch for {source_url}: expected {expected}, got {actual}. The file may be corrupt or tampered with — do not use it."
                ));
            }
        }
    }

    tmp.as_file()
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|e| format!("Failed to rewind temp file: {e}"))?;

    Ok(tmp)
}

/// Lowercase hex SHA-256 of a file's contents, read in chunks.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("Failed to open {} for hashing: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK_BYTES];
    loop {
        // digest 0.11 dropped the `io::Write` impl on hashers, so feed the
        // bytes in explicitly rather than reaching for `io::copy`.
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("Failed to read {} for hashing: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finalize()))
}

/// Lowercase hex, one allocation. digest 0.11's output no longer implements
/// `LowerHex`, so the encoding is done by hand.
fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- host_allowed: the allowlist ------------------------------------

    #[test]
    fn exact_allowed_hosts_pass() {
        for host in ALLOWED_HOSTS {
            assert!(
                host_allowed(&format!("https://{host}/some/path")),
                "{host} should be allowed"
            );
        }
    }

    #[test]
    fn legitimate_subdomains_pass() {
        assert!(host_allowed("https://foo.github.com/a"));
        assert!(host_allowed("https://a.b.c.github.com/"));
        assert!(host_allowed("https://downloads.reshade.me/ReShade.exe"));
    }

    #[test]
    fn host_match_is_case_insensitive() {
        assert!(host_allowed("https://GitHub.COM/owner/repo"));
        assert!(host_allowed("https://Raw.GithubUserContent.com/x"));
    }

    #[test]
    fn trailing_dot_host_is_normalized() {
        assert!(host_allowed("https://github.com./owner/repo"));
    }

    // --- host_allowed: the attacks --------------------------------------

    #[test]
    fn suffix_lookalike_is_rejected() {
        // A bare `ends_with("github.com")` would accept every one of these.
        assert!(!host_allowed("https://evilgithub.com/payload"));
        assert!(!host_allowed("https://notgithub.com/payload"));
        assert!(!host_allowed("https://xreshade.me/payload"));
        assert!(!host_allowed("https://my-github.com/payload"));
    }

    #[test]
    fn allowed_host_as_a_prefix_label_is_rejected() {
        assert!(!host_allowed("https://github.com.evil.tld/payload"));
        assert!(!host_allowed(
            "https://raw.githubusercontent.com.evil.tld/x"
        ));
        assert!(!host_allowed("https://reshade.me.attacker.example/x"));
    }

    #[test]
    fn non_tls_is_rejected() {
        assert!(!host_allowed("http://github.com/owner/repo"));
        assert!(!host_allowed("http://reshade.me/"));
        assert!(!host_allowed("ftp://github.com/x"));
        assert!(!host_allowed(
            "file:///C:/Windows/System32/drivers/etc/hosts"
        ));
    }

    #[test]
    fn userinfo_tricks_are_rejected() {
        assert!(!host_allowed("https://github.com@evil.tld/"));
        assert!(!host_allowed("https://github.com:token@evil.tld/payload"));
        assert!(!host_allowed("https://user@evilgithub.com/"));
    }

    #[test]
    fn garbage_urls_are_rejected() {
        assert!(!host_allowed(""));
        assert!(!host_allowed("   "));
        assert!(!host_allowed("not a url at all"));
        assert!(!host_allowed("github.com"));
        assert!(!host_allowed("https://"));
        assert!(!host_allowed("https:///path-with-no-host"));
        assert!(!host_allowed("javascript:alert(1)"));
        // A Windows path is not a URL. Built as a Rust literal on purpose:
        // backslashes do not survive a shell heredoc.
        assert!(!host_allowed("C:\\Users\\test\\ReShade.exe"));
        assert!(!host_allowed("\\\\server\\share\\ReShade.exe"));
    }

    #[test]
    fn unrelated_hosts_are_rejected() {
        assert!(!host_allowed("https://example.com/"));
        assert!(!host_allowed("https://gitlab.com/owner/repo"));
        // Not on the allowlist even though it is a real GitHub property.
        assert!(!host_allowed("https://api.github.io/x"));
    }

    // --- sha256_file ----------------------------------------------------

    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn temp_with(bytes: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().expect("temp file");
        f.write_all(bytes).expect("write");
        f.flush().expect("flush");
        f
    }

    #[test]
    fn sha256_of_empty_file_matches_the_known_vector() {
        let f = temp_with(b"");
        assert_eq!(sha256_file(f.path()).unwrap(), EMPTY_SHA256);
    }

    #[test]
    fn sha256_of_abc_matches_the_known_vector() {
        let f = temp_with(b"abc");
        assert_eq!(sha256_file(f.path()).unwrap(), ABC_SHA256);
    }

    #[test]
    fn sha256_spans_multiple_chunks() {
        // Larger than CHUNK_BYTES so the read loop runs more than once; the
        // digest must match a single-shot hash of the same bytes.
        let data: Vec<u8> = (0..(CHUNK_BYTES * 3 + 17))
            .map(|i| (i % 251) as u8)
            .collect();
        let f = temp_with(&data);
        let expected = to_hex(&Sha256::digest(&data));
        assert_eq!(sha256_file(f.path()).unwrap(), expected);
    }

    #[test]
    fn sha256_output_is_lowercase_hex() {
        let f = temp_with(b"Kalpa");
        let hex = sha256_file(f.path()).unwrap();
        assert_eq!(hex.len(), 64);
        assert!(hex
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()));
    }

    #[test]
    fn sha256_of_a_missing_file_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist.bin");
        let err = sha256_file(&missing).unwrap_err();
        assert!(err.contains("Failed to open"), "unexpected error: {err}");
    }

    // --- helpers --------------------------------------------------------

    #[test]
    fn to_hex_pads_single_digit_bytes() {
        assert_eq!(to_hex(&[0x00, 0x0f, 0xff, 0xa5]), "000fffa5");
        assert_eq!(to_hex(&[]), "");
    }

    #[test]
    fn transient_statuses_are_the_documented_set() {
        for code in [429u16, 502, 503, 504] {
            assert!(is_transient_status(
                reqwest::StatusCode::from_u16(code).unwrap()
            ));
        }
        for code in [200u16, 301, 400, 403, 404, 500, 501] {
            assert!(!is_transient_status(
                reqwest::StatusCode::from_u16(code).unwrap()
            ));
        }
    }

    #[test]
    fn download_rejects_an_untrusted_url_without_touching_the_network() {
        let spec = DownloadSpec {
            url: "https://evilgithub.com/payload.zip".to_string(),
            expected_sha256: None,
            max_bytes: None,
        };
        let err = download_to_temp(&spec, &|_| {}).unwrap_err();
        assert!(err.contains("untrusted"), "unexpected error: {err}");
    }

    #[test]
    fn download_rejects_a_non_tls_url_without_touching_the_network() {
        let spec = DownloadSpec {
            url: "http://github.com/owner/repo/archive/main.zip".to_string(),
            expected_sha256: None,
            max_bytes: None,
        };
        assert!(download_to_temp(&spec, &|_| {}).is_err());
    }

    // --- stream_to_temp: the body loop ----------------------------------
    //
    // These drive the extracted streaming helper with in-memory readers, which
    // is the only way to reach the mid-stream cap, the length check, the
    // checksum check and the progress throttle without a live server.

    use std::cell::{Cell, RefCell};
    use std::io::Cursor;

    /// Collects every `DownloadProgress` the helper emits.
    #[derive(Default)]
    struct Progress(RefCell<Vec<DownloadProgress>>);

    impl Progress {
        fn calls(&self) -> Vec<DownloadProgress> {
            self.0.borrow().clone()
        }
    }

    /// A body of `len` identical bytes that records how much was actually read,
    /// so "did it abort mid-stream?" is observable rather than assumed.
    struct CountingReader<'a> {
        remaining: u64,
        read_so_far: &'a Cell<u64>,
    }

    impl Read for CountingReader<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.remaining == 0 {
                return Ok(0);
            }
            let n = std::cmp::min(buf.len() as u64, self.remaining) as usize;
            buf[..n].fill(0x5a);
            self.remaining -= n as u64;
            self.read_so_far.set(self.read_so_far.get() + n as u64);
            Ok(n)
        }
    }

    /// Hands out `prefix` and then fails the way a dead connection does.
    struct StallingReader {
        prefix: Vec<u8>,
        kind: std::io::ErrorKind,
    }

    impl Read for StallingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.prefix.is_empty() {
                return Err(std::io::Error::new(self.kind, "boom"));
            }
            let n = std::cmp::min(buf.len(), self.prefix.len());
            buf[..n].copy_from_slice(&self.prefix[..n]);
            self.prefix.drain(..n);
            Ok(n)
        }
    }

    fn contents_of(f: &mut NamedTempFile) -> Vec<u8> {
        let mut out = Vec::new();
        f.as_file_mut().read_to_end(&mut out).expect("read back");
        out
    }

    #[test]
    fn stream_writes_every_byte_of_a_normal_body() {
        // Several chunks plus a partial one, so the loop runs more than once.
        let data: Vec<u8> = (0..(CHUNK_BYTES * 2 + 123))
            .map(|i| (i % 251) as u8)
            .collect();
        let mut reader = Cursor::new(data.clone());
        let mut tmp = stream_to_temp(
            &mut reader,
            Some(data.len() as u64),
            MAX_DOWNLOAD_BYTES,
            None,
            "https://github.com/x",
            &|_| {},
        )
        .expect("stream should succeed");
        assert_eq!(contents_of(&mut tmp), data);
    }

    #[test]
    fn stream_returns_a_temp_file_rewound_to_the_start() {
        let mut reader = Cursor::new(b"kalpa".to_vec());
        let tmp = stream_to_temp(
            &mut reader,
            None,
            MAX_DOWNLOAD_BYTES,
            None,
            "https://github.com/x",
            &|_| {},
        )
        .expect("stream should succeed");
        let pos = tmp
            .as_file()
            .try_clone()
            .expect("clone")
            .stream_position()
            .expect("position");
        assert_eq!(pos, 0, "temp file should be rewound for the caller");
    }

    #[test]
    fn stream_aborts_mid_body_once_the_cap_is_passed() {
        let cap = 128 * 1024u64;
        let body = 64 * 1024 * 1024u64; // far larger than the cap
        let read_so_far = Cell::new(0u64);
        let mut reader = CountingReader {
            remaining: body,
            read_so_far: &read_so_far,
        };

        let err = stream_to_temp(
            &mut reader,
            None,
            cap,
            None,
            "https://github.com/x",
            &|_| {},
        )
        .expect_err("cap should be enforced");
        assert!(
            err.contains("exceeds the 131072-byte limit"),
            "unexpected error: {err}"
        );

        // The point of enforcing on the running total: the rest of the body was
        // never pulled off the wire.
        let consumed = read_so_far.get();
        assert!(
            consumed <= cap + CHUNK_BYTES as u64,
            "read {consumed} bytes, expected to stop within one chunk of the {cap}-byte cap"
        );
        assert!(consumed < body, "the whole body was consumed anyway");
    }

    #[test]
    fn stream_rejects_a_body_shorter_than_the_declared_length() {
        let mut reader = Cursor::new(vec![1u8; 100]);
        let err = stream_to_temp(
            &mut reader,
            Some(500),
            MAX_DOWNLOAD_BYTES,
            None,
            "https://github.com/x",
            &|_| {},
        )
        .expect_err("short body should fail");
        assert_eq!(
            err,
            "Download incomplete: received 100 bytes, expected 500. Try again."
        );
    }

    #[test]
    fn stream_accepts_a_body_matching_the_declared_length() {
        let mut reader = Cursor::new(vec![1u8; 500]);
        let mut tmp = stream_to_temp(
            &mut reader,
            Some(500),
            MAX_DOWNLOAD_BYTES,
            None,
            "https://github.com/x",
            &|_| {},
        )
        .expect("exact length should succeed");
        assert_eq!(contents_of(&mut tmp).len(), 500);
    }

    #[test]
    fn stream_accepts_a_matching_checksum() {
        let mut reader = Cursor::new(b"abc".to_vec());
        let tmp = stream_to_temp(
            &mut reader,
            Some(3),
            MAX_DOWNLOAD_BYTES,
            Some(ABC_SHA256),
            "https://github.com/x",
            &|_| {},
        );
        assert!(tmp.is_ok(), "expected success, got {:?}", tmp.err());
    }

    #[test]
    fn stream_accepts_an_uppercase_checksum() {
        let mut reader = Cursor::new(b"abc".to_vec());
        let tmp = stream_to_temp(
            &mut reader,
            Some(3),
            MAX_DOWNLOAD_BYTES,
            Some(&ABC_SHA256.to_ascii_uppercase()),
            "https://github.com/x",
            &|_| {},
        );
        assert!(tmp.is_ok(), "expected success, got {:?}", tmp.err());
    }

    #[test]
    fn stream_rejects_a_mismatched_checksum() {
        let mut reader = Cursor::new(b"abc".to_vec());
        let err = stream_to_temp(
            &mut reader,
            Some(3),
            MAX_DOWNLOAD_BYTES,
            Some(EMPTY_SHA256),
            "https://github.com/owner/repo.zip",
            &|_| {},
        )
        .expect_err("wrong hash should fail");
        assert!(
            err.starts_with("Download checksum mismatch for https://github.com/owner/repo.zip:"),
            "unexpected error: {err}"
        );
        assert!(err.contains(ABC_SHA256), "should report the actual digest");
        assert!(err.contains("do not use it"), "should warn the user: {err}");
    }

    #[test]
    fn stream_skips_verification_for_an_empty_expected_checksum() {
        let mut reader = Cursor::new(b"abc".to_vec());
        let tmp = stream_to_temp(
            &mut reader,
            Some(3),
            MAX_DOWNLOAD_BYTES,
            Some(""),
            "https://github.com/x",
            &|_| {},
        );
        assert!(tmp.is_ok(), "expected success, got {:?}", tmp.err());
    }

    #[test]
    fn stream_opens_progress_at_zero_and_closes_on_the_true_total() {
        let data: Vec<u8> = (0..(CHUNK_BYTES * 4 + 9))
            .map(|i| (i % 253) as u8)
            .collect();
        let total = data.len() as u64;
        let mut reader = Cursor::new(data);
        let seen = Progress::default();

        stream_to_temp(
            &mut reader,
            Some(total),
            MAX_DOWNLOAD_BYTES,
            None,
            "https://github.com/x",
            &|p| seen.0.borrow_mut().push(p),
        )
        .expect("stream should succeed");

        let calls = seen.calls();
        assert!(calls.len() >= 2, "expected an opening and a closing call");
        assert_eq!(
            calls.first().unwrap(),
            &DownloadProgress {
                downloaded: 0,
                total: Some(total)
            },
            "the first call must report zero bytes"
        );
        assert_eq!(
            calls.last().unwrap(),
            &DownloadProgress {
                downloaded: total,
                total: Some(total)
            },
            "the last call must report the exact total"
        );
        assert!(
            calls.iter().all(|p| p.total == Some(total)),
            "every call carries the declared total"
        );
    }

    #[test]
    fn stream_throttles_progress_on_a_small_body() {
        // 260 KiB crosses PROGRESS_BYTES exactly once, so a throttled loop emits
        // far fewer calls than the five chunk reads it takes to drain.
        let data = vec![7u8; 260 * 1024];
        let seen = Progress::default();
        let mut reader = Cursor::new(data);

        stream_to_temp(
            &mut reader,
            None,
            MAX_DOWNLOAD_BYTES,
            None,
            "https://github.com/x",
            &|p| seen.0.borrow_mut().push(p),
        )
        .expect("stream should succeed");

        let calls = seen.calls();
        assert!(
            calls.len() <= 4,
            "throttle should keep a 260 KiB body to a handful of calls, got {}",
            calls.len()
        );
        assert_eq!(calls.first().unwrap().downloaded, 0);
        assert_eq!(calls.last().unwrap().downloaded, 260 * 1024);
        assert!(
            calls.windows(2).all(|w| w[0].downloaded <= w[1].downloaded),
            "progress must be monotonic"
        );
    }

    #[test]
    fn stream_reports_a_mid_body_timeout_as_a_stalled_connection() {
        for kind in [std::io::ErrorKind::TimedOut, std::io::ErrorKind::WouldBlock] {
            let mut reader = StallingReader {
                prefix: vec![3u8; 4096],
                kind,
            };
            let err = stream_to_temp(
                &mut reader,
                None,
                MAX_DOWNLOAD_BYTES,
                None,
                "https://github.com/x",
                &|_| {},
            )
            .expect_err("a stalled body should fail");
            assert!(
                err.contains("Download stalled"),
                "unexpected error for {kind:?}: {err}"
            );
            assert!(
                !err.contains("temp file") && !err.contains("disk"),
                "a network stall must not be reported as a disk problem: {err}"
            );
        }
    }

    #[test]
    fn stream_reports_other_read_errors_as_server_read_failures() {
        let mut reader = StallingReader {
            prefix: vec![3u8; 16],
            kind: std::io::ErrorKind::ConnectionReset,
        };
        let err = stream_to_temp(
            &mut reader,
            None,
            MAX_DOWNLOAD_BYTES,
            None,
            "https://github.com/x",
            &|_| {},
        )
        .expect_err("a reset body should fail");
        assert!(
            err.starts_with("Download failed while reading from the server:"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn stream_handles_an_empty_body() {
        let mut reader = Cursor::new(Vec::new());
        let seen = Progress::default();
        let mut tmp = stream_to_temp(
            &mut reader,
            Some(0),
            MAX_DOWNLOAD_BYTES,
            Some(EMPTY_SHA256),
            "https://github.com/x",
            &|p| seen.0.borrow_mut().push(p),
        )
        .expect("an empty body is a valid zero-length download");
        assert!(contents_of(&mut tmp).is_empty());
        let calls = seen.calls();
        assert_eq!(calls.first().unwrap().downloaded, 0);
        assert_eq!(calls.last().unwrap().downloaded, 0);
    }

    #[test]
    fn stream_accepts_a_body_exactly_at_the_cap() {
        // The cap rejects `> max_bytes`, never `== max_bytes`.
        let cap = (CHUNK_BYTES * 2) as u64;
        let mut reader = Cursor::new(vec![9u8; cap as usize]);
        let mut tmp = stream_to_temp(
            &mut reader,
            Some(cap),
            cap,
            None,
            "https://github.com/x",
            &|_| {},
        )
        .expect("a body exactly at the cap is allowed");
        assert_eq!(contents_of(&mut tmp).len() as u64, cap);
    }
}
