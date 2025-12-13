use tokio::sync::mpsc;

use crate::app_core::{reduce, DomainEvent};
use crate::domain::{AppSettings, AppState, FlatpakSteamAvailability, Profile, ProfileId, Route};
use crate::launcher::LauncherImpl;
use crate::orchestrator::PipelineOrchestrator;
use crate::persistence::FilePersistence;
use crate::pipeline::{PipelineRunEvent, PipelineRunId, StepStatus};
use crate::ports::SyncPipelinePort;

use fleet_core::repo::Repository;
use fleet_core::SyncPlan;
use std::fs;
use std::path::Path;
use std::process::Command;

pub struct FleetApplication {
    pub state: AppState,

    // Concrete Implementations
    persistence: FilePersistence,
    launcher: LauncherImpl,
    orchestrator: PipelineOrchestrator,

    msg_rx: mpsc::Receiver<DomainEvent>,
    msg_tx: mpsc::Sender<DomainEvent>,
}

impl Default for FleetApplication {
    fn default() -> Self {
        Self::new()
    }
}
impl FleetApplication {
    pub fn new() -> Self {
        let (msg_tx, msg_rx) = mpsc::channel(100);
        let client =
            fleet_infra::net::default_http_client().unwrap_or_else(|_| reqwest::Client::new());
        let engine = fleet_pipeline::default_engine(client);
        let engine = std::sync::Arc::new(engine);

        Self {
            state: AppState::default(),
            persistence: FilePersistence::new(),
            launcher: LauncherImpl::new(),
            orchestrator: PipelineOrchestrator::new(engine, msg_tx.clone()),
            msg_rx,
            msg_tx,
        }
    }

    pub fn load_initial_state(&mut self) -> anyhow::Result<()> {
        let profiles = self.persistence.load_profiles()?;
        let settings = self.persistence.load_settings()?;

        self.state.profiles = profiles;
        self.state.settings = settings;
        self.state.flatpak_steam = detect_flatpak_steam_availability();
        self.state.selected_profile_id = self.state.profiles.first().map(|p| p.id.clone());
        self.state.route = if let Some(ref id) = self.state.selected_profile_id {
            Route::ProfileDashboard(id.clone())
        } else {
            Route::ProfileHub
        };
        Ok(())
    }

    // --- Actions ---

    /// Full remote check - fetch remote manifest and compare against local state.
    pub fn start_check(&mut self, profile_id: ProfileId) -> anyhow::Result<()> {
        let profile = self.get_profile(profile_id)?.clone();
        let run_id: PipelineRunId = uuid::Uuid::new_v4();
        self.state.pipeline.run_id = Some(run_id);
        self.state.last_plan = None;

        if let Err(e) = self
            .orchestrator
            .start_check(profile, self.state.settings.clone(), run_id, false)
        {
            self.state = reduce(self.state.clone(), DomainEvent::UserError(e.to_string()));
            return Err(e);
        }
        Ok(())
    }

    /// Fast local-only check - compares local files against cached local state.
    pub fn start_local_check(&mut self, profile_id: ProfileId) -> anyhow::Result<()> {
        let profile = self.get_profile(profile_id)?.clone();
        let run_id: PipelineRunId = uuid::Uuid::new_v4();
        self.state.pipeline.run_id = Some(run_id);
        // Do not clear last_plan here; remote comparison is unchanged.

        if let Err(e) = self
            .orchestrator
            .start_check(profile, self.state.settings.clone(), run_id, true)
        {
            self.state = reduce(self.state.clone(), DomainEvent::UserError(e.to_string()));
            return Err(e);
        }
        Ok(())
    }

    pub fn execute_sync(&mut self, profile_id: ProfileId) -> anyhow::Result<()> {
        // Do not start sync if no plan is available
        if self.state.last_plan.is_none() {
            return Ok(());
        }

        let profile = self.get_profile(profile_id)?.clone();
        let plan = self.state.last_plan.clone().unwrap();

        let run_id: PipelineRunId = uuid::Uuid::new_v4();
        self.state.pipeline.run_id = Some(run_id);
        if let Err(e) =
            self.orchestrator
                .start_sync(profile, plan, self.state.settings.clone(), run_id)
        {
            self.state = reduce(self.state.clone(), DomainEvent::UserError(e.to_string()));
            return Err(e);
        }
        Ok(())
    }

    pub fn cancel_pipeline(&mut self) {
        self.orchestrator.cancel();
        let run_id = self
            .state
            .pipeline
            .run_id
            .unwrap_or_else(uuid::Uuid::new_v4);
        let _ = self.msg_tx.try_send(DomainEvent::PipelineEvent {
            run_id,
            ev: PipelineRunEvent::Cancelled,
        });
    }

    pub fn acknowledge_pipeline_completion(&mut self) {
        self.state.pipeline =
            crate::pipeline::PipelineState::idle_for(self.state.selected_profile_id.clone())
                .with_run_id(self.state.pipeline.run_id);
        self.state.pipeline.error = None;
    }

