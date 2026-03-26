use crate::esoui::{self, EsouiAddonInfo};
use crate::installer;
use crate::manifest::{self, AddonManifest};
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    pub installed_folders: Vec<String>,
    pub installed_deps: Vec<String>,
    pub failed_deps: Vec<String>,
    pub skipped_deps: Vec<String>,
}

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
pub fn install_addon(addons_path: String, download_url: String) -> Result<InstallResult, String> {
    let addons_dir = PathBuf::from(&addons_path);
    if !addons_dir.is_dir() {
        return Err(format!("AddOns folder not found: {}", addons_path));
    }

    // Download and extract the main addon
    let tmp_file = esoui::download_addon(&download_url)?;
    let installed_folders = installer::extract_addon_zip(tmp_file.path(), &addons_dir)?;

    // Collect all installed folder names (existing + newly installed)
    let mut all_installed: HashSet<String> = HashSet::new();
    if let Ok(entries) = fs::read_dir(&addons_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    all_installed.insert(name.to_string());
                }
            }
        }
    }

    // Parse manifests of newly installed addons to find dependencies
    let mut missing_deps: Vec<String> = Vec::new();
    for folder in &installed_folders {
        let manifest_path = addons_dir.join(folder).join(format!("{}.txt", folder));
        if let Some(addon) = manifest::parse_manifest(folder, &manifest_path) {
            for dep in &addon.depends_on {
                if !all_installed.contains(&dep.name) && !missing_deps.contains(&dep.name) {
                    missing_deps.push(dep.name.clone());
                }
            }
        }
    }

    // Try to auto-install missing dependencies
    let mut installed_deps: Vec<String> = Vec::new();
    let mut failed_deps: Vec<String> = Vec::new();
    let mut skipped_deps: Vec<String> = Vec::new();

    for dep_name in &missing_deps {
        // Search ESOUI for the dependency
        match esoui::search_addon_by_name(dep_name) {
            Ok(Some(dep_id)) => {
                // Found it — fetch info and install
                match esoui::fetch_addon_info(dep_id) {
                    Ok(dep_info) => match esoui::download_addon(&dep_info.download_url) {
                        Ok(dep_tmp) => {
                            match installer::extract_addon_zip(dep_tmp.path(), &addons_dir) {
                                Ok(dep_folders) => {
                                    for f in &dep_folders {
                                        all_installed.insert(f.clone());
                                    }
                                    installed_deps.push(dep_name.clone());
                                }
                                Err(_) => failed_deps.push(dep_name.clone()),
                            }
                        }
                        Err(_) => failed_deps.push(dep_name.clone()),
                    },
                    Err(_) => failed_deps.push(dep_name.clone()),
                }
            }
            Ok(None) => skipped_deps.push(dep_name.clone()),
            Err(_) => failed_deps.push(dep_name.clone()),
        }
    }

    Ok(InstallResult {
        installed_folders,
        installed_deps,
        failed_deps,
        skipped_deps,
    })
}

#[tauri::command]
pub fn remove_addon(addons_path: String, folder_name: String) -> Result<(), String> {
    let addons_dir = PathBuf::from(&addons_path);
    installer::remove_addon(&addons_dir, &folder_name)
}
