//! Uninstall and emergency removal for the ESO **client install** directory.
//!
//! This lands before any installer on purpose. It is the recovery path for
//! everything built after it, and its worst failure mode is the mild one: the
//! bad outcome here is "ReShade stops loading", not "the game will not start".
//!
//! # Two paths, and why the second one exists
//!
//! **Managed uninstall** is the normal path. Kalpa records every file it
//! places in a manifest with the SHA-256 of the bytes it wrote, so uninstall
//! can tell "Kalpa put this here and nobody has touched it" from "someone has
//! edited this since". It removes the first and refuses the second, restoring
//! whatever each file displaced. That refusal is the whole point of the hash:
//! a user who hand-tuned a preset should get their file back, not a shrug.
//!
//! **Emergency removal** exists because the manifest can be lost — a reinstall,
//! a wiped app-data folder, a machine migration, or simply a ReShade the user
//! placed by hand before Kalpa existed. Without it a foreign `dxgi.dll` is
//! effectively permanent: Steam's *Verify integrity* only restores files Steam
//! shipped, and the ZOS launcher's *Repair* only checks its own manifest, so
//! neither notices, let alone removes, a proxy DLL that was never theirs. The
//! user is left deleting a DLL out of their game folder by hand on the advice
//! of a forum post.
//!
//! So emergency removal is deliberately narrow rather than a general "delete
//! this file for me":
//!
//! 1. Only the two file names the game's DLL search order actually loads
//!    (`dxgi.dll`, `d3d11.dll`), only at the root of the client folder.
//! 2. Only files the manifest does *not* know about — anything managed goes
//!    through the hash-checked path instead, and this one refuses it.
//! 3. Only after the PE version resource positively says `ProductName` is
//!    ReShade. A file with no version resource, or an unreadable one, is
//!    refused: absence of evidence is not evidence, and the cost of being
//!    wrong is deleting a DLL some other software needs.
//! 4. Only with an explicit typed confirmation from the UI.
//! 5. The file is *moved into quarantine*, never deleted. Kalpa did not place
//!    it and cannot know what it was, so it does not get to destroy it.
//!
//! # Write safety
//!
//! Both write paths go through `client_write::begin_write` and therefore hold
//! an [`ApprovedRoot`](crate::client_write::ApprovedRoot): the approved-root,
//! containment, filename-policy, client-not-running and sandbox gates all
//! apply here exactly as they do to installation. Removing a proxy DLL out
//! from under a running client is as bad as installing one.
//!
//! Listing is read-only and needs no token.

use crate::client_backup;
use crate::client_write::{
    self, AllowedGameInstallPath, ApprovedRoot, ManagedFile, ManagedKind, ManagedManifest,
};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The only file names the game's DLL search order loads as a proxy, and so
/// the only ones emergency removal will consider.
pub const INJECTOR_NAMES: [&str; 2] = ["dxgi.dll", "d3d11.dll"];

/// Substring the PE `ProductName` must contain, compared case-insensitively,
/// before emergency removal will touch a file.
pub const RESHADE_PRODUCT_MARKER: &str = "reshade";

/// Whether a file Kalpa placed is still the file Kalpa placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedFileState {
    /// On disk and byte-identical to what Kalpa wrote. Safe to remove.
    Present,
    /// On disk but the hash differs — someone changed it. Uninstall will
    /// refuse to delete it and says so.
    Modified,
    /// Recorded in the manifest but no longer on disk. Uninstall drops the
    /// entry and restores any displaced original.
    Missing,
}

/// One managed file, as the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedFileStatus {
    /// Forward-slashed, relative to the client directory.
    pub relative_path: String,
    pub kind: ManagedKind,
    /// RFC3339, copied from the manifest entry.
    pub placed_at: String,
    pub state: ManagedFileState,
    /// True when removing this file would put a displaced original back rather
    /// than simply leaving a gap. Worth showing: it is the difference between
    /// "uninstall" and "revert to what you had".
    pub restores_backup: bool,
}

/// Everything Kalpa has placed in one client directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedInventory {
    /// Echoed back so a late response can be matched to the request that asked
    /// for it.
    pub client_dir: String,
    /// Sorted by `relative_path`, so the list does not reorder between polls.
    pub files: Vec<ManagedFileStatus>,
    /// Injector DLLs present in the folder that the manifest does not know
    /// about and whose version resource identifies them as ReShade. Empty in
    /// the ordinary case; non-empty is what unlocks emergency removal.
    pub orphan_injectors: Vec<OrphanInjector>,
}

/// The result of a managed uninstall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UninstallOutcome {
    /// Paths removed (and whose displaced originals were restored).
    pub removed: Vec<String>,
    /// Paths left alone, because their bytes no longer match the manifest or
    /// because the manifest never had them. Never an error — leaving a
    /// modified file in place is the correct outcome, not a failure.
    pub skipped: Vec<String>,
}

