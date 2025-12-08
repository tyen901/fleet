use camino::Utf8PathBuf;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use std::process::Stdio;
use thiserror::Error;

pub mod platform;
use crate::launcher::platform::PathTranslator;

#[derive(Debug, Error)]
pub enum LaunchError {
    #[error("Launch configuration error: {0}")]
    Config(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Command parsing error")]
    ParseError,
}

pub struct Launcher {
    exe_path: String,
    base_args: String,
    template: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedLaunchCommand {
    pub program: String,
    pub args: Vec<String>,
    pub working_dir: std::path::PathBuf,
}

const MODS_PLACEHOLDER: &str = "__FLEET_MODS__";
const FLATPAK_STEAM_APP_ID: &str = "com.valvesoftware.Steam";
const ARMA3_APP_ID: &str = "107410";

fn open_url(url: &str) -> Result<(), LaunchError> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?;
        Ok(())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?;
        Ok(())
    }
}

fn steam_run_url_from_flatpak_cmd(cmd: &ResolvedLaunchCommand) -> Option<String> {
    if cmd.program != "flatpak" {
        return None;
    }

    // flatpak run com.valvesoftware.Steam -applaunch 107410 <args...>
    if cmd.args.len() < 4 {
        return None;
    }
    if cmd.args[0] != "run" {
        return None;
    }
    if cmd.args[1] != FLATPAK_STEAM_APP_ID {
        return None;
    }
    if cmd.args[2] != "-applaunch" || cmd.args[3] != ARMA3_APP_ID {
        return None;
    }

    let extra_args = &cmd.args[4..];

    if extra_args.is_empty() {
        // No arguments to forward; just launch the app.
        return Some(format!("steam://run/{ARMA3_APP_ID}"));
    }

    // Percent-encode the argument string so it can be safely embedded in a steam://run URL.
    let arg_string = extra_args.join(" ");
    let encoded = utf8_percent_encode(&arg_string, NON_ALPHANUMERIC).to_string();

    // Match the pattern from the reference implementation: steam://run/APP_ID//encoded_args/
    Some(format!("steam://run/{ARMA3_APP_ID}//{encoded}/"))
}

#[cfg(target_os = "windows")]
fn split_command_windows(cmd: &str) -> Option<Vec<String>> {
    // Windows paths use backslashes heavily; treating `\` as an escape (POSIX shlex)
    // breaks paths like `C:\test` into `C:test`. For Windows launches, we only need
    // basic double-quote grouping and whitespace splitting.
    let mut parts = Vec::<String>::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in cmd.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if in_quotes {
        return None;
    }

    if !current.is_empty() {
        parts.push(current);
    }

    Some(parts)
}

impl Launcher {
    pub fn new(exe_path: String, base_args: String, template: String) -> Self {
        Self {
            exe_path,
            base_args,
            template,
        }
    }

    pub(crate) fn resolve_command(
        &self,
        mods: Vec<Utf8PathBuf>,
    ) -> Result<ResolvedLaunchCommand, LaunchError> {
        let exe_path = self.exe_path.clone();

        let mod_list: Vec<String> = mods
            .iter()
            .map(|p| PathTranslator::to_game_path(p).to_string())
            .collect();

        let mod_arg = if mod_list.is_empty() {
            String::new()
        } else {
            format!("-mod={};", mod_list.join(";"))
        };

        let cmd_str = self
            .template
            .replace("$GAME", &exe_path)
            .replace("$ARGS", &self.base_args)
            // Replace `$MODS` with a placeholder so argument splitting happens before we inject
            // any paths containing whitespace (e.g. `C:\New folder\@mod`). This makes the launch
            // robust even if the template doesn't quote `$MODS`.
            .replace("$MODS", MODS_PLACEHOLDER);

        #[cfg(target_os = "windows")]
        let mut parts = split_command_windows(&cmd_str).ok_or(LaunchError::ParseError)?;
        #[cfg(not(target_os = "windows"))]
        let mut parts = shlex::split(&cmd_str).ok_or(LaunchError::ParseError)?;

        if mod_arg.is_empty() {
            parts.retain(|p| p != MODS_PLACEHOLDER);
        } else {
            for p in &mut parts {
                if p.contains(MODS_PLACEHOLDER) {
                    *p = p.replace(MODS_PLACEHOLDER, &mod_arg);
                }
            }
        }

        if parts.is_empty() {
            return Err(LaunchError::Config(
                "Launch template produced empty command".into(),
            ));
        }

        let program = parts[0].clone();
        let args = parts[1..].to_vec();

        let working_dir = if self.template.contains("$GAME") {
            std::path::PathBuf::from(&exe_path)
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."))
        } else {
            std::path::PathBuf::from(".")
        };

        Ok(ResolvedLaunchCommand {
            program,
            args,
            working_dir,
        })
    }

