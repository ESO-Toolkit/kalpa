use crate::commands::{count_addon_manifests, documents_candidates, is_onedrive_path};
use serde::Serialize;
use std::path::PathBuf;

/// Whether ESO was installed via the standalone (Bethesda/ZOS) launcher or Steam.
/// Detected once per app launch via the Windows registry; purely informational.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ClientType {
    Native,
    Steam,
}

/// Which ESO server region this AddOns directory belongs to.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ServerRegion {
    /// North America (Documents\Elder Scrolls Online\live\)
    Na,
    /// Europe (Documents\Elder Scrolls Online\liveeu\)
    Eu,
    /// Public Test Server (Documents\Elder Scrolls Online\pts\)
    Pts,
}

impl ServerRegion {
    pub fn env_folder(&self) -> &'static str {
        match self {
            Self::Na => "live",
            Self::Eu => "liveeu",
            Self::Pts => "pts",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Na => "NA",
            Self::Eu => "EU",
            Self::Pts => "PTS",
        }
    }
}

/// A fully-identified ESO game installation instance: one region × one launcher.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInstance {
    /// Stable ID: `"live"` | `"liveeu"` | `"pts"` (region env-folder name).
    /// Both launchers share the same Documents path for a given region, so the
    /// id does not include the client type.
    pub id: String,
    /// How the game is launched (informational — does not affect path).
    pub client_type: ClientType,
    pub region: ServerRegion,
    /// Absolute path to the AddOns directory for this instance.
    pub addons_path: String,
    /// Number of valid addon manifests found in the AddOns directory.
    pub addon_count: usize,
    /// Whether the AddOns directory is inside an OneDrive-synced folder.
    pub is_onedrive: bool,
    /// Whether a SavedVariables directory exists next to AddOns.
    pub has_saved_variables: bool,
    /// Whether an AddOnSettings.txt file exists next to AddOns (game has been run).
    pub has_addon_settings: bool,
    /// Human-readable label combining client and region (e.g. "Steam · EU").
    pub display_label: String,
}

// ── Steam detection ──────────────────────────────────────────────────────────

/// Returns `true` if a Steam installation of ESO (App ID 306130) is detected
/// on this machine.
///
/// Detection strategy:
/// 1. Read `HKLM\SOFTWARE\Wow6432Node\Valve\Steam\InstallPath` (falls back to
///    the 32-bit key path) to find the Steam root.
/// 2. Collect all library folders by parsing `steamapps/libraryfolders.vdf`.
/// 3. Look for `steamapps/appmanifest_306130.acf` in any library.
///
/// Non-Windows builds always return `false`.
#[cfg(target_os = "windows")]
fn is_steam_eso_installed() -> bool {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let steam_key = hklm
        .open_subkey("SOFTWARE\\Wow6432Node\\Valve\\Steam")
        .or_else(|_| hklm.open_subkey("SOFTWARE\\Valve\\Steam"));

    let steam_root = match steam_key {
        Ok(key) => match key.get_value::<String, _>("InstallPath") {
            Ok(path) => PathBuf::from(path),
            Err(_) => return false,
        },
        Err(_) => return false,
    };

    for library in steam_library_paths(&steam_root) {
        if library
            .join("steamapps")
            .join("appmanifest_306130.acf")
            .is_file()
        {
            return true;
        }
    }

    false
}

#[cfg(not(target_os = "windows"))]
fn is_steam_eso_installed() -> bool {
    false
}

