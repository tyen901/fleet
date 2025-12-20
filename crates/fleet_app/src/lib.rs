mod arma3;
mod registry;

use std::fmt;
use std::path::PathBuf;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};

pub use arma3::{build_arma3_steam_url, launch_arma3_via_steam, LaunchError};
pub use registry::{normalize_repo_url, registry_path, Arma3Config, Profile, Registry};

#[derive(Debug)]
pub enum AppError {
    Io(std::io::Error),
    InvalidInput(String),
    NoProfileSelected,
    NotFound(String),
    Coordinator(coordinator::CoordinatorError),
    Launch(LaunchError),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Io(err) => write!(f, "io error: {err}"),
            AppError::InvalidInput(msg) => write!(f, "{msg}"),
            AppError::NoProfileSelected => write!(f, "no profile selected"),
            AppError::NotFound(msg) => write!(f, "{msg}"),
            AppError::Coordinator(err) => write!(f, "{err}"),
            AppError::Launch(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Io(err)
    }
}

impl From<coordinator::CoordinatorError> for AppError {
    fn from(err: coordinator::CoordinatorError) -> Self {
        AppError::Coordinator(err)
    }
}

impl From<LaunchError> for AppError {
    fn from(err: LaunchError) -> Self {
        AppError::Launch(err)
    }
}

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

#[derive(Debug, Clone)]
pub struct SyncTuning {
    pub full_download_part_threshold: usize,
    pub full_download_byte_ratio_threshold: f64,
    pub max_concurrent_files: Option<usize>,
    pub max_concurrent_range_requests: Option<usize>,
    pub io_buffer_bytes: usize,
    pub use_index: bool,
}

impl Default for SyncTuning {
    fn default() -> Self {
        Self {
            full_download_part_threshold: 256,
            full_download_byte_ratio_threshold: 0.60,
            max_concurrent_files: None,
            max_concurrent_range_requests: None,
            io_buffer_bytes: 1024 * 1024,
            use_index: true,
        }
    }
}

pub struct SyncJob {
    done_rx: Option<oneshot::Receiver<Result<(), coordinator::CoordinatorError>>>,
    handle: tokio::task::JoinHandle<()>,
}

impl SyncJob {
    pub fn cancel(&self) {
        self.handle.abort();
    }

    pub fn take_done_rx(
        &mut self,
    ) -> Option<oneshot::Receiver<Result<(), coordinator::CoordinatorError>>> {
        self.done_rx.take()
    }
}

#[derive(Clone)]
pub struct FleetApp {
    path: Utf8PathBuf,
    registry: Registry,
}

impl FleetApp {
    pub fn open_default() -> Result<Self, AppError> {
        let path = registry_path()?;
        let registry = registry::load_registry(&path)?;
        Ok(Self { path, registry })
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

        let registry = match registry::load_registry(&path) {
            Ok(reg) => reg,
            Err(e) => {
                warning = Some(e.to_string());
                Registry::default()
            }
        };

        (Self { path, registry }, warning)
    }

    pub fn registry_path(&self) -> &Utf8Path {
        &self.path
    }

    pub fn refresh_registry(&mut self) -> Result<(), AppError> {
        self.registry = registry::load_registry(&self.path)?;
        Ok(())
    }

    pub fn init_registry(&mut self) -> Result<(), AppError> {
        if self.path.exists() {
            return Ok(());
        }
        registry::save_registry_atomic(&self.path, &Registry::default())?;
        Ok(())
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
        if self.registry.profiles.iter().any(|p| p.id == id) {
            self.registry.selected_profile = Some(id.to_string());
            registry::save_registry_atomic(&self.path, &self.registry)?;
            Ok(())
        } else {
            Err(AppError::NotFound(format!("profile not found: {id}")))
        }
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

        let prev_selected = self.registry.selected_profile.clone();
        let profile = Profile {
            id: String::new(),
            name: name.to_string(),
            repo_url: normalized_repo,
            checkout_root: checkout_path.to_string(),
            created_unix_s: registry::unix_now(),
            last_sync_unix_s: None,
            arma3: Arma3Config::default(),
        };

        self.registry.add_profile(profile);
        if !select {
            self.registry.selected_profile = prev_selected;
        }
        registry::save_registry_atomic(&self.path, &self.registry)?;

        let added = self.registry.profiles.last().cloned().unwrap();
        Ok(ProfileSpec::from(added))
    }

