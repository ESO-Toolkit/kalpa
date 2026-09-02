//! ESO **client install** discovery.
//!
//! Kalpa historically only ever resolved the *Documents* side of an ESO
//! install (`.../Elder Scrolls Online/<live|liveeu|pts>/AddOns`). The client
//! install root — the directory holding `eso64.exe` — was never resolved:
//! `game_instances::is_steam_eso_installed` and `is_native_eso_installed` both
//! computed the path and then threw it away to return a `bool`.
//!
//! This module recovers those paths. It is **read-only**: nothing here writes
//! to, or hands out a write capability for, the client directory. The
//! `AllowedAddonsPath` invariant in `commands.rs` is untouched — see the module
//! docs on `client_health` for why a read-only diagnostic needs no second
//! write root.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How a client location was discovered. Surfaced to the UI so the user can
/// tell an auto-detected path from one they picked themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientSource {
    /// Found via a Steam library + `appmanifest_306130.acf`.
    Steam,
    /// Found via `HKLM\SOFTWARE\WOW6432Node\Zenimax_Online\Launcher\InstallPath`.
    ZosRegistry,
    /// Found under a Proton/Steam library on Linux.
    Proton,
    /// Supplied by the user through a file picker.
    Manual,
}

/// A resolved ESO client install.
///
/// `client_dir` is the directory that contains `eso64.exe` (conventionally
/// `.../The Elder Scrolls Online/game/client`). `exe_path` is that executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EsoClientLocation {
    pub client_dir: PathBuf,
    pub exe_path: PathBuf,
    pub source: ClientSource,
}

/// The Steam app id for The Elder Scrolls Online.
pub const ESO_STEAM_APP_ID: &str = "306130";

/// Executable names that identify a client directory. `eso.exe` is the legacy
/// 32-bit client; it is accepted so an old install still resolves.
pub const CLIENT_EXE_NAMES: [&str; 2] = ["eso64.exe", "eso.exe"];

// Several helpers below are only reachable from the Windows and Linux detection
// paths (and the tests). macOS compiles them too — CI builds all three targets —
// so they carry `allow(dead_code)` rather than a cfg thicket that would have to
// be kept in sync with every future platform arm.

/// The folder Steam uses for ESO when `installdir` cannot be read out of the
/// appmanifest. Steam's own value has been `Zenimax Online` for the life of the
/// app; the nested game folder is constant.
#[allow(dead_code)]
const STEAM_FALLBACK_INSTALLDIR: &str = "Zenimax Online";
/// Path from a Steam `common/<installdir>` root down to the client directory.
#[allow(dead_code)]
const GAME_CLIENT_RELATIVE: [&str; 2] = ["game", "client"];

// ── Shared helpers ───────────────────────────────────────────────────────────

