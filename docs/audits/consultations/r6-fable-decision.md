Verified against the tree: the write loop (`installer.rs:384-483`), the update path ordering (`commands.rs:3455` backup → `:3475` extract → `:3487` baseline promotion → metadata), `compute_baseline_with_zip` (`file_hashes.rs:292`), `detect_modifications` persisting its result (`file_hashes.rs:472`), and P0-A2's `transaction_lock.rs` (`.kalpa.lock` suffix, 2s default timeout). One fact the brief omits that shapes the decision: **updates never delete files** — files removed upstream and user-added files linger in the live folder and get recorded by the disk walk in `compute_baseline_with_zip`. Any swap design must reproduce that or it silently changes update semantics.

```text
DECISION:
Chosen: C-lite — per-folder staging that is a MERGE of (fresh ZIP bytes) + (residual
copy of every live file the ZIP does not cover), swapped with a tombstone under an
on-disk transaction journal. This is candidate A with its merge step made mandatory
and candidate C with the "copy the whole folder first" removed; they converge, and
the residual copy is exactly what keep-mine, lingering-upstream-removed, and
user-added files need. Peak disk = new version + residual (not 2× the addon).

1. New shared module `src-tauri/src/install_txn.rs`, added to the Slint `#[path]`
   list alongside installer.rs. Layout, all under `<addons_dir>/.kalpa-staging/`:
     <txn>/journal.json                 phase + folder list + hash-manifest paths
     <txn>/stage/<Folder>/...           new folder image
     <txn>/tombstone/<Folder>/...       old folder during swap
     <txn>/hashes/<Folder>.json         baseline to promote
   Everything a crash can leave behind lives in ONE dot-directory, so scan needs
   one exclusion rule and recovery needs one directory listing. <txn> = pid +
   monotonic counter + random suffix; never a timestamp alone (two binaries).
2. Acquire the P0-A2 transaction lock on addons_dir for the whole txn
   (stage + swap + promote + cleanup). Run `recover_staging(addons_dir)` first,
   under the same lock (step 9). No new process-wide statics.
3. Validate every top-level name from `collect_zip_top_folders`/wrap_name as a
   single path component (reuse the `remove_addon` checks) AND reject names
   starting with `.kalpa-`. Today nothing stops a ZIP whose top folder is
   `.kalpa-hashes`. The staging target is
   `stage/<Folder>/<enclosed-minus-first-component>`; `enclosed_name` runs
   unchanged before the join, so containment is the existing proof plus "first
   component is a validated single segment" — extend
   `enclosed_top_component_matches_enclosed_name_first_component` to assert the
   stage path's ancestor is `stage/<Folder>`.
4. Extraction loop (`extract_addon_zip_inner`) keeps its body — MAX_EXTRACT_SIZE,
   symlink skip, skip_files matching, cancel/progress — but `out_path` is the
   stage path. Two additions per file: assert `bytes_written == entry.size()`
   (the zip crate already CRC-checks on EOF) and `outfile.sync_all()`. Staging
   files are never live, so no per-file AtomicFile is needed there; the fsync is
   what makes the later rename old-or-new. Error → remove `<txn>/` and return;
   the live folder was never touched.
5. Residual merge, per folder that pre-exists: `walk_addon_files(live)`; for
   every key absent from `stage/<Folder>`, copy live → stage (try
   `fs::hard_link`, fall back to `fs::copy`; hard links are safe ONLY because
   nothing writes into stage after this point — document that invariant).
   This reproduces keep-mine, upstream-removed lingering files, and user-added
   files exactly as the in-place path preserves them today. Skip if the live
   folder is a symlink/junction (see RISKS 1).
6. Baseline: run `compute_baseline_with_zip(stage/<Folder>, zip_hashes)` +
   `write_folder_manifest` against `<txn>/hashes/<Folder>.json` (add an
   output-path parameter; today it writes to `.kalpa-hashes` directly). The
   disk walk now sees the merged stage image, so "keys only for files that
   exist" stays true and the ZIP-hash shortcut stays sound because step 4
   verified byte counts. Journal phase → `staged` (atomic_write + fsync).
7. Swap, sequential, per folder, `finalize_backup_replace` shape with the
   AtomicFile retry loop (5×40ms on 5/32/33/PermissionDenied) around BOTH
   renames: live → tombstone, stage → live. Do NOT delete tombstones yet.
   On any rename failure: restore every already-swapped folder from its
   tombstone (reverse order), then remove `<txn>/`, then return the error
   with the "close ESO / editor" hint. Journal phase → `swapped` only after
   every folder landed.
8. Promote: `atomic_write` each `<txn>/hashes/<Folder>.json` →
   `.kalpa-hashes/<Folder>.json`; phase → `promoted`; then remove_dir_all
   `<txn>/`. Metadata save (`commands.rs:3497`) happens after this, as today.
   Keep-mine cannot bless a partial install because the only baseline that
   ever reaches `.kalpa-hashes` was computed from a fully-verified stage image
   and written after the swap completed.
