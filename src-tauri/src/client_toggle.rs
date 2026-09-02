//! Switching a managed client stack off, and back on.
//!
//! # What "disable" means here
//!
//! Not "remove everything". Disable puts **ESO back to stock** and leaves the
//! stack sitting in the folder, switched off:
//!
//! * The injector (`dxgi.dll` / `d3d11.dll`) is **parked** — renamed to
//!   `…{PARKED_SUFFIX}`. Nothing else in the stack is loaded by anything except
//!   ReShade, so removing the one file the game's DLL search order picks up is
//!   what actually switches the stack off.
//! * Files that **replace something ESO ships** — `nvngx_dlss.dll`,
//!   `d3dcompiler_47.dll` — cannot merely be parked, because the game loads
//!   those itself. Their originals have to go **live**: park the modded file,
//!   then copy the user's own preserved original back over the live name.
//! * The Neural Rendering runtime, the add-ons, the shader tree, the preset and
//!   the tuning block are **left exactly where they are**. Without an injector
//!   nothing loads them, so they are inert, and leaving them untouched is what
//!   makes re-enable a pure reversal rather than a reinstall.
//!
//! # The suffix
//!
//! Kalpa parks as [`crate::client_stack::PARKED_SUFFIX`] and never anything
//! else. `.disabled-bak` and `.eso-orig-bak` are the *user's* names for their
//! own originals — in a real install `nvngx_dlss.dll.disabled-bak` is the stock
//! DLL this whole operation depends on. Parking a live file under one of those
//! names would overwrite the one file disable exists to restore.
//!
//! # Why the plan is the confirmation
//!
//! The user is being asked to approve changes to their game folder. A dialog
//! that says "disable the stack?" asks them to trust a description; the plan
//! lists every operation, one line each, in the order it will run, and is
//! computed by the backend from what is actually on disk. The UI's button stays
//! disabled until the plan has loaded, so nothing is ever approved sight-unseen.
//!
//! Nothing here writes to the filesystem itself: [`plan_toggle`] is pure and
//! the apply path hands a [`FileOp`] batch to
//! [`crate::client_backup::run_file_ops`], which owns the lock, the
//! re-assertion of the client-not-running gate, and the rollback.

use crate::client_backup::FileOp;
use crate::client_stack::{ClientStack, StackRole};
use crate::client_write::{AllowedGameInstallPath, ManagedFile};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Which direction the user is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToggleAction {
    Disable,
    Enable,
}

/// The shape of one planned step, so the UI can group and icon them without
/// parsing prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToggleOpKind {
    /// Rename a live file aside so nothing loads it.
    Park,
    /// Copy the user's own preserved original back over a live name.
    RestoreOriginal,
    /// Rename a parked file back to its live name.
    Unpark,
    /// Remove the stock file a previous disable put live, freeing the name.
    RemoveRestored,
    /// Nothing happens to this file, and the plan says so explicitly. An
    /// operation list that silently omits two thirds of the stack invites the
    /// question "so what happened to my shaders?".
    LeaveInPlace,
}

/// One line of the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedOp {
    pub kind: ToggleOpKind,
    /// The file this step is about, relative to the client directory.
    pub file_name: String,
    /// One short line, shown verbatim: "Park dxgi.dll as dxgi.dll.kalpa-off".
    pub summary: String,
    /// Why this step exists, in the user's terms.
    pub detail: String,
}

/// Everything the confirmation needs, computed from the folder as it is now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TogglePlan {
    pub client_dir: String,
    /// The action this plan describes — the opposite of the current state.
    pub action: ToggleAction,
    /// True when the stack is currently switched off.
    pub is_disabled: bool,
    /// Ordered exactly as the operations will run. Steps of kind
    /// [`ToggleOpKind::LeaveInPlace`] are informational and produce no
    /// [`FileOp`].
    pub operations: Vec<PlannedOp>,
    /// Reasons the action cannot proceed, each shown verbatim. Non-empty means
    /// the confirm button stays disabled.
    pub blockers: Vec<String>,
}

