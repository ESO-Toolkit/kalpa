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
use crate::client_stack::{ClientStack, StackRole, PARKED_SUFFIX};
use crate::client_write::{AllowedGameInstallPath, ManagedFile, ManagedKind};
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
    /// The other file this step involves, for the two steps that take two: the
    /// live name a [`ToggleOpKind::RestoreOriginal`] copies *to*, and the live
    /// name a [`ToggleOpKind::RemoveRestored`] frees.
    ///
    /// Carried explicitly rather than inferred from the neighbouring step.
    /// Pairing by adjacency reads fine and fails silently: a `Park` whose
    /// `RestoreOriginal` was dropped leaves ESO with no file at all under a
    /// name it loads itself, which is the worst outcome this module can
    /// produce. [`to_file_ops`] refuses a plan with a missing partner instead.
    pub partner: Option<String>,
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
///
/// Note what is deliberately *not* a blocker: a manifest entry whose `parked`
/// flag the folder contradicts. The folder decides — a stale flag gets an
/// informational line and `client_backup::reconcile_parked_flags` corrects the
/// record on the next batch. Refusing on the flag was itself a dead end, since
/// nothing in the app could then clear it.
pub fn plan_toggle(stack: &ClientStack, managed: &[ManagedFile], client_dir: &str) -> TogglePlan {
    // **Disk decides, not the manifest.** Any `.kalpa-off` file at all means there
    // is parking to undo, even one — a batch that died between parking a runtime
    // and restoring the original leaves exactly that, and treating it as "still
    // switched on" would plan a fresh disable that skips the half-parked file and
    // leaves ESO with no file under a name it loads itself.
    let action = if stack.parked.is_empty() {
        ToggleAction::Disable
    } else {
        ToggleAction::Enable
    };

    let mut operations = Vec::new();
    let mut blockers = Vec::new();

    match action {
        ToggleAction::Disable if managed.is_empty() => {
            blockers.push(
                "This install has nothing managed yet, so there is nothing for Kalpa to switch \
                 off."
                    .to_string(),
            );
        }
        ToggleAction::Disable => plan_disable(stack, managed, &mut operations, &mut blockers),
        // Deliberately not gated on `managed`: putting parked files back must
        // work from the folder alone. See `plan_enable`.
        ToggleAction::Enable => plan_enable(stack, managed, &mut operations, &mut blockers),
    }

    // A plan with nothing to do and nothing to say would render as an empty
    // confirmation with a live button, and applying it would report success
    // having changed nothing. Enable cannot reach this: it is only chosen when
    // something is parked, and every parked file produces an operation or a
    // blocker.
    if action == ToggleAction::Disable
        && blockers.is_empty()
        && operations
            .iter()
            .all(|op| op.kind == ToggleOpKind::LeaveInPlace)
    {
        blockers.push("There is nothing here for Kalpa to switch off.".to_string());
    }

    TogglePlan {
        client_dir: client_dir.to_string(),
        action,
        is_disabled: stack.is_disabled,
        operations,
        blockers,
    }
}

