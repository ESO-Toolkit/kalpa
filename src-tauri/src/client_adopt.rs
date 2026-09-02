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
    match role {
        StackRole::Injector => ManagedKind::ReShadeCore,
        StackRole::NeuralRendering | StackRole::SuperSampling | StackRole::FrameGeneration => {
            ManagedKind::NvidiaRuntime
        }
        StackRole::ShaderCompiler => ManagedKind::ShaderCompiler,
        StackRole::Addon => ManagedKind::Addon,
        // A companion is whatever it looks like: `dlss5-feed.cfg` is
        // configuration; the host executable is not something any
        // `ManagedKind` can legitimately place, so it takes the closest
        // honest label rather than inventing a new kind for one file.
        StackRole::Companion => {
            if file_name.to_ascii_lowercase().ends_with(".cfg") {
                ManagedKind::ReShadeConfig
            } else {
                ManagedKind::Addon
            }
        }
    }
}

/// Which stack items are worth keeping a copy of.
///
/// Only the runtimes, and only because the ZOS patcher rewrites them on a game
/// update. Copying the 165 MB Neural Rendering runtime is defensible for that
/// reason; copying the shader tree is not, since nothing overwrites it.
pub fn is_copyable(item: &StackItem) -> bool {
    matches!(
        item.role,
        StackRole::NeuralRendering | StackRole::SuperSampling | StackRole::FrameGeneration
    )
}

/// The user's own backup of `file_name`, if one sits next to it.
///
/// `client_stack` already recognised these; this only pairs them back up with
/// the live file they shadow.
fn in_place_original(stack: &ClientStack, file_name: &str) -> Option<String> {
    stack
        .preserved_originals
        .iter()
        .find(|original| {
            original
                .backs_up
                .as_deref()
                .is_some_and(|target| target.eq_ignore_ascii_case(file_name))
        })
        .map(|original| original.file_name.clone())
}

/// Compute the adoption plan for a client directory. Read-only.
pub fn plan_adoption_for(stack: &ClientStack, already_managed: bool) -> AdoptionPlan {
    let entries: Vec<AdoptionEntry> = stack
        .items
        .iter()
        .map(|item| AdoptionEntry {
            relative_path: item.file_name.clone(),
            kind: kind_for_role(item.role, &item.file_name),
            role: item.role,
            display_name: item.display_name.clone(),
            version: item.version.clone(),
            size_bytes: item.size_bytes,
            displaced_in_place: in_place_original(stack, &item.file_name),
            copyable: is_copyable(item),
        })
        .collect();

    let copy_bytes = entries
        .iter()
        .filter(|entry| entry.copyable)
        .map(|entry| entry.size_bytes)
        .sum();

    AdoptionPlan {
        client_dir: stack.client_dir.clone(),
        is_empty: entries.is_empty(),
        entries,
        copy_bytes,
        already_managed,
    }
}

/// Record an adoption plan into the manifest.
///
/// Inner form taking the write token and explicit paths, so it is testable
/// without Tauri.
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
    // Adoption writes nothing into the client directory, but it does read
    // every file to hash it, and copying a runtime the launcher's patcher is
    // part-way through rewriting would preserve torn bytes as if they were
    // the user's good copy. The gate costs nothing and rules that out.
    root.reassert_idle()?;
    let client_root = root.path();

    let copy_id = crate::client_backup::new_backup_id();
    let mut outcome = AdoptionOutcome {
        recorded: Vec::new(),
        copied: Vec::new(),
        skipped: Vec::new(),
    };
    let mut entries: Vec<ManagedFile> = Vec::new();

    for entry in &plan.entries {
        let resolved =
            match crate::client_write::safe_relative_join(client_root, &entry.relative_path) {
                Ok(path) => path,
                Err(error) => {
                    outcome
                        .skipped
                        .push(format!("{}: {error}", entry.relative_path));
                    continue;
                }
            };
        if !resolved.is_file() {
            outcome.skipped.push(format!(
                "{}: no longer in the client folder",
                entry.relative_path
            ));
            continue;
        }
        // Best-effort per file. One unreadable DLL must not abandon the rest
        // of a stack the user asked to have managed.
        let sha256 = match crate::client_backup::hash_file(&resolved) {
            Ok(hash) => hash,
            Err(error) => {
                outcome
                    .skipped
                    .push(format!("{}: {error}", entry.relative_path));
                continue;
            }
        };

        let mut displaced_backup = None;
        if keep_copies && entry.copyable {
            match copy_into_backup(backup_root, &copy_id, &entry.relative_path, &resolved) {
                Ok(()) => {
                    displaced_backup = Some(copy_id.clone());
                    outcome.copied.push(entry.relative_path.clone());
                }
                Err(error) => {
                    // A failed copy is not a failed adoption. Record the entry
                    // anyway so drift is still detected; only the one-click
                    // re-apply is unavailable, and the user is told which file
                    // lost it rather than discovering it after a game update.
                    outcome
                        .skipped
                        .push(format!("{}: copy failed, {error}", entry.relative_path));
                }
            }
        }

        entries.push(ManagedFile {
            relative_path: entry.relative_path.clone(),
            kind: entry.kind,
            sha256,
            placed_at: crate::client_backup::rfc3339_now(),
            displaced_backup,
            origin: FileOrigin::Adopted,
            displaced_in_place: entry.displaced_in_place.clone(),
        });
        outcome.recorded.push(entry.relative_path.clone());
    }

    if entries.is_empty() {
        return Err(
            "Nothing could be recorded from this folder. Refresh and try again.".to_string(),
        );
    }
    crate::client_backup::record_adopted(manifest_path, client_root, entries)?;
    Ok(outcome)
}

