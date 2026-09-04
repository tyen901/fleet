use crate::ui::progress::spawn_flow_printer;
use fleet_core::{CheckReport, Core, LocalFileReport, OperationOutput, SyncReport};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlowOutput {
    Progress { no_progress: bool },
    Quiet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FlowRunOptions {
    pub(crate) output: FlowOutput,
}

pub(crate) async fn run_flow_session(
    core: &Core,
    session_id: u64,
    options: FlowRunOptions,
) -> anyhow::Result<OperationOutput> {
    let progress_handle = match options.output {
        FlowOutput::Progress { no_progress } => Some(spawn_flow_printer(
            session_id,
            core.subscribe_events(),
            no_progress,
        )),
        FlowOutput::Quiet => None,
    };

    let cancel_task = {
        let core = core.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = core.cancel_session(session_id);
        })
    };

    let result = core
        .await_finished(session_id)
        .await
        .map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message));

    cancel_task.abort();
    if let Some(handle) = progress_handle {
        handle.abort();
        let _ = handle.await;
    }

    result
}

pub(crate) async fn run_sync_session(
    core: &Core,
    session_id: u64,
    options: FlowRunOptions,
) -> anyhow::Result<SyncReport> {
    match run_flow_session(core, session_id, options).await? {
        OperationOutput::Sync(report) => Ok(report),
        _ => Err(anyhow::anyhow!("internal: expected sync result")),
    }
}

pub(crate) async fn run_check_session(
    core: &Core,
    session_id: u64,
    options: FlowRunOptions,
) -> anyhow::Result<CheckReport> {
    match run_flow_session(core, session_id, options).await? {
        OperationOutput::Check(report) => Ok(report),
        _ => Err(anyhow::anyhow!("internal: expected check result")),
    }
}

pub(crate) async fn run_validation_session(
    core: &Core,
    session_id: u64,
    options: FlowRunOptions,
) -> anyhow::Result<LocalFileReport> {
    match run_flow_session(core, session_id, options).await? {
        OperationOutput::Validate(report) => Ok(report),
        _ => Err(anyhow::anyhow!("internal: expected validation result")),
    }
}

#[cfg(test)]
mod tests {
    use super::{run_sync_session, FlowOutput, FlowRunOptions};
    use fleet_core::{Core, OperationKind, OperationSessionEventKind, Profile};
    use fleet_domain::AppSettings;
    use std::ffi::OsString;
    use std::sync::OnceLock;
    use std::time::Duration;

    static ENV_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = self.previous.as_ref() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[tokio::test]
    async fn completed_session_does_not_wait_for_a_missed_terminal_event() {
        let _lock = ENV_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        let temp_dir = tempfile::tempdir().expect("test config directory");
        let config_dir = EnvVarGuard::set("FLEET_CONFIG_DIR", temp_dir.path());
        let mut settings = AppSettings::default();
        settings.startup.auto_check_profiles_on_startup = false;
        let profile = Profile {
            id: "p1".to_string(),
            name: "Profile".to_string(),
            source: "https://example.com/repo.json".to_string(),
            destination: temp_dir.path().join("profile").display().to_string(),
            ..Default::default()
        };
        std::fs::write(
            temp_dir.path().join("settings.json"),
            serde_json::to_vec(&settings).expect("serialize settings"),
        )
        .expect("write settings");
        std::fs::write(
            temp_dir.path().join("profiles.json"),
            serde_json::to_vec(&serde_json::json!({ "profiles": [profile] }))
                .expect("serialize profiles"),
        )
        .expect("write profiles");
        let _simulate_sync = EnvVarGuard::set("FLEET_SIMULATE_SYNC", "1");
        let core = Core::new_in_current_runtime_default().expect("core");
        let mut state = core.subscribe_state();
        tokio::time::timeout(Duration::from_secs(1), async {
            while state.borrow().version == 0 || !state.borrow().profiles.contains_key("p1") {
                state
                    .changed()
                    .await
                    .expect("core state channel must stay open");
            }
        })
        .await
        .expect("core must load the fixture");

        let mut events = core.subscribe_events();
        let session_id = core
            .start_operation("p1".to_string(), OperationKind::Sync)
            .await
            .expect("start sync");
        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .expect("sync must finish")
                .expect("event channel must stay open");
            if event.session_id == session_id
                && matches!(
                    event.kind,
                    OperationSessionEventKind::Finished { .. }
                        | OperationSessionEventKind::Failed { .. }
                        | OperationSessionEventKind::Canceled
                )
            {
                break;
            }
        }

        let report = tokio::time::timeout(
            Duration::from_secs(1),
            run_sync_session(
                &core,
                session_id,
                FlowRunOptions {
                    output: FlowOutput::Progress { no_progress: true },
                },
            ),
        )
        .await
        .expect("completed session must not wait for the printer");
        assert_eq!(report.expect("sync output").profile_id, "p1");
        drop(config_dir);
    }
}
