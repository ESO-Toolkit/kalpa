use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Whether `esoui_last_update` is known to belong to the artifact installed
    /// locally. Metadata written before this flag existed only recorded an API
    /// observation, so it must not suppress a version mismatch until a matching
    /// observation or real install establishes the marker's provenance.
    #[serde(default, skip_serializing_if = "is_false")]
    pub esoui_marker_installed: bool,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

fn is_false(v: &bool) -> bool {
    !*v
}

const CURRENT_METADATA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataStore {
    pub version: u32,
    pub addons: HashMap<String, AddonMetadata>,
    /// In-memory baseline used to turn a potentially long-running operation's
    /// result into a short, lock-protected mutation against the latest disk
    /// state. Never serialized; the persisted JSON shape is unchanged.
    #[serde(skip)]
    baseline_addons: Option<HashMap<String, AddonMetadata>>,
    #[serde(skip)]
    baseline_version: Option<u32>,
}

impl Default for MetadataStore {
    fn default() -> Self {
        Self {
            version: CURRENT_METADATA_VERSION,
            addons: HashMap::new(),
            baseline_addons: None,
            baseline_version: None,
        }
    }
}

fn metadata_path(addons_path: &Path) -> std::path::PathBuf {
    addons_path.join("kalpa.json")
}

