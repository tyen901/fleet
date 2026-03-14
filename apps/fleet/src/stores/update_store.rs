use crate::services::updates;
use dioxus::prelude::*;
use fleet_domain::ReleaseChannel;

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

pub async fn check_for_updates_status(channel: ReleaseChannel) -> AppUpdateStatus {
    tokio::task::spawn_blocking(move || {
        let feed = updates::resolve_feed_url(channel)?;
        updates::check_for_updates(&feed, channel)
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

pub async fn apply_update(channel: ReleaseChannel) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let feed = updates::resolve_feed_url(channel)?;
        updates::download_apply_and_restart(&feed, channel)
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r)
}
