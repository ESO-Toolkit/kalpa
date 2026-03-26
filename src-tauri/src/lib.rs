mod commands;
mod manifest;

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::detect_addons_folder,
            commands::scan_installed_addons,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
