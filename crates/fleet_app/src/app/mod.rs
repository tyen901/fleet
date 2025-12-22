pub mod error;

use std::path::PathBuf;
use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use fleet_index::{DesiredState, FleetIndex};
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::events;
use crate::launch;
use crate::registry;
use crate::registry::{normalize_repo_url, registry_path, Profile, Registry};
use crate::settings::{Arma3Config, LaunchSettings};
use crate::storage::RegistryStore;
use crate::sync;
use crate::sync::adapters::{FleetIndexStore, Md5Checksummer, SyncEventSink};

pub use error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct ProfileSpec {
    pub id: String,
    pub name: String,
    pub repo_url: String,
    pub checkout_root: String,
    pub created_unix_s: i64,
    pub last_sync_unix_s: Option<i64>,
    pub arma3: Arma3Config,
}

impl From<Profile> for ProfileSpec {
    fn from(p: Profile) -> Self {
        Self {
            id: p.id,
            name: p.name,
            repo_url: p.repo_url,
            checkout_root: p.checkout_root,
            created_unix_s: p.created_unix_s,
            last_sync_unix_s: p.last_sync_unix_s,
            arma3: p.arma3,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProfileUpdate {
    pub name: Option<String>,
    pub repo_url: Option<String>,
    pub checkout_root: Option<String>,
    pub select: Option<bool>,
    pub arma3_extra_args: Option<String>,
}

pub struct SyncJob {
    done_rx: Option<oneshot::Receiver<Result<(), AppError>>>,
    _handle: tokio::task::JoinHandle<()>,
    cancel: CancellationToken,
}

impl SyncJob {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn take_done_rx(&mut self) -> Option<oneshot::Receiver<Result<(), AppError>>> {
        self.done_rx.take()
    }
}

#[derive(Clone)]
pub struct FleetApp {
    store: RegistryStore,
    registry: Registry,
}

impl FleetApp {
    pub fn open_default() -> Result<Self, AppError> {
        let path = registry_path()?;
        let store = RegistryStore::new(path);
        let registry = store.load()?;
        Ok(Self { store, registry })
    }

    pub fn open_default_with_recovery() -> (Self, Option<String>) {
        let mut warning = None;
        let path = match registry_path() {
            Ok(path) => path,
            Err(e) => {
                warning = Some(format!("Failed to resolve registry path: {e}"));
                Utf8PathBuf::from("registry.json")
            }
        };

        let store = RegistryStore::new(path);
        let registry = match store.load() {
            Ok(reg) => reg,
            Err(e) => {
                warning = Some(e.to_string());
                Registry::default()
            }
        };

        (Self { store, registry }, warning)
    }

    pub fn registry_path(&self) -> &Utf8Path {
        self.store.path()
    }

    pub fn refresh_registry(&mut self) -> Result<(), AppError> {
        self.registry = self.store.load()?;
        Ok(())
    }

    pub fn init_registry(&mut self) -> Result<(), AppError> {
        // Create file if missing; save default under lock.
        let store = self.store.clone();
        store.update(|reg| {
            // If this is a fresh file, it will be default already; still safe.
            if reg.schema_version == 0 {
                *reg = Registry::default();
            }
            Ok(())
        })?;
        self.refresh_registry()?;
        Ok(())
    }

    pub fn launch_settings(&self) -> LaunchSettings {
        self.registry.launch.clone()
    }

    pub fn set_launch_settings(&mut self, settings: LaunchSettings) -> Result<(), AppError> {
        let store = self.store.clone();
        store.update(|reg| {
            reg.launch = settings;
            Ok(())
        })?;
        self.refresh_registry()?;
        Ok(())
    }

    pub fn open_folder(&self, path: &std::path::Path) -> Result<(), AppError> {
        let s = path.to_string_lossy();
        crate::platform::open_target::open_target(self.registry.launch.mode.clone(), &s)?;
        Ok(())
    }

    pub fn arma3_launch_args_for_profile(
        &self,
        id: &str,
        extra_args_override: Option<String>,
    ) -> Result<String, AppError> {
        let profile = self
            .registry
            .profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;

        let extra = extra_args_override.unwrap_or(profile.arma3.extra_args);
        let base_path = std::path::PathBuf::from(profile.checkout_root);
        let (cmdline, _mods) = launch::arma3::build_arma3_commandline(
            &base_path,
            &profile.arma3.enabled_mods,
            &extra,
        )?;

        #[cfg(target_os = "linux")]
        {
            Ok(format!("steam steam://rungameid/107410 {cmdline}"))
        }

        #[cfg(not(target_os = "linux"))]
        {
            Ok(cmdline)
        }
    }

    pub fn list_profiles(&self) -> Vec<ProfileSpec> {
        self.registry
            .profiles
            .clone()
            .into_iter()
            .map(ProfileSpec::from)
            .collect()
    }

    pub fn selected_profile(&self) -> Option<ProfileSpec> {
        self.registry.selected().cloned().map(ProfileSpec::from)
    }

    pub fn get_profile(&self, id: &str) -> Option<ProfileSpec> {
        self.registry
            .profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .map(ProfileSpec::from)
    }

    pub fn select_profile(&mut self, id: &str) -> Result<(), AppError> {
        let store = self.store.clone();
        store.update(|reg| {
            if reg.profiles.iter().any(|p| p.id == id) {
                reg.selected_profile = Some(id.to_string());
                Ok(())
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("profile not found: {id}"),
                ))
            }
        })?;
        self.refresh_registry()?;
        Ok(())
    }

    pub fn add_profile(
        &mut self,
        name: &str,
        repo_url: &str,
        checkout_root: &str,
        select: bool,
    ) -> Result<ProfileSpec, AppError> {
        let normalized_repo = normalize_repo_url(repo_url);
        let checkout_path = Utf8PathBuf::from(checkout_root);

        registry::setup_checkout_root(&checkout_path)?;

        let created = registry::unix_now();

        let store = self.store.clone();
        let added = store.update(|reg| {
            let prev_selected = reg.selected_profile.clone();
            let profile = Profile {
                id: String::new(),
                name: name.to_string(),
                repo_url: normalized_repo,
                checkout_root: checkout_path.to_string(),
                created_unix_s: created,
                last_sync_unix_s: None,
                arma3: Arma3Config::default(),
            };
            reg.add_profile(profile);
            if !select {
                reg.selected_profile = prev_selected;
            }
            Ok(reg.profiles.last().cloned().unwrap())
        })?;

        self.refresh_registry()?;
        Ok(ProfileSpec::from(added))
    }

    pub fn update_profile(&mut self, id: &str, update: ProfileUpdate) -> Result<(), AppError> {
        let store = self.store.clone();
        store.update(|reg| {
            let profile = reg
                .profiles
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("profile not found: {id}"),
                    )
                })?;

            if let Some(name) = update.name {
                profile.name = name;
            }
            if let Some(repo_url) = update.repo_url {
                profile.repo_url = normalize_repo_url(&repo_url);
            }
            if let Some(checkout_root) = update.checkout_root {
                let checkout_path = Utf8PathBuf::from(checkout_root);
                registry::setup_checkout_root(&checkout_path)?;
                profile.checkout_root = checkout_path.to_string();
            }
            if let Some(extra) = update.arma3_extra_args {
                profile.arma3.extra_args = extra;
            }
            if let Some(select) = update.select {
                if select {
                    reg.selected_profile = Some(profile.id.clone());
                }
            }
            Ok(())
        })?;
        self.refresh_registry()?;
        Ok(())
    }

    pub fn remove_profile(&mut self, id: &str) -> Result<(), AppError> {
        let store = self.store.clone();
        let removed = store.update(|reg| Ok(reg.remove_profile(id)))?;
        if removed {
            self.refresh_registry()?;
            Ok(())
        } else {
            Err(AppError::NotFound(format!("profile not found: {id}")))
        }
    }

    pub fn spawn_sync_selected(
        &mut self,
        handle: tokio::runtime::Handle,
        tuning: sync::SyncTuning,
        ev_tx: mpsc::Sender<events::SyncEvent>,
    ) -> Result<SyncJob, AppError> {
        let profile = self.selected_profile().ok_or(AppError::NoProfileSelected)?;
        let checkout_root = Utf8PathBuf::from(profile.checkout_root.clone());
        self.spawn_sync(
            &profile.repo_url,
            &checkout_root,
            handle,
            tuning,
            Some(profile.id),
            ev_tx,
        )
    }

    pub fn spawn_sync(
        &mut self,
        repo_url: &str,
        checkout_root: &Utf8Path,
        handle: tokio::runtime::Handle,
        tuning: sync::SyncTuning,
        profile_id_to_update: Option<String>,
        ev_tx: mpsc::Sender<events::SyncEvent>,
    ) -> Result<SyncJob, AppError> {
        let repo_url = normalize_repo_url(repo_url);
        registry::setup_checkout_root(checkout_root)?;

        let (done_tx, done_rx) = oneshot::channel::<Result<(), AppError>>();
        let checkout_root_buf = checkout_root.to_owned();
        let store = self.store.clone();
        let cancel = CancellationToken::new();
        let cancel_task = cancel.clone();

        let handle = handle.spawn(async move {
            let res: Result<(), AppError> = async {
                let remote: Arc<fleet_remote_http::HttpRemote> = Arc::new(
                    fleet_remote_http::HttpRemote::new(&repo_url)
                        .map_err(|e| AppError::SyncEngine(e.to_string()))?,
                );
                let raw_spec = remote
                    .fetch_repo_spec()
                    .await
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;

                let gated_remote: Arc<dyn fleet_sync::ports::RemoteRepo> =
                    Arc::new(sync::GatedRemote {
                        inner: remote,
                        enable_patch_repair: tuning.enable_patch_repair,
                    });

                let repo_name = format!("{} (v{})", raw_spec.repo_name, raw_spec.version);

                let enabled_mods: Vec<String> = raw_spec
                    .required_mods
                    .iter()
                    .filter(|m| m.enabled)
                    .map(|m| m.mod_name.clone())
                    .collect();

                let sink = SyncEventSink::new(ev_tx.clone());

                let repo_id = fleet_index::normalize_repo_id(&raw_spec.checksum);
                let repo_revision = format!("{}|{}", raw_spec.version, raw_spec.checksum);
                let mut enabled_sorted = enabled_mods.clone();
                enabled_sorted.sort();
                let enabled_hash = fleet_index::enabled_mods_hash(&enabled_sorted);
                let state_id = fleet_index::state_id(&repo_id, &enabled_hash, &repo_revision);

                let fleet_dir = checkout_root_buf.as_std_path().join(".fleet");
                std::fs::create_dir_all(&fleet_dir)
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;
                let idx_path = fleet_dir.join("index.sqlite");
                let mut idx = FleetIndex::open_or_recover_at_path(&idx_path)
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;
                let desired = DesiredState {
                    repo_url: repo_url.clone(),
                    repo_id,
                    repo_revision,
                    enabled_mods_hash: enabled_hash,
                    state_id,
                    updated_at_unix_s: registry::unix_now(),
                };
                idx.set_desired_state(desired)
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;

                let store = Arc::new(FleetIndexStore::new(idx));
                let engine =
                    fleet_sync::SyncEngine::new(gated_remote, store, Arc::new(Md5Checksummer));

                let outcome = match tuning.mode {
                    sync::SyncMode::Repair => {
                        let request = fleet_sync::RepairRequest {
                            repo_name,
                            checkout_root: checkout_root_buf.as_std_path().to_path_buf(),
                            enabled_mods,
                            tuning: tuning.to_repair_tuning(),
                        };

                        let outcome = engine
                            .repair(request, &sink, &cancel_task)
                            .await
                            .map_err(|e| AppError::SyncEngine(e.to_string()))?;
                        sync::SyncOutcome::from_repair(outcome)
                    }

                    sync::SyncMode::SyncFresh => {
                        let request = fleet_sync::SyncFreshRequest {
                            repo_name,
                            checkout_root: checkout_root_buf.as_std_path().to_path_buf(),
                            enabled_mods,
                            tuning: tuning.to_sync_fresh_tuning(),
                        };

                        let outcome = engine
                            .sync_fresh(request, &sink, &cancel_task)
                            .await
                            .map_err(|e| AppError::SyncEngine(e.to_string()))?;
                        sync::SyncOutcome::from_sync_fresh(outcome)
                    }

                    sync::SyncMode::Check => {
                        let request = fleet_sync::CheckRequest {
                            repo_name,
                            checkout_root: checkout_root_buf.as_std_path().to_path_buf(),
                            enabled_mods,
                            tuning: tuning.to_check_tuning(),
                        };

                        let report = engine
                            .check(request, &sink, &cancel_task)
                            .await
                            .map_err(|e| AppError::SyncEngine(e.to_string()))?;
                        sync::SyncOutcome::from_check_report(report)
                    }
                };

                if !outcome.ok {
                    return Err(AppError::SyncFailed(outcome));
                }
                Ok(())
            }
            .await;

            if res.is_ok() {
                if let Some(profile_id) = profile_id_to_update {
                    // Update last_sync_unix_s via locked update to avoid clobbering.
                    let _ = store.update(|reg| {
                        if let Some(profile) = reg.profiles.iter_mut().find(|p| p.id == profile_id)
                        {
                            profile.last_sync_unix_s = Some(registry::unix_now());
                        }
                        Ok(())
                    });
                }
            }

            let _ = done_tx.send(res);
        });

        Ok(SyncJob {
            done_rx: Some(done_rx),
            _handle: handle,
            cancel,
        })
    }

    pub fn launch_arma3_for_profile(
        &self,
        id: &str,
        extra_args_override: Option<String>,
    ) -> Result<(), AppError> {
        let profile = self
            .registry
            .profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;

        let extra = extra_args_override.unwrap_or(profile.arma3.extra_args);
        let base_path = PathBuf::from(profile.checkout_root);

        let plan = launch::arma3::plan_launch(&base_path, &profile.arma3.enabled_mods, &extra)?;
        crate::platform::open_target::open_target(
            self.registry.launch.mode.clone(),
            &plan.steam_url,
        )?;
        Ok(())
    }

    pub fn launch_arma3_for_path(
        &self,
        base_path: &std::path::Path,
        extra_args: &str,
    ) -> Result<(), AppError> {
        let plan = launch::arma3::plan_launch(base_path, &[], extra_args)?;
        crate::platform::open_target::open_target(
            self.registry.launch.mode.clone(),
            &plan.steam_url,
        )?;
        Ok(())
    }
}
