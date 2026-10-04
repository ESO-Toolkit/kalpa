# Fable Consultation — R5 Folder-Qualified Conflict Protection

## Finding, restated precisely after inventory

The audit text says sibling folder entries "never participate" in conflict
classification, backups, selective extraction, or diffs. Inventory shows the
shape is narrower and more specific, and this changes the fix:

**Storage is already per-folder and works.** `record_hashes_with_zip_baseline`
(`file_hashes.rs:568`) re-calls `hash_zip_entries(zip_path, folder)` for each
secondary folder and writes each its own `.kalpa-hashes/<Folder>.json`. Siblings
*have* baselines, and `detect_modifications` flags their edits for the file
browser (`load_modified_file_count`, `file_hashes.rs:407`).

**The conflict pipeline is single-folder-scoped.** `build_conflict_report`
(`commands.rs:2462`) takes exactly one `folder_name` — the primary — so
classification, backups, skip keys, and diffs only ever see primary-folder
entries.

Net user-visible bug: a modified file in a bundled sibling has a baseline, is
even displayed as "modified" in the file browser, and is then **silently
overwritten on update with no prompt and no backup.**

## Root of the bug

`file_hashes.rs:324` — every entry outside `folder_name` is dropped:

```rust
// A FLAT archive has no `<folder>/` prefix to strip - the folder only comes
// into existence when `extract_addon_zip` re-roots the entries under it.
let is_flat = crate::installer::flat_archive_wrap_name(&archive).as_deref() == Some(folder_name);
let prefix = format!("{folder_name}/");
...
    if let Some(mode) = entry.unix_mode() {
        if mode & 0o170000 == 0o120000 { continue; }        // symlink skip
    }
    let name = match entry.enclosed_name() {
        Some(p) => p.to_string_lossy().replace('\\', "/"),
        None => continue,
    };
    let relative = if is_flat {
        name.clone()
    } else {
        match name.strip_prefix(&prefix) {
            Some(r) if !r.is_empty() => r.to_string(),
            _ => continue,          // <-- EVERY SIBLING FOLDER ENTRY DIES HERE
        }
    };
```

## Key format per layer, as it exists today

| Layer | file:line | Current key | Folder-qualified? |
|---|---|---|---|
| ZIP entry classification | `file_hashes.rs:315` | `relative/path.lua` | No |
| Stored hash manifests | `file_hashes.rs:27`, path `:47` | `relative/path.lua`, one JSON **file per folder** | Implicitly |
| Pending conflict state | `lib.rs:86` `PendingUpdate` (`folder_name:88`, `zip_hashes:101`) | single folder | No |
| Batch conflict state | `commands.rs:2555`, `:2565` | `conflicts[].relativePath` bare | No |
| Conflict diff generation | `commands.rs:3206`; ZIP side `:3260` | re-qualified as `format!("{folder_name}/{relative_path}")` using the **pending** folder | Wrong folder for siblings |
| Keep-mine / take-update apply | `commands.rs:3293` `FileDecision`, apply `:3379` | bare, folder implied `pu.folder_name` | No |
| Edit backups | `edit_backups.rs:43`, dest root `:56` | `relative/path.lua` under `.kalpa-backups/<folder>/<ts>/` | Implicitly |
| **Selective-extraction skip keys** | built `commands.rs:3428`, consumed `installer.rs:425` | **`Folder/relative/path.lua`** | **YES** |
| Metadata recording | `metadata.rs:222`, driver `commands.rs:232` | folder names only, no file paths | n/a |
| Tauri structs | `commands.rs:2340` `FileConflict`, `:2348` `ConflictReport`, `:3197` `DiffData` | bare `relativePath`, separate `folderName` scalar | No |
| Frontend types | `src/types.ts:585-678` | bare | No |
| Slint orchestration | `main.rs:617`, `:12856`, `:12936`, `:13232`, `:14485` | same as Tauri | No |

Two consequences worth weighing:

- **The skip-key layer is already folder-qualified and its consumer already
  handles both shapes** (`installer.rs:425`):

```rust
let key = enclosed.to_string_lossy().replace('\\', "/");
let wrapped_key = wrap_name.map(|name| format!("{name}/{key}"));
if skip_files.contains(&key)
    || wrapped_key.as_deref().is_some_and(|k| skip_files.contains(k))
{ continue; }
```
  Only the *producer* hardcodes the primary: `format!("{}/{}", pu.folder_name, p)`.

- **Manifests and backups are directory-partitioned by folder**, so they need
  caller-side grouping, not a key-format change.

So the question is whether to make keys folder-qualified *globally*, or to lift
the pipeline into a loop over folders and keep per-folder keys.

## Flat archives