    pub fn launch(&self, mods: Vec<Utf8PathBuf>) -> Result<(), LaunchError> {
        let cmd = self.resolve_command(mods)?;

        // Log the resolved command so it can be inspected when debugging launch issues.
        eprintln!(
            "[fleet] Launching program: {:?}, args: {:?}, cwd: {:?}",
            cmd.program, cmd.args, cmd.working_dir
        );

        std::process::Command::new(&cmd.program)
            .args(&cmd.args)
            .current_dir(&cmd.working_dir)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_mod_paths_survive_template_splitting_without_quotes() {
        let launcher = Launcher::new(
            "".to_string(),
            "-noSplash -skipIntro -noLauncher".to_string(),
            "steam --applaunch 107410 $ARGS $MODS".to_string(),
        );

        let mods = [
            Utf8PathBuf::from(r"C:\test\@addon1"),
            Utf8PathBuf::from(r"C:\test mods\@addon2"),
        ];

        let cmd = launcher
            .resolve_command(mods.to_vec())
            .expect("expected command to resolve");

        let mod_part = cmd
            .args
            .iter()
            .find(|p| p.starts_with("-mod="))
            .expect("expected -mod argument");

        assert!(mod_part.contains(r"C:\test\@addon1"));
        assert!(mod_part.contains(r"C:\test mods\@addon2"));
        assert!(!mod_part.contains("C:test"));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_steam_template_resolves_to_steam_applaunch_107410() {
        let launcher = Launcher::new(
            "".to_string(),
            "-noSplash -skipIntro -noLauncher".to_string(),
            "steam -applaunch 107410 $ARGS \"$MODS\"".to_string(),
        );

        let mods = vec![
            Utf8PathBuf::from("/home/tyen/Mods/@mod1"),
            Utf8PathBuf::from("/home/tyen/Mods With Spaces/@mod2"),
        ];

        let cmd = launcher
            .resolve_command(mods)
            .expect("expected command to resolve");

        assert_eq!(cmd.program, "steam");
        assert_eq!(cmd.working_dir, std::path::PathBuf::from("."));

        assert!(cmd.args.starts_with(&[
            "-applaunch".to_string(),
            "107410".to_string(),
            "-noSplash".to_string(),
            "-skipIntro".to_string(),
            "-noLauncher".to_string(),
        ]));

        let mod_part = cmd
            .args
            .iter()
            .find(|p| p.starts_with("-mod="))
            .expect("expected -mod argument");

        assert!(mod_part.contains("Z:\\home\\tyen\\Mods\\@mod1"));
        assert!(mod_part.contains("Z:\\home\\tyen\\Mods With Spaces\\@mod2"));
    }

    #[test]
    fn flatpak_steam_launch_uses_steam_run_url() {
        let cmd = ResolvedLaunchCommand {
            program: "flatpak".to_string(),
            args: vec![
                "run".to_string(),
                "com.valvesoftware.Steam".to_string(),
                "-applaunch".to_string(),
                "107410".to_string(),
                "-noSplash".to_string(),
                "-noLauncher".to_string(),
                "-mod=C:\\mods\\@ace;".to_string(),
            ],
            working_dir: std::path::PathBuf::from("."),
        };

        let url = steam_run_url_from_flatpak_cmd(&cmd).expect("expected url");
        assert_eq!(
            url,
            "steam://run/107410//%2DnoSplash%20%2DnoLauncher%20%2Dmod%3DC%3A%5Cmods%5C%40ace%3B/"
        );
    }
}
