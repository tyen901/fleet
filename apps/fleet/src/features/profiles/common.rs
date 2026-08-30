use crate::style::{Button, ButtonVariant, TextField};
use base64::Engine as _;
use dioxus::prelude::*;
use dioxus_router::Navigator;
use tracing::{error, info};

use crate::app::router::Route;
use crate::features::shared::browse_field::BrowseField;
use crate::services::bridge::FleetBridge;
use crate::stores::app_store::AppStore;
use crate::stores::toast_store::ToastStore;

#[derive(Props, Clone, PartialEq)]
pub(crate) struct ProfileFormFieldProps {
    pub title: String,
    pub value: String,
    #[props(default)]
    pub placeholder: Option<String>,
    #[props(default)]
    pub folder_select: bool,
    #[props(default)]
    pub pick_button_text: Option<String>,
    #[props(default = false)]
    pub show_open_button: bool,
    #[props(default)]
    pub open_button_text: Option<String>,
    #[props(default)]
    pub error: Option<String>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = false)]
    pub readonly: bool,
    pub on_change: EventHandler<String>,
}

/// A form field with its label above a full-width control.
#[component]
pub(crate) fn ProfileFormField(props: ProfileFormFieldProps) -> Element {
    rsx! {
        div { class: "form-field",
            span { class: "form-field__label", "{props.title}" }
            if props.folder_select {
                BrowseField {
                    value: props.value,
                    placeholder: props.placeholder,
                    readonly: props.readonly,
                    folder_select: true,
                    pick_button_text: props.pick_button_text,
                    show_open_button: props.show_open_button,
                    open_button_text: props.open_button_text,
                    invalid: props.error.is_some(),
                    on_change: move |v| props.on_change.call(v),
                }
            } else {
                TextField {
                    value: props.value,
                    placeholder: props.placeholder,
                    disabled: props.disabled,
                    readonly: props.readonly,
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

pub(crate) fn select_profile_in_background(bridge: FleetBridge, profile_id: String) {
    spawn(async move {
        let _ = bridge.core().profile_set_selected(Some(profile_id)).await;
    });
}

pub(crate) fn profile_not_found_page(nav: Navigator) -> Element {
    let nav_for_profiles = nav;
    rsx! {
        div { class: "page-frame",
            div { class: "page-frame__body",
                div { class: "page__inner stack-sm",
                    h1 { class: "page-title", "Profile not found" }
                    p { class: "page__muted", "This profile no longer exists." }
                    Button {
                        variant: ButtonVariant::Primary,
                        onclick: move |_| {
                            let _ = nav_for_profiles.push(Route::Profiles {});
                        },
                        "Back to profiles"
                    }
                }
            }
        }
    }
}

pub(crate) fn profile_icon_src(
    settings: &fleet_core::AppSettings,
    profile: &fleet_core::Profile,
) -> Option<String> {
    if !settings.ui.show_profile_icons {
        return None;
    }

    let repo_url = profile.source.trim();
    if repo_url.is_empty() {
        return None;
    }

    let state_root = fleet_core::profile_state_root_dir().ok()?;
    let repo_cache_root = fleet_domain::repo_cache_dir(&state_root, &profile.id);
    let icon_path = swifty_repo::repo_cache_asset_path(&repo_cache_root, repo_url, "icon.png");
    if !icon_path.is_file() {
        return None;
    }

    let icon_bytes = std::fs::read(icon_path).ok()?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(icon_bytes);
    Some(format!("data:image/png;base64,{encoded}"))
}

pub(crate) fn stage_phase_label(stage: fleet_core::OperationStage) -> &'static str {
    match stage {
        fleet_core::OperationStage::Validating => "Checking",
        fleet_core::OperationStage::LoadingExpectedState => "Planning",
        fleet_core::OperationStage::ScanningDisk
        | fleet_core::OperationStage::VerifyingInventory => "Verifying",
        fleet_core::OperationStage::Sync => "Downloading",
        fleet_core::OperationStage::RemovingObsoleteFiles => "Removing obsolete files",
        fleet_core::OperationStage::Finalizing => "Installing",
    }
}

pub(crate) fn format_clock(total_seconds: u64) -> String {
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
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

pub(crate) fn format_speed(bytes_per_sec: u64) -> String {
    format!("{}/s", fleet_domain::utils::format_bytes(bytes_per_sec))
}

pub(crate) fn local_files_need_sync(status: &fleet_core::ProfileStatusState) -> bool {
    matches!(
        status.local_health,
        fleet_core::LocalFileHealth::Missing
            | fleet_core::LocalFileHealth::Dirty
            | fleet_core::LocalFileHealth::MissingDestination
            | fleet_core::LocalFileHealth::ExpectedStateUnavailable
            | fleet_core::LocalFileHealth::InventoryUnavailable
    )
}

pub(crate) fn repo_update_available(
    status: Option<&fleet_core::ProfileStatusState>,
    operation_active: bool,
) -> bool {
    !operation_active
        && status.is_some_and(|status| {
            status.repo_freshness == Some(fleet_core::RepoCheckFreshness::UpdateAvailable)
        })
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
        start_profile_operation_request(
            bridge,
            toasts,
            profile_id,
            operation,
            action,
            error_reason,
            fail_title,
        )
        .await;
    });
}

pub(crate) async fn start_profile_operation_request(
    bridge: FleetBridge,
    toasts: ToastStore,
    profile_id: String,
    operation: fleet_core::OperationKind,
    action: &'static str,
    error_reason: &'static str,
    fail_title: &'static str,
) -> bool {
    info!(
        op = "profile_action",
        profile_id = %profile_id,
        action = action,
        "profile operation requested"
    );
    match bridge
        .core()
        .start_operation(profile_id.clone(), operation)
        .await
    {
        Ok(_) => true,
        Err(err) => {
            if err.code == "profile_busy" {
                return false;
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
            false
        }
    }
}

pub(crate) fn build_profile_edit_candidate(
    original: &fleet_core::Profile,
    draft: &crate::features::profiles::draft::ProfileDraft,
    use_default_args: bool,
    launch_params: &str,
    additional_mod_folders: &[String],
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
    next.additional_mod_folders = additional_mod_folders
        .iter()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
        .collect();
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

#[cfg(test)]
mod tests {
    use super::{build_profile_edit_candidate, repo_update_available};

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
            &[],
            None,
        );
        assert_ne!(candidate.name, profile.name);
        assert_eq!(candidate.source, profile.source);
        assert_eq!(candidate.destination, profile.destination);
    }

    #[test]
    fn user_story_update_action_appears_only_after_check_detects_an_update() {
        let mut status = fleet_core::ProfileStatusState::unknown(0);
        assert!(!repo_update_available(Some(&status), false));

        status.repo_freshness = Some(fleet_core::RepoCheckFreshness::UpToDate);
        assert!(!repo_update_available(Some(&status), false));

        status.repo_freshness = Some(fleet_core::RepoCheckFreshness::UpdateAvailable);
        assert!(repo_update_available(Some(&status), false));
        assert!(!repo_update_available(Some(&status), true));
    }
}
