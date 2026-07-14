//! Cross-platform helpers shared by game detection, auth, and the uploader:
//! Steam root discovery, Steam library enumeration, Proton/Wine prefix
//! scanning, browser opening, and process detection.
//!
//! Windows keeps its registry-based detection in `game_instances.rs`; this
//! module hosts the pieces that are either shared by all platforms (the VDF
//! library parser, `open_url`) or specific to macOS/Linux (Steam root
//! candidates, Proton prefixes).

use std::path::{Path, PathBuf};

/// ESO's Steam App ID — names `appmanifest_306130.acf` and the Proton
/// compatdata prefix directory.
pub const ESO_STEAM_APP_ID: &str = "306130";

// ── Steam library discovery ──────────────────────────────────────────────

/// Collect all Steam library root paths (including the default one inside the
/// Steam install dir) by scanning `steamapps/libraryfolders.vdf`.
///
/// The VDF format is a simple key-value text file; we use a regex to extract
/// all `"path"  "..."` entries rather than a full parser. The `\\` → `\`
/// unescape only matters for Windows-style paths inside the VDF and is a
/// no-op for Unix paths.
pub fn steam_library_paths(steam_root: &Path) -> Vec<PathBuf> {
    let mut paths = vec![steam_root.to_path_buf()];

    let vdf_path = steam_root.join("steamapps").join("libraryfolders.vdf");
    let Ok(contents) = std::fs::read_to_string(&vdf_path) else {
        return paths;
    };

    // Match lines like:  "path"    "D:\\SteamLibrary"  or  "/mnt/games/Steam"
    let re = regex::Regex::new(r#""path"\s+"([^"]+)""#).expect("static regex");
    for cap in re.captures_iter(&contents) {
        let lib = PathBuf::from(cap[1].replace("\\\\", "\\"));
        if lib.is_dir() && !paths.contains(&lib) {
            paths.push(lib);
        }
    }

    paths
}

/// Steam installation roots to probe on Linux, covering native (deb/rpm),
/// XDG-data, Flatpak, and Snap installs. Only existing directories are
/// returned; symlinked duplicates (`~/.steam/steam` usually points at
/// `~/.local/share/Steam`) are deduped by the caller's canonical-path checks.
#[cfg(target_os = "linux")]
pub fn steam_root_candidates() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    [
        ".steam/steam",
        ".steam/root",
        ".local/share/Steam",
        // Flatpak Steam's internal layout has varied across releases; probe
        // every known variant — nonexistent ones are filtered out below and
        // the caller's canonical-path dedupe collapses any overlap.
        ".var/app/com.valvesoftware.Steam/data/Steam",
        ".var/app/com.valvesoftware.Steam/.local/share/Steam",
        ".var/app/com.valvesoftware.Steam/.steam/steam",
        "snap/steam/common/.local/share/Steam", // Snap
    ]
    .iter()
    .map(|rel| home.join(rel))
    .filter(|p| p.is_dir())
    .collect()
}

// ── Proton / Wine prefix scanning ────────────────────────────────────────

/// Given a set of Steam roots, return every existing Proton `Documents`
/// directory for ESO — i.e. the Wine-prefix equivalent of the Windows
/// `Documents` folder that the game writes AddOns/SavedVariables under:
/// `<library>/steamapps/compatdata/306130/pfx/drive_c/users/steamuser/Documents`.
pub fn proton_documents_roots_from(steam_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for root in steam_roots {
        for library in steam_library_paths(root) {
            let docs = library
                .join("steamapps")
                .join("compatdata")
                .join(ESO_STEAM_APP_ID)
                .join("pfx")
                .join("drive_c")
                .join("users")
                .join("steamuser")
                .join("Documents");
            if docs.is_dir() && !out.contains(&docs) {
                out.push(docs);
            }
        }
    }
    out
}

/// All Proton `Documents` roots for ESO across every detected Steam install.
#[cfg(target_os = "linux")]
pub fn proton_documents_roots() -> Vec<PathBuf> {
    proton_documents_roots_from(&steam_root_candidates())
}

