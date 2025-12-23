use crate::core::types::{AppError, RequestId};
use eframe::egui;
use fleet_app::{FleetApp, LaunchSettings, ProfileSpec, ProfileUpdate, SyncMode, SyncTuning};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ProfilesSnapshot {
    pub warning: Option<String>,
    pub ui_error: Option<String>,
    pub profiles: Vec<ProfileSpec>,
    pub selected_id: Option<String>,
    pub sidebar_filter: String,
}

#[derive(Debug, Clone)]
pub struct DashboardSnapshot {
    pub loading: bool,
    pub error: Option<AppError>,
    pub profile: Option<ProfileSpec>,
    pub sync_mode: SyncMode,
    pub launch_args_preview: Option<String>,
    pub launch_args_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EditorDraft {
    pub id: Option<String>,
    pub name: String,
    pub repo_url: String,
    pub checkout_root: String,
    pub select: bool,
    pub arma3_extra_args: String,
}

impl EditorDraft {
    pub fn new_empty() -> Self {
        Self {
            id: None,
            name: String::new(),
            repo_url: String::new(),
            checkout_root: String::new(),
            select: true,
            arma3_extra_args: String::new(),
        }
    }

    pub fn from_spec(p: &ProfileSpec) -> Self {
        Self {
            id: Some(p.id.clone()),
            name: p.name.clone(),
            repo_url: p.repo_url.clone(),
            checkout_root: p.checkout_root.clone(),
            select: true,
            arma3_extra_args: p.arma3.extra_args.clone(),
        }
    }

