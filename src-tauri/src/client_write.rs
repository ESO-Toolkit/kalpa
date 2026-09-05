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
//! ## How gate 4 is enforced
//!
//! [`begin_write`] checks all of the above and hands back an [`ApprovedRoot`],
//! not a plain path. `ApprovedRoot` has private fields and exactly one
//! constructor outside of tests, so a caller cannot fabricate one: the only way
//! to name a client directory to `client_backup::apply_placements` or
//! `client_backup::revert_placements` is to have gone through every gate here.
//!
//! That alone would still only prove the mutable gates held *at some point*: a
//! caller can pass them, wait behind another transaction, and then write after
//! the user selected a different installation or the client started. The token
//! therefore carries both checks and the transaction path calls
//! [`ApprovedRoot::reassert_write_allowed`] from inside its critical section,
//! immediately before it touches the filesystem. The checks are injected
//! closures so tests can drive each stale-token case deterministically.
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
use std::sync::{Arc, Mutex};

/// A client directory the user has explicitly approved for writes.
#[derive(Debug, Clone)]
pub struct ApprovedClientPath {
    /// The path as supplied, used for actual filesystem operations.
    pub configured: PathBuf,
    /// Canonical form, used only for comparison.
    pub canonical: PathBuf,
}

#[derive(Debug, Default)]
struct GameInstallApproval {
    current: Option<ApprovedClientPath>,
    generation: u64,
}

/// Managed state holding the single approved client directory, mirroring
/// `AllowedAddonsPath`. `None` until the user approves one this session.
pub struct AllowedGameInstallPath(Arc<Mutex<GameInstallApproval>>);

impl AllowedGameInstallPath {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(GameInstallApproval::default())))
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

/// How a file came to be in the manifest.
///
/// The distinction is not bookkeeping. It decides whether Kalpa is allowed to
/// delete the file: uninstall removes what Kalpa put there, and Kalpa put
/// none of an adopted stack there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOrigin {
    /// Kalpa wrote this file. Uninstall may remove it and restore whatever it
    /// displaced.
    #[default]
    Placed,
    /// The file was already here and the user asked Kalpa to manage it.
    /// Recorded for drift detection only. Uninstall must never delete it —
    /// there is no displaced original of Kalpa's to put back, and the bytes
    /// are the user's.
    Adopted,
}

/// One file Kalpa placed in, or adopted from, the client directory.
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
    /// Backup folder name holding the file this one displaced, if any. Set
    /// only when Kalpa did the displacing, and the copy lives under
    /// `client_backup`'s backup root.
    pub displaced_backup: Option<String>,
    /// Defaults to [`FileOrigin::Placed`] so manifests written before adoption
    /// existed keep their meaning.
    #[serde(default)]
    pub origin: FileOrigin,
    /// Relative path, inside the client directory, of a backup the **user**
    /// made before Kalpa was involved — `nvngx_dlss.dll.disabled-bak` and the
    /// like.
    ///
    /// This is deliberately not folded into [`displaced_backup`]. That field
    /// names a folder under the backup root, and
    /// `prune_unreferenced_backups` deletes folders there that nothing points
    /// at. A path recorded here is somewhere Kalpa does not own and must never
    /// move, rename or prune: it is the user's file, sitting where they left
    /// it, and it is frequently the only copy of the original in existence.
    #[serde(default)]
    pub displaced_in_place: Option<String>,
    /// True while this file is parked — moved aside to
    /// `relative_path + client_stack::PARKED_SUFFIX` so the game does not load
    /// it, with the stack switched off.
    ///
    /// The entry keeps its live `relative_path` rather than being rewritten to
    /// the parked name, because parking is temporary and the live name is what
    /// re-enabling puts back. Anything that resolves an entry to a file on disk
    /// has to consult this flag: without it a parked file reads as deleted, and
    /// uninstall would "restore" over a stack the user only switched off.
    #[serde(default)]
    pub parked: bool,
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

