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

use crate::client_stack::StackRole;
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
    /// The bytes differ from the manifest, but **ESO does not ship this file**,
    /// so no game update can have put anything back over it. Somebody changed
    /// it deliberately, and the overwhelmingly likely somebody is the user
    /// installing a newer runtime.
    ///
    /// This is a separate state from the drifted ones because the action is
    /// different — there is nothing to "put back", and offering to overwrite it
    /// with Kalpa's older kept copy would destroy a newer runtime that has no
    /// redistributable source and cannot be fetched again.
    ChangedNotByUpdate,
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
/// ship them, so no patch restores anything over them. This must agree with
/// `client_adopt::is_copyable`: that function decides which files are worth
/// keeping a copy of for exactly this reason, and a role this module treats as
/// drift-prone but that adoption never offers to copy could never become
/// `DriftedRecoverable`.
pub fn is_drift_prone(role: StackRole) -> bool {
    matches!(
        role,
        StackRole::NeuralRendering
            | StackRole::SuperSampling
            | StackRole::FrameGeneration
            | StackRole::ShaderCompiler
    )
}

/// Does **ESO itself ship a file in this role**, such that the launcher's
/// patcher can put its own build back over the user's swap?
///
/// This is not the same question as [`is_drift_prone`], and conflating the two
/// is how the re-apply action came to offer to overwrite a user's newer Neural
/// Rendering runtime with Kalpa's older copy. Two distinct predicates:
///
/// * `client_adopt::is_copyable` — *worth keeping a copy of*. True for the NR
///   runtime: 165 MB, user-supplied, no redistributable source, so losing it
///   costs the user a hunt. Keeping a copy is right.
/// * `eso_ships` — *a game update can revert this*. **False** for the NR
///   runtime. ESO has never shipped `nvngx_dlssnr.dll`, so nothing the patcher
///   does can put "ESO's own build" back over it, and a hash change there can
///   only be a deliberate replacement — almost always the user installing a
///   newer runtime.
///
/// Treating a change to a file ESO does not ship as an update to be undone
/// means offering to overwrite the newer file with the older one, and filing the
/// newer one somewhere `prune_unreferenced_backups` can reach. It is exactly the
/// wrong way round.
///
/// `nvngx_dlssg.dll` ([`StackRole::FrameGeneration`]) is deliberately absent,
/// and the call is a judgement rather than a certainty: it is not in the primary
/// user's install, and ESO is not known to ship or request frame generation. The
/// two ways of being wrong are not symmetric. Treating it as shipped when it is
/// not **blocks disable outright** for anyone holding one, with no way past;
/// treating it as not shipped when it is means disable leaves that one file
/// modded instead of stock, which is a smaller and visible miss. If a stock
/// install is ever confirmed to contain it, move it here.
pub fn eso_ships(role: StackRole) -> bool {
    matches!(role, StackRole::SuperSampling | StackRole::ShaderCompiler)
}

/// Resolve the [`StackRole`] a managed relative path corresponds to, the same
/// way [`crate::client_stack::inspect_stack`] assigns roles to these exact
/// file names. Returns `None` for anything that is not one of the fixed names
/// this module tracks — notably `dxgi.dll`/`d3d11.dll`, which ESO does not
/// ship and therefore cannot revert.
/// The role of a managed file, from its name alone.
///
/// Deliberately not read from [`crate::client_stack::ClientStack::items`], which
/// only lists files that are on disk under their live name. The two states this
/// module most needs to report — a runtime the patcher deleted, and one that is
/// parked — are exactly the states where the file is *not* there, so an
/// inventory lookup would classify them as "not a runtime" and drop them from
/// the report entirely.
///
/// It does duplicate `client_stack::inspect_stack`'s own name-to-role table, so
/// `the_role_table_agrees_with_the_stack_inventory` pins the two together.
fn role_for_relative_path(relative_path: &str) -> Option<StackRole> {
    let name = relative_path
        .rsplit('/')
        .next()
        .unwrap_or(relative_path)
        .to_ascii_lowercase();
    match name.as_str() {
        "nvngx_dlssnr.dll" => Some(StackRole::NeuralRendering),
        "nvngx_dlss.dll" => Some(StackRole::SuperSampling),
        "nvngx_dlssg.dll" => Some(StackRole::FrameGeneration),
        "d3dcompiler_47.dll" => Some(StackRole::ShaderCompiler),
        _ => None,
    }
}

