use crate::platform::error::PlatformError;
use crate::settings::LaunchMode;

/// Open a URL or filesystem path using the configured launch mode.
///
/// This isolates OS/process side effects from "planning" logic.
pub fn open_target(mode: LaunchMode, target: &str) -> Result<(), PlatformError> {
    match mode {
        LaunchMode::SystemDefault => {
            #[cfg(target_os = "linux")]
            {
                // Steam URI handling on Linux often needs explicit `steam` command
                if target.starts_with("steam://") {
                    use std::process::Command;
                    Command::new("steam")
                        .arg(target)
                        .spawn()
                        .map_err(PlatformError::Io)?;
                    return Ok(());
                }
            }

            open::that(target).map_err(|e| PlatformError::OpenFailed(e.to_string()))?;
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
                        .map_err(PlatformError::Io)?;
                    Ok(())
                } else {
                    let st = Command::new("flatpak-spawn")
                        .args(["--host", "xdg-open", target])
                        .status()
                        .map_err(PlatformError::Io)?;

                    if st.success() {
                        Ok(())
                    } else {
                        Err(PlatformError::OpenFailed(format!(
                            "flatpak-spawn failed (exit={:?})",
                            st.code()
                        )))
                    }
                }
            }

            #[cfg(not(target_os = "linux"))]
            {
                open::that(target).map_err(|e| PlatformError::OpenFailed(e.to_string()))?;
                Ok(())
            }
        }
    }
}
