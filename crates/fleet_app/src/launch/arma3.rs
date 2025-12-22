use std::collections::HashSet;
use std::path::{Path, PathBuf};

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

use crate::registry::Arma3Config;

#[derive(Debug, Clone)]
pub struct Arma3LaunchPlan {
    /// Unencoded commandline (what goes in steam:// url).
    pub commandline: String,
    /// Steam URL suitable for opening via system/steam.
    pub steam_url: String,
    /// Discovered and enabled mods (folder names) in deterministic order.
    pub enabled_mods: Vec<String>,
}

#[derive(thiserror::Error, Debug)]
pub enum LaunchError {
    #[error("no mod directories found under {0}")]
    NoModsFound(String),

    #[error("on Linux/Proton, base path must be inside a 'drive_c' directory (got {0})")]
    DriveCNotFound(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

pub fn discover_mod_dirs(base: &Path) -> Vec<String> {
    let mut mods = Vec::new();
    for entry in walkdir::WalkDir::new(base).min_depth(1).max_depth(1) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('@') {
            mods.push(name);
        }
    }
    mods.sort();
    mods
}

/// Convert a host filesystem path to the Proton/Windows path representation used by Arma 3.
/// This is only meaningful on non-Windows hosts when launching via Proton.
///
/// Notes:
/// - This implementation assumes the Proton prefix structure, where `drive_c` exists.
/// - If you later support a "native Windows" runtime on Linux (e.g., Wine without Steam),
///   split this into a strategy selected by settings.
#[cfg(windows)]
pub fn convert_host_base_path_to_proton_base_path(
    host_base_path: &Path,
) -> Result<PathBuf, LaunchError> {
    Ok(host_base_path.to_owned())
}

#[cfg(not(windows))]
pub fn convert_host_base_path_to_proton_base_path(
    host_base_path: &Path,
) -> Result<PathBuf, LaunchError> {
    let drive_c = host_base_path
        .ancestors()
        .find(|p| p.ends_with("drive_c"))
        .ok_or_else(|| LaunchError::DriveCNotFound(host_base_path.display().to_string()))?;

    let relative = host_base_path
        .strip_prefix(drive_c)
        .map_err(|_| LaunchError::DriveCNotFound(host_base_path.display().to_string()))?;

    Ok(Path::new("c:/").join(relative))
}

/// Build the Arma 3 commandline (unencoded), including fully expanded mod folder paths.
/// This is pure planning logic: no process spawning, no OS integration.
pub fn build_arma3_commandline(
    base_path: &Path,
    enabled_mod_dirs: &[String],
    extra_args: &str,
) -> Result<(String, Vec<String>), LaunchError> {
    let discovered = discover_mod_dirs(base_path);
    let mods = if enabled_mod_dirs.is_empty() {
        discovered
    } else {
        let set: HashSet<_> = discovered.into_iter().collect();
        enabled_mod_dirs
            .iter()
            .filter(|m| set.contains(*m))
            .cloned()
            .collect::<Vec<_>>()
    };

    if mods.is_empty() {
        return Err(LaunchError::NoModsFound(base_path.display().to_string()));
    }

    let proton_base = convert_host_base_path_to_proton_base_path(base_path)?;

    let mod_list = mods
        .iter()
        .map(|m| proton_base.join(m).to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(";");

    let extra = extra_args.trim();
    let effective_extra = if extra.is_empty() {
        Arma3Config::DEFAULT_EXTRA_ARGS
    } else {
        extra
    };

    Ok((format!(r#"{effective_extra} -mod="{mod_list}""#), mods))
}

pub fn build_arma3_steam_url(
    base_path: &Path,
    enabled_mod_dirs: &[String],
    extra_args: &str,
) -> Result<(String, String, Vec<String>), LaunchError> {
    let (cmdline, mods) = build_arma3_commandline(base_path, enabled_mod_dirs, extra_args)?;
    let encoded = utf8_percent_encode(&cmdline, NON_ALPHANUMERIC);
    let url = format!("steam://rungameid/107410//{encoded}/");
    Ok((url, cmdline, mods))
}

pub fn plan_launch(
    base_path: &Path,
    enabled_mod_dirs: &[String],
    extra_args: &str,
) -> Result<Arma3LaunchPlan, LaunchError> {
    let (steam_url, commandline, enabled_mods) =
        build_arma3_steam_url(base_path, enabled_mod_dirs, extra_args)?;
    Ok(Arma3LaunchPlan {
        commandline,
        steam_url,
        enabled_mods,
    })
}