/// Whether `id` names a backup folder that actually holds `relative_path`.
///
/// A recorded id is not proof the copy exists: pruning or a wiped app-data
/// folder can remove it out from under the manifest entry. This is what turns
/// that gap into an honest `DriftedUnrecoverable` instead of a fix that fails
/// at the last moment.
fn kept_copy_exists(backup_root: &Path, id: &str, relative_path: &str) -> bool {
    crate::client_backup::backup_file_path(backup_root, id, relative_path)
        .map(|path| path.is_file())
        .unwrap_or(false)
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
    managed: &[ManagedFile],
    backup_root: &Path,
    client_root: &Path,
) -> RuntimeReport {
    let mut runtimes: Vec<RuntimeStatus> = Vec::new();
    let mut recoverable: Vec<String> = Vec::new();
    let mut unrecoverable: Vec<String> = Vec::new();

    for file in managed {
        let Some(role) = role_for_relative_path(&file.relative_path) else {
            continue;
        };
        if !is_drift_prone(role) {
            continue;
        }

        let resolved =
            match crate::client_write::safe_relative_join(client_root, &file.relative_path) {
                Ok(path) => path,
                Err(_) => continue,
            };

        let on_disk = resolved.is_file();
        let size_bytes = if on_disk {
            std::fs::metadata(&resolved).map(|m| m.len()).unwrap_or(0)
        } else {
            0
        };
        let current_version = if on_disk {
            crate::client_health::file_version(&resolved)
        } else {
            None
        };
        let kept_version = file
            .displaced_backup
            .as_deref()
            .and_then(|id| {
                crate::client_backup::backup_file_path(backup_root, id, &file.relative_path).ok()
            })
            .filter(|path| path.is_file())
            .and_then(|path| crate::client_health::file_version(&path));

        let state = if file.parked {
            DriftState::Parked
        } else if !on_disk {
            DriftState::Missing
        } else {
            match crate::client_backup::hash_file(&resolved) {
                Ok(hash) if hash == file.sha256 => DriftState::Unchanged,
                // The bytes changed. Whether that is *drift* depends entirely on
                // whether ESO ships this file — see `eso_ships`. For one it does
                // not, there is no update to undo and nothing to put back, and
                // offering to would overwrite a newer runtime with an older one.
                _ if !eso_ships(role) => DriftState::ChangedNotByUpdate,
                _ => {
                    let has_kept_copy = file
                        .displaced_backup
                        .as_deref()
                        .is_some_and(|id| kept_copy_exists(backup_root, id, &file.relative_path));
                    if has_kept_copy {
                        recoverable.push(file.relative_path.clone());
                        DriftState::DriftedRecoverable
                    } else {
                        unrecoverable.push(file.relative_path.clone());
                        DriftState::DriftedUnrecoverable
                    }
                }
            }
        };

        runtimes.push(RuntimeStatus {
            relative_path: file.relative_path.clone(),
            role,
            state,
            current_version,
            kept_version,
            kept_backup_id: file.displaced_backup.clone(),
            size_bytes,
            displaced_in_place: file.displaced_in_place.clone(),
        });
    }

    runtimes.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    RuntimeReport {
        client_dir: client_root.to_string_lossy().to_string(),
        runtimes,
        recoverable,
        unrecoverable,
    }
}

/// "Put your 310.1 back over ESO's 2.2.16" — or, when either file has no
/// version resource, a sentence built from the current file's size instead.
fn reapply_summary(status: &RuntimeStatus) -> String {
    match (&status.current_version, &status.kept_version) {
        (Some(current), Some(kept)) => {
            format!("Put your {kept} back over ESO's {current}")
        }
        _ => format!(
            "Put your kept copy of {} back over the {} bytes currently there",
            status.relative_path, status.size_bytes
        ),
    }
}

