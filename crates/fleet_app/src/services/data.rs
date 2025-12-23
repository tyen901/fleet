//! Data service and model for Fleet.
//!
//! The data service is responsible for all profile and settings management as
//! well as any domain‑derived display state that must persist across UI
//! frames.  It exposes a single authoritative [`DataModel`] protected by
//! `Arc<RwLock<…>` and provides a pull‑based [`snapshot`] method to obtain
//! a cheap, read‑only clone of the current state.  All mutation methods
//! update the underlying model immediately; there are no event streams.

use std::path::Path;
use std::sync::{Arc, RwLock};

use egui;

use crate::app::{AppError, FleetApp, ProfileSpec, ProfileUpdate};
use crate::settings::LaunchSettings;

/// User‑facing application settings.
///
/// At present Fleet persists only launch settings in its registry.  This
/// typedef exists so that the UI does not depend directly on
/// [`LaunchSettings`].  Additional settings may be added later without
/// breaking the public API.
pub type AppSettings = LaunchSettings;

/// Request payload for creating a new profile.
///
/// When creating a profile the caller must specify the profile name,
/// repository URL, checkout root, whether the profile should become
/// selected immediately and any extra Arma 3 arguments.  The data service
/// converts this into a `FleetApp` call via [`FleetApp::add_profile`].
#[derive(Debug, Clone)]
pub struct ProfileCreate {
    pub name: String,
    pub repo_url: String,
    pub checkout_root: String,
    pub select: bool,
    pub arma3_extra_args: String,
}

/// Authoritative data model for the UI.
///
/// The fields of this struct are deliberately plain and ready for UI
/// consumption.  All state that the UI needs to render across frames—such
/// as the list of profiles, the currently selected profile id, the
/// persisted settings and any cached launch preview or error—is stored
/// here.  UI‑only state (e.g. editor drafts, sidebar filter) must be held
/// locally in the appropriate screen and not cached here.
#[derive(Debug, Clone)]
pub struct DataModel {
    pub warning: Option<String>,
    pub profiles: Vec<ProfileSpec>,
    pub selected_id: Option<String>,
    pub settings: AppSettings,
    pub launch_args_preview: Option<String>,
    pub launch_args_error: Option<String>,
}

/// Data service trait.
///
/// Consumers obtain an immutable snapshot of the current [`DataModel`] via
/// [`snapshot`].  Mutations update the underlying model immediately.  See
/// the implementation on [`FleetDataService`] for details.
pub trait DataService: Send + Sync {
    /// Return a cheap clone of the current data model.
    fn snapshot(&self) -> Arc<DataModel>;

    /// Refresh the profile list from disk.
    fn refresh_profiles(&self) -> Result<(), AppError>;

    /// Mark the given profile as selected.
    fn select_profile(&self, id: &str) -> Result<(), AppError>;

    /// Create a new profile.
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

    /// Launch Arma 3 for the given profile.
    fn launch_arma3_for_profile(&self, profile_id: &str) -> Result<(), AppError>;

    /// Request a preview of the launch arguments for the given profile.  The
    /// preview or any error will be written back into the model.
    fn request_launch_args_preview(&self, profile_id: &str);

    /// Copy the launch arguments for the given profile to the clipboard.
    fn copy_launch_args_to_clipboard(
        &self,
        egui_ctx: &egui::Context,
        profile_id: &str,
    ) -> Result<(), AppError>;
}

/// Concrete data service implementation backed by a [`FleetApp`].
pub struct FleetDataService {
    app: Arc<RwLock<FleetApp>>,
    model: Arc<RwLock<Arc<DataModel>>>,
}

impl FleetDataService {
    /// Create a new data service around the given [`FleetApp`].
    pub fn new(app: Arc<RwLock<FleetApp>>, warning: Option<String>) -> Arc<Self> {
        let app_settings = {
            let app = app.read().expect("lock poisoned");
            app.launch_settings()
        };
        let profiles;
        let selected_id;
        {
            let app = app.read().expect("lock poisoned");
            profiles = app.list_profiles();
            selected_id = app.selected_profile().map(|p| p.id.clone());
        }
        let model = DataModel {
            warning,
            profiles,
            selected_id,
            settings: app_settings,
            launch_args_preview: None,
            launch_args_error: None,
        };
        Arc::new(Self {
            app,
            model: Arc::new(RwLock::new(Arc::new(model))),
        })
    }

    /// Internal helper to refresh the cached profile list and selected id.
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
        // Cheaply clone the current model.
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
        // Immediately update any extra args.
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
        let defaults = LaunchSettings::default();
        self.set_settings(defaults)
    }

    fn open_folder(&self, path: &Path) -> Result<(), AppError> {
        let app = self.app.read().expect("lock poisoned");
        app.open_folder(path)
    }

    fn launch_arma3_for_profile(&self, profile_id: &str) -> Result<(), AppError> {
        let app = self.app.read().expect("lock poisoned");
        app.launch_arma3_for_profile(profile_id, None)
    }

    fn request_launch_args_preview(&self, profile_id: &str) {
        let result = {
            let app = self.app.read().expect("lock poisoned");
            // Try to preview; capture result or error.
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

    fn copy_launch_args_to_clipboard(
        &self,
        egui_ctx: &egui::Context,
        profile_id: &str,
    ) -> Result<(), AppError> {
        let preview = {
            let app = self.app.read().expect("lock poisoned");
            app.arma3_launch_preview_for_profile(profile_id, None)?
        };
        // Use egui's clipboard API to copy text to the system clipboard.
        egui_ctx.copy_text(preview);
        Ok(())
    }
}

fn with_model_mut<M: Clone>(slot: &RwLock<Arc<M>>, f: impl FnOnce(&mut M)) {
    let mut guard = slot.write().unwrap_or_else(|e| e.into_inner());
    let mut next = (**guard).clone();
    f(&mut next);
    *guard = Arc::new(next);
}
