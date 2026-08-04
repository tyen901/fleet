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
    tokio::task::spawn_blocking(move || {
        let feed = updates::resolve_feed_url()?;
        updates::check_for_updates(&feed)
    })
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
    tokio::task::spawn_blocking(move || {
        let feed = updates::resolve_feed_url()?;
        updates::download_apply_and_restart(&feed)
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r)
}
