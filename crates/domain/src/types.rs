use anyhow::Context;
use serde::{Deserialize, Deserializer, Serialize};
use specta::Type;
use std::path::PathBuf;
use std::str::FromStr;

pub type ProfileId = String;
pub const INVENTORY_REBUILD_REQUIRED_CODE: &str = "inventory_rebuild_required";

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
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

    pub fn is_inventory_rebuild_required(&self) -> bool {
        self.code == INVENTORY_REBUILD_REQUIRED_CODE
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, Type, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct ProfileServerInfo {
    pub address: String,
    pub port: u16,
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct RepoServer {
    pub address: String,
    pub port: u16,
    pub password: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileSourceKind<'a> {
    Http(&'a str),
}

impl Profile {
    pub fn source_kind(&self) -> ProfileSourceKind<'_> {
        let source = self.source.trim();
        ProfileSourceKind::Http(source)
    }

    pub fn validated_source_kind(&self) -> anyhow::Result<ProfileSourceKind<'_>> {
        let source = self.source.trim();
        if source.is_empty() {
            anyhow::bail!("profile.source is empty (expected swifty repo URL)");
        }

        if !source.starts_with("http://") && !source.starts_with("https://") {
            anyhow::bail!("profile.source must be an http(s) Swifty repo manifest URL");
        }

        let url = url::Url::parse(source).context("parse profile.source URL")?;
        let has_filename = matches!(
            url.path_segments()
                .and_then(|mut segments| segments.next_back()),
            Some(seg) if !seg.is_empty()
        );
        if !has_filename {
            anyhow::bail!("remote profile.source must be a Swifty repo manifest URL");
        }
        Ok(ProfileSourceKind::Http(source))
    }

    pub fn dest_path(&self) -> anyhow::Result<PathBuf> {
        let dest = PathBuf::from(self.destination.trim());
        if dest.as_os_str().is_empty() {
            anyhow::bail!("profile.destination is empty");
        }
        Ok(dest)
    }
}

#[cfg(test)]
mod tests {
    use super::Profile;

    fn profile_with_source(source: &str) -> Profile {
        Profile {
            id: "p".to_string(),
            name: "p".to_string(),
            source: source.to_string(),
            destination: "/tmp".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn validated_source_rejects_root_url() {
        let profile = profile_with_source("https://example.com/");
        assert!(profile.validated_source_kind().is_err());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Type)]
pub enum TelemetryPreference {
    #[default]
    Unset,
    Allowed,
    Denied,
}

impl TelemetryPreference {
    pub fn is_enabled(self) -> bool {
        matches!(self, TelemetryPreference::Allowed)
    }
}

impl Serialize for TelemetryPreference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            TelemetryPreference::Unset => serializer.serialize_none(),
            TelemetryPreference::Allowed => serializer.serialize_bool(true),
            TelemetryPreference::Denied => serializer.serialize_bool(false),
        }
    }
}

impl<'de> Deserialize<'de> for TelemetryPreference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RawTelemetryPreference {
            Bool(bool),
            String(String),
            Null,
        }

        let raw = RawTelemetryPreference::deserialize(deserializer)?;
        let parsed = match raw {
            RawTelemetryPreference::Bool(true) => TelemetryPreference::Allowed,
            RawTelemetryPreference::Bool(false) => TelemetryPreference::Denied,
            RawTelemetryPreference::String(value) => {
                match value.trim().to_ascii_lowercase().as_str() {
                    "allowed" | "true" | "yes" | "on" => TelemetryPreference::Allowed,
                    "denied" | "false" | "no" | "off" => TelemetryPreference::Denied,
                    _ => TelemetryPreference::Unset,
                }
            }
            RawTelemetryPreference::Null => TelemetryPreference::Unset,
        };
        Ok(parsed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PrivacySettings {
    #[serde(default)]
    pub telemetry_consent: TelemetryPreference,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            telemetry_consent: TelemetryPreference::Unset,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct RuntimeSettings {
    /// If true, enable debug-level logs to disk (trace is always disabled).
    #[serde(default)]
    pub debug_log_to_disk: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct AppSettings {
    #[serde(flatten)]
    pub arma3: Arma3Settings,
    #[serde(flatten)]
    pub ui: UiSettings,
    #[serde(flatten)]
    pub privacy: PrivacySettings,
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
