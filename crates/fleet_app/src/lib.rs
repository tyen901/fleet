pub mod events;
mod registry;
mod storage;

pub mod launch;
pub mod platform;

use std::path::PathBuf;
use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use fleet_index::{DesiredState, FleetIndex};
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub use crate::launch::arma3::{Arma3LaunchPlan, LaunchError};
pub use registry::{
    normalize_repo_url, registry_path, Arma3Config, LaunchMode, LaunchSettings, Profile, Registry,
};
pub use storage::RegistryStore;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    InvalidInput(String),

    #[error("no profile selected")]
    NoProfileSelected,

    #[error("{0}")]
    NotFound(String),

    #[error("sync error: {0}")]
    SyncEngine(String),

    #[error(transparent)]
    Launch(#[from] LaunchError),
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
    done_rx: Option<oneshot::Receiver<Result<(), AppError>>>,
    handle: tokio::task::JoinHandle<()>,
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
        platform::open_target::open_target(self.registry.launch.mode.clone(), &s)?;
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
        tuning: SyncTuning,
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
        tuning: SyncTuning,
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
                let remote = Arc::new(
                    fleet_remote_http::HttpRemote::new(&repo_url)
                        .map_err(|e| AppError::SyncEngine(e.to_string()))?,
                );

                let raw_spec = remote
                    .fetch_repo_spec()
                    .await
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;

                let repo_name = format!("{} (v{})", raw_spec.repo_name, raw_spec.version);

                let enabled_mods: Vec<String> = raw_spec
                    .required_mods
                    .iter()
                    .filter(|m| m.enabled)
                    .map(|m| m.mod_name.clone())
                    .collect();

                let engine_tuning = sync_engine::RepairTuning {
                    file_concurrency: tuning
                        .max_concurrent_files
                        .unwrap_or(sync_engine::RepairTuning::default().file_concurrency),
                    range_concurrency: tuning
                        .max_concurrent_range_requests
                        .unwrap_or(sync_engine::RepairTuning::default().range_concurrency),
                    scan_concurrency: sync_engine::RepairTuning::default().scan_concurrency,
                    patch_max_bad_ratio: tuning.full_download_byte_ratio_threshold as f32,
                    patch_max_bad_parts: Some(tuning.full_download_part_threshold),
                    patch_merge_gap_bytes: sync_engine::RepairTuning::default()
                        .patch_merge_gap_bytes,
                    patch_min_range_bytes: sync_engine::RepairTuning::default()
                        .patch_min_range_bytes,
                    patch_max_fetch_ratio: sync_engine::RepairTuning::default()
                        .patch_max_fetch_ratio,
                    patch_max_range_requests: sync_engine::RepairTuning::default()
                        .patch_max_range_requests,
                    durability: sync_engine::Durability::BestEffort,
                    unexpected_paths: sync_engine::UnexpectedPathPolicy::Prompt,
                    max_unexpected_delete_bytes: sync_engine::RepairTuning::default()
                        .max_unexpected_delete_bytes,
                    delete_empty_dirs: true,
                    use_index: tuning.use_index,
                    emit_progress: true,
                    auto_fix_case: sync_engine::RepairTuning::default().auto_fix_case,
                };

                let sink = SyncEventSink { tx: ev_tx.clone() };

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

                let request = sync_engine::RepairRequest {
                    repo_name,
                    checkout_root: checkout_root_buf.as_std_path().to_path_buf(),
                    enabled_mods,
                    tuning: engine_tuning,
                };

                let store = Arc::new(FleetIndexStore::new(idx));
                let engine = sync_engine::SyncEngine::new(
                    remote as Arc<dyn sync_engine::RemoteRepo>,
                    store,
                    Arc::new(Md5Checksummer),
                );
                let _outcome = engine
                    .repair(request, &sink, &cancel_task)
                    .await
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;
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
            handle,
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
        platform::open_target::open_target(self.registry.launch.mode.clone(), &plan.steam_url)?;
        Ok(())
    }

    pub fn launch_arma3_for_path(
        &self,
        base_path: &std::path::Path,
        extra_args: &str,
    ) -> Result<(), AppError> {
        let plan = launch::arma3::plan_launch(base_path, &[], extra_args)?;
        platform::open_target::open_target(self.registry.launch.mode.clone(), &plan.steam_url)?;
        Ok(())
    }
}

