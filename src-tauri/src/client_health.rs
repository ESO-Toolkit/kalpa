//! Read-only diagnostics for an ESO **client install**.
//!
//! This module answers "what is loaded into the game client, and is any of it
//! stale or misconfigured?" It exists because that question is the first step
//! of nearly every crash triage, and because ESO's own bundled DLSS runtime
//! has not been updated since 2021.
//!
//! # Read-only by construction
//!
//! Every function here opens files for reading. Nothing writes, renames,
//! deletes, or downloads, and no path from this module is ever passed to a
//! write helper. That is deliberate and load-bearing: `commands.rs` enforces a
//! single write root via `require_allowed_path`, whose `validate_addons_path`
//! requires the leaf directory be literally named `AddOns`. A *write*-capable
//! client-directory feature would need a second, parallel allowed-path guard
//! (only `copy_addons_to_instance` has ever needed one). A read-only feature
//! needs none — reads outside the AddOns tree already have precedent in
//! `safe_migration::backup_minion_config`, which reads `~/.minion`.
//!
//! If this module ever grows a write path, that guard must be added first.

use crate::client_install::EsoClientLocation;
use serde::Serialize;
use std::path::Path;

/// A DLL the diagnostic looks for next to `eso64.exe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DllInfo {
    /// File name as found on disk, e.g. `nvngx_dlss.dll`.
    pub name: String,
    /// Four-part file version, e.g. `2.2.16.0`. `None` when the version
    /// resource is missing or unreadable — not an error, just unknown.
    pub version: Option<String>,
}

/// Severity of a finding. `Ok` is reported explicitly so the panel can show
/// green rows rather than only listing problems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthLevel {
    Ok,
    Info,
    Warning,
    /// No finding in this module emits `Danger` today — every problem it can
    /// detect is recoverable and non-destructive. The level exists so the
    /// frontend's severity ladder is complete and a future probe (a corrupt
    /// install, say) can use it without a breaking enum change.
    #[allow(dead_code)]
    Danger,
}

/// One diagnostic result. `id` is a stable slug so the frontend can key off it
/// without string-matching prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthFinding {
    pub id: String,
    pub level: HealthLevel,
    pub title: String,
    pub detail: String,
    /// Optional deep link to the relevant esotk.com documentation.
    pub guide_url: Option<String>,
}

/// A log line that matched a known failure signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogExcerpt {
    /// Source file name, e.g. `ReShade.log`.
    pub file: String,
    /// Slug of the rule that matched, shared with the corresponding
    /// [`HealthFinding::id`] where one is emitted.
    pub rule: String,
    /// The matching line, trimmed and truncated to a sane display length.
    pub line: String,
}

/// The full diagnostic report for one client install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientHealthReport {
    pub location: EsoClientLocation,
    /// The ReShade/injector proxy DLL, if one is present. ESO is Direct3D 11,
    /// so this is `d3d11.dll` for a stock ReShade install and `dxgi.dll` for
    /// setups that need DXGI-level add-on hooks. Both are reported; which one
    /// is correct is setup-dependent and deliberately not asserted here.
    pub injector: Option<DllInfo>,
    /// ESO's bundled DLSS super-resolution runtime. ESO has shipped DLSS and
    /// DLAA natively since 2021 (it was the first DLAA title); the bundled
    /// build is 2.2.16 and has not been updated since.
    pub dlss: Option<DllInfo>,
    /// The D3D shader compiler ESO ships. The bundled copy is a 2013 Windows
    /// 8.1 SDK build (6.3.9600) which wins DLL search order.
    pub d3dcompiler: Option<DllInfo>,
    /// `PresetPath` read out of `ReShade.ini`, when ReShade is installed.
    pub reshade_preset: Option<String>,
    pub findings: Vec<HealthFinding>,
    pub log_excerpts: Vec<LogExcerpt>,
}

// ── Probe constants ──────────────────────────────────────────────────────

/// Stock ReShade proxies as `d3d11.dll` on a Direct3D 11 title like ESO.
const INJECTOR_D3D11: &str = "d3d11.dll";
/// Setups that need DXGI-level add-on hooks proxy as `dxgi.dll` instead.
const INJECTOR_DXGI: &str = "dxgi.dll";
const DLSS_DLL: &str = "nvngx_dlss.dll";
const D3DCOMPILER_DLL: &str = "d3dcompiler_47.dll";

