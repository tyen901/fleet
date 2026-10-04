use crate::services::updates;
use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub enum AppUpdateStatus {
    Idle,
    Checking,
    UpToDate,
    UpdateAvailable { version: String },
    Downloading,
    Error(String),
}

#[derive(Clone)]
pub struct UpdateStore {
    pub status: Signal<AppUpdateStatus>,
}

pub async fn check_for_updates_status() -> AppUpdateStatus {
    tokio::task::spawn_blocking(move || updates::check_for_updates(updates::UPDATE_URL))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r)
        .map(|version| match version {
            Some(version) => AppUpdateStatus::UpdateAvailable { version },
            None => AppUpdateStatus::UpToDate,
        })
        .unwrap_or_else(AppUpdateStatus::Error)
}

pub async fn apply_update() -> Result<(), String> {
    tokio::task::spawn_blocking(move || updates::download_apply_and_restart(updates::UPDATE_URL))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r)
}
