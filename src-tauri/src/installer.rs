use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

pub fn extract_addon_zip(zip_path: &Path, addons_dir: &Path) -> Result<Vec<String>, String> {
    let file =
        fs::File::open(zip_path).map_err(|e| format!("Failed to open ZIP file: {}", e))?;

    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Failed to read ZIP archive: {}", e))?;

    let mut created_folders: HashSet<String> = HashSet::new();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry: {}", e))?;

        // Use enclosed_name for path traversal safety
        let relative_path = match entry.enclosed_name() {
            Some(p) => p.to_owned(),
            None => continue,
        };

        let out_path = addons_dir.join(&relative_path);

        // Track top-level folder names
        if let Some(first_component) = relative_path.components().next() {
            let folder = first_component.as_os_str().to_string_lossy().to_string();
            created_folders.insert(folder);
        }

        if entry.is_dir() {
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("Failed to create directory {:?}: {}", out_path, e))?;
        } else {
            // Ensure parent directory exists
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory {:?}: {}", parent, e))?;
            }

            let mut outfile = fs::File::create(&out_path)
                .map_err(|e| format!("Failed to create file {:?}: {}", out_path, e))?;

            io::copy(&mut entry, &mut outfile)
                .map_err(|e| format!("Failed to extract {:?}: {}", out_path, e))?;
        }
    }

    Ok(created_folders.into_iter().collect())
}

pub fn remove_addon(addons_dir: &Path, folder_name: &str) -> Result<(), String> {
    // Validate folder name — no path traversal
    if folder_name.contains("..")
        || folder_name.contains('/')
        || folder_name.contains('\\')
        || folder_name.is_empty()
    {
        return Err("Invalid addon folder name.".to_string());
    }

    let addon_path = addons_dir.join(folder_name);

    if !addon_path.is_dir() {
        return Err(format!("Addon folder not found: {}", folder_name));
    }

    // Verify the folder is actually inside the addons directory
    let canonical_addons = addons_dir
        .canonicalize()
        .map_err(|e| format!("Failed to resolve addons path: {}", e))?;
    let canonical_addon = addon_path
        .canonicalize()
        .map_err(|e| format!("Failed to resolve addon path: {}", e))?;

    if !canonical_addon.starts_with(&canonical_addons) {
        return Err("Addon path is outside the AddOns directory.".to_string());
    }

    fs::remove_dir_all(&addon_path)
        .map_err(|e| format!("Failed to remove addon {}: {}", folder_name, e))?;

    Ok(())
}
