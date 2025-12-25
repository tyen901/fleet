mod commands;

use commands::AppState;
use fleet_app::services::FleetServices;
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
            commands::data_snapshot,
            commands::data_refresh_profiles,
            commands::data_select_profile,
            commands::data_create_profile,
            commands::data_update_profile,
            commands::data_delete_profile,
            commands::data_launch_arma3,
            commands::sync_snapshot,
            commands::sync_start,
            commands::sync_cancel,
            commands::subscribe_sync_state,
            commands::get_sync_logs,
            commands::update_snapshot,
            commands::update_check,
            commands::update_apply,
            commands::subscribe_update_state,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