/// An injector DLL in the client folder that Kalpa did not place, positively
/// identified as ReShade by its PE version resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrphanInjector {
    /// Lower-case file name, always one of [`INJECTOR_NAMES`].
    pub file_name: String,
    /// `ProductName` exactly as read from the version resource, so the UI can
    /// show the user what Kalpa matched on rather than asking for trust.
    pub product_name: String,
    /// Four-part file version, when the resource carries one.
    pub version: Option<String>,
}

/// The result of an emergency removal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmergencyRemoval {
    pub file_name: String,
    /// Absolute path the file was moved to, shown to the user so the move is
    /// verifiable and reversible by hand.
    pub quarantine_path: String,
}

// ── Inventory (read-only) ────────────────────────────────────────────────

/// Build the inventory for one client directory.
///
/// Inner form, testable without an `AppHandle`. Reads only: it hashes files on
/// disk and compares them to the manifest, and never writes, moves or deletes
/// anything.
///
/// An unreadable file is reported as [`ManagedFileState::Modified`] rather than
/// propagating an error — "Kalpa cannot prove this is still its file" and
/// "someone changed it" lead to the same safe refusal, and one unreadable file
/// must not blank the whole list.
pub fn inventory_in(
    manifest_path: &Path,
    client_root: &Path,
    client_dir_label: &str,
) -> ManagedInventory {
    todo!("inventory_in")
}

/// Classify one manifest entry against what is on disk.
fn status_for(client_root: &Path, entry: &ManagedFile) -> ManagedFileStatus {
    todo!("status_for")
}

/// Injector DLLs present at the root of `client_root` that `manifest` does not
/// record and whose version resource identifies them as ReShade.
///
/// The version-resource read is [`crate::client_health::version_string`], which
/// returns `None` off Windows — so this returns an empty list on Linux and
/// macOS, which is correct: there is no ESO client-directory ReShade to find
/// there, and a positive identification is required before anything is offered.
pub fn scan_orphan_injectors(
    client_root: &Path,
    manifest: &ManagedManifest,
) -> Vec<OrphanInjector> {
    todo!("scan_orphan_injectors")
}

// ── Managed uninstall ────────────────────────────────────────────────────

/// Remove managed files, restoring whatever they displaced.
///
/// Inner form taking the write token, so tests can drive it without Tauri.
/// Delegates the actual work to [`client_backup::revert_placements`], which
/// re-asserts the client-not-running gate, holds the manifest lock, and skips
/// files whose hash no longer matches.
pub fn uninstall_in(
    manifest_path: &Path,
    backup_root: &Path,
    root: &ApprovedRoot,
    relative_paths: &[String],
) -> Result<UninstallOutcome, String> {
    todo!("uninstall_in")
}

// ── Emergency removal ────────────────────────────────────────────────────

/// Everything emergency removal checks before it will move a file, minus the
/// write gates (which `begin_write` has already applied by the time the token
/// exists).
///
/// Split out from the move itself so every refusal is unit-testable without a
/// filesystem move: this is the function that decides, and it decides on a
/// positive identification only.
pub fn vet_emergency_removal(
    client_root: &Path,
    manifest: &ManagedManifest,
    file_name: &str,
    confirmation: &str,
) -> Result<OrphanInjector, String> {
    todo!("vet_emergency_removal")
}

/// Move a vetted orphan injector into `quarantine_root`.
///
/// Copy-then-remove rather than a rename, because the quarantine directory is
/// in the app-data folder and the client folder is very often on another
/// volume. The copy is verified before the original is removed: a quarantine
/// that lost the file it was supposed to preserve is worse than no quarantine.
pub fn quarantine_file(
    client_root: &Path,
    quarantine_root: &Path,
    file_name: &str,
) -> Result<PathBuf, String> {
    todo!("quarantine_file")
}

// ── Commands ─────────────────────────────────────────────────────────────

/// List what Kalpa has placed in a client directory, and any orphan injector
/// it can positively identify. Read-only; needs no write approval.
#[tauri::command]
pub fn list_managed_client_files(
    app: tauri::AppHandle,
    client_dir: String,
) -> Result<ManagedInventory, String> {
    todo!("list_managed_client_files")
}

/// Remove managed files from a client directory.
#[tauri::command]
pub async fn uninstall_managed_client_files(
    app: tauri::AppHandle,
    state: tauri::State<'_, AllowedGameInstallPath>,
    client_dir: String,
    relative_paths: Vec<String>,
) -> Result<UninstallOutcome, String> {
    todo!("uninstall_managed_client_files")
}

/// Move an unmanaged ReShade injector into quarantine.
///
/// `confirmation` must be the file name typed back by the user; see
/// [`vet_emergency_removal`].
#[tauri::command]
pub async fn emergency_remove_injector(
    app: tauri::AppHandle,
    state: tauri::State<'_, AllowedGameInstallPath>,
    client_dir: String,
    file_name: String,
    confirmation: String,
) -> Result<EmergencyRemoval, String> {
    todo!("emergency_remove_injector")
}
