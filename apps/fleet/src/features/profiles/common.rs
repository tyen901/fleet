use crate::style::{Button, ButtonVariant, TextField};
use base64::Engine as _;
use dioxus::dioxus_core::Task;
use dioxus::prelude::*;
use dioxus_i18n::t;
use dioxus_router::Navigator;
use tracing::{error, info};

use crate::app::router::Route;
use crate::features::action_error::ActionError;
use crate::features::shared::browse_field::BrowseField;
use crate::services::bridge::FleetBridge;

#[derive(Props, Clone, PartialEq)]
pub(crate) struct ProfileFormFieldProps {
    pub title: String,
    pub value: String,
    #[props(default)]
    pub placeholder: Option<String>,
    #[props(default)]
    pub folder_select: bool,
    #[props(default)]
    pub action: Option<Element>,
    #[props(default)]
    pub feedback: Option<Element>,
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
                    disabled: props.disabled,
                    folder_select: true,
                    compact_picker: true,
                    accessible_label: Some(props.title.clone()),
                    pick_button_text: Some(t!("browse-mod-install-folder")),
                    invalid: props.error.is_some(),
                    on_change: move |v| props.on_change.call(v),
                }
            } else {
                TextField {
                    value: props.value,
                    accessible_label: Some(props.title.clone()),
                    action: props.action,
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
            {props.feedback}
        }
    }
}

#[component]
pub(crate) fn ProfileSourceField(
    value: String,
    #[props(default)] readonly: bool,
    on_change: EventHandler<String>,
) -> Element {
    let bridge = use_context::<FleetBridge>();
    let mut test_task = use_signal(|| None::<Task>);
    let mut test_result = use_signal(|| None::<Result<(), fleet_core::ApiError>>);
    let valid = fleet_domain::validated_repo_url(&value).is_ok();
    let testing = test_task().is_some();

    let test_source = value.clone();
    rsx! {
        ProfileFormField {
            title: t!("profile-source-url"),
            placeholder: Some(t!("profile-repo-url-placeholder")),
            error: (!valid && !value.trim().is_empty()).then(|| {
                t!("invalid-profile-source")
            }),
            readonly,
            value,
            on_change: move |value| {
                if let Some(task) = test_task.take() {
                    task.cancel();
                }
                test_result.set(None);
                on_change.call(value);
            },
            action: Some(rsx! {
                Button {
                    variant: ButtonVariant::Secondary,
                    disabled: !valid || readonly,
                    loading: testing,
                    onclick: move |_| {
                        let core = bridge.core();
                        let source = test_source.clone();
                        test_result.set(None);
                        test_task.set(Some(spawn(async move {
                            let result = core.test_profile_source(&source).await;
                            test_result.set(Some(result));
                            test_task.set(None);
                        })));
                    },
                    if testing { {t!("testing-url")} } else { {t!("test-url")} }
                }
            }),
            feedback: test_result().map(|result| rsx! {
                p { class: "field__error", role: "status",
                    match result {
                        Ok(()) => t!("manifest-found"),
                        Err(error) => error.message,
                    }
                }
            }),
        }
    }
}

pub(crate) fn profile_not_found_page(nav: Navigator) -> Element {
    let nav_for_profiles = nav;
    rsx! {
        div { class: "page-frame",
            div { class: "page-frame__body",
                div { class: "page__inner stack-sm",
                    h1 { class: "page-title", {t!("profile-not-found")} }
                    p { class: "page__muted", {t!("profile-gone")} }
                    Button {
                        variant: ButtonVariant::Primary,
                        onclick: move |_| {
                            let _ = nav_for_profiles.push(Route::Profiles {});
                        },
                        {t!("back-to-profiles")}
                    }
                }
            }
        }
    }
}

pub(crate) fn profile_icon_src(
    core: &fleet_core::Core,
    settings: &fleet_core::AppSettings,
    profile: &fleet_core::Profile,
) -> Option<String> {
    if !settings.ui.show_profile_icons {
        return None;
    }

    let repo_url = fleet_domain::validated_repo_url(&profile.source).ok()?;

    let state_root = core.profile_state_root_dir().ok()?;
    let repo_cache_root = fleet_domain::repo_cache_dir(&state_root, &profile.id);
    let icon_path = swifty_repo::repo_icon_cache_path(&repo_cache_root, repo_url);
    if !icon_path.is_file() {
        return None;
    }

    let icon_bytes = std::fs::read(icon_path).ok()?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(icon_bytes);
    Some(format!("data:image/png;base64,{encoded}"))
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

pub(crate) fn format_repo_server_label(server: &fleet_core::RepoServer) -> String {
    if server.port == 0 {
        server.address.clone()
    } else {
        format!("{}:{}", server.address, server.port)
    }
}

pub(crate) fn start_profile_operation(
    bridge: FleetBridge,
    feedback: ActionError,
    profile_id: String,
    operation: fleet_core::OperationKind,
    action: &'static str,
    error_reason: &'static str,
) {
    spawn(async move {
        start_profile_operation_request(
            bridge,
            feedback,
            profile_id,
            operation,
            action,
            error_reason,
        )
        .await;
    });
}

pub(crate) async fn start_profile_operation_request(
    bridge: FleetBridge,
    feedback: ActionError,
    profile_id: String,
    operation: fleet_core::OperationKind,
    action: &'static str,
    error_reason: &'static str,
) -> bool {
    feedback.clear();
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
        Ok(session_id) => {
            let core = bridge.core();
            tokio::spawn(async move {
                let _ = core.await_finished(session_id).await;
            });
            true
        }
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
            feedback.set(&err);
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
    use super::build_profile_edit_candidate;

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
}