/// Major version at/above which the bundled DLSS runtime is considered current.
/// ESO ships 2.2.16; the modern transformer-model runtimes are 310+.
const DLSS_CURRENT_MAJOR: u32 = 310;

/// Version prefix of the 2013 Windows 8.1 SDK `d3dcompiler_47.dll` ESO ships.
const LEGACY_D3DCOMPILER_PREFIX: &str = "6.3.9600";

const GUIDE_URL: &str = "https://esotk.com/docs/dlss5-neural-rendering/";

/// Log files scanned for known failure signatures.
const LOG_FILES: [&str; 2] = ["ReShade.log", "dlss5-feed.log"];

/// `(rule_slug, needle)` — matched case-insensitively against each tailed line.
/// Deliberately a flat table so new signatures are a one-line addition.
const LOG_RULES: &[(&str, &str)] = &[
    ("dlss-error-0xbad00010", "0xBAD00010"),
    ("dlss-unsupported-parameter", "UnsupportedParameter"),
    ("shader-compile-failed", "failed to compile"),
    ("effect-compile-failed", "Effect compilation failed"),
];

/// Never read more than this much of a log file — these grow unbounded.
const MAX_LOG_TAIL_BYTES: u64 = 256 * 1024;
/// Of the tailed bytes, only the last this many lines are considered.
const MAX_LOG_TAIL_LINES: usize = 400;
/// Total excerpts across all log files, so one spammy log cannot flood the UI.
const MAX_LOG_EXCERPTS: usize = 20;
/// Per-excerpt display cap, in characters.
const MAX_EXCERPT_CHARS: usize = 300;

// ── File version ─────────────────────────────────────────────────────────

/// Read a Windows file-version resource as a dotted four-part string.
///
/// Returns `None` on non-Windows targets and whenever the resource is absent
/// or malformed.
#[cfg(target_os = "windows")]
pub fn file_version(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
    };

    /// `dwSignature` of a well-formed `VS_FIXEDFILEINFO`.
    const VS_FFI_SIGNATURE: u32 = 0xFEEF_04BD;

    // A path containing an interior NUL cannot name a real file; bail rather
    // than truncating it into some *other* file's version resource.
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return None;
    }
    wide.push(0);

    // The root sub-block `"\"` selects the fixed-file-info struct.
    let sub_block: [u16; 2] = ['\\' as u16, 0];

    unsafe {
        let name = PCWSTR(wide.as_ptr());
        let mut handle: u32 = 0;
        let size = GetFileVersionInfoSizeW(name, Some(&mut handle));
        if size == 0 {
            return None;
        }
        let mut buffer = vec![0u8; size as usize];
        GetFileVersionInfoW(name, None, size, buffer.as_mut_ptr().cast()).ok()?;

        let mut value: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut value_len: u32 = 0;
        let ok = VerQueryValueW(
            buffer.as_ptr().cast(),
            PCWSTR(sub_block.as_ptr()),
            &mut value,
            &mut value_len,
        );
        if !ok.as_bool()
            || value.is_null()
            || (value_len as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>()
        {
            return None;
        }

        // `value` points into `buffer`, which is still alive and correctly
        // aligned (Vec<u8> from the allocator; the struct is u32-aligned and
        // the version block is laid out DWORD-aligned by the OS). Copy it out
        // unaligned anyway so a malformed block cannot cause UB.
        let info: VS_FIXEDFILEINFO = std::ptr::read_unaligned(value.cast());
        if info.dwSignature != VS_FFI_SIGNATURE {
            return None;
        }
        Some(format_version(info.dwFileVersionMS, info.dwFileVersionLS))
    }
}

/// Non-Windows counterpart: there is no version resource to read.
#[cfg(not(target_os = "windows"))]
pub fn file_version(path: &Path) -> Option<String> {
    let _ = path;
    None
}

/// Format the two packed DWORDs of a `VS_FIXEDFILEINFO` as `a.b.c.d`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn format_version(ms: u32, ls: u32) -> String {
    format!(
        "{}.{}.{}.{}",
        (ms >> 16) & 0xFFFF,
        ms & 0xFFFF,
        (ls >> 16) & 0xFFFF,
        ls & 0xFFFF
    )
}

