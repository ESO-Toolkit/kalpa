//! Minion 3/4 installed-addon data shared by the desktop app and native sidecar.

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine};
use regex::Regex;
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MinionAddon {
    pub uid: u32,
    pub version: String,
    pub folders: Vec<String>,
    #[serde(skip)]
    pub addons_path: Option<PathBuf>,
}

pub(crate) fn find_minion_data_in(minion_dir: &Path) -> Option<PathBuf> {
    // Minion 4 imports the old XML once and then leaves it stale. Never use
    // that imported snapshot when the current ESO database is present.
    [
        minion_dir.join("config/eso.json"),
        minion_dir.join("minion.xml"),
    ]
    .into_iter()
    .find(|path| path.exists())
}

pub fn find_minion_data() -> Option<PathBuf> {
    if let Some(fixtures) = crate::app_profile::e2e_fixture_dir() {
        return find_minion_data_in(&fixtures.join("minion"));
    }
    dirs::home_dir().and_then(|home| find_minion_data_in(&home.join(".minion")))
}

pub fn load_minion_addons() -> Result<Vec<MinionAddon>, String> {
    let path = find_minion_data().ok_or("Minion installation not found.")?;
    load_minion_addons_from(&path)
}

pub(crate) fn load_minion_addons_from(path: &Path) -> Result<Vec<MinionAddon>, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read Minion data: {e}"))?;
    if path
        .extension()
        .is_some_and(|extension| extension == "json")
    {
        parse_minion4_addons(&content)
    } else {
        Ok(parse_minion_addons(&content))
    }
}

pub(crate) fn parse_minion4_addons(content: &str) -> Result<Vec<MinionAddon>, String> {
    let data: serde_json::Value =
        serde_json::from_str(content).map_err(|e| format!("Invalid Minion 4 data: {e}"))?;
    let games = data["installedGames"]
        .as_array()
        .ok_or("Invalid Minion 4 data: installedGames must be an array.")?;
    let mut addons = Vec::new();
    for game in games {
        let Some(install_path) = game["installPath"]
            .as_str()
            .filter(|path| !path.trim().is_empty())
        else {
            continue;
        };
        let Some(bundles) = game["installedBundles"].as_object() else {
            continue;
        };
        for (key, bundle) in bundles {
            let Some(uid) = key
                .strip_prefix("esoui:")
                .and_then(|id| id.parse::<u32>().ok())
                .filter(|id| *id > 0)
            else {
                continue;
            };
            let Some(version) = bundle["version"]
                .as_str()
                .filter(|version| !version.trim().is_empty())
            else {
                continue;
            };
            let Some(manifests) = bundle["addOns"].as_object() else {
                continue;
            };
            // Minion stores addon directory paths relative to the AddOns root.
            // Nested libraries belong to their containing addon; they are not
            // evidence that a standalone library release is installed.
            let folders: Vec<String> = manifests
                .keys()
                .filter(|folder| {
                    !folder.is_empty()
                        && !matches!(folder.as_str(), "." | "..")
                        && !folder.contains(['/', '\\', ':'])
                })
                .cloned()
                .collect();
            if !folders.is_empty() {
                addons.push(MinionAddon {
                    uid,
                    version: version.to_string(),
                    folders,
                    addons_path: Some(PathBuf::from(install_path).join("AddOns")),
                });
            }
        }
    }
    Ok(addons)
}

