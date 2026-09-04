use anyhow::Context;
use serde::{Deserialize, Deserializer, Serialize};
use std::path::PathBuf;
use std::str::FromStr;

pub type ProfileId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub source: String,
    pub destination: String,
    #[serde(default)]
    pub arma3_server: Option<ProfileServerInfo>,
    #[serde(default)]
    pub launch_params: String,
    #[serde(default)]
    pub additional_mod_folders: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileServerInfo {
    pub address: String,
    pub port: u16,
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoServer {
    pub address: String,
    pub port: u16,
    pub password: String,
}

impl Profile {
    pub fn dest_path(&self) -> anyhow::Result<PathBuf> {
        let dest = PathBuf::from(self.destination.trim());
        if dest.as_os_str().is_empty() {
            anyhow::bail!("profile.destination is empty");
        }
        Ok(dest)
    }
}

pub fn validated_repo_url(source: &str) -> anyhow::Result<&str> {
    let source = source.trim();
    if source.is_empty() {
        anyhow::bail!("profile.source is empty (expected Swifty repo manifest URL)");
    }

    let url = url::Url::parse(source).context("parse profile.source URL")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        anyhow::bail!("profile.source must be an http(s) Swifty repo manifest URL");
    }
    if url.path().rsplit('/').next().is_none_or(str::is_empty) {
        anyhow::bail!("profile.source must name a Swifty repo manifest");
    }

    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::validated_repo_url;

    #[test]
    fn repo_url_accepts_manifest_filenames_and_queries() {
        assert_eq!(
            validated_repo_url(" https://example.com/releases/current.manifest?channel=stable ")
                .expect("valid manifest URL"),
            "https://example.com/releases/current.manifest?channel=stable"
        );
    }

    #[test]
    fn repo_url_rejects_sources_without_an_http_manifest_file() {
        for source in [
            "",
            "https://example.com/",
            "https://example.com",
            "ftp://example.com/repo.json",
            "not a URL",
        ] {
            assert!(validated_repo_url(source).is_err(), "{source:?} must fail");
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Arma3LaunchMethod {
    Arma3Exe,
    Steam,
    Custom,
}

impl Arma3LaunchMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Arma3LaunchMethod::Arma3Exe => "arma3-exe",
            Arma3LaunchMethod::Steam => "steam",
            Arma3LaunchMethod::Custom => "custom",
        }
    }

    pub fn default_for_current_platform() -> Self {
        #[cfg(target_os = "windows")]
        {
            Arma3LaunchMethod::Arma3Exe
        }
        #[cfg(not(target_os = "windows"))]
        {
            Arma3LaunchMethod::Steam
        }
    }

    pub fn normalize_for_current_platform(self) -> Self {
        #[cfg(target_os = "windows")]
        {
            match self {
                Arma3LaunchMethod::Arma3Exe | Arma3LaunchMethod::Custom => self,
                _ => Arma3LaunchMethod::Arma3Exe,
            }
        }
        #[cfg(target_os = "linux")]
        {
            match self {
                Arma3LaunchMethod::Steam | Arma3LaunchMethod::Custom => self,
                Arma3LaunchMethod::Arma3Exe => Arma3LaunchMethod::Steam,
            }
        }
        #[cfg(all(not(target_os = "windows"), not(target_os = "linux")))]
        {
            match self {
                Arma3LaunchMethod::Steam | Arma3LaunchMethod::Custom => self,
                _ => Arma3LaunchMethod::Steam,
            }
        }
    }

    pub fn selectable_for_current_platform() -> &'static [Self] {
        #[cfg(target_os = "windows")]
        {
            &[Arma3LaunchMethod::Arma3Exe, Arma3LaunchMethod::Custom]
        }
        #[cfg(target_os = "linux")]
        {
            &[Arma3LaunchMethod::Steam, Arma3LaunchMethod::Custom]
        }
        #[cfg(all(not(target_os = "windows"), not(target_os = "linux")))]
        {
            &[Arma3LaunchMethod::Steam, Arma3LaunchMethod::Custom]
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Arma3LaunchMethod::Arma3Exe => "Launch Arma 3 directly from the configured executable.",
            Arma3LaunchMethod::Steam => "Launch via system Steam.",
            Arma3LaunchMethod::Custom => "Run a custom launch command template.",
        }
    }

    pub fn display_label(self) -> &'static str {
        match self {
            Arma3LaunchMethod::Arma3Exe => "Arma 3 executable",
            Arma3LaunchMethod::Steam => "Steam",
            Arma3LaunchMethod::Custom => "Custom command",
        }
    }
}

