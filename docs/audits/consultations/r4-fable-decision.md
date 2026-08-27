I have what I need; the code matches the consultation's description. Here is the decision.

```text
DECISION:
Chosen design: B-lite — "separately tracked sibling keeps its own identity;
the parent records that it also ships files there." Concretely: an
`AddonMetadata.bundled_by: Vec<u32>` provenance set (mirrors
`HashManifest.esoui_ids`), plus an explicit ownership rule in ONE write
primitive that both binaries share via the `#[path]` include of metadata.rs.
Not A (no signal), not C/D (both block or interrupt installs the ESOUI
ecosystem relies on — many "addons" legitimately vendor their libs).

1. metadata.rs — add to AddonMetadata:
     #[serde(default, skip_serializing_if = "Vec::is_empty")]
     pub bundled_by: Vec<u32>,
   Old files round-trip (default = empty). Sorted+deduped on write.

2. metadata.rs — replace the sibling write with an explicit primitive:
     pub fn record_bundled_folder(store, folder, parent_id: u32,
                                  parent_url: &str, local_version: &str)
   Rules, evaluated against `existing = store.addons.get(folder)`:
     a. existing.esoui_id != 0 && != parent_id  → SEPARATELY TRACKED.
        Keep esoui_id, download_url, esoui_last_update, tags, bundled_by∪{parent_id}.
        installed_version := local_version (the manifest on disk — the files
        WERE overwritten, so metadata must say what is actually installed;
        this is the "don't preserve stale version" clause).
        installed_at := now (a download did land in this folder).
     b. otherwise (absent, or id 0, or id == parent_id) → GENUINELY BUNDLED.
        esoui_id := 0, download_url := parent_url, esoui_last_update := 0,
        installed_version := local_version, bundled_by := {parent_id},
        tags preserved.
   `record_install_ext` stays for the PRIMARY only and additionally does
   `bundled_by.clear()` when its esoui_id != 0 — a folder that is now the
   primary of its own install is owned, not bundled. Keep its existing
   unconditional id/url clobber for the primary: that is correct there.

3. commands.rs `record_installed_folders` + Slint
   `record_native_installed_folders`: primary → `record_install_ext`,
   siblings → `record_bundled_folder`. Same edit in both; they remain
   duplicated but now each is two lines of dispatch over the shared rule.

