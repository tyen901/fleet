use dioxus::prelude::*;

use crate::features::action_error::ActionError;

pub(crate) fn spawn_settings_task<F>(feedback: ActionError, task: F)
where
    F: std::future::Future<Output = Result<(), fleet_core::ApiError>> + 'static,
{
    feedback.clear();
    spawn(async move {
        if let Err(err) = task.await {
            feedback.set(&err);
        }
    });
}