struct SyncEventSink {
    tx: mpsc::Sender<events::SyncEvent>,
}

struct FleetIndexStore {
    inner: std::sync::Mutex<FleetIndex>,
}

impl FleetIndexStore {
    fn new(idx: FleetIndex) -> Self {
        Self {
            inner: std::sync::Mutex::new(idx),
        }
    }
}

impl sync_engine::StateStore for FleetIndexStore {
    fn desired_state_get(
        &self,
    ) -> Result<Option<sync_engine::DesiredState>, sync_engine::StoreError> {
        let got = self
            .inner
            .lock()
            .unwrap()
            .get_desired_state()
            .map_err(|e| sync_engine::StoreError::Other(e.to_string()))?;
        Ok(got.map(|s| sync_engine::DesiredState {
            state_id: s.state_id,
            enabled_mods_hash: s.enabled_mods_hash,
        }))
    }

    fn expected_replace_all_if_digest_changed(
        &self,
        state_id: &str,
        rows: Vec<sync_engine::ExpectedFile>,
        digest_hex: &str,
    ) -> Result<(), sync_engine::StoreError> {
        let rows: Vec<fleet_index::ExpectedFile> = rows
            .into_iter()
            .map(|r| fleet_index::ExpectedFile {
                mod_id: r.mod_id,
                rel_path: r.rel_path,
                size: r.size,
            })
            .collect();
        self.inner
            .lock()
            .unwrap()
            .expected_replace_all_if_digest_changed(state_id, rows, digest_hex)
            .map(|_| ())
            .map_err(|e| sync_engine::StoreError::Other(e.to_string()))
    }

    fn baseline_exists(&self, state_id: &str) -> Result<bool, sync_engine::StoreError> {
        self.inner
            .lock()
            .unwrap()
            .baseline_exists(state_id)
            .map_err(|e| sync_engine::StoreError::Other(e.to_string()))
    }

    fn expected_get_all(
        &self,
        state_id: &str,
    ) -> Result<Vec<sync_engine::ExpectedFile>, sync_engine::StoreError> {
        let mut out = Vec::new();
        self.inner
            .lock()
            .unwrap()
            .expected_for_each(state_id, |row| {
                out.push(sync_engine::ExpectedFile {
                    mod_id: row.mod_id,
                    rel_path: row.rel_path,
                    size: row.size,
                });
                Ok(())
            })
            .map_err(|e| sync_engine::StoreError::Other(e.to_string()))?;
        Ok(out)
    }

