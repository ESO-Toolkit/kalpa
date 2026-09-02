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
    self, AllowedGameInstallPath, ApprovedRoot, FileOrigin, ManagedFile, ManagedKind,
    ManagedManifest,
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
    let manifest = client_backup::load_manifest_at(manifest_path);
    let key = install_key(client_root);

    let mut files: Vec<ManagedFileStatus> = manifest
        .installs
        .get(&key)
        .into_iter()
        .flatten()
        .map(|entry| status_for(client_root, entry))
        .collect();
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    let orphan_injectors = scan_orphan_injectors(client_root, &manifest);

    ManagedInventory {
        client_dir: client_dir_label.to_string(),
        files,
        orphan_injectors,
    }
}

/// Manifest key for one client directory.
///
/// Mirrors `client_backup::install_key`, which is private to that module, and
/// must stay in step with it: canonicalize where possible, fall back to the
/// path as supplied when the directory has gone away.
fn install_key(client_root: &Path) -> String {
    dunce::canonicalize(client_root)
        .unwrap_or_else(|_| client_root.to_path_buf())
        .to_string_lossy()
        .to_string()
}

/// Classify one manifest entry against what is on disk.
fn status_for(client_root: &Path, entry: &ManagedFile) -> ManagedFileStatus {
    let state = match client_write::safe_relative_join(client_root, &entry.relative_path) {
        Err(_) => ManagedFileState::Modified,
        Ok(resolved) => {
            if !resolved.is_file() {
                ManagedFileState::Missing
            } else {
                match client_backup::hash_file(&resolved) {
                    Ok(hash) if hash == entry.sha256 => ManagedFileState::Present,
                    _ => ManagedFileState::Modified,
                }
            }
        }
    };

    ManagedFileStatus {
        relative_path: entry.relative_path.clone(),
        kind: entry.kind,
        placed_at: entry.placed_at.clone(),
        state,
        restores_backup: entry.displaced_backup.is_some(),
    }
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
    let key = install_key(client_root);
    let known: std::collections::HashSet<String> = manifest
        .installs
        .get(&key)
        .into_iter()
        .flatten()
        .map(|entry| entry.relative_path.replace('\\', "/").to_ascii_lowercase())
        .collect();

    let mut out = Vec::new();
    for name in INJECTOR_NAMES {
        if known.contains(&name.to_ascii_lowercase()) {
            continue;
        }
        let path = client_root.join(name);
        if !path.is_file() {
            continue;
        }
        let Some(product_name) = crate::client_health::version_string(&path, "ProductName") else {
            continue;
        };
        if !product_name.to_lowercase().contains(RESHADE_PRODUCT_MARKER) {
            continue;
        }
        let version = crate::client_health::file_version(&path);
        out.push(OrphanInjector {
            file_name: name.to_string(),
            product_name,
            version,
        });
    }
    out
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
    let skipped =
        client_backup::revert_placements_in(manifest_path, backup_root, root, relative_paths)?;
    let removed: Vec<String> = relative_paths
        .iter()
        .filter(|path| !skipped.contains(path))
        .cloned()
        .collect();
    Ok(UninstallOutcome { removed, skipped })
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
    if file_name.contains(['/', '\\']) || file_name.contains("..") {
        return Err(format!(
            "{file_name} is not a valid injector file name: it must be a bare file name."
        ));
    }
    let lower = file_name.to_ascii_lowercase();
    if !INJECTOR_NAMES.contains(&lower.as_str()) {
        return Err(format!(
            "{file_name} is not one of the files emergency removal can act on ({}).",
            INJECTOR_NAMES.join(", ")
        ));
    }

    if !confirmation.trim().eq_ignore_ascii_case(&lower) {
        return Err(
            "The typed confirmation did not match the file name. Type the file name exactly \
             to confirm removal."
                .to_string(),
        );
    }

    let key = install_key(client_root);
    let already_managed = manifest
        .installs
        .get(&key)
        .into_iter()
        .flatten()
        .any(|entry| entry.relative_path.replace('\\', "/").to_ascii_lowercase() == lower);
    if already_managed {
        return Err(format!(
            "{file_name} is already managed by Kalpa. Use the normal uninstall instead — it \
             restores whatever this file displaced."
        ));
    }

    let orphans = scan_orphan_injectors(client_root, manifest);
    orphans
        .into_iter()
        .find(|orphan| orphan.file_name == lower)
        .ok_or_else(|| {
            if client_root.join(&lower).is_file() {
                format!(
                    "{file_name} is present, but Kalpa could not positively confirm it is \
                     ReShade. Refusing to remove it."
                )
            } else {
                format!("{file_name} was not found in the client folder.")
            }
        })
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
    let source = client_write::safe_relative_join(client_root, file_name)?;

    let folder = quarantine_root.join(client_backup::new_backup_id());
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("Failed to create quarantine folder: {e}"))?;
    // Not `folder.join(file_name)`. On Windows a segment beginning `C:` is
    // drive-relative: join discards the base and resolves against the current
    // directory on that drive, so a bad name would write the quarantine copy
    // somewhere else entirely and then delete the original. The source join
    // above would already have refused such a name, which is exactly why this
    // one must not be the weaker of the two.
    let destination = client_write::safe_relative_join(&folder, file_name)?;

    std::fs::copy(&source, &destination)
        .map_err(|e| format!("Failed to copy {file_name} to quarantine: {e}"))?;

    let source_hash = client_backup::hash_file(&source)?;
    let destination_hash = client_backup::hash_file(&destination)?;
    if source_hash != destination_hash {
        let _ = std::fs::remove_file(&destination);
        return Err(format!(
            "Quarantine copy of {file_name} did not match the original; the original was left \
             in place and nothing was removed."
        ));
    }

    std::fs::remove_file(&source).map_err(|e| {
        format!(
            "{file_name} was safely copied to quarantine, but the original in the client \
             folder could not be removed: {e}"
        )
    })?;

    Ok(destination)
}

