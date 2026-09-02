//! Write safety for the ESO **client install** directory.
//!
//! Kalpa has historically enforced exactly one write root: every write funnels
//! through `commands::require_allowed_path`, whose `validate_addons_path`
//! requires the leaf directory be literally named `AddOns`. Exactly one command
//! (`copy_addons_to_instance`) has ever needed a second root, and it validates
//! its target against detected game instances and hard-refuses under the e2e
//! sandbox.
//!
//! Managing ReShade means placing files *next to `eso64.exe`*, which is a
//! second write root of a different kind. This module is that root's guard. It
//! is deliberately stricter than the AddOns guard, because the blast radius is
//! worse: a bad write here does not corrupt an addon, it stops the game
//! launching.
//!
//! # The gates
//!
//! 1. **Registered root.** The target directory must equal the client path the
//!    user explicitly approved via [`set_game_install_path`]. Approval requires
//!    a file named `eso64.exe` or `eso.exe` to be present. That is a mis-click
//!    guard, not an authenticity check — a zero-byte file of that name passes.
//! 2. **Containment.** Every placed path is resolved through
//!    [`safe_relative_join`], which rejects absolute paths, `..`, drive-relative
//!    forms and reserved device names, then re-checks the result is still under
//!    the root via [`assert_contained`] *after* parent directories exist, so a
//!    symlinked subdirectory cannot redirect a lexically-clean path.
//! 3. **Filename policy.** Containment alone only proves a path is *inside* the
//!    client folder — `eso64.exe` is inside it too. [`validate_placement`]
//!    additionally requires the filename to match the [`ManagedKind`] being
//!    placed, and refuses ZOS-owned names outright regardless of kind.
//! 4. **Game not running.** Swapping a proxy DLL under a live client is how you
//!    corrupt an install.
//! 5. **Not sandboxed.** Under the e2e sandbox (`KALPA_ADDONS_DIR`, debug-only)
//!    client writes are refused outright, mirroring `copy_addons_to_instance`.
//!
//! ## What gate 4 does and does not promise
//!
//! [`begin_write`] checks all of the above and hands back a plain path, so the
//! running check happens *when the caller calls it*. A caller that calls
//! `begin_write`, downloads for five minutes, and then places files has checked
//! nothing useful. Callers must re-check immediately before placing. This is
//! currently a convention rather than something the types enforce; making
//! placement require a token that only `begin_write` can mint is the obvious
//! hardening and is not yet done.
//!
//! # Reversibility
//!
//! Nothing is overwritten without first being copied into a timestamped backup
//! outside the game directory, and every file Kalpa places is recorded in a
//! manifest with its hash, so uninstall can distinguish "Kalpa put this here
//! and nobody touched it" from "someone edited this" and skip the latter.
//!
//! This is *not* an unconditional guarantee. Rollback after a failed placement
//! is best-effort: see `client_backup`'s module doc for exactly what survives a
//! rollback that itself fails. Nothing here should be read as promising the
//! client folder can always be returned to its prior state.

use crate::client_install::{validate_client_dir, EsoClientLocation};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

/// A client directory the user has explicitly approved for writes.
#[derive(Debug, Clone)]
pub struct ApprovedClientPath {
    /// The path as supplied, used for actual filesystem operations.
    pub configured: PathBuf,
    /// Canonical form, used only for comparison.
    pub canonical: PathBuf,
}

/// Managed state holding the single approved client directory, mirroring
/// `AllowedAddonsPath`. `None` until the user approves one this session.
pub struct AllowedGameInstallPath(pub Mutex<Option<ApprovedClientPath>>);

impl AllowedGameInstallPath {
    pub fn new() -> Self {
        Self(Mutex::new(None))
    }
}

impl Default for AllowedGameInstallPath {
    fn default() -> Self {
        Self::new()
    }
}

/// What kind of thing Kalpa placed, so uninstall can be selective and the UI
/// can explain each file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedKind {
    /// The ReShade proxy DLL itself (`d3d11.dll` / `dxgi.dll`).
    ReShadeCore,
    /// `ReShade.ini` and other ReShade-owned configuration.
    ReShadeConfig,
    /// A shader effect or texture under `reshade-shaders/`.
    Shader,
    /// A `.ini` preset.
    Preset,
    /// A ReShade add-on binary (`*.addon64`).
    Addon,
    /// An NVIDIA runtime DLL the user supplied.
    NvidiaRuntime,
    /// A replacement `d3dcompiler_47.dll`.
    ShaderCompiler,
}