/// Build the plan. Pure: no filesystem access beyond what the caller already
/// read into `stack`, and no writes at all.
///
/// `managed` is this install's manifest bucket. Disable operates on managed
/// entries only — Kalpa switching off files it has no record of would be
/// rearranging a folder it was never asked to manage.
///
/// ## Disable, in order
///
/// 1. For every non-parked managed entry whose `displaced_in_place` names a
///    preserved original that is present on disk, and whose role is one the
///    game loads itself ([`StackRole::SuperSampling`],
///    [`StackRole::ShaderCompiler`], [`StackRole::FrameGeneration`]): a
///    [`ToggleOpKind::Park`] of the live file followed immediately by a
///    [`ToggleOpKind::RestoreOriginal`] copying the original over the freed
///    name. Park first — the copy needs the name to be free.
/// 2. The injector, parked last, so a failure part-way through leaves a folder
///    whose injector is still the thing loading it rather than a half-stock mix.
/// 3. One [`ToggleOpKind::LeaveInPlace`] line per remaining managed file.
///
/// ## Enable
///
/// The exact reverse: unpark the injector, then for each other parked entry a
/// [`ToggleOpKind::RemoveRestored`] (naming its `displaced_in_place` as the
/// file whose bytes must still match) followed by a [`ToggleOpKind::Unpark`].
///
/// ## Blockers
///
/// * Disable with no injector present: nothing to switch off.
/// * Disable with no managed entries: the stack is not managed yet.
/// * Disable where a runtime that replaces an ESO-shipped file has no
///   `displaced_in_place` on disk: switching off would leave the game with no
///   file under that name at all. Name the file.
/// * Enable where a parked file is no longer in the folder. Name it.
pub fn plan_toggle(stack: &ClientStack, managed: &[ManagedFile], client_dir: &str) -> TogglePlan {
    let _ = (stack, managed, client_dir);
    todo!("plan_toggle")
}

/// The role of a managed entry, resolved from the stack inventory.
///
/// A manifest entry records a `ManagedKind`, which is a write-policy category,
/// not a position in the pipeline. The role is what decides whether a file can
/// simply be parked or has to have an original put back in its place, so it is
/// read from the live inventory.
pub fn role_of(stack: &ClientStack, relative_path: &str) -> Option<StackRole> {
    let _ = (stack, relative_path);
    todo!("role_of")
}

/// True for roles the **game itself** loads, which therefore cannot be left
/// with no file under their name.
///
/// `nvngx_dlssnr.dll` is deliberately absent: ESO does not ship or load it, so
/// with the injector parked it is simply inert.
pub fn game_loads_itself(role: StackRole) -> bool {
    let _ = role;
    todo!("game_loads_itself")
}

/// Translate the plan into the batch [`crate::client_backup::run_file_ops`]
/// will execute. [`ToggleOpKind::LeaveInPlace`] steps produce nothing.
pub fn to_file_ops(plan: &TogglePlan) -> Vec<FileOp> {
    let _ = plan;
    todo!("to_file_ops")
}

// ── Commands ─────────────────────────────────────────────────────────────

/// Read-only: what switching this stack would do.
#[tauri::command]
pub fn plan_client_toggle(app: tauri::AppHandle, client_dir: String) -> Result<TogglePlan, String> {
    let _ = (app, client_dir);
    todo!("plan_client_toggle")
}

/// Switch the stack off, or back on.
///
/// `expected` is a compare-and-swap against the current state, not a request:
/// the plan the user approved described one direction, and if the folder has
/// changed under them since (another window, a game update) the right answer is
/// to refuse and re-plan, not to run the other direction silently.
///
/// The plan is recomputed here from the directory rather than accepted from the
/// caller, for the same reason `adopt_stack` recomputes its own.
#[tauri::command]
pub async fn apply_client_toggle(
    app: tauri::AppHandle,
    state: tauri::State<'_, AllowedGameInstallPath>,
    client_dir: String,
    expected: ToggleAction,
) -> Result<crate::client_backup::FileOpOutcome, String> {
    let _ = (app, state, client_dir, expected);
    todo!("apply_client_toggle")
}

/// This install's manifest bucket.
pub fn managed_entries(manifest_path: &Path, client_root: &Path) -> Vec<ManagedFile> {
    let manifest = crate::client_backup::load_manifest_at(manifest_path);
    manifest
        .installs
        .get(&crate::client_backup::install_key(client_root))
        .cloned()
        .unwrap_or_default()
}
