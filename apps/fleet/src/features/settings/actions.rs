use dioxus::prelude::*;

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