impl FromStr for Arma3LaunchMethod {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "arma3-exe" => Ok(Arma3LaunchMethod::Arma3Exe),
            "steam" => Ok(Arma3LaunchMethod::Steam),
            "custom" => Ok(Arma3LaunchMethod::Custom),
            _ => Err(()),
        }
    }
}

impl<'de> Deserialize<'de> for Arma3LaunchMethod {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(raw
            .parse::<Arma3LaunchMethod>()
            .unwrap_or_else(|_| Arma3LaunchMethod::default_for_current_platform()))
    }
}

fn default_arma3_launch_method() -> Arma3LaunchMethod {
    Arma3LaunchMethod::default_for_current_platform()
}

fn default_arma3_custom_launch_template() -> String {
    #[cfg(target_os = "windows")]
    {
        "arma3_x64.exe $ARGS $MODS".to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        "steam $ARGS $MODS".to_string()
    }
}

pub const DEFAULT_ARMA3_ARGS: &str = "-noPause -noSplash -skipIntro -noLauncher";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Arma3Settings {
    #[serde(default)]
    pub arma3_default_args: String,
    #[serde(default)]
    pub arma3_game_dir: String,
    #[serde(default = "default_arma3_launch_method")]
    pub arma3_launch_method: Arma3LaunchMethod,
    #[serde(default = "default_arma3_custom_launch_template")]
    pub arma3_custom_launch_template: String,
}

impl Default for Arma3Settings {
    fn default() -> Self {
        Self {
            arma3_default_args: String::new(),
            arma3_game_dir: String::new(),
            arma3_launch_method: default_arma3_launch_method(),
            arma3_custom_launch_template: default_arma3_custom_launch_template(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiSettings {
    /// If true, the app will skip the first-run onboarding flow.
    ///
    /// - New installs default to `false` (so onboarding always runs on first launch).
    /// - Existing installs that deserialize older `settings.json` (missing this field)
    ///   default to `true` to avoid unexpectedly re-triggering onboarding.
    #[serde(default = "default_true")]
    pub onboarding_completed: bool,
    /// If true, the UI shows profile `icon.png` images in the sidebar/dashboard.
    /// If false, the UI falls back to text.
    #[serde(default = "default_true")]
    pub show_profile_icons: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            onboarding_completed: false,
            show_profile_icons: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeSettings {
    /// If true, enable debug-level logs to disk (trace is always disabled).
    #[serde(default)]
    pub debug_log_to_disk: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartupSettings {
    /// If true, automatically check profiles once after app startup.
    #[serde(default = "default_true")]
    pub auto_check_profiles_on_startup: bool,
}

impl Default for StartupSettings {
    fn default() -> Self {
        Self {
            auto_check_profiles_on_startup: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateSettings {
    /// If true, automatically check for Fleet app updates once after app startup.
    #[serde(default = "default_true")]
    pub auto_check_on_startup: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            auto_check_on_startup: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSettings {
    #[serde(flatten)]
    pub arma3: Arma3Settings,
    #[serde(flatten)]
    pub ui: UiSettings,
    #[serde(flatten)]
    pub runtime: RuntimeSettings,
    #[serde(flatten)]
    pub startup: StartupSettings,
    #[serde(flatten)]
    pub updates: UpdateSettings,
}

fn default_true() -> bool {
    true
}

pub fn normalize_app_settings(mut settings: AppSettings) -> AppSettings {
    if settings.arma3.arma3_default_args.trim().is_empty() {
        settings.arma3.arma3_default_args = DEFAULT_ARMA3_ARGS.to_string();
    }
    settings.arma3.arma3_launch_method = settings
        .arma3
        .arma3_launch_method
        .normalize_for_current_platform();
    settings
}
