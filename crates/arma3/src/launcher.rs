use std::path::PathBuf;

use crate::command::{LaunchCommand, LaunchMethod};
use crate::mods::{ModList, ModPathStyle};
use crate::steam::{resolve_arma3_executable, validate_arma3_install_dir, ARMA3_APP_ID};
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct Arma3Install {
    pub dir: PathBuf,
}

impl Arma3Install {
    pub fn validate(&self) -> Result<()> {
        validate_arma3_install_dir(&self.dir)
    }

    pub fn executable_path(&self) -> Result<PathBuf> {
        resolve_arma3_executable(&self.dir)
    }
}

#[derive(Debug, Clone)]
pub struct LaunchRequest {
    pub method: LaunchMethod,
    /// Extra user args (e.g. `-skipIntro`, `-world=empty`).
    pub args: Vec<String>,
    /// Local mod directories.
    pub mods: ModList,
}

impl LaunchRequest {
    pub fn new(method: LaunchMethod, mods: ModList) -> Self {
        Self {
            method,
            args: Vec::new(),
            mods,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Launcher {
    install: Arma3Install,
}

impl Launcher {
    pub fn new(install: Arma3Install) -> Self {
        Self { install }
    }

    /// Build a spawnable launch command, without launching.
    pub fn build_command(&self, req: &LaunchRequest) -> Result<LaunchCommand> {
        // Decide mod path style. If you're launching through Steam+Proton, use Z: mapping.
        let mod_style = match req.method {
            LaunchMethod::Arma3Exe => ModPathStyle::Native,
            LaunchMethod::SteamNative => {
                if cfg!(target_os = "linux") {
                    ModPathStyle::ProtonZDrive
                } else {
                    ModPathStyle::Native
                }
            }
        };

        let mut args: Vec<String> = Vec::new();

        // Base launch line
        match req.method {
            LaunchMethod::Arma3Exe => {}
            LaunchMethod::SteamNative => {
                args.push("-applaunch".into());
                args.push(ARMA3_APP_ID.to_string());
                args.push("-nolauncher".into());
            }
        }

        // Add mods as a single `-mod=...` arg if any.
        if !req.mods.paths().is_empty() {
            args.push(req.mods.to_mod_arg(mod_style));
        }

        // Add user args
        args.extend(req.args.clone());

        // Validate executable availability when chosen
        let program = match req.method {
            LaunchMethod::Arma3Exe => self.install.executable_path()?.display().to_string(),
            LaunchMethod::SteamNative => {
                let Some(path) = resolve_bin("steam", steam_fallbacks()) else {
                    return Err(Error::SteamNotFound);
                };
                path.display().to_string()
            }
        };

        Ok(LaunchCommand { program, args })
    }
}

/// Tiny PATH lookup helper so we don't need an extra crate.
fn which_in_path(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for p in std::env::split_paths(&path) {
        let candidate = p.join(bin);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let candidate_exe = p.join(format!("{bin}.exe"));
            if candidate_exe.is_file() {
                return Some(candidate_exe);
            }
        }
    }
    None
}

fn resolve_bin(bin: &str, fallbacks: &[&str]) -> Option<PathBuf> {
    if let Some(p) = which_in_path(bin) {
        return Some(p);
    }
    for candidate in fallbacks {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn steam_fallbacks() -> &'static [&'static str] {
    if cfg!(unix) {
        &[
            "/usr/bin/steam",
            "/bin/steam",
            "/usr/local/bin/steam",
            "/snap/bin/steam",
        ]
    } else {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::{Arma3Install, LaunchMethod, LaunchRequest, Launcher};
    use crate::mods::ModList;

    #[test]
    fn arma3_exe_build_command_uses_x64_executable_and_runtime_args() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let install_dir = tmp.path();
        std::fs::write(install_dir.join("arma3_x64.exe"), b"").expect("write exe");

        let install = Arma3Install {
            dir: install_dir.to_path_buf(),
        };
        let launcher = Launcher::new(install);
        let mut req = LaunchRequest::new(LaunchMethod::Arma3Exe, ModList::new(Vec::new()));
        req.args = vec!["-noSplash".to_string()];

        let cmd = launcher.build_command(&req).expect("build command");
        assert!(cmd.program.to_ascii_lowercase().ends_with("arma3_x64.exe"));
        assert_eq!(cmd.args, vec!["-noSplash".to_string()]);
    }

    #[test]
    fn arma3_exe_build_command_prefers_x64_over_x86() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let install_dir = tmp.path();
        std::fs::write(install_dir.join("arma3.exe"), b"").expect("write x86 exe");
        std::fs::write(install_dir.join("arma3_x64.exe"), b"").expect("write x64 exe");

        let install = Arma3Install {
            dir: install_dir.to_path_buf(),
        };
        let launcher = Launcher::new(install);
        let req = LaunchRequest::new(LaunchMethod::Arma3Exe, ModList::new(Vec::new()));

        let cmd = launcher.build_command(&req).expect("build command");
        assert!(cmd.program.to_ascii_lowercase().ends_with("arma3_x64.exe"));
    }
}
