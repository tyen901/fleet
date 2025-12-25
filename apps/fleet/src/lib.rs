mod commands;

use fleet_app::services::FleetServices;
use commands::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run(services: FleetServices) {
    tauri::Builder::default()
        .setup(move |app| {
            app.manage(AppState::new(services));
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_system_status,
            // Add future commands here
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
