use crate::services::updates;
use dioxus::prelude::*;
use fleet_core::ApiError;

#[derive(Clone, PartialEq)]
pub enum AppUpdateStatus {
    Idle,
    Checking,
    UpToDate,
    UpdateAvailable { version: String },
    Downloading,
    Error(ApiError),
}

#[derive(Clone)]
pub struct UpdateStore {
    pub status: Signal<AppUpdateStatus>,
}

pub async fn check_for_updates_status() -> AppUpdateStatus {
    tokio::task::spawn_blocking(move || updates::check_for_updates(updates::UPDATE_URL))
        .await
        .map_err(|error| {
            ApiError::reported(
                "update_check",
                "Could not finish checking for updates. Try again.",
                error,
            )
        })
        .and_then(|r| r)
        .map(|version| match version {
            Some(version) => AppUpdateStatus::UpdateAvailable { version },
            None => AppUpdateStatus::UpToDate,
        })
        .unwrap_or_else(AppUpdateStatus::Error)
}

pub async fn apply_update() -> Result<(), ApiError> {
    tokio::task::spawn_blocking(move || updates::download_apply_and_restart(updates::UPDATE_URL))
        .await
        .map_err(|error| {
            ApiError::reported(
                "update_apply",
                "Could not finish applying the update. Try again.",
                error,
            )
        })
        .and_then(|r| r)
}