9. `recover_staging` (install_txn.rs), called at the entry of scan, install,
   update, batch-update, remove, profile-apply, restore — anywhere that reads
   `.kalpa-hashes` or writes addon folders; NOT from app setup. Scan is the
   mandatory site: it is the call that would turn a torn folder into a persisted
   "user edit" (`file_hashes.rs:472`). Semantics in CRASH_RECOVERY.
10. `scan`/`list` must skip `.kalpa-staging` (verify the existing dot-dir filter
    covers it; if the filter is name-specific, add it).
11. Delete the in-process rollback in `extract_with_rollback` (`installer.rs:158`)
    and rewrite `cancel_midway_preserves_pre_existing_addon_files` to assert the
    stronger property: after cancel, the pre-existing folder is byte-identical
    and `.kalpa-staging` is gone.

REJECTED:
1. B — journal + in-place per-file AtomicFile. Each file is old-or-new, but the
   FOLDER is torn: crash after 3 of 10 files leaves a mixed-version addon
   (version-3 Main.lua loading version-2 Lib.lua). The baseline is still stale,
   so the 3 new files are flagged modified and persisted as edits at next scan;
   keep-mine then blesses them. Same corruption chain, no truncation needed.
   Rolling back requires a pre-image of every replaced file — the same bytes C-lite
   stages, at worse atomicity. Also cannot survive failure mode 4: file 4 open in
   ESO fails after 1-3 were already published.
2. Pure A (swap without merge). Swapping the fresh extraction over the live folder
   deletes keep-mine files (skipped ⇒ absent from stage), user-added files, and
   upstream-removed lingering files that today survive. Keep-mine's hash override
   would then point at a file that no longer exists → "user deleted" on every
   subsequent update. A-with-merge is the chosen design.
3. Full C (hard-link/copy the entire folder, then extract over it). Extracting
   INTO a hard-linked tree with `File::create` truncates the live inode — the
   exact bug being fixed, now via staging. Avoidable only by unlink-before-write
   on every file, at which point you have C-lite with extra copies. Doubles disk
   for large addons (brief constraint).
4. Sidecar-side or new global registry. Per-process statics don't cross the two
   binaries (brief). Everything cross-process rides on P0-A2's lock + the
   journal on disk.

CRASH_RECOVERY:
recover_staging lists `.kalpa-staging/*`, under the P0-A2 lock, and for each txn
reads journal.json (missing/unparseable ⇒ treat as phase < staged):
1. phase < staged (kill mid-extract, disk full, ZIP error): remove `<txn>/`.
   Live folders untouched by construction. Deterministic rollback.
2. phase == staged (kill anywhere during step 7): per folder, three-way
   discriminator as in recover_orphaned_backups:
     live exists & tombstone exists  → rename landed for this folder? Compare:
       stage exists ⇒ swap not started; stage absent ⇒ swap landed.
     live absent & tombstone exists → mid-swap: rename tombstone → live.
     live absent & tombstone absent → folder was new (no pre-image): nothing.
   Then ROLL BACK: every folder whose stage is absent (landed) gets tombstone →
   live. Rationale: promotion never happened, so the stale baseline is still in
   force; rolling back is the only outcome consistent with it. Then remove
   `<txn>/`. Roll-forward is possible (stage is complete) but requires the same
   swap-retry path on a startup call site — choose rollback for determinism and
   because the user's next click re-runs the update from the still-present ZIP
   cache or a fresh download.
3. phase == swapped (power loss after last swap, before/during promotion):
   ROLL FORWARD — idempotently atomic_write each `<txn>/hashes/*.json` into
   `.kalpa-hashes`, then remove `<txn>/`. This is failure mode 5. Old baseline is
   never consulted for a swapped folder because promotion is re-run before any
   scan can call detect_modifications.
4. phase == promoted: just remove `<txn>/`.
5. Windows power-loss caveat inherited from atomic_file.rs: the parent directory
   rename may be lost, so a folder can be old-complete with the journal at
   `swapped`. Recovery step 3 then promotes a NEW baseline over an OLD folder.
   Guard: in phase `swapped`, before promoting a folder, confirm `stage/<Folder>`
   is absent AND tombstone is present (or was new); if stage still exists the
   rename was lost → treat that folder as phase 2 (roll back), and roll back all.
6. Timeout: the P0-A2 lock times out (2s) if the other binary is mid-txn → return
   "another Kalpa process is installing", no recovery attempted, nothing touched.
7. Partial write that DOES NOT crash (short write / disk full inside io::copy):
   `bytes_written != entry.size()` or the write error ⇒ phase < staged ⇒ path 1.
8. Stale txn from a process that is still alive (pid in journal) — irrelevant:
   the lock, not the pid, decides; a live process holds the lock.

