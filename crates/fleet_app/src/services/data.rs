//! Data service and model for Fleet.
//!
//! This service is presenter-facing (UI + CLI). It exposes presentation-ready
//! state via a pull-based snapshot, and exposes intent methods that mutate the
//! authoritative backend immediately.
//!
//! IMPORTANT: this crate must not depend on egui (or any UI framework). UI-only
//! capabilities (clipboard, dialogs, etc.) are owned by the UI crate.

use std::path::Path;
use std::sync::{Arc, RwLock};

use serde::Serialize;

use crate::app::{AppError, FleetApp, ProfileCreate, ProfileSpec, ProfileUpdate};
use crate::settings::LaunchSettings;

/// User-facing application settings.
///
/// Fleet persists launch settings in `settings.json`. This typedef exists so that
/// presenters do not depend directly on internal storage structures.
pub type AppSettings = LaunchSettings;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataModel {
    pub warning: Option<String>,
    pub profiles: Vec<ProfileSpec>,
    pub selected_id: Option<String>,
    pub settings: AppSettings,

    // Cached outputs for UI/CLI convenience (optional).
    pub launch_args_preview: Option<String>,
    pub launch_args_error: Option<String>,

    pub repo_spec: Option<fleet_types::RepoSpec>,
    pub repo_spec_error: Option<String>,
    pub repo_spec_generation: u64,

    pub linux_validation: Option<crate::launch::arma3::LinuxTemplateValidation>,
    pub linux_validation_error: Option<String>,

    pub last_sync_outcome: Option<crate::sync::SyncOutcome>,
}

/// Data service trait (port) consumed by UI and CLI.
pub trait DataService: Send + Sync {
    /// Obtain a cheap, immutable snapshot of the current data model.
    fn snapshot(&self) -> Arc<DataModel>;

    /// Refresh the profile list from disk.
    fn refresh_profiles(&self) -> Result<(), AppError>;

    /// Mark the given profile as selected.
    fn select_profile(&self, id: &str) -> Result<(), AppError>;

    /// Create a new profile. Returns the created profile id.
    fn create_profile(&self, create: ProfileCreate) -> Result<String, AppError>;

    /// Update an existing profile.
    fn update_profile(&self, id: &str, update: ProfileUpdate) -> Result<(), AppError>;

    /// Delete a profile.
    fn delete_profile(&self, id: &str) -> Result<(), AppError>;

    /// Persist application settings.
    fn set_settings(&self, settings: AppSettings) -> Result<(), AppError>;

    /// Reset application settings to their defaults.
    fn reset_settings_to_defaults(&self) -> Result<(), AppError>;

    /// Open a folder in the operating system.
    fn open_folder(&self, path: &Path) -> Result<(), AppError>;

    /// Open the checkout root for the given profile.
    fn open_checkout_root(&self, profile_id: &str) -> Result<(), AppError>;

    /// Launch Arma 3 for the given profile.
    fn launch_arma3_for_profile(&self, profile_id: &str) -> Result<(), AppError>;

    /// Launch Arma 3 for an explicit path (for CLI parity).
    fn launch_arma3_for_path(&self, base_path: &Path, extra_args: &str) -> Result<(), AppError>;

    /// Compute launch args preview for the given profile (pure; no UI types).
    fn launch_args_preview(&self, profile_id: &str) -> Result<String, AppError>;

    /// Request that the service compute and cache a preview (and error if any)
    /// into the model.
    fn request_launch_args_preview(&self, profile_id: &str);

    /// Request that the service fetch and cache the repo spec for the profile.
    fn request_repo_spec(&self, profile_id: &str);

    /// Request that the service fetch and cache the repo spec for a given URL.
    fn request_repo_spec_for_url(&self, repo_url: &str);

    /// Request that the service validate and cache Linux template for the profile.
    fn request_linux_validation(&self, profile_id: &str);

