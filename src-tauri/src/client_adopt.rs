//! Adopting a stack Kalpa did not install.
//!
//! Everything else in the client-directory layer assumes Kalpa placed the
//! files: the manifest records what was written, the hash proves nobody has
//! touched it since, and uninstall puts back what was displaced. That model
//! describes a stack Kalpa built. It has nothing to say about the far more
//! common case — a working DLSS 5 / ReShade setup the user assembled by hand,
//! possibly years before Kalpa could do any of this.
//!
//! Adoption is the bridge. It records what is already there so drift becomes
//! detectable and the management features have something to act on. It is
//! deliberately the least invasive operation in this module tree:
//!
//! * **Nothing is downloaded, moved, renamed or deleted.** Adoption writes
//!   manifest entries and, at the user's option, copies the swapped runtimes
//!   into Kalpa's backup root. The client directory is not modified at all.
//! * **The user's own backups stay exactly where they are.** A hand-made
//!   `nvngx_dlss.dll.disabled-bak` is recorded through
//!   [`ManagedFile::displaced_in_place`], which names a path Kalpa does not
//!   own. It is never folded into `displaced_backup`, because that field names
//!   a folder under the backup root and `prune_unreferenced_backups` deletes
//!   unreferenced folders there. Filing the user's only copy of their original
//!   under a pruning policy would be a way to lose it.
//! * **Adopted entries are never deletable by uninstall.** They carry
//!   [`FileOrigin::Adopted`], and `client_backup::revert_placements` skips
//!   them. Kalpa did not place them, so there is no displaced original of
//!   Kalpa's to restore and removing them would just destroy the user's files.
//!   Dropping the records is [`forget_stack`]; putting originals back live is
//!   the separate disable operation.
//!
//! # Why keeping a copy is opt-in, and what it buys
//!
//! Kalpa hosts and mirrors nothing, and the NVIDIA runtimes are not
//! redistributable, so Kalpa can never *fetch* a replacement for a runtime the
//! ZOS patcher overwrites. Keeping a copy of the user's own swapped bytes at
//! adoption time is therefore the only mechanism by which "re-apply after a
//! game update" can exist at all. It is opt-in because it is not free — the
//! Neural Rendering runtime alone is around 165 MB. Declining is a real
//! choice, and drift detection still works without it; only the one-click fix
//! goes away.

use crate::client_stack::{ClientStack, StackItem, StackRole};
use crate::client_write::{
    AllowedGameInstallPath, ApprovedRoot, FileOrigin, ManagedFile, ManagedKind,
};
use serde::Serialize;
use std::path::Path;

/// One file adoption proposes to record.
///
/// This is the unit the confirmation UI lists. Every field exists so the user
/// can check Kalpa's reasoning rather than take it on trust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdoptionEntry {
    /// Forward-slashed, relative to the client directory.
    pub relative_path: String,
    pub kind: ManagedKind,
    pub role: StackRole,
    /// Best available human name, or `None` when the file genuinely carries
    /// no identifying resource.
    pub display_name: Option<String>,
    pub version: Option<String>,
    pub size_bytes: u64,
    /// The user's own backup of whatever this file replaced, if one was found
    /// next to it. Recorded as a reference; never moved.
    pub displaced_in_place: Option<String>,
    /// True when this file is one Kalpa would keep a copy of, given the
    /// option — the runtimes a game update can overwrite.
    pub copyable: bool,
}

/// What [`adopt_stack`] would do, computed without touching anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdoptionPlan {
    pub client_dir: String,
    pub entries: Vec<AdoptionEntry>,
    /// Total bytes of the entries marked `copyable`, so the UI can state the
    /// cost of keeping copies instead of making the user guess.
    pub copy_bytes: u64,
    /// True when this install already has adopted entries recorded.
    pub already_managed: bool,
    /// True when there is nothing recognisable to adopt.
    pub is_empty: bool,
}

/// The result of adopting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdoptionOutcome {
    pub recorded: Vec<String>,
    /// Files whose bytes were copied into Kalpa's backup root so a later
    /// game update can be undone. Empty when the user declined.
    pub copied: Vec<String>,
    /// Entries that could not be recorded, with the reason. Adoption is
    /// best-effort per file: one unreadable DLL must not abandon the rest.
    pub skipped: Vec<String>,
}

/// The manifest kind that best describes a stack item.
///
/// The mapping is not cosmetic: `ManagedKind` drives
/// `client_write::validate_placement`, so a wrong kind here would let a later
/// write put the wrong sort of file in the wrong place.
pub fn kind_for_role(role: StackRole, file_name: &str) -> ManagedKind {
    todo!("kind_for_role")
}

/// Which stack items are worth keeping a copy of.
///
/// Only the runtimes, and only because the ZOS patcher rewrites them on a game
/// update. Copying the 165 MB Neural Rendering runtime is defensible for that
/// reason; copying the shader tree is not, since nothing overwrites it.
pub fn is_copyable(item: &StackItem) -> bool {
    todo!("is_copyable")
}

/// Compute the adoption plan for a client directory. Read-only.
pub fn plan_adoption_for(stack: &ClientStack, already_managed: bool) -> AdoptionPlan {
    todo!("plan_adoption_for")
}

/// Record an adoption plan into the manifest.
///
/// Inner form taking the write token and explicit paths, so it is testable
/// without Tauri. Holds the manifest lock for the whole read-modify-write, for
/// the same reason `apply_placements` does.
///
/// `keep_copies` copies the `copyable` entries into `backup_root` under one
/// shared folder id, and records that id in `displaced_backup` so the copy is
/// referenced and therefore never pruned.
pub fn adopt_in(
    manifest_path: &Path,
    backup_root: &Path,
    root: &ApprovedRoot,
    plan: &AdoptionPlan,
    keep_copies: bool,
) -> Result<AdoptionOutcome, String> {
    todo!("adopt_in")
}

/// Drop every adopted record for this install. Touches no file in the client
/// directory, and leaves files Kalpa actually placed alone.
pub fn forget_in(manifest_path: &Path, client_root: &Path) -> Result<Vec<String>, String> {
    todo!("forget_in")
}

// ── Commands ─────────────────────────────────────────────────────────────

/// Read-only: what adopting this client directory would record.
#[tauri::command]
pub fn plan_adoption(app: tauri::AppHandle, client_dir: String) -> Result<AdoptionPlan, String> {
    todo!("plan_adoption")
}

/// Record the stack in this client directory as managed.
#[tauri::command]
pub async fn adopt_stack(
    app: tauri::AppHandle,
    state: tauri::State<'_, AllowedGameInstallPath>,
    client_dir: String,
    keep_copies: bool,
) -> Result<AdoptionOutcome, String> {
    todo!("adopt_stack")
}

/// Forget the adopted stack. Records only; no file is touched.
#[tauri::command]
pub fn forget_stack(app: tauri::AppHandle, client_dir: String) -> Result<Vec<String>, String> {
    todo!("forget_stack")
}
