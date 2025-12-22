use serde::{Deserialize, Serialize};

use crate::constants::ARMA3_DEFAULT_EXTRA_ARGS;

/// How the app should open URLs/paths (folder open, steam:// handling, etc).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpenMode {
    /// Default OS handler (Windows ShellExecute / Linux xdg-open)
    SystemDefault,
    /// When running inside Flatpak, open via host: `flatpak-spawn --host xdg-open ...`
    LinuxFlatpakHost,
}

impl Default for OpenMode {
    fn default() -> Self {
        Self::SystemDefault
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaunchSettings {
    /// NOTE: alias preserves old persisted field name `mode` (from the previous design).
    #[serde(default, alias = "mode")]
    pub open_mode: OpenMode,

    /// Per-game launch configuration (currently only Arma 3 is modeled).
    #[serde(default)]
    pub arma3: Arma3LaunchSettings,
}

impl Default for LaunchSettings {
    fn default() -> Self {
        Self {
            open_mode: OpenMode::default(),
            arma3: Arma3LaunchSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Arma3LaunchSettings {
    #[serde(default)]
    pub windows: WindowsArma3LaunchSettings,
    #[serde(default)]
    pub linux: LinuxArma3LaunchSettings,
}

impl Default for Arma3LaunchSettings {
    fn default() -> Self {
        Self {
            windows: WindowsArma3LaunchSettings::default(),
            linux: LinuxArma3LaunchSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowsLaunchMethod {
    /// Launch Arma 3 directly via Arma3_x64.exe
    DirectExe,
    /// Launch via Steam executable: Steam.exe -applaunch 107410 <args>
    SteamAppLaunch,
    /// Launch via protocol handler: steam://rungameid/107410//<encoded cmdline>
    SteamUri,
}

impl Default for WindowsLaunchMethod {
    fn default() -> Self {
        // Default that "works" without requiring Steam.exe path configuration.
        Self::SteamUri
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowsArma3LaunchSettings {
    #[serde(default)]
    pub method: WindowsLaunchMethod,

    /// Required if method=direct_exe
    pub arma3_exe: Option<String>,

    /// Required if method=steam_app_launch
    pub steam_exe: Option<String>,
}

impl Default for WindowsArma3LaunchSettings {
    fn default() -> Self {
        Self {
            method: WindowsLaunchMethod::default(),
            arma3_exe: None,
            steam_exe: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LinuxModPathStyle {
    /// Use native host paths (e.g. /home/user/...).
    Native,
    /// Convert host paths to a Windows-ish Z:\... form (for Proton/Wine style usage).
    ProtonZ,
}

impl Default for LinuxModPathStyle {
    fn default() -> Self {
        Self::Native
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinuxArma3LaunchSettings {
    /// User template command; $ARGS and $MODS will be substituted.
    ///
    /// Example:
    ///   steam -applaunch 107410 $ARGS $MODS
    #[serde(default = "default_linux_arma3_template")]
    pub template: String,

    /// How to render mod paths in $MODS.
    #[serde(default)]
    pub mod_path_style: LinuxModPathStyle,

    /// Shell program to run template through (Linux only).
    /// Default: "sh"
    pub shell: Option<String>,
}

fn default_linux_arma3_template() -> String {
    "steam -applaunch 107410 $ARGS $MODS".to_string()
}

impl Default for LinuxArma3LaunchSettings {
    fn default() -> Self {
        Self {
            template: default_linux_arma3_template(),
            mod_path_style: LinuxModPathStyle::default(),
            shell: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
