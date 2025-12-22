use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

use crate::constants::ARMA3_DEFAULT_EXTRA_ARGS;
use crate::platform::LaunchAction;
#[cfg(target_os = "windows")]
use crate::settings::WindowsLaunchMethod;
use crate::settings::{LaunchSettings, LinuxModPathStyle};

#[derive(Debug, Clone)]
pub struct Arma3LaunchPlan {
    /// Human-readable preview of what will be executed.
    pub preview: String,
    /// Executable action (spawn/open/shell) for the current OS.
    pub action: LaunchAction,
}

#[derive(thiserror::Error, Debug)]
pub enum LaunchError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid extra args: {0}")]
    InvalidArgs(String),

    #[error("missing configuration: {0}")]
    MissingConfig(String),

    #[error("invalid linux template: {0}")]
    InvalidTemplate(String),

    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct LinuxTemplateValidation {
    pub ok: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub normalized_template: String,
    pub preview: String,
}

#[derive(Debug, Clone)]
struct ResolvedMod {
    _name: String,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct ArgsPlan {
    base_args: Vec<String>,
    mods: Vec<ResolvedMod>,
}

pub fn plan_launch(
    _base: &Path,
    _enabled_mods: &[String],
    _extra_args: &str,
    settings: &LaunchSettings,
) -> Result<Arma3LaunchPlan, LaunchError> {
    #[cfg(target_os = "windows")]
    {
        let plan = build_args_plan(_base, _enabled_mods, _extra_args)?;
        return plan_windows(plan, settings);
    }

    #[cfg(target_os = "linux")]
    {
        let plan = build_args_plan(_base, _enabled_mods, _extra_args)?;
        return plan_linux(plan, settings);
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = settings;
        Err(LaunchError::Other("unsupported target OS".to_string()))
    }
}

fn build_args_plan(
    base: &Path,
    enabled_mods: &[String],
    extra_args: &str,
) -> Result<ArgsPlan, LaunchError> {
    let base_abs = std::fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());

    let base_args = parse_extra_args(extra_args)?;
    let mods = resolve_enabled_mods(&base_abs, enabled_mods)?;

    Ok(ArgsPlan { base_args, mods })
}

fn parse_extra_args(extra_args: &str) -> Result<Vec<String>, LaunchError> {
    let s = extra_args.trim();
    let effective = if s.is_empty() {
        ARMA3_DEFAULT_EXTRA_ARGS
    } else {
        s
    };

    shell_words::split(effective).map_err(|e| LaunchError::InvalidArgs(e.to_string()))
}

fn resolve_enabled_mods(
    base: &Path,
    enabled_mods: &[String],
) -> Result<Vec<ResolvedMod>, std::io::Error> {
    let mut available: HashMap<String, PathBuf> = HashMap::new();

    if let Ok(rd) = std::fs::read_dir(base) {
        for ent in rd.flatten() {
            let Ok(ft) = ent.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            let name = ent.file_name().to_string_lossy().to_string();
            if name.starts_with('@') {
                available.insert(name.clone(), base.join(&name));
            }
        }
    }

    let mut out: Vec<ResolvedMod> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for m in enabled_mods {
        let name = normalize_mod_name(m);
        if seen.contains(&name) {
            continue;
        }
        if let Some(path) = available.get(&name) {
            seen.insert(name.clone());
            out.push(ResolvedMod {
                name,
                path: path.clone(),
            });
        }
    }

    Ok(out)
}

fn normalize_mod_name(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        return String::new();
    }
    if t.starts_with('@') {
        t.to_string()
    } else {
        format!("@{t}")
    }
}

fn build_mod_arg(mods: &[ResolvedMod], style: LinuxModPathStyle) -> Option<String> {
    if mods.is_empty() {
        return None;
    }

    let mut paths: Vec<String> = Vec::with_capacity(mods.len());
    for m in mods {
        let p = m.path.to_string_lossy().to_string();
        let rendered = match style {
            LinuxModPathStyle::Native => p,
            LinuxModPathStyle::ProtonZ => host_path_to_proton_z(&p),
        };
        paths.push(rendered);
    }

    Some(format!("-mod={}", paths.join(";")))
}

fn host_path_to_proton_z(path: &str) -> String {
    let windows_style = path.replace('/', "\\");
    format!("Z:{}", windows_style)
}

pub fn validate_linux_template(template: &str) -> LinuxTemplateValidation {
    let normalized_template = normalize_template_ws(template);

    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    if normalized_template.trim().is_empty() {
        errors.push("template is empty".to_string());
    }

    if normalized_template.contains('\0') {
        errors.push("template contains NUL byte".to_string());
    }

    if normalized_template.contains('\n') || normalized_template.contains('\r') {
        errors.push("template contains newline; must be a single command".to_string());
    }

    let args_count = count_substring(&normalized_template, "$ARGS");
    let mods_count = count_substring(&normalized_template, "$MODS");

    if args_count == 0 && mods_count == 0 {
        errors.push("template must include $ARGS and/or $MODS".to_string());
    }

    if args_count == 0 {
        warnings.push("template does not include $ARGS; extra args will not be passed".to_string());
    }
    if mods_count == 0 {
        warnings
            .push("template does not include $MODS; enabled mods will not be passed".to_string());
    }

    if args_count > 1 {
        warnings.push("template includes $ARGS more than once".to_string());
    }
    if mods_count > 1 {
        warnings.push("template includes $MODS more than once".to_string());
    }

    LinuxTemplateValidation {
        ok: errors.is_empty(),
        errors,
        warnings,
        normalized_template,
        preview: String::new(),
    }
}