/// Load a JSON file with automatic recovery from crash artifacts.
///
/// Recovery order when the primary file is missing or corrupted:
/// 1. Legacy `.json.tmp` — a completed write made by an older Kalpa version
///    that was never renamed into place. New writes use unique staging names
///    and never treat those uncommitted leftovers as recovery candidates.
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

    // Primary missing or corrupted — preserve compatibility with the fixed
    // staging artifact written by older Kalpa versions. New unique staging
    // leftovers are deliberately ignored because they were never committed.
    let tmp = path.with_extension("json.tmp");
    if let Ok(content) = fs::read_to_string(&tmp) {
        if let Ok(data) = serde_json::from_str::<T>(&content) {
            eprintln!("Recovered data from incomplete write {}.", tmp.display());
            // Publish through the shared writer so replacing a corrupt primary
            // never opens a remove-before-rename gap. Remove only this known
            // legacy artifact after the complete primary has been published.
            if let Err(e) = crate::atomic_file::atomic_write(path, content.as_bytes()) {
                eprintln!(
                    "Warning: could not promote {} to primary: {e}",
                    tmp.display()
                );
            } else if let Err(e) = fs::remove_file(&tmp) {
                eprintln!("Warning: could not remove legacy {}: {e}", tmp.display());
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

/// Load the primary, legacy recovery file, or backup without modifying disk.
///
/// This preserves the same recovery order as [`load_json_with_backup`] for
/// callers that hold only a shared lock or have no write access. In particular,
/// a valid legacy `.json.tmp` may be observed but is never promoted or removed.
pub(crate) fn load_json_read_only_with_backup<T: DeserializeOwned + Default>(path: &Path) -> T {
    for candidate in [
        path.to_path_buf(),
        path.with_extension("json.tmp"),
        path.with_extension("json.bak"),
    ] {
        if let Ok(content) = fs::read_to_string(candidate) {
            if let Ok(data) = serde_json::from_str(&content) {
                return data;
            }
        }
    }
    T::default()
}

/// Save data as JSON with atomic write and automatic backup.
///
/// Writes and fsyncs a uniquely owned sibling staging file, copies the current
/// primary to `.json.bak`, then atomically publishes the replacement. The
/// ordering matters: copying to `.bak`
/// first would overwrite the last good backup with a possibly-corrupt primary
/// before the replacement was safe on disk — destroying the very copy the `.bak`
/// recovery in [`load_json_with_backup`] exists to provide.
pub fn save_json_with_backup<T: Serialize>(path: &Path, data: &T) -> Result<(), String> {
    let json =
        serde_json::to_string_pretty(data).map_err(|e| format!("Failed to serialize: {e}"))?;

    let mut replacement = crate::atomic_file::AtomicFile::create(path)
        .map_err(|e| format!("Failed to create temp file: {e}"))?;
    replacement
        .write_all(json.as_bytes())
        .map_err(|e| format!("Failed to write temp file: {e}"))?;

    // commit_with flushes and syncs the replacement before this callback. Only
    // then is the previous primary copied. The backup itself uses the same
    // crash-safe publisher, so a failed refresh preserves the old backup.
    let bak = path.with_extension("json.bak");
    replacement
        .commit_with(|_| {
            if path.is_file() {
                // Backup refresh has historically been best-effort. Preserve
                // that behavior while using atomic publication so a failed
                // refresh cannot truncate the previously valid backup.
                if let Ok(previous) = fs::read(path) {
                    let _ = crate::atomic_file::atomic_write(&bak, &previous);
                }
            }
            Ok(())
        })
        .map_err(|e| format!("Failed to finalize write: {e}"))
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
    let path = metadata_path(addons_path);
    let transaction =
        crate::transaction_lock::acquire(&path, crate::transaction_lock::LockOptions::default());
    let (mut store, baseline_addons, baseline_version) = match transaction {
        Ok(_guard) => {
            let mut store: MetadataStore = load_json_with_backup(&path);
            let migrated = migrate_metadata(&mut store);
            if migrated {
                if let Err(e) = save_json_with_backup(&path, &store) {
                    eprintln!(
                        "Warning: failed to persist metadata migration for {}: {e}",
                        path.display()
                    );
                }
            }
            let baseline_addons = store.addons.clone();
            let baseline_version = store.version;
            (store, baseline_addons, baseline_version)
        }
        Err(error) => {
            // A timed-out read must not run recovery writes without ownership.
            // Read committed primary/backup data only and leave recovery to a
            // later lock holder.
            eprintln!("Warning: {error}");
            let mut store: MetadataStore = load_json_read_only_with_backup(&path);
            let baseline_addons = store.addons.clone();
            let baseline_version = store.version;
            migrate_metadata(&mut store);
            (store, baseline_addons, baseline_version)
        }
    };
    store.baseline_addons = Some(baseline_addons);
    store.baseline_version = Some(baseline_version);
    store
}

fn migrate_metadata(store: &mut MetadataStore) -> bool {
    // Version 1 stored every filelist marker as if it belonged to the local
    // artifact, even when the user deferred the corresponding update. Keep the
    // value for diagnostics/relearning, but do not let it veto a version
    // mismatch until a post-migration install or matching observation confirms
    // its provenance.
    if store.version < CURRENT_METADATA_VERSION {
        for addon in store.addons.values_mut() {
            addon.esoui_marker_installed = false;
        }
        store.version = CURRENT_METADATA_VERSION;
        return true;
    }
    false
}

pub fn save_metadata(addons_path: &Path, store: &MetadataStore) -> Result<(), String> {
    let path = metadata_path(addons_path);
    let _transaction =
        crate::transaction_lock::acquire(&path, crate::transaction_lock::LockOptions::default())
            .map_err(|error| error.to_string())?;

    let Some(baseline) = store.baseline_addons.as_ref() else {
        return save_json_with_backup(&path, store);
    };

    // Expensive download/extract/hash work happens before this function. Under
    // the OS lock, reload the latest store and apply only fields this caller
    // changed from its baseline, then publish atomically. This makes the actual
    // disk read -> mutate -> write transaction short while preserving unrelated
    // changes made by the other shell during the long operation.
    let mut latest: MetadataStore = load_json_with_backup(&path);
    if store
        .baseline_version
        .is_some_and(|baseline| baseline != store.version)
    {
        latest.version = store.version;
    }
    let names: std::collections::HashSet<&String> =
        baseline.keys().chain(store.addons.keys()).collect();
    for name in names {
        match (baseline.get(name), store.addons.get(name)) {
            (Some(_before), None) => {
                latest.addons.remove(name);
            }
            (None, Some(after)) => match latest.addons.entry(name.clone()) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(after.clone());
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    apply_changed_fields(entry.get_mut(), &empty_metadata(), after);
                }
            },
            (Some(before), Some(after)) if before != after => {
                let current = latest
                    .addons
                    .entry(name.clone())
                    .or_insert_with(|| before.clone());
                apply_changed_fields(current, before, after);
            }
            _ => {}
        }
    }
    latest.baseline_addons = None;
    latest.baseline_version = None;
    save_json_with_backup(&path, &latest)
}

fn empty_metadata() -> AddonMetadata {
    AddonMetadata {
        esoui_id: 0,
        installed_version: String::new(),
        download_url: String::new(),
        installed_at: String::new(),
        tags: Vec::new(),
        esoui_last_update: 0,
        bundled_by: Vec::new(),
        esoui_marker_installed: false,
    }
}

fn apply_changed_fields(
    current: &mut AddonMetadata,
    before: &AddonMetadata,
    after: &AddonMetadata,
) {
    if before.esoui_id != after.esoui_id {
        current.esoui_id = after.esoui_id;
    }
    if before.installed_version != after.installed_version {
        current.installed_version = after.installed_version.clone();
    }
    if before.download_url != after.download_url {
        current.download_url = after.download_url.clone();
    }
    if before.installed_at != after.installed_at {
        current.installed_at = after.installed_at.clone();
    }
    if before.tags != after.tags {
        current.tags = after.tags.clone();
    }
    if before.esoui_last_update != after.esoui_last_update {
        current.esoui_last_update = after.esoui_last_update;
    }
    if before.bundled_by != after.bundled_by {
        current.bundled_by = after.bundled_by.clone();
    }
    if before.esoui_marker_installed != after.esoui_marker_installed {
        current.esoui_marker_installed = after.esoui_marker_installed;
    }
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
    // The filelist is eventually consistent with filedetails. When this
    // install has a marker, preserve the greatest marker already proven to
    // belong to an installed artifact so a lagging response cannot make that
    // artifact look older than it is. Observation-only markers from migrated
    // metadata do not belong to the artifact being installed and must be
    // replaced by the download's marker. A zero marker means this install has
    // no proven ESOUI publication identity (manual imports and dependency
    // installs use this path), so an older artifact's marker must not be
    // inherited.
    let last_update = if esoui_last_update > 0 {
        existing
            .filter(|m| m.esoui_marker_installed)
            .map(|m| m.esoui_last_update.max(esoui_last_update))
            .unwrap_or(esoui_last_update)
    } else {
        0
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
    // A real-ID entry can still be separately tracked while an archive also
    // ships its files. In that case `bundled_by` is live provenance, not stale
    // state, so keep it when refreshing or re-recording that sibling. A zero
    // ID cannot assert ownership and likewise preserves existing provenance.
    // The only conversion that clears provenance is a genuinely bundled (ID 0)
    // entry becoming a standalone primary install.
    let bundled_by = existing
        .filter(|m| esoui_id == 0 || m.esoui_id != 0)
        .map(|m| m.bundled_by.clone())
        .unwrap_or_default();
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
            esoui_marker_installed: esoui_last_update > 0,
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

    let (esoui_id, download_url, esoui_last_update, esoui_marker_installed) = if separately_tracked
    {
        let owner = existing.expect("separately_tracked implies an existing entry");
        (
            owner.esoui_id,
            owner.download_url.clone(),
            owner.esoui_last_update,
            owner.esoui_marker_installed,
        )
    } else {
        (0, parent_url.to_string(), 0, false)
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
            esoui_marker_installed,
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
    reconcile_addon_identity(meta, esoui_id, download_url);

    // Reconciliation can observe an older filelist entry than the marker
    // captured when the artifact was downloaded. Never regress the marker.
    meta.esoui_last_update = meta.esoui_last_update.max(esoui_last_update);
    meta.esoui_marker_installed = true;
}

/// Reconcile API identity fields without attaching a publication marker.
/// Auto-link uses this when the observed API version differs from the local
/// artifact: linking an addon must not make a pending update look installed.
pub fn reconcile_addon_identity(meta: &mut AddonMetadata, esoui_id: u32, download_url: &str) {
    let was_bundled = meta.esoui_id == 0 && !meta.bundled_by.is_empty();
    if esoui_id > 0 {
        meta.esoui_id = esoui_id;
    }
    if (was_bundled || meta.download_url.is_empty()) && !download_url.is_empty() {
        meta.download_url = download_url.to_string();
    }
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

        // An ID-0/manual record cannot disprove that archive ownership.
        record_install_ext(&mut store, "LibFoo", 0, "1.0", "https://esoui/addon-a", 0);
        assert_eq!(store.addons.get("LibFoo").unwrap().bundled_by, vec![3]);

        // A real standalone identity is the deliberate conversion point.
        record_install_ext(&mut store, "LibFoo", 7, "1.5", "https://esoui/lib-foo", 0);
        assert!(store.addons.get("LibFoo").unwrap().bundled_by.is_empty());
    }

    /// This is the standalone ID 7 -> parent ID 3 -> standalone ID 7
    /// regression: refreshing the library must retain the parent's provenance.
    #[test]
    fn recording_a_separately_tracked_primary_preserves_parent_provenance() {
        let mut store = MetadataStore::default();
        record_install_ext(&mut store, "LibFoo", 7, "1.5", "https://esoui/lib-foo", 0);
        record_bundled_folder(&mut store, "LibFoo", 3, "https://esoui/addon-a", "1.2");

        record_install_ext(&mut store, "LibFoo", 7, "1.5", "https://esoui/lib-foo", 0);

        let lib = store.addons.get("LibFoo").unwrap();
        assert_eq!(lib.esoui_id, 7);
        assert_eq!(lib.bundled_by, vec![3]);
    }

    /// Updating a separately tracked sibling must not erase the archives that
    /// still own the copy on disk when there are multiple parent archives.
    #[test]
    fn recording_a_separately_tracked_primary_preserves_multiple_parents() {
        let mut store = MetadataStore::default();
        record_install_ext(&mut store, "LibFoo", 7, "1.5", "https://esoui/lib-foo", 0);
        record_bundled_folder(&mut store, "LibFoo", 3, "https://esoui/addon-a", "1.2");
        record_bundled_folder(&mut store, "LibFoo", 9, "https://esoui/addon-b", "1.3");

        record_install_ext(&mut store, "LibFoo", 7, "1.5", "https://esoui/lib-foo", 0);

        let lib = store.addons.get("LibFoo").unwrap();
        assert_eq!(lib.esoui_id, 7);
        assert_eq!(lib.bundled_by, vec![3, 9]);
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
        assert_eq!(loaded.version, CURRENT_METADATA_VERSION);
        assert_eq!(loaded.addons.len(), 1);
        assert_eq!(loaded.addons["TestAddon"].esoui_id, 123);
        assert_eq!(loaded.addons["TestAddon"].installed_version, "1.0.0");
    }

    #[test]
    fn load_returns_default_for_missing_file() {
        let loaded: MetadataStore = load_json_with_backup(Path::new("/nonexistent/path.json"));
        assert_eq!(loaded.version, CURRENT_METADATA_VERSION);
        assert!(loaded.addons.is_empty());
    }

    #[test]
    fn load_metadata_migrates_observation_only_markers() {
        let tmp = tempfile::tempdir().unwrap();
        let path = metadata_path(tmp.path());

        // Version 1 metadata could not distinguish a filelist observation
        // from a marker belonging to the artifact installed on disk.
        fs::write(
            &path,
            r#"{
                "version": 1,
                "addons": {
                    "Addon": {
                        "esouiId": 7,
                        "installedVersion": "v1",
                        "downloadUrl": "https://example.invalid/addon.zip",
                        "installedAt": "",
                        "esouiLastUpdate": 42
                    }
                }
            }"#,
        )
        .unwrap();

        let loaded = load_metadata(tmp.path());
        assert_eq!(loaded.version, CURRENT_METADATA_VERSION);
        assert_eq!(loaded.addons["Addon"].esoui_last_update, 42);
        assert!(!loaded.addons["Addon"].esoui_marker_installed);

        // Migration is persisted so a later process does not re-enter the
        // ambiguous legacy state.
        let persisted: MetadataStore = load_json_with_backup(&path);
        assert_eq!(persisted.version, CURRENT_METADATA_VERSION);
        assert!(!persisted.addons["Addon"].esoui_marker_installed);
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

        struct SerializationFailure;
        impl serde::Serialize for SerializationFailure {
            fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                Err(<S::Error as serde::ser::Error>::custom(
                    "injected serialization failure",
                ))
            }
        }
        assert!(save_json_with_backup(&path, &SerializationFailure).is_err());

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
    fn concurrent_saves_never_share_a_staging_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = std::sync::Arc::new(tmp.path().join("test.json"));
        let start = std::sync::Arc::new(std::sync::Barrier::new(8));

        let threads: Vec<_> = (0..8)
            .map(|writer| {
                let path = path.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    for iteration in 0..100 {
                        let value = serde_json::json!({
                            "writer": writer,
                            "iteration": iteration,
                        });
                        save_json_with_backup(path.as_ref(), &value)?;
                    }
                    Ok::<(), String>(())
                })
            })
            .collect();

        for thread in threads {
            thread.join().unwrap().unwrap();
        }
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(path.as_ref()).unwrap()).unwrap();
        assert!(value.get("writer").is_some());
        assert!(value.get("iteration").is_some());
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
    fn publication_marker_never_regresses() {
        let mut store = MetadataStore::default();
        record_install_ext(&mut store, "Addon", 1, "2.0", "url", 200);
        record_install_ext(&mut store, "Addon", 1, "2.0", "url", 100);
        assert_eq!(store.addons["Addon"].esoui_last_update, 200);

        reconcile_addon(store.addons.get_mut("Addon").unwrap(), 1, 50, "url");
        assert_eq!(store.addons["Addon"].esoui_last_update, 200);
    }

    #[test]
    fn install_replaces_an_observation_only_marker() {
        let mut store = MetadataStore::default();
        record_install_ext(&mut store, "Addon", 1, "1.0", "url", 300);
        store
            .addons
            .get_mut("Addon")
            .unwrap()
            .esoui_marker_installed = false;

        record_install_ext(&mut store, "Addon", 1, "2.0", "url", 200);

        let meta = &store.addons["Addon"];
        assert_eq!(meta.esoui_last_update, 200);
        assert!(meta.esoui_marker_installed);
    }

    #[test]
    fn install_without_marker_clears_an_older_artifacts_provenance() {
        let mut store = MetadataStore::default();
        record_install_ext(&mut store, "Addon", 1, "2.0", "url", 200);

        record_install(&mut store, "Addon", 1, "1.0", "manual-url");

        let meta = &store.addons["Addon"];
        assert_eq!(meta.installed_version, "1.0");
        assert_eq!(meta.esoui_last_update, 0);
        assert!(!meta.esoui_marker_installed);
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
    fn load_never_promotes_an_uncommitted_unique_staging_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.json");
        let staging = tmp.path().join("test.json.tmp-999-1-deadbeef");
        let bak = tmp.path().join("test.json.bak");
        fs::write(&path, "corrupted").unwrap();

        let mut uncommitted = MetadataStore::default();
        record_install(&mut uncommitted, "Uncommitted", 1, "1.0", "url");
        fs::write(&staging, serde_json::to_vec(&uncommitted).unwrap()).unwrap();

        let mut backup = MetadataStore::default();
        record_install(&mut backup, "Backup", 2, "2.0", "url");
        fs::write(&bak, serde_json::to_vec(&backup).unwrap()).unwrap();

        let loaded: MetadataStore = load_json_with_backup(&path);
        assert!(loaded.addons.contains_key("Backup"));
        assert!(!loaded.addons.contains_key("Uncommitted"));
        assert!(
            staging.exists(),
            "load must not claim another writer's staging"
        );
    }

    #[test]
    fn concurrent_metadata_writers_preserve_distinct_addons() {
        let dir = tempfile::tempdir().unwrap();
        save_metadata(dir.path(), &MetadataStore::default()).unwrap();
        let mut left = load_metadata(dir.path());
        let mut right = load_metadata(dir.path());
        record_install(&mut left, "Left", 1, "1", "left");
        record_install(&mut right, "Right", 2, "1", "right");
        let root_a = dir.path().to_path_buf();
        let root_b = root_a.clone();
        let a = std::thread::spawn(move || save_metadata(&root_a, &left));
        let b = std::thread::spawn(move || save_metadata(&root_b, &right));
        a.join().unwrap().unwrap();
        b.join().unwrap().unwrap();
        let saved = load_metadata(dir.path());
        assert!(saved.addons.contains_key("Left"));
        assert!(saved.addons.contains_key("Right"));
    }

    #[test]
    fn concurrent_metadata_writers_merge_independent_fields() {
        let dir = tempfile::tempdir().unwrap();
        let mut initial = MetadataStore::default();
        record_install(&mut initial, "Addon", 1, "1", "url");
        save_metadata(dir.path(), &initial).unwrap();
        let mut tags = load_metadata(dir.path());
        let mut version = load_metadata(dir.path());
        tags.addons.get_mut("Addon").unwrap().tags = vec!["favorite".to_string()];
        version.addons.get_mut("Addon").unwrap().installed_version = "2".to_string();
        let root_a = dir.path().to_path_buf();
        let root_b = root_a.clone();
        let a = std::thread::spawn(move || save_metadata(&root_a, &tags));
        let b = std::thread::spawn(move || save_metadata(&root_b, &version));
        a.join().unwrap().unwrap();
        b.join().unwrap().unwrap();
        let saved = load_metadata(dir.path());
        assert_eq!(saved.addons["Addon"].tags, vec!["favorite".to_string()]);
        assert_eq!(saved.addons["Addon"].installed_version, "2");
    }

    #[test]
    fn concurrent_first_writers_merge_identity_and_tags() {
        let dir = tempfile::tempdir().unwrap();
        save_metadata(dir.path(), &MetadataStore::default()).unwrap();
        let mut install = load_metadata(dir.path());
        let mut tags = load_metadata(dir.path());
        record_install(&mut install, "Addon", 42, "1", "url");
        let mut tagged = empty_metadata();
        tagged.tags = vec!["favorite".to_string()];
        tags.addons.insert("Addon".to_string(), tagged);
        let root_a = dir.path().to_path_buf();
        let root_b = root_a.clone();
        let a = std::thread::spawn(move || save_metadata(&root_a, &install));
        let b = std::thread::spawn(move || save_metadata(&root_b, &tags));
        a.join().unwrap().unwrap();
        b.join().unwrap().unwrap();
        let saved = load_metadata(dir.path());
        assert_eq!(saved.addons["Addon"].esoui_id, 42);
        assert_eq!(saved.addons["Addon"].tags, vec!["favorite".to_string()]);
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