/// Build the disable half of [`plan_toggle`].
///
/// Runs over `managed` in its existing (relative-path-sorted) order, which is
/// what `client_backup` always keeps it in, so the plan is deterministic
/// without this module imposing its own sort.
fn plan_disable(
    stack: &ClientStack,
    managed: &[ManagedFile],
    operations: &mut Vec<PlannedOp>,
    blockers: &mut Vec<String>,
) {
    // The injector has to be both managed and actually loaded by the game
    // right now — a manifest entry alone does not prove a live dxgi.dll is
    // still sitting in the folder to park.
    // `role_of` reading the live inventory is the presence test; the entry's
    // own `parked` flag is deliberately not consulted. A stale flag would
    // otherwise hide an injector that is sitting right there and refuse the
    // whole operation with "no injector present" — which the folder plainly
    // contradicts.
    //
    // Every live managed injector is parked, not just the first one found. The
    // DLL search order will load `dxgi.dll` and `d3d11.dll` alike, so a folder
    // carrying both under management has two doors into ReShade. Parking one
    // and reporting the stack "off" — which `is_disabled` would, since it asks
    // only whether *an* injector name is parked — would be a false statement
    // about the thing the user pressed the switch for.
    let injectors: Vec<&ManagedFile> = managed
        .iter()
        .filter(|entry| {
            entry.kind == ManagedKind::ReShadeCore
                && role_of(stack, &entry.relative_path) == Some(StackRole::Injector)
        })
        .collect();
    if injectors.is_empty() {
        blockers.push(
            "No injector (dxgi.dll or d3d11.dll) is present in the folder to switch off."
                .to_string(),
        );
    }

    for entry in managed {
        // The folder decides, not the flag. A record saying "parked" whose
        // `.kalpa-off` file is not there is stale — most often because the user
        // renamed it back by hand — and the live file is sitting right where it
        // belongs. Believing the flag would refuse to switch off a folder that
        // is working, with no way to clear it; `reconcile_parked_flags` will
        // correct the record as soon as this batch runs.
        let parked_on_disk = stack
            .parked
            .iter()
            .any(|file| file.restores.eq_ignore_ascii_case(&entry.relative_path));
        let live_on_disk = role_of(stack, &entry.relative_path).is_some();

        if entry.parked && !parked_on_disk && !live_on_disk {
            // Genuinely absent, both names. Worth saying, but it is one file
            // missing — not a reason to refuse to switch off the rest.
            operations.push(PlannedOp {
                kind: ToggleOpKind::LeaveInPlace,
                file_name: entry.relative_path.clone(),
                partner: None,
                summary: format!("{} is not in the folder", entry.relative_path),
                detail: "Kalpa has a record of this file but neither it nor a parked copy is \
                         here, so there is nothing to switch off. The record is left alone in \
                         case you put the file back."
                    .to_string(),
            });
            continue;
        }
        // Already off on disk, or is the injector itself — the injector gets
        // its own step below, parked last.
        if parked_on_disk || entry.kind == ManagedKind::ReShadeCore {
            continue;
        }

        let must_go_stock = role_of(stack, &entry.relative_path).is_some_and(game_loads_itself);
        if !must_go_stock {
            operations.push(PlannedOp {
                kind: ToggleOpKind::LeaveInPlace,
                file_name: entry.relative_path.clone(),
                partner: None,
                summary: format!("Leave {} in place", entry.relative_path),
                detail: "Nothing loads this file once the injector is parked, so it can stay \
                         in the folder, switched off along with the rest of the stack."
                    .to_string(),
            });
            continue;
        }

        match &entry.displaced_in_place {
            Some(original)
                if stack
                    .preserved_originals
                    .iter()
                    .any(|preserved| preserved.file_name.eq_ignore_ascii_case(original)) =>
            {
                operations.push(PlannedOp {
                    kind: ToggleOpKind::Park,
                    file_name: entry.relative_path.clone(),
                    partner: None,
                    summary: format!(
                        "Park {} as {}{PARKED_SUFFIX}",
                        entry.relative_path, entry.relative_path
                    ),
                    detail: format!(
                        "ESO loads {} itself, so it has to be moved aside before your own \
                         preserved original can go live under that name.",
                        entry.relative_path
                    ),
                });
                operations.push(PlannedOp {
                    kind: ToggleOpKind::RestoreOriginal,
                    file_name: original.clone(),
                    partner: Some(entry.relative_path.clone()),
                    summary: format!("Copy your own {original} back to {}", entry.relative_path),
                    detail: format!(
                        "{original} is the copy you kept before Kalpa managed this install; \
                         putting it back is what makes {} the file ESO actually loads.",
                        entry.relative_path
                    ),
                });
            }
            Some(original) => {
                blockers.push(format!(
                    "{} replaces a file ESO loads itself, but the preserved original {original} \
                     is no longer in the folder.",
                    entry.relative_path
                ));
            }
            None => {
                blockers.push(format!(
                    "{} replaces a file ESO loads itself, but Kalpa has no preserved original \
                     for it, so switching off would leave the game with no file under that name.",
                    entry.relative_path
                ));
            }
        }
    }

    // Parked last: a failure part-way through this batch leaves the folder
    // still loading the stack, rather than a half-stock mix.
    for injector in &injectors {
        operations.push(PlannedOp {
            kind: ToggleOpKind::Park,
            file_name: injector.relative_path.clone(),
            partner: None,
            summary: format!(
                "Park {} as {}{PARKED_SUFFIX}",
                injector.relative_path, injector.relative_path
            ),
            detail: "Parking the injector last means a failure earlier in this batch leaves \
                     the folder still loading the stack, rather than a half-stock mix."
                .to_string(),
        });
    }
}