/// A licence or attribution notice, matched by **exact name** rather than by
/// extension.
///
/// These are permitted in the shader tree because dropping them can be a
/// licence breach: LumeniteFX's `NOTICE` carries the MIT attributions for
/// Glamarye's Fast Effects and Alan Wolfe's blue-noise texture, and MIT
/// requires that the notice accompany copies. Installing a pack's shaders and
/// silently discarding its attribution file is not a tidier install, it is a
/// non-compliant one.
///
/// Scoped as narrowly as it can be. Extension matching would let anything named
/// `evil.md` through; whole-name matching admits six specific files, none of
/// which is executable or loadable by ReShade. `license.txt` and `notice.txt`
/// are listed for completeness — they already pass on their extension.
fn is_attribution_file(file_name: &str) -> bool {
    matches!(
        file_name,
        "license" | "license.md" | "license.txt" | "notice" | "notice.md" | "notice.txt"
    )
}

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
        // `cfg` is here because ReShade addons ship their own configuration
        // next to the injector -- `dlss5-feed.cfg` is a real example. Without
        // it, adoption records a companion file under a kind this policy then
        // refuses to write, so managing the file later fails at the last gate.
        ManagedKind::ReShadeConfig => {
            at_root && matches!(extension.as_str(), "ini" | "log" | "cfg")
        }
        ManagedKind::Shader => {
            in_shader_tree
                && (matches!(extension.as_str(), "fx" | "fxh" | "png" | "jpg" | "txt")
                    || is_attribution_file(&file_name))
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
    resolve_allowed_client_path(state, client_dir).map(|(configured, _, _)| configured)
}