// ── Probing ──────────────────────────────────────────────────────────────

/// Look for `name` directly inside `dir`, resolving its version when present.
fn probe_dll(dir: &Path, name: &str) -> Option<DllInfo> {
    let path = dir.join(name);
    if !path.is_file() {
        return None;
    }
    Some(DllInfo {
        name: name.to_string(),
        version: file_version(&path),
    })
}

/// First dotted component of a version string, when it parses as a number.
fn major_version(version: &str) -> Option<u32> {
    version.split('.').next()?.trim().parse::<u32>().ok()
}

/// Human-readable version for prose, when the resource was unreadable.
fn version_label(info: &DllInfo) -> String {
    match &info.version {
        Some(v) => v.clone(),
        None => "unknown".to_string(),
    }
}

// ── ReShade.ini ──────────────────────────────────────────────────────────

/// Extract `PresetPath` from ReShade's plain Windows INI.
///
/// Tolerant by design: unknown sections, comments, blank lines, missing keys
/// and `key = value` spacing are all fine, and anything unparseable is simply
/// skipped rather than reported as an error.
fn parse_reshade_preset(contents: &str) -> Option<String> {
    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty()
            || line.starts_with(';')
            || line.starts_with('#')
            || line.starts_with('[')
        {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("PresetPath") {
            continue;
        }
        let value = value.trim().trim_matches('"').trim();
        if value.is_empty() {
            return None;
        }
        return Some(value.to_string());
    }
    None
}

// ── Log tailing ──────────────────────────────────────────────────────────

/// Read at most the last [`MAX_LOG_TAIL_BYTES`] of `path` and return at most
/// the last [`MAX_LOG_TAIL_LINES`] lines of it.
///
/// Returns an empty vec (never an error) for a missing or unreadable file.
fn read_log_tail(path: &Path) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};

    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let Ok(meta) = file.metadata() else {
        return Vec::new();
    };
    let len = meta.len();
    let offset = len.saturating_sub(MAX_LOG_TAIL_BYTES);
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }

    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.lines().collect();
    // Seeking mid-file almost certainly lands inside a line; drop that shard.
    if offset > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    let start = lines.len().saturating_sub(MAX_LOG_TAIL_LINES);
    lines[start..].iter().map(|l| l.to_string()).collect()
}

/// Truncate to [`MAX_EXCERPT_CHARS`] *characters* (not bytes, so multi-byte
/// log output is never split mid-codepoint).
fn truncate_line(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.chars().count() <= MAX_EXCERPT_CHARS {
        return trimmed.to_string();
    }
    trimmed.chars().take(MAX_EXCERPT_CHARS).collect()
}

/// Match `lines` against [`LOG_RULES`], appending to `out` until the global
/// excerpt cap is reached.
fn scan_lines(file: &str, lines: &[String], out: &mut Vec<LogExcerpt>) {
    for line in lines {
        if out.len() >= MAX_LOG_EXCERPTS {
            return;
        }
        let lowered = line.to_lowercase();
        for (rule, needle) in LOG_RULES {
            if out.len() >= MAX_LOG_EXCERPTS {
                return;
            }
            if lowered.contains(&needle.to_lowercase()) {
                out.push(LogExcerpt {
                    file: file.to_string(),
                    rule: (*rule).to_string(),
                    line: truncate_line(line),
                });
            }
        }
    }
}

/// Scan every known log file in `dir`.
fn scan_logs(dir: &Path) -> Vec<LogExcerpt> {
    let mut out = Vec::new();
    for name in LOG_FILES {
        if out.len() >= MAX_LOG_EXCERPTS {
            break;
        }
        let lines = read_log_tail(&dir.join(name));
        if lines.is_empty() {
            continue;
        }
        scan_lines(name, &lines, &mut out);
    }
    out
}

// ── Findings ─────────────────────────────────────────────────────────────

fn finding(
    id: &str,
    level: HealthLevel,
    title: &str,
    detail: String,
    guide: bool,
) -> HealthFinding {
    HealthFinding {
        id: id.to_string(),
        level,
        title: title.to_string(),
        detail,
        guide_url: if guide {
            Some(GUIDE_URL.to_string())
        } else {
            None
        },
    }
}

