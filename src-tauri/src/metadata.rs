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
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

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
            version: 1,
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
    let mut store: MetadataStore = match transaction {
        Ok(_guard) => load_json_with_backup(&path),
        Err(error) => {
            // A timed-out read must not run recovery writes without ownership.
            // Read committed primary/backup data only and leave recovery to a
            // later lock holder.
            eprintln!("Warning: {error}");
            load_json_read_only(&path)
        }
    };
    store.baseline_addons = Some(store.addons.clone());
    store.baseline_version = Some(store.version);
    store
}

fn load_json_read_only<T: DeserializeOwned + Default>(path: &Path) -> T {
    for candidate in [path.to_path_buf(), path.with_extension("json.bak")] {
        if let Ok(content) = fs::read_to_string(candidate) {
            if let Ok(data) = serde_json::from_str(&content) {
                return data;
            }
        }
    }
    T::default()
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
    store.addons.insert(
        folder_name.to_string(),
        AddonMetadata {
            esoui_id,
            installed_version: version.to_string(),
            download_url: download_url.to_string(),
            installed_at,
            tags: existing_tags,
            esoui_last_update: last_update,
        },
    );
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
    if esoui_id > 0 {
        meta.esoui_id = esoui_id;
    }
    if meta.download_url.is_empty() && !download_url.is_empty() {
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
