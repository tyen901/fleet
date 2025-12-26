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
            // Data
            commands::data_snapshot,
            commands::data_refresh_profiles,
            commands::data_select_profile,
            commands::data_create_profile,
            commands::data_update_profile,
            commands::data_delete_profile,
            commands::data_launch_arma3,
            commands::data_open_checkout_root,
            commands::data_open_folder,
            commands::data_set_settings,
            commands::data_reset_settings_to_defaults,
            commands::data_launch_args_preview,
            commands::data_request_launch_args_preview,
            commands::data_request_repo_spec,
            commands::data_request_repo_spec_for_url,
            commands::data_request_linux_validation,
            commands::data_request_linux_validation_with_settings,
            commands::data_rebuild_index,
            commands::data_clear_cache,
            commands::data_clear_last_sync_outcome,
            commands::data_init_storage,
            commands::data_profiles_path,
            commands::data_settings_path,
            // Sync
            commands::sync_snapshot,
            commands::sync_start,
            commands::sync_cancel,
            commands::subscribe_sync_state,
            commands::get_sync_logs,
            // Update
            commands::update_snapshot,
            commands::update_check,
            commands::update_apply,
            commands::update_clear_error,
            commands::subscribe_update_state,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
