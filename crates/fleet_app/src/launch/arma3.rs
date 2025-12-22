use std::collections::HashSet;
use std::path::Path;

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

use crate::constants::ARMA3_DEFAULT_EXTRA_ARGS;

#[derive(Debug, Clone)]
pub struct Arma3LaunchPlan {
    /// Unencoded commandline (what goes in steam:// url).
    pub commandline: String,
    /// The steam:// run game URL.
    pub steam_url: String,
}

#[derive(thiserror::Error, Debug)]
pub enum LaunchError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

pub fn discover_mod_dirs(base: &Path) -> Vec<String> {
    let mut mods = Vec::new();
    let Ok(rd) = std::fs::read_dir(base) else {
        return mods;
    };
    for ent in rd.flatten() {
        let Ok(ft) = ent.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let name = ent.file_name().to_string_lossy().to_string();
        if name.starts_with('@') {
            mods.push(name);
        }
    }
    mods.sort();
    mods
}

pub fn plan_launch(
    base: &Path,
    enabled_mods: &[String],
    extra_args: &str,
) -> Result<Arma3LaunchPlan, LaunchError> {
    let (commandline, _mods) = build_arma3_commandline(base, enabled_mods, extra_args)?;
    let steam_url = build_arma3_steam_url(&commandline);
    Ok(Arma3LaunchPlan {
        commandline,
        steam_url,
    })
}

pub fn build_arma3_steam_url(commandline: &str) -> String {
    let encoded = utf8_percent_encode(commandline, NON_ALPHANUMERIC).to_string();
    format!("steam://rungameid/107410//{encoded}")
}

pub fn build_arma3_commandline(
    base: &Path,
    enabled_mods: &[String],
    extra_args: &str,
) -> Result<(String, Vec<String>), LaunchError> {
    // 1. Discover all @folders
    let available = discover_mod_dirs(base);
    let available_set: HashSet<_> = available.iter().cloned().collect();

    // 2. Filter enabled
    let mut mods = Vec::new();
    for m in enabled_mods {
        let name = if m.starts_with('@') {
            m.clone()
        } else {
            format!("@{m}")
        };
        if available_set.contains(&name) {
            mods.push(name);
        }
    }

    // 3. Build -mod="..." string
    // Windows: -mod=C:\...\@foo;C:\...\@bar
    // Proton/Linux: -mod=Z:\...\@foo;Z:\...\@bar  (Steam translates paths better if we use relative or absolute)
    // Actually for Proton, if we use Windows paths mapped to Z:, it works. But simpler: use relative paths if cwd is correct.
    // However, steam launch option doesn't set cwd easily.
    // Best practice for robust cross-platform launch via Steam URL: use absolute paths.
    // On Linux (Proton), we need to convert /home/foo -> Z:\home\foo or similar if we want to be "correct",
    // OR we just pass mod names if they are in the game dir.
    // But Fleet checks out mods to a library folder, not game dir.
    // So we need absolute paths.

    // For now, let's assume we pass full paths.
    let mod_paths: Vec<String> = mods
        .iter()
        .map(|m| base.join(m).to_string_lossy().to_string())
        .collect();

    // On Linux we might need to massage paths for Proton if we were doing this manually.
    // But since we are launching via steam://, Steam Linux client handles some translation?
    // Actually, widespread advice is that -mod parameter needs Windows-style paths for Proton.
    // So /home/user/games/fleet/@mod -> Z:\home\user\games\fleet\@mod
    #[cfg(target_os = "linux")]
    let mod_arg = {
        let mut parts = Vec::new();
        for p in mod_paths {
            parts.push(convert_host_path_to_proton(&p));
        }
        parts.join(";")
    };

    #[cfg(not(target_os = "linux"))]
    let mod_arg = mod_paths.join(";");

    let mod_list = if mod_arg.is_empty() {
        String::new()
    } else {
        // Quote entire mod string? Arma 3 launcher often does -mod="a;b;c"
        mod_arg
    };

    let extra = extra_args.trim();
    let effective_extra = if extra.is_empty() {
        ARMA3_DEFAULT_EXTRA_ARGS
    } else {
        extra
    };

    Ok((format!(r#"{effective_extra} -mod="{mod_list}""#), mods))
}

#[cfg(target_os = "linux")]
fn convert_host_path_to_proton(path: &str) -> String {
    // Simple heuristic: map / to \ and prepend Z:
    let windows_style = path.replace('/', "\\");
    format!("Z:{}", windows_style)
}
