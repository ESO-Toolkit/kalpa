use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonMetadata {
    pub esoui_id: u32,
    pub installed_version: String,
    pub download_url: String,
    pub installed_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// ESOUI last-updated timestamp in epoch milliseconds (from the API).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub esoui_last_update: u64,
    /// ESOUI IDs of other addons whose archives also ship files into this
    /// folder.
    ///
    /// A folder can be written by more than one addon: many ESOUI addons
    /// vendor their libraries. Recording that as provenance means a library
    /// that is *also* tracked in its own right keeps its own identity instead
    /// of being demoted, while Kalpa still knows the folder is not solely its
    /// own. Empty for the ordinary single-owner case, so old metadata files
    /// round-trip unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bundled_by: Vec<u32>,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataStore {
    pub version: u32,
    pub addons: HashMap<String, AddonMetadata>,
}

impl Default for MetadataStore {
    fn default() -> Self {
        Self {
            version: 1,
            addons: HashMap::new(),
        }
    }
}

fn metadata_path(addons_path: &Path) -> std::path::PathBuf {
    addons_path.join("kalpa.json")
}

/// Load a JSON file with automatic recovery from crash artifacts.
///
/// Recovery order when the primary file is missing or corrupted:
/// 1. `.json.tmp` — a completed write that was never renamed into place
///    (crash between the temp write and the rename in `save_json_with_backup`).
/// 2. `.json.bak` — the copy of the previous primary taken during the last save.
///
/// Returns `T::default()` if all sources are missing or corrupted.
pub fn load_json_with_backup<T: DeserializeOwned + Default>(path: &Path) -> T {
    // Try the primary file first.
    if let Ok(content) = fs::read_to_string(path) {
        if let Ok(data) = serde_json::from_str(&content) {
            return data;
        }
        eprintln!(
            "Warning: {} corrupted, trying recovery files...",
            path.display()
        );
    }

    // Primary missing or corrupted — try .tmp (newest data, written but not renamed).
    let tmp = path.with_extension("json.tmp");
    if let Ok(content) = fs::read_to_string(&tmp) {
        if let Ok(data) = serde_json::from_str::<T>(&content) {
            eprintln!("Recovered data from incomplete write {}.", tmp.display());
            // Promote the .tmp so subsequent loads hit the primary path.
            // On Windows fs::rename can't overwrite, so remove the corrupt primary first.
            // Best-effort: if promotion fails the data is still returned correctly;
            // the next load will recover from .tmp again.
            if let Err(e) = fs::remove_file(path) {
                eprintln!(
                    "Warning: could not remove corrupt primary {}: {e}",
                    path.display()
                );
            }
            if let Err(e) = fs::rename(&tmp, path) {
                eprintln!(
                    "Warning: could not promote {} to primary: {e}",
                    tmp.display()
                );
            }
            return data;
        }
    }

    // Try .bak (previous good version).
    let bak = path.with_extension("json.bak");
    if let Ok(content) = fs::read_to_string(&bak) {
        if let Ok(data) = serde_json::from_str::<T>(&content) {
            eprintln!("Recovered data from backup file {}.", bak.display());
            return data;
        }
        eprintln!("Backup also corrupted, using defaults.");
    }

    T::default()
}

/// Save data as JSON with atomic write and automatic backup.
///
/// Writes and fsyncs `.json.tmp`, copies the current primary to `.json.bak`, then
/// renames the temp over the primary. The ordering matters: copying to `.bak`
/// first would overwrite the last good backup with a possibly-corrupt primary
/// before the replacement was safe on disk — destroying the very copy the `.bak`
/// recovery in [`load_json_with_backup`] exists to provide.
pub fn save_json_with_backup<T: Serialize>(path: &Path, data: &T) -> Result<(), String> {
    let json =
        serde_json::to_string_pretty(data).map_err(|e| format!("Failed to serialize: {e}"))?;

    // Flush the replacement to stable storage before anything else is touched.
    // Without sync_all a power loss can journal the rename (and the .bak copy)
    // while both files' data blocks are still in page cache, leaving primary,
    // .tmp and .bak all zero-length — the whole recovery ladder defeated and the
    // store silently reset to T::default() on the next load.
    let tmp = path.with_extension("json.tmp");
    let write_tmp = || -> std::io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()
    };
    if let Err(e) = write_tmp() {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Failed to write temp file: {e}"));
    }

    // Only now is the previous primary expendable (ignore if it doesn't exist).
    let bak = path.with_extension("json.bak");
    let _ = fs::copy(path, &bak);

    // fs::rename replaces the destination atomically on both Unix and Windows
    // (MoveFileExW with MOVEFILE_REPLACE_EXISTING). Removing the primary first
    // would only add a window in which no primary exists at all.
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("Failed to finalize write: {e}")
    })
}