    pub fn launch_profile(&mut self, profile_id: ProfileId) -> anyhow::Result<()> {
        let profile = self.get_profile(profile_id)?;

        let repo = load_local_repo_json(&profile.local_path);
        let mods_from_repo = repo
            .as_ref()
            .map(|r| enabled_mod_paths(r, &profile.local_path))
            .unwrap_or_default();
        let mods = if !mods_from_repo.is_empty() {
            mods_from_repo
        } else {
            discover_mod_dirs(&profile.local_path)
        };

        let params = self.state.settings.launch_params.trim().to_string();

        self.launcher
            .launch("", &params, &self.state.settings.launch_template, &mods)
    }

    pub fn join_profile(&mut self, profile_id: ProfileId) -> anyhow::Result<()> {
        let profile = self.get_profile(profile_id)?;

        let repo = load_local_repo_json(&profile.local_path)
            .ok_or_else(|| anyhow::anyhow!("No repo.json found in {}", profile.local_path))?;

        let mods_from_repo = enabled_mod_paths(&repo, &profile.local_path);
        let mods = if !mods_from_repo.is_empty() {
            mods_from_repo
        } else {
            discover_mod_dirs(&profile.local_path)
        };

        let server = repo
            .servers
            .first()
            .ok_or_else(|| anyhow::anyhow!("No servers configured in repo.json"))?;

        let mut params = self.state.settings.launch_params.trim().to_string();
        let mut join_args = format!("-connect={} -port={}", server.address, server.port);
        if !server.password.trim().is_empty() {
            join_args.push_str(&format!(" -password={}", server.password));
        }

        if params.is_empty() {
            params = join_args;
        } else {
            params = format!("{params} {join_args}");
        }

        self.launcher
            .launch("", &params, &self.state.settings.launch_template, &mods)
    }

    // --- State Management ---

    /// Call this from your UI loop/tick to process async messages
    pub fn handle_pipeline_events(&mut self) {
        while let Ok(ev) = self.msg_rx.try_recv() {
            if let DomainEvent::PipelineEvent { run_id, .. } = &ev {
                if self.state.pipeline.run_id != Some(*run_id) {
                    continue;
                }
            }
            self.state = reduce(self.state.clone(), ev);
        }
    }

    // --- CRUD boilerplate (simplified) ---

    pub fn get_profile(&self, id: ProfileId) -> anyhow::Result<&Profile> {
        self.state
            .profiles
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| anyhow::anyhow!("Profile not found"))
    }

    pub fn is_pipeline_running(&self) -> bool {
        self.state.pipeline.is_running()
    }

    pub fn navigate(&mut self, route: Route) {
        if !matches!(route, Route::Settings) {
            self.state.settings_draft = None;
        }
        self.state.route = route;
    }
    pub fn editor_draft(&self) -> Option<&Profile> {
        self.state.editor_draft.as_ref()
    }
    pub fn start_new_profile(&mut self) {
        self.state.editor_draft = Some(Profile::default());
        self.state.route = Route::ProfileEditor(String::new());
    }
    pub fn edit_profile(&mut self, id: ProfileId) {
        if let Ok(p) = self.get_profile(id.clone()) {
            self.state.editor_draft = Some(p.clone());
            self.state.route = Route::ProfileEditor(id);
        }
    }
    pub fn save_profile(&mut self) -> anyhow::Result<()> {
        if let Some(draft) = self.state.editor_draft.clone() {
            // Optimistically commit and close draft via reducer
            self.state = reduce(
                self.state.clone(),
                DomainEvent::DraftCommitted(draft.clone()),
            );
            self.state = reduce(
                self.state.clone(),
                DomainEvent::RouteChanged(Route::ProfileHub),
            );

            let profiles_snapshot = self.state.profiles.clone();
            let repo_url = draft.repo_url.clone();
            let tx = self.msg_tx.clone();
            let reopen_draft = draft.clone();
            let reopen_id = draft.id.clone();
            let reopen_draft_for_thread = reopen_draft.clone();
            let reopen_id_for_thread = reopen_id.clone();

            let spawn_res = std::thread::Builder::new()
                .name("fleet-save-profile".into())
                .spawn(move || {
                    let res: anyhow::Result<()> = (|| {
                        let client = fleet_infra::net::default_http_client()
                            .unwrap_or_else(|_| reqwest::Client::new());
                        let engine = fleet_pipeline::default_engine(client);
                        crate::async_runtime::runtime()?
                            .block_on(engine.validate_repo_url(&repo_url))?;

                        let persistence = FilePersistence::new();
                        persistence.save_profiles(&profiles_snapshot)?;
                        Ok(())
                    })();

                    if let Err(e) = res {
                        let _ = tx.blocking_send(DomainEvent::UserError(e.to_string()));
                        let _ = tx.blocking_send(DomainEvent::DraftOpened(reopen_draft_for_thread));
                        let _ = tx.blocking_send(DomainEvent::RouteChanged(Route::ProfileEditor(
                            reopen_id_for_thread,
                        )));
                    }
                });

            if let Err(e) = spawn_res {
                let msg = format!("Failed to start profile save worker thread: {e}");
                let _ = self.msg_tx.try_send(DomainEvent::UserError(msg));
                let _ = self.msg_tx.try_send(DomainEvent::DraftOpened(reopen_draft));
                let _ = self
                    .msg_tx
                    .try_send(DomainEvent::RouteChanged(Route::ProfileEditor(reopen_id)));
            }
        }
        Ok(())
    }
    pub fn cancel_edit(&mut self) {
        self.state.editor_draft = None;
        self.state.route = Route::ProfileHub;
    }
    pub fn delete_profile(&mut self, id: ProfileId) -> anyhow::Result<()> {
        self.state.profiles.retain(|p| p.id != id);
        self.persistence.save_profiles(&self.state.profiles)?;
        Ok(())
    }
    pub fn update_settings(&mut self, s: AppSettings) -> anyhow::Result<()> {
        self.state.pipeline.error = None;
        self.state.settings = s.clone();
        self.persistence.save_settings(&s)
    }
}

