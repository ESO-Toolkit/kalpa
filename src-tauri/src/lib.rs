mod commands;
mod esoui;
mod installer;
mod manifest;

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::detect_addons_folder,
            commands::scan_installed_addons,
            commands::resolve_esoui_addon,
            commands::install_addon,
            commands::remove_addon,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
