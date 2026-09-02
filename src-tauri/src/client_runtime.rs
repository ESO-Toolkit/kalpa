//! Runtime drift: noticing a game update undid a swap, and putting it back.
//!
//! # The failure this exists for
//!
//! ESO ships its own `nvngx_dlss.dll` and `d3dcompiler_47.dll`. A DLSS 5 setup
//! replaces both. The ZOS launcher's patcher rewrites the client folder on every
//! game update, which puts ESO's own builds back over the user's swap — silently,
//! with no error, leaving a stack that loads fine and renders worse. The tell in
//! a real install is that the user keeps `nvngx_dlss.dll.disabled-bak` and
//! `d3dcompiler_47.dll.eso-orig-bak` by hand: they have been through this.
//!
//! # Kalpa downloads nothing
//!
//! The NVIDIA runtimes are not redistributable and Kalpa hosts and mirrors
//! nothing, so there is no upstream to fetch a replacement from. **The user's
//! own kept bytes are the only source.** Adoption offers to copy the swapped
//! runtimes into Kalpa's backup root for exactly this moment, and that offer is
//! opt-in because the Neural Rendering runtime alone is around 165 MB.
//!
//! # Degrading honestly
//!
//! When no copy was kept there is nothing clever to do, and pretending otherwise
//! would be worse than saying so. Kalpa reports the drift, names the file, says
//! plainly that it cannot put it back and why, and points at re-adopting with
//! copies kept so the *next* game update is recoverable. It does not offer a
//! button that will fail, and it never suggests a download.

use crate::client_stack::{ClientStack, StackRole};
use crate::client_write::{AllowedGameInstallPath, ManagedFile};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// What has happened to one managed runtime since Kalpa last recorded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftState {
    /// On disk and byte-identical to the manifest. Nothing to do.
    Unchanged,
    /// The bytes differ from the manifest and Kalpa holds a kept copy, so the
    /// swap can be put back with one action.
    DriftedRecoverable,
    /// The bytes differ and no copy was kept. Reportable, not fixable — see the
    /// module doc.
    DriftedUnrecoverable,
    /// The manifest names a file that is not in the folder at all.
    Missing,
    /// Parked by "switch this stack off". Not drift: the live file under this
    /// name is supposed to be the game's own.
    Parked,
}

/// One managed runtime and what has become of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeStatus {
    pub relative_path: String,
    pub role: StackRole,
    pub state: DriftState,
    /// Version resource of what is on disk now, when it has one.
    pub current_version: Option<String>,
    /// Version resource of the kept copy, when there is one. Shown beside
    /// `current_version` so "reverted to 2.2.16, yours was 310.1" is a fact the
    /// user can read rather than a claim Kalpa makes.
    pub kept_version: Option<String>,
    /// Backup folder id holding the kept copy.
    pub kept_backup_id: Option<String>,
    pub size_bytes: u64,
    /// The user's own preserved original beside it, if any. Never touched here;
    /// shown because it is what makes the drift legible.
    pub displaced_in_place: Option<String>,
}

/// Drift across one client directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeReport {
    pub client_dir: String,
    /// Every managed runtime, in path order.
    pub runtimes: Vec<RuntimeStatus>,
    /// Paths that can be put back. Empty means the re-apply action is not
    /// offered at all, rather than offered and failing.
    pub recoverable: Vec<String>,
    /// Paths that drifted with no kept copy. The honest-degradation list: the
    /// UI names these and says Kalpa cannot fix them.
    pub unrecoverable: Vec<String>,
}

/// What re-applying would do, one line per file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReapplyStep {
    pub relative_path: String,
    /// "Put your 310.1 back over ESO's 2.2.16" — built from the two version
    /// resources, or from sizes when a file has none.
    pub summary: String,
    pub kept_backup_id: String,
}

/// The result of a re-apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReapplyOutcome {
    pub restored: Vec<String>,
    /// Paths that could not be restored, with the reason. Best-effort per file:
    /// one unreadable kept copy must not abandon the rest.
    pub skipped: Vec<String>,
}

/// Roles this module tracks: the files a game update can overwrite.
///
/// The injector, the add-ons and the shader tree are not here — ESO does not
/// ship them, so no patch restores anything over them.
pub fn is_drift_prone(role: StackRole) -> bool {
    let _ = role;
    todo!("is_drift_prone")
}

/// Build the drift report. Read-only.
///
/// For each managed entry whose role [`is_drift_prone`]:
///
/// * `parked` entries are [`DriftState::Parked`] and never counted as drift.
/// * A file that is not on disk is [`DriftState::Missing`].
/// * A file whose SHA-256 matches the manifest is [`DriftState::Unchanged`].
/// * Otherwise it has drifted, and which of the two drifted states it gets
///   depends on one thing only: whether `displaced_backup` names a backup folder
///   that actually **contains** the file. A recorded id whose file has gone —
///   pruned, or a wiped app-data folder — is unrecoverable, and must be reported
///   as such rather than offered as a fix that fails at the last moment.
pub fn inspect_runtimes(
    stack: &ClientStack,
    managed: &[ManagedFile],
    backup_root: &Path,
    client_root: &Path,
) -> RuntimeReport {
    let _ = (stack, managed, backup_root, client_root);
    todo!("inspect_runtimes")
}

/// The plan for putting `paths` back, in the report's order.
///
/// Only paths in [`RuntimeReport::recoverable`] produce a step; anything else is
/// silently absent from the plan rather than producing a step that would fail.
pub fn plan_reapply(report: &RuntimeReport, paths: &[String]) -> Vec<ReapplyStep> {
    let _ = (report, paths);
    todo!("plan_reapply")
}

/// Where one kept copy lives.
pub fn kept_copy_path(
    backup_root: &Path,
    backup_id: &str,
    relative_path: &str,
) -> Result<PathBuf, String> {
    crate::client_backup::backup_file_path(backup_root, backup_id, relative_path)
}

// ── Commands ─────────────────────────────────────────────────────────────

/// Read-only: which managed runtimes have drifted, and which can be put back.
#[tauri::command]
pub fn inspect_client_runtimes(
    app: tauri::AppHandle,
    client_dir: String,
) -> Result<RuntimeReport, String> {
    let _ = (app, client_dir);
    todo!("inspect_client_runtimes")
}

/// Put the user's own kept bytes back over the files a game update reverted.
///
/// Each file goes through `client_backup::edit_managed_file_from`, which backs
/// up what is there now, streams the kept copy in, and refreshes the manifest
/// hash **without** changing how the entry got there — these are adopted files,
/// and re-recording them as placed would tell uninstall it may delete them.
///
/// The report is recomputed here rather than accepted from the caller: a report
/// the frontend built minutes ago describes a folder that may have changed.
#[tauri::command]
pub async fn reapply_client_runtimes(
    app: tauri::AppHandle,
    state: tauri::State<'_, AllowedGameInstallPath>,
    client_dir: String,
    relative_paths: Vec<String>,
) -> Result<ReapplyOutcome, String> {
    let _ = (app, state, client_dir, relative_paths);
    todo!("reapply_client_runtimes")
}
