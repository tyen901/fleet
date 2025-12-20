use std::fmt;
use std::path::{Path, PathBuf};

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

use crate::registry::LaunchMode;

#[derive(Debug)]
pub enum LaunchError {
    NoModsFound(String),
    DriveCNotFound(String),
    OpenFailed(std::io::Error),
    Other(String),
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LaunchError::NoModsFound(base) => {
                write!(f, "no mod directories found under {base}")
            }
            LaunchError::DriveCNotFound(base) => write!(
                f,
                "on Linux/Proton, base path must be inside a 'drive_c' directory (got {base})"
            ),
            LaunchError::OpenFailed(err) => write!(f, "failed to open: {err}"),
            LaunchError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for LaunchError {}

impl From<std::io::Error> for LaunchError {
    fn from(err: std::io::Error) -> Self {
        Self::OpenFailed(err)
    }
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

/// Build the actual Arma 3 command line (what goes inside the steam:// url),
/// including fully expanded mod folder paths.
pub fn build_arma3_commandline(
    base_path: &Path,
    enabled_mod_dirs: &[String],
    extra_args: &str,
) -> Result<String, LaunchError> {
    let discovered = discover_mod_dirs(base_path);
    let mods = if enabled_mod_dirs.is_empty() {
        discovered
    } else {
        let set: std::collections::HashSet<_> = discovered.into_iter().collect();
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

    let mut mod_list = String::new();
    for m in mods {
        let p = proton_base.join(m).to_string_lossy().to_string();
        mod_list.push_str(&p);
        mod_list.push(';');
    }

    let extra = extra_args.trim();
    if extra.is_empty() {
        Ok(format!(r#"-noLauncher -mod="{}""#, mod_list))
    } else {
        Ok(format!(r#"-noLauncher -mod="{}" {}"#, mod_list, extra))
    }
}

pub fn build_arma3_steam_url(
    base_path: &Path,
    enabled_mod_dirs: &[String],
    extra_args: &str,
) -> Result<String, LaunchError> {
    let cmdline = build_arma3_commandline(base_path, enabled_mod_dirs, extra_args)?;
    let encoded = utf8_percent_encode(&cmdline, NON_ALPHANUMERIC);
    Ok(format!("steam://run/107410//{encoded}/"))
}

fn open_target(mode: LaunchMode, target: &str) -> Result<(), LaunchError> {
    match mode {
        LaunchMode::SystemDefault => {
            open::that(target)?;
            Ok(())
        }

        LaunchMode::LinuxFlatpakHost => {
            #[cfg(target_os = "linux")]
            {
                use std::process::Command;
                let st = Command::new("flatpak-spawn")
                    .args(["--host", "xdg-open", target])
                    .status()
                    .map_err(LaunchError::OpenFailed)?;

                if st.success() {
                    Ok(())
                } else {
                    Err(LaunchError::Other(format!(
                        "flatpak-spawn failed (exit={:?})",
                        st.code()
                    )))
                }
            }

            #[cfg(not(target_os = "linux"))]
            {
                open::that(target)?;
                Ok(())
            }
        }
    }
}

pub fn launch_arma3_via_steam(steam_url: String) -> Result<(), LaunchError> {
    open::that(steam_url)?;
    Ok(())
}

pub fn launch_arma3_via_steam_with_mode(
    steam_url: String,
    mode: LaunchMode,
) -> Result<(), LaunchError> {
    open_target(mode, &steam_url)
}

pub fn open_folder_in_file_manager(path: &Path, mode: LaunchMode) -> Result<(), LaunchError> {
    let s = path.to_string_lossy();
    open_target(mode, &s)
}
