use crate::launch::arma3::LaunchError;
use crate::registry::LaunchMode;

/// Open a URL or filesystem path using the configured launch mode.
///
/// This isolates OS/process side effects from "planning" logic.
pub fn open_target(mode: LaunchMode, target: &str) -> Result<(), LaunchError> {
    match mode {
        LaunchMode::SystemDefault => {
            #[cfg(target_os = "linux")]
            {
                if target.starts_with("steam://") {
                    use std::process::Command;
                    Command::new("steam")
                        .arg(target)
                        .spawn()
                        .map_err(LaunchError::Io)?;
                    return Ok(());
                }
            }

            open::that(target).map_err(|e| LaunchError::Other(e.to_string()))?;
            Ok(())
        }

        LaunchMode::LinuxFlatpakHost => {
            #[cfg(target_os = "linux")]
            {
                use std::process::Command;

                if target.starts_with("steam://") {
                    Command::new("flatpak-spawn")
                        .args(["--host", "steam", target])
                        .spawn()
                        .map_err(LaunchError::Io)?;
                    Ok(())
                } else {
                    let st = Command::new("flatpak-spawn")
                        .args(["--host", "xdg-open", target])
                        .status()
                        .map_err(LaunchError::Io)?;

                    if st.success() {
                        Ok(())
                    } else {
                        Err(LaunchError::Other(format!(
                            "flatpak-spawn failed (exit={:?})",
                            st.code()
                        )))
                    }
                }
            }

            #[cfg(not(target_os = "linux"))]
            {
                open::that(target).map_err(|e| LaunchError::Other(e.to_string()))?;
                Ok(())
            }
        }
    }
}
