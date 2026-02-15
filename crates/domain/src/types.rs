use anyhow::Context;
use serde::{Deserialize, Deserializer, Serialize};
use specta::Type;
use std::path::PathBuf;
use std::str::FromStr;

pub type ProfileId = String;

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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, Type)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub source: String,
    pub destination: String,
    #[serde(default)]
    pub arma3_server: Option<ProfileServerInfo>,
    #[serde(default)]
    pub launch_template: String,
    #[serde(default)]
    pub launch_params: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ProfileServerInfo {
    pub address: String,
    pub port: u16,
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
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
    use super::{Profile, ProfileSourceKind};

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
    fn validated_source_rejects_empty() {
        let profile = profile_with_source("   ");
        assert!(profile.validated_source_kind().is_err());
    }

    #[test]
    fn validated_source_rejects_root_url() {
        let profile = profile_with_source("https://example.com/");
        assert!(profile.validated_source_kind().is_err());
    }

    #[test]
    fn validated_source_accepts_http_manifest() {
        let profile = profile_with_source("https://example.com/repo.json");
        assert!(matches!(
            profile.validated_source_kind().unwrap(),
            ProfileSourceKind::Http(_)
        ));
    }

    #[test]
    fn validated_source_rejects_local_path() {
        let profile = profile_with_source("/tmp/manifest.json");
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
            Arma3LaunchMethod::Arma3Exe => "arma3 exe",
            Arma3LaunchMethod::Steam => "Steam (Native)",
            Arma3LaunchMethod::Custom => "Custom Command",
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

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppSettings {
    pub theme_mode: String,
    /// If true, the app will skip the first-run onboarding flow.
    ///
    /// - New installs default to `false` (so onboarding always runs on first launch).
    /// - Existing installs that deserialize older `settings.json` (missing this field)
    ///   default to `true` to avoid unexpectedly re-triggering onboarding.
    #[serde(default = "default_true")]
    pub onboarding_completed: bool,
    #[serde(default)]
    pub arma3_default_args: String,
    #[serde(default)]
    pub arma3_game_dir: String,
    #[serde(default = "default_arma3_launch_method")]
    pub arma3_launch_method: Arma3LaunchMethod,
    #[serde(default = "default_arma3_custom_launch_template")]
    pub arma3_custom_launch_template: String,
    /// If true, Desktop auto-cleans unexpected files detected during checks/sync flows.
    /// If false, Desktop prompts before cleanup.
    #[serde(default)]
    pub auto_cleanup_unexpected_files: bool,
    /// If true, the UI shows profile `icon.png` images in the sidebar/dashboard.
    /// If false, the UI falls back to text.
    #[serde(default = "default_true")]
    pub show_profile_icons: bool,
    #[serde(default = "default_telemetry_consent")]
    pub telemetry_consent: Option<bool>,
    /// If true, enable debug-level logs to disk (trace is always disabled).
    #[serde(default)]
    pub debug_log_to_disk: bool,
    #[serde(default = "default_release_channel")]
    pub release_channel: String,
    /// .gitignore-style patterns (one per line) applied to inventory scan/check paths.
    #[serde(default = "default_inventory_ignore_rules")]
    pub inventory_ignore_rules: String,
}

fn default_true() -> bool {
    true
}

fn default_telemetry_consent() -> Option<bool> {
    Some(true)
}

fn default_release_channel() -> String {
    "stable".to_string()
}

fn default_inventory_ignore_rules() -> String {
    crate::inventory::InventoryIgnoreRules::default().to_multiline_string()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme_mode: "dark".to_string(),
            onboarding_completed: false,
            arma3_default_args: String::new(),
            arma3_game_dir: String::new(),
            arma3_launch_method: default_arma3_launch_method(),
            arma3_custom_launch_template: default_arma3_custom_launch_template(),
            auto_cleanup_unexpected_files: false,
            show_profile_icons: true,
            telemetry_consent: Some(true),
            debug_log_to_disk: false,
            release_channel: "stable".to_string(),
            inventory_ignore_rules: default_inventory_ignore_rules(),
        }
    }
}
