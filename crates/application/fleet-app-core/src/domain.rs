use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::pipeline::PipelineState;
use fleet_scanner::ScanStats;

pub type ProfileId = String;

#[cfg(target_os = "windows")]
pub const STEAM_LAUNCH_TEMPLATE: &str = "steam --applaunch 107410 $ARGS \"$MODS\"";
#[cfg(not(target_os = "windows"))]
pub const STEAM_LAUNCH_TEMPLATE: &str = "steam -applaunch 107410 $ARGS \"$MODS\"";
pub const FLATPAK_STEAM_LAUNCH_TEMPLATE: &str =
    "flatpak run com.valvesoftware.Steam -applaunch 107410 $ARGS \"$MODS\"";

#[derive(Debug, Clone)]
pub enum FlatpakSteamAvailability {
    Unknown,
    Available,
    Unavailable(String),
}

fn default_launch_template() -> String {
    STEAM_LAUNCH_TEMPLATE.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub repo_url: String,
    pub local_path: String,
    pub last_synced: Option<DateTime<Utc>>,
    pub last_scan: Option<ScanStats>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "New Profile".to_string(),
            repo_url: String::new(),
            local_path: String::new(),
            last_synced: None,
            last_scan: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub max_threads: usize,
    pub speed_limit_enabled: bool,
    pub max_speed_bytes: u64,
    pub launch_params: String,
    #[serde(default = "default_launch_template")]
    pub launch_template: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            max_threads: fleet_config::DEFAULT_DOWNLOAD_THREADS,
            speed_limit_enabled: false,
            max_speed_bytes: fleet_config::DEFAULT_SPEED_LIMIT_BYTES,
            launch_params: "-noPause -noSplash -skipIntro -noLauncher".to_string(),
            launch_template: default_launch_template(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    ProfileHub,
    ProfileDashboard(ProfileId),
    ProfileEditor(ProfileId),
    Settings,
}

#[derive(Debug, Clone)]
pub enum BootState {
    Loading,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub boot: BootState,
    pub route: Route,

    pub profiles: Vec<Profile>,
    pub settings: AppSettings,
    pub settings_draft: Option<AppSettings>,
    pub flatpak_steam: FlatpakSteamAvailability,
    pub selected_profile_id: Option<ProfileId>,

    pub editor_draft: Option<Profile>,

    pub pipeline: PipelineState,
    pub last_plan: Option<fleet_core::SyncPlan>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            boot: BootState::Loading,
            route: Route::ProfileHub,
            profiles: Vec::new(),
            settings: AppSettings::default(),
            settings_draft: None,
            flatpak_steam: FlatpakSteamAvailability::Unknown,
            selected_profile_id: None,
            editor_draft: None,
            pipeline: PipelineState::idle(),
            last_plan: None,
        }
    }
}
