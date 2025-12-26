use std::sync::Arc;
use std::time::Duration;

use fleet_app::services::FleetServices;
use fleet_app::{AppError, ProfileCreate, ProfileUpdate};
use tauri::ipc::Channel;
use tauri::State;

pub struct AppState {
    pub services: FleetServices,
}

impl AppState {
    pub fn new(services: FleetServices) -> Self {
        Self { services }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
}

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        ApiError {
            code: "app_error",
            message: e.to_string(),
        }
    }
}

// -------------------- Data --------------------

#[tauri::command]
pub async fn data_snapshot(state: State<'_, AppState>) -> Result<fleet_app::DataModel, ApiError> {
    Ok((*state.services.data.snapshot()).clone())
}

#[tauri::command]
pub async fn data_refresh_profiles(state: State<'_, AppState>) -> Result<(), ApiError> {
    state.services.data.refresh_profiles().map_err(Into::into)
}

#[tauri::command]
pub async fn data_select_profile(state: State<'_, AppState>, id: String) -> Result<(), ApiError> {
    state.services.data.select_profile(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn data_create_profile(
    state: State<'_, AppState>,
    create: ProfileCreate,
) -> Result<String, ApiError> {
    state
        .services
        .data
        .create_profile(create)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn data_update_profile(
    state: State<'_, AppState>,
    id: String,
    update: ProfileUpdate,
) -> Result<(), ApiError> {
    state
        .services
        .data
        .update_profile(&id, update)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn data_delete_profile(state: State<'_, AppState>, id: String) -> Result<(), ApiError> {
    state.services.data.delete_profile(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn data_launch_arma3(state: State<'_, AppState>, id: String) -> Result<(), ApiError> {
    state
        .services
        .data
        .launch_arma3_for_profile(&id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn data_open_checkout_root(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<(), ApiError> {
    state
        .services
        .data
        .open_checkout_root(&profile_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn data_open_folder(state: State<'_, AppState>, path: String) -> Result<(), ApiError> {
    state
        .services
        .data
        .open_folder(std::path::Path::new(&path))
        .map_err(Into::into)
}

#[tauri::command]
pub async fn data_set_settings(
    state: State<'_, AppState>,
    settings: fleet_app::LaunchSettings,
) -> Result<(), ApiError> {
    state
        .services
        .data
        .set_settings(settings)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn data_reset_settings_to_defaults(state: State<'_, AppState>) -> Result<(), ApiError> {
    state
        .services
        .data
        .reset_settings_to_defaults()
        .map_err(Into::into)
}

#[tauri::command]
pub async fn data_launch_args_preview(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<String, ApiError> {
    state
        .services
        .data
        .launch_args_preview(&profile_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn data_request_launch_args_preview(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<(), ApiError> {
    state.services.data.request_launch_args_preview(&profile_id);
    Ok(())
}

#[tauri::command]
pub async fn data_request_repo_spec(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<(), ApiError> {
    state.services.data.request_repo_spec(&profile_id);
    Ok(())
}

#[tauri::command]
pub async fn data_request_repo_spec_for_url(
    state: State<'_, AppState>,
    repo_url: String,
) -> Result<(), ApiError> {
    state.services.data.request_repo_spec_for_url(&repo_url);
    Ok(())
}

#[tauri::command]
pub async fn data_request_linux_validation(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<(), ApiError> {
    state.services.data.request_linux_validation(&profile_id);
    Ok(())
}

#[tauri::command]
pub async fn data_request_linux_validation_with_settings(
    state: State<'_, AppState>,
    profile_id: String,
    settings: fleet_app::LaunchSettings,
) -> Result<(), ApiError> {
    state
        .services
        .data
        .request_linux_validation_with_settings(&profile_id, settings);
    Ok(())
}

#[tauri::command]
pub async fn data_rebuild_index(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<(), ApiError> {
    state
        .services
        .data
        .rebuild_index(&profile_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn data_clear_cache(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<(), ApiError> {
    state
        .services
        .data
        .clear_cache(&profile_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn data_clear_last_sync_outcome(state: State<'_, AppState>) -> Result<(), ApiError> {
    state.services.data.clear_last_sync_outcome();
    Ok(())
}

#[tauri::command]
pub async fn data_init_registry(state: State<'_, AppState>) -> Result<(), ApiError> {
    state.services.data.init_registry().map_err(Into::into)
}

#[tauri::command]
pub async fn data_registry_path(state: State<'_, AppState>) -> Result<String, ApiError> {
    state.services.data.registry_path().map_err(Into::into)
}

// -------------------- Sync --------------------

#[tauri::command]
pub async fn sync_snapshot(
    state: State<'_, AppState>,
) -> Result<fleet_app::SyncReadModel, ApiError> {
    Ok(state.services.sync.snapshot())
}

#[tauri::command]
pub async fn sync_start(
    state: State<'_, AppState>,
    mode: fleet_app::SyncMode,
    tuning: fleet_app::SyncTuning,
) -> Result<(), ApiError> {
    state.services.sync.start(mode, tuning).map_err(Into::into)
}

#[tauri::command]
pub async fn sync_cancel(state: State<'_, AppState>) -> Result<(), ApiError> {
    state.services.sync.cancel();
    Ok(())
}

#[tauri::command]
pub async fn subscribe_sync_state(
    state: State<'_, AppState>,
    on_snapshot: Channel<fleet_app::SyncReadModel>,
) -> Result<(), ApiError> {
    let mut rx = state.services.sync.subscribe_snapshots();

    tauri::async_runtime::spawn(async move {
        loop {
            let snap = rx.borrow().clone();
            if on_snapshot.send(snap).is_err() {
                break;
            }
            if rx.changed().await.is_err() {
                break;
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn get_sync_logs(
    state: State<'_, AppState>,
    cursor: u64,
) -> Result<fleet_app::LogPage, ApiError> {
    Ok(state.services.sync.log_page(cursor, 100))
}

// -------------------- Update --------------------

#[tauri::command]
pub async fn update_snapshot(
    state: State<'_, AppState>,
) -> Result<fleet_app::UpdateModel, ApiError> {
    Ok((*state.services.update.snapshot()).clone())
}

#[tauri::command]
pub async fn update_check(state: State<'_, AppState>) -> Result<(), ApiError> {
    state.services.update.check().map_err(Into::into)
}

#[tauri::command]
pub async fn update_apply(state: State<'_, AppState>) -> Result<(), ApiError> {
    state.services.update.apply().map_err(Into::into)
}

#[tauri::command]
pub async fn update_clear_error(state: State<'_, AppState>) -> Result<(), ApiError> {
    state.services.update.clear_error();
    Ok(())
}

#[tauri::command]
pub async fn subscribe_update_state(
    state: State<'_, AppState>,
    on_state: Channel<fleet_app::UpdateModel>,
) -> Result<(), ApiError> {
    let update = state.services.update.clone();

    let _ = on_state.send((*update.snapshot()).clone());

    tauri::async_runtime::spawn(async move {
        let mut last_ptr: Option<Arc<fleet_app::UpdateModel>> = None;
        let mut interval = tokio::time::interval(Duration::from_millis(250));

        loop {
            interval.tick().await;

            let current = update.snapshot();
            let changed = match last_ptr.as_ref() {
                Some(last) => !Arc::ptr_eq(last, &current),
                None => true,
            };

            if changed {
                if on_state.send((*current).clone()).is_err() {
                    break;
                }
                last_ptr = Some(current);
            }
        }
    });

    Ok(())
}