/// One file Kalpa placed in the client directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedFile {
    /// Path relative to the client directory, always forward-slashed.
    pub relative_path: String,
    pub kind: ManagedKind,
    /// SHA-256 of the bytes Kalpa wrote, so drift is detectable: if the file on
    /// disk no longer matches, the user (or another tool) changed it and
    /// uninstall should not silently delete their work.
    pub sha256: String,
    /// RFC3339 timestamp.
    pub placed_at: String,
    /// Backup folder name holding the file this one displaced, if any.
    pub displaced_backup: Option<String>,
}

/// Everything Kalpa has placed in one client directory.
///
/// Stored in the app data dir rather than inside the game folder: the game
/// directory is exactly the thing whose integrity is in question, and a
/// manifest living inside it would be destroyed by the launcher's Repair.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManagedManifest {
    /// Keyed by canonical client directory, so multiple installs are tracked
    /// independently.
    #[serde(default)]
    pub installs: BTreeMap<String, Vec<ManagedFile>>,
}

// ── Path containment ─────────────────────────────────────────────────────

/// Join a caller-supplied relative path onto `root`, refusing anything that
/// could escape it.
///
/// Rejects absolute paths, UNC/device prefixes, `..`, empty components, and
/// Windows reserved device names. The check is purely lexical *and* is followed
/// by a containment re-check by the caller after canonicalization, because a
/// symlink planted mid-tree can still redirect a lexically-clean path.
pub fn safe_relative_join(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.trim().is_empty() {
        return Err("Empty file path.".to_string());
    }
    let rel = Path::new(relative);
    if crate::platform::has_unc_or_verbatim_prefix(rel) {
        return Err(format!("Unsupported path form: {relative}"));
    }

    let mut out = root.to_path_buf();
    for component in rel.components() {
        match component {
            Component::Normal(part) => {
                let Some(text) = part.to_str() else {
                    return Err(format!("Unsupported characters in path: {relative}"));
                };
                validate_path_segment(text)?;
                out.push(text);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!("Path may not contain '..': {relative}"));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("Path must be relative: {relative}"));
            }
        }
    }
    if out == root {
        return Err(format!(
            "Path resolves to the client folder itself: {relative}"
        ));
    }
    Ok(out)
}

/// Windows device names are reserved at every directory level and with any
/// extension, so `reshade-shaders/CON.fx` is still a device.
const RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn validate_path_segment(segment: &str) -> Result<(), String> {
    if segment.is_empty() {
        return Err("Empty path segment.".to_string());
    }
    if segment.contains(['<', '>', ':', '"', '|', '?', '*', '\0']) {
        return Err(format!("Illegal characters in path segment: {segment}"));
    }
    if segment.ends_with('.') || segment.ends_with(' ') {
        return Err(format!(
            "Path segment may not end with a dot or space: {segment}"
        ));
    }
    let stem = segment
        .split('.')
        .next()
        .unwrap_or(segment)
        .to_ascii_uppercase();
    if RESERVED_NAMES.contains(&stem.as_str()) {
        return Err(format!("Reserved device name in path: {segment}"));
    }
    Ok(())
}

/// Files owned by ZeniMax that Kalpa must never place, back up, or overwrite,
/// whatever [`ManagedKind`] a caller claims.
///
/// Containment is not enough on its own: `eso64.exe` is inside the client
/// folder, so `safe_relative_join` returns it happily. Overwriting the game
/// binary with a proxy DLL would be recorded in the manifest as a tidy,
/// reversible placement right up until the user tried to launch the game.
const PROTECTED_NAMES: [&str; 6] = [
    "eso64.exe",
    "eso.exe",
    "eso64.pdb",
    "bethesda.net_launcher.exe",
    "esolauncher.exe",
    "uninstall.exe",
];

/// Extensions belonging to the game's own data, never to a mod.
const PROTECTED_EXTENSIONS: [&str; 4] = ["mnf", "dat", "sig", "manifest"];