pub fn validate_linux_template_strict(template: &str) -> Result<(), LaunchError> {
    let rep = validate_linux_template(template);
    if rep.ok {
        Ok(())
    } else {
        Err(LaunchError::InvalidTemplate(rep.errors.join("; ")))
    }
}

fn normalize_template_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn count_substring(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack.match_indices(needle).count()
}

pub fn linux_template_preview(
    base: &Path,
    enabled_mods: &[String],
    extra_args: &str,
    settings: &LaunchSettings,
) -> Result<LinuxTemplateValidation, LaunchError> {
    let plan = build_args_plan(base, enabled_mods, extra_args)?;

    let mut rep = validate_linux_template(&settings.arma3.linux.template);

    let args_str = join_shell_escaped(&plan.base_args);

    let mod_arg = build_mod_arg(&plan.mods, settings.arma3.linux.mod_path_style.clone());
    let mods_str = match mod_arg {
        Some(ma) => join_shell_escaped(&[ma]),
        None => String::new(),
    };

    if !rep.ok {
        rep.preview = rep.normalized_template.clone();
        return Ok(rep);
    }

    rep.preview = render_linux_template_strict(&rep.normalized_template, &args_str, &mods_str);
    Ok(rep)
}

fn render_linux_template_strict(template: &str, args_str: &str, mods_str: &str) -> String {
    let mut out = template.to_string();
    out = out.replace("$ARGS", args_str);
    out = out.replace("$MODS", mods_str);

    normalize_template_ws(&out)
}

#[cfg(target_os = "windows")]
fn plan_windows(plan: ArgsPlan, settings: &LaunchSettings) -> Result<Arma3LaunchPlan, LaunchError> {
    let win = &settings.arma3.windows;

    let mod_arg = build_mod_arg(&plan.mods, LinuxModPathStyle::Native);
    let mut args = plan.base_args.clone();
    if let Some(ma) = mod_arg.clone() {
        args.push(ma);
    }

    match win.method {
        WindowsLaunchMethod::DirectExe => {
            let exe = win
                .arma3_exe
                .as_ref()
                .ok_or_else(|| {
                    LaunchError::MissingConfig("arma3_exe is required for direct_exe".to_string())
                })?
                .clone();

            let cwd = PathBuf::from(&exe).parent().map(|p| p.to_path_buf());

            Ok(Arma3LaunchPlan {
                preview: format!("{} {}", quote_preview(&exe), join_preview(&args)),
                action: LaunchAction::Spawn {
                    program: exe,
                    args,
                    cwd,
                },
            })
        }

        WindowsLaunchMethod::SteamAppLaunch => {
            let steam = win
                .steam_exe
                .as_ref()
                .ok_or_else(|| {
                    LaunchError::MissingConfig(
                        "steam_exe is required for steam_app_launch".to_string(),
                    )
                })?
                .clone();

            let mut steam_args = vec!["-applaunch".to_string(), "107410".to_string()];
            steam_args.extend(args.clone());

            Ok(Arma3LaunchPlan {
                preview: format!("{} {}", quote_preview(&steam), join_preview(&steam_args)),
                action: LaunchAction::Spawn {
                    program: steam,
                    args: steam_args,
                    cwd: None,
                },
            })
        }

        WindowsLaunchMethod::SteamUri => {
            let cmdline = join_cmdline_for_steam_uri(&args);
            let url = build_arma3_steam_url(&cmdline);

            Ok(Arma3LaunchPlan {
                preview: url.clone(),
                action: LaunchAction::OpenUrl { url },
            })
        }
    }
}

#[cfg(target_os = "linux")]
fn plan_linux(plan: ArgsPlan, settings: &LaunchSettings) -> Result<Arma3LaunchPlan, LaunchError> {
    let lin = &settings.arma3.linux;

    validate_linux_template_strict(&lin.template)?;

    let normalized_template = normalize_template_ws(&lin.template);

    let mod_arg = build_mod_arg(&plan.mods, lin.mod_path_style);

    let args_str = join_shell_escaped(&plan.base_args);
    let mods_str = match mod_arg {
        Some(ma) => join_shell_escaped(&[ma]),
        None => String::new(),
    };

    let cmd_norm = render_linux_template_strict(&normalized_template, &args_str, &mods_str);

    Ok(Arma3LaunchPlan {
        preview: cmd_norm.clone(),
        action: LaunchAction::Shell {
            command: cmd_norm,
            shell: lin.shell.clone(),
        },
    })
}

pub fn build_arma3_steam_url(commandline: &str) -> String {
    let encoded = utf8_percent_encode(commandline, NON_ALPHANUMERIC).to_string();
    format!("steam://rungameid/107410//{encoded}")
}

#[cfg(target_os = "windows")]
fn join_cmdline_for_steam_uri(args: &[String]) -> String {
    args.iter()
        .map(|a| quote_for_cmdline(a))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(target_os = "windows")]
fn quote_for_cmdline(arg: &str) -> String {
    let needs = arg.chars().any(|c| c.is_whitespace() || c == '"');
    if !needs {
        return arg.to_string();
    }
    let escaped = arg.replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(target_os = "windows")]
fn join_preview(args: &[String]) -> String {
    args.iter()
        .map(|a| quote_preview(a))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(target_os = "windows")]
fn quote_preview(arg: &str) -> String {
    if arg.chars().any(|c| c.is_whitespace() || c == '"') {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_string()
    }
}

fn join_shell_escaped(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|t| shell_escape::unix::escape(t.as_str().into()).to_string())
        .collect::<Vec<_>>()
        .join(" ")
}