/// Build the finding list from already-resolved DLL info.
///
/// Split out from [`inspect_client`] so the version-dependent branches stay
/// testable on Linux and macOS, where [`file_version`] always returns `None`
/// and no real file could exercise them.
fn build_findings(
    injector: Option<&DllInfo>,
    both_injectors: bool,
    dlss: Option<&DllInfo>,
    d3dcompiler: Option<&DllInfo>,
) -> Vec<HealthFinding> {
    let mut findings = Vec::new();

    // — DLSS runtime —
    match dlss {
        Some(info) => {
            let major = info.version.as_deref().and_then(major_version);
            if major.is_some_and(|m| m >= DLSS_CURRENT_MAJOR) {
                findings.push(finding(
                    "dlss-current",
                    HealthLevel::Ok,
                    "DLSS runtime is current",
                    format!(
                        "{} reports version {}, which is a current DLSS runtime.",
                        info.name,
                        version_label(info)
                    ),
                    false,
                ));
            } else {
                findings.push(finding(
                    "dlss-stale",
                    HealthLevel::Warning,
                    "Bundled DLSS runtime is stale",
                    format!(
                        "{} in the client folder reports version {}. ESO has shipped DLSS and \
                         DLAA natively since 2021 — it was the first DLAA title — and the bundled \
                         runtime has not been refreshed since. You do not need to replace the \
                         file: open NVIDIA App → Graphics → DLSS Override and let the driver \
                         substitute a newer DLSS model, which updates the runtime without \
                         modifying any game file.",
                        info.name,
                        version_label(info)
                    ),
                    true,
                ));
            }
        }
        None => {
            findings.push(finding(
                "dlss-absent",
                HealthLevel::Info,
                "No DLSS runtime in the client folder",
                format!(
                    "{DLSS_DLL} was not found next to the game executable. ESO still exposes DLSS \
                     and DLAA in the in-game Anti-Aliasing dropdown on RTX GPUs, so this is only \
                     worth investigating if those options are missing there too."
                ),
                false,
            ));
        }
    }

    // — Injector / proxy DLL —
    match injector {
        Some(info) => {
            findings.push(finding(
                "injector-present",
                HealthLevel::Info,
                "Injector proxy DLL found",
                format!(
                    "{} is present in the client folder. This is expected if you installed \
                     ReShade — ESO is a Direct3D 11 title, so a stock ReShade install proxies as \
                     {INJECTOR_D3D11} while setups that need DXGI-level add-on hooks use \
                     {INJECTOR_DXGI}; which one is right depends on your setup. If the game \
                     crashes on launch, renaming or removing this file is the first thing to try.",
                    info.name
                ),
                false,
            ));
            if both_injectors {
                findings.push(finding(
                    "injector-both",
                    HealthLevel::Warning,
                    "Two injector proxy DLLs are installed",
                    format!(
                        "Both {INJECTOR_D3D11} and {INJECTOR_DXGI} are present in the client \
                         folder. Only one proxy should be installed at a time; having both is a \
                         known misconfiguration and a common cause of launch crashes and \
                         double-hooked overlays. Keep whichever one your setup requires and \
                         remove the other."
                    ),
                    false,
                ));
            }
        }
        None => {
            findings.push(finding(
                "injector-absent",
                HealthLevel::Ok,
                "No injector proxy DLL",
                format!(
                    "Neither {INJECTOR_D3D11} nor {INJECTOR_DXGI} is present in the client \
                     folder, so nothing is proxying the graphics API."
                ),
                false,
            ));
        }
    }

    // — Legacy shader compiler —
    //
    // Only meaningful when something is actually compiling shaders at runtime,
    // which for ESO means an injector is installed. Without one the stale
    // compiler is inert, so reporting it would be noise.
    if injector.is_some() {
        if let Some(info) = d3dcompiler {
            let legacy = info
                .version
                .as_deref()
                .is_some_and(|v| v.starts_with(LEGACY_D3DCOMPILER_PREFIX));
            if legacy {
                findings.push(finding(
                    "d3dcompiler-legacy",
                    HealthLevel::Warning,
                    "ESO ships a 2013 shader compiler",
                    format!(
                        "{} in the client folder reports version {} — the Windows 8.1 SDK build \
                         from 2013. Because it sits next to the executable it wins DLL search \
                         order over any newer copy on the system, so effects that compile at the \
                         cs_5_1 profile fail: that profile did not exist in 2013. If shader \
                         compilation is failing, this is almost always why.",
                        info.name,
                        version_label(info)
                    ),
                    true,
                ));
            }
        }
    }

    findings
}

