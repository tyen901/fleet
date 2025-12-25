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

#[tauri::command]
pub async fn data_snapshot(
    state: State<'_, AppState>,
) -> Result<fleet_app::DataModel, ApiError> {
    Ok((*state.services.data.snapshot()).clone())
}

#[tauri::command]
pub async fn data_refresh_profiles(state: State<'_, AppState>) -> Result<(), ApiError> {
    state.services.data.refresh_profiles().map_err(Into::into)
}

#[tauri::command]
pub async fn data_select_profile(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), ApiError> {
    state.services.data.select_profile(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn data_create_profile(
    state: State<'_, AppState>,
    create: ProfileCreate,
) -> Result<String, ApiError> {
    state.services.data.create_profile(create).map_err(Into::into)
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
pub async fn data_delete_profile(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), ApiError> {
    state.services.data.delete_profile(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn data_launch_arma3(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), ApiError> {
    state
        .services
        .data
        .launch_arma3_for_profile(&id)
        .map_err(Into::into)
}

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
