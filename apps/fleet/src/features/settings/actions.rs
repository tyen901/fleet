use dioxus::prelude::*;

use crate::services::bridge::FleetBridge;
use crate::stores::toast_store::ToastStore;

pub(crate) fn spawn_settings_task<F>(toasts: ToastStore, title: &'static str, task: F)
where
    F: std::future::Future<Output = Result<(), fleet_core::ApiError>> + 'static,
{
    spawn(async move {
        if let Err(err) = task.await {
            toasts.push_api_error(title, &err);
        }
    });
}

pub(crate) fn spawn_debounced_settings_save<F>(
    bridge: FleetBridge,
    toasts: ToastStore,
    value: String,
    seq: u64,
    seq_signal: Signal<u64>,
    assign: F,
) where
    F: Fn(&mut fleet_core::AppSettings, String) + Send + 'static,
{
    spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        if seq_signal() != seq {
            return;
        }
        let mut settings = bridge.get_snapshot().settings.clone();
        assign(&mut settings, value);
        if let Err(err) = bridge.core().settings_save(settings).await {
            toasts.push_api_error("Save settings", &err);
        }
    });
}

pub(crate) fn spawn_settings_update<F>(bridge: FleetBridge, toasts: ToastStore, apply: F)
where
    F: FnOnce(&mut fleet_core::AppSettings) + Send + 'static,
{
    spawn(async move {
        let mut settings = bridge.get_snapshot().settings.clone();
        apply(&mut settings);
        if let Err(err) = bridge.core().settings_save(settings).await {
            toasts.push_api_error("Save settings", &err);
        }
    });
}