/// Produce the full read-only diagnostic for a client install.
pub fn inspect_client(location: &EsoClientLocation) -> ClientHealthReport {
    let dir = location.client_dir.as_path();

    let d3d11 = probe_dll(dir, INJECTOR_D3D11);
    let dxgi = probe_dll(dir, INJECTOR_DXGI);
    let both_injectors = d3d11.is_some() && dxgi.is_some();
    // When both are installed, report the DXGI proxy: it hooks at the lower
    // level and is the one that actually loads. The `injector-both` finding
    // carries the fact that the other exists.
    let injector = dxgi.or(d3d11);

    let dlss = probe_dll(dir, DLSS_DLL);
    let d3dcompiler = probe_dll(dir, D3DCOMPILER_DLL);

    let reshade_preset = std::fs::read_to_string(dir.join("ReShade.ini"))
        .ok()
        .as_deref()
        .and_then(parse_reshade_preset);

    let findings = build_findings(
        injector.as_ref(),
        both_injectors,
        dlss.as_ref(),
        d3dcompiler.as_ref(),
    );
    let log_excerpts = scan_logs(dir);

    ClientHealthReport {
        location: location.clone(),
        injector,
        dlss,
        d3dcompiler,
        reshade_preset,
        findings,
        log_excerpts,
    }
}

// ── Tauri commands ───────────────────────────────────────────────────────
//
// These live here rather than in `commands.rs` for the same reason the
// uploader keeps its own `uploader/commands.rs`: this is a self-contained
// subsystem, and `commands.rs` is a heavily-contended file. Both are
// registered in the single `generate_handler!` list in `lib.rs`.

/// Enumerate ESO client installs found on this machine.
#[tauri::command]
pub fn detect_eso_clients() -> Vec<EsoClientLocation> {
    crate::client_install::detect_client_locations()
}

/// Validate a user-picked path as a client install, for the browse fallback.
#[tauri::command]
pub fn validate_eso_client(path: String) -> Result<EsoClientLocation, String> {
    crate::client_install::validate_client_dir(Path::new(&path))
}