TESTS:
1. Crash-injection hook in ExtractHooks (test-only `fail_after_entry: Option<usize>`
   and `fail_before_swap_of: Option<String>`). Assert: pre-existing folder is
   byte-identical after every injection point < swapped; `.kalpa-hashes` is
   unchanged; `recover_staging` leaves no `.kalpa-staging`.
   A per-file-atomic (B) design fails the byte-identical assertion.
2. Multi-folder: LibFoo pre-exists, MainAddon new; force rename failure on
   MainAddon (Windows: hold a file open in MainAddon's stage path; cross-platform:
   inject). Assert LibFoo restored byte-identical AND MainAddon absent AND stage
   gone. (Failure mode 2 — today's code fails this.)
3. Keep-mine + user-added + upstream-removed: seed folder with edited a.lua
   (kept), user-only z.lua, and old.lua absent from the new ZIP. After update:
   a.lua bytes unchanged, z.lua present, old.lua present, baseline has a.lua =
   upstream hash, and `detect_modifications` returns exactly ["a.lua","z.lua"]
   — same result as the current in-place path on the same fixture. Pure A fails.
4. Simulate phase==swapped with no `.kalpa-hashes` promotion (write the journal
   and layout by hand): `recover_staging` → baseline promoted, folder new, scan
   reports no modifications. Then the failure-mode-5 variant: stage still
   present + journal swapped → assert rollback, not promotion (a design that
   trusts the journal alone fails this).
5. Simulate phase==staged with folder 1 landed and folder 2 not: assert both
   folders at pre-image and `.kalpa-hashes` untouched.
6. Stale-baseline non-blessing: kill mid-extract, run scan → assert
   `modified_files` in the manifest is unchanged from before the update (the
   corruption chain's step 2 must not happen).
7. Windows-only (#[cfg(windows)]): open a .lua in the live folder with default
   share flags, run update → error mentions closing ESO, live folder unchanged,
   no `.kalpa-staging` left. Retry loop test: release the handle after 60ms →
   update succeeds.
8. Traversal: ZIP with top folder `..`, `.kalpa-hashes`, `a/../b`, and an entry
   that escapes via enclosed_name → all rejected before any staging dir exists.
9. Disk-full: mock a writer that errors with ENOSPC at entry N → live untouched,
   error text preserved, staging removed.
10. Cross-process: reuse P0-A2's `process_helper` pattern to hold the lock from a
    child process; assert the parent's install returns the timeout error without
    creating `.kalpa-staging`.
11. Zip-bomb and symlink-skip tests must pass unchanged (they exist; run them).

RISKS:
1. Addon folders that are symlinks/junctions (developers pointing AddOns/Foo at a
   git checkout). In-place writes follow the link; a swap replaces the link with a
   real directory and remove_dir_all on the tombstone removes only the link (std
   does not follow), so the dev checkout survives but the link is gone. DECISION
   NEEDED: fall back to the legacy in-place path for symlinked folders, or refuse
   with a message. Recommend refuse-with-message; document it.
2. Hard-link residual copies share inodes with the tombstone until the tombstone
   is deleted; anything that opens stage files for write during that window
   corrupts the live pre-image. Invariant is easy to state, easy to violate in a
   future refactor. If uncomfortable, use fs::copy only (cost: residual size,
   usually small).
3. Per-file sync_all adds ~1-5ms per file on Windows; a 2,000-file addon adds a
   few seconds. Acceptable vs. correctness; could batch (write all, then fsync
   all) if it shows up in the ~2min-update complaints.
4. Rename of a directory on Windows fails if ANY file inside is open without
   FILE_SHARE_DELETE (ESO keeps SavedVariables open, not addon files, but
   editors/AV do). Behaviour becomes "clean refusal" instead of "half-overwrite",
   which is strictly better, but users will see a new error where they used to see
   a (corrupting) success. Needs the CFA-style explanatory message.
5. Rollback-on-recovery (phase staged) discards a complete stage the user paid to
   download. Deliberate; if the download cache is not retained, the user re-downloads.
6. Stacking: depends on #380 (atomic_file) and P0-A2 (transaction_lock) both
   landing first; this branch cannot merge independently.
7. The e2e sandbox has no crash-injection; the destructive assertions above are
   Rust unit tests only. Nothing in CI exercises WebView2 — unchanged.
8. Backups (`edit_backups`) still copy from the live folder before staging, so the
   pre-image is captured; unchanged. But `backup_user_files` on a symlinked folder
   inherits risk 1.
```

Recap: recommendation is a merged per-folder stage + tombstone swap under a single `.kalpa-staging/<txn>/` journal, with rollback-by-default recovery below the `swapped` phase and idempotent baseline promotion above it, recovery invoked at scan/mutation entry under the P0-A2 lock. The two facts that drove it — updates never delete files, and the Windows-rename-loss caveat — rule out pure A, B, and trusting the journal alone. Two human decisions are needed: symlinked addon folders (recommend refuse) and hard-link vs plain copy for residuals (recommend hard-link with the documented invariant, or copy if you want zero aliasing risk).
