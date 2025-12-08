use fleet_app_core::app_core::{AppStore, DomainEvent};
use fleet_app_core::domain::AppState;
use fleet_app_core::kernel::AppKernel;
use fleet_app_core::pipeline::{PipelineRunEvent, PipelineRunId, PipelineStep, StepStatus};
use fleet_app_core::ports::{LauncherPort, ProfilesRepo, SettingsRepo, SyncPipelinePort};

struct DummyProfilesRepo;
impl ProfilesRepo for DummyProfilesRepo {
    fn load(&self) -> anyhow::Result<Vec<fleet_app_core::Profile>> {
        Ok(vec![])
    }
    fn save(&self, _profiles: &[fleet_app_core::Profile]) -> anyhow::Result<()> {
        Ok(())
    }
}

struct DummySettingsRepo;
impl SettingsRepo for DummySettingsRepo {
    fn load(&self) -> anyhow::Result<fleet_app_core::AppSettings> {
        Ok(fleet_app_core::AppSettings::default())
    }
    fn save(&self, _settings: &fleet_app_core::AppSettings) -> anyhow::Result<()> {
        Ok(())
    }
}

struct DummyLauncher;
impl LauncherPort for DummyLauncher {
    fn launch(
        &self,
        _exe_path: &str,
        _params: &str,
        _template: &str,
        _mods: &[camino::Utf8PathBuf],
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

struct DummySync;
impl SyncPipelinePort for DummySync {
    fn validate_repo_url_blocking(&self, _repo_url: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn stale_pipeline_events_are_ignored_in_tick() {
    let current: PipelineRunId = uuid::Uuid::new_v4();
    let stale: PipelineRunId = uuid::Uuid::new_v4();

    let mut state = AppState::default();
    state.pipeline.run_id = Some(current);

    let store = AppStore::new(state);
    let mut kernel = AppKernel::new(
        store.clone(),
        DummyProfilesRepo,
        DummySettingsRepo,
        DummyLauncher,
        DummySync,
    );

    let before = store.state();

    kernel
        .sender()
        .send(DomainEvent::PipelineEvent {
            run_id: stale,
            ev: PipelineRunEvent::StepChanged {
                step: PipelineStep::Fetch,
                status: StepStatus::Failed,
                detail: "stale".into(),
            },
        })
        .await
        .unwrap();

    kernel.tick();

    let after = store.state();
    assert_eq!(before.pipeline.run_id, after.pipeline.run_id);
    assert_eq!(before.pipeline.error, after.pipeline.error);
    assert_eq!(before.pipeline.fetch_status, after.pipeline.fetch_status);
}