/// CrossOver bottle `Documents` directories on macOS. ESO's native Mac client
/// uses `~/Documents` directly (covered by `dirs::document_dir()`), but many
/// Apple Silicon players run the Windows client through CrossOver, which puts
/// the game's Documents inside the bottle.
#[cfg(target_os = "macos")]
pub fn crossover_documents_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let bottles = home.join("Library/Application Support/CrossOver/Bottles");
    let Ok(entries) = std::fs::read_dir(&bottles) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let users = entry.path().join("drive_c").join("users");
        let Ok(user_dirs) = std::fs::read_dir(&users) else {
            continue;
        };
        for user in user_dirs.flatten() {
            // Default bottle user is "crossover", but scan every user dir
            // (excluding Wine's "Public") so renamed users still work.
            if user
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case("Public")
            {
                continue;
            }
            let docs = user.path().join("Documents");
            if docs.is_dir() && !out.contains(&docs) {
                out.push(docs);
            }
        }
    }
    out
}

// ── Browser / URL opening ────────────────────────────────────────────────

/// Open `url` in the system default browser. Used by backend-initiated flows
/// (OAuth login, uploader handoff) that have no window handle; frontend code
/// keeps using `tauri-plugin-opener`.
pub fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map_err(|e| format!("Failed to open browser: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Failed to open browser: {e}"))?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Failed to open browser: {e}"))?;
    }
    Ok(())
}

// ── Process detection ────────────────────────────────────────────────────

/// Case-insensitive process search over full command lines (`pgrep -if`).
/// Matching the full command line (not just the comm name) is what catches
/// ESO under Proton, where `eso64.exe` appears as an argument to the Wine
/// loader rather than as the 15-char truncated comm name.
///
/// Only ever call this with static, known-safe patterns — the value is passed
/// to `pgrep` as a single argv entry (no shell), but keeping inputs static
/// also keeps the results predictable.
#[cfg(not(windows))]
pub fn unix_process_running(pattern: &str) -> bool {
    std::process::Command::new("pgrep")
        .args(["-if", pattern])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch_dir(path: &Path) {
        std::fs::create_dir_all(path).expect("create test dir");
    }

    #[test]
    fn steam_library_paths_parses_unix_vdf() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let steam_root = tmp.path().join("Steam");
        touch_dir(&steam_root.join("steamapps"));
        let extra_lib = tmp.path().join("ExtraLibrary");
        touch_dir(&extra_lib);

        let vdf = format!(
            r#""libraryfolders"
{{
    "0"
    {{
        "path"		"{}"
    }}
    "1"
    {{
        "path"		"{}"
    }}
    "2"
    {{
        "path"		"/nonexistent/steam/library"
    }}
}}
"#,
            steam_root.display(),
            extra_lib.display()
        );
        std::fs::write(steam_root.join("steamapps/libraryfolders.vdf"), vdf).expect("write vdf");

        let paths = steam_library_paths(&steam_root);
        assert!(paths.contains(&steam_root), "steam root always included");
        assert!(paths.contains(&extra_lib), "existing extra library found");
        assert_eq!(paths.len(), 2, "nonexistent library filtered out");
    }

    #[test]
    fn steam_library_paths_survives_missing_vdf() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let steam_root = tmp.path().join("Steam");
        touch_dir(&steam_root);
        let paths = steam_library_paths(&steam_root);
        assert_eq!(paths, vec![steam_root]);
    }

    #[test]
    fn proton_documents_roots_finds_eso_prefix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let steam_root = tmp.path().join("Steam");
        let docs = steam_root
            .join("steamapps")
            .join("compatdata")
            .join(ESO_STEAM_APP_ID)
            .join("pfx/drive_c/users/steamuser/Documents");
        touch_dir(&docs);
        // A different game's prefix must not match.
        touch_dir(
            &steam_root.join("steamapps/compatdata/12345/pfx/drive_c/users/steamuser/Documents"),
        );

        let roots = proton_documents_roots_from(std::slice::from_ref(&steam_root));
        assert_eq!(roots, vec![docs]);
    }

    #[test]
    fn proton_documents_roots_empty_when_no_prefix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let steam_root = tmp.path().join("Steam");
        touch_dir(&steam_root.join("steamapps"));
        assert!(proton_documents_roots_from(&[steam_root]).is_empty());
    }
}