    /// Request that the service validate and cache Linux template for the profile with overridden settings.
    fn request_linux_validation_with_settings(&self, profile_id: &str, settings: AppSettings);

    /// Set the last sync outcome manually (e.g. from the sync job).
    fn set_last_sync_outcome(&self, outcome: Option<crate::sync::SyncOutcome>);

    /// Rebuild the index for the given profile.
    fn rebuild_index(&self, profile_id: &str) -> Result<(), AppError>;

    /// Clear the cache folder for the given profile.
    fn clear_cache(&self, profile_id: &str) -> Result<(), AppError>;

    /// Clear the last recorded sync outcome.
    fn clear_last_sync_outcome(&self);

    /// Initialize profile/settings storage files (creating if necessary) and reload services.
    fn init_storage(&self) -> Result<(), AppError>;

    /// Return the resolved profiles.json path for diagnostics and scripts.
    fn profiles_path(&self) -> Result<String, AppError>;

    /// Return the resolved settings.json path for diagnostics and scripts.
    fn settings_path(&self) -> Result<String, AppError>;
}

/// Concrete data service implementation backed by a [`FleetApp`].
pub struct FleetDataService {
    app: Arc<RwLock<FleetApp>>,
    model: Arc<RwLock<Arc<DataModel>>>,
}

impl FleetDataService {
    pub fn new(app: Arc<RwLock<FleetApp>>, warning: Option<String>) -> Arc<Self> {
        let settings = {
            let app = app.read().expect("lock poisoned");
            app.launch_settings()
        };

        let (profiles, selected_id) = {
            let app = app.read().expect("lock poisoned");
            (
                app.list_profiles(),
                app.selected_profile().map(|p| p.id.clone()),
            )
        };

        let model = DataModel {
            warning,
            profiles,
            selected_id,
            settings,
            launch_args_preview: None,
            launch_args_error: None,
            repo_spec: None,
            repo_spec_error: None,
            repo_spec_generation: 0,
            linux_validation: None,
            linux_validation_error: None,
            last_sync_outcome: None,
        };

        Arc::new(Self {
            app,
            model: Arc::new(RwLock::new(Arc::new(model))),
        })
    }

    fn refresh_cached_profiles(&self, clear_preview: bool) -> Result<(), AppError> {
        let mut app = self.app.write().expect("lock poisoned");
        app.refresh_storage()?;

        let profiles = app.list_profiles();
        let selected = app.selected_profile().map(|p| p.id.clone());

        with_model_mut(&self.model, |model| {
            model.profiles = profiles;
            model.selected_id = selected;
            if clear_preview {
                model.launch_args_preview = None;
                model.launch_args_error = None;
                model.repo_spec = None;
                model.repo_spec_error = None;
                model.repo_spec_generation = model.repo_spec_generation.wrapping_add(1);
                model.linux_validation = None;
                model.linux_validation_error = None;
                model.last_sync_outcome = None;
            }
        });

        Ok(())
    }
}

impl DataService for FleetDataService {
    fn snapshot(&self) -> Arc<DataModel> {
        self.model.read().expect("lock poisoned").clone()
    }

    fn refresh_profiles(&self) -> Result<(), AppError> {
        self.refresh_cached_profiles(false)
    }

    fn select_profile(&self, id: &str) -> Result<(), AppError> {
        {
            let mut app = self.app.write().expect("lock poisoned");
            app.select_profile(id)?;
        }
        self.refresh_cached_profiles(true)
    }

    fn create_profile(&self, create: ProfileCreate) -> Result<String, AppError> {
        let spec = {
            let mut app = self.app.write().expect("lock poisoned");
            app.add_profile(create)?
        };

        self.refresh_cached_profiles(false)?;
        Ok(spec.id)
    }

    fn update_profile(&self, id: &str, update: ProfileUpdate) -> Result<(), AppError> {
        {
            let mut app = self.app.write().expect("lock poisoned");
            app.update_profile(id, update)?;
        }
        self.refresh_cached_profiles(false)
    }