/// Copy one adopted file into the backup root, verifying the copy before
/// reporting success.
///
/// An unverified copy is worse than none: it would make "Kalpa can put this
/// back after a game update" a claim with nothing behind it, discovered only
/// at the moment it was needed.
fn copy_into_backup(
    backup_root: &Path,
    id: &str,
    relative_path: &str,
    source: &Path,
) -> Result<(), String> {
    let destination = crate::client_backup::backup_file_path(backup_root, id, relative_path)?;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create the copy folder: {e}"))?;
    }
    std::fs::copy(source, &destination).map_err(|e| format!("{e}"))?;

    let source_hash = crate::client_backup::hash_file(source)?;
    let copy_hash = crate::client_backup::hash_file(&destination)?;
    if source_hash != copy_hash {
        let _ = std::fs::remove_file(&destination);
        return Err("the copy did not match the original".to_string());
    }
    Ok(())
}

/// Drop every adopted record for this install. Touches no file in the client
/// directory, and leaves files Kalpa actually placed alone.
pub fn forget_in(manifest_path: &Path, client_root: &Path) -> Result<Vec<String>, String> {
    crate::client_backup::forget_adopted(manifest_path, client_root)
}

/// True when this install already has adopted entries recorded.
fn is_already_managed(manifest_path: &Path, client_root: &Path) -> bool {
    let manifest = crate::client_backup::load_manifest_at(manifest_path);
    let key = dunce::canonicalize(client_root)
        .unwrap_or_else(|_| client_root.to_path_buf())
        .to_string_lossy()
        .to_string();
    manifest
        .installs
        .get(&key)
        .is_some_and(|bucket| bucket.iter().any(|file| file.origin == FileOrigin::Adopted))
}

// ── Commands ─────────────────────────────────────────────────────────────

/// Read-only: what adopting this client directory would record.
#[tauri::command]
pub fn plan_adoption(app: tauri::AppHandle, client_dir: String) -> Result<AdoptionPlan, String> {
    let location = crate::client_install::validate_client_dir(Path::new(&client_dir))?;
    let manifest_path = crate::client_backup::manifest_path(&app)?;
    let stack = crate::client_stack::inspect_stack(&location.client_dir);
    let already_managed = is_already_managed(&manifest_path, &location.client_dir);
    Ok(plan_adoption_for(&stack, already_managed))
}