/// Run the read-only diagnostic against a previously-resolved client dir.
///
/// The path is re-validated rather than trusted, so a stale or hand-edited
/// value from the frontend cannot aim the probe at an arbitrary directory.
#[tauri::command]
pub fn inspect_eso_client(client_dir: String) -> Result<ClientHealthReport, String> {
    let location = crate::client_install::validate_client_dir(Path::new(&client_dir))?;
    Ok(inspect_client(&location))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_install::ClientSource;
    use std::path::PathBuf;

    fn location(dir: &Path) -> EsoClientLocation {
        EsoClientLocation {
            client_dir: dir.to_path_buf(),
            exe_path: dir.join("eso64.exe"),
            source: ClientSource::Manual,
        }
    }

    /// Create a zero-length placeholder DLL. It has no version resource, so
    /// `file_version` returns `None` even on Windows — which is exactly why
    /// the version-dependent branches are tested through `build_findings`.
    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"").expect("write placeholder");
    }

    fn ids(findings: &[HealthFinding]) -> Vec<&str> {
        findings.iter().map(|f| f.id.as_str()).collect()
    }

    fn find<'a>(findings: &'a [HealthFinding], id: &str) -> &'a HealthFinding {
        findings
            .iter()
            .find(|f| f.id == id)
            .unwrap_or_else(|| panic!("expected finding {id}, got {:?}", ids(findings)))
    }

    fn dll(name: &str, version: Option<&str>) -> DllInfo {
        DllInfo {
            name: name.to_string(),
            version: version.map(|v| v.to_string()),
        }
    }

    #[test]
    fn format_version_splits_packed_dwords() {
        // 2.2.16.0 as ESO ships it.
        assert_eq!(format_version((2 << 16) | 2, 16 << 16), "2.2.16.0");
        assert_eq!(format_version(0, 0), "0.0.0.0");
    }

    #[test]
    fn file_version_of_a_non_dll_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("not-a-dll.dll");
        std::fs::write(&path, b"nope").unwrap();
        assert_eq!(file_version(&path), None);
        assert_eq!(file_version(&tmp.path().join("missing.dll")), None);
    }

    #[test]
    fn stale_dlss_version_yields_dlss_stale() {
        let findings = build_findings(None, false, Some(&dll(DLSS_DLL, Some("2.2.16.0"))), None);
        let f = find(&findings, "dlss-stale");
        assert_eq!(f.level, HealthLevel::Warning);
        assert!(f.detail.contains("2.2.16.0"), "detail names the version");
        assert!(f.detail.contains("2021"), "detail dates the runtime");
        assert!(
            f.detail.contains("DLSS Override"),
            "detail points at the file-free fix"
        );
        assert_eq!(f.guide_url.as_deref(), Some(GUIDE_URL));
        assert!(!ids(&findings).contains(&"dlss-current"));
    }

    #[test]
    fn unknown_dlss_version_is_treated_as_stale() {
        let findings = build_findings(None, false, Some(&dll(DLSS_DLL, None)), None);
        let f = find(&findings, "dlss-stale");
        assert!(f.detail.contains("unknown"));
    }

    #[test]
    fn modern_dlss_version_yields_dlss_current() {
        let findings = build_findings(None, false, Some(&dll(DLSS_DLL, Some("310.2.1.0"))), None);
        let f = find(&findings, "dlss-current");
        assert_eq!(f.level, HealthLevel::Ok);
        assert_eq!(f.guide_url, None);
    }

    #[test]
    fn stale_dlss_dll_on_disk_yields_dlss_stale() {
        let tmp = tempfile::tempdir().unwrap();
        touch(tmp.path(), DLSS_DLL);
        let report = inspect_client(&location(tmp.path()));
        assert_eq!(
            report.dlss.as_ref().map(|d| d.name.as_str()),
            Some(DLSS_DLL)
        );
        find(&report.findings, "dlss-stale");
    }

    #[test]
    fn empty_dir_yields_dlss_absent_and_injector_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let report = inspect_client(&location(tmp.path()));

        assert_eq!(report.injector, None);
        assert_eq!(report.dlss, None);
        assert_eq!(report.d3dcompiler, None);
        assert_eq!(report.reshade_preset, None);
        assert!(report.log_excerpts.is_empty());

        let absent = find(&report.findings, "dlss-absent");
        assert_eq!(absent.level, HealthLevel::Info);
        assert!(absent.detail.contains("Anti-Aliasing"));

        let none = find(&report.findings, "injector-absent");
        assert_eq!(none.level, HealthLevel::Ok);
        assert!(!ids(&report.findings).contains(&"injector-present"));
    }

    #[test]
    fn single_injector_is_reported_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        touch(tmp.path(), INJECTOR_D3D11);
        let report = inspect_client(&location(tmp.path()));

        assert_eq!(
            report.injector.as_ref().map(|d| d.name.as_str()),
            Some(INJECTOR_D3D11)
        );
        let f = find(&report.findings, "injector-present");
        assert_eq!(f.level, HealthLevel::Info);
        assert!(f.detail.contains(INJECTOR_D3D11));
        assert!(f.detail.contains("ReShade"));
        assert!(!ids(&report.findings).contains(&"injector-both"));
        assert!(!ids(&report.findings).contains(&"injector-absent"));
    }

    #[test]
    fn both_proxy_dlls_yield_injector_both() {
        let tmp = tempfile::tempdir().unwrap();
        touch(tmp.path(), INJECTOR_D3D11);
        touch(tmp.path(), INJECTOR_DXGI);
        let report = inspect_client(&location(tmp.path()));

        // The DXGI proxy is the one reported when both exist.
        assert_eq!(
            report.injector.as_ref().map(|d| d.name.as_str()),
            Some(INJECTOR_DXGI)
        );
        let both = find(&report.findings, "injector-both");
        assert_eq!(both.level, HealthLevel::Warning);
        assert!(both.detail.contains(INJECTOR_D3D11));
        assert!(both.detail.contains(INJECTOR_DXGI));
        // The plain "present" finding is still emitted alongside it.
        find(&report.findings, "injector-present");
    }

    #[test]
    fn d3dcompiler_legacy_requires_an_injector() {
        let legacy = dll(D3DCOMPILER_DLL, Some("6.3.9600.16384"));

        // No injector → the stale compiler is inert, so no finding.
        let without = build_findings(None, false, None, Some(&legacy));
        assert!(!ids(&without).contains(&"d3dcompiler-legacy"));

        // Injector present → the finding fires.
        let injector = dll(INJECTOR_D3D11, None);
        let with = build_findings(Some(&injector), false, None, Some(&legacy));
        let f = find(&with, "d3dcompiler-legacy");
        assert_eq!(f.level, HealthLevel::Warning);
        assert!(f.detail.contains("6.3.9600.16384"));
        assert!(f.detail.contains("cs_5_1"));
        assert!(f.detail.contains("search order"));
        assert_eq!(f.guide_url.as_deref(), Some(GUIDE_URL));
    }

    #[test]
    fn modern_d3dcompiler_is_not_flagged() {
        let injector = dll(INJECTOR_DXGI, None);
        let modern = dll(D3DCOMPILER_DLL, Some("10.0.19041.1"));
        let findings = build_findings(Some(&injector), false, None, Some(&modern));
        assert!(!ids(&findings).contains(&"d3dcompiler-legacy"));

        // An unreadable version is likewise not flagged — we only warn on a
        // positively-identified 2013 build.
        let unknown = dll(D3DCOMPILER_DLL, None);
        let findings = build_findings(Some(&injector), false, None, Some(&unknown));
        assert!(!ids(&findings).contains(&"d3dcompiler-legacy"));
    }

    #[test]
    fn preset_path_parses_out_of_reshade_ini() {
        // Windows paths must come from Rust string literals — a shell heredoc
        // would eat the backslashes.
        let ini = concat!(
            "; ReShade config\r\n",
            "\r\n",
            "[GENERAL]\r\n",
            "EffectSearchPaths=.\\reshade-shaders\\Shaders\r\n",
            "PresetPath = .\\ESO-Clarity.ini\r\n",
            "PerformanceMode=0\r\n",
            "[INPUT]\r\n",
            "KeyOverlay=36,0,0,0\r\n",
        );
        assert_eq!(
            parse_reshade_preset(ini),
            Some(".\\ESO-Clarity.ini".to_string())
        );
    }

    #[test]
    fn preset_path_parsing_is_tolerant() {
        // Quoted, absolute, odd casing.
        let quoted = concat!(
            "[GENERAL]\r\n",
            "presetpath=\"C:\\Games\\ESO\\reshade-presets\\Vivid.ini\"\r\n",
        );
        assert_eq!(
            parse_reshade_preset(quoted),
            Some("C:\\Games\\ESO\\reshade-presets\\Vivid.ini".to_string())
        );

        // Missing key, empty value, and junk lines all yield None.
        assert_eq!(parse_reshade_preset(""), None);
        assert_eq!(parse_reshade_preset("[GENERAL]\nPerformanceMode=1\n"), None);
        assert_eq!(parse_reshade_preset("PresetPath=\n"), None);
        assert_eq!(parse_reshade_preset("not an ini at all\n"), None);
        assert_eq!(parse_reshade_preset("; PresetPath=commented.ini\n"), None);
    }

    #[test]
    fn reshade_ini_is_read_from_the_client_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("ReShade.ini"),
            "[GENERAL]\r\nPresetPath=.\\my-preset.ini\r\n",
        )
        .unwrap();
        let report = inspect_client(&location(tmp.path()));
        assert_eq!(report.reshade_preset.as_deref(), Some(".\\my-preset.ini"));
    }

    #[test]
    fn log_tail_caps_lines_and_drops_the_partial_first_line() {
        let tmp = tempfile::tempdir().unwrap();
        let path: PathBuf = tmp.path().join("ReShade.log");

        // 1000 short lines: well under the byte cap, over the line cap.
        let body: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&path, &body).unwrap();
        let lines = read_log_tail(&path);
        assert_eq!(lines.len(), MAX_LOG_TAIL_LINES);
        assert_eq!(lines.last().unwrap(), "line 999");
        assert_eq!(lines.first().unwrap(), &format!("line {}", 1000 - 400));

        // Now exceed the byte cap: 4000 lines of ~120 bytes ≈ 480 KB.
        let padding = "x".repeat(100);
        let big: String = (0..4000)
            .map(|i| format!("line {i} {padding}\n"))
            .collect::<String>();
        assert!(big.len() as u64 > MAX_LOG_TAIL_BYTES);
        std::fs::write(&path, &big).unwrap();
        let lines = read_log_tail(&path);
        assert_eq!(lines.len(), MAX_LOG_TAIL_LINES);
        assert!(lines.last().unwrap().starts_with("line 3999 "));
        // Every retained line is whole — the mid-file shard was dropped.
        assert!(lines.iter().all(|l| l.starts_with("line ")));
    }

    #[test]
    fn log_tail_of_a_missing_file_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_log_tail(&tmp.path().join("nope.log")).is_empty());
    }

    #[test]
    fn log_rules_match_case_insensitively_and_truncate() {
        let tmp = tempfile::tempdir().unwrap();
        let long = "z".repeat(500);
        let body = format!(
            "info: all good\n   NGX error 0xbad00010 returned   \n\
             warn: unsupportedparameter for sharpness\n\
             error: failed to compile Clarity.fx {long}\n\
             Effect compilation FAILED for 3 effects\n"
        );
        std::fs::write(tmp.path().join("ReShade.log"), body).unwrap();

        let report = inspect_client(&location(tmp.path()));
        let rules: Vec<&str> = report
            .log_excerpts
            .iter()
            .map(|e| e.rule.as_str())
            .collect();
        assert!(rules.contains(&"dlss-error-0xbad00010"));
        assert!(rules.contains(&"dlss-unsupported-parameter"));
        assert!(rules.contains(&"shader-compile-failed"));
        assert!(rules.contains(&"effect-compile-failed"));
        assert!(report.log_excerpts.iter().all(|e| e.file == "ReShade.log"));

        let trimmed = report
            .log_excerpts
            .iter()
            .find(|e| e.rule == "dlss-error-0xbad00010")
            .unwrap();
        assert_eq!(trimmed.line, "NGX error 0xbad00010 returned");

        let truncated = report
            .log_excerpts
            .iter()
            .find(|e| e.rule == "shader-compile-failed")
            .unwrap();
        assert_eq!(truncated.line.chars().count(), MAX_EXCERPT_CHARS);
    }

    #[test]
    fn excerpts_are_capped_across_files() {
        let tmp = tempfile::tempdir().unwrap();
        let spam: String = (0..500)
            .map(|i| format!("error {i}: failed to compile something\n"))
            .collect();
        std::fs::write(tmp.path().join("ReShade.log"), &spam).unwrap();
        std::fs::write(tmp.path().join("dlss5-feed.log"), &spam).unwrap();

        let report = inspect_client(&location(tmp.path()));
        assert_eq!(report.log_excerpts.len(), MAX_LOG_EXCERPTS);
    }

    /// Positive coverage for the Win32 version-resource FFI. `kernel32.dll`
    /// always carries a version resource, so this is the one place the real
    /// `GetFileVersionInfoW`/`VerQueryValueW` path is exercised end to end.
    /// Read-only, like everything else in this module.
    #[test]
    #[cfg(target_os = "windows")]
    fn file_version_reads_a_real_system_dll() {
        let system_dll = Path::new(r"C:\Windows\System32\kernel32.dll");
        if !system_dll.is_file() {
            return; // non-standard Windows layout; nothing to assert against
        }
        let version = file_version(system_dll).expect("kernel32 has a version resource");
        let parts: Vec<&str> = version.split('.').collect();
        assert_eq!(parts.len(), 4, "four-part dotted version, got {version}");
        assert!(
            parts.iter().all(|p| p.parse::<u32>().is_ok()),
            "every part numeric, got {version}"
        );
        assert!(major_version(&version).is_some());
    }

    #[test]
    fn major_version_parsing() {
        assert_eq!(major_version("2.2.16.0"), Some(2));
        assert_eq!(major_version("310.2.1.0"), Some(310));
        assert_eq!(major_version(""), None);
        assert_eq!(major_version("v3.1"), None);
    }
}
