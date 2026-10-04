# Fable Consultation — R6 Crash-Safe Installer Transaction

The master prompt forbids implementing an ad hoc directory swap without this
review.

## Finding

Existing addon folders are modified **in place**. A crash during copy can leave
truncated files that later appear to be user edits, and can become a permanent
keep-mine baseline in the Protected Edits system.

## Current write path — `src-tauri/src/installer.rs:443`

No staging, no rename, no fsync. `File::create` truncates the user's file to
zero, then bytes stream in:

```rust
if entry.is_dir() {
    fs::create_dir_all(&out_path).map_err(|e| describe_write_error(&out_path, &e))?;
} else {
    let declared_size = entry.size();
    if total_extracted + declared_size > MAX_EXTRACT_SIZE { ... }
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).map_err(|e| describe_write_error(parent, &e))?;
    }
    let mut outfile = fs::File::create(&out_path)          // O_TRUNC on the LIVE file
        .map_err(|e| describe_write_error(&out_path, &e))?;
    let bytes_written = io::copy(&mut entry, &mut outfile)
        .map_err(|e| describe_extract_error(&out_path, &e))?;
    total_extracted += bytes_written;
}
```

Entries are iterated in raw central-directory order (`installer.rs:384`), so a
multi-folder archive is written **interleaved**; nothing groups by folder. The
returned folder set is a `HashSet` (`installer.rs:486`), so order is
nondeterministic.

## Current rollback — `installer.rs:120`, fresh-install only

```rust
let mut pre_existing: HashSet<String> = HashSet::new();
for folder in &top_level {
    if addons_dir.join(folder).is_dir() { pre_existing.insert(folder.clone()); }
}
let result = extract_addon_zip_inner(&mut archive, addons_dir, skip_files, hooks, wrap_name);
if let Err(ref err_msg) = result {
    for folder in &top_level {
        if !pre_existing.contains(folder) {          // updates are deliberately skipped
            let folder_path = addons_dir.join(folder);
            if folder_path.is_dir() { let _ = fs::remove_dir_all(&folder_path); }
        }
    }
}
```

This is in-process and error-path only. **A hard crash runs no rollback at all,
and no startup pass inspects `addons_dir`.**