/// Collect all Steam library root paths (including the default one inside the
/// Steam install dir) by scanning `steamapps/libraryfolders.vdf`.
///
/// The VDF format is a simple key-value text file; we use a regex to extract
/// all `"path"  "..."` entries rather than a full parser.
#[cfg(target_os = "windows")]
fn steam_library_paths(steam_root: &std::path::Path) -> Vec<PathBuf> {
    let mut paths = vec![steam_root.to_path_buf()];

    let vdf_path = steam_root.join("steamapps").join("libraryfolders.vdf");
    let Ok(contents) = std::fs::read_to_string(&vdf_path) else {
        return paths;
    };

    // Match lines like:  "path"    "D:\\SteamLibrary"
    // We rely on the `regex` crate already in Cargo.toml.
    let re = regex::Regex::new(r#""path"\s+"([^"]+)""#).expect("static regex");
    for cap in re.captures_iter(&contents) {
        let lib = PathBuf::from(cap[1].replace("\\\\", "\\"));
        if lib.is_dir() && !paths.contains(&lib) {
            paths.push(lib);
        }
    }

    paths
}

/// Returns `true` if the standalone ZOS/Bethesda launcher has written its
/// registry key, indicating a native (non-Steam) ESO install exists.
///
/// Key: `HKLM\SOFTWARE\WOW6432Node\Zenimax_Online\Launcher\InstallPath`
#[cfg(target_os = "windows")]
fn is_native_eso_installed() -> bool {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\WOW6432Node\\Zenimax_Online\\Launcher")
        .and_then(|key| key.get_value::<String, _>("InstallPath"))
        .is_ok()
}

#[cfg(not(target_os = "windows"))]
fn is_native_eso_installed() -> bool {
    false
}

/// Determine the launcher type for this machine.
///
/// - If only Steam ESO is found → `Steam`
/// - Everything else → `Native` (standalone launcher present, both launchers
///   present, or neither detectable — both write to the same Documents path so
///   the distinction is informational only)
fn detect_client_type() -> ClientType {
    let has_steam = is_steam_eso_installed();
    let has_native = is_native_eso_installed();

    if has_steam && !has_native {
        ClientType::Steam
    } else {
        ClientType::Native
    }
}

// ── Instance scanning ────────────────────────────────────────────────────────

/// Scan all document roots for ESO AddOns directories and return a structured
/// list of detected game instances, sorted by activity score (most-active first).
///
/// Both Steam and native launcher write AddOns to the same Documents path for
/// a given region. Multiple document roots (e.g., a local Documents folder and
/// a redirected OneDrive folder) can each contain a valid AddOns directory for
/// the same region — those are kept as separate candidates and only collapsed
/// when they resolve to the same canonical path. The `client_type` field is
/// determined once by checking the Windows registry and applied to all instances.
pub fn detect_all_game_instances() -> Vec<GameInstance> {
    let client_type = detect_client_type();
    let regions = [ServerRegion::Na, ServerRegion::Eu, ServerRegion::Pts];
    let mut instances: Vec<GameInstance> = Vec::new();

    for base in documents_candidates() {
        let eso_root = base.join("Elder Scrolls Online");
        if !eso_root.is_dir() {
            continue;
        }

        for region in &regions {
            let env_dir = eso_root.join(region.env_folder());
            let addons_dir = env_dir.join("AddOns");
            if !addons_dir.is_dir() {
                continue;
            }

            // Deduplicate only on canonical path equality. Same-region directories
            // from different document roots (e.g., local vs. OneDrive-redirected) are
            // distinct candidates and must not be collapsed by region id alone.
            let canonical = addons_dir.canonicalize().unwrap_or(addons_dir.clone());
            let already_seen = instances.iter().any(|inst: &GameInstance| {
                PathBuf::from(&inst.addons_path)
                    .canonicalize()
                    .unwrap_or_default()
                    == canonical
            });
            if already_seen {
                continue;
            }

            let addons_path_str = addons_dir.to_string_lossy().to_string();
            let is_onedrive = is_onedrive_path(&addons_dir);
            let has_saved_variables = env_dir.join("SavedVariables").is_dir();
            let has_addon_settings = env_dir.join("AddOnSettings.txt").is_file();
            let addon_count = count_addon_manifests(&addons_dir);

            let client_label = match &client_type {
                ClientType::Steam => "Steam",
                ClientType::Native => "Native",
            };
            let onedrive_suffix = if is_onedrive { " · OneDrive" } else { "" };
            let display_label =
                format!("{} · {}{}", client_label, region.display_name(), onedrive_suffix);

            // Build a unique id. The first discovered path for a region gets the plain
            // env-folder name ("live"); additional same-region paths are numbered
            // ("live-2", "live-3", …) so React keys never collide.
            let base_id = region.env_folder();
            let existing_count = instances
                .iter()
                .filter(|i| i.id == base_id || i.id.starts_with(&format!("{}-", base_id)))
                .count();
            let id = if existing_count == 0 {
                base_id.to_string()
            } else {
                format!("{}-{}", base_id, existing_count + 1)
            };

            instances.push(GameInstance {
                id,
                client_type: client_type.clone(),
                region: region.clone(),
                addons_path: addons_path_str,
                addon_count,
                is_onedrive,
                has_saved_variables,
                has_addon_settings,
                display_label,
            });
        }
    }

    // Sort by activity score descending so the most-active instance is first.
    // The setup wizard and settings switcher treat index 0 as "Recommended".
    instances.sort_by_key(|inst| std::cmp::Reverse(instance_score(inst)));
    instances
}

/// Score an instance by evidence that it is the user's active game directory.
/// Higher scores surface first; OneDrive paths are penalised.
fn instance_score(inst: &GameInstance) -> i32 {
    let mut score = 0i32;
    if inst.has_saved_variables {
        score += 3; // strongest signal — game has been played here
    }
    if inst.has_addon_settings {
        score += 2; // game has been configured/run here
    }
    score += inst.addon_count as i32; // more addons = more invested region
    if inst.is_onedrive {
        score -= 10; // cloud-synced copies are less reliable
    }
    score
}