    pub fn update_profile(&mut self, id: &str, update: ProfileUpdate) -> Result<(), AppError> {
        let profile = self
            .registry
            .profiles
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| AppError::NotFound(format!("profile not found: {id}")))?;

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
                self.registry.selected_profile = Some(profile.id.clone());
            }
        }

        registry::save_registry_atomic(&self.path, &self.registry)?;
        Ok(())
    }

    pub fn remove_profile(&mut self, id: &str) -> Result<(), AppError> {
        if self.registry.remove_profile(id) {
            registry::save_registry_atomic(&self.path, &self.registry)?;
            Ok(())
        } else {
            Err(AppError::NotFound(format!("profile not found: {id}")))
        }
    }

    pub fn spawn_sync_selected(
        &mut self,
        handle: tokio::runtime::Handle,
        tuning: SyncTuning,
        ev_tx: mpsc::Sender<coordinator::events::Event>,
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
        tuning: SyncTuning,
        profile_id_to_update: Option<String>,
        ev_tx: mpsc::Sender<coordinator::events::Event>,
    ) -> Result<SyncJob, AppError> {
        let repo_url = normalize_repo_url(repo_url);
        registry::setup_checkout_root(checkout_root)?;

        let apply = apply_options_from_tuning(&tuning);
        let (done_tx, done_rx) = oneshot::channel();
        let checkout_root_buf = checkout_root.to_owned();
        let path = self.path.clone();

        let handle = handle.spawn(async move {
            let res = coordinator::sync_checkout_with_events(
                &repo_url,
                &checkout_root_buf,
                coordinator::SyncOptions {
                    apply,
                    ..coordinator::SyncOptions::default()
                },
                Some(ev_tx),
            )
            .await;

            if res.is_ok() {
                if let Some(profile_id) = profile_id_to_update {
                    let _ = update_last_sync_on_disk(&path, &profile_id);
                }
            }

            let _ = done_tx.send(res);
        });

        Ok(SyncJob {
            done_rx: Some(done_rx),
            handle,
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
        let url = build_arma3_steam_url(&base_path, &profile.arma3.enabled_mods, &extra)?;
        launch_arma3_via_steam(url)?;
        Ok(())
    }

    pub fn launch_arma3_for_path(
        &self,
        base_path: &std::path::Path,
        extra_args: &str,
    ) -> Result<(), AppError> {
        let url = build_arma3_steam_url(base_path, &[], extra_args)?;
        launch_arma3_via_steam(url)?;
        Ok(())
    }
}

fn apply_options_from_tuning(tuning: &SyncTuning) -> sync_apply::ApplyOptions {
    let mut apply = sync_apply::ApplyOptions {
        full_download_part_threshold: tuning.full_download_part_threshold,
        full_download_byte_ratio_threshold: tuning
            .full_download_byte_ratio_threshold
            .clamp(0.0, 1.0),
        io_buffer_bytes: tuning.io_buffer_bytes.max(64 * 1024),
        ..sync_apply::ApplyOptions::default()
    };

    if let Some(v) = tuning.max_concurrent_files {
        apply.max_concurrent_files = v.max(1);
    }
    if let Some(v) = tuning.max_concurrent_range_requests {
        apply.max_concurrent_range_requests = v.max(1);
    }

    apply
}

fn update_last_sync_on_disk(path: &Utf8Path, profile_id: &str) -> Result<(), AppError> {
    let mut reg = registry::load_registry(path)?;
    if let Some(profile) = reg.profiles.iter_mut().find(|p| p.id == profile_id) {
        profile.last_sync_unix_s = Some(registry::unix_now());
        registry::save_registry_atomic(path, &reg)?;
    }
    Ok(())
}