/// Record the stack in this client directory as managed.
///
/// The plan is recomputed here from the directory rather than accepted from
/// the caller. A plan the frontend built minutes ago describes a folder that
/// may have changed since, and adoption must record what is actually there.
#[tauri::command]
pub async fn adopt_stack(
    app: tauri::AppHandle,
    state: tauri::State<'_, AllowedGameInstallPath>,
    client_dir: String,
    keep_copies: bool,
) -> Result<AdoptionOutcome, String> {
    let root = crate::client_write::begin_write(&state, &client_dir).await?;
    let manifest_path = crate::client_backup::manifest_path(&app)?;
    let backup_root = crate::client_backup::backup_root(&app)?;

    tokio::task::spawn_blocking(move || {
        let stack = crate::client_stack::inspect_stack(root.path());
        let already = is_already_managed(&manifest_path, root.path());
        let plan = plan_adoption_for(&stack, already);
        adopt_in(&manifest_path, &backup_root, &root, &plan, keep_copies)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Forget the adopted stack. Records only; no file is touched.
#[tauri::command]
pub fn forget_stack(app: tauri::AppHandle, client_dir: String) -> Result<Vec<String>, String> {
    let location = crate::client_install::validate_client_dir(Path::new(&client_dir))?;
    let manifest_path = crate::client_backup::manifest_path(&app)?;
    forget_in(&manifest_path, &location.client_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_stack::inspect_stack;
    use std::path::PathBuf;

    struct Harness {
        _temp: tempfile::TempDir,
        manifest: PathBuf,
        backups: PathBuf,
        client: PathBuf,
    }

    impl Harness {
        /// A client folder shaped like the primary user's: a swapped DLSS with
        /// their own `.disabled-bak` original beside it, plus the rest of the
        /// stack.
        fn new() -> Self {
            let temp = tempfile::tempdir().expect("tempdir");
            let manifest = temp.path().join("appdata").join("client-managed.json");
            let backups = temp.path().join("appdata").join("client-backups");
            let client = temp.path().join("client");
            std::fs::create_dir_all(manifest.parent().unwrap()).expect("mkdir appdata");
            std::fs::create_dir_all(&backups).expect("mkdir backups");
            std::fs::create_dir_all(&client).expect("mkdir client");

            for (name, body) in [
                ("eso64.exe", "game"),
                ("dxgi.dll", "reshade"),
                ("nvngx_dlssnr.dll", "neural rendering runtime"),
                ("nvngx_dlss.dll", "modern dlss"),
                ("nvngx_dlss.dll.disabled-bak", "the original 2.2.16"),
                ("renodx-dlss5.addon64", "nr addon"),
                ("d3dcompiler_47.dll", "modern compiler"),
                ("d3dcompiler_47.dll.eso-orig-bak", "the original compiler"),
            ] {
                std::fs::write(client.join(name), body).expect("write fixture");
            }

            Self {
                _temp: temp,
                manifest,
                backups,
                client,
            }
        }

        fn plan(&self) -> AdoptionPlan {
            plan_adoption_for(&inspect_stack(&self.client), false)
        }

        fn root(&self) -> ApprovedRoot {
            ApprovedRoot::for_tests_idle(self.client.clone())
        }

        fn adopt(&self, keep_copies: bool) -> Result<AdoptionOutcome, String> {
            adopt_in(
                &self.manifest,
                &self.backups,
                &self.root(),
                &self.plan(),
                keep_copies,
            )
        }

        fn entries(&self) -> Vec<ManagedFile> {
            let manifest = crate::client_backup::load_manifest_at(&self.manifest);
            let key = dunce::canonicalize(&self.client)
                .unwrap_or_else(|_| self.client.clone())
                .to_string_lossy()
                .to_string();
            manifest.installs.get(&key).cloned().unwrap_or_default()
        }

        fn entry(&self, relative: &str) -> ManagedFile {
            self.entries()
                .into_iter()
                .find(|file| file.relative_path == relative)
                .unwrap_or_else(|| panic!("no entry for {relative}"))
        }
    }

    #[test]
    fn the_plan_pairs_each_file_with_the_users_own_backup() {
        let h = Harness::new();
        let plan = h.plan();

        let dlss = plan
            .entries
            .iter()
            .find(|e| e.relative_path == "nvngx_dlss.dll")
            .expect("dlss entry");
        assert_eq!(
            dlss.displaced_in_place.as_deref(),
            Some("nvngx_dlss.dll.disabled-bak"),
            "the user's own backup is the displaced original"
        );

        let compiler = plan
            .entries
            .iter()
            .find(|e| e.relative_path == "d3dcompiler_47.dll")
            .expect("compiler entry");
        assert_eq!(
            compiler.displaced_in_place.as_deref(),
            Some("d3dcompiler_47.dll.eso-orig-bak")
        );

        // A file nobody backed up has no original, and that is not an error.
        let injector = plan
            .entries
            .iter()
            .find(|e| e.relative_path == "dxgi.dll")
            .expect("injector entry");
        assert_eq!(injector.displaced_in_place, None);
    }

    /// Only the runtimes are worth the disk. The 165 MB figure in the module
    /// doc is why this is not "copy everything".
    #[test]
    fn only_runtimes_are_marked_copyable() {
        let h = Harness::new();
        let plan = h.plan();

        let copyable: Vec<&str> = plan
            .entries
            .iter()
            .filter(|e| e.copyable)
            .map(|e| e.relative_path.as_str())
            .collect();
        assert!(copyable.contains(&"nvngx_dlss.dll"), "{copyable:?}");
        assert!(copyable.contains(&"nvngx_dlssnr.dll"), "{copyable:?}");
        assert!(!copyable.contains(&"dxgi.dll"), "{copyable:?}");
        assert!(!copyable.contains(&"renodx-dlss5.addon64"), "{copyable:?}");

        let expected: u64 = plan
            .entries
            .iter()
            .filter(|e| e.copyable)
            .map(|e| e.size_bytes)
            .sum();
        assert_eq!(plan.copy_bytes, expected);
        assert!(plan.copy_bytes > 0);
    }

    /// The whole point of adoption: the client directory is not touched.
    #[test]
    fn adoption_changes_nothing_in_the_client_folder() {
        let h = Harness::new();
        let before = snapshot(&h.client);

        h.adopt(true).expect("adoption should succeed");

        assert_eq!(
            before,
            snapshot(&h.client),
            "adoption must not add, remove or alter a single file in the client folder"
        );
    }

    fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
            .expect("read client")
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| {
                (
                    e.file_name().to_string_lossy().to_string(),
                    std::fs::read(e.path()).unwrap_or_default(),
                )
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn adopted_entries_record_the_in_place_original_and_never_a_backup_folder_for_it() {
        let h = Harness::new();
        h.adopt(false).expect("adoption should succeed");

        let dlss = h.entry("nvngx_dlss.dll");
        assert_eq!(dlss.origin, FileOrigin::Adopted);
        assert_eq!(
            dlss.displaced_in_place.as_deref(),
            Some("nvngx_dlss.dll.disabled-bak")
        );
        assert_eq!(
            dlss.displaced_backup, None,
            "the user's own backup must never be recorded as a backup-root folder, \
             which prune_unreferenced_backups is entitled to delete"
        );
    }

    #[test]
    fn keeping_copies_stores_verified_bytes_under_a_referenced_id() {
        let h = Harness::new();
        let outcome = h.adopt(true).expect("adoption should succeed");

        assert!(outcome.copied.contains(&"nvngx_dlss.dll".to_string()));
        assert!(outcome.copied.contains(&"nvngx_dlssnr.dll".to_string()));

        let dlss = h.entry("nvngx_dlss.dll");
        let id = dlss
            .displaced_backup
            .as_deref()
            .expect("a kept copy must be referenced by its entry, or pruning may delete it");
        let copy = crate::client_backup::backup_file_path(&h.backups, id, "nvngx_dlss.dll")
            .expect("backup path");
        assert!(copy.is_file(), "the copy should exist at {copy:?}");
        assert_eq!(
            std::fs::read(&copy).unwrap(),
            std::fs::read(h.client.join("nvngx_dlss.dll")).unwrap(),
            "the kept copy must be byte-identical or it cannot be put back"
        );

        // The in-place original is still recorded alongside the kept copy.
        assert_eq!(
            dlss.displaced_in_place.as_deref(),
            Some("nvngx_dlss.dll.disabled-bak")
        );
    }

    #[test]
    fn declining_copies_records_everything_and_copies_nothing() {
        let h = Harness::new();
        let outcome = h.adopt(false).expect("adoption should succeed");

        assert!(outcome.copied.is_empty());
        assert!(!outcome.recorded.is_empty());
        assert!(h
            .entries()
            .iter()
            .all(|entry| entry.displaced_backup.is_none()));
    }

    /// Uninstall removes what Kalpa placed. It placed none of this.
    #[test]
    fn uninstall_refuses_to_delete_adopted_files() {
        let h = Harness::new();
        h.adopt(false).expect("adoption should succeed");

        let skipped = crate::client_backup::revert_placements_in(
            &h.manifest,
            &h.backups,
            &h.root(),
            &["nvngx_dlss.dll".to_string(), "dxgi.dll".to_string()],
        )
        .expect("revert should not error");

        assert!(skipped.contains(&"nvngx_dlss.dll".to_string()));
        assert!(skipped.contains(&"dxgi.dll".to_string()));
        assert!(
            h.client.join("nvngx_dlss.dll").is_file(),
            "an adopted file must survive an uninstall that names it"
        );
        assert!(h.client.join("dxgi.dll").is_file());
        assert!(
            h.client.join("nvngx_dlss.dll.disabled-bak").is_file(),
            "and so must the user's own original"
        );
        assert_eq!(
            h.entries().len(),
            h.plan().entries.len(),
            "a refused revert must not drop the records either"
        );
    }

    #[test]
    fn forgetting_drops_records_and_touches_no_file() {
        let h = Harness::new();
        h.adopt(true).expect("adoption should succeed");
        let before = snapshot(&h.client);

        let forgotten = forget_in(&h.manifest, &h.client).expect("forget should succeed");

        assert!(!forgotten.is_empty());
        assert!(
            h.entries().is_empty(),
            "every adopted record should be gone"
        );
        assert_eq!(
            before,
            snapshot(&h.client),
            "forgetting is Kalpa giving up its records, not undoing anything"
        );
    }

    #[test]
    fn forgetting_leaves_files_kalpa_actually_placed() {
        let h = Harness::new();
        h.adopt(false).expect("adoption should succeed");

        // A placed entry alongside the adopted ones.
        crate::client_backup::record_adopted(
            &h.manifest,
            &h.client,
            vec![ManagedFile {
                relative_path: "ReShade.ini".to_string(),
                kind: ManagedKind::ReShadeConfig,
                sha256: "b".repeat(64),
                placed_at: "2026-01-01T00:00:00Z".to_string(),
                displaced_backup: None,
                origin: FileOrigin::Placed,
                displaced_in_place: None,
            }],
        )
        .expect("record placed entry");

        forget_in(&h.manifest, &h.client).expect("forget should succeed");

        let remaining: Vec<String> = h
            .entries()
            .into_iter()
            .map(|entry| entry.relative_path)
            .collect();
        assert_eq!(
            remaining,
            vec!["ReShade.ini".to_string()],
            "a placed entry still describes a real write uninstall must reverse"
        );
    }

    #[test]
    fn adopting_twice_refreshes_rather_than_duplicates() {
        let h = Harness::new();
        h.adopt(false).expect("first adoption");
        let first = h.entries().len();

        std::fs::write(h.client.join("nvngx_dlss.dll"), "a newer swap").unwrap();
        h.adopt(false).expect("second adoption");

        assert_eq!(h.entries().len(), first, "no duplicate rows");
        let expected = crate::client_backup::hash_file(&h.client.join("nvngx_dlss.dll")).unwrap();
        assert_eq!(
            h.entry("nvngx_dlss.dll").sha256,
            expected,
            "re-adopting should refresh the hash to what is on disk now"
        );
    }

    #[test]
    fn already_managed_is_reported_after_adopting() {
        let h = Harness::new();
        assert!(!is_already_managed(&h.manifest, &h.client));
        h.adopt(false).expect("adoption");
        assert!(is_already_managed(&h.manifest, &h.client));
    }

    #[test]
    fn adoption_refuses_while_the_client_is_running() {
        let h = Harness::new();
        let active = ApprovedRoot::for_tests(h.client.clone(), std::sync::Arc::new(|| Ok(true)));

        let error = adopt_in(&h.manifest, &h.backups, &active, &h.plan(), true)
            .expect_err("adoption must refuse while the client is active");
        assert!(error.contains("running"), "{error}");
        assert!(h.entries().is_empty());
    }

    #[test]
    fn an_empty_folder_produces_an_empty_plan() {
        let temp = tempfile::tempdir().unwrap();
        let client = temp.path().join("client");
        std::fs::create_dir_all(&client).unwrap();
        std::fs::write(client.join("eso64.exe"), "game").unwrap();

        let plan = plan_adoption_for(&inspect_stack(&client), false);
        assert!(plan.is_empty);
        assert_eq!(plan.copy_bytes, 0);
    }

    #[test]
    fn roles_map_to_kinds_that_match_the_write_policy() {
        // A wrong kind here would let a later write put the wrong sort of file
        // in the wrong place, so pin the mapping against validate_placement.
        use crate::client_write::validate_placement;
        assert!(
            validate_placement(kind_for_role(StackRole::Injector, "dxgi.dll"), "dxgi.dll").is_ok()
        );
        assert!(validate_placement(
            kind_for_role(StackRole::SuperSampling, "nvngx_dlss.dll"),
            "nvngx_dlss.dll"
        )
        .is_ok());
        assert!(validate_placement(
            kind_for_role(StackRole::NeuralRendering, "nvngx_dlssnr.dll"),
            "nvngx_dlssnr.dll"
        )
        .is_ok());
        assert!(validate_placement(
            kind_for_role(StackRole::ShaderCompiler, "d3dcompiler_47.dll"),
            "d3dcompiler_47.dll"
        )
        .is_ok());
        assert!(validate_placement(
            kind_for_role(StackRole::Addon, "renodx-dlss5.addon64"),
            "renodx-dlss5.addon64"
        )
        .is_ok());
        assert!(validate_placement(
            kind_for_role(StackRole::Companion, "dlss5-feed.cfg"),
            "dlss5-feed.cfg"
        )
        .is_ok());
    }
}