pub fn format_timestamp(secs: u64) -> String {
    // Simple UTC timestamp without chrono dependency
    let days = secs / 86400;
    let rem = secs % 86400;
    let hours = rem / 3600;
    let mins = (rem % 3600) / 60;
    let s = rem % 60;

    // Days since epoch to date (simplified)
    let mut y = 1970i64;
    let mut d = days as i64;
    loop {
        let year_days = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
        if d < year_days {
            break;
        }
        d -= year_days;
        y += 1;
        if y > 3000 {
            return format!("9999-12-31T{hours:02}:{mins:02}:{s:02}Z");
        }
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 0;
    for &md in &month_days {
        if d < md {
            break;
        }
        d -= md;
        m += 1;
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m + 1,
        d + 1,
        hours,
        mins,
        s
    )
}

pub fn load_metadata(addons_path: &Path) -> MetadataStore {
    load_json_with_backup(&metadata_path(addons_path))
}

pub fn save_metadata(addons_path: &Path, store: &MetadataStore) -> Result<(), String> {
    save_json_with_backup(&metadata_path(addons_path), store)
}

pub fn record_install(
    store: &mut MetadataStore,
    folder_name: &str,
    esoui_id: u32,
    version: &str,
    download_url: &str,
) {
    record_install_ext(store, folder_name, esoui_id, version, download_url, 0);
}

pub fn record_install_ext(
    store: &mut MetadataStore,
    folder_name: &str,
    esoui_id: u32,
    version: &str,
    download_url: &str,
    esoui_last_update: u64,
) {
    let existing = store.addons.get(folder_name);
    // Preserve existing tags when re-recording an install (e.g. update)
    let existing_tags = existing.map(|m| m.tags.clone()).unwrap_or_default();
    // Keep existing last_update if new one is 0
    let last_update = if esoui_last_update == 0 {
        existing.map(|m| m.esoui_last_update).unwrap_or(0)
    } else {
        esoui_last_update
    };
    // `installed_at` records the last time Kalpa actually downloaded/installed
    // this addon locally (a real install or update). It is intentionally
    // refreshed on every such call so the "Recently Downloaded" sort reflects
    // recent local activity. Metadata-only reconciliation (auto_link) must NOT
    // route through here — it uses `reconcile_addon`, which leaves this field
    // untouched — so a pending, un-downloaded update never falsely stamps it.
    let installed_at = format_timestamp(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    );
    // A folder recorded under a real ID is owned by that addon, not bundled by
    // it, so stale provenance from an earlier bundled install is cleared. An ID
    // of 0 means the caller has no identity to assert, so any existing
    // provenance is left alone.
    let bundled_by = if esoui_id == 0 {
        existing.map(|m| m.bundled_by.clone()).unwrap_or_default()
    } else {
        Vec::new()
    };
    store.addons.insert(
        folder_name.to_string(),
        AddonMetadata {
            esoui_id,
            installed_version: version.to_string(),
            download_url: download_url.to_string(),
            installed_at,
            tags: existing_tags,
            esoui_last_update: last_update,
            bundled_by,
        },
    );
}

/// Record a folder that an archive wrote into but that the archive does not
/// own outright.
///
/// This is the counterpart to [`record_install_ext`], which records the archive
/// **primary** folder. Assigning every non-primary folder `esoui_id = 0` (the
/// previous behaviour) silently demoted a library that the user also tracks in
/// its own right: update checks skip `esoui_id == 0`, so the library stopped
/// receiving updates entirely, and the demotion was effectively permanent.
///
/// Two cases, decided by what is already recorded for the folder:
///
/// * **Separately tracked** - a different, nonzero ID is already recorded. The
///   folder keeps its identity, URL and ESOUI timestamp, and gains `parent_id`
///   as provenance. `installed_version` is still refreshed from the manifest on
///   disk, because the files really were overwritten and metadata must describe
///   what is actually installed rather than preserving a stale version.
/// * **Genuinely bundled** - nothing recorded, ID 0, or the same ID. The folder
///   belongs to `parent_id`, so it takes the parent URL and stays out of update
///   checks in its own right.
pub fn record_bundled_folder(
    store: &mut MetadataStore,
    folder_name: &str,
    parent_id: u32,
    parent_url: &str,
    local_version: &str,
) {
    let existing = store.addons.get(folder_name);
    let separately_tracked = existing
        .map(|m| m.esoui_id != 0 && m.esoui_id != parent_id)
        .unwrap_or(false);

    let tags = existing.map(|m| m.tags.clone()).unwrap_or_default();
    let mut bundled_by = existing.map(|m| m.bundled_by.clone()).unwrap_or_default();
    if parent_id != 0 && !bundled_by.contains(&parent_id) {
        bundled_by.push(parent_id);
    }
    bundled_by.sort_unstable();
    bundled_by.dedup();

    let (esoui_id, download_url, esoui_last_update) = if separately_tracked {
        let owner = existing.expect("separately_tracked implies an existing entry");
        (
            owner.esoui_id,
            owner.download_url.clone(),
            owner.esoui_last_update,
        )
    } else {
        (0, parent_url.to_string(), 0)
    };

    let installed_at = format_timestamp(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    );

    store.addons.insert(
        folder_name.to_string(),
        AddonMetadata {
            esoui_id,
            installed_version: local_version.to_string(),
            download_url,
            installed_at,
            tags,
            esoui_last_update,
            bundled_by,
        },
    );
}

/// Drop `parent_id` from every folder that records it as provenance.
///
/// Used when the parent addon is removed. A separately tracked sibling keeps
/// its own identity and its files; only the record that this parent also wrote
/// there goes away. Genuinely bundled folders are left on disk exactly as
/// before - other addons may declare a dependency on them.
pub fn forget_bundled_parent(store: &mut MetadataStore, parent_id: u32) {
    if parent_id == 0 {
        return;
    }
    for meta in store.addons.values_mut() {
        meta.bundled_by.retain(|id| *id != parent_id);
    }
}

/// Reconcile API-derived fields on an existing metadata entry during `auto_link`
/// without touching `installed_at` (the local download time) or `tags`. Used
/// when ESOUI metadata changes (e.g. a new upstream version is published) but no
/// download happened, so the "last downloaded" time must stay put. Keeps a known
/// `esoui_id`/`download_url` rather than clobbering it with an empty API value.
pub fn reconcile_addon(
    meta: &mut AddonMetadata,
    esoui_id: u32,
    esoui_last_update: u64,
    download_url: &str,
) {
    let was_bundled = meta.esoui_id == 0 && !meta.bundled_by.is_empty();
    if esoui_id > 0 {
        meta.esoui_id = esoui_id;
    }
    if (was_bundled || meta.download_url.is_empty()) && !download_url.is_empty() {
        meta.download_url = download_url.to_string();
    }
    meta.esoui_last_update = esoui_last_update;
}

pub fn remove_entry(store: &mut MetadataStore, folder_name: &str) {
    store.addons.remove(folder_name);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of R4: a library the user tracks in its own right keeps
    /// its identity when some other addon's archive also ships it.
    #[test]
    fn a_separately_tracked_library_keeps_its_identity_when_bundled() {
        let mut store = MetadataStore::default();
        record_install_ext(&mut store, "LibFoo", 7, "1.5", "https://esoui/lib-foo", 900);
        store
            .addons
            .get_mut("LibFoo")
            .unwrap()
            .tags
            .push("favorite".to_string());

        // Addon 3 bundles an older LibFoo and overwrites the folder.
        record_bundled_folder(&mut store, "LibFoo", 3, "https://esoui/addon-a", "1.2");

        let lib = store.addons.get("LibFoo").expect("LibFoo still tracked");
        assert_eq!(
            lib.esoui_id, 7,
            "must not be demoted to an untracked folder"
        );
        assert_eq!(lib.download_url, "https://esoui/lib-foo");
        assert_eq!(lib.esoui_last_update, 900);
        assert_eq!(lib.tags, vec!["favorite".to_string()]);
        assert_eq!(lib.bundled_by, vec![3]);
        // The files really were overwritten, so the recorded version describes
        // what is on disk now - not the newer version that used to be there.
        assert_eq!(lib.installed_version, "1.2");
    }

    /// Re-bundling by the owner itself is not a second owner, and a folder
    /// nobody else tracks stays genuinely bundled.
    #[test]
    fn a_genuinely_bundled_folder_belongs_to_its_parent() {
        let mut store = MetadataStore::default();

        // Never seen before.
        record_bundled_folder(&mut store, "LibBar", 3, "https://esoui/addon-a", "1.0");
        let bar = store.addons.get("LibBar").unwrap();
        assert_eq!(bar.esoui_id, 0);
        assert_eq!(bar.download_url, "https://esoui/addon-a");
        assert_eq!(bar.bundled_by, vec![3]);

        // A second addon ships it too: provenance accumulates, sorted.
        record_bundled_folder(&mut store, "LibBar", 9, "https://esoui/addon-b", "1.0");
        assert_eq!(store.addons.get("LibBar").unwrap().bundled_by, vec![3, 9]);

        // Bundled by the addon that already owns the folder is not a new owner.
        record_install_ext(&mut store, "LibBaz", 4, "2.0", "https://esoui/baz", 0);
        record_bundled_folder(&mut store, "LibBaz", 4, "https://esoui/baz", "2.0");
        let baz = store.addons.get("LibBaz").unwrap();
        assert_eq!(
            baz.esoui_id, 0,
            "same-owner re-record is an ordinary bundle"
        );
        assert_eq!(baz.bundled_by, vec![4]);
    }

    /// A folder recorded under a real ID is owned, not bundled, so stale
    /// provenance from an earlier life as someone else's sibling is cleared.
    #[test]
    fn recording_a_primary_install_clears_stale_provenance() {
        let mut store = MetadataStore::default();
        record_bundled_folder(&mut store, "LibFoo", 3, "https://esoui/addon-a", "1.0");
        assert_eq!(store.addons.get("LibFoo").unwrap().bundled_by, vec![3]);

        record_install_ext(&mut store, "LibFoo", 7, "1.5", "https://esoui/lib-foo", 0);
        assert!(store.addons.get("LibFoo").unwrap().bundled_by.is_empty());
    }

    /// Removing the parent drops only the provenance record.
    #[test]
    fn forgetting_a_parent_leaves_the_sibling_intact() {
        let mut store = MetadataStore::default();
        record_install_ext(&mut store, "LibFoo", 7, "1.5", "https://esoui/lib-foo", 0);
        record_bundled_folder(&mut store, "LibFoo", 3, "https://esoui/addon-a", "1.2");
        record_bundled_folder(&mut store, "LibOnly", 3, "https://esoui/addon-a", "1.0");

        forget_bundled_parent(&mut store, 3);

        let lib = store.addons.get("LibFoo").unwrap();
        assert!(lib.bundled_by.is_empty());
        assert_eq!(lib.esoui_id, 7, "identity survives the parent going away");
        assert_eq!(lib.installed_version, "1.2");
        // A genuinely bundled folder stays recorded: other addons may depend
        // on it, so it is not ours to delete.
        let only = store.addons.get("LibOnly").expect("still recorded");
        assert!(only.bundled_by.is_empty());
        assert_eq!(only.esoui_id, 0);
    }

    /// Metadata written before `bundled_by` existed must load, and an entry
    /// without provenance must not start writing an empty array into the file.
    #[test]
    fn metadata_without_provenance_round_trips_unchanged() {
        let legacy = r#"{"version":1,"addons":{"LibFoo":{"esouiId":7,"installedVersion":"1.5","downloadUrl":"u","installedAt":"t"}}}"#;
        let store: MetadataStore = serde_json::from_str(legacy).expect("legacy file loads");
        let lib = store.addons.get("LibFoo").expect("entry present");
        assert_eq!(lib.esoui_id, 7);
        assert!(lib.bundled_by.is_empty());

        let written = serde_json::to_string(&store).expect("serializes");
        assert!(
            !written.contains("bundledBy"),
            "an empty provenance set must stay absent from the file, got: {written}"
        );
    }

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.json");

        let mut store = MetadataStore::default();
        record_install(&mut store, "TestAddon", 123, "1.0.0", "https://example.com");

        save_json_with_backup(&path, &store).unwrap();

        let loaded: MetadataStore = load_json_with_backup(&path);
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.addons.len(), 1);
        assert_eq!(loaded.addons["TestAddon"].esoui_id, 123);
        assert_eq!(loaded.addons["TestAddon"].installed_version, "1.0.0");
    }

    #[test]
    fn load_returns_default_for_missing_file() {
        let loaded: MetadataStore = load_json_with_backup(Path::new("/nonexistent/path.json"));
        assert_eq!(loaded.version, 1);
        assert!(loaded.addons.is_empty());
    }

    #[test]
    fn load_recovers_from_corrupted_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.json");
        let bak = tmp.path().join("test.json.bak");

        // Write a valid backup
        let mut store = MetadataStore::default();
        record_install(&mut store, "Recovered", 42, "2.0.0", "https://example.com");
        let json = serde_json::to_string(&store).unwrap();
        fs::write(&bak, &json).unwrap();

        // Write corrupted primary
        fs::write(&path, "this is not valid json{{{").unwrap();

        let loaded: MetadataStore = load_json_with_backup(&path);
        assert_eq!(loaded.addons.len(), 1);
        assert_eq!(loaded.addons["Recovered"].esoui_id, 42);
    }

    #[test]
    fn load_returns_default_when_both_corrupted() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.json");
        let bak = tmp.path().join("test.json.bak");

        fs::write(&path, "corrupted").unwrap();
        fs::write(&bak, "also corrupted").unwrap();

        let loaded: MetadataStore = load_json_with_backup(&path);
        assert!(loaded.addons.is_empty());
    }

    #[test]
    fn failed_save_preserves_the_previous_backup() {
        // The .bak copy must not run until the replacement is safely on disk.
        // Copying first meant a save that failed mid-write left the primary AND
        // the last good backup unusable, silently resetting the store.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.json");
        let bak = tmp.path().join("test.json.bak");

        let mut good = MetadataStore::default();
        record_install(&mut good, "Good", 1, "1.0", "url");
        save_json_with_backup(&path, &good).unwrap();
        // A second save establishes the .bak from the known-good primary.
        save_json_with_backup(&path, &good).unwrap();
        assert!(bak.exists());

        // Occupying the temp path with a directory makes the write fail.
        fs::create_dir(tmp.path().join("test.json.tmp")).unwrap();
        let mut other = MetadataStore::default();
        record_install(&mut other, "Other", 2, "2.0", "url");
        assert!(save_json_with_backup(&path, &other).is_err());

        let backup: MetadataStore =
            serde_json::from_str(&fs::read_to_string(&bak).unwrap()).unwrap();
        assert!(backup.addons.contains_key("Good"));
        let primary: MetadataStore =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(primary.addons.contains_key("Good"));
    }

    #[test]
    fn save_creates_backup_of_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.json");
        let bak = tmp.path().join("test.json.bak");

        // First save
        let store1 = MetadataStore::default();
        save_json_with_backup(&path, &store1).unwrap();
        assert!(!bak.exists());

        // Second save should create backup
        let mut store2 = MetadataStore::default();
        record_install(&mut store2, "New", 1, "1.0", "url");
        save_json_with_backup(&path, &store2).unwrap();
        assert!(bak.exists());

        // Backup should contain the first version (empty addons)
        let backup: MetadataStore =
            serde_json::from_str(&fs::read_to_string(&bak).unwrap()).unwrap();
        assert!(backup.addons.is_empty());
    }

    #[test]
    fn installed_at_refreshes_on_real_rerecord() {
        let mut store = MetadataStore::default();
        record_install(&mut store, "Addon", 1, "1.0", "url");

        // A real install/update is a download, so `installed_at` (the local
        // "last downloaded" time) is refreshed. Pin a known old value, re-record,
        // and confirm it moved forward while other fields also updated.
        store.addons.get_mut("Addon").unwrap().installed_at = "2020-01-01T00:00:00Z".to_string();
        record_install_ext(&mut store, "Addon", 1, "2.0", "url2", 12345);

        let meta = &store.addons["Addon"];
        assert_ne!(meta.installed_at, "2020-01-01T00:00:00Z");
        assert!(meta.installed_at.starts_with("20"));
        assert_eq!(meta.installed_version, "2.0");
        assert_eq!(meta.esoui_last_update, 12345);
    }

    #[test]
    fn reconcile_addon_preserves_install_time_and_tags() {
        let mut store = MetadataStore::default();
        record_install(&mut store, "Addon", 0, "1.0", "url");
        let meta = store.addons.get_mut("Addon").unwrap();
        meta.installed_at = "2020-01-01T00:00:00Z".to_string();
        meta.tags = vec!["favorite".to_string()];

        // Auto-link reconciliation: ESOUI publishes a newer version the user has
        // NOT downloaded. The local download time and tags must not move; only
        // the API-derived fields change.
        reconcile_addon(store.addons.get_mut("Addon").unwrap(), 555, 999, "new-url");

        let meta = &store.addons["Addon"];
        assert_eq!(meta.installed_at, "2020-01-01T00:00:00Z");
        assert_eq!(meta.tags, vec!["favorite".to_string()]);
        assert_eq!(meta.esoui_id, 555);
        assert_eq!(meta.esoui_last_update, 999);
        // download_url was non-empty, so it is preserved (not clobbered).
        assert_eq!(meta.download_url, "url");
    }

    #[test]
    fn reconcile_addon_replaces_legacy_parent_url_when_healing_id_zero() {
        let mut store = MetadataStore::default();
        record_bundled_folder(&mut store, "LibFoo", 3, "https://esoui/parent", "1.0");

        reconcile_addon(
            store.addons.get_mut("LibFoo").unwrap(),
            7,
            999,
            "https://esoui/lib-foo",
        );

        let meta = &store.addons["LibFoo"];
        assert_eq!(meta.esoui_id, 7);
        assert_eq!(meta.download_url, "https://esoui/lib-foo");
    }

    #[test]
    fn record_and_remove_entry() {
        let mut store = MetadataStore::default();

        record_install(&mut store, "Addon1", 10, "1.0", "url1");
        record_install(&mut store, "Addon2", 20, "2.0", "url2");
        assert_eq!(store.addons.len(), 2);

        remove_entry(&mut store, "Addon1");
        assert_eq!(store.addons.len(), 1);
        assert!(!store.addons.contains_key("Addon1"));
        assert!(store.addons.contains_key("Addon2"));
    }

    #[test]
    fn format_timestamp_produces_valid_iso8601() {
        // 2024-01-01T00:00:00Z
        let ts = format_timestamp(1704067200);
        assert_eq!(ts, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn load_recovers_from_tmp_when_primary_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.json");
        let tmp_file = tmp.path().join("test.json.tmp");

        // Simulate crash: .tmp exists (completed write) but primary was deleted
        let mut store = MetadataStore::default();
        record_install(
            &mut store,
            "CrashRecovered",
            99,
            "3.0.0",
            "https://example.com",
        );
        let json = serde_json::to_string(&store).unwrap();
        fs::write(&tmp_file, &json).unwrap();

        // Primary does NOT exist — .tmp should be recovered and promoted
        let loaded: MetadataStore = load_json_with_backup(&path);
        assert_eq!(loaded.addons.len(), 1);
        assert_eq!(loaded.addons["CrashRecovered"].esoui_id, 99);

        // .tmp should be promoted to primary
        assert!(path.exists());
        assert!(!tmp_file.exists());
    }

    #[test]
    fn load_recovers_from_tmp_over_corrupted_primary() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.json");
        let tmp_file = tmp.path().join("test.json.tmp");

        // Simulate crash: primary is corrupted, .tmp has the latest data
        fs::write(&path, "corrupted json{{{").unwrap();

        let mut store = MetadataStore::default();
        record_install(&mut store, "LatestData", 77, "5.0.0", "https://example.com");
        let json = serde_json::to_string(&store).unwrap();
        fs::write(&tmp_file, &json).unwrap();

        // Should prefer .tmp (newest data) over .bak
        let loaded: MetadataStore = load_json_with_backup(&path);
        assert_eq!(loaded.addons.len(), 1);
        assert_eq!(loaded.addons["LatestData"].esoui_id, 77);

        // Corrupt primary should be replaced by promoted .tmp
        assert!(path.exists());
        let reloaded: MetadataStore = load_json_with_backup(&path);
        assert_eq!(reloaded.addons["LatestData"].esoui_id, 77);
    }

    #[test]
    fn save_is_atomic_via_temp_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.json");
        let tmp_path = tmp.path().join("test.json.tmp");

        let store = MetadataStore::default();
        save_json_with_backup(&path, &store).unwrap();

        // Temp file should not remain
        assert!(!tmp_path.exists());
        // Main file should exist
        assert!(path.exists());
    }
}
