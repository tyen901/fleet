use std::path::{Path, PathBuf};

use crate::{Error, Result};

pub const ARMA3_APP_ID: u32 = 107_410;

/// Discover Arma 3 install directory using HEMTT's steam helper.
/// This returns the *Steam common* install dir for the app (e.g. .../steamapps/common/Arma 3).
pub fn discover_steam_arma3() -> Option<PathBuf> {
    // HEMTT: hemtt_common::steam::find_app uses steamlocate under the hood
    hemtt_common::steam::find_app(ARMA3_APP_ID)
}

/// Minimal sanity checks (you can tighten these as you like).
pub fn validate_arma3_install_dir(dir: &Path) -> Result<()> {
    resolve_arma3_executable(dir)?;
    Ok(())
}

pub fn resolve_arma3_executable(path: &Path) -> Result<PathBuf> {
    if path.is_file() {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if name == "arma3_x64.exe" || name == "arma3.exe" {
            return Ok(path.to_path_buf());
        }
        return Err(Error::InvalidInstall {
            path: path.to_path_buf(),
        });
    }

    if !path.is_dir() {
        return Err(Error::InvalidInstall {
            path: path.to_path_buf(),
        });
    }

    // Common Windows exe names stored in the install dir. (Even on Linux/Proton the content is the same.)
    let x64 = path.join("arma3_x64.exe");
    let x86 = path.join("arma3.exe");

    if x64.is_file() {
        return Ok(x64);
    }
    if x86.is_file() {
        return Ok(x86);
    }

    Err(Error::MissingExecutable { path: x64 })
}