/// Require that `relative_path` is a plausible destination for `kind`.
///
/// Each kind may only write filenames that kind is actually about. This is what
/// stops a caller — or a malformed upstream archive listing — from parking
/// arbitrary bytes anywhere inside the client folder under a tidy-looking
/// `ManagedKind`. The type and the path have to agree.
pub fn validate_placement(kind: ManagedKind, relative_path: &str) -> Result<(), String> {
    let normalized = relative_path.replace('\\', "/");
    let file_name = normalized
        .rsplit('/')
        .next()
        .unwrap_or(&normalized)
        .to_ascii_lowercase();
    let extension = file_name.rsplit('.').next().unwrap_or_default().to_string();
    let in_shader_tree = normalized
        .to_ascii_lowercase()
        .starts_with("reshade-shaders/");
    let at_root = !normalized.trim_matches('/').contains('/');

    if PROTECTED_NAMES.contains(&file_name.as_str()) {
        return Err(format!(
            "Refusing to write {file_name}: that file belongs to the game, not to Kalpa."
        ));
    }
    if PROTECTED_EXTENSIONS.contains(&extension.as_str()) {
        return Err(format!(
            "Refusing to write {file_name}: .{extension} files belong to the game client."
        ));
    }

    let ok = match kind {
        // The proxy DLL is the injector, and only these two names are ever
        // loaded by the game's DLL search order.
        ManagedKind::ReShadeCore => {
            at_root && matches!(file_name.as_str(), "dxgi.dll" | "d3d11.dll")
        }
        ManagedKind::ReShadeConfig => at_root && matches!(extension.as_str(), "ini" | "log"),
        ManagedKind::Shader => {
            in_shader_tree && matches!(extension.as_str(), "fx" | "fxh" | "png" | "jpg" | "txt")
        }
        ManagedKind::Preset => extension == "ini",
        ManagedKind::Addon => {
            at_root && matches!(extension.as_str(), "addon64" | "addon32" | "addon")
        }
        // Kalpa never downloads these; they are user-supplied and signature
        // checked. The name still has to look like an NGX runtime.
        ManagedKind::NvidiaRuntime => {
            at_root && file_name.starts_with("nvngx_") && extension == "dll"
        }
        ManagedKind::ShaderCompiler => at_root && file_name == "d3dcompiler_47.dll",
    };

    if ok {
        Ok(())
    } else {
        Err(format!(
            "Refusing to write {relative_path}: it is not a valid destination for {kind:?}."
        ))
    }
}

/// Confirm `candidate` really sits under `root` after both are canonicalized.
///
/// Run this *after* creating parent directories but *before* writing, to catch
/// a symlinked subdirectory redirecting an otherwise-clean relative path.
pub fn assert_contained(root: &Path, candidate: &Path) -> Result<(), String> {
    let root_c = dunce::canonicalize(root)
        .map_err(|e| format!("Could not resolve the client folder: {e}"))?;
    // The candidate itself may not exist yet; canonicalize its nearest
    // existing ancestor instead.
    let mut probe = candidate.to_path_buf();
    loop {
        if probe.exists() {
            break;
        }
        match probe.parent() {
            Some(parent) if parent != probe => probe = parent.to_path_buf(),
            _ => return Err("Could not resolve the target path.".to_string()),
        }
    }
    let probe_c =
        dunce::canonicalize(&probe).map_err(|e| format!("Could not resolve target path: {e}"))?;
    if !probe_c.starts_with(&root_c) {
        return Err("Refusing to write outside the client folder.".to_string());
    }
    Ok(())
}

// ── Gates ────────────────────────────────────────────────────────────────

/// Resolve the approved client root, or explain why there isn't one.
pub fn require_allowed_client_path(
    state: &tauri::State<'_, AllowedGameInstallPath>,
    client_dir: &str,
) -> Result<PathBuf, String> {
    let location = validate_client_dir(Path::new(client_dir))?;
    let canonical = dunce::canonicalize(&location.client_dir)
        .map_err(|e| format!("Could not resolve the client folder: {e}"))?;

    let guard = state.0.lock().map_err(|_| "Internal error.".to_string())?;
    let Some(approved) = &*guard else {
        return Err(
            "No ESO client folder has been approved for changes in this session.".to_string(),
        );
    };
    if canonical != approved.canonical {
        return Err("Client folder does not match the approved folder.".to_string());
    }
    Ok(approved.configured.clone())
}

/// Refuse client writes while the e2e sandbox override is active.
///
/// The sandbox redirects only the AddOns folder; the client directory it would
/// otherwise reach is the developer's real game install. `copy_addons_to_instance`
/// makes the same refusal for the same reason.
fn refuse_under_sandbox() -> Result<(), String> {
    #[cfg(debug_assertions)]
    {
        if std::env::var_os("KALPA_ADDONS_DIR").is_some() {
            return Err(
                "Client folder changes are disabled while the addons sandbox override is active."
                    .to_string(),
            );
        }
    }
    Ok(())
}