`installer.rs:951`, `cancel_midway_preserves_pre_existing_addon_files`, asserts
the current in-place semantics as a requirement ("no file is removed, only some
are overwritten"). Any transaction supersedes that test rather than preserving it.

## The exact corruption chain

1. Crash mid-`io::copy` leaves a truncated file; the **old**
   `.kalpa-hashes/<Folder>.json` is still in place (promotion never ran).
2. Next scan, `detect_modifications` (`file_hashes.rs:434`) compares disk to the
   stale baseline, flags the truncated file as modified, and **persists that**
   (`file_hashes.rs:472`): `manifest.modified_files = modified.clone();
   save_hash_manifest(...)?`. Corruption is now recorded as a user edit.
3. Next update it surfaces as a conflict. Keep-mine puts it in `hash_overrides`,
   and `write_folder_manifest` (`file_hashes.rs:653`) inserts the **upstream**
   hash as the baseline while the truncated bytes stay on disk:

```rust
if let Some(overrides) = hash_overrides {
    for (path, upstream_hash) in overrides {
        files.insert(path.clone(), upstream_hash.clone());
    }
}
```

The file is now permanently divergent-and-blessed, re-flagged every update, with
no repair path.

Secondary hazard: `compute_baseline_with_zip` (`file_hashes.rs:292`) assumes
extraction wrote the ZIP's bytes verbatim and **does not read covered files from
disk**. A silent short write records a baseline for bytes that are not there.

## Available building blocks

`src-tauri/src/atomic_file.rs` — **exists only on `fix/audit-p0-a1-atomic-writer`
(#380), not on `main`.** R6 stacks on it. Single-*file* API:

```rust
pub const STAGING_INFIX: &str = ".tmp-";
pub struct AtomicFile { /* create_new staging, 16 unique-name attempts */ }
impl AtomicFile {
    pub fn create(target: &Path) -> io::Result<Self>;
    pub fn commit(self) -> io::Result<()>;
    /// flush -> sync_all -> drop handle -> before_rename(staging) -> rename -> parent fsync
    pub fn commit_with(self, before_rename: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()>;
}
impl Write for AtomicFile {} impl Read for AtomicFile {} impl Seek for AtomicFile {}
impl Drop for AtomicFile { /* removes only its own staging path */ }
pub fn atomic_write(target: &Path, bytes: &[u8]) -> io::Result<()>;

const RENAME_ATTEMPTS: usize = 5;
const RENAME_BACKOFF: Duration = Duration::from_millis(40);
fn is_transient_rename_error(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied
        || matches!(e.raw_os_error(), Some(5 | 32 | 33))   // ACCESS_DENIED/SHARING/LOCK
}
```

Its own docs note two limits that a directory transaction inherits: it does not
serialize cross-process read-modify-write (that is P0-A2's
`transaction_lock.rs`), and Windows has no portable directory fsync, so power
loss may lose the rename and make the old complete file reappear — "old-or-new,
never torn".

**The one existing directory-level swap** is the character-backup finalizer,
`commands.rs:6746`, and it is the closest precedent:

```rust
fn finalize_backup_replace(staging: &Path, final_dir: &Path, tombstone: &Path)
    -> std::io::Result<()> {
    let _guard = BACKUP_FINALIZE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let had_previous = final_dir.exists();
    if had_previous {
        let _ = fs::remove_dir_all(tombstone);
        fs::rename(final_dir, tombstone)?;
    }
    match fs::rename(staging, final_dir) {
        Ok(()) => { if had_previous { let _ = fs::remove_dir_all(tombstone); } Ok(()) }
        Err(e) => {
            if had_previous {
                match fs::rename(tombstone, final_dir) {
                    Ok(()) => { let _ = fs::remove_dir_all(staging); }
                    Err(_) => return Err(e),
                }
            } else { let _ = fs::remove_dir_all(staging); }
            Err(e)
        }
    }
}
```

Its startup recovery, `recover_orphaned_backups` (`commands.rs:6799`), uses
**staging existence as the crash proof** rather than a journal, with a
three-way discriminator: `final_dir` exists → replacement landed, drop tombstone;
`final_dir` absent and staging present → true mid-finalize crash, restore
tombstone; `final_dir` absent and staging absent → user deleted it later, never
resurrect. It is called at operation entry (`commands.rs:4973`, `:5611`,
`:7071`), **not** from app setup.

## Ordering constraint that binds the design

Protected Edits backups (`edit_backups.rs:66`) `fs::copy` from the **live**
addon folder and always run **before** extraction (`commands.rs:3055`, `:3455`;
Slint `main.rs:12881`, `:13109`). A staging/swap design must keep the backup
capturing the pre-image. `.kalpa-backups/` and `.kalpa-hashes/` live *beside*
the addon folders under `addons_dir`, not inside them, so a per-addon-folder
swap is layout-safe.

## Windows reality

`installer.rs` has **zero** retry loops. A `File::create` that fails because ESO,
a text editor, or an AV scanner holds a `.lua` open is a hard error with no
retry — after earlier files were already overwritten. Only diagnostics exist
(`describe_write_error`, `installer.rs:17`, which explains Controlled Folder
Access). Project history: CFA silently blocks writes under `Documents` for both
Kalpa and `eso64.exe`.

Also: `fs::rename` on Windows replaces atomically via `MoveFileExW`, but in the
`.tmp` promotion path the primary must be removed first because rename cannot
overwrite (`metadata.rs:71`).

## Repository constraints

- The Slint sidecar **shares `installer.rs`, `file_hashes.rs`, `edit_backups.rs`
  and `metadata.rs` verbatim** via `#[path]` (`main.rs:17`), so fixing
  `installer.rs` fixes both binaries. But process-wide statics are per-process:
  a publish lock or transaction registry introduced here is **not** shared
  between the two binaries, which can run concurrently against one AddOns
  folder. Cross-process serialization must ride on P0-A2, not a third mechanism.
- No new dependency without maintainer approval.
- `MAX_EXTRACT_SIZE` zip-bomb guard and the symlink skip
  (`mode & 0o170000 == 0o120000`) must survive unchanged.
- Traversal containment is proved by `enclosed_name` plus
  `enclosed_top_component_matches_enclosed_name_first_component`
  (`installer.rs:535`); any new staging path scheme must re-prove it.
- Disk cost matters: staging a full copy doubles peak usage for large addons.

## Acceptance criteria

- Interrupted extraction cannot leave a partially committed addon presented as healthy.
- Original files remain recoverable until commit.
- A failed multi-folder install has explicit rollback semantics.
- Hash baselines are promoted only after successful commit.
- Keep-mine cannot bless a known incomplete installation.
- Recovery after process restart is deterministic.
- Existing protected-edit backups remain compatible.

## Candidate designs

**A — Per-folder staging + swap.** Extract each top-level folder into
`<addons_dir>/.kalpa-staging/<Folder>-<txn>/`, then `finalize_backup_replace`-
style swap per folder with a tombstone. Needs a merge step, because selective
extraction (keep-mine) intentionally leaves some files from the *old* folder.

**B — Journal + in-place with per-file atomic publish.** Keep writing into the
live folder but publish every file through `AtomicFile`, guarded by an on-disk
transaction journal listing intended targets, with startup recovery.

**C — Copy-on-write shadow.** Hard-link/copy the existing folder into staging,
apply the update there, then swap. Preserves keep-mine files naturally.

## Failure modes to evaluate

1. Crash between committing folder 1 and folder 2 of a multi-folder archive.
2. A mixed archive where `LibFoo` pre-exists and `MainAddon` is new: today a
   failure deletes `MainAddon` and leaves `LibFoo` half-overwritten.
3. Selective extraction: staging must reproduce kept-mine files or the swap
   deletes user edits that keep-mine promised to preserve.
4. ESO holds a `.lua` open during the swap on Windows.
5. Power loss after the staging rename but before the baseline promotion.
6. Abandoned staging directories from a previous crash, including a partially
   swapped tombstone. Where does recovery run, given no startup pass currently
   touches `addons_dir`?
7. Both binaries running concurrently, with per-process statics.
8. Disk exhaustion mid-staging for a large addon.
9. `compute_baseline_with_zip` never reads covered files from disk. Does the
   chosen design keep that optimization sound, or must it verify?

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