fn detect_flatpak_steam_availability() -> FlatpakSteamAvailability {
    #[cfg(not(target_os = "linux"))]
    {
        return FlatpakSteamAvailability::Unavailable("Flatpak is only supported on Linux".into());
    }

    #[cfg(target_os = "linux")]
    {
        let flatpak = Command::new("flatpak").arg("--version").output();
        if let Err(e) = flatpak {
            return FlatpakSteamAvailability::Unavailable(format!("`flatpak` not found: {e}"));
        }

        let checks = [
            vec!["info", "com.valvesoftware.Steam"],
            vec!["info", "--user", "com.valvesoftware.Steam"],
            vec!["info", "--system", "com.valvesoftware.Steam"],
        ];

        for args in checks {
            match Command::new("flatpak").args(args).output() {
                Ok(out) => {
                    if out.status.success() {
                        return FlatpakSteamAvailability::Available;
                    }
                    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    let _ = stderr;
                }
                Err(e) => {
                    return FlatpakSteamAvailability::Unavailable(format!(
                        "Failed to run `flatpak`: {e}"
                    ));
                }
            }
        }

        FlatpakSteamAvailability::Unavailable(
            "Flatpak app `com.valvesoftware.Steam` is not installed".into(),
        )
    }
}

fn load_local_repo_json(local_root: &str) -> Option<Repository> {
    let repo_path = Path::new(local_root).join("repo.json");
    let content = fs::read_to_string(repo_path).ok()?;
    serde_json::from_str(&content).ok()
}

fn enabled_mod_paths(repo: &Repository, local_root: &str) -> Vec<camino::Utf8PathBuf> {
    let root = camino::Utf8PathBuf::from(local_root.to_string());
    let mut mods = Vec::new();

    for m in &repo.required_mods {
        mods.push(root.join(m.mod_name.trim()));
    }

    for m in &repo.optional_mods {
        if m.enabled {
            mods.push(root.join(m.mod_name.trim()));
        }
    }

    mods
}

fn discover_mod_dirs(local_root: &str) -> Vec<camino::Utf8PathBuf> {
    let mut mods = Vec::new();
    let entries = match fs::read_dir(local_root) {
        Ok(v) => v,
        Err(_) => return mods,
    };

    for entry in entries.flatten() {
        let ft = match entry.file_type() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if !ft.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with('@') {
            continue;
        }

        let utf = match camino::Utf8PathBuf::from_path_buf(entry.path()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        mods.push(utf);
    }

    mods.sort();
    mods
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_json_with_hostname_server_address_parses_and_resolves_mods() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_string_lossy().to_string();

        let repo_json = r#"
        {
          "repoName": "pca",
          "checksum": "abc",
          "requiredMods": [
            { "modName": "@ace", "checkSum": "111", "enabled": true }
          ],
          "optionalMods": [
            { "modName": "@optional", "checkSum": "222", "enabled": true }
          ],
          "servers": [
            {
              "name": "Test",
              "address": "server.example.com",
              "port": 2302,
              "password": "",
              "battleEye": false
            }
          ]
        }
        "#;

        fs::write(Path::new(&root).join("repo.json"), repo_json).expect("write repo.json");

        let repo = load_local_repo_json(&root).expect("repo.json should parse");
        let mods = enabled_mod_paths(&repo, &root);

        assert!(mods
            .iter()
            .any(|p| p.as_str().ends_with(r"\@ace") || p.as_str().ends_with("/@ace")));
        assert!(mods
            .iter()
            .any(|p| p.as_str().ends_with(r"\@optional") || p.as_str().ends_with("/@optional")));
    }
}