/// Return the first `CLIENT_EXE_NAMES` entry that exists inside `dir`.
///
/// Ordering matters: `eso64.exe` wins over the legacy `eso.exe` when both are
/// present, which is the normal state of a modern install.
fn find_client_exe(dir: &Path) -> Option<PathBuf> {
    CLIENT_EXE_NAMES
        .iter()
        .map(|name| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Build an [`EsoClientLocation`] for `dir` if it actually holds a client
/// executable, otherwise `None`. Never errors: a directory that does not exist
/// is simply "not a client".
#[allow(dead_code)]
fn location_for_dir(dir: &Path, source: ClientSource) -> Option<EsoClientLocation> {
    let exe_path = find_client_exe(dir)?;
    Some(EsoClientLocation {
        client_dir: dir.to_path_buf(),
        exe_path,
        source,
    })
}

/// Canonical form used only as a dedupe key. Falls back to the path as-is when
/// canonicalization fails (a race, or a permission-denied parent) so a
/// still-valid location is never dropped.
#[allow(dead_code)]
fn dedupe_key(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Push `candidate` onto `out` unless an equivalent `client_dir` is already
/// present. First source wins, which is what makes the caller's probe order the
/// preference order.
#[allow(dead_code)]
fn push_unique(out: &mut Vec<EsoClientLocation>, candidate: EsoClientLocation) {
    let key = dedupe_key(&candidate.client_dir);
    if out
        .iter()
        .any(|existing| dedupe_key(&existing.client_dir) == key)
    {
        return;
    }
    out.push(candidate);
}

/// Extract `"installdir"  "<value>"` from an `appmanifest_*.acf`.
///
/// ACF is the same VDF key/value text format `platform::steam_library_paths`
/// parses, so this uses the same regex approach rather than a full parser. The
/// folder name is read rather than hardcoded because Steam is free to rename
/// it, and because a manually-moved install keeps its old name.
#[allow(dead_code)]
fn parse_acf_installdir(acf_path: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(acf_path).ok()?;
    let re = regex::Regex::new(r#""installdir"\s+"([^"]+)""#).expect("static regex");
    let raw = re.captures(&contents)?[1].replace("\\\\", "\\");
    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed)
}

/// Resolve the client directory for ESO inside one Steam library, if the
/// library actually has ESO installed.
///
/// Returns `None` when the appmanifest is absent — that is the "ESO is not in
/// this library" signal, not an error.
#[allow(dead_code)]
fn steam_client_dir_in_library(library: &Path) -> Option<PathBuf> {
    let steamapps = library.join("steamapps");
    let acf = steamapps.join(format!("appmanifest_{ESO_STEAM_APP_ID}.acf"));
    if !acf.is_file() {
        return None;
    }
    let installdir =
        parse_acf_installdir(&acf).unwrap_or_else(|| STEAM_FALLBACK_INSTALLDIR.to_string());

    let mut dir = steamapps.join("common").join(installdir);
    // Steam's `installdir` is the folder directly under `common/`; the client
    // lives two levels deeper. Some installs name that folder
    // "Zenimax Online" and nest "The Elder Scrolls Online" inside it, others
    // name it "The Elder Scrolls Online" directly — probe both shapes.
    let nested = dir.join("The Elder Scrolls Online");
    if nested.is_dir() {
        dir = nested;
    }
    for part in GAME_CLIENT_RELATIVE {
        dir = dir.join(part);
    }
    Some(dir)
}

// ── Windows detection ────────────────────────────────────────────────────────

/// Read the Steam install root from the registry, preferring the WOW6432 view
/// (Steam is a 32-bit process) and falling back to the native key.
#[cfg(target_os = "windows")]
fn steam_root_from_registry() -> Option<PathBuf> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm
        .open_subkey("SOFTWARE\\Wow6432Node\\Valve\\Steam")
        .or_else(|_| hklm.open_subkey("SOFTWARE\\Valve\\Steam"))
        .ok()?;
    let path: String = key.get_value("InstallPath").ok()?;
    if path.trim().is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

/// Read the ZOS/Bethesda launcher install root from the registry.
#[cfg(target_os = "windows")]
fn zos_launcher_root_from_registry() -> Option<PathBuf> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\WOW6432Node\\Zenimax_Online\\Launcher")
        .ok()?;
    let path: String = key.get_value("InstallPath").ok()?;
    if path.trim().is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

/// Client directories to probe for a ZOS launcher root.
///
/// `InstallPath` is the *launcher* directory (the one holding
/// `Bethesda.net_Launcher.exe` / `ZOS Launcher`), so the game sits one level
/// down under `The Elder Scrolls Online`. Installs where the value already
/// points at the game root are covered by the second candidate.
#[allow(dead_code)]
fn zos_client_candidates(launcher_root: &Path) -> Vec<PathBuf> {
    let mut game_root = launcher_root.join("The Elder Scrolls Online");
    let mut direct = launcher_root.to_path_buf();
    for part in GAME_CLIENT_RELATIVE {
        game_root = game_root.join(part);
        direct = direct.join(part);
    }
    vec![game_root, direct]
}

#[cfg(target_os = "windows")]
fn detect_platform_locations(out: &mut Vec<EsoClientLocation>) {
    if let Some(steam_root) = steam_root_from_registry() {
        for library in crate::platform::steam_library_paths(&steam_root) {
            if let Some(dir) = steam_client_dir_in_library(&library) {
                if let Some(found) = location_for_dir(&dir, ClientSource::Steam) {
                    push_unique(out, found);
                }
            }
        }
    }

    if let Some(launcher_root) = zos_launcher_root_from_registry() {
        for dir in zos_client_candidates(&launcher_root) {
            if let Some(found) = location_for_dir(&dir, ClientSource::ZosRegistry) {
                push_unique(out, found);
            }
        }
    }
}

// ── Linux / Proton detection ─────────────────────────────────────────────────

/// Linux best-effort: the Windows client installed under Proton lives in the
/// ordinary Steam library layout (`steamapps/common/<installdir>/game/client`),
/// a sibling of the `steamapps/compatdata/306130` prefix that
/// `platform::proton_documents_roots_from` walks. No registry is involved, so
/// the only discovery input is the known Steam roots.
#[cfg(target_os = "linux")]
fn detect_platform_locations(out: &mut Vec<EsoClientLocation>) {
    for root in crate::platform::steam_root_candidates() {
        for library in crate::platform::steam_library_paths(&root) {
            if let Some(dir) = steam_client_dir_in_library(&library) {
                if let Some(found) = location_for_dir(&dir, ClientSource::Proton) {
                    push_unique(out, found);
                }
            }
        }
    }
}

/// macOS (and any other target): nothing to detect. ESO's native Mac client is
/// out of scope for this diagnostic — it has no `eso64.exe` — and a CrossOver
/// bottle's client path is not discoverable without user input, which is what
/// [`validate_client_dir`] is for.
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn detect_platform_locations(out: &mut Vec<EsoClientLocation>) {
    let _ = out;
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Discover every ESO client install on this machine.
///
/// Returns an empty vec rather than erroring when nothing is found — "no
/// install detected" is a normal state the UI renders, not a failure.
/// Duplicates (the same `client_dir` found by two sources) are collapsed,
/// preferring the earlier-listed source.
pub fn detect_client_locations() -> Vec<EsoClientLocation> {
    let mut out = Vec::new();
    detect_platform_locations(&mut out);
    out
}

/// Validate a user-supplied directory (or executable path) as a client
/// install, for the "Browse for eso64.exe" fallback.
///
/// Accepts either the `client` directory itself or a path to the executable
/// inside it. Rejects UNC/device-namespace paths via
/// [`crate::platform::has_unc_or_verbatim_prefix`] before touching the
/// filesystem — the same SMB-resolution guard the rest of the codebase uses.
pub fn validate_client_dir(candidate: &Path) -> Result<EsoClientLocation, String> {
    // MUST come first: `is_dir`/`canonicalize` themselves perform SMB name
    // resolution, so the prefix check cannot move below them.
    if crate::platform::has_unc_or_verbatim_prefix(candidate) {
        return Err(
            "Network (UNC) and device paths are not supported. Pick a local drive path instead."
                .to_string(),
        );
    }

    // A path to the executable resolves to its parent directory. Checked by
    // file name rather than by `is_file` so a mistyped exe name still produces
    // the specific "no ESO executable" message below.
    let named_exe = candidate
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            CLIENT_EXE_NAMES
                .iter()
                .any(|exe| name.eq_ignore_ascii_case(exe))
        })
        .unwrap_or(false);

    let dir: PathBuf = if named_exe {
        match candidate.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            // A bare "eso64.exe" with no directory part.
            _ => PathBuf::from("."),
        }
    } else {
        candidate.to_path_buf()
    };

    if !dir.exists() {
        return Err(format!("Path not found: {}", dir.display()));
    }
    if !dir.is_dir() {
        return Err(format!("Not a directory: {}", dir.display()));
    }

    let Some(exe_path) = find_client_exe(&dir) else {
        return Err(format!(
            "No ESO executable ({}) found in {}. Pick the game's \"client\" folder.",
            CLIENT_EXE_NAMES.join(" or "),
            dir.display()
        ));
    };

    let client_dir = dunce::canonicalize(&dir).unwrap_or(dir);
    let exe_path = dunce::canonicalize(&exe_path).unwrap_or(exe_path);

    Ok(EsoClientLocation {
        client_dir,
        exe_path,
        source: ClientSource::Manual,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create `dir` plus a zero-byte `eso64.exe` inside it.
    fn make_client_dir(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).expect("create client dir");
        let exe = dir.join("eso64.exe");
        std::fs::write(&exe, b"").expect("write fake exe");
        exe
    }

    #[test]
    fn accepts_directory_containing_eso64() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let client = tmp.path().join("game").join("client");
        let exe = make_client_dir(&client);

        let found = validate_client_dir(&client).expect("client dir accepted");
        assert_eq!(found.source, ClientSource::Manual);
        assert_eq!(found.client_dir, dedupe_key(&client));
        assert_eq!(found.exe_path, dedupe_key(&exe));
    }

    #[test]
    fn accepts_the_executable_path_itself() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let client = tmp.path().join("client");
        let exe = make_client_dir(&client);

        let found = validate_client_dir(&exe).expect("exe path accepted");
        assert_eq!(found.client_dir, dedupe_key(&client));
        assert_eq!(found.exe_path, dedupe_key(&exe));
    }

    #[test]
    fn accepts_legacy_eso_exe() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let client = tmp.path().join("client");
        std::fs::create_dir_all(&client).expect("create dir");
        let exe = client.join("eso.exe");
        std::fs::write(&exe, b"").expect("write fake exe");

        let found = validate_client_dir(&client).expect("legacy client accepted");
        assert_eq!(found.exe_path, dedupe_key(&exe));
    }

    #[test]
    fn rejects_empty_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let err = validate_client_dir(tmp.path()).expect_err("empty dir rejected");
        assert!(
            err.contains("No ESO executable"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn rejects_nonexistent_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing = tmp.path().join("nope").join("client");
        let err = validate_client_dir(&missing).expect_err("missing dir rejected");
        assert!(err.contains("not found"), "unexpected message: {err}");
    }

    #[test]
    fn rejects_a_file_that_is_not_a_client_exe() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("readme.txt");
        std::fs::write(&file, b"hi").expect("write file");
        let err = validate_client_dir(&file).expect_err("plain file rejected");
        assert!(err.contains("Not a directory"), "unexpected message: {err}");
    }

    // Rust only parses Windows path prefixes on Windows targets, so the UNC
    // rejection is only observable there.
    #[cfg(windows)]
    #[test]
    fn rejects_unc_paths_before_touching_the_filesystem() {
        let err = validate_client_dir(Path::new(r"\\attacker.example\share\client"))
            .expect_err("UNC rejected");
        assert!(err.contains("UNC"), "unexpected message: {err}");
    }

    #[test]
    fn parses_installdir_out_of_an_acf() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let acf = tmp.path().join("appmanifest_306130.acf");
        std::fs::write(
            &acf,
            "\"AppState\"\n{\n\t\"appid\"\t\t\"306130\"\n\t\"installdir\"\t\t\"Zenimax Online\"\n}\n",
        )
        .expect("write acf");
        assert_eq!(
            parse_acf_installdir(&acf).as_deref(),
            Some("Zenimax Online")
        );
    }

    #[test]
    fn missing_acf_yields_no_installdir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(parse_acf_installdir(&tmp.path().join("absent.acf")).is_none());
    }

    #[test]
    fn steam_library_without_eso_is_skipped() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("steamapps")).expect("create steamapps");
        assert!(steam_client_dir_in_library(tmp.path()).is_none());
    }

    #[test]
    fn steam_library_with_eso_resolves_the_client_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let steamapps = tmp.path().join("steamapps");
        std::fs::create_dir_all(&steamapps).expect("create steamapps");
        std::fs::write(
            steamapps.join("appmanifest_306130.acf"),
            "\"AppState\"\n{\n\t\"installdir\"\t\t\"Zenimax Online\"\n}\n",
        )
        .expect("write acf");
        let expected = steamapps
            .join("common")
            .join("Zenimax Online")
            .join("game")
            .join("client");

        assert_eq!(steam_client_dir_in_library(tmp.path()), Some(expected));
    }

    #[test]
    fn steam_library_falls_back_when_installdir_is_unreadable() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let steamapps = tmp.path().join("steamapps");
        std::fs::create_dir_all(&steamapps).expect("create steamapps");
        // Present but with no installdir key at all.
        std::fs::write(
            steamapps.join("appmanifest_306130.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"306130\"\n}\n",
        )
        .expect("write acf");

        let resolved = steam_client_dir_in_library(tmp.path()).expect("resolved via fallback");
        assert!(
            resolved.starts_with(steamapps.join("common").join(STEAM_FALLBACK_INSTALLDIR)),
            "unexpected fallback path: {}",
            resolved.display()
        );
    }

    #[test]
    fn steam_library_prefers_the_nested_game_folder_when_present() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let steamapps = tmp.path().join("steamapps");
        std::fs::create_dir_all(&steamapps).expect("create steamapps");
        std::fs::write(
            steamapps.join("appmanifest_306130.acf"),
            "\"AppState\"\n{\n\t\"installdir\"\t\t\"Zenimax Online\"\n}\n",
        )
        .expect("write acf");
        let nested = steamapps
            .join("common")
            .join("Zenimax Online")
            .join("The Elder Scrolls Online");
        std::fs::create_dir_all(&nested).expect("create nested game root");

        assert_eq!(
            steam_client_dir_in_library(tmp.path()),
            Some(nested.join("game").join("client"))
        );
    }

    #[test]
    fn zos_candidates_cover_launcher_and_game_roots() {
        let root = Path::new("C:").join("Games").join("ZOS");
        let candidates = zos_client_candidates(&root);
        assert_eq!(
            candidates,
            vec![
                root.join("The Elder Scrolls Online")
                    .join("game")
                    .join("client"),
                root.join("game").join("client"),
            ]
        );
    }

    #[test]
    fn push_unique_keeps_the_first_source() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let client = tmp.path().join("client");
        make_client_dir(&client);

        let mut out = Vec::new();
        push_unique(
            &mut out,
            location_for_dir(&client, ClientSource::Steam).expect("steam location"),
        );
        push_unique(
            &mut out,
            location_for_dir(&client, ClientSource::ZosRegistry).expect("zos location"),
        );

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].source, ClientSource::Steam);
    }

    #[test]
    fn location_for_dir_ignores_a_directory_without_an_exe() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(location_for_dir(tmp.path(), ClientSource::Manual).is_none());
    }

    #[test]
    fn detect_client_locations_never_panics() {
        // Real machine state is whatever it is; the contract is only that this
        // returns (empty is a valid answer) and never panics.
        let _ = detect_client_locations();
    }
}