4. Determinism: `installer.rs:486` returns `created_folders` sorted
   (`let mut v: Vec<_> = ...collect(); v.sort();`). Strengthen
   `determine_primary_folder` (both binaries) to prefer, in order:
   (i) a folder already tracked in the store with `esoui_id == this id`
   (updates: the existing primary wins, so a previously-demoted A can't flip);
   (ii) exact case-insensitive title == folder; (iii) title contains folder,
   longest folder first; (iv) sorted first. Requires passing `&store` in —
   it is already `&mut` at every call site (1433, 2074, 2304, 3140, 3519).

5. Update check (`commands.rs:1794`, Slint `main.rs:12613`): unchanged
   condition — `esoui_id == 0 → skip`. With rule (a), L keeps its id so it
   is checked against L's ESOUI entry and its on-disk version. Failure
   mode 1 (bundled L older): installed_version now says the older version,
   so L's own check fires and offers the upgrade back — a visible,
   user-fixable downgrade rather than a silent one. Failure mode 2 (bundled
   L newer): installed_version ≥ ESOUI's, no update offered until L
   publishes; correct.

6. Migration for already-demoted users (failure mode 4), in `auto_link`
   (`commands.rs:4053`): relax `is_bundled_secondary` to
     m.esoui_id == 0 && m.bundled_by.is_empty()
       && <existing shared-download_url test>
   i.e. legacy-shaped entries keep the old conservative skip. Then add a
   ONE-TIME, user-visible heal: when the folder's on-disk manifest `## Version`
   equals the ESOUI entry's version string for that folder name, re-link it
   (the bundled copy is byte-for-version identical to the standalone, so no
   mismatch risk); otherwise leave it and surface it in the existing
   "unlinked addons" UI list rather than guessing. Slint has no auto_link,
   so this heals only in the main app — acceptable; record in RISKS.

7. Dependency paths (`commands.rs:531`, `:1595`, Slint `main.rs:13461`,
   failure mode 5): route through the same two primitives — primary via
   `determine_primary_folder(dep_folders, dep_info.title)` gets
   `record_install_ext(dep_id …)`, the rest `record_bundled_folder(dep_id …)`.
   This changes behaviour: a dep's own siblings stop being update-checked
   under the dep's id. That is the CORRECT direction — today a two-folder
   dependency produces two identical update rows and two downloads.
   Flag in RISKS for sign-off.

8. Uninstall (`commands.rs:1458`, failure mode 6): when removing A, for
   every entry with `parent_id ∈ bundled_by` remove A's id from the set —
   nothing else. L's folder stays on disk with L's identity (its files
   are the last-written ones, which `installed_version` already reflects).
   Genuinely bundled siblings (id 0, bundled_by now empty) are left on disk
   exactly as today; deleting siblings is out of scope and unsafe (other
   addons may DependsOn them).

9. `download_url` (failure mode 7): rule (a) preserves it; rule (b) sets it
   to the parent's. `is_bundled_secondary` keeps keying on it for legacy
   entries; new entries key on `bundled_by`.

10. `file_hashes.rs:664` writer: change to
      esoui_ids: union(existing.esoui_ids, [esoui_id]) when the folder is a
      separately tracked sibling, `vec![esoui_id]` when primary/genuine
      bundle — same predicate as step 2, reusing its result so the two
      stores cannot disagree.

11. Export/import (`commands.rs:3969`, `:4433`): serialize `bundled_by`
    with `#[serde(default)]`; nothing else changes because L now exports
    with its real id.

REJECTED:
1. A (preserve nonzero id in record_install_ext). No signal to distinguish
   "primary legitimately re-linking to a new id after auto_link correction"
   from "sibling passing 0"; and it would keep L's OLD installed_version
   beside overwritten files, so if A bundles L 1.2 over L 1.5 the update
   check sees 1.5 and never offers the repair — exactly the silent
   downgrade the invariant forbids.
2. C (skip extracting a separately tracked sibling). Many ESOUI archives
   ship a lib the addon requires at a specific API level; skipping leaves a
   folder the addon may not load with, and partial extraction of one ZIP
   breaks `extract_with_rollback`'s all-or-nothing contract.
3. D (present as a conflict). `pending-conflict` machinery is per-file
   user-edit protection; plumbing a per-folder ownership question through
   it would block every LibAddonMenu-bundling install with a modal, on
   every update. Wrong tool, unacceptable UX.
4. Full multi-owner `esoui_ids: Vec<u32>` on AddonMetadata with a chosen
   "authoritative" id. Every reader (update check, export, three frontend
   gates, Slint) would need to learn the selection rule; `bundled_by` +
   scalar `esoui_id` keeps the single authoritative id readers already
   use and adds provenance without changing any consumer's contract.

CRASH_RECOVERY:
1. All writes go through `save_json_with_backup` (temp + rename + .bak),
   so a kill mid-save yields either the old or new kalpa.json, never a
   torn one; `load_json_with_backup` recovers .tmp/.bak.
2. Files extracted but metadata not yet saved (kill between extract and
   save): folder holds A's copy of L, metadata still says L's id + old
   version. Next `auto_link`/scan reads the manifest version from disk;
   the existing scan path already reconciles `installed_version` against
   the manifest, so the state self-corrects on next open. No new marker.
3. Metadata saved but hash manifest not (step 10 second write): hash
   manifest's `esoui_ids` lacks the parent; consequence is only that a
   later Protected-Edits diff attributes files to L alone. Next update of
   A rewrites it. Acceptable; no ordering change needed.
4. Migration (step 6) is idempotent and re-evaluated on every auto_link;
   an interrupted heal leaves entries in the legacy shape and is retried.
5. Old binaries (Slint sidecar from a previous release) reading a file
   with `bundled_by`: serde ignores unknown fields by default (no
   `deny_unknown_fields` on the struct — verified), and they drop it on
   write, reverting that entry to legacy shape, which the skip rule still
   handles conservatively.

TESTS:
1. metadata.rs: L tracked (id 7, v1.5) → `record_bundled_folder(L, parent
   3, url_A, "1.2")` → id 7, url_L, version "1.2", bundled_by [3], tags
   kept. (Distinguishes from A, which would keep "1.5".)
2. Same, but existing id == 3 (parent) → treated as genuinely bundled:
   id 0, url_A, bundled_by [3].
3. Absent entry → id 0, url_A, bundled_by [3]; second install by parent 9
   → bundled_by [3,9] sorted, id still 0.
4. `record_install_ext` for primary clears `bundled_by`.
5. Round-trip: pre-change JSON (no `bundled_by`) loads, saves without the
   field when empty (snapshot byte-equality on a fixture).
6. commands.rs `determine_primary_folder`: folders ["LibFoo","FooAddon"]
   shuffled 100× with title "Foo Addon" → always "FooAddon"; with an
   existing store entry FooAddon:id=3 and title mismatch → still
   "FooAddon"; nothing matches → sorted first, deterministic.
7. `record_installed_folders` integration: store {L:7}, install A(3) with
   folders [A, L] → A:3 primary, L:7 bundled_by [3]; run the update-check
   filter → L not skipped. (Distinguishes from the current code, which
   skips L.)
8. Update path: bundled L older → installed_version "1.2" < ESOUI "1.5" →
   update row emitted for L. Bundled L newer → no row.
9. auto_link heal: legacy entry L{id 0, url_A}, A{id 3, url_A}, manifest
   version == ESOUI L version → relinked to L's id; version differs → left
   alone AND appears in the unlinked list. Legacy entry with no sharing
   parent → normal link (regression guard for existing behaviour).
10. remove_addon(A): L.bundled_by loses 3, id/url/version untouched;
    genuine bundle B{id 0, bundled_by [3]} → bundled_by [] and still on
    disk.
11. Dependency install with two folders: exactly one entry gets dep_id,
    the other id 0 with bundled_by [dep_id]; update check yields one row.
12. file_hashes: sibling case → esoui_ids [3,7] (union); primary case →
    [3] only.
13. Slint: mirror of tests 6–7 against `record_native_installed_folders`
    (the crate has its own test target; the shared rule is in metadata.rs,
    so this mainly guards the dispatch not regressing).

RISKS:
1. Dependency semantics change (step 7): a multi-folder dependency now
   update-checks once, not N times. Behaviourally better, but it is an
   install-outcome change — needs your explicit sign-off.
2. The heal (step 6) is main-app only; Slint users stay demoted until
   they open the main app once. Alternative is porting auto_link to
   Slint — a separate, larger task.
3. Version-equality heal is conservative: users whose bundled copy differs
   from ESOUI's stay unlinked and are surfaced, not fixed. Decide whether
   a one-click "link anyway" in the unlinked list is wanted.
4. "Bundled L older → offer L's update" is deliberately loud. If A
   actually requires the older L, the user will re-upgrade L and possibly
   break A. Same trade-off Minion makes; documenting it in the update row
   ("also shipped by A") is a UI follow-up.
5. `determine_primary_folder` still ends in a heuristic when the title
   matches nothing; the store-lookup rule fixes the flip-flop for updates
   but the very first install of an unusually named archive can still
   pick a wrong primary. Truly fixing that needs the ESOUI file list,
   which is a scraping change in esoui.rs — out of scope here.
6. Two duplicated dispatch functions (main + Slint) remain a divergence
   hazard; consider moving `determine_primary_folder` and the dispatch into
   metadata.rs itself so the `#[path]` include carries them too.
```

Recap: chosen design is a small provenance field (`bundled_by`) plus one shared write rule — a separately tracked sibling keeps its id and URL but reports the version actually on disk, so its own update check keeps firing and any downgrade is visible; genuinely bundled folders behave as today. It also fixes the nondeterministic primary, heals already-demoted users conservatively via `auto_link`, and makes dependency multi-folder stamping coherent. Items 1, 2 and 7 in RISKS need your decision before implementation.
