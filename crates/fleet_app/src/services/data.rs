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

use crate::app::{AppError, FleetApp, ProfileSpec, ProfileUpdate};
use crate::settings::LaunchSettings;

/// User-facing application settings.
///
/// Fleet persists launch settings in its registry. This typedef exists so that
/// presenters do not depend directly on internal registry structures.
pub type AppSettings = LaunchSettings;

/// Request payload for creating a new profile.
#[derive(Debug, Clone)]
pub struct ProfileCreate {
    pub name: String,
    pub repo_url: String,
    pub checkout_root: String,
    pub select: bool,
    pub arma3_extra_args: String,
}

/// Presenter-ready authoritative data model (snapshot).
///
/// This model is designed for rendering. It may include derived and cached fields
/// that should persist across frames (e.g., last preview result).
#[derive(Debug, Clone)]
pub struct DataModel {
    pub warning: Option<String>,
    pub profiles: Vec<ProfileSpec>,
    pub selected_id: Option<String>,
    pub settings: AppSettings,

    // Cached outputs for UI/CLI convenience (optional).
    pub launch_args_preview: Option<String>,
    pub launch_args_error: Option<String>,
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

    /// Launch Arma 3 for the given profile.
    fn launch_arma3_for_profile(&self, profile_id: &str) -> Result<(), AppError>;

    /// Launch Arma 3 for an explicit path (for CLI parity).
    fn launch_arma3_for_path(&self, base_path: &Path, extra_args: &str) -> Result<(), AppError>;

    /// Compute launch args preview for the given profile (pure; no UI types).
    fn launch_args_preview(&self, profile_id: &str) -> Result<String, AppError>;

    /// Request that the service compute and cache a preview (and error if any)
    /// into the model.
    fn request_launch_args_preview(&self, profile_id: &str);
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
        };

        Arc::new(Self {
            app,
            model: Arc::new(RwLock::new(Arc::new(model))),
        })
    }

    fn refresh_cached_profiles(&self, clear_preview: bool) -> Result<(), AppError> {
        let mut app = self.app.write().expect("lock poisoned");
        app.refresh_registry()?;

        let profiles = app.list_profiles();
        let selected = app.selected_profile().map(|p| p.id.clone());

        with_model_mut(&self.model, |model| {
            model.profiles = profiles;
            model.selected_id = selected;
            if clear_preview {
                model.launch_args_preview = None;
                model.launch_args_error = None;
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
            app.add_profile(
                &create.name,
                &create.repo_url,
                &create.checkout_root,
                create.select,
            )?
        };

        // Immediately update extra args if provided.
        if !create.arma3_extra_args.is_empty() {
            let update = ProfileUpdate {
                name: None,
                repo_url: None,
                checkout_root: None,
                select: None,
                arma3_extra_args: Some(create.arma3_extra_args.clone()),
            };
            let mut app = self.app.write().expect("lock poisoned");
            app.update_profile(&spec.id, update)?;
        }

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
}

fn with_model_mut<M: Clone>(slot: &RwLock<Arc<M>>, f: impl FnOnce(&mut M)) {
    let mut guard = slot.write().unwrap_or_else(|e| e.into_inner());
    let mut next = (**guard).clone();
    f(&mut next);
    *guard = Arc::new(next);
}