/// Build the enable half of [`plan_toggle`] **from the folder, not the manifest**.
///
/// This is deliberately the one operation in the client layer that does not need
/// Kalpa's records. Every other action can reasonably say "manage this stack
/// first"; putting parked files back cannot, because the states that most need it
/// are exactly the states where the records are gone or wrong:
///
/// * the manifest was lost, or the app-data folder wiped, while the stack was off
/// * a batch died between moving the files and recording that it had
/// * the user pressed "Stop managing" while switched off
///
/// In all three the folder still holds the whole truth. `.kalpa-off` is Kalpa's
/// own suffix — nothing else writes it — so a file carrying it is one Kalpa
/// parked, `restores` says what it goes back to, and `target_present` says
/// whether something has to come out of the way first. The manifest is consulted
/// only for `displaced_in_place`, and the shape of the folder answers that too
/// when it is missing.
fn plan_enable(
    stack: &ClientStack,
    managed: &[ManagedFile],
    operations: &mut Vec<PlannedOp>,
    blockers: &mut Vec<String>,
) {
    // Which preserved original a live name belongs to: the manifest's record if
    // there is one, else the user's own `.disabled-bak` sitting next to it,
    // which is where the manifest got it from in the first place.
    let original_for = |live: &str| -> Option<String> {
        managed
            .iter()
            .find(|entry| entry.relative_path.eq_ignore_ascii_case(live))
            .and_then(|entry| entry.displaced_in_place.clone())
            .or_else(|| {
                stack
                    .preserved_originals
                    .iter()
                    .find(|original| {
                        original
                            .backs_up
                            .as_deref()
                            .is_some_and(|target| target.eq_ignore_ascii_case(live))
                    })
                    .map(|original| original.file_name.clone())
            })
    };

    // Records claiming a park the folder does not have. Two cases, and only one
    // of them is a problem: if the live file is there, the record is merely
    // stale (a hand-renamed `.kalpa-off`) and `reconcile_parked_flags` clears it
    // after this batch. If neither name is present the file is genuinely gone,
    // which is worth saying — but it is one file, and refusing to put the rest
    // of the stack back because of it would be the dead end this planner exists
    // to avoid.
    for entry in managed.iter().filter(|entry| entry.parked) {
        let parked_on_disk = stack
            .parked
            .iter()
            .any(|file| file.restores.eq_ignore_ascii_case(&entry.relative_path));
        if !parked_on_disk {
            // Said whether or not the live name is occupied. What is missing is
            // the *parked copy* — the bytes Kalpa moved aside — and that is
            // equally true if someone renamed it back by hand or deleted it. A
            // planner that stays quiet because *something* holds the live name
            // would be reassuring the user about a file it cannot account for.
            operations.push(PlannedOp {
                kind: ToggleOpKind::LeaveInPlace,
                file_name: entry.relative_path.clone(),
                partner: None,
                summary: format!("No parked copy of {} to put back", entry.relative_path),
                detail: format!(
                    "Kalpa recorded {} as switched off, but there is no {}{PARKED_SUFFIX} in \
                     the folder. Either it was renamed back by hand — in which case nothing \
                     needs doing — or it was deleted.",
                    entry.relative_path, entry.relative_path
                ),
            });
        }
    }

    // The injector first, so ReShade is loading the stack again before anything
    // else moves. Sorted order otherwise, so the plan is stable between reads.
    let mut parked: Vec<&crate::client_stack::ParkedFile> = stack.parked.iter().collect();
    parked.sort_by_key(|file| {
        (
            !crate::client_uninstall::INJECTOR_NAMES.contains(&file.restores.as_str()),
            file.restores.clone(),
        )
    });

    for file in parked {
        let live = &file.restores;

        // Something occupies the name. For a file ESO loads itself that is the
        // stock build disable put there, and it has to come out first — but only
        // once Kalpa can prove it is not about to delete something else.
        if file.target_present {
            let Some(original) = original_for(live) else {
                blockers.push(format!(
                    "{live} is parked, but something already occupies that name and Kalpa has no \
                     preserved original to check it against, so it will not remove it. Move it \
                     aside yourself, then switch the stack back on."
                ));
                continue;
            };
            operations.push(PlannedOp {
                kind: ToggleOpKind::RemoveRestored,
                file_name: original.clone(),
                partner: Some(live.clone()),
                summary: format!("Remove the stock {live}"),
                detail: format!(
                    "Disable put ESO's own {live} live under this name; it has to come out \
                     before your own copy can go back. Kalpa checks it still matches {original} \
                     first, and keeps a copy if it does not."
                ),
            });
        }

        operations.push(PlannedOp {
            kind: ToggleOpKind::Unpark,
            file_name: live.clone(),
            partner: None,
            summary: format!("Put {live} back from {}", file.file_name),
            detail: if crate::client_uninstall::INJECTOR_NAMES.contains(&live.as_str()) {
                "Putting the injector back is what starts ReShade loading the stack again."
                    .to_string()
            } else {
                "Putting your own file back where the modded one was.".to_string()
            },
        });
    }
}

/// The role of a managed entry, resolved from the stack inventory.
///
/// A manifest entry records a `ManagedKind`, which is a write-policy category,
/// not a position in the pipeline. The role is what decides whether a file can
/// simply be parked or has to have an original put back in its place, so it is
/// read from the live inventory.
pub fn role_of(stack: &ClientStack, relative_path: &str) -> Option<StackRole> {
    stack
        .items
        .iter()
        .find(|item| item.file_name.eq_ignore_ascii_case(relative_path))
        .map(|item| item.role)
}

/// True for roles the **game itself** loads, which therefore cannot be left
/// with no file under their name.
///
/// `nvngx_dlssnr.dll` is deliberately absent: ESO does not ship or load it, so
/// with the injector parked it is simply inert.
pub fn game_loads_itself(role: StackRole) -> bool {
    // The same predicate as "a game update can revert this", and deliberately
    // the same function: both are asking whether ESO ships a file under this
    // name. Keeping two lists in step by hand is how `nvngx_dlssnr.dll` — which
    // ESO does not ship — ended up being treated as reverted by an update.
    crate::client_runtime::eso_ships(role)
}

/// Translate the plan into the batch [`crate::client_backup::run_file_ops`]
/// will execute. [`ToggleOpKind::LeaveInPlace`] steps produce nothing.
pub fn to_file_ops(plan: &TogglePlan) -> Result<Vec<FileOp>, String> {
    // Note which side of each two-file step the names sit on. A backup name
    // (`.disabled-bak`, `.eso-orig-bak`, …) may only ever appear as a
    // `RestoreInPlace` source or a `RemoveRestored::must_match` — never as
    // something Kalpa parks, unparks or removes. Those files are the user's
    // originals, and they are the reason disable can be undone at all.
    let mut file_ops = Vec::with_capacity(plan.operations.len());

    for op in &plan.operations {
        let partner = || {
            op.partner.clone().ok_or_else(|| {
                format!(
                    "Internal error: the planned step for {} is missing the file it pairs with, \
                     so Kalpa is refusing to run this batch.",
                    op.file_name
                )
            })
        };
        match op.kind {
            ToggleOpKind::Park => file_ops.push(FileOp::Park {
                relative_path: op.file_name.clone(),
            }),
            ToggleOpKind::Unpark => file_ops.push(FileOp::Unpark {
                relative_path: op.file_name.clone(),
            }),
            ToggleOpKind::RestoreOriginal => file_ops.push(FileOp::RestoreInPlace {
                source: op.file_name.clone(),
                destination: partner()?,
            }),
            ToggleOpKind::RemoveRestored => file_ops.push(FileOp::RemoveRestored {
                relative_path: partner()?,
                must_match: op.file_name.clone(),
            }),
            ToggleOpKind::LeaveInPlace => {}
        }
    }

    Ok(file_ops)
}

