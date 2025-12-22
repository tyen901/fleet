use serde::{Deserialize, Serialize};

use crate::constants::ARMA3_DEFAULT_EXTRA_ARGS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMode {
    /// Default OS handler (Windows ShellExecute / Linux xdg-open)
    #[default]
    SystemDefault,
    /// When running inside Flatpak, open via host: `flatpak-spawn --host xdg-open ...`
    LinuxFlatpakHost,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaunchSettings {
    #[serde(default)]
    pub mode: LaunchMode,
}

impl Default for LaunchSettings {
    fn default() -> Self {
        Self {
            mode: LaunchMode::SystemDefault,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arma3Config {
    #[serde(default)]
    pub extra_args: String,
    #[serde(default)]
    pub enabled_mods: Vec<String>,
}

impl Arma3Config {
    pub const DEFAULT_EXTRA_ARGS: &'static str = ARMA3_DEFAULT_EXTRA_ARGS;
}

impl Default for Arma3Config {
    fn default() -> Self {
        Self {
            extra_args: Self::DEFAULT_EXTRA_ARGS.to_string(),
            enabled_mods: Vec::new(),
        }
    }
}