/// All four gates, checked immediately before a write.
///
/// Returns the approved client root. The ESO-running check is deliberately
/// *here* rather than at the start of a long download, so a user who launches
/// the game mid-download is still caught.
pub async fn begin_write(
    state: &tauri::State<'_, AllowedGameInstallPath>,
    client_dir: &str,
) -> Result<PathBuf, String> {
    refuse_under_sandbox()?;
    let root = require_allowed_client_path(state, client_dir)?;
    if crate::commands::is_eso_running().await? {
        return Err(
            "The Elder Scrolls Online is running. Close the game before changing client files."
                .to_string(),
        );
    }
    Ok(root)
}

// ── Commands ─────────────────────────────────────────────────────────────

/// Approve a client directory for writes for the rest of this session.
///
/// Validation is stronger than the AddOns equivalent: rather than checking a
/// folder name, this requires an actual ESO executable to be present.
#[tauri::command]
pub fn set_game_install_path(
    state: tauri::State<'_, AllowedGameInstallPath>,
    path: String,
) -> Result<EsoClientLocation, String> {
    let location = validate_client_dir(Path::new(&path))?;
    let canonical = dunce::canonicalize(&location.client_dir)
        .map_err(|e| format!("Could not resolve the client folder: {e}"))?;
    let mut guard = state.0.lock().map_err(|_| "Internal error.".to_string())?;
    *guard = Some(ApprovedClientPath {
        configured: location.client_dir.clone(),
        canonical,
    });
    Ok(location)
}