fn resolve_allowed_client_path(
    state: &AllowedGameInstallPath,
    client_dir: &str,
) -> Result<(PathBuf, PathBuf, u64), String> {
    let location = validate_client_dir(Path::new(client_dir))?;
    let canonical = dunce::canonicalize(&location.client_dir)
        .map_err(|e| format!("Could not resolve the client folder: {e}"))?;

    let guard = state.0.lock().map_err(|_| "Internal error.".to_string())?;
    let Some(approved) = &guard.current else {
        return Err(
            "No ESO client folder has been approved for changes in this session.".to_string(),
        );
    };
    if canonical != approved.canonical {
        return Err("Client folder does not match the approved folder.".to_string());
    }
    Ok((
        approved.configured.clone(),
        approved.canonical.clone(),
        guard.generation,
    ))
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

/// Answers "is the ESO client or its launcher active right now?".
///
/// Injected into [`ApprovedRoot`] rather than called directly so the
/// client-started-mid-batch path is testable without a running game.
pub type ClientActiveCheck = Arc<dyn Fn() -> Result<bool, String> + Send + Sync>;
type WriteAuthorityCheck = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

/// The message shown whenever gate 4 refuses. One constant so the initial
/// check and every re-assertion say the same thing.
const CLIENT_ACTIVE_MESSAGE: &str =
    "The Elder Scrolls Online or its launcher is running. Close both before changing \
     client files — the launcher's patcher writes to this folder too.";

/// Proof that a client directory passed every gate in this module, and the
/// means to re-prove every gate that can go stale.
///
/// This is the capability token for client-directory writes. Its fields are
/// private and [`begin_write`] is its only non-test constructor, so a function
/// that takes an `&ApprovedRoot` cannot be handed an arbitrary path: possession
/// of the token *is* the evidence that the sandbox check, the approved-root
/// check and the running check all ran.
///
/// Containment and filename policy are stable properties of paths, and the
/// sandbox override does not appear mid-session. Approval and client-idle state
/// can both decay while a token waits, so the token keeps the checks rather
/// than their answers and the transaction re-runs them via
/// [`reassert_write_allowed`](Self::reassert_write_allowed).
///
/// Cloning is deliberately allowed: a clone re-checks like the original, so it
/// carries no stale authority.
#[derive(Clone)]
pub struct ApprovedRoot {
    root: PathBuf,
    is_client_active: ClientActiveCheck,
    is_authority_current: WriteAuthorityCheck,
}

impl std::fmt::Debug for ApprovedRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApprovedRoot")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl ApprovedRoot {
    /// The approved client directory, in the form supplied by the user (not
    /// canonicalized), for actual filesystem operations.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Re-run gate 4.
    ///
    /// Call this immediately before touching the filesystem, from inside
    /// whatever lock the write path holds. Returning `Ok` means the client was
    /// idle at this instant; it is not a lease, and a batch that runs for a
    /// long time should not assume one check covers all of it.
    pub fn reassert_idle(&self) -> Result<(), String> {
        if (self.is_client_active)()? {
            return Err(CLIENT_ACTIVE_MESSAGE.to_string());
        }
        Ok(())
    }

    /// Re-prove that the approval which minted this token is still current.
    pub fn reassert_authority(&self) -> Result<(), String> {
        (self.is_authority_current)()
    }

    /// Re-run every gate that can change after this token is minted.
    pub fn reassert_write_allowed(&self) -> Result<(), String> {
        self.reassert_authority()?;
        self.reassert_idle()
    }

    /// Mint a token with an arbitrary root and running check.
    ///
    /// Test-only, and `#[cfg(test)]` rather than `#[doc(hidden)]` on purpose:
    /// the entire value of the type is that production code has exactly one
    /// way to obtain one. `cfg(test)` is crate-wide under `cargo test`, so
    /// `client_backup`'s tests can call this too.
    #[cfg(test)]
    pub fn for_tests(root: PathBuf, is_client_active: ClientActiveCheck) -> Self {
        Self {
            root,
            is_client_active,
            is_authority_current: Arc::new(|| Ok(())),
        }
    }

    #[cfg(test)]
    pub fn for_tests_with_authority(
        root: PathBuf,
        is_authority_current: WriteAuthorityCheck,
    ) -> Self {
        Self {
            root,
            is_client_active: Arc::new(|| Ok(false)),
            is_authority_current,
        }
    }

    /// [`for_tests`](Self::for_tests) with a check that always reports the
    /// client idle — the usual case for tests that are not about gate 4.
    #[cfg(test)]
    pub fn for_tests_idle(root: PathBuf) -> Self {
        Self::for_tests(root, Arc::new(|| Ok(false)))
    }
}

