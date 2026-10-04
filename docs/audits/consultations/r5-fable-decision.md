Inventory checks out against the code, with one correction that drives the decision: `installer.rs:229` already has `collect_zip_top_folders`, and `installed_folders` from extraction already feeds `record_hashes_with_zip_baseline` per folder. The only missing piece is that the *scan* side never enumerates folders — everything downstream is keyed by whatever the scan produces.

```text
DECISION: C — folder-qualify the cross-folder plumbing, keep per-folder storage keys.

Rationale in one line: the skip-key layer (installer.rs:425) is already
`Folder/rel` and already tolerates the flat wrap; manifests and backups are
already partitioned by folder on disk. Qualifying only the transport (report,
decisions, diff lookup, overrides) makes every layer meet at the boundary that
already exists, with zero storage migration.

Key contract: every string that crosses a folder boundary (Tauri/Slint wire,
PendingUpdate, decisions, skip set) is `Folder/relative/path`; every string that
lives inside a folder (HashManifest.files, backup paths, classify input) stays
bare. The split is `split_once('/')` and happens in exactly one Rust helper.

1. file_hashes.rs — add
   `pub fn hash_zip_entries_by_folder(zip) -> Result<ZipHashSet, String>` where
   `ZipHashSet { folders: BTreeMap<String, HashMap<String,String>>, flat_wrap: Option<String> }`.
   Enumerate with `collect_zip_top_folders` (make it pub(crate)); if
   `flat_archive_wrap_name` is Some, the set has exactly one folder = wrap name
   with unprefixed entry names. Symlink guard and `enclosed_name` verbatim
   from the current loop. Keep `hash_zip_entries(zip, folder)` as a wrapper
   (`by_folder(zip)?.folders.remove(folder).unwrap_or_default()`) so Slint's
   private call sites and the secondary-folder baseline loop at :568 compile
   unchanged.
   Add `ZipHashSet::zip_entry_name(&self, folder, rel) -> String` — returns
   `rel` when `flat_wrap == Some(folder)`, else `format!("{folder}/{rel}")`.
   This is the ONLY place the flat divergence is encoded.
   Add `pub fn split_qualified(&str) -> Option<(&str, &str)>` (first `/`,
   both halves non-empty) and `pub fn qualify(folder, rel) -> String`.

2. lib.rs PendingUpdate — replace `zip_hashes: Arc<HashMap>` with
   `zip_hashes: Arc<ZipHashSet>`. Keep `folder_name` as the primary (still
   needed for metadata download_url carry-forward at commands.rs:~3505 and
   for the session id). Since PendingUpdates is in-memory only, no
   migration.

3. commands.rs classification —
   `classify_update_files(addons_dir, folder, &per_folder_hashes)` stays
   per-folder and bare. New `classify_update_archive(addons_dir, &ZipHashSet)`
   loops folders, calls the existing function, and qualifies every output
   path (`safe_files`, `auto_kept_files`, `conflicts[].relative_path`) with
   `qualify(folder, rel)`. `build_conflict_report` calls this. Wire shape of
   `ConflictReport`/`FileConflict` is unchanged in field names; the
   `relative_path` VALUES become folder-qualified. Add
   `folders: Vec<String>` to `ConflictReport` (the archive's top folders) so
   the UI can group. `folder_name` remains the primary.
   Batch path (`:2555/:2565`) inherits this automatically because it
   consumes the same report.

4. commands.rs apply (`update_with_decisions_inner` :3379) —
   a. `validate_relative_path` on each decision (already accepts `Folder/rel`),
      then REJECT any decision whose folder half is not in
      `zip_hashes.folders` — this is the server-side re-derive guard
      extended to the folder axis.
   b. Re-derive via `classify_update_archive` (not `classify_update_files`
      on `pu.folder_name`) so a sibling edited during deliberation is caught.
   c. `skip_files` = kept qualified paths verbatim — the producer stops
      prepending `pu.folder_name`. Flat archives already work because the
      consumer checks `wrapped_key`.
   d. Group `files_to_backup` and `kept_files` by folder with
      `split_qualified`; call `backup_user_files(addons_dir, folder, bare_paths,
      from_version_for_that_folder, ...)` once per folder, reading
      `from_version` from THAT folder's manifest.
   e. `hash_overrides` becomes `HashMap<String /*folder*/, HashMap<String,String>>`;
      `record_hashes_with_zip_baseline` takes it and selects
      `overrides.get(folder)` instead of `if is_primary {..} else {None}`
      (file_hashes.rs:~590). Its `primary_zip_hashes` param becomes the whole
      `ZipHashSet` so the secondary-folder branch no longer re-opens the ZIP.
   f. Batch apply (`:3035`) gets the same grouping via one shared helper
      `group_by_folder(&[String]) -> BTreeMap<String, Vec<String>>` — do not
      duplicate the loop.

5. commands.rs `get_conflict_diff` (:3206) — `split_qualified(relative_path)`;
   disk side = `addons_dir/folder/rel`; ZIP side = `zip_hashes.zip_entry_name(folder, rel)`.
   Reject if the folder is not in the pending set.

6. Frontend — types.ts: add `folders: string[]` to ConflictReport, document
   that `relativePath` is folder-qualified. update-conflict-panel.tsx:35 keeps
   keying `decisions` by `relativePath` — the collision is fixed by the key
   change alone; group rows under a folder header when `folders.length > 1`.
   addon-file-browser.tsx:215 — build the path from the qualified string, not
   `folderName + path`. Update-conflict "diff" call passes the qualified path
   unchanged.

7. Slint main.rs — mirror steps 3–5 at :12856–13260 and :14485. The private
   `NativeHashManifest` (:686/:17905) touches on-disk manifests only, which
   are unchanged; leave it, but add a test in the slint crate that round-trips
   a shared-module manifest through it (guard against R7 drift).

8. Tests: rewrite `hash_zip_ignores_other_folders` (file_hashes.rs:912) to
   assert BOTH folders are returned, primary and sibling each with their own
   bare keys. Keep `file_hashes.rs:931` and `installer.rs:765` flat tests as-is;
   add `flat_wrap` assertions to them.

REJECTED:
1. A — global `Folder/rel` keys. Fails on migration (mode 6): every existing
   `.kalpa-hashes/<Folder>.json` has bare keys, so `detect_modifications`
   would report every file as "untracked → re-extract" on the first post-
   upgrade update, silently overwriting edits — the exact bug class being
   fixed. A dual-read shim is more code than C and still leaves the flat
   case as a key/entry divergence in every layer instead of one helper.
2. B — restructure `ConflictReport` into `Vec<FolderReport>` and add a
   `folder` field to `FileDecision`. Correct, but it changes every wire
   struct across Tauri, Slint and the frontend, and the frontend collision
   (panel.tsx:35) would still need a composite key built by hand. C gets the
   same guarantees with the composite key being the wire value itself.
3. Keep bare keys and just loop `build_conflict_report` per folder with
   separate sessions. Fails mode 5: a single ZIP extraction cannot be split
   per session, so the second session's apply re-extracts the first folder
   and overwrites the first session's kept files.

CRASH_RECOVERY:
1. Kill during scan: pending map is in-memory, the persisted temp ZIP is
   orphaned exactly as today; no on-disk state changed. Unchanged.
2. Kill after backups, before extraction: backups for N folders sit under
   `.kalpa-backups/<Folder>/<ts>/`; disk unchanged; re-scan produces the same
   report. Backups are per-folder-atomic today and remain so; a partially
   written backup set is harmless (extra backup, no data loss).
3. Kill mid-extraction: identical to today's single-folder story — partial
   folder contents, manifest not yet rewritten, next scan sees "modified"
   files everywhere in the touched folders and prompts. Kept (skipped) files
   are untouched because skipping is a no-write.
4. Kill between extraction and `record_hashes_with_zip_baseline`: the
   existing "fail the update if baseline can't persist" rule holds; since
   `record_each_folder` attempts all folders and reports the first error,
   a partial set of new manifests is possible. Next scan for the stale-
   manifest folder classifies every upstream-changed file as a conflict
   (baseline is the OLD version's hash, disk is NEW) — noisy but safe;
   already the behaviour for secondaries today.
5. Stale `PendingUpdate` from a pre-upgrade binary: impossible, in-memory.
6. Timeout on `hash_zip_entries_by_folder` over a huge multi-folder ZIP:
   the per-entry streaming cap applies per entry as today; the existing
   fallback to `compute_addon_hashes` at :575 remains per folder.

TESTS:
1. file_hashes: two-folder ZIP → `by_folder` returns both maps, keys bare,
   `flat_wrap == None`. (Distinguishes from the current single-folder
   behaviour the old test enshrined.)
2. file_hashes: flat ZIP → one folder named by the wrap, keys bare,
   `zip_entry_name(wrap, "init.lua") == "init.lua"`; foldered ZIP →
   `"Folder/init.lua"`. (Catches a design that qualifies flat entries.)
3. commands: primary and sibling both ship `init.lua`; user edits the
   SIBLING's copy only → report has exactly one conflict, path
   `"Sibling/init.lua"`, primary's `init.lua` in `safe_files`. (Catches the
   bare-key collision and the pending-folder diff bug.)
4. commands: keep_mine on `"Sibling/init.lua"` → sibling bytes preserved,
   primary's `init.lua` overwritten, `Sibling.json` manifest stores the
   UPSTREAM hash for it and lists it in `modified_files`; `Primary.json`
   has no override. (Catches overrides applied to the wrong folder.)
5. commands: take_update on `"Sibling/init.lua"` → backup exists under
   `.kalpa-backups/Sibling/<ts>/init.lua`, none under `Primary/`, and its
   `from_version` is read from `Sibling.json`. (Catches backup routed by
   `pu.folder_name`.)
6. commands: decision with folder not in the ZIP (`"Evil/x.lua"`) → Err,
   nothing extracted.
7. commands: user-deleted sibling file upstream didn't change → auto-kept,
   skipped, override stored (mode 4 extended to siblings; mirror of the
   existing :1093/:1426 tests on a secondary).
8. commands: batch path with `conflict_policy = keep_mine` and a sibling
   conflict → skip set contains the qualified sibling path; with
   `take_update` → sibling backup written.
9. Migration: write a manifest with today's bare keys, run a scan → no
   spurious conflicts. (Guards against anyone later "cleaning up" to A.)
10. get_conflict_diff on a qualified sibling path returns the sibling's
    upstream bytes, not the primary's same-named file.
11. Frontend (vitest): ConflictReport with `["A/init.lua","B/init.lua"]`
    yields two independent decision entries.

RISKS:
1. Wire values change meaning (bare → qualified) without a field rename.
   A stale frontend bundle against a new backend, or Slint vs Tauri skew,
   would mis-route. Mitigation: bump nothing but rely on the monorepo
   lockstep; if you want belt-and-braces, rename to `qualifiedPath` and
   accept the churn. Human decision.
2. R4 interaction (mode 8): a sibling that is ALSO separately tracked under
   its own esoui_id gets its manifest rewritten by this update with THIS
   esoui_id in `esoui_ids`. That is today's behaviour for secondaries and
   `esoui_ids` is a Vec for exactly this reason, but the R4 decision on
   ownership precedence must land first or be explicitly ordered after.
3. The batch panel currently shows one conflict count per addon; with
   siblings the count can jump for addons users think of as single-folder.
   Copy/UX decision, not a code risk.
4. Slint's private manifest copy is not touched here; it remains a known
   divergence point (R7). The round-trip test only detects, doesn't prevent.
5. Extra disk hashing: `classify_update_archive` hashes every touched folder
   that has a manifest. Bounded by the addon's own size; no new cap needed,
   but worth a timing check on the largest bundle (LibAddonMenu-style).
```

Summary: pick design C. The archive is enumerated once into a per-folder hash set with a single flat-wrap flag; the report, decisions, skip set and diff lookups carry `Folder/rel` strings; storage stays bare and per-folder, so no manifest migration. Ordering constraint: land the R4 ownership decision first or explicitly sequence this after it.