/// Forget the approved client directory, revoking write access.
#[tauri::command]
pub fn clear_game_install_path(
    state: tauri::State<'_, AllowedGameInstallPath>,
) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|_| "Internal error.".to_string())?;
    *guard = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from(if cfg!(windows) {
            "C:\\Games\\ESO\\game\\client"
        } else {
            "/games/eso/game/client"
        })
    }

    #[test]
    fn joins_a_simple_relative_path() {
        let joined = safe_relative_join(&root(), "dxgi.dll").expect("should join");
        assert!(joined.ends_with("dxgi.dll"));
        assert!(joined.starts_with(root()));
    }

    #[test]
    fn joins_a_nested_shader_path() {
        let joined =
            safe_relative_join(&root(), "reshade-shaders/Shaders/Bloom.fx").expect("should join");
        assert!(joined.ends_with("Bloom.fx"));
        assert!(joined.starts_with(root()));
    }

    #[test]
    fn rejects_parent_traversal() {
        assert!(safe_relative_join(&root(), "../eso64.exe").is_err());
        assert!(safe_relative_join(&root(), "reshade-shaders/../../x.dll").is_err());
    }

    #[test]
    fn rejects_absolute_paths() {
        let abs = if cfg!(windows) {
            "C:\\Windows\\System32\\evil.dll"
        } else {
            "/etc/passwd"
        };
        assert!(safe_relative_join(&root(), abs).is_err());
    }

    #[test]
    fn rejects_empty_and_self_referential() {
        assert!(safe_relative_join(&root(), "").is_err());
        assert!(safe_relative_join(&root(), "   ").is_err());
        assert!(safe_relative_join(&root(), ".").is_err());
    }

    #[test]
    fn rejects_reserved_device_names_at_any_depth() {
        assert!(safe_relative_join(&root(), "NUL").is_err());
        assert!(safe_relative_join(&root(), "reshade-shaders/CON.fx").is_err());
        assert!(safe_relative_join(&root(), "aux.dll").is_err());
    }

    #[test]
    fn rejects_trailing_dot_or_space() {
        assert!(safe_relative_join(&root(), "evil.dll ").is_err());
        assert!(safe_relative_join(&root(), "evil.").is_err());
    }

    #[test]
    fn allows_current_dir_segments() {
        let joined = safe_relative_join(&root(), "./ReShade.ini").expect("should join");
        assert!(joined.ends_with("ReShade.ini"));
    }

    #[test]
    fn the_game_binary_is_never_a_valid_destination() {
        // Containment allows this path — it really is inside the client folder.
        assert!(safe_relative_join(&root(), "eso64.exe").is_ok());
        // The filename policy is what actually refuses it, under every kind.
        for kind in [
            ManagedKind::ReShadeCore,
            ManagedKind::ReShadeConfig,
            ManagedKind::Shader,
            ManagedKind::Preset,
            ManagedKind::Addon,
            ManagedKind::NvidiaRuntime,
            ManagedKind::ShaderCompiler,
        ] {
            let err = validate_placement(kind, "eso64.exe")
                .expect_err("the game binary must never be writable");
            assert!(err.contains("belongs to the game"), "{kind:?}: {err}");
        }
    }

    #[test]
    fn game_data_extensions_are_protected() {
        for name in ["game.mnf", "eso.dat", "client.sig", "install.manifest"] {
            assert!(
                validate_placement(ManagedKind::Shader, name).is_err(),
                "{name} should be refused"
            );
        }
    }

    #[test]
    fn each_kind_accepts_only_its_own_shape() {
        assert!(validate_placement(ManagedKind::ReShadeCore, "dxgi.dll").is_ok());
        assert!(validate_placement(ManagedKind::ReShadeCore, "d3d11.dll").is_ok());
        // A core DLL under any other name is not an injector the game will load.
        assert!(validate_placement(ManagedKind::ReShadeCore, "d3d9.dll").is_err());
        assert!(validate_placement(ManagedKind::ReShadeCore, "sub/dxgi.dll").is_err());

        assert!(
            validate_placement(ManagedKind::Shader, "reshade-shaders/Shaders/Bloom.fx").is_ok()
        );
        // A shader must live in the shader tree, not next to the executable.
        assert!(validate_placement(ManagedKind::Shader, "Bloom.fx").is_err());
        // …and a DLL is not a shader however it is labelled.
        assert!(validate_placement(ManagedKind::Shader, "reshade-shaders/evil.dll").is_err());

        assert!(validate_placement(ManagedKind::Addon, "dlss5-feed.addon64").is_ok());
        assert!(validate_placement(ManagedKind::Addon, "dlss5-feed.dll").is_err());

        assert!(validate_placement(ManagedKind::NvidiaRuntime, "nvngx_dlss.dll").is_ok());
        assert!(validate_placement(ManagedKind::NvidiaRuntime, "nvngx_dlssnr.dll").is_ok());
        // Not every DLL is an NGX runtime.
        assert!(validate_placement(ManagedKind::NvidiaRuntime, "dxgi.dll").is_err());

        assert!(validate_placement(ManagedKind::ShaderCompiler, "d3dcompiler_47.dll").is_ok());
        assert!(validate_placement(ManagedKind::ShaderCompiler, "d3dcompiler_43.dll").is_err());
    }

    #[test]
    fn a_config_log_must_still_sit_at_the_root() {
        assert!(validate_placement(ManagedKind::ReShadeConfig, "ReShade.ini").is_ok());
        assert!(validate_placement(ManagedKind::ReShadeConfig, "ReShade.log").is_ok());
        // Guards a precedence mistake: `at_root && ini || log` would let this pass.
        assert!(validate_placement(ManagedKind::ReShadeConfig, "nested/deep/ReShade.log").is_err());
    }

    #[test]
    fn the_policy_is_case_insensitive() {
        assert!(validate_placement(ManagedKind::ReShadeCore, "DXGI.DLL").is_ok());
        assert!(validate_placement(ManagedKind::Shader, "ESO64.EXE").is_err());
        assert!(validate_placement(ManagedKind::Shader, "RESHADE-SHADERS/A.FX").is_ok());
    }

    #[test]
    fn backslash_separators_are_normalised_before_matching() {
        // An upstream archive listing may use Windows separators; the policy
        // must see the same shape either way.
        assert!(validate_placement(ManagedKind::Shader, "reshade-shaders\\Shaders\\A.fx").is_ok());
        assert!(validate_placement(ManagedKind::ReShadeCore, "sub\\dxgi.dll").is_err());
    }

    #[test]
    fn assert_contained_accepts_a_child_and_rejects_a_sibling() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("client");
        std::fs::create_dir_all(&root).expect("mkdir");
        let inside = root.join("dxgi.dll");
        assert!(assert_contained(&root, &inside).is_ok());

        let outside = tmp.path().join("elsewhere").join("dxgi.dll");
        std::fs::create_dir_all(outside.parent().unwrap()).expect("mkdir");
        assert!(assert_contained(&root, &outside).is_err());
    }
}