/// All four gates, checked immediately before a write.
///
/// Returns an [`ApprovedRoot`] rather than a path, so the gates cannot be
/// bypassed by a caller that simply never asked. The ESO-running check runs
/// here *and* again inside the placement itself, so a user who launches the
/// game part-way through a long download is still caught.
pub async fn begin_write(
    state: &tauri::State<'_, AllowedGameInstallPath>,
    client_dir: &str,
) -> Result<ApprovedRoot, String> {
    refuse_under_sandbox()?;
    let (root, canonical, generation) = resolve_allowed_client_path(state, client_dir)?;
    let approval = Arc::clone(&state.0);
    let authority_root = root.clone();
    let is_authority_current: WriteAuthorityCheck = Arc::new(move || {
        let current_canonical = dunce::canonicalize(&authority_root)
            .map_err(|e| format!("The approved client folder is no longer available: {e}"))?;
        let guard = approval.lock().map_err(|_| "Internal error.".to_string())?;
        let Some(current) = &guard.current else {
            return Err("Write access to this client folder has been revoked.".to_string());
        };
        if guard.generation != generation
            || current.canonical != canonical
            || current_canonical != canonical
        {
            return Err(
                "The approved client folder changed while this operation was waiting. Try again."
                    .to_string(),
            );
        }
        Ok(())
    });
    // Deliberately the launcher-aware check, not `is_eso_running`. The ZOS
    // launcher's patcher rewrites files in the client directory during a game
    // update or a Repair — including `nvngx_dlss.dll` — so writing while it is
    // open is a race against a process that is also mid-write. `is_eso_running`
    // stays narrow because the migration preconditions and the ESO-running
    // dialog use it to say "close the game", and an idle launcher should not
    // trip those.
    let approved = ApprovedRoot {
        root,
        is_client_active: Arc::new(crate::commands::is_eso_or_launcher_running_blocking),
        is_authority_current,
    };
    // The first assertion goes to the blocking pool because `begin_write` is
    // called from async command handlers; later re-assertions are synchronous
    // because they happen inside placement work that is already blocking.
    let probe = approved.clone();
    tokio::task::spawn_blocking(move || probe.reassert_write_allowed())
        .await
        .map_err(|e| format!("Task failed: {e}"))??;
    Ok(approved)
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
    guard.generation = guard.generation.wrapping_add(1);
    guard.current = Some(ApprovedClientPath {
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
    guard.generation = guard.generation.wrapping_add(1);
    guard.current = None;
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

    /// Attribution files are allowed in the shader tree because MIT-derived
    /// packs require their notice to travel with the copies. The allowance is
    /// by whole name, so it must not generalise to the extension.
    #[test]
    fn attribution_files_are_allowed_only_by_exact_name() {
        for allowed in [
            "reshade-shaders/Shaders/LICENSE",
            "reshade-shaders/Shaders/NOTICE",
            "reshade-shaders/Shaders/license.md",
            "reshade-shaders/Shaders/NOTICE.md",
        ] {
            assert!(
                validate_placement(ManagedKind::Shader, allowed).is_ok(),
                "{allowed} should be allowed"
            );
        }

        for refused in [
            // Not an attribution name — the .md allowance must not generalise.
            "reshade-shaders/Shaders/readme.md",
            "reshade-shaders/Shaders/evil.md",
            // Still not loadable content, whatever it is called.
            "reshade-shaders/Shaders/LICENSE.dll",
            "reshade-shaders/Shaders/notice.exe",
            // And the tree boundary still applies to attribution files.
            "LICENSE",
            "NOTICE",
        ] {
            assert!(
                validate_placement(ManagedKind::Shader, refused).is_err(),
                "{refused} should be refused"
            );
        }
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

    #[test]
    fn reassert_idle_passes_only_while_the_client_is_idle() {
        let idle = ApprovedRoot::for_tests_idle(PathBuf::from("client"));
        assert!(idle.reassert_idle().is_ok());

        let active = ApprovedRoot::for_tests(PathBuf::from("client"), Arc::new(|| Ok(true)));
        let error = active
            .reassert_idle()
            .expect_err("an active client must refuse");
        assert!(error.contains("running"), "{error}");
    }

    /// The check runs on every call rather than being answered once and
    /// cached. This is the entire point of holding the closure instead of a
    /// boolean: a token minted before a five-minute download must not still
    /// be asserting the state of the machine five minutes ago.
    #[test]
    fn reassert_idle_re_runs_the_check_each_time() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let root = ApprovedRoot::for_tests(
            PathBuf::from("client"),
            Arc::new(move || {
                // Idle at first, then the user launches the game.
                Ok(counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0)
            }),
        );

        assert!(
            root.reassert_idle().is_ok(),
            "first check sees an idle client"
        );
        assert!(
            root.reassert_idle().is_err(),
            "the second check must see the client that started in between"
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    /// A clone must not be a way to keep an answer that has gone stale.
    #[test]
    fn a_clone_re_checks_like_the_original() {
        let root = ApprovedRoot::for_tests(PathBuf::from("client"), Arc::new(|| Ok(true)));
        assert!(root.clone().reassert_idle().is_err());
    }

    #[test]
    fn approved_root_exposes_the_path_it_was_minted_for() {
        let root = ApprovedRoot::for_tests_idle(PathBuf::from("some/client"));
        assert_eq!(root.path(), Path::new("some/client"));
    }
}
