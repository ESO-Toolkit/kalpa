# Fable Consultation — R4 Preserve Separately Tracked Sibling Ownership

## Finding

The install recording path assigns `esoui_id = 0` to every non-primary folder in
an archive, overwriting separately tracked addon/library identity.

If addon A's ZIP bundles library L, and L is *also* separately tracked with its
own ESOUI id, installing or updating A silently demotes L to `esoui_id = 0`.

## Acceptance invariant

A separately tracked sibling folder must never be silently demoted, silently
downgraded, or excluded from future update checks merely because another ZIP
bundles the same folder.

Explicitly: do **not** blindly preserve stale version metadata while
overwriting files. Choose explicit ownership semantics.

## Current code

The demotion site, `src-tauri/src/commands.rs:229`:

```rust
/// Record metadata for a set of installed folders. The primary folder gets
/// the esoui_id and version from ESOUI; secondary folders get id 0 and
/// their local manifest version.
fn record_installed_folders(
    store: &mut metadata::MetadataStore, addons_dir: &Path,
    installed_folders: &[String], esoui_id: u32, esoui_version: &str,
    esoui_title: &str, download_url: &str, esoui_last_update: u64,
) {
    let primary = determine_primary_folder(installed_folders, esoui_title);
    for folder in installed_folders {
        let is_primary = *folder == primary;
        let version = if is_primary && !esoui_version.is_empty() {
            esoui_version.to_string()
        } else {
            read_local_version(addons_dir, folder)
        };
        metadata::record_install_ext(
            store, folder,
            if is_primary { esoui_id } else { 0 },      // <-- the demotion
            &version, download_url,
            if is_primary { esoui_last_update } else { 0 },
        );
    }
}
```

The write primitive, `src-tauri/src/metadata.rs:222`. Note it already has
selective preserve rules for `tags` and `esoui_last_update`, but not `esoui_id`:

```rust
pub fn record_install_ext(store, folder_name, esoui_id, version, download_url,
                          esoui_last_update) {
    let existing = store.addons.get(folder_name);
    let existing_tags = existing.map(|m| m.tags.clone()).unwrap_or_default();
    let last_update = if esoui_last_update == 0 {
        existing.map(|m| m.esoui_last_update).unwrap_or(0)
    } else { esoui_last_update };
    store.addons.insert(folder_name.to_string(), AddonMetadata {
        esoui_id,                                  // unconditional clobber
        installed_version: version.to_string(),
        download_url: download_url.to_string(),    // unconditional clobber
        installed_at, tags: existing_tags, esoui_last_update: last_update,
    });
}
```

`AddonMetadata` (`metadata.rs:10`) has **no ownership/source/provenance field**:
`esoui_id: u32`, `installed_version`, `download_url`, `installed_at`,
`tags: Vec<String>`, `esoui_last_update: u64`.

Primary determination, `commands.rs:215` — a substring match on the ESOUI
title, falling back to `.first()`:

```rust
fn determine_primary_folder(installed_folders: &[String], esoui_title: &str) -> String {
    installed_folders.iter()
        .find(|f| esoui_title.contains(f.as_str()))
        .or(installed_folders.first())
        .cloned().unwrap_or_default()
}
```

`installed_folders` originates from a `HashSet` (`installer.rs:380`, returned at
`:486`), so **when the title heuristic misses, which folder is demoted is
nondeterministic between runs.**

## What demotion costs

- `commands.rs:1794` update checks: `if meta.esoui_id == 0 { continue; }` — the
  sibling silently stops receiving updates. Same in Slint (`main.rs:12613`).
- Export/import round-trip loses it (`commands.rs:3969`, `:4433`).
- Frontend gates hide the ESOUI link and update button, and exclude it from
  Packs (`packs.tsx:96,450`, `addon-detail.tsx:253,439,619`).
- API compatibility checks are **not** affected (`commands.rs:4842` parses
  manifests directly and never reads `esoui_id`).

## The permanence problem

`commands.rs:4053`, in the `auto_link` recovery path:

