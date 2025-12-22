use std::path::{Path, PathBuf};
use std::process::Command;

use crate::platform::error::PlatformError;
use crate::settings::OpenMode;

#[derive(Debug, Clone)]
pub enum LaunchAction {
    /// Spawn a process directly (preferred when possible).
    Spawn {
        program: String,
        args: Vec<String>,
        cwd: Option<PathBuf>,
    },

    /// Open a URL (steam://, http://, etc).
    OpenUrl { url: String },

    /// Execute a shell command string (Linux template runner).
    Shell {
        command: String,
        shell: Option<String>,
    },
}

pub fn execute(open_mode: OpenMode, action: LaunchAction) -> Result<(), PlatformError> {
    match action {
        LaunchAction::Spawn { program, args, cwd } => {
            let mut cmd = Command::new(program);
            cmd.args(args);
            if let Some(cwd) = cwd {
                cmd.current_dir(cwd);
            }
            cmd.spawn().map_err(PlatformError::Io)?;
            Ok(())
        }

        LaunchAction::OpenUrl { url } => open_url(open_mode, &url),

        LaunchAction::Shell { command, shell } => run_shell(&command, shell.as_deref()),
    }
}

pub fn open_path(open_mode: OpenMode, path: &Path) -> Result<(), PlatformError> {
    let s = path.to_string_lossy().to_string();

    match open_mode {
        OpenMode::SystemDefault => {
            open::that(&s).map_err(|e| PlatformError::OpenFailed(e.to_string()))
        }
        OpenMode::LinuxFlatpakHost => {
            #[cfg(target_os = "linux")]
            {
                let st = Command::new("flatpak-spawn")
                    .args(["--host", "xdg-open", &s])
                    .status()
                    .map_err(PlatformError::Io)?;
                if st.success() {
                    Ok(())
                } else {
                    Err(PlatformError::OpenFailed(format!(
                        "flatpak-spawn xdg-open failed (exit={:?})",
                        st.code()
                    )))
                }
            }

            #[cfg(not(target_os = "linux"))]
            {
                open::that(&s).map_err(|e| PlatformError::OpenFailed(e.to_string()))
            }
        }
    }
}

fn open_url(open_mode: OpenMode, url: &str) -> Result<(), PlatformError> {
    // Special-case steam:// on Linux because protocol handlers are inconsistent across distros/packaging.
    #[cfg(target_os = "linux")]
    {
        if url.starts_with("steam://") {
            match open_mode {
                OpenMode::SystemDefault => {
                    Command::new("steam")
                        .arg(url)
                        .spawn()
                        .map_err(PlatformError::Io)?;
                    return Ok(());
                }
                OpenMode::LinuxFlatpakHost => {
                    Command::new("flatpak-spawn")
                        .args(["--host", "steam", url])
                        .spawn()
                        .map_err(PlatformError::Io)?;
                    return Ok(());
                }
            }
        }
    }

    // Fallback: generic opener
    match open_mode {
        OpenMode::SystemDefault => {
            open::that(url).map_err(|e| PlatformError::OpenFailed(e.to_string()))
        }
        OpenMode::LinuxFlatpakHost => {
            #[cfg(target_os = "linux")]
            {
                let st = Command::new("flatpak-spawn")
                    .args(["--host", "xdg-open", url])
                    .status()
                    .map_err(PlatformError::Io)?;
                if st.success() {
                    Ok(())
                } else {
                    Err(PlatformError::OpenFailed(format!(
                        "flatpak-spawn xdg-open failed (exit={:?})",
                        st.code()
                    )))
                }
            }

            #[cfg(not(target_os = "linux"))]
            {
                open::that(url).map_err(|e| PlatformError::OpenFailed(e.to_string()))
            }
        }
    }
}

fn run_shell(command: &str, shell: Option<&str>) -> Result<(), PlatformError> {
    #[cfg(target_os = "linux")]
    {
        let sh = shell.unwrap_or("sh");
        Command::new(sh)
            .args(["-lc", command])
            .spawn()
            .map_err(PlatformError::Io)?;
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = command;
        let _ = shell;
        Err(PlatformError::Unsupported(
            "shell launch actions are only supported on linux".to_string(),
        ))
    }
}