pub fn parse_minion_addons(xml_content: &str) -> Vec<MinionAddon> {
    let mut addons: Vec<MinionAddon> = Vec::new();
    static RE_GAME: OnceLock<Regex> = OnceLock::new();
    let re_game = RE_GAME.get_or_init(|| Regex::new(r#"<game\s+([^>]*)>"#).unwrap());
    static RE_ADDON: OnceLock<Regex> = OnceLock::new();
    let re_addon = RE_ADDON.get_or_init(|| Regex::new(r#"<addon\s+([^>]*)>"#).unwrap());
    static RE_UID: OnceLock<Regex> = OnceLock::new();
    let re_uid = RE_UID.get_or_init(|| Regex::new(r#"(?:^|\s)uid="(\d+)""#).unwrap());
    static RE_VERSION: OnceLock<Regex> = OnceLock::new();
    let re_version =
        RE_VERSION.get_or_init(|| Regex::new(r#"(?:^|\s)ui-version="([^"]*)""#).unwrap());
    static RE_ADDONS_PATH: OnceLock<Regex> = OnceLock::new();
    let re_addons_path =
        RE_ADDONS_PATH.get_or_init(|| Regex::new(r#"(?:^|\s)addon-path="([^"]*)""#).unwrap());
    static RE_DIR: OnceLock<Regex> = OnceLock::new();
    let re_dir = RE_DIR.get_or_init(|| Regex::new(r"<dir>([^<]+)</dir>").unwrap());

    // Simple state machine parser for Minion XML
    let mut current_addons_path: Option<PathBuf> = None;
    let mut current_uid: Option<u32> = None;
    let mut current_version = String::new();
    let mut current_dirs: Vec<String> = Vec::new();

    for line in xml_content.lines() {
        let line = line.trim();
        if let Some(caps) = re_game.captures(line) {
            current_addons_path = re_addons_path
                .captures(&caps[1])
                .and_then(|path_caps| decode_minion_addons_path(&path_caps[1]));
        } else if line.contains("</game>") {
            current_addons_path = None;
        } else if let Some(caps) = re_addon.captures(line) {
            // Save previous addon if any
            if let Some(uid) = current_uid {
                if !current_dirs.is_empty() {
                    addons.push(MinionAddon {
                        uid,
                        version: current_version.clone(),
                        folders: current_dirs.clone(),
                        addons_path: current_addons_path.clone(),
                    });
                }
            }
            current_uid = re_uid
                .captures(&caps[1])
                .and_then(|uid_caps| uid_caps[1].parse::<u32>().ok());
            current_version = re_version
                .captures(&caps[1])
                .map(|version_caps| version_caps[1].to_string())
                .unwrap_or_default();
            current_dirs = Vec::new();
        } else if let Some(caps) = re_dir.captures(line) {
            current_dirs.push(caps[1].to_string());
        } else if line.contains("</addon>") {
            if let Some(uid) = current_uid {
                if !current_dirs.is_empty() {
                    addons.push(MinionAddon {
                        uid,
                        version: current_version.clone(),
                        folders: current_dirs.clone(),
                        addons_path: current_addons_path.clone(),
                    });
                }
            }
            current_uid = None;
            current_dirs = Vec::new();
        }
    }

    addons
}

fn decode_minion_addons_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    BASE64_STANDARD
        .decode(value)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from(value)))
}

pub(crate) fn minion_addon_is_for_root(
    addon: &MinionAddon,
    addons_dir: &Path,
    has_scoped_entries: bool,
) -> bool {
    match addon.addons_path.as_deref() {
        Some(path) => paths_refer_to_same_dir(path, addons_dir),
        None => !has_scoped_entries,
    }
}

fn paths_refer_to_same_dir(left: &Path, right: &Path) -> bool {
    let left = dunce::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = dunce::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());

    #[cfg(windows)]
    {
        left.to_string_lossy()
            .trim_end_matches(['/', '\\'])
            .eq_ignore_ascii_case(right.to_string_lossy().trim_end_matches(['/', '\\']))
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minion_root_and_attribute_order() {
        let encoded_path = BASE64_STANDARD.encode(r"C:\Games\ESO\live\AddOns");
        let xml = format!(
            r#"<game addon-path="{encoded_path}" game-id="ESO">
                <addon md5="abc123" ui-version="2.0 r7" uid="123">
                    <dir>MyAddon</dir>
                </addon>
            </game>"#
        );

        let addons = parse_minion_addons(&xml);

        assert_eq!(addons.len(), 1);
        assert_eq!(addons[0].uid, 123);
        assert_eq!(addons[0].version, "2.0 r7");
        assert_eq!(addons[0].folders, ["MyAddon"]);
        assert_eq!(
            addons[0].addons_path.as_deref(),
            Some(Path::new(r"C:\Games\ESO\live\AddOns"))
        );
    }

    #[test]
    fn scoped_minion_entries_only_match_their_addons_root() {
        let temp = tempfile::tempdir().unwrap();
        let live = temp.path().join("live").join("AddOns");
        let pts = temp.path().join("pts").join("AddOns");
        std::fs::create_dir_all(&live).unwrap();
        std::fs::create_dir_all(&pts).unwrap();
        let addon = MinionAddon {
            uid: 123,
            version: "2.0".to_string(),
            folders: vec!["MyAddon".to_string()],
            addons_path: Some(live.clone()),
        };

        assert!(minion_addon_is_for_root(&addon, &live, true));
        assert!(!minion_addon_is_for_root(&addon, &pts, true));
    }
}