```rust
// Skip bundled secondary folders: if esouiId is 0 and another addon in the
// store installed this folder (shares download_url), don't auto-link it...
let is_bundled_secondary = already_tracked.is_some_and(|m| {
    m.esoui_id == 0 && store.addons.values()
        .any(|other| other.esoui_id != 0 && other.download_url == m.download_url)
});
if is_bundled_secondary { continue; }
```

After demotion the sibling shares the parent's `download_url`, so the recovery
path deliberately refuses to restore it. **A write-side-only fix does not heal
existing users.** The Slint sidecar has no `auto_link` path at all.

## The opposite failure mode

The dependency install paths (`commands.rs:531`, `:1595`; Slint
`main.rs:13461`) stamp the *same* `dep_id` onto **every** extracted folder. Any
ownership model must be coherent in both directions.

## Existing precedent worth reusing

`file_hashes.rs:26` already models multi-owner as a set, with a legacy migration:

```rust
pub struct HashManifest {
    pub addon_folder: String,
    /// Canonical list of ESOUI IDs that ship files into this folder.
    #[serde(default)] pub esoui_ids: Vec<u32>,
    /// Legacy single-ID field ... Migrated to esoui_ids on load; never written.
    #[serde(default, skip_serializing_if = "is_zero")] pub esoui_id: u32,
    ...
}
```
```rust
if manifest.esoui_ids.is_empty() && manifest.esoui_id != 0 {
    manifest.esoui_ids = vec![manifest.esoui_id];
}
```
…but its writer still clobbers (`file_hashes.rs:664`): `esoui_ids: vec![esoui_id]`.

## Repository constraints

- `metadata.rs` is `#[path]`-included by the Slint crate, so a fix *there*
  reaches both binaries. But `record_installed_folders` is **duplicated** as
  `record_native_installed_folders` (Slint `main.rs:12543`) and must be fixed
  in both, as must `determine_primary_folder` (Slint `main.rs:12571`).
- `AddonMetadata` is persisted JSON; any new field needs `#[serde(default)]`
  and must round-trip old files.
- **Zero test coverage** exists on `record_installed_folders`,
  `record_native_installed_folders`, `determine_primary_folder`, `auto_link`,
  or the `esoui_id == 0` update-check skip.
- Changes to install outcomes require explicit design review before
  implementation (this consultation).

## Candidate designs

**A — Preserve a nonzero existing id.** In `record_install_ext`, never overwrite
a nonzero `esoui_id` with 0 (mirroring the existing `reconcile_addon` rule at
`metadata.rs:269`). One edit, reaches both binaries. But it cannot distinguish
"primary passing a real id" from "sibling passing 0" without a signal, and it
silently keeps the old version string next to newly overwritten files.

**B — Multi-owner metadata.** Give `AddonMetadata` an `esoui_ids: Vec<u32>` set
mirroring `HashManifest`, recording that both A and L ship files into L's
folder, with an explicit authoritative-source selection for update checks.

**C — Skip extraction of a separately tracked sibling.** Do not overwrite L's
files at all when L has its own nonzero id; the user updates L through L.

**D — Classify the bundled overwrite as a conflict** and present it, reusing the
existing pending-conflict machinery.

## Failure modes to evaluate

1. Bundled L is *older* than the separately tracked L. Files are overwritten;
   what version and what id should the folder report, and can this silently
   downgrade the user?
2. Bundled L is *newer*. Does L's own update check still fire afterwards?
3. The nondeterministic primary: with a `HashSet` source and a substring title
   heuristic, A itself can be demoted on some runs. Should the extractor's
   folder list be ordered, and should primary selection be strengthened?
4. Existing users already demoted, blocked by `is_bundled_secondary`. What
   migration or relaxation heals them without re-linking a genuinely bundled-only
   folder to a wrong ESOUI entry?
5. The dependency paths assign one id to N folders. Under the chosen model, is
   that now wrong, and does fixing it change dependency update behavior?
6. Uninstalling A when L is separately tracked and L's folder was overwritten
   by A's archive: what must survive?
7. `download_url` is also clobbered unconditionally and is what
   `is_bundled_secondary` keys on. Does the chosen model need to preserve it too?

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
