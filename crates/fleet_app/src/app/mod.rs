pub mod error;

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use camino::{Utf8Path, Utf8PathBuf};
use fleet_index::{DesiredState, FleetIndex};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::launch::arma3;
use crate::paths;
use crate::profiles;
use crate::profiles::{normalize_repo_url, Profile, ProfilesDb};
use crate::settings::{Arma3Config, LaunchSettings};
use crate::store::json_store::JsonStore;
use crate::sync;
use crate::sync::adapters::{FleetIndexStore, Md5Checksummer};
use crate::sync::{model::SyncModel, sync_model_sink::SyncModelSink};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileCreate {
    pub name: String,
    pub repo_url: String,
    pub checkout_root: String,
    pub select: bool,
    pub arma3_extra_args: String,
    pub arma3_enabled_mods: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileUpdate {
    pub name: Option<String>,
    pub repo_url: Option<String>,
    pub checkout_root: Option<String>,
    pub select: Option<bool>,
    pub arma3_extra_args: Option<String>,
    pub arma3_enabled_mods: Option<Vec<String>>,
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
    profiles_store: JsonStore<ProfilesDb>,
    profiles: ProfilesDb,
    settings_store: JsonStore<LaunchSettings>,
    settings: LaunchSettings,
}

impl FleetApp {
    pub fn open_default() -> Result<Self, AppError> {
        let profiles_path = paths::profiles_path()?;
        let settings_path = paths::settings_path()?;
        let profiles_store = JsonStore::new(profiles_path);
        let settings_store = JsonStore::new(settings_path);
        let profiles = profiles_store.load()?;
        let settings = settings_store.load()?;
        Ok(Self {
            profiles_store,
            profiles,
            settings_store,
            settings,
        })
    }

    pub fn open_default_with_recovery() -> (Self, Option<String>) {
        let mut warnings: Vec<String> = Vec::new();
        let profiles_path = match paths::profiles_path() {
            Ok(path) => path,
            Err(e) => {
                warnings.push(format!("Failed to resolve profiles path: {e}"));
                Utf8PathBuf::from("profiles.json")
            }
        };
        let settings_path = match paths::settings_path() {
            Ok(path) => path,
            Err(e) => {
                warnings.push(format!("Failed to resolve settings path: {e}"));
                Utf8PathBuf::from("settings.json")
            }
        };

        let profiles_store = JsonStore::new(profiles_path);
        let settings_store = JsonStore::new(settings_path);

        let profiles = match profiles_store.load() {
            Ok(p) => p,
            Err(e) => {
                warnings.push(e.to_string());
                ProfilesDb::default()
            }
        };
        let settings = match settings_store.load() {
            Ok(s) => s,
            Err(e) => {
                warnings.push(e.to_string());
                LaunchSettings::default()
            }
        };

        (
            Self {
                profiles_store,
                profiles,
                settings_store,
                settings,
            },
            if warnings.is_empty() {
                None
            } else {
                Some(warnings.join("\n"))
            },
        )
    }

    pub fn profiles_path(&self) -> &Utf8Path {
        self.profiles_store.path()
    }

    pub fn settings_path(&self) -> &Utf8Path {
        self.settings_store.path()
    }

    pub fn refresh_storage(&mut self) -> Result<(), AppError> {
        self.profiles = self.profiles_store.load()?;
        self.settings = self.settings_store.load()?;
        Ok(())
    }

    pub fn init_storage(&mut self) -> Result<(), AppError> {
        let profiles_store = self.profiles_store.clone();
        profiles_store.update(|_db| Ok(()))?;
        let settings_store = self.settings_store.clone();
        settings_store.update(|_settings| Ok(()))?;
        self.refresh_storage()?;
        Ok(())
    }

    pub fn launch_settings(&self) -> LaunchSettings {
        self.settings.clone()
    }

    pub fn set_launch_settings(&mut self, settings: LaunchSettings) -> Result<(), AppError> {
        // REQUIRED: reject invalid Linux templates at the settings boundary
        crate::launch::arma3::validate_linux_template_strict(&settings.arma3.linux.template)?;

        let store = self.settings_store.clone();
        store.update(|current| {
            *current = settings;
            Ok(())
        })?;
        self.refresh_storage()?;
        Ok(())
    }

    pub fn open_folder(&self, path: &std::path::Path) -> Result<(), AppError> {
        crate::platform::open_path(self.settings.open_mode.clone(), path)?;
        Ok(())
    }

    pub fn arma3_launch_preview_for_profile(
        &self,
        id: &str,
        extra_args_override: Option<String>,
    ) -> Result<String, AppError> {
        let profile = self
            .profiles
            .profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;

        let extra = extra_args_override.unwrap_or(profile.arma3.extra_args);
        let base_path = std::path::PathBuf::from(profile.checkout_root);

        let plan = arma3::plan_launch(
            &base_path,
            &profile.arma3.enabled_mods,
            &extra,
            &self.settings,
        )?;

        Ok(plan.preview)
    }

    pub fn arma3_linux_template_validation_for_profile(
        &self,
        id: &str,
        extra_args_override: Option<String>,
        settings_override: Option<LaunchSettings>,
    ) -> Result<crate::launch::arma3::LinuxTemplateValidation, AppError> {
        let profile = self
            .profiles
            .profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;

        let extra = extra_args_override.unwrap_or(profile.arma3.extra_args);
        let base_path = std::path::PathBuf::from(profile.checkout_root);

        let report = crate::launch::arma3::linux_template_preview(
            &base_path,
            &profile.arma3.enabled_mods,
            &extra,
            settings_override.as_ref().unwrap_or(&self.settings),
        )?;

        Ok(report)
    }

    pub fn list_profiles(&self) -> Vec<ProfileSpec> {
        self.profiles
            .profiles
            .clone()
            .into_iter()
            .map(ProfileSpec::from)
            .collect()
    }

    pub fn selected_profile(&self) -> Option<ProfileSpec> {
        self.profiles.selected().cloned().map(ProfileSpec::from)
    }

    pub fn get_profile(&self, id: &str) -> Option<ProfileSpec> {
        self.profiles
            .profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .map(ProfileSpec::from)
    }

    pub fn select_profile(&mut self, id: &str) -> Result<(), AppError> {
        let store = self.profiles_store.clone();
        store.update(|db| {
            if db.profiles.iter().any(|p| p.id == id) {
                db.selected_profile = Some(id.to_string());
                Ok(())
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("profile not found: {id}"),
                ))
            }
        })?;
        self.refresh_storage()?;
        Ok(())
    }

    fn require_profile(&self, profile_id: &str) -> Result<ProfileSpec, AppError> {
        self.get_profile(profile_id)
            .ok_or_else(|| AppError::NotFound(format!("profile not found: {profile_id}")))
    }

    pub fn add_profile(&mut self, create: ProfileCreate) -> Result<ProfileSpec, AppError> {
        let normalized_repo = normalize_repo_url(&create.repo_url);
        let checkout_path = Utf8PathBuf::from(&create.checkout_root);

        profiles::setup_checkout_root(&checkout_path)?;

        let created = profiles::unix_now();

        let store = self.profiles_store.clone();
        let added = store.update(|reg| {
            let prev_selected = reg.selected_profile.clone();
            let profile = Profile {
                id: String::new(),
                name: create.name.to_string(),
                repo_url: normalized_repo,
                checkout_root: checkout_path.to_string(),
                created_unix_s: created,
                last_sync_unix_s: None,
                arma3: Arma3Config {
                    extra_args: create.arma3_extra_args,
                    enabled_mods: create.arma3_enabled_mods,
                },
            };
            reg.add_profile(profile);

            let created = reg
                .profiles
                .last()
                .cloned()
                .ok_or_else(|| std::io::Error::other("profile insert failed"))?;

            if create.select {
                reg.selected_profile = Some(created.id.clone());
            } else {
                reg.selected_profile = prev_selected;
            }
            Ok(created)
        })?;

        self.refresh_storage()?;
        Ok(ProfileSpec::from(added))
    }

    pub fn update_profile(&mut self, id: &str, update: ProfileUpdate) -> Result<(), AppError> {
        let store = self.profiles_store.clone();
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
                profiles::setup_checkout_root(&checkout_path)?;
                profile.checkout_root = checkout_path.to_string();
            }
            if let Some(extra) = update.arma3_extra_args {
                profile.arma3.extra_args = extra;
            }
            if let Some(mods) = update.arma3_enabled_mods {
                profile.arma3.enabled_mods = mods;
            }
            if let Some(select) = update.select {
                if select {
                    reg.selected_profile = Some(profile.id.clone());
                }
            }
            Ok(())
        })?;
        self.refresh_storage()?;
        Ok(())
    }

    pub fn remove_profile(&mut self, id: &str) -> Result<(), AppError> {
        let store = self.profiles_store.clone();
        let removed = store.update(|reg| Ok(reg.remove(id)))?;
        if removed {
            self.refresh_storage()?;
            let data_dir = paths::profile_data_dir(id)
                .map_err(|e| AppError::Maintenance(format!("profile cleanup failed: {e}")))?;
            let cache_dir = paths::profile_cache_dir(id)
                .map_err(|e| AppError::Maintenance(format!("profile cleanup failed: {e}")))?;

            std::fs::remove_dir_all(&data_dir).or_else(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(e)
                }
            })?;
            std::fs::remove_dir_all(&cache_dir).or_else(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(e)
                }
            })?;
            Ok(())
        } else {
            Err(AppError::NotFound(format!("profile not found: {id}")))
        }
    }

    pub fn spawn_sync_selected(
        &mut self,
        handle: tokio::runtime::Handle,
        tuning: sync::SyncTuning,
        model: Arc<RwLock<SyncModel>>,
    ) -> Result<SyncJob, AppError> {
        let profile = self.selected_profile().ok_or(AppError::NoProfileSelected)?;
        let checkout_root = Utf8PathBuf::from(profile.checkout_root.clone());
        self.spawn_sync(
            &profile.repo_url,
            &checkout_root,
            handle,
            tuning,
            profile.id,
            model,
        )
    }

    pub fn spawn_sync(
        &mut self,
        repo_url: &str,
        checkout_root: &Utf8Path,
        handle: tokio::runtime::Handle,
        tuning: sync::SyncTuning,
        profile_id: String,
        model: Arc<RwLock<SyncModel>>,
    ) -> Result<SyncJob, AppError> {
        let repo_url = normalize_repo_url(repo_url);
        profiles::setup_checkout_root(checkout_root)?;

        let (done_tx, done_rx) = oneshot::channel::<Result<(), AppError>>();
        let checkout_root_buf = checkout_root.to_owned();
        let profiles_store = self.profiles_store.clone();
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

                {
                    let mut m = model.write().unwrap_or_else(|e| e.into_inner());
                    *m = SyncModel::new();
                    m.phase = "Starting…".to_string();
                }

                let sink = SyncModelSink::new(Arc::clone(&model));

                let repo_id = fleet_index::normalize_repo_id(&raw_spec.checksum);
                let repo_revision = format!("{}|{}", raw_spec.version, raw_spec.checksum);
                let mut enabled_sorted = enabled_mods.clone();
                enabled_sorted.sort();
                let enabled_hash = fleet_index::enabled_mods_hash(&enabled_sorted);
                let state_id = fleet_index::state_id(&repo_id, &enabled_hash, &repo_revision);

                let idx_path = paths::profile_index_path(&profile_id)
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;
                let lock_path = paths::profile_index_lock_path(&profile_id)
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;
                let lock_file = std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .read(true)
                    .write(true)
                    .open(lock_path)
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;
                lock_file
                    .lock_exclusive()
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;
                let mut idx = FleetIndex::open_or_recover_at_path(&idx_path)
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;
                let desired = DesiredState {
                    repo_url: repo_url.clone(),
                    repo_id,
                    repo_revision,
                    enabled_mods_hash: enabled_hash,
                    state_id,
                    updated_at_unix_s: profiles::unix_now(),
                };
                idx.set_desired_state(desired.clone())
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;
                let _ = idx.gc_not_state(&desired.state_id);

                let store = Arc::new(FleetIndexStore::new(lock_file, idx));
                let engine =
                    fleet_sync::SyncEngine::new(gated_remote, store, Arc::new(Md5Checksummer));

                let staging_root = paths::profile_staging_dir(&profile_id)
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;
                std::fs::create_dir_all(&staging_root)
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;

                let outcome = match tuning.mode {
                    sync::SyncMode::Repair => {
                        let request = fleet_sync::RepairRequest {
                            repo_name,
                            checkout_root: checkout_root_buf.as_std_path().to_path_buf(),
                            staging_root: staging_root.clone(),
                            enabled_mods,
                            tuning: tuning.to_repair_tuning(),
                        };

                        let outcome = engine.repair(request, &sink, &cancel_task).await;
                        let outcome = match outcome {
                            Ok(o) => o,
                            Err(e) => {
                                let mut m = model.write().unwrap_or_else(|e| e.into_inner());
                                m.error = Some(e.to_string());
                                m.finished = true;
                                return Err(AppError::SyncEngine(e.to_string()));
                            }
                        };
                        sync::SyncOutcome::from_repair(outcome)
                    }

                    sync::SyncMode::SyncFresh => {
                        let request = fleet_sync::SyncFreshRequest {
                            repo_name,
                            checkout_root: checkout_root_buf.as_std_path().to_path_buf(),
                            staging_root: staging_root.clone(),
                            enabled_mods,
                            tuning: tuning.to_sync_fresh_tuning(),
                        };

                        let outcome = engine.sync_fresh(request, &sink, &cancel_task).await;
                        let outcome = match outcome {
                            Ok(o) => o,
                            Err(e) => {
                                let mut m = model.write().unwrap_or_else(|e| e.into_inner());
                                m.error = Some(e.to_string());
                                m.finished = true;
                                return Err(AppError::SyncEngine(e.to_string()));
                            }
                        };
                        sync::SyncOutcome::from_sync_fresh(outcome)
                    }

                    sync::SyncMode::Check | sync::SyncMode::Verify => {
                        let request = fleet_sync::CheckRequest {
                            repo_name,
                            checkout_root: checkout_root_buf.as_std_path().to_path_buf(),
                            enabled_mods,
                            tuning: if tuning.mode == sync::SyncMode::Verify {
                                tuning.to_verify_tuning()
                            } else {
                                tuning.to_check_tuning()
                            },
                        };

                        let report = engine.check(request, &sink, &cancel_task).await;
                        let report = match report {
                            Ok(r) => r,
                            Err(e) => {
                                let mut m = model.write().unwrap_or_else(|e| e.into_inner());
                                m.error = Some(e.to_string());
                                m.finished = true;
                                return Err(AppError::SyncEngine(e.to_string()));
                            }
                        };
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
                // Update last_sync_unix_s via locked update to avoid clobbering.
                let _ = profiles_store.update(|db| {
                    if let Some(profile) = db.profiles.iter_mut().find(|p| p.id == profile_id) {
                        profile.last_sync_unix_s = Some(profiles::unix_now());
                    }
                    Ok(())
                });
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
            .profiles
            .profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;

        let extra = extra_args_override.unwrap_or(profile.arma3.extra_args);
        let base_path = PathBuf::from(profile.checkout_root);

        let plan = arma3::plan_launch(
            &base_path,
            &profile.arma3.enabled_mods,
            &extra,
            &self.settings,
        )?;

        crate::platform::execute(self.settings.open_mode.clone(), plan.action)?;
        Ok(())
    }

    pub fn launch_arma3_for_path(
        &self,
        base_path: &std::path::Path,
        extra_args: &str,
    ) -> Result<(), AppError> {
        let plan = arma3::plan_launch(base_path, &[], extra_args, &self.settings)?;

        crate::platform::execute(self.settings.open_mode.clone(), plan.action)?;
        Ok(())
    }

    pub fn clear_index(&self, profile_id: &str) -> Result<(), AppError> {
        let _profile = self.require_profile(profile_id)?;
        let idx_path = paths::profile_index_path(profile_id)
            .map_err(|e| AppError::Maintenance(format!("clear index failed: {e}")))?;
        for path in [
            idx_path.clone(),
            idx_path.with_extension("sqlite-wal"),
            idx_path.with_extension("sqlite-shm"),
        ] {
            std::fs::remove_file(&path).or_else(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(e)
                }
            })?;
        }
        Ok(())
    }

    pub fn clear_cache(&self, profile_id: &str) -> Result<(), AppError> {
        let _profile = self.require_profile(profile_id)?;
        let path = paths::profile_staging_dir(profile_id)
            .map_err(|e| AppError::Maintenance(format!("clear cache failed: {e}")))?;
        std::fs::remove_dir_all(&path)
            .or_else(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(e)
                }
            })
            .map_err(|e| AppError::Maintenance(format!("clear cache failed: {e}")))?;
        Ok(())
    }
}