// ── Commands ─────────────────────────────────────────────────────────────

/// Read-only: what switching this stack would do.
#[tauri::command(async)]
pub fn plan_client_toggle(app: tauri::AppHandle, client_dir: String) -> Result<TogglePlan, String> {
    let location = crate::client_install::validate_client_dir(Path::new(&client_dir))?;
    let stack = crate::client_stack::inspect_stack(&location.client_dir);
    let manifest_path = crate::client_backup::manifest_path(&app)?;
    let managed = managed_entries(&manifest_path, &location.client_dir);
    let dir = stack.client_dir.clone();
    Ok(plan_toggle(&stack, &managed, &dir))
}

/// Switch the stack off, or back on.
///
/// `expected` is a compare-and-swap against the current state, not a request:
/// the plan the user approved described one direction, and if the folder has
/// changed under them since (another window, a game update) the right answer is
/// to refuse and re-plan, not to run the other direction silently.
///
/// The plan is recomputed here from the directory rather than accepted from the
/// caller, for the same reason `adopt_stack` recomputes its own — and it is
/// recomputed *inside* `MANIFEST_LOCK`, so the folder and the manifest that
/// `expected` is checked against are the same ones the batch then acts on.
/// Read before the lock, the direction check answers for a state another batch
/// may already have replaced.
#[tauri::command]
pub async fn apply_client_toggle(
    app: tauri::AppHandle,
    state: tauri::State<'_, AllowedGameInstallPath>,
    client_dir: String,
    expected: ToggleAction,
) -> Result<crate::client_backup::FileOpOutcome, String> {
    let root = crate::client_write::begin_write(&state, &client_dir).await?;
    let manifest_path = crate::client_backup::manifest_path(&app)?;
    let backup_root = crate::client_backup::backup_root(&app)?;

    tokio::task::spawn_blocking(move || {
        crate::client_backup::run_planned_file_ops_in(&manifest_path, &backup_root, &root, || {
            let stack = crate::client_stack::inspect_stack(root.path());
            let managed = managed_entries(&manifest_path, root.path());
            let dir = stack.client_dir.clone();
            let plan = plan_toggle(&stack, &managed, &dir);

            let action_word = |action: ToggleAction| match action {
                ToggleAction::Disable => "disable",
                ToggleAction::Enable => "enable",
            };

            if !plan.blockers.is_empty() {
                return Err(format!(
                    "Cannot {} this stack: {}",
                    action_word(expected),
                    plan.blockers.join(" ")
                ));
            }
            if plan.action != expected {
                return Err(format!(
                    "The client folder has changed since this was planned, so Kalpa is refusing to \
                     {} it. Re-open the confirmation to see the current plan.",
                    action_word(expected)
                ));
            }

            to_file_ops(&plan)
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_stack::inspect_stack;
    use crate::client_write::{ApprovedRoot, FileOrigin};

    fn write(dir: &Path, name: &str, contents: &str) {
        std::fs::write(dir.join(name), contents).expect("write fixture");
    }

    /// The install shape from the module doc: an injector, both ESO-shipped
    /// files with the user's own preserved originals beside them, the inert
    /// Neural Rendering runtime, an addon, and ReShade's own config.
    fn real_install(dir: &Path) {
        write(dir, "eso64.exe", "game");
        write(dir, "dxgi.dll", "reshade");
        write(dir, "nvngx_dlssnr.dll", "neural rendering runtime");
        write(dir, "nvngx_dlss.dll", "the modded dlss");
        write(dir, "nvngx_dlss.dll.disabled-bak", "the stock dlss");
        write(dir, "d3dcompiler_47.dll", "the modded compiler");
        write(dir, "d3dcompiler_47.dll.eso-orig-bak", "the stock compiler");
        write(dir, "renodx-dlss5.addon64", "addon");
        write(dir, "ReShade.ini", "[GENERAL]\n");
    }

    fn managed_file(
        relative_path: &str,
        kind: ManagedKind,
        displaced_in_place: Option<&str>,
        parked: bool,
    ) -> ManagedFile {
        ManagedFile {
            relative_path: relative_path.to_string(),
            kind,
            sha256: "a".repeat(64),
            placed_at: "2026-01-01T00:00:00Z".to_string(),
            displaced_backup: None,
            origin: FileOrigin::Adopted,
            displaced_in_place: displaced_in_place.map(str::to_string),
            parked,
        }
    }

    /// The manifest bucket for a fully-adopted real install, not yet disabled.
    fn managed_stack() -> Vec<ManagedFile> {
        vec![
            managed_file("dxgi.dll", ManagedKind::ReShadeCore, None, false),
            managed_file(
                "nvngx_dlss.dll",
                ManagedKind::NvidiaRuntime,
                Some("nvngx_dlss.dll.disabled-bak"),
                false,
            ),
            managed_file("nvngx_dlssnr.dll", ManagedKind::NvidiaRuntime, None, false),
            managed_file(
                "d3dcompiler_47.dll",
                ManagedKind::ShaderCompiler,
                Some("d3dcompiler_47.dll.eso-orig-bak"),
                false,
            ),
            managed_file("renodx-dlss5.addon64", ManagedKind::Addon, None, false),
        ]
    }

    fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
            .expect("read dir")
            .flatten()
            .filter(|entry| entry.path().is_file())
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().to_string(),
                    std::fs::read(entry.path()).unwrap_or_default(),
                )
            })
            .collect();
        out.sort();
        out
    }

    /// A step that takes two files must carry both. Dropping the copy that
    /// puts the game's own DLL back would leave ESO with no file under a name
    /// it loads itself, so the batch is refused rather than run short.
    #[test]
    fn to_file_ops_refuses_a_step_that_lost_its_partner() {
        let plan = TogglePlan {
            client_dir: "client".to_string(),
            action: ToggleAction::Disable,
            is_disabled: false,
            blockers: Vec::new(),
            operations: vec![
                PlannedOp {
                    kind: ToggleOpKind::Park,
                    file_name: "nvngx_dlss.dll".to_string(),
                    partner: None,
                    summary: String::new(),
                    detail: String::new(),
                },
                PlannedOp {
                    kind: ToggleOpKind::RestoreOriginal,
                    file_name: "nvngx_dlss.dll.disabled-bak".to_string(),
                    partner: None,
                    summary: String::new(),
                    detail: String::new(),
                },
            ],
        };

        let error = to_file_ops(&plan).expect_err("a half-described step must not run");
        assert!(error.contains("nvngx_dlss.dll.disabled-bak"), "{error}");
    }

    /// `.kalpa-off` files on disk with no manifest rows to explain them. The
    /// plan has nothing to do, and must say so rather than render an empty
    /// confirmation whose button reports success having changed nothing.
    #[test]
    fn a_parked_stack_kalpa_has_no_record_of_is_recovered_from_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        real_install(tmp.path());
        std::fs::rename(
            tmp.path().join("dxgi.dll"),
            tmp.path().join("dxgi.dll.kalpa-off"),
        )
        .unwrap();
        let stack = inspect_stack(tmp.path());
        assert!(stack.is_disabled);

        // Managed, but nothing recorded as parked — the manifest was lost and
        // rebuilt, or the files were moved by hand.
        let managed = vec![managed_file(
            "dxgi.dll",
            ManagedKind::ReShadeCore,
            None,
            false,
        )];
        let plan = plan_toggle(&stack, &managed, "client");

        assert_eq!(plan.action, ToggleAction::Enable);
        assert!(
            plan.blockers.is_empty(),
            "the folder holds the whole truth, so this must not block: {:?}",
            plan.blockers
        );
        assert_eq!(
            to_file_ops(&plan).expect("plan is complete"),
            vec![FileOp::Unpark {
                relative_path: "dxgi.dll".to_string(),
            }],
            "putting parked files back must work from the folder alone"
        );
    }

    /// The same recovery with no manifest at all — a wiped app-data folder.
    #[test]
    fn a_parked_stack_with_no_manifest_at_all_is_still_recoverable() {
        let tmp = tempfile::tempdir().unwrap();
        real_install(tmp.path());
        for name in ["dxgi.dll", "nvngx_dlss.dll"] {
            std::fs::rename(
                tmp.path().join(name),
                tmp.path().join(format!("{name}.kalpa-off")),
            )
            .unwrap();
        }
        std::fs::copy(
            tmp.path().join("nvngx_dlss.dll.disabled-bak"),
            tmp.path().join("nvngx_dlss.dll"),
        )
        .unwrap();

        let stack = inspect_stack(tmp.path());
        let plan = plan_toggle(&stack, &[], "client");

        assert_eq!(plan.action, ToggleAction::Enable);
        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
        assert_eq!(
            to_file_ops(&plan).expect("plan is complete"),
            vec![
                FileOp::Unpark {
                    relative_path: "dxgi.dll".to_string(),
                },
                // The original to check against came from the folder, not the
                // manifest: it is the user's own `.disabled-bak` sitting there.
                FileOp::RemoveRestored {
                    relative_path: "nvngx_dlss.dll".to_string(),
                    must_match: "nvngx_dlss.dll.disabled-bak".to_string(),
                },
                FileOp::Unpark {
                    relative_path: "nvngx_dlss.dll".to_string(),
                },
            ]
        );
    }

    #[test]
    fn role_of_reads_the_live_inventory() {
        let tmp = tempfile::tempdir().unwrap();
        real_install(tmp.path());
        let stack = inspect_stack(tmp.path());

        assert_eq!(role_of(&stack, "dxgi.dll"), Some(StackRole::Injector));
        assert_eq!(role_of(&stack, "DXGI.DLL"), Some(StackRole::Injector));
        assert_eq!(
            role_of(&stack, "nvngx_dlss.dll"),
            Some(StackRole::SuperSampling)
        );
        assert_eq!(role_of(&stack, "does-not-exist.dll"), None);
    }

    #[test]
    fn game_loads_itself_covers_exactly_the_files_eso_ships() {
        assert!(game_loads_itself(StackRole::SuperSampling));
        assert!(game_loads_itself(StackRole::ShaderCompiler));
        // ESO ships neither of these, so parking them leaves no gap — and a
        // change to one of them is not a game update undoing a swap.
        assert!(!game_loads_itself(StackRole::FrameGeneration));
        assert!(!game_loads_itself(StackRole::NeuralRendering));
        assert!(!game_loads_itself(StackRole::Injector));
        assert!(!game_loads_itself(StackRole::Addon));
        assert!(!game_loads_itself(StackRole::Companion));
    }

    #[test]
    fn disable_parks_the_injector_and_restores_both_eso_shipped_runtimes() {
        let tmp = tempfile::tempdir().unwrap();
        real_install(tmp.path());
        let stack = inspect_stack(tmp.path());
        let managed = managed_stack();

        let plan = plan_toggle(&stack, &managed, &stack.client_dir);

        assert_eq!(plan.action, ToggleAction::Disable);
        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);

        let dlss_pos = plan
            .operations
            .iter()
            .position(|op| op.kind == ToggleOpKind::Park && op.file_name == "nvngx_dlss.dll")
            .expect("park of nvngx_dlss.dll");
        assert_eq!(
            plan.operations[dlss_pos + 1].kind,
            ToggleOpKind::RestoreOriginal
        );
        assert_eq!(
            plan.operations[dlss_pos + 1].file_name,
            "nvngx_dlss.dll.disabled-bak"
        );

        let compiler_pos = plan
            .operations
            .iter()
            .position(|op| op.kind == ToggleOpKind::Park && op.file_name == "d3dcompiler_47.dll")
            .expect("park of d3dcompiler_47.dll");
        assert_eq!(
            plan.operations[compiler_pos + 1].kind,
            ToggleOpKind::RestoreOriginal
        );
        assert_eq!(
            plan.operations[compiler_pos + 1].file_name,
            "d3dcompiler_47.dll.eso-orig-bak"
        );

        // The injector's Park is the last non-LeaveInPlace operation.
        let last_non_leave = plan
            .operations
            .iter()
            .rev()
            .find(|op| op.kind != ToggleOpKind::LeaveInPlace)
            .expect("some operation");
        assert_eq!(last_non_leave.kind, ToggleOpKind::Park);
        assert_eq!(last_non_leave.file_name, "dxgi.dll");
    }

    #[test]
    fn inert_files_are_left_in_place_not_parked() {
        let tmp = tempfile::tempdir().unwrap();
        real_install(tmp.path());
        let stack = inspect_stack(tmp.path());
        let managed = managed_stack();

        let plan = plan_toggle(&stack, &managed, &stack.client_dir);

        for name in ["nvngx_dlssnr.dll", "renodx-dlss5.addon64"] {
            let op = plan
                .operations
                .iter()
                .find(|op| op.file_name == name)
                .unwrap_or_else(|| panic!("no operation for {name}"));
            assert_eq!(op.kind, ToggleOpKind::LeaveInPlace, "{name}");
        }
    }

    #[test]
    fn to_file_ops_matches_the_disable_batch_and_never_names_a_backup_as_a_destination() {
        let tmp = tempfile::tempdir().unwrap();
        real_install(tmp.path());
        let stack = inspect_stack(tmp.path());
        let managed = managed_stack();
        let plan = plan_toggle(&stack, &managed, &stack.client_dir);

        let ops = to_file_ops(&plan).expect("every planned step must carry its partner");

        assert_eq!(
            ops,
            vec![
                FileOp::Park {
                    relative_path: "nvngx_dlss.dll".to_string(),
                },
                FileOp::RestoreInPlace {
                    source: "nvngx_dlss.dll.disabled-bak".to_string(),
                    destination: "nvngx_dlss.dll".to_string(),
                },
                FileOp::Park {
                    relative_path: "d3dcompiler_47.dll".to_string(),
                },
                FileOp::RestoreInPlace {
                    source: "d3dcompiler_47.dll.eso-orig-bak".to_string(),
                    destination: "d3dcompiler_47.dll".to_string(),
                },
                FileOp::Park {
                    relative_path: "dxgi.dll".to_string(),
                },
            ]
        );

        let is_backup = |name: &str| {
            name.ends_with(".disabled-bak")
                || name.ends_with(".eso-orig-bak")
                || name.ends_with(".bak")
                || name.ends_with(".orig")
        };
        for op in &ops {
            match op {
                FileOp::Park { relative_path } | FileOp::Unpark { relative_path } => {
                    assert!(!is_backup(relative_path), "{op:?}");
                }
                FileOp::RestoreInPlace { destination, .. } => {
                    assert!(!is_backup(destination), "{op:?}");
                }
                FileOp::RemoveRestored { relative_path, .. } => {
                    assert!(!is_backup(relative_path), "{op:?}");
                }
            }
        }
    }

    #[test]
    fn enable_unparks_the_injector_first_then_removes_stock_before_unparking_each_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        real_install(tmp.path());
        std::fs::rename(
            tmp.path().join("dxgi.dll"),
            tmp.path().join("dxgi.dll.kalpa-off"),
        )
        .unwrap();
        std::fs::rename(
            tmp.path().join("nvngx_dlss.dll"),
            tmp.path().join("nvngx_dlss.dll.kalpa-off"),
        )
        .unwrap();
        std::fs::copy(
            tmp.path().join("nvngx_dlss.dll.disabled-bak"),
            tmp.path().join("nvngx_dlss.dll"),
        )
        .unwrap();
        std::fs::rename(
            tmp.path().join("d3dcompiler_47.dll"),
            tmp.path().join("d3dcompiler_47.dll.kalpa-off"),
        )
        .unwrap();
        std::fs::copy(
            tmp.path().join("d3dcompiler_47.dll.eso-orig-bak"),
            tmp.path().join("d3dcompiler_47.dll"),
        )
        .unwrap();

        let stack = inspect_stack(tmp.path());
        assert!(stack.is_disabled);

        let managed = vec![
            managed_file("dxgi.dll", ManagedKind::ReShadeCore, None, true),
            managed_file(
                "nvngx_dlss.dll",
                ManagedKind::NvidiaRuntime,
                Some("nvngx_dlss.dll.disabled-bak"),
                true,
            ),
            managed_file("nvngx_dlssnr.dll", ManagedKind::NvidiaRuntime, None, false),
            managed_file(
                "d3dcompiler_47.dll",
                ManagedKind::ShaderCompiler,
                Some("d3dcompiler_47.dll.eso-orig-bak"),
                true,
            ),
            managed_file("renodx-dlss5.addon64", ManagedKind::Addon, None, false),
        ];

        let plan = plan_toggle(&stack, &managed, &stack.client_dir);
        assert_eq!(plan.action, ToggleAction::Enable);
        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);

        assert_eq!(plan.operations[0].kind, ToggleOpKind::Unpark);
        assert_eq!(plan.operations[0].file_name, "dxgi.dll");

        let ops = to_file_ops(&plan).expect("every planned step must carry its partner");
        assert_eq!(
            ops,
            vec![
                FileOp::Unpark {
                    relative_path: "dxgi.dll".to_string(),
                },
                // Injector first; everything else in name order, so the plan
                // the user approves is stable between reads rather than
                // following whatever order the manifest happened to be in.
                FileOp::RemoveRestored {
                    relative_path: "d3dcompiler_47.dll".to_string(),
                    must_match: "d3dcompiler_47.dll.eso-orig-bak".to_string(),
                },
                FileOp::Unpark {
                    relative_path: "d3dcompiler_47.dll".to_string(),
                },
                FileOp::RemoveRestored {
                    relative_path: "nvngx_dlss.dll".to_string(),
                    must_match: "nvngx_dlss.dll.disabled-bak".to_string(),
                },
                FileOp::Unpark {
                    relative_path: "nvngx_dlss.dll".to_string(),
                },
            ]
        );
    }

    /// Disable, then enable, driven through the real batch runner. The
    /// directory must end up byte-identical to how it started.
    #[test]
    fn round_trip_disable_then_enable_restores_the_folder_exactly() {
        let root = tempfile::tempdir().unwrap();
        let client = root.path().join("client");
        let manifest = root.path().join("client-managed.json");
        let backups = root.path().join("client-backups");
        std::fs::create_dir_all(&client).unwrap();
        std::fs::create_dir_all(&backups).unwrap();
        real_install(&client);

        crate::client_backup::record_adopted(&manifest, &client, managed_stack())
            .expect("record adoption");

        let before = snapshot(&client);
        let approved = ApprovedRoot::for_tests_idle(client.clone());

        // Disable.
        let stack = inspect_stack(&client);
        let managed = managed_entries(&manifest, &client);
        let plan = plan_toggle(&stack, &managed, &stack.client_dir);
        assert_eq!(plan.action, ToggleAction::Disable);
        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
        let ops = to_file_ops(&plan).expect("every planned step must carry its partner");
        crate::client_backup::run_file_ops_in(&manifest, &backups, &approved, &ops)
            .expect("disable batch should apply");

        let stack = inspect_stack(&client);
        assert!(stack.is_disabled);

        // Enable.
        let managed = managed_entries(&manifest, &client);
        let plan = plan_toggle(&stack, &managed, &stack.client_dir);
        assert_eq!(plan.action, ToggleAction::Enable);
        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
        let ops = to_file_ops(&plan).expect("every planned step must carry its partner");
        crate::client_backup::run_file_ops_in(&manifest, &backups, &approved, &ops)
            .expect("enable batch should apply");

        assert_eq!(
            before,
            snapshot(&client),
            "the folder must be byte-identical after a full round trip"
        );
    }

    /// The DLL search order loads `dxgi.dll` and `d3d11.dll` alike, so a
    /// folder carrying both under management has two doors into ReShade.
    /// Parking one and calling the stack off would be a false statement about
    /// the thing the switch is for — `is_disabled` only asks whether *an*
    /// injector name is parked.
    #[test]
    fn disable_parks_every_managed_injector_not_just_the_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        real_install(dir.path());
        write(dir.path(), "d3d11.dll", "the other reshade");

        let stack = inspect_stack(dir.path());
        let mut managed = managed_stack();
        managed.push(managed_file(
            "d3d11.dll",
            ManagedKind::ReShadeCore,
            None,
            false,
        ));
        managed.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

        let plan = plan_toggle(&stack, &managed, &stack.client_dir);
        assert_eq!(plan.action, ToggleAction::Disable);
        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);

        let parked: Vec<&str> = plan
            .operations
            .iter()
            .filter(|op| op.kind == ToggleOpKind::Park)
            .map(|op| op.file_name.as_str())
            .collect();
        assert!(parked.contains(&"dxgi.dll"), "{parked:?}");
        assert!(parked.contains(&"d3d11.dll"), "{parked:?}");
    }

    #[test]
    fn disable_blocks_when_there_is_no_injector() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "eso64.exe", "game");
        write(tmp.path(), "nvngx_dlss.dll", "modded");
        write(tmp.path(), "nvngx_dlss.dll.disabled-bak", "stock");
        let stack = inspect_stack(tmp.path());
        let managed = vec![managed_file(
            "nvngx_dlss.dll",
            ManagedKind::NvidiaRuntime,
            Some("nvngx_dlss.dll.disabled-bak"),
            false,
        )];

        let plan = plan_toggle(&stack, &managed, &stack.client_dir);

        assert_eq!(plan.action, ToggleAction::Disable);
        assert!(
            plan.blockers
                .iter()
                .any(|blocker| blocker.to_lowercase().contains("injector")),
            "{:?}",
            plan.blockers
        );
    }

    #[test]
    fn disable_blocks_when_a_runtime_has_no_preserved_original() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "eso64.exe", "game");
        write(tmp.path(), "dxgi.dll", "reshade");
        write(tmp.path(), "nvngx_dlss.dll", "modded, no backup anywhere");
        let stack = inspect_stack(tmp.path());
        let managed = vec![
            managed_file("dxgi.dll", ManagedKind::ReShadeCore, None, false),
            managed_file("nvngx_dlss.dll", ManagedKind::NvidiaRuntime, None, false),
        ];

        let plan = plan_toggle(&stack, &managed, &stack.client_dir);

        assert!(
            plan.blockers
                .iter()
                .any(|blocker| blocker.contains("nvngx_dlss.dll")),
            "{:?}",
            plan.blockers
        );
    }

    #[test]
    fn disable_blocks_when_nothing_is_managed() {
        let tmp = tempfile::tempdir().unwrap();
        real_install(tmp.path());
        let stack = inspect_stack(tmp.path());

        let plan = plan_toggle(&stack, &[], &stack.client_dir);

        assert_eq!(plan.action, ToggleAction::Disable);
        assert!(!plan.blockers.is_empty());
        assert!(plan.operations.is_empty());
    }

    #[test]
    fn a_deleted_parked_copy_is_reported_without_refusing_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        real_install(tmp.path());
        std::fs::rename(
            tmp.path().join("dxgi.dll"),
            tmp.path().join("dxgi.dll.kalpa-off"),
        )
        .unwrap();
        std::fs::rename(
            tmp.path().join("nvngx_dlss.dll"),
            tmp.path().join("nvngx_dlss.dll.kalpa-off"),
        )
        .unwrap();
        std::fs::copy(
            tmp.path().join("nvngx_dlss.dll.disabled-bak"),
            tmp.path().join("nvngx_dlss.dll"),
        )
        .unwrap();
        // The parked copy then vanishes out from under Kalpa.
        std::fs::remove_file(tmp.path().join("nvngx_dlss.dll.kalpa-off")).unwrap();

        let stack = inspect_stack(tmp.path());
        assert!(stack.is_disabled);

        let managed = vec![
            managed_file("dxgi.dll", ManagedKind::ReShadeCore, None, true),
            managed_file(
                "nvngx_dlss.dll",
                ManagedKind::NvidiaRuntime,
                Some("nvngx_dlss.dll.disabled-bak"),
                true,
            ),
        ];

        let plan = plan_toggle(&stack, &managed, &stack.client_dir);

        assert_eq!(plan.action, ToggleAction::Enable);
        // One file being gone must not refuse to put the rest of the stack
        // back — that was the dead end this planner exists to avoid.
        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
        assert_eq!(
            to_file_ops(&plan).expect("plan is complete"),
            vec![FileOp::Unpark {
                relative_path: "dxgi.dll".to_string(),
            }],
            "the injector still goes back"
        );
        // But the missing parked copy is still named rather than silently
        // dropped — something holds the live name, and Kalpa cannot say it is
        // the file it moved aside.
        assert!(
            plan.operations.iter().any(|op| {
                op.kind == ToggleOpKind::LeaveInPlace
                    && op.file_name == "nvngx_dlss.dll"
                    && op.summary.contains("No parked copy")
            }),
            "{:?}",
            plan.operations
        );
    }

    /// The state a user creates by "fixing it themselves": they rename
    /// `x.dll.kalpa-off` back by hand, so the folder is correct and only Kalpa's
    /// record is stale. Believing the record would refuse to switch off a stack
    /// that is working, with nothing in the app able to clear the flag.
    #[test]
    fn a_stale_parked_record_does_not_wedge_a_folder_that_is_fine() {
        let tmp = tempfile::tempdir().unwrap();
        real_install(tmp.path());
        let stack = inspect_stack(tmp.path());
        assert!(stack.parked.is_empty(), "nothing is parked on disk");

        // The manifest still claims the injector is parked.
        let managed = vec![
            managed_file("dxgi.dll", ManagedKind::ReShadeCore, None, true),
            managed_file(
                "nvngx_dlss.dll",
                ManagedKind::NvidiaRuntime,
                Some("nvngx_dlss.dll.disabled-bak"),
                false,
            ),
        ];
        let plan = plan_toggle(&stack, &managed, "client");

        assert_eq!(plan.action, ToggleAction::Disable);
        assert!(
            plan.blockers.is_empty(),
            "a stale flag must not block a working folder: {:?}",
            plan.blockers
        );
        assert!(
            plan.operations
                .iter()
                .any(|op| { op.kind == ToggleOpKind::Park && op.file_name == "dxgi.dll" }),
            "the injector is live, so it is parked like any other: {:?}",
            plan.operations
        );
    }
}
