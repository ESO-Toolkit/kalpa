use crate::manifest::AddonManifest;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Open (or create) the manifest cache database in the app data directory.
/// Stored outside the ESO AddOns folder so the game's recursive folder scanner
/// doesn't encounter unexpected files.
fn open_cache(cache_dir: &Path) -> Result<Connection, rusqlite::Error> {
    // Ensure the app data directory exists (Tauri doesn't guarantee it on first run)
    let _ = std::fs::create_dir_all(cache_dir);
    let db_path = cache_dir.join("manifest-cache.db");
    let conn = Connection::open(db_path)?;
    // Cache rows are disposable. Rebuild the old mtime-only schema rather than
    // accepting entries that cannot prove which file contents were parsed.
    let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version != 1 {
        conn.execute_batch(
            "BEGIN;
             DROP TABLE IF EXISTS manifest_cache;
             CREATE TABLE manifest_cache (
                 folder_name TEXT NOT NULL,
                 manifest_path TEXT NOT NULL,
                 content_hash BLOB NOT NULL,
                 data TEXT NOT NULL,
                 PRIMARY KEY (folder_name, manifest_path)
             );
             PRAGMA user_version = 1;
             COMMIT;",
        )?;
    }
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
    Ok(conn)
}

/// Identify the actual manifest and its bytes. Archive extraction can preserve
/// timestamps even when a manifest changes, so mtime and size are insufficient.
fn read_manifest_source(path: &Path) -> Option<(String, Vec<u8>)> {
    let path = std::fs::canonicalize(path).ok()?;
    let bytes = std::fs::read(&path).ok()?;
    Some((path.to_str()?.to_owned(), bytes))
}

/// Try to load a cached manifest if its path and contents match.
pub fn parse_manifest_cached(
    conn: &Connection,
    folder_name: &str,
    manifest_path: &Path,
) -> Option<AddonManifest> {
    let (path, bytes) = read_manifest_source(manifest_path)?;
    let content_hash = Sha256::digest(&bytes).to_vec();

    let mut stmt = conn
        .prepare_cached(
            "SELECT data FROM manifest_cache WHERE folder_name = ?1 AND manifest_path = ?2 AND content_hash = ?3",
        )
        .ok()?;
    let data: String = stmt
        .query_row(rusqlite::params![folder_name, path, content_hash], |row| {
            row.get(0)
        })
        .ok()?;
    serde_json::from_str(&data).ok()
}

/// Cache the current manifest bytes, preserving the scan's disabled state.
pub fn store_parsed(
    conn: &Connection,
    folder_name: &str,
    manifest_path: &Path,
    manifest: &AddonManifest,
) {
    let Some((path, bytes)) = read_manifest_source(manifest_path) else {
        return;
    };
    // The file may have changed since the scan parsed it. Parse the same bytes
    // we hash so a concurrent replacement cannot stamp old data with a new hash.
    let mut current = crate::manifest::parse_manifest_bytes(folder_name, &bytes);
    current.disabled = manifest.disabled;
    let content_hash = Sha256::digest(&bytes).to_vec();
    if let Ok(json) = serde_json::to_string(&current) {
        let _ = conn.execute(
            "INSERT OR REPLACE INTO manifest_cache (folder_name, manifest_path, content_hash, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![folder_name, path, content_hash, json],
        );
    }
}

/// Remove stale entries from the cache for folders that no longer exist.
/// Batches deletes in groups of 500 to stay under SQLite's 999-parameter limit.
fn prune_stale(conn: &Connection, existing_folders: &[&str]) {
    if existing_folders.is_empty() {
        let _ = conn.execute("DELETE FROM manifest_cache", []);
        return;
    }

    // Use a temp table to hold existing folder names, then delete rows not in it.
    // This avoids the SQLITE_LIMIT_VARIABLE_NUMBER (999) cap for large addon lists.
    let _ = conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS _existing_folders (name TEXT PRIMARY KEY);
         DELETE FROM _existing_folders;",
    );

    for chunk in existing_folders.chunks(500) {
        let placeholders: Vec<&str> = chunk.iter().map(|_| "(?)").collect();
        let sql = format!(
            "INSERT INTO _existing_folders (name) VALUES {}",
            placeholders.join(",")
        );
        let params: Vec<&dyn rusqlite::types::ToSql> = chunk
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let _ = conn.execute(&sql, params.as_slice());
    }

    let _ = conn.execute(
        "DELETE FROM manifest_cache WHERE folder_name NOT IN (SELECT name FROM _existing_folders)",
        [],
    );
}