/// The plan for putting `paths` back, in the report's order.
///
/// Only paths in [`RuntimeReport::recoverable`] produce a step; anything else is
/// silently absent from the plan rather than producing a step that would fail.
pub fn plan_reapply(report: &RuntimeReport, paths: &[String]) -> Vec<ReapplyStep> {
    report
        .runtimes
        .iter()
        .filter(|status| report.recoverable.contains(&status.relative_path))
        .filter(|status| paths.contains(&status.relative_path))
        .filter_map(|status| {
            let kept_backup_id = status.kept_backup_id.clone()?;
            Some(ReapplyStep {
                relative_path: status.relative_path.clone(),
                summary: reapply_summary(status),
                kept_backup_id,
            })
        })
        .collect()
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
///
/// Never calls [`crate::client_write::begin_write`] — this reads the client
/// directory and the manifest, and writes nothing, so none of the write gates
/// apply.
#[tauri::command]
pub fn inspect_client_runtimes(
    app: tauri::AppHandle,
    client_dir: String,
) -> Result<RuntimeReport, String> {
    let location = crate::client_install::validate_client_dir(Path::new(&client_dir))?;
    let client_root = location.client_dir;
    let manifest = crate::client_backup::load_manifest(&app);
    let managed = manifest
        .installs
        .get(&crate::client_backup::install_key(&client_root))
        .cloned()
        .unwrap_or_default();
    let backup_root = crate::client_backup::backup_root(&app)?;
    Ok(inspect_runtimes(&managed, &backup_root, &client_root))
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
    let root = crate::client_write::begin_write(&state, &client_dir).await?;
    let manifest_path = crate::client_backup::manifest_path(&app)?;
    let backup_root = crate::client_backup::backup_root(&app)?;

    tokio::task::spawn_blocking(move || {
        let client_root = root.path();
        let manifest = crate::client_backup::load_manifest_at(&manifest_path);
        let managed = manifest
            .installs
            .get(&crate::client_backup::install_key(client_root))
            .cloned()
            .unwrap_or_default();
        // Recomputed fresh, never trusted from the caller: a report built
        // minutes ago describes a folder that may have changed, and only a
        // path that is *currently* recoverable may be re-applied.
        let report = inspect_runtimes(&managed, &backup_root, client_root);
        let plan = plan_reapply(&report, &relative_paths);

        let mut outcome = ReapplyOutcome {
            restored: Vec::new(),
            skipped: Vec::new(),
        };

        for step in plan {
            let Some(kind) = managed
                .iter()
                .find(|file| file.relative_path == step.relative_path)
                .map(|file| file.kind)
            else {
                outcome
                    .skipped
                    .push(format!("{}: no longer a managed file", step.relative_path));
                continue;
            };

            let source =
                match kept_copy_path(&backup_root, &step.kept_backup_id, &step.relative_path) {
                    Ok(path) => path,
                    Err(error) => {
                        outcome
                            .skipped
                            .push(format!("{}: {error}", step.relative_path));
                        continue;
                    }
                };
            if !source.is_file() {
                outcome.skipped.push(format!(
                    "{}: the kept copy is no longer on disk",
                    step.relative_path
                ));
                continue;
            }

            // Best-effort: one unreadable kept copy must not abandon the rest
            // of the batch. `edit_managed_file_from` preserves the entry's
            // origin (Adopted), so re-applying never re-records the user's own
            // runtime as something Kalpa placed.
            match crate::client_backup::edit_managed_file_from_in(
                &manifest_path,
                &backup_root,
                &root,
                &step.relative_path,
                kind,
                &source,
            ) {
                Ok(_) => outcome.restored.push(step.relative_path),
                Err(error) => outcome
                    .skipped
                    .push(format!("{}: {error}", step.relative_path)),
            }
        }

        Ok(outcome)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_write::{ApprovedRoot, FileOrigin};
    use std::path::PathBuf;

    /// A client folder shaped like the primary user's DLSS 5 setup, already
    /// adopted, with a kept copy of `nvngx_dlss.dll` in the backup root.
    struct Harness {
        _temp: tempfile::TempDir,
        manifest: PathBuf,
        backups: PathBuf,
        client: PathBuf,
    }

    impl Harness {
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
                ("nvngx_dlss.dll", "modern dlss 310.1"),
                ("nvngx_dlss.dll.disabled-bak", "stock 2.2.16"),
                ("nvngx_dlssnr.dll", "neural rendering runtime"),
                ("d3dcompiler_47.dll", "modern compiler"),
                ("dxgi.dll", "reshade injector"),
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

        fn root(&self) -> ApprovedRoot {
            ApprovedRoot::for_tests_idle(self.client.clone())
        }

        /// Adopt the current shape of the client folder, matching what
        /// `client_adopt::adopt_in` writes for a real stack.
        fn adopt(&self) {
            let stack = crate::client_stack::inspect_stack(&self.client);
            let plan = crate::client_adopt::plan_adoption_for(&stack, false);
            crate::client_adopt::adopt_in(&self.manifest, &self.backups, &self.root(), &plan, true)
                .expect("adoption should succeed");
        }

        fn managed(&self) -> Vec<ManagedFile> {
            let manifest = crate::client_backup::load_manifest_at(&self.manifest);
            let key = crate::client_backup::install_key(&self.client);
            manifest.installs.get(&key).cloned().unwrap_or_default()
        }

        fn stack(&self) -> crate::client_stack::ClientStack {
            crate::client_stack::inspect_stack(&self.client)
        }

        fn report(&self) -> RuntimeReport {
            inspect_runtimes(&self.managed(), &self.backups, &self.client)
        }

        fn entry(&self, relative: &str) -> ManagedFile {
            self.managed()
                .into_iter()
                .find(|file| file.relative_path == relative)
                .unwrap_or_else(|| panic!("no managed entry for {relative}"))
        }
    }

    /// This module maps a file name to a role itself, because the inventory
    /// cannot answer for a file that is missing or parked. That duplication is
    /// only safe while the two tables say the same thing.
    #[test]
    fn the_role_table_agrees_with_the_stack_inventory() {
        let h = Harness::new();
        let stack = h.stack();
        assert!(!stack.items.is_empty(), "the fixture should have runtimes");

        for item in &stack.items {
            let Some(role) = role_for_relative_path(&item.file_name) else {
                // Names this module does not track (the injector, add-ons)
                // simply have no entry, which is what keeps them out of the
                // report.
                assert!(
                    !is_drift_prone(item.role),
                    "{} is drift-prone but has no role entry",
                    item.file_name
                );
                continue;
            };
            assert_eq!(
                role, item.role,
                "{} is {:?} to the inventory but {role:?} here",
                item.file_name, item.role
            );
        }
    }

    #[test]
    fn is_drift_prone_agrees_with_is_copyable() {
        // These have to describe the same set of roles: a role adoption never
        // offers to copy could never become DriftedRecoverable, and a role
        // this module ignores would never get a kept copy in the first place.
        for role in [
            StackRole::Injector,
            StackRole::NeuralRendering,
            StackRole::SuperSampling,
            StackRole::FrameGeneration,
            StackRole::ShaderCompiler,
            StackRole::Addon,
            StackRole::Companion,
        ] {
            let stack_item = StackItemForTest(role);
            assert_eq!(
                is_drift_prone(role),
                stack_item.is_copyable(),
                "role {role:?} disagrees between is_drift_prone and is_copyable"
            );
        }
    }

    /// Minimal stand-in so [`is_drift_prone_agrees_with_is_copyable`] can drive
    /// `client_adopt::is_copyable`, which takes a full `StackItem`.
    struct StackItemForTest(StackRole);
    impl StackItemForTest {
        fn is_copyable(&self) -> bool {
            crate::client_adopt::is_copyable(&crate::client_stack::StackItem {
                role: self.0,
                file_name: "x".to_string(),
                display_name: None,
                version: None,
                company: None,
                description: None,
                size_bytes: 0,
            })
        }
    }

    #[test]
    fn a_runtime_byte_identical_to_the_manifest_is_unchanged() {
        let h = Harness::new();
        h.adopt();
        let report = h.report();

        let dlss = report
            .runtimes
            .iter()
            .find(|r| r.relative_path == "nvngx_dlss.dll")
            .expect("dlss should be in the report");
        assert_eq!(dlss.state, DriftState::Unchanged);
        assert!(report.recoverable.is_empty());
        assert!(report.unrecoverable.is_empty());
    }

    #[test]
    fn drift_with_a_kept_copy_is_recoverable() {
        let h = Harness::new();
        h.adopt();
        // The launcher's patcher reverts the user's swap.
        std::fs::write(h.client.join("nvngx_dlss.dll"), "eso's own 2.2.16").unwrap();

        let report = h.report();
        let dlss = report
            .runtimes
            .iter()
            .find(|r| r.relative_path == "nvngx_dlss.dll")
            .unwrap();
        assert_eq!(dlss.state, DriftState::DriftedRecoverable);
        assert!(report.recoverable.contains(&"nvngx_dlss.dll".to_string()));
        assert!(!report.unrecoverable.contains(&"nvngx_dlss.dll".to_string()));

        let plan = plan_reapply(&report, &["nvngx_dlss.dll".to_string()]);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].relative_path, "nvngx_dlss.dll");
    }

    /// ESO does not ship `nvngx_dlssnr.dll`, so a change to it cannot be a game
    /// update. Calling it drift and offering to "put your copy back" would
    /// overwrite a newer 165 MB runtime — one with no redistributable source —
    /// with Kalpa's older copy, and file the newer one somewhere prunable.
    #[test]
    fn a_changed_nr_runtime_is_not_treated_as_a_reverted_update() {
        let h = Harness::new();
        h.adopt();
        std::fs::write(h.client.join("nvngx_dlssnr.dll"), "a newer NR runtime").unwrap();

        let report = h.report();
        let nr = report
            .runtimes
            .iter()
            .find(|r| r.relative_path == "nvngx_dlssnr.dll")
            .expect("the NR runtime is still reported");

        assert_eq!(nr.state, DriftState::ChangedNotByUpdate);
        assert!(
            !report.recoverable.contains(&"nvngx_dlssnr.dll".to_string()),
            "there must be no offer to overwrite it"
        );
        assert!(
            !report
                .unrecoverable
                .contains(&"nvngx_dlssnr.dll".to_string()),
            "nor is it a failure — the user changed it on purpose"
        );
        assert!(
            plan_reapply(&report, &["nvngx_dlssnr.dll".to_string()]).is_empty(),
            "and no plan can be built for it even if asked directly"
        );
    }

    /// The predicates that were conflated, kept apart by a test rather than by
    /// memory: *worth keeping a copy of* is not *a game update can revert this*.
    #[test]
    fn keeping_a_copy_and_being_revertible_are_different_questions() {
        assert!(is_drift_prone(StackRole::NeuralRendering));
        assert!(
            !eso_ships(StackRole::NeuralRendering),
            "ESO has never shipped nvngx_dlssnr.dll, so no patch puts its build back"
        );
        for role in [StackRole::SuperSampling, StackRole::ShaderCompiler] {
            assert!(eso_ships(role), "{role:?}");
            assert!(is_drift_prone(role), "{role:?}");
        }
    }

    #[test]
    fn drift_with_no_kept_copy_is_unrecoverable_and_unplannable() {
        let h = Harness::new();
        // Adopt without keeping copies.
        let stack = h.stack();
        let plan = crate::client_adopt::plan_adoption_for(&stack, false);
        crate::client_adopt::adopt_in(&h.manifest, &h.backups, &h.root(), &plan, false)
            .expect("adoption should succeed");

        std::fs::write(h.client.join("nvngx_dlss.dll"), "eso's own 2.2.16").unwrap();

        let report = h.report();
        let dlss = report
            .runtimes
            .iter()
            .find(|r| r.relative_path == "nvngx_dlss.dll")
            .unwrap();
        assert_eq!(dlss.state, DriftState::DriftedUnrecoverable);
        assert!(report.unrecoverable.contains(&"nvngx_dlss.dll".to_string()));
        assert!(!report.recoverable.contains(&"nvngx_dlss.dll".to_string()));

        let steps = plan_reapply(&report, &["nvngx_dlss.dll".to_string()]);
        assert!(
            steps.is_empty(),
            "an unrecoverable path must produce no step"
        );
    }

    /// The pruned-backup case: the manifest still names a backup id, but the
    /// file under it is gone. This must not be reported as recoverable — a
    /// button offered on the strength of a stale id would fail at the last
    /// moment.
    #[test]
    fn a_recorded_but_pruned_backup_is_unrecoverable() {
        let h = Harness::new();
        h.adopt();

        let id = h
            .entry("nvngx_dlss.dll")
            .displaced_backup
            .clone()
            .expect("adoption with keep_copies should have recorded a backup id");
        let backup_file =
            crate::client_backup::backup_file_path(&h.backups, &id, "nvngx_dlss.dll").unwrap();
        assert!(
            backup_file.is_file(),
            "sanity: the kept copy exists before pruning"
        );
        std::fs::remove_file(&backup_file).expect("simulate a pruned backup");

        std::fs::write(h.client.join("nvngx_dlss.dll"), "eso's own 2.2.16").unwrap();

        let report = h.report();
        let dlss = report
            .runtimes
            .iter()
            .find(|r| r.relative_path == "nvngx_dlss.dll")
            .unwrap();
        assert_eq!(
            dlss.state,
            DriftState::DriftedUnrecoverable,
            "a recorded id whose file is gone must not be reported recoverable"
        );
        assert!(report.unrecoverable.contains(&"nvngx_dlss.dll".to_string()));
        assert!(!report.recoverable.contains(&"nvngx_dlss.dll".to_string()));
    }

    #[test]
    fn a_parked_entry_is_parked_not_drift_even_with_different_live_bytes() {
        let h = Harness::new();
        h.adopt();

        // Mark the manifest entry parked directly; parking in the real flow
        // also moves the bytes aside, but the state check only consults the
        // flag plus whatever is at the live path.
        let mut manifest = crate::client_backup::load_manifest_at(&h.manifest);
        let key = crate::client_backup::install_key(&h.client);
        for entry in manifest.installs.get_mut(&key).unwrap() {
            if entry.relative_path == "nvngx_dlss.dll" {
                entry.parked = true;
            }
        }
        let bytes = serde_json::to_vec_pretty(&manifest).expect("serialize manifest");
        std::fs::write(&h.manifest, bytes).expect("write manifest directly for the test");

        std::fs::write(h.client.join("nvngx_dlss.dll"), "different bytes entirely").unwrap();

        let report = h.report();
        let dlss = report
            .runtimes
            .iter()
            .find(|r| r.relative_path == "nvngx_dlss.dll")
            .unwrap();
        assert_eq!(dlss.state, DriftState::Parked);
        assert!(!report.recoverable.contains(&"nvngx_dlss.dll".to_string()));
        assert!(!report.unrecoverable.contains(&"nvngx_dlss.dll".to_string()));
    }

    #[test]
    fn the_injector_is_never_in_the_report() {
        let h = Harness::new();
        h.adopt();
        let report = h.report();
        assert!(
            !report
                .runtimes
                .iter()
                .any(|r| r.relative_path == "dxgi.dll"),
            "ESO does not ship dxgi.dll, so no patch reverts it"
        );
    }

    #[test]
    fn reapplying_restores_exact_bytes_and_keeps_origin_adopted() {
        let h = Harness::new();
        h.adopt();
        let kept_bytes = std::fs::read(h.client.join("nvngx_dlss.dll")).unwrap();

        // The launcher's patcher reverts the swap.
        std::fs::write(h.client.join("nvngx_dlss.dll"), "eso's own 2.2.16").unwrap();

        let report = h.report();
        assert!(
            report.recoverable.contains(&"nvngx_dlss.dll".to_string()),
            "sanity: the drift must be recoverable before re-applying"
        );
        let entry = h.entry("nvngx_dlss.dll");
        let backup_id = entry.displaced_backup.clone().unwrap();
        let source = kept_copy_path(&h.backups, &backup_id, "nvngx_dlss.dll").unwrap();
        assert!(source.is_file(), "sanity: the kept copy exists");

        let outcome = crate::client_backup::edit_managed_file_from_in(
            &h.manifest,
            &h.backups,
            &h.root(),
            "nvngx_dlss.dll",
            entry.kind,
            &source,
        )
        .expect("re-apply should succeed");
        assert!(outcome.manifest_updated);

        let restored = std::fs::read(h.client.join("nvngx_dlss.dll")).unwrap();
        assert_eq!(
            restored, kept_bytes,
            "the exact kept bytes must be restored"
        );
        assert!(
            source.is_file(),
            "the kept copy must still be on disk afterwards"
        );

        let refreshed = h.entry("nvngx_dlss.dll");
        let expected_hash =
            crate::client_backup::hash_file(&h.client.join("nvngx_dlss.dll")).unwrap();
        assert_eq!(
            refreshed.sha256, expected_hash,
            "the manifest hash must be refreshed"
        );
        assert_eq!(
            refreshed.origin,
            FileOrigin::Adopted,
            "re-apply must never re-record an adopted file as Kalpa's own placement"
        );

        // The plan for this now-healed path is empty: it is no longer drifted.
        let healed_report = inspect_runtimes(&h.managed(), &h.backups, &h.client);
        assert!(healed_report.recoverable.is_empty());
        assert!(healed_report
            .runtimes
            .iter()
            .find(|r| r.relative_path == "nvngx_dlss.dll")
            .map(|r| r.state)
            .is_some_and(|s| s == DriftState::Unchanged));
    }
}