    fn delete_profile(&self, id: &str) -> Result<(), AppError> {
        {
            let mut app = self.app.write().expect("lock poisoned");
            app.remove_profile(id)?;
        }
        self.refresh_cached_profiles(false)
    }

    fn set_settings(&self, settings: AppSettings) -> Result<(), AppError> {
        {
            let mut app = self.app.write().expect("lock poisoned");
            app.set_launch_settings(settings.clone())?;
        }
        with_model_mut(&self.model, |model| {
            model.settings = settings;
        });
        Ok(())
    }

    fn reset_settings_to_defaults(&self) -> Result<(), AppError> {
        self.set_settings(LaunchSettings::default())
    }

    fn open_folder(&self, path: &Path) -> Result<(), AppError> {
        let app = self.app.read().expect("lock poisoned");
        app.open_folder(path)
    }

    fn open_checkout_root(&self, profile_id: &str) -> Result<(), AppError> {
        let app = self.app.read().expect("lock poisoned");
        let profile = app
            .get_profile(profile_id)
            .ok_or_else(|| AppError::NotFound(format!("profile not found: {profile_id}")))?;
        app.open_folder(Path::new(&profile.checkout_root))
    }

    fn launch_arma3_for_profile(&self, profile_id: &str) -> Result<(), AppError> {
        let app = self.app.read().expect("lock poisoned");
        app.launch_arma3_for_profile(profile_id, None)
    }

    fn launch_arma3_for_path(&self, base_path: &Path, extra_args: &str) -> Result<(), AppError> {
        let app = self.app.read().expect("lock poisoned");
        app.launch_arma3_for_path(base_path, extra_args)
    }

    fn launch_args_preview(&self, profile_id: &str) -> Result<String, AppError> {
        let app = self.app.read().expect("lock poisoned");
        app.arma3_launch_preview_for_profile(profile_id, None)
    }

    fn request_launch_args_preview(&self, profile_id: &str) {
        let result = {
            let app = self.app.read().expect("lock poisoned");
            app.arma3_launch_preview_for_profile(profile_id, None)
        };
        with_model_mut(&self.model, |model| match result {
            Ok(preview) => {
                model.launch_args_preview = Some(preview);
                model.launch_args_error = None;
            }
            Err(e) => {
                model.launch_args_preview = None;
                model.launch_args_error = Some(e.to_string());
            }
        });
    }

    fn request_repo_spec(&self, profile_id: &str) {
        let app_lock = Arc::clone(&self.app);
        let model_lock = Arc::clone(&self.model);
        let profile_id = profile_id.to_string();

        let generation = {
            let mut gen = 0u64;
            with_model_mut(&self.model, |model| {
                model.repo_spec = None;
                model.repo_spec_error = None;
                model.repo_spec_generation = model.repo_spec_generation.wrapping_add(1);
                gen = model.repo_spec_generation;
            });
            gen
        };

        // Repo fetching is async.
        tokio::spawn(async move {
            let res = async {
                let profile = {
                    let app = app_lock.read().expect("lock poisoned");
                    app.get_profile(&profile_id)
                        .ok_or_else(|| AppError::NotFound(profile_id.clone()))?
                };

                let remote = fleet_remote_http::HttpRemote::new(&profile.repo_url)
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;
                remote
                    .fetch_repo_spec()
                    .await
                    .map_err(|e| AppError::SyncEngine(e.to_string()))
            }
            .await;

            with_model_mut(&model_lock, |model| {
                if model.repo_spec_generation != generation {
                    return;
                }
                match res {
                    Ok(spec) => {
                        model.repo_spec = Some(spec);
                        model.repo_spec_error = None;
                    }
                    Err(e) => {
                        model.repo_spec = None;
                        model.repo_spec_error = Some(e.to_string());
                    }
                }
            });
        });
    }

