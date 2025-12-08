use std::sync::Arc;
use tokio::sync::mpsc;

use crate::app_core::{AppCommand, DomainEvent};
use crate::domain::{Profile, ProfileId, Route};
use crate::pipeline::PipelineRunId;
use crate::ports::{LauncherPort, ProfilesRepo, SettingsRepo, SyncPipelinePort};

pub struct AppKernel<P, S, L, Y> {
    pub store: crate::app_core::AppStore,
    profiles: Arc<P>,
    settings: Arc<S>,
    launcher: Arc<L>,
    sync: Arc<Y>,

    tx: mpsc::Sender<DomainEvent>,
    rx: mpsc::Receiver<DomainEvent>,
}

impl<P, S, L, Y> AppKernel<P, S, L, Y>
where
    P: ProfilesRepo,
    S: SettingsRepo,
    L: LauncherPort,
    Y: SyncPipelinePort,
{
    pub fn new(
        store: crate::app_core::AppStore,
        profiles: P,
        settings: S,
        launcher: L,
        sync: Y,
    ) -> Self {
        let (tx, rx) = mpsc::channel(100);
        Self {
            store,
            profiles: Arc::new(profiles),
            settings: Arc::new(settings),
            launcher: Arc::new(launcher),
            sync: Arc::new(sync),
            tx,
            rx,
        }
    }

    pub fn dispatch(&mut self, cmd: AppCommand) {
        match cmd {
            AppCommand::LoadInitialState => {
                self.store.apply(DomainEvent::BootLoadingStarted);
                let tx = self.tx.clone();
                let profiles = self.profiles.clone();
                let settings = self.settings.clone();
                let spawn_res = std::thread::Builder::new()
                    .name("fleet-load-initial-state".into())
                    .spawn(move || {
                        let res: anyhow::Result<(Vec<Profile>, crate::domain::AppSettings)> =
                            (|| {
                                let p = profiles.load()?;
                                let s = settings.load()?;
                                Ok((p, s))
                            })();

                        match res {
                            Ok((p, s)) => {
                                let _ = tx.blocking_send(DomainEvent::InitialStateLoaded {
                                    profiles: p,
                                    settings: s,
                                });
                            }
                            Err(e) => {
                                let _ = tx.blocking_send(DomainEvent::BootFailed {
                                    message: e.to_string(),
                                });
                            }
                        }
                    });

                if let Err(e) = spawn_res {
                    self.store.apply(DomainEvent::BootFailed {
                        message: format!("Failed to start boot worker thread: {e}"),
                    });
                }
            }

            AppCommand::Navigate(r) => self.store.apply(DomainEvent::RouteChanged(r)),

            AppCommand::StartNewProfile => {
                let p = Profile::default();
                self.store.apply(DomainEvent::DraftOpened(p));
                self.store
                    .apply(DomainEvent::RouteChanged(Route::ProfileEditor(
                        String::new(),
                    )));
            }

            AppCommand::EditProfile(id) => {
                if let Some(p) = self
                    .store
                    .state()
                    .profiles
                    .iter()
                    .find(|p| p.id == id)
                    .cloned()
                {
                    self.store.apply(DomainEvent::DraftOpened(p));
                    self.store
                        .apply(DomainEvent::RouteChanged(Route::ProfileEditor(id)));
                }
            }

            AppCommand::SaveProfileDraft => {
                let draft = self.store.state().editor_draft.clone();
                if let Some(draft) = draft {
                    let id = draft.id.trim();
                    if id.is_empty() {
                        self.store
                            .apply(DomainEvent::UserError("Profile ID cannot be empty".into()));
                        return;
                    }

                    if !id
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                    {
                        self.store.apply(DomainEvent::UserError(
                            "Profile ID must use only a-z, 0-9, - and _".into(),
                        ));
                        return;
                    }

                    if let Route::ProfileEditor(route_id) = self.store.state().route.clone() {
                        // New profile: route_id is empty; enforce uniqueness.
                        // Existing profile: disallow changing ID (for now) to avoid implicit renames.
                        if route_id.is_empty() {
                            if self.store.state().profiles.iter().any(|p| p.id == draft.id) {
                                self.store.apply(DomainEvent::UserError(format!(
                                    "Profile ID '{}' already exists",
                                    draft.id
                                )));
                                return;
                            }
                        } else if draft.id != route_id {
                            self.store.apply(DomainEvent::UserError(
                                "Profile ID cannot be changed after creation".into(),
                            ));
                            return;
                        }
                    }

                    let validate_res = self.sync.validate_repo_url_blocking(&draft.repo_url);
                    if let Err(e) = validate_res {
                        self.store.apply(DomainEvent::UserError(format!(
                            "Repository URL validation failed: {e}"
                        )));
                        return;
                    }

                    self.store.apply(DomainEvent::DraftCommitted(draft));
                    self.store
                        .apply(DomainEvent::RouteChanged(Route::ProfileHub));

                    let profiles_repo = self.profiles.clone();
                    let tx = self.tx.clone();
                    let profiles_snapshot = self.store.state().profiles;
                    let spawn_res = std::thread::Builder::new()
                        .name("fleet-save-profiles".into())
                        .spawn(move || {
                            if let Err(e) = profiles_repo.save(&profiles_snapshot) {
                                let _ = tx.blocking_send(DomainEvent::UserError(e.to_string()));
                            }
                        });
                    if let Err(e) = spawn_res {
                        self.store.apply(DomainEvent::UserError(format!(
                            "Failed to start profiles save worker thread: {e}"
                        )));
                    }
                }
            }

            AppCommand::CancelProfileDraft => {
                self.store.apply(DomainEvent::DraftCancelled);
                self.store
                    .apply(DomainEvent::RouteChanged(Route::ProfileHub));
            }

            AppCommand::DeleteProfile(id) => {
                let profiles_repo = self.profiles.clone();
                let tx = self.tx.clone();
                self.store.with_state_mut(|state| {
                    state.profiles.retain(|p| p.id != id);
                    if state.selected_profile_id == Some(id) {
                        state.selected_profile_id = state.profiles.first().map(|p| p.id.clone());
                        state.route = state
                            .selected_profile_id
                            .clone()
                            .map(Route::ProfileDashboard)
                            .unwrap_or(Route::ProfileHub);
                    }
                });

                let profiles_snapshot = self.store.state().profiles;
                let spawn_res = std::thread::Builder::new()
                    .name("fleet-delete-profile".into())
                    .spawn(move || {
                        if let Err(e) = profiles_repo.save(&profiles_snapshot) {
                            let _ = tx.blocking_send(DomainEvent::UserError(e.to_string()));
                        }
                    });
                if let Err(e) = spawn_res {
                    self.store.apply(DomainEvent::UserError(format!(
                        "Failed to start profiles save worker thread: {e}"
                    )));
                }
            }

            AppCommand::StartCheck(profile_id) => {
                let run_id: PipelineRunId = uuid::Uuid::new_v4();
                self.store.with_state_mut(|state| {
                    state.pipeline.run_id = Some(run_id);
                });
            }

            AppCommand::ExecuteSync(profile_id) => {
                let run_id: PipelineRunId = uuid::Uuid::new_v4();
                self.store.with_state_mut(|state| {
                    state.pipeline.run_id = Some(run_id);
                });
            }

            AppCommand::CancelPipeline => {}

            AppCommand::Launch(profile_id) => {
                let snapshot = self.store.state();
                if let Some(profile) = snapshot.profiles.iter().find(|p| p.id == profile_id) {
                    let _ = self.launcher.launch(
                        "",
                        &snapshot.settings.launch_params,
                        &snapshot.settings.launch_template,
                        &[],
                    );
                }
            }
        }
    }

    pub fn tick(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            if let DomainEvent::PipelineEvent { run_id, .. } = &ev {
                let current = self.store.state().pipeline.run_id;
                if current != Some(*run_id) {
                    continue;
                }
            }
            self.store.apply(ev);
        }
    }

    pub fn sender(&self) -> mpsc::Sender<DomainEvent> {
        self.tx.clone()
    }
}