// ── Commands ─────────────────────────────────────────────────────────────

/// List what Kalpa has placed in a client directory, and any orphan injector
/// it can positively identify. Read-only; needs no write approval.
#[tauri::command]
pub fn list_managed_client_files(
    app: tauri::AppHandle,
    client_dir: String,
) -> Result<ManagedInventory, String> {
    let manifest_path = client_backup::manifest_path(&app)?;
    let location = crate::client_install::validate_client_dir(Path::new(&client_dir))?;
    Ok(inventory_in(
        &manifest_path,
        &location.client_dir,
        &client_dir,
    ))
}

/// Remove managed files from a client directory.
#[tauri::command]
pub async fn uninstall_managed_client_files(
    app: tauri::AppHandle,
    state: tauri::State<'_, AllowedGameInstallPath>,
    client_dir: String,
    relative_paths: Vec<String>,
) -> Result<UninstallOutcome, String> {
    let root = client_write::begin_write(&state, &client_dir).await?;
    let manifest_path = client_backup::manifest_path(&app)?;
    let backup_root = client_backup::backup_root(&app)?;

    tokio::task::spawn_blocking(move || {
        uninstall_in(&manifest_path, &backup_root, &root, &relative_paths)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
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
    let root = client_write::begin_write(&state, &client_dir).await?;
    let manifest_path = client_backup::manifest_path(&app)?;
    let quarantine_root = client_backup::quarantine_root(&app)?;

    tokio::task::spawn_blocking(move || {
        let manifest = client_backup::load_manifest_at(&manifest_path);
        let orphan = vet_emergency_removal(root.path(), &manifest, &file_name, &confirmation)?;
        // The vetting above hashes files and can take real time; re-assert
        // idleness immediately before the move so a client that started
        // mid-vet is still caught.
        root.reassert_idle()?;
        let destination = quarantine_file(root.path(), &quarantine_root, &orphan.file_name)?;
        Ok(EmergencyRemoval {
            file_name: orphan.file_name,
            quarantine_path: destination.to_string_lossy().to_string(),
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Harness {
        _temp: tempfile::TempDir,
        manifest: PathBuf,
        backups: PathBuf,
        quarantine: PathBuf,
        client: PathBuf,
    }

    impl Harness {
        fn new() -> Self {
            let temp = tempfile::tempdir().expect("tempdir");
            let manifest = temp.path().join("appdata").join("client-managed.json");
            let backups = temp.path().join("appdata").join("client-backups");
            let quarantine = temp.path().join("appdata").join("client-quarantine");
            let client = temp.path().join("client");
            std::fs::create_dir_all(manifest.parent().unwrap()).expect("mkdir appdata");
            std::fs::create_dir_all(&backups).expect("mkdir backups");
            std::fs::create_dir_all(&quarantine).expect("mkdir quarantine");
            std::fs::create_dir_all(&client).expect("mkdir client");
            Self {
                _temp: temp,
                manifest,
                backups,
                quarantine,
                client,
            }
        }

        fn root(&self) -> ApprovedRoot {
            ApprovedRoot::for_tests_idle(self.client.clone())
        }

        fn key(&self) -> String {
            install_key(&self.client)
        }

        fn write_client_file(&self, relative: &str, contents: &str) -> PathBuf {
            let path = self.client.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("mkdir client subdir");
            }
            std::fs::write(&path, contents).expect("write client file");
            path
        }

        fn hash_of(&self, contents: &str) -> String {
            let tmp = tempfile::NamedTempFile::new().expect("named temp file");
            std::fs::write(tmp.path(), contents).expect("write temp");
            client_backup::hash_file(tmp.path()).expect("hash")
        }

        fn managed_file(
            &self,
            relative: &str,
            contents: &str,
            displaced_backup: Option<String>,
        ) -> ManagedFile {
            ManagedFile {
                relative_path: relative.to_string(),
                kind: ManagedKind::ReShadeCore,
                sha256: self.hash_of(contents),
                placed_at: "2026-01-01T00:00:00Z".to_string(),
                displaced_backup,
                origin: FileOrigin::Placed,
                displaced_in_place: None,
            }
        }

        fn save_manifest(&self, files: Vec<ManagedFile>) {
            let mut installs = BTreeMap::new();
            installs.insert(self.key(), files);
            let manifest = ManagedManifest { installs };
            let bytes = serde_json::to_vec_pretty(&manifest).expect("serialize manifest");
            std::fs::write(&self.manifest, bytes).expect("write manifest");
        }

        /// Places a file both on disk and in the backup folder that stands in
        /// for what it displaced, returning the backup id used.
        fn write_backup(&self, id: &str, relative: &str, contents: &str) {
            let path = self.backups.join(id).join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("mkdir backup subdir");
            }
            std::fs::write(&path, contents).expect("write backup file");
        }
    }

    // ── inventory_in / status_for ───────────────────────────────────────

    #[test]
    fn inventory_classifies_present_modified_and_missing() {
        let h = Harness::new();
        h.write_client_file("dxgi.dll", "present-bytes");
        h.write_client_file("d3d11.dll", "changed-bytes");
        // "ReShade.ini" is recorded but never written to disk.

        h.save_manifest(vec![
            h.managed_file("dxgi.dll", "present-bytes", None),
            h.managed_file("d3d11.dll", "original-bytes", None),
            h.managed_file("ReShade.ini", "[GENERAL]", None),
        ]);

        let inventory = inventory_in(&h.manifest, &h.client, "the client dir");
        assert_eq!(inventory.client_dir, "the client dir");
        assert_eq!(inventory.files.len(), 3);

        let by_path = |name: &str| {
            inventory
                .files
                .iter()
                .find(|f| f.relative_path == name)
                .unwrap_or_else(|| panic!("missing entry for {name}"))
        };
        assert_eq!(by_path("dxgi.dll").state, ManagedFileState::Present);
        assert_eq!(by_path("d3d11.dll").state, ManagedFileState::Modified);
        assert_eq!(by_path("ReShade.ini").state, ManagedFileState::Missing);
    }

    #[test]
    fn inventory_reflects_restores_backup_flag() {
        let h = Harness::new();
        h.write_client_file("dxgi.dll", "bytes");
        h.save_manifest(vec![h.managed_file(
            "dxgi.dll",
            "bytes",
            Some("some-backup-id".to_string()),
        )]);

        let inventory = inventory_in(&h.manifest, &h.client, "client");
        assert!(inventory.files[0].restores_backup);
    }

    #[test]
    fn inventory_without_backup_does_not_restore() {
        let h = Harness::new();
        h.write_client_file("dxgi.dll", "bytes");
        h.save_manifest(vec![h.managed_file("dxgi.dll", "bytes", None)]);

        let inventory = inventory_in(&h.manifest, &h.client, "client");
        assert!(!inventory.files[0].restores_backup);
    }

    #[test]
    fn inventory_is_sorted_by_relative_path() {
        let h = Harness::new();
        h.write_client_file("zeta.ini", "z");
        h.write_client_file("alpha.ini", "a");
        h.write_client_file("mid.ini", "m");
        h.save_manifest(vec![
            h.managed_file("zeta.ini", "z", None),
            h.managed_file("alpha.ini", "a", None),
            h.managed_file("mid.ini", "m", None),
        ]);

        let inventory = inventory_in(&h.manifest, &h.client, "client");
        let paths: Vec<&str> = inventory
            .files
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect();
        assert_eq!(paths, vec!["alpha.ini", "mid.ini", "zeta.ini"]);
    }

    #[test]
    fn inventory_is_empty_for_a_missing_manifest() {
        let h = Harness::new();
        // Never wrote h.manifest at all.
        let inventory = inventory_in(&h.manifest, &h.client, "client");
        assert!(inventory.files.is_empty());
        assert!(inventory.orphan_injectors.is_empty());
    }

    #[test]
    fn status_for_treats_an_unresolvable_path_as_modified() {
        let h = Harness::new();
        let entry = h.managed_file("../escape.dll", "bytes", None);
        let status = status_for(&h.client, &entry);
        assert_eq!(status.state, ManagedFileState::Modified);
    }

    // ── uninstall_in ─────────────────────────────────────────────────────

    #[test]
    fn uninstall_removes_a_present_file_and_restores_its_backup() {
        let h = Harness::new();
        h.write_client_file("dxgi.dll", "placed-bytes");
        h.write_backup("backup-1", "dxgi.dll", "original-bytes");
        h.save_manifest(vec![h.managed_file(
            "dxgi.dll",
            "placed-bytes",
            Some("backup-1".to_string()),
        )]);

        let outcome = uninstall_in(
            &h.manifest,
            &h.backups,
            &h.root(),
            &["dxgi.dll".to_string()],
        )
        .expect("uninstall should succeed");

        assert_eq!(outcome.removed, vec!["dxgi.dll".to_string()]);
        assert!(outcome.skipped.is_empty());
        assert_eq!(
            std::fs::read_to_string(h.client.join("dxgi.dll")).expect("read restored"),
            "original-bytes"
        );
    }

    #[test]
    fn uninstall_skips_a_modified_file_and_leaves_it_on_disk() {
        let h = Harness::new();
        h.write_client_file("dxgi.dll", "someone-edited-this");
        h.save_manifest(vec![h.managed_file("dxgi.dll", "placed-bytes", None)]);

        let outcome = uninstall_in(
            &h.manifest,
            &h.backups,
            &h.root(),
            &["dxgi.dll".to_string()],
        )
        .expect("uninstall should not error on a modified file");

        assert!(outcome.removed.is_empty());
        assert_eq!(outcome.skipped, vec!["dxgi.dll".to_string()]);
        assert_eq!(
            std::fs::read_to_string(h.client.join("dxgi.dll")).expect("read untouched"),
            "someone-edited-this"
        );
    }

    #[test]
    fn uninstall_partitions_a_mixed_request() {
        let h = Harness::new();
        h.write_client_file("dxgi.dll", "placed-bytes");
        h.write_client_file("d3d11.dll", "tampered-bytes");
        h.save_manifest(vec![
            h.managed_file("dxgi.dll", "placed-bytes", None),
            h.managed_file("d3d11.dll", "original-placed-bytes", None),
        ]);

        let outcome = uninstall_in(
            &h.manifest,
            &h.backups,
            &h.root(),
            &["dxgi.dll".to_string(), "d3d11.dll".to_string()],
        )
        .expect("uninstall should succeed");

        assert_eq!(outcome.removed, vec!["dxgi.dll".to_string()]);
        assert_eq!(outcome.skipped, vec!["d3d11.dll".to_string()]);
        assert!(!h.client.join("dxgi.dll").exists());
        assert!(h.client.join("d3d11.dll").exists());
    }

    // ── vet_emergency_removal ────────────────────────────────────────────

    #[test]
    fn vet_rejects_a_file_name_with_a_path_separator() {
        let h = Harness::new();
        let manifest = ManagedManifest::default();
        let err = vet_emergency_removal(&h.client, &manifest, "sub/dxgi.dll", "sub/dxgi.dll")
            .expect_err("should refuse a path separator");
        assert!(err.contains("valid injector file name"), "{err}");
    }

    #[test]
    fn vet_rejects_a_file_name_with_parent_traversal() {
        let h = Harness::new();
        let manifest = ManagedManifest::default();
        let err = vet_emergency_removal(&h.client, &manifest, "../dxgi.dll", "../dxgi.dll")
            .expect_err("should refuse parent traversal");
        assert!(err.contains("valid injector file name"), "{err}");
    }

    #[test]
    fn vet_rejects_a_file_name_outside_the_injector_list() {
        let h = Harness::new();
        let manifest = ManagedManifest::default();
        let err = vet_emergency_removal(&h.client, &manifest, "notes.txt", "notes.txt")
            .expect_err("should refuse a non-injector name");
        assert!(err.contains("emergency removal can act on"), "{err}");
    }

    #[test]
    fn vet_rejects_a_confirmation_that_does_not_match() {
        let h = Harness::new();
        let manifest = ManagedManifest::default();
        let err = vet_emergency_removal(&h.client, &manifest, "dxgi.dll", "not-the-name")
            .expect_err("should refuse a mismatched confirmation");
        assert!(err.contains("did not match"), "{err}");
    }

    #[test]
    fn vet_accepts_a_trimmed_case_insensitive_confirmation() {
        // This still refuses overall (nothing on disk to identify), but it must
        // fail for the "not found" reason, not the confirmation-mismatch reason.
        let h = Harness::new();
        let manifest = ManagedManifest::default();
        let err = vet_emergency_removal(&h.client, &manifest, "dxgi.dll", "  DXGI.DLL  ")
            .expect_err("still refused: nothing on disk");
        assert!(err.contains("was not found"), "{err}");
    }

    #[test]
    fn vet_rejects_a_file_already_recorded_in_the_manifest() {
        let h = Harness::new();
        h.write_client_file("dxgi.dll", "bytes");
        let mut installs = BTreeMap::new();
        installs.insert(h.key(), vec![h.managed_file("dxgi.dll", "bytes", None)]);
        let manifest = ManagedManifest { installs };

        let err = vet_emergency_removal(&h.client, &manifest, "dxgi.dll", "dxgi.dll")
            .expect_err("should refuse a managed file");
        assert!(err.contains("already managed"), "{err}");
    }

    #[test]
    fn vet_distinguishes_missing_from_unconfirmed() {
        let h = Harness::new();
        let manifest = ManagedManifest::default();

        // Nothing on disk at all.
        let err = vet_emergency_removal(&h.client, &manifest, "dxgi.dll", "dxgi.dll")
            .expect_err("should refuse: file not present");
        assert!(err.contains("was not found"), "{err}");

        // Present, but a plain file has no version resource on any platform,
        // so it can never be positively identified as ReShade.
        h.write_client_file("d3d11.dll", "not-actually-reshade");
        let err = vet_emergency_removal(&h.client, &manifest, "d3d11.dll", "d3d11.dll")
            .expect_err("should refuse: cannot confirm ReShade");
        assert!(err.contains("could not positively confirm"), "{err}");
    }

    // ── scan_orphan_injectors ────────────────────────────────────────────

    #[test]
    fn scan_never_reports_a_file_with_no_version_resource() {
        // Negative assertion that holds on every platform: version_string
        // returns None for a plain file even on Windows, since it carries no
        // PE version resource at all.
        let h = Harness::new();
        h.write_client_file("dxgi.dll", "not-a-real-dll");
        let manifest = ManagedManifest::default();
        let orphans = scan_orphan_injectors(&h.client, &manifest);
        assert!(orphans.is_empty());
    }

    #[test]
    fn scan_ignores_injectors_already_recorded_in_the_manifest() {
        let h = Harness::new();
        h.write_client_file("dxgi.dll", "bytes");
        let mut installs = BTreeMap::new();
        installs.insert(h.key(), vec![h.managed_file("dxgi.dll", "bytes", None)]);
        let manifest = ManagedManifest { installs };

        let orphans = scan_orphan_injectors(&h.client, &manifest);
        assert!(orphans.is_empty());
    }

    // ── quarantine_file ──────────────────────────────────────────────────

    #[test]
    fn quarantine_moves_the_file_with_identical_bytes() {
        let h = Harness::new();
        h.write_client_file("dxgi.dll", "quarantine-me");

        let destination = quarantine_file(&h.client, &h.quarantine, "dxgi.dll")
            .expect("quarantine should succeed");

        assert!(
            destination.starts_with(&h.quarantine),
            "destination should live under the quarantine root"
        );
        assert_eq!(
            std::fs::read_to_string(&destination).expect("read quarantined file"),
            "quarantine-me"
        );
        assert!(
            !h.client.join("dxgi.dll").exists(),
            "the original must be gone from the client folder"
        );
    }
}
