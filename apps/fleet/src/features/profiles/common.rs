use crate::style::{
    Button, ButtonSize, ButtonVariant, FieldRow, FieldRowMeta, FieldRowStack, TextField,
};
use dioxus::prelude::*;
use dioxus_router::Navigator;
use fleet_core::LocalStateMetrics;
use tracing::{error, info};

use crate::app::router::Route;
use crate::features::shared::browse_field::BrowseField;
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::ToastStore;

pub(crate) const UNEXPECTED_PATH_PREVIEW_LIMIT: usize = 6;

#[derive(Props, Clone, PartialEq)]
pub(crate) struct ProfileTextFieldRowProps {
    pub title: String,
    pub value: String,
    #[props(default)]
    pub placeholder: Option<String>,
    #[props(default)]
    pub class: Option<String>,
    #[props(default)]
    pub folder_select: bool,
    #[props(default)]
    pub disabled: bool,
    #[props(default)]
    pub open_folder_when_disabled: bool,
    #[props(default)]
    pub error: Option<String>,
    pub on_change: EventHandler<String>,
}

#[component]
pub(crate) fn ProfileTextFieldRow(props: ProfileTextFieldRowProps) -> Element {
    let class = props.class.unwrap_or_default();

    rsx! {
        div { class: class,
            FieldRow {
                FieldRowMeta { title: props.title }
                FieldRowStack {
                    if props.folder_select {
                        BrowseField {
                            value: props.value,
                            placeholder: props.placeholder,
                            disabled: props.disabled,
                            folder_select: true,
                            open_folder_when_disabled: props.open_folder_when_disabled,
                            invalid: props.error.is_some(),
                            on_change: move |v| props.on_change.call(v),
                        }
                    } else {
                        TextField {
                            value: props.value,
                            placeholder: props.placeholder,
                            disabled: props.disabled,
                            invalid: props.error.is_some(),
                            on_change: move |v| props.on_change.call(v),
                        }
                    }
                    if let Some(error) = props.error {
                        div { class: "field__error", "{error}" }
                    }
                }
            }
        }
    }
}

pub(crate) fn select_profile_in_background(bridge: FleetBridge, profile_id: String) {
    spawn(async move {
        let _ = bridge.core().profile_set_selected(Some(profile_id)).await;
    });
}

pub(crate) fn profile_not_found_page(nav: Navigator) -> Element {
    let nav_for_home = nav;
    rsx! {
        div { class: "page page--form-rows",
            div { class: "page__inner stack-sm",
                h1 { class: "page-header__title", "Profile not found" }
                p { class: "page__muted", "This profile no longer exists." }
                Button {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Lg,
                    onclick: move |_| {
                        let _ = nav_for_home.push(Route::Home {});
                    },
                    "Back to Home"
                }
            }
        }
    }
}

pub(crate) fn default_arma3_args(settings: &fleet_core::AppSettings) -> String {
    let v = settings.arma3.arma3_default_args.clone();
    if v.trim().is_empty() {
        fleet_core::DEFAULT_ARMA3_ARGS.to_string()
    } else {
        v
    }
}

pub(crate) fn new_profile_from_draft(
    draft: &crate::features::profiles::draft::ProfileDraft,
) -> fleet_core::Profile {
    let draft = draft.trimmed();
    fleet_core::Profile {
        id: String::new(),
        name: draft.name,
        source: draft.source,
        destination: draft.destination,
        ..Default::default()
    }
}

pub(crate) async fn save_profile_and_update_state(
    bridge: FleetBridge,
    mut store: AppStore,
    _toasts: ToastStore,
    profile: fleet_core::Profile,
    _warning_detail: &'static str,
) -> Result<fleet_core::Profile, fleet_core::ApiError> {
    let saved = bridge.core().profile_save(profile).await?;

    let mut next_state = (store.state)();
    next_state.profiles.insert(saved.id.clone(), saved.clone());
    next_state.selected_profile_id = Some(saved.id.clone());
    store.state.set(next_state);

    Ok(saved)
}

pub(crate) fn format_progress_metric(metric: &fleet_core::UiProgressMetric) -> String {
    metric.rendered.clone()
}

pub(crate) fn format_speed(bytes_per_sec: u64) -> String {
    format!("{}/s", fleet_domain::utils::format_bytes(bytes_per_sec))
}