    pub fn to_update(&self, original: &EditorDraft) -> ProfileUpdate {
        ProfileUpdate {
            name: (self.name != original.name).then(|| self.name.clone()),
            repo_url: (self.repo_url != original.repo_url).then(|| self.repo_url.clone()),
            checkout_root: (self.checkout_root != original.checkout_root)
                .then(|| self.checkout_root.clone()),
            select: Some(self.select),
            arma3_extra_args: (self.arma3_extra_args != original.arma3_extra_args)
                .then(|| self.arma3_extra_args.clone()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SettingsSnapshot {
    pub draft: SyncTuning,
    pub draft_launch: LaunchSettings,
    pub original: SyncTuning,
    pub original_launch: LaunchSettings,
}

#[derive(Debug, Clone)]
pub struct DataSnapshot {
    pub profiles: ProfilesSnapshot,
    pub dashboard: DashboardSnapshot,
    pub settings: Option<SettingsSnapshot>,
    pub launch: LaunchSettings,
    pub tuning: SyncTuning,
}

pub trait DataService: Send + Sync {
    fn snapshot(&self) -> Arc<DataSnapshot>;

    fn set_sidebar_filter(&self, filter: String);

    fn refresh_profiles(&self) -> RequestId;
    fn select_profile(&self, id: &str) -> Result<(), AppError>;

    fn set_sync_mode(&self, mode: SyncMode);

    fn begin_launch_args_preview(&self, profile_id: &str) -> RequestId;

    fn get_profile_for_edit(&self, id: &str) -> Option<ProfileSpec>;
    fn save_profile(&self, draft: EditorDraft) -> Result<String, AppError>; // returns new/edited id for navigation
    fn delete_profile(&self, id: &str) -> Result<(), AppError>;

    fn begin_settings(&self);
    fn cancel_settings(&self);
    fn reset_settings_to_defaults(&self);
    fn save_settings(&self) -> Result<(), AppError>;

    fn clear_ui_error(&self);

    fn open_folder(&self, path: &std::path::Path) -> Result<(), AppError>;
    fn launch_arma3_for_profile(&self, profile_id: &str) -> Result<(), AppError>;
    fn copy_launch_args_to_clipboard(
        &self,
        egui_ctx: &egui::Context,
        profile_id: &str,
    ) -> Result<(), AppError>;
}

struct Inner {
    snap: Arc<DataSnapshot>,
}

pub struct FleetDataService {
    app: Arc<RwLock<FleetApp>>,
    req: AtomicU64,
    inner: RwLock<Inner>,
}

impl FleetDataService {
    pub fn new(
        app: Arc<RwLock<FleetApp>>,
        warning: Option<String>,
        tuning: SyncTuning,
        launch: LaunchSettings,
    ) -> Arc<Self> {
        let profiles = app.read().list_profiles();
        let selected_id = app.read().selected_profile().map(|p| p.id.clone());

        let dashboard_profile = selected_id
            .as_deref()
            .and_then(|id| profiles.iter().find(|p| p.id == id).cloned());

        Arc::new(Self {
            app,
            req: AtomicU64::new(1),
            inner: RwLock::new(Inner {
                snap: Arc::new(DataSnapshot {
                    profiles: ProfilesSnapshot {
                        warning,
                        ui_error: None,
                        profiles: profiles.clone(),
                        selected_id: selected_id.clone(),
                        sidebar_filter: String::new(),
                    },
                    dashboard: DashboardSnapshot {
                        loading: false,
                        error: None,
                        profile: dashboard_profile,
                        sync_mode: tuning.mode,
                        launch_args_preview: None,
                        launch_args_error: None,
                    },
                    settings: None,
                    launch,
                    tuning,
                }),
            }),
        })
    }

    fn next_req(&self) -> RequestId {
        RequestId(self.req.fetch_add(1, Ordering::Relaxed))
    }

    fn refresh_locked(app: &mut FleetApp, snap: &mut DataSnapshot) {
        let _ = app.refresh_registry();
        let profiles = app.list_profiles();
        let selected_id = app.selected_profile().map(|p| p.id.clone());

        snap.profiles.profiles = profiles.clone();
        snap.profiles.selected_id = selected_id.clone();

        if let Some(id) = selected_id.as_deref() {
            snap.dashboard.profile = profiles.iter().find(|p| p.id == id).cloned();
        } else {
            snap.dashboard.profile = None;
        }
    }

    fn set_ui_error(snap: &mut DataSnapshot, err: AppError) {
        snap.profiles.ui_error = Some(err.message.clone());
        snap.dashboard.error = Some(err);
    }
}

impl DataService for FleetDataService {
    fn snapshot(&self) -> Arc<DataSnapshot> {
        Arc::clone(&self.inner.read().snap)
    }

    fn set_sidebar_filter(&self, filter: String) {
        let mut inner = self.inner.write();
        Arc::make_mut(&mut inner.snap).profiles.sidebar_filter = filter;
    }

    fn refresh_profiles(&self) -> RequestId {
        let id = self.next_req();
        let mut app = self.app.write();
        let mut inner = self.inner.write();
        Self::refresh_locked(&mut app, Arc::make_mut(&mut inner.snap));
        id
    }

    fn select_profile(&self, id: &str) -> Result<(), AppError> {
        let mut app = self.app.write();
        app.select_profile(id)
            .map_err(|e| AppError::new("select_profile_failed", format!("{e}")))?;

        let mut inner = self.inner.write();
        let snap = Arc::make_mut(&mut inner.snap);
        Self::refresh_locked(&mut app, snap);
        snap.dashboard.launch_args_preview = None;
        snap.dashboard.launch_args_error = None;
        Ok(())
    }

    fn set_sync_mode(&self, mode: SyncMode) {
        let mut inner = self.inner.write();
        let snap = Arc::make_mut(&mut inner.snap);
        snap.tuning.mode = mode;
        snap.dashboard.sync_mode = mode;
    }

    fn begin_launch_args_preview(&self, profile_id: &str) -> RequestId {
        let req = self.next_req();
        let app = self.app.write();
        let mut inner = self.inner.write();
        let snap = Arc::make_mut(&mut inner.snap);

        // Must not run during sync; that policy is enforced by the sync screen/HUD.
        let res: Result<String, String> = app
            .arma3_launch_preview_for_profile(profile_id, None)
            .map_err(|e| e.to_string());

        match res {
            Ok(s) => {
                snap.dashboard.launch_args_preview = Some(s);
                snap.dashboard.launch_args_error = None;
            }
            Err(e) => {
                snap.dashboard.launch_args_preview = None;
                snap.dashboard.launch_args_error = Some(e);
            }
        }

        req
    }

    fn get_profile_for_edit(&self, id: &str) -> Option<ProfileSpec> {
        let inner = self.inner.read();
        inner
            .snap
            .profiles
            .profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
    }

    fn save_profile(&self, draft: EditorDraft) -> Result<String, AppError> {
        let mut app = self.app.write();
        let mut inner = self.inner.write();
        let snap = Arc::make_mut(&mut inner.snap);

        if draft.id.is_none() {
            let p = app
                .add_profile(
                    draft.name.trim(),
                    draft.repo_url.trim(),
                    draft.checkout_root.trim(),
                    draft.select,
                )
                .map_err(|e| AppError::new("profile_add_failed", format!("{e}")))?;

            // Best-effort: update extra args via update call if the backend supports it.
            // (Current backend already supports arma3_extra_args in ProfileUpdate.)
            let upd = ProfileUpdate {
                name: None,
                repo_url: None,
                checkout_root: None,
                select: Some(draft.select),
                arma3_extra_args: Some(draft.arma3_extra_args.clone()),
            };
            let _ = app.update_profile(&p.id, upd);

            Self::refresh_locked(&mut app, snap);
            Ok(p.id)
        } else {
            let id = draft
                .id
                .clone()
                .ok_or_else(|| AppError::new("missing_profile_id", "Missing profile id"))?;

            let original = EditorDraft::from_spec(
                snap.profiles
                    .profiles
                    .iter()
                    .find(|p| p.id == id)
                    .ok_or_else(|| AppError::new("missing_profile", "Profile no longer exists"))?,
            );
            let update = draft.to_update(&original);
            app.update_profile(&id, update)
                .map_err(|e| AppError::new("profile_update_failed", format!("{e}")))?;

            Self::refresh_locked(&mut app, snap);
            Ok(id)
        }
    }

    fn delete_profile(&self, id: &str) -> Result<(), AppError> {
        let mut app = self.app.write();
        let mut inner = self.inner.write();

        app.remove_profile(id)
            .map_err(|e| AppError::new("profile_remove_failed", format!("{e}")))?;

        Self::refresh_locked(&mut app, Arc::make_mut(&mut inner.snap));
        Ok(())
    }

    fn begin_settings(&self) {
        let mut inner = self.inner.write();
        let snap = Arc::make_mut(&mut inner.snap);
        snap.settings = Some(SettingsSnapshot {
            draft: snap.tuning.clone(),
            draft_launch: snap.launch.clone(),
            original: snap.tuning.clone(),
            original_launch: snap.launch.clone(),
        });
    }

    fn cancel_settings(&self) {
        let mut inner = self.inner.write();
        Arc::make_mut(&mut inner.snap).settings = None;
    }

    fn reset_settings_to_defaults(&self) {
        let mut inner = self.inner.write();
        if let Some(s) = Arc::make_mut(&mut inner.snap).settings.as_mut() {
            s.draft = SyncTuning::default();
            s.draft_launch = LaunchSettings::default();
        }
    }

    fn save_settings(&self) -> Result<(), AppError> {
        let mut app = self.app.write();
        let mut inner = self.inner.write();
        let snap = Arc::make_mut(&mut inner.snap);

        let Some(s) = snap.settings.as_ref() else {
            return Err(AppError::new("settings_missing", "Settings state missing"));
        };

        let updated_launch = s.draft_launch.clone();
        app.set_launch_settings(updated_launch.clone())
            .map_err(|e| AppError::new("save_launch_failed", format!("{e}")))?;

        snap.tuning = s.draft.clone();
        snap.launch = updated_launch;
        snap.dashboard.sync_mode = snap.tuning.mode;
        snap.settings = None;
        Ok(())
    }

    fn clear_ui_error(&self) {
        let mut inner = self.inner.write();
        let snap = Arc::make_mut(&mut inner.snap);
        snap.profiles.ui_error = None;
        snap.dashboard.error = None;
    }

    fn open_folder(&self, path: &std::path::Path) -> Result<(), AppError> {
        let app = self.app.write();
        app.open_folder(path)
            .map_err(|e| AppError::new("open_folder_failed", format!("{e}")))?;
        Ok(())
    }

    fn launch_arma3_for_profile(&self, profile_id: &str) -> Result<(), AppError> {
        let app = self.app.write();
        app.launch_arma3_for_profile(profile_id, None)
            .map_err(|e| AppError::new("launch_failed", format!("{e}")))?;
        Ok(())
    }

    fn copy_launch_args_to_clipboard(
        &self,
        egui_ctx: &egui::Context,
        profile_id: &str,
    ) -> Result<(), AppError> {
        let app = self.app.write();
        let s = app
            .arma3_launch_preview_for_profile(profile_id, None)
            .map_err(|e| AppError::new("launch_args_failed", format!("{e}")))?;
        egui_ctx.copy_text(s);
        Ok(())
    }
}