/// Open the cache and prune stale entries. Returns the connection for use
/// during the scan. If the cache can't be opened, returns None (caller
/// should fall back to uncached parsing).
pub fn open_and_prune(cache_dir: &Path, existing_folders: &[&str]) -> Option<Connection> {
    let conn = open_cache(cache_dir).ok()?;
    prune_stale(&conn, existing_folders);
    Some(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::parse_manifest;
    use std::fs::{self, File, FileTimes};

    fn write_manifest(root: &Path, version: &str) -> std::path::PathBuf {
        let folder = root.join("Example");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("Example.txt");
        fs::write(&path, format!("## Title: Example\n## Version: {version}\n")).unwrap();
        path
    }

    #[test]
    fn cache_hit_requires_the_same_manifest_path() {
        let tmp = tempfile::tempdir().unwrap();
        let first = write_manifest(&tmp.path().join("live"), "1.0");
        let second = write_manifest(&tmp.path().join("pts"), "2.0");
        let modified = fs::metadata(&first).unwrap().modified().unwrap();
        File::options()
            .write(true)
            .open(&second)
            .unwrap()
            .set_times(FileTimes::new().set_modified(modified))
            .unwrap();
        let conn = open_cache(&tmp.path().join("cache")).unwrap();
        store_parsed(
            &conn,
            "Example",
            &first,
            &parse_manifest("Example", &first).unwrap(),
        );
        assert_eq!(
            parse_manifest_cached(&conn, "Example", &first)
                .unwrap()
                .version,
            "1.0"
        );
        assert!(parse_manifest_cached(&conn, "Example", &second).is_none());
        store_parsed(
            &conn,
            "Example",
            &second,
            &parse_manifest("Example", &second).unwrap(),
        );
        assert_eq!(
            parse_manifest_cached(&conn, "Example", &second)
                .unwrap()
                .version,
            "2.0"
        );
        assert_eq!(
            parse_manifest_cached(&conn, "Example", &first)
                .unwrap()
                .version,
            "1.0"
        );
    }

    #[test]
    fn same_size_overwrite_with_preserved_mtime_is_a_cache_miss() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_manifest(tmp.path(), "1.0");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let conn = open_cache(&tmp.path().join("cache")).unwrap();
        store_parsed(
            &conn,
            "Example",
            &path,
            &parse_manifest("Example", &path).unwrap(),
        );
        write_manifest(tmp.path(), "2.0");
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(modified))
            .unwrap();
        assert!(parse_manifest_cached(&conn, "Example", &path).is_none());
        let current = parse_manifest("Example", &path).unwrap();
        assert_eq!(current.version, "2.0");
        store_parsed(&conn, "Example", &path, &current);
        assert_eq!(
            parse_manifest_cached(&conn, "Example", &path)
                .unwrap()
                .version,
            "2.0"
        );
    }

    #[test]
    fn replacement_between_parse_and_store_cannot_cache_old_data() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_manifest(tmp.path(), "1.0");
        let mut earlier = parse_manifest("Example", &path).unwrap();
        earlier.disabled = true;
        write_manifest(tmp.path(), "2.0");
        let conn = open_cache(&tmp.path().join("cache")).unwrap();
        store_parsed(&conn, "Example", &path, &earlier);
        let cached = parse_manifest_cached(&conn, "Example", &path).unwrap();
        assert_eq!(cached.version, "2.0");
        assert!(cached.disabled);
    }

    #[test]
    fn old_mtime_schema_is_rebuilt_without_reusing_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_manifest(tmp.path(), "2.0");
        let cache_dir = tmp.path().join("cache");
        fs::create_dir(&cache_dir).unwrap();
        let old = Connection::open(cache_dir.join("manifest-cache.db")).unwrap();
        old.execute_batch(
            "CREATE TABLE manifest_cache (
                folder_name TEXT PRIMARY KEY, mtime_secs INTEGER NOT NULL,
                mtime_nanos INTEGER NOT NULL, data TEXT NOT NULL
             );
             INSERT INTO manifest_cache VALUES ('Example', 0, 0, '{}');",
        )
        .unwrap();
        drop(old);
        let conn = open_and_prune(&cache_dir, &["Example"]).unwrap();
        assert!(parse_manifest_cached(&conn, "Example", &path).is_none());
        store_parsed(
            &conn,
            "Example",
            &path,
            &parse_manifest("Example", &path).unwrap(),
        );
        drop(conn);
        let reopened = open_and_prune(&cache_dir, &["Example"]).unwrap();
        assert_eq!(
            parse_manifest_cached(&reopened, "Example", &path)
                .unwrap()
                .version,
            "2.0"
        );
    }

    #[test]
    fn cache_failure_allows_uncached_parsing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_manifest(tmp.path(), "2.0");
        let blocked_cache = tmp.path().join("not-a-directory");
        fs::write(&blocked_cache, "occupied").unwrap();
        assert!(open_and_prune(&blocked_cache, &["Example"]).is_none());
        assert_eq!(parse_manifest("Example", &path).unwrap().version, "2.0");

        let conn = open_cache(&tmp.path().join("cache")).unwrap();
        store_parsed(
            &conn,
            "Example",
            &path,
            &parse_manifest("Example", &path).unwrap(),
        );
        conn.execute("UPDATE manifest_cache SET data = 'invalid json'", [])
            .unwrap();
        let parsed = parse_manifest_cached(&conn, "Example", &path)
            .or_else(|| parse_manifest("Example", &path))
            .unwrap();
        assert_eq!(parsed.version, "2.0");
    }
}