`flat_archive_wrap_name` (`installer.rs:258`) returns the synthesized folder for
an archive with no top-level directory: `contains_foldered_addon` short-circuits
to `None`; otherwise exactly one root `.txt` stem wins, with a
`has_sibling_with_stem` tiebreak, then `sanitize_wrap_name` rejects
`.`/`..`/empty/`/`/`\`/`\0`/`:`. Applied at `installer.rs:414` *after*
`enclosed_name`, so containment is unaffected. Hashing mirrors it via `is_flat`
so flat keys come out unprefixed.

A flat archive is by definition single-folder and so is not affected by the
sibling bug. But if keys become `Folder/rel` globally, `is_flat` must synthesize
the wrap name into the key, and flat becomes the one case where the ZIP entry
name and the key differ. Two tests encode this contract:
`file_hashes.rs:931` and `installer.rs:765`.

## Symlinks

Skipped in lockstep at `installer.rs:394` and `file_hashes.rs:345`; disk walks
skip via `symlink_metadata` (`file_hashes.rs:225`). `entry.unix_mode()` returns
`None` for Windows-created ZIPs so the guard is a no-op there. Preserve verbatim.

## Latent collision already present

`update-conflict-panel.tsx:35` keys `decisions` by bare `relativePath`. Two
siblings shipping `init.lua` collide today. `addon-file-browser.tsx:215` joins
`addonsPath + folderName + path` using the *pending* folder.

## Repository constraints

- `commands.rs:3412` deliberately re-derives classification server-side rather
  than trusting client `decisions`. A multi-folder version must preserve that,
  which means the re-derive loop must enumerate folders from the ZIP rather than
  from `pu.folder_name`.
- Wire-format changes need paired Rust + frontend updates and tests.
  `validate_relative_path` (`commands.rs:3688`) already accepts `Folder/rel`.
- Slint shares `file_hashes.rs`/`installer.rs`/`edit_backups.rs`/`metadata.rs`
  by `#[path]`, so those edits are free there; `main.rs` orchestration
  (~`:12856-13260`, `:14485`) must be mirrored, plus its **private copy** of
  manifest logic for the editor at `main.rs:686` / `:17905`.
- `hash_overrides` (`file_hashes.rs:646`) currently assumes primary-folder paths
  and must be split per folder.
- **No frontend or e2e test covers conflicts at all.**
- `file_hashes.rs:912` `hash_zip_ignores_other_folders` asserts
  `hashes.len() == 1` for a two-folder ZIP — it **encodes the bug** and must be
  rewritten, not preserved.

## Acceptance criteria

- Every top-level folder touched by an archive is classified independently.
- Modified sibling files produce conflicts.
- Keep-mine preserves the correct folder-qualified file.
- Take-update backs up the correct folder-qualified file.
- Unmodified sibling content still updates.
- Flat archives remain correctly wrapped and classified.
- Wire-format changes receive paired Rust and frontend updates and tests.

## Candidate designs

**A — Globally folder-qualified keys.** Make every key `Folder/relative/path`
end to end, matching the skip-key layer that already works. Largest wire change;
flat archives become the special case; manifests keyed by a path that repeats
the folder already implied by the filename.

**B — Lift the pipeline to a loop over folders.** Keep per-folder keys
everywhere; change `ConflictReport`/`PendingUpdate`/`FileDecision` to carry a
folder alongside the path, and iterate the archive's top-level folders. Smaller
key churn; more structural churn in the report/decision types.

**C — Hybrid.** Folder-qualify only the cross-folder plumbing (report entries,
decisions, skip-key producer, diff lookup, backup grouping) while leaving
per-folder storage keys untouched.

## Failure modes to evaluate

1. Same relative filename in primary and sibling (`init.lua` in both) —
   decisions map collision, backup path collision, diff lookup ambiguity.
2. A flat archive under the chosen key format, where ZIP entry name and key
   diverge.
3. A user-added sibling file that upstream begins shipping.
4. A deleted sibling file, and the kept-deletion override path
   (`file_hashes.rs:1093`, `:1426`).
5. Batch update vs single-addon update, which have separate apply paths
   (`commands.rs:3035` vs `:3379`).
6. Migration: existing `.kalpa-hashes/<Folder>.json` files written with the old
   key format must keep loading.
7. Slint's private `NativeHashManifest` copy diverging from the shared module.
8. A sibling folder that is *also* separately tracked — the R4 interaction.

## Required output

```text
DECISION:
1. Chosen design and numbered implementation steps

REJECTED:
1. Alternative and the concrete failure that rejects it

CRASH_RECOVERY:
1. Behavior after process kill, power loss, stale marker, timeout, or partial write

TESTS:
1. Tests distinguishing a correct design from a plausible but incorrect one

RISKS:
1. Remaining risks and required human decisions
```