pub(crate) fn format_eta(eta_seconds: u64) -> String {
    let minutes = eta_seconds / 60;
    let seconds = eta_seconds % 60;
    if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

pub(crate) fn inventory_out_of_sync(status: &fleet_core::ProfileStatusState) -> bool {
    matches!(
        status.local_health,
        fleet_core::LocalStateHealth::LocalDrift
            | fleet_core::LocalStateHealth::LocalStateMissing
            | fleet_core::LocalStateHealth::InventoryCorrupt
    )
}

pub(crate) fn modpack_size_text(metrics: Option<&LocalStateMetrics>, loading: bool) -> String {
    if loading && metrics.is_none() {
        return "Loading...".to_string();
    }
    let Some(metrics) = metrics else {
        return "Unavailable".to_string();
    };
    let bytes = metrics
        .last_stamp
        .as_ref()
        .map(|stamp| stamp.total_bytes)
        .unwrap_or(metrics.files_bytes);
    fleet_domain::utils::format_bytes(bytes)
}

pub(crate) fn preview_unexpected_paths(paths: &[String], limit: usize) -> (Vec<String>, usize) {
    let preview = paths.iter().take(limit).cloned().collect::<Vec<_>>();
    let remaining = paths.len().saturating_sub(preview.len());
    (preview, remaining)
}

pub(crate) fn show_unexpected_paths_panel(unexpected_available: bool, sync_running: bool) -> bool {
    unexpected_available && !sync_running
}

pub(crate) fn format_repo_server_label(server: &fleet_core::RepoServer) -> String {
    if server.port == 0 {
        server.address.clone()
    } else {
        format!("{}:{}", server.address, server.port)
    }
}

pub(crate) fn start_profile_operation(
    bridge: FleetBridge,
    toasts: ToastStore,
    profile_id: String,
    operation: fleet_core::OperationKind,
    action: &'static str,
    error_reason: &'static str,
    fail_title: &'static str,
) {
    spawn(async move {
        info!(
            op = "profile_action",
            profile_id = %profile_id,
            action = action,
            "profile operation requested"
        );
        if let Err(err) = bridge
            .core()
            .start_operation(profile_id.clone(), operation)
            .await
        {
            if err.code == "profile_busy" {
                return;
            }
            error!(
                op = "profile_action",
                profile_id = %profile_id,
                action = action,
                outcome = "failed",
                code = %err.code,
                reason = error_reason,
                "profile operation failed"
            );
            toasts.push_api_error(fail_title, &err);
        }
    });
}

pub(crate) fn cancel_operation(bridge: FleetBridge, toasts: ToastStore, session_id: u64) {
    spawn(async move {
        info!(
            op = "profile_action",
            session_id = session_id,
            action = "cancel",
            "profile cancel requested"
        );
        match bridge.core().cancel_session(session_id) {
            Ok(fleet_core::CancelResult::Requested) => {}
            Ok(fleet_core::CancelResult::AlreadyTerminal) => {
                info!(
                    op = "profile_action",
                    session_id = session_id,
                    action = "cancel",
                    outcome = "noop",
                    reason = "already_terminal",
                    "profile cancel ignored because session is already terminal"
                );
            }
            Ok(fleet_core::CancelResult::NotFound) => {
                info!(
                    op = "profile_action",
                    session_id = session_id,
                    action = "cancel",
                    outcome = "noop",
                    reason = "not_found",
                    "profile cancel ignored because session was not found"
                );
            }
            Err(err) => {
                error!(
                    op = "profile_action",
                    session_id = session_id,
                    action = "cancel",
                    outcome = "failed",
                    code = %err.code,
                    reason = "cancel_failed",
                    "profile cancel failed"
                );
                toasts.push_api_error("Cancel failed", &err);
            }
        }
    });
}

pub(crate) fn build_profile_edit_candidate(
    original: &fleet_core::Profile,
    draft: &crate::features::profiles::draft::ProfileDraft,
    use_default_args: bool,
    launch_params: &str,
    repo_servers: &[fleet_core::RepoServer],
    selected_repo_server: Option<usize>,
) -> fleet_core::Profile {
    let draft = draft.trimmed();
    let mut next = original.clone();
    next.name = draft.name;
    next.source = draft.source;
    next.destination = draft.destination;
    next.launch_params = if use_default_args {
        String::new()
    } else {
        launch_params.trim().to_string()
    };
    next.arma3_server = if repo_servers.is_empty() {
        original.arma3_server.clone()
    } else {
        selected_repo_server
            .and_then(|idx| repo_servers.get(idx))
            .map(|server| fleet_domain::types::ProfileServerInfo {
                address: server.address.clone(),
                port: server.port,
                password: server.password.clone(),
            })
    };
    next
}

pub(crate) fn profile_row_class() -> &'static str {
    "dash-profile-row dash-profile-row--edit"
}

pub(crate) fn profile_folder_row_class() -> &'static str {
    "dash-profile-row dash-profile-row--edit dash-folder-row dash-folder-row--edit"
}

#[cfg(test)]
mod tests {
    use super::{build_profile_edit_candidate, modpack_size_text, preview_unexpected_paths};

    #[test]
    fn unexpected_path_preview_truncates() {
        let paths = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ];
        let (preview, remaining) = preview_unexpected_paths(&paths, 2);
        assert_eq!(preview, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(remaining, 2);
    }

    #[test]
    fn profile_edit_candidate_differs_when_name_changes() {
        let profile = fleet_core::Profile {
            id: "p1".to_string(),
            name: "Alpha".to_string(),
            source: "https://example.com/repo.json".to_string(),
            destination: "/tmp/alpha".to_string(),
            ..Default::default()
        };
        let candidate = build_profile_edit_candidate(
            &profile,
            &crate::features::profiles::draft::ProfileDraft::from_fields(
                "Beta",
                profile.source.clone(),
                profile.destination.clone(),
            ),
            true,
            "",
            &[],
            None,
        );
        assert_ne!(candidate.name, profile.name);
        assert_eq!(candidate.source, profile.source);
        assert_eq!(candidate.destination, profile.destination);
    }

    #[test]
    fn modpack_size_prefers_stamp_total_bytes() {
        let metrics = fleet_core::LocalStateMetrics {
            root_path: "/tmp/x".to_string(),
            files_count: 1,
            files_bytes: 10,
            last_stamp: Some(fleet_core::BaselineStamp {
                algo: "quick-v1".to_string(),
                hash64: 1,
                file_count: 1,
                total_bytes: 20,
            }),
        };
        let text = modpack_size_text(Some(&metrics), false);
        assert!(text.contains("20"));
    }

    #[test]
    fn modpack_size_keeps_existing_metrics_visible_while_refreshing() {
        let metrics = fleet_core::LocalStateMetrics {
            root_path: "/tmp/x".to_string(),
            files_count: 1,
            files_bytes: 10,
            last_stamp: Some(fleet_core::BaselineStamp {
                algo: "quick-v1".to_string(),
                hash64: 1,
                file_count: 1,
                total_bytes: 20,
            }),
        };
        let text = modpack_size_text(Some(&metrics), true);
        assert!(text.contains("20"));
    }
}
