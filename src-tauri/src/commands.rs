use crate::esoui::{self, EsouiAddonInfo};
use crate::installer;
use crate::manifest::{self, AddonManifest};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

fn default_addons_path() -> Option<PathBuf> {
    let docs = dirs::document_dir()?;
    let addons = docs
        .join("Elder Scrolls Online")
        .join("live")
        .join("AddOns");
    if addons.is_dir() {
        Some(addons)
    } else {
        None
    }
}

#[tauri::command]
pub fn detect_addons_folder() -> Result<String, String> {
    default_addons_path()
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| "Could not find ESO AddOns folder. Please set it manually.".to_string())
}

#[tauri::command]
pub fn scan_installed_addons(addons_path: String) -> Result<Vec<AddonManifest>, String> {
    let addons_dir = PathBuf::from(&addons_path);
    if !addons_dir.is_dir() {
        return Err(format!("AddOns folder not found: {}", addons_path));
    }

    let entries = fs::read_dir(&addons_dir)
        .map_err(|e| format!("Failed to read AddOns folder: {}", e))?;

    let mut addons: Vec<AddonManifest> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let folder_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };

        let manifest_path = path.join(format!("{}.txt", &folder_name));
        if !manifest_path.exists() {
            continue;
        }

        if let Some(addon) = manifest::parse_manifest(&folder_name, &manifest_path) {
            addons.push(addon);
        }
    }

    // Build set of installed folder names for dependency checking
    let installed: HashSet<String> = addons.iter().map(|a| a.folder_name.clone()).collect();

    // Check for missing dependencies
    for addon in &mut addons {
        addon.missing_dependencies = addon
            .depends_on
            .iter()
            .filter(|dep| !installed.contains(&dep.name))
            .map(|dep| dep.name.clone())
            .collect();
    }

    addons.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));

    Ok(addons)
}

#[tauri::command]
pub fn resolve_esoui_addon(input: String) -> Result<EsouiAddonInfo, String> {
    let id = esoui::parse_esoui_input(&input)?;
    esoui::fetch_addon_info(id)
}

#[tauri::command]
pub fn install_addon(addons_path: String, download_url: String) -> Result<Vec<String>, String> {
    let addons_dir = PathBuf::from(&addons_path);
    if !addons_dir.is_dir() {
        return Err(format!("AddOns folder not found: {}", addons_path));
    }

    let tmp_file = esoui::download_addon(&download_url)?;
    let folders = installer::extract_addon_zip(tmp_file.path(), &addons_dir)?;

    Ok(folders)
}

#[tauri::command]
pub fn remove_addon(addons_path: String, folder_name: String) -> Result<(), String> {
    let addons_dir = PathBuf::from(&addons_path);
    installer::remove_addon(&addons_dir, &folder_name)
}