    fn request_repo_spec_for_url(&self, repo_url: &str) {
        let model_lock = Arc::clone(&self.model);
        let repo_url = repo_url.to_string();

        let generation = {
            let mut gen = 0u64;
            with_model_mut(&self.model, |model| {
                model.repo_spec = None;
                model.repo_spec_error = None;
                model.repo_spec_generation = model.repo_spec_generation.wrapping_add(1);
                gen = model.repo_spec_generation;
            });
            gen
        };

        tokio::spawn(async move {
            let res = async {
                let remote = fleet_remote_http::HttpRemote::new(&repo_url)
                    .map_err(|e| AppError::SyncEngine(e.to_string()))?;
                remote
                    .fetch_repo_spec()
                    .await
                    .map_err(|e| AppError::SyncEngine(e.to_string()))
            }
            .await;

            with_model_mut(&model_lock, |model| {
                if model.repo_spec_generation != generation {
                    return;
                }
                match res {
                    Ok(spec) => {
                        model.repo_spec = Some(spec);
                        model.repo_spec_error = None;
                    }
                    Err(e) => {
                        model.repo_spec = None;
                        model.repo_spec_error = Some(e.to_string());
                    }
                }
            });
        });
    }

    fn request_linux_validation(&self, profile_id: &str) {
        let result = {
            let app = self.app.read().expect("lock poisoned");
            app.arma3_linux_template_validation_for_profile(profile_id, None, None)
        };
        with_model_mut(&self.model, |model| match result {
            Ok(v) => {
                model.linux_validation = Some(v);
                model.linux_validation_error = None;
            }
            Err(e) => {
                model.linux_validation = None;
                model.linux_validation_error = Some(e.to_string());
            }
        });
    }

    fn request_linux_validation_with_settings(&self, profile_id: &str, settings: AppSettings) {
        let result = {
            let app = self.app.read().expect("lock poisoned");
            app.arma3_linux_template_validation_for_profile(profile_id, None, Some(settings))
        };
        with_model_mut(&self.model, |model| match result {
            Ok(v) => {
                model.linux_validation = Some(v);
                model.linux_validation_error = None;
            }
            Err(e) => {
                model.linux_validation = None;
                model.linux_validation_error = Some(e.to_string());
            }
        });
    }

    fn set_last_sync_outcome(&self, outcome: Option<crate::sync::SyncOutcome>) {
        with_model_mut(&self.model, |model| {
            model.last_sync_outcome = outcome;
        });
    }

    fn rebuild_index(&self, profile_id: &str) -> Result<(), AppError> {
        let app = self.app.read().expect("lock poisoned");
        app.clear_index(profile_id)
    }

    fn clear_cache(&self, profile_id: &str) -> Result<(), AppError> {
        let app = self.app.read().expect("lock poisoned");
        app.clear_cache(profile_id)
    }

    fn clear_last_sync_outcome(&self) {
        with_model_mut(&self.model, |model| {
            model.last_sync_outcome = None;
        });
    }

    fn init_storage(&self) -> Result<(), AppError> {
        {
            let mut app = self.app.write().expect("lock poisoned");
            app.init_storage()?;
        }
        self.refresh_cached_profiles(true)
    }

    fn profiles_path(&self) -> Result<String, AppError> {
        let app = self.app.read().expect("lock poisoned");
        Ok(app.profiles_path().to_string())
    }

    fn settings_path(&self) -> Result<String, AppError> {
        let app = self.app.read().expect("lock poisoned");
        Ok(app.settings_path().to_string())
    }
}

fn with_model_mut<M: Clone>(slot: &RwLock<Arc<M>>, f: impl FnOnce(&mut M)) {
    let mut guard = slot.write().unwrap_or_else(|e| e.into_inner());
    let mut next = (**guard).clone();
    f(&mut next);
    *guard = Arc::new(next);
}