    fn file_state_get_all_for_mod(
        &self,
        state_id: &str,
        mod_id: &str,
    ) -> Result<std::collections::HashMap<String, sync_engine::FileState>, sync_engine::StoreError>
    {
        let got = self
            .inner
            .lock()
            .unwrap()
            .file_state_get_all_for_mod(state_id, mod_id)
            .map_err(|e| sync_engine::StoreError::Other(e.to_string()))?;
        Ok(got
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    sync_engine::FileState {
                        size: v.size,
                        mtime_ns: sync_engine::TimestampNs(v.mtime_ns),
                        checksum: v.checksum,
                    },
                )
            })
            .collect())
    }

    fn file_state_apply_batch(
        &self,
        state_id: &str,
        upserts: Vec<sync_engine::FileStateUpsert>,
        deletes: Vec<sync_engine::FileStateDelete>,
    ) -> Result<(), sync_engine::StoreError> {
        let up = upserts
            .into_iter()
            .map(|u| (u.mod_id, u.rel_path, u.size, u.mtime_ns.0, u.checksum));
        let del = deletes.into_iter().map(|d| (d.mod_id, d.rel_path));
        self.inner
            .lock()
            .unwrap()
            .file_state_apply_batch(state_id, up, del)
            .map_err(|e| sync_engine::StoreError::Other(e.to_string()))
    }

    fn file_state_delete(
        &self,
        state_id: &str,
        mod_id: &str,
        rel_path: &str,
    ) -> Result<(), sync_engine::StoreError> {
        self.inner
            .lock()
            .unwrap()
            .file_state_delete(state_id, mod_id, rel_path)
            .map_err(|e| sync_engine::StoreError::Other(e.to_string()))
    }

    fn verified_get(&self) -> Result<Option<sync_engine::VerifiedState>, sync_engine::StoreError> {
        let got = self
            .inner
            .lock()
            .unwrap()
            .verified_get()
            .map_err(|e| sync_engine::StoreError::Other(e.to_string()))?;
        Ok(got.map(|v| sync_engine::VerifiedState {
            state_id: v.state_id,
            verified_at: sync_engine::TimestampNs(v.verified_at_ns),
        }))
    }

    fn verified_set(
        &self,
        state_id: &str,
        verified_at: sync_engine::TimestampNs,
    ) -> Result<(), sync_engine::StoreError> {
        self.inner
            .lock()
            .unwrap()
            .verified_set(state_id, verified_at.0)
            .map_err(|e| sync_engine::StoreError::Other(e.to_string()))
    }

    fn verified_clear(&self) -> Result<(), sync_engine::StoreError> {
        self.inner
            .lock()
            .unwrap()
            .verified_clear()
            .map_err(|e| sync_engine::StoreError::Other(e.to_string()))
    }
}

impl sync_engine::EventSink for SyncEventSink {
    fn push(&self, ev: sync_engine::SyncEvent) {
        let app_ev: events::SyncEvent = ev.into();

        // High-frequency progress can be lossy; state transitions should be reliable.
        if app_ev.is_high_frequency() {
            let _ = self.tx.try_send(app_ev);
            return;
        }

        // Prefer guaranteed delivery for important events.
        // blocking_send is acceptable here because these events are low-frequency.
        let _ = self.tx.blocking_send(app_ev);
    }
}

#[derive(Clone, Copy)]
struct Md5Checksummer;

impl sync_engine::Checksummer for Md5Checksummer {
    fn algorithm_name(&self) -> &str {
        "md5"
    }

    fn hash_file(&self, path: &std::path::Path) -> anyhow::Result<Vec<u8>> {
        use std::io::Read;

        let mut file = std::fs::File::open(path)?;
        let mut ctx = md5::Context::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            ctx.consume(&buf[..n]);
        }
        Ok(ctx.compute().0.to_vec())
    }

    fn hash_range(&self, path: &std::path::Path, offset: u64, len: u64) -> anyhow::Result<Vec<u8>> {
        use std::io::{Read, Seek};

        let mut file = std::fs::File::open(path)?;
        file.seek(std::io::SeekFrom::Start(offset))?;

        let mut remaining = len;
        let mut ctx = md5::Context::new();
        let mut buf = [0u8; 64 * 1024];
        while remaining > 0 {
            let want = usize::try_from(remaining.min(buf.len() as u64))?;
            let n = file.read(&mut buf[..want])?;
            if n == 0 {
                anyhow::bail!("unexpected EOF while hashing range");
            }
            ctx.consume(&buf[..n]);
            remaining -= n as u64;
        }
        Ok(ctx.compute().0.to_vec())
    }
}

// Backward-compat re-exports (keep for now to avoid churn in other crates)
pub use launch::arma3::{build_arma3_commandline, build_arma3_steam_url};

pub fn launch_arma3_via_steam(steam_url: String) -> Result<(), LaunchError> {
    platform::open_target::open_target(LaunchMode::SystemDefault, &steam_url)
}

pub fn launch_arma3_via_steam_with_mode(
    steam_url: String,
    mode: LaunchMode,
) -> Result<(), LaunchError> {
    platform::open_target::open_target(mode, &steam_url)
}

pub fn open_folder_in_file_manager(
    path: &std::path::Path,
    mode: LaunchMode,
) -> Result<(), LaunchError> {
    let s = path.to_string_lossy();
    platform::open_target::open_target(mode, &s)
}
