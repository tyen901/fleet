use crate::storage::profile_state_root_dir;
use crate::ApiError;
use crate::Core;
use fleet_arma3::{
    Arma3Install, Error as Arma3Error, LaunchCommand, LaunchMethod, LaunchRequest, Launcher,
    ModList,
};
use fleet_domain::health::{LocalHealthState, OperationKind, ProfileAssessmentReport};
use fleet_domain::{AppSettings, Arma3LaunchMethod, Profile, ProfileId, ProfileSourceKind};
use serde::Serialize;
use specta::Type;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tracing::info;

#[derive(Clone, Debug, Serialize, Type)]
pub struct ArmaLaunchResult {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionKind {
    Launch,
    Join,
}

impl Core {
    pub fn arma3_detect_install_dir(&self) -> Option<PathBuf> {
        detect_arma3_install_path()
    }

    pub fn arma3_steam_available(&self) -> bool {
        fleet_arma3::steam_available()
    }

    pub fn arma3_launch(
        &self,
        profile: &Profile,
        settings: &AppSettings,
        extra_args: Option<Vec<OsString>>,
        dry_run: bool,
    ) -> Result<ArmaLaunchResult, ApiError> {
        arma3_execute(profile, settings, ActionKind::Launch, extra_args, dry_run)
    }

    pub fn arma3_join(
        &self,
        profile: &Profile,
        settings: &AppSettings,
        extra_args: Option<Vec<OsString>>,
        dry_run: bool,
    ) -> Result<ArmaLaunchResult, ApiError> {
        arma3_execute(profile, settings, ActionKind::Join, extra_args, dry_run)
    }

    pub async fn arma3_launch_by_profile_id(
        &self,
        profile_id: ProfileId,
        extra_args: Option<Vec<String>>,
        dry_run: bool,
    ) -> Result<ArmaLaunchResult, ApiError> {
        self.arma3_run_action_by_profile_id(profile_id, ActionKind::Launch, extra_args, dry_run)
            .await
    }

    pub async fn arma3_join_by_profile_id(
        &self,
        profile_id: ProfileId,
        extra_args: Option<Vec<String>>,
        dry_run: bool,
    ) -> Result<ArmaLaunchResult, ApiError> {
        self.arma3_run_action_by_profile_id(profile_id, ActionKind::Join, extra_args, dry_run)
            .await
    }

    async fn arma3_run_action_by_profile_id(
        &self,
        profile_id: ProfileId,
        action: ActionKind,
        extra_args: Option<Vec<String>>,
        dry_run: bool,
    ) -> Result<ArmaLaunchResult, ApiError> {
        let assessment = self.launch_assessment_by_profile_id(&profile_id).await?;
        validate_launch_compatibility(&assessment)?;
        let (profile, settings) = self.load_profile_and_settings(&profile_id).await?;
        let extra_args_os: Option<Vec<OsString>> =
            extra_args.map(|v| v.into_iter().map(OsString::from).collect());

        let result = match action {
            ActionKind::Launch => self.arma3_launch(&profile, &settings, extra_args_os, dry_run),
            ActionKind::Join => self.arma3_join(&profile, &settings, extra_args_os, dry_run),
        };

        if let Ok(settings) = self.load_settings().await {
            self.update_state(|state| {
                state.settings = settings;
            });
        }

        result
    }

    async fn load_profile_and_settings(
        &self,
        profile_id: &ProfileId,
    ) -> Result<(Profile, AppSettings), ApiError> {
        let profile = self
            .load_profile(profile_id)
            .await
            .map_err(|e| ApiError::new("not_found", e.to_string()))?;

        let mut settings = self
            .load_settings()
            .await
            .map_err(|e| ApiError::new("settings_error", e.to_string()))?;
        self.ensure_arma3_settings(&mut settings).await?;

        Ok((profile, settings))
    }

    async fn launch_assessment_by_profile_id(
        &self,
        profile_id: &ProfileId,
    ) -> Result<ProfileAssessmentReport, ApiError> {
        if let Some(report) = self.read_state(|state| {
            state
                .profile_runtime_by_id
                .get(profile_id)
                .and_then(|runtime| runtime.assessment.clone())
        }) {
            return Ok(report);
        }

        let profile = self
            .load_profile(profile_id)
            .await
            .map_err(|e| ApiError::new("not_found", e.to_string()))?;
        let cfg = self.current_flow_config();
        let session_id = self
            .flow()
            .spawn_operation_with_config(cfg, profile, OperationKind::CheckLocal)
            .await
            .map_err(|e| ApiError::new("pipeline_error", e.to_string()))?;

        self.await_assessment(session_id).await
    }

    async fn ensure_arma3_settings(&self, settings: &mut AppSettings) -> Result<(), ApiError> {
        if settings.arma3.arma3_game_dir.trim().is_empty() {
            if let Some(path) = self.arma3_detect_install_dir() {
                settings.arma3.arma3_game_dir = path.to_string_lossy().to_string();
                self.save_settings(settings.clone())
                    .await
                    .map_err(|e| ApiError::new("settings_error", e.to_string()))?;
            } else {
                return Err(ApiError::new(
                    "arma3_not_found",
                    "Arma 3 install not found. Set the game directory or arma3_x64.exe in Settings.",
                ));
            }
        }
        Ok(())
    }
}

fn arma3_execute(
    profile: &Profile,
    settings: &AppSettings,
    kind: ActionKind,
    extra_args: Option<Vec<OsString>>,
    dry_run: bool,
) -> Result<ArmaLaunchResult, ApiError> {
    let built = build_launch(profile, settings, kind, extra_args.unwrap_or_default())
        .map_err(|e| ApiError::new("launch_failed", e.to_string()))?;

    let pid = if dry_run {
        None
    } else {
        info!(
            profile_id = %profile.id,
            profile_name = %profile.name,
            profile_source = %profile.source,
            profile_destination = %profile.destination,
            action = ?kind,
            launch_method = ?settings.arma3.arma3_launch_method,
            program = %built.spec.program,
            args = ?built.spec.args,
            "arma3 launch command"
        );
        let child = built
            .spec
            .spawn()
            .map_err(|e| ApiError::new("launch_failed", e))?;
        Some(child.id())
    };

    Ok(ArmaLaunchResult {
        program: built.spec.program.clone(),
        args: built.spec.args.clone(),
        cwd: built
            .spec
            .cwd
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        env: built.spec.env.clone(),
        pid,
    })
}

fn build_launch(
    profile: &Profile,
    settings: &AppSettings,
    kind: ActionKind,
    extra_args: Vec<OsString>,
) -> Result<LaunchPlan, Arma3Error> {
    let game_dir = resolve_game_dir(settings)?;
    let resolved_mode = resolve_launch_mode(settings)?;

    let install = Arma3Install { dir: game_dir };
    install.validate()?;

    let mods = discover_mods(profile)?;
    let mod_list = ModList::validate_and_normalize(mods.clone())?;

    let args = build_args(profile, settings, kind, extra_args)?;

    let mut req = LaunchRequest::new(resolved_mode.wrapper_method, mod_list);
    req.args = args;

    let launcher = Launcher::new(install);
    let base_command = launcher.build_command(&req)?;
    let spec = if let Some(template) = resolved_mode.custom_template {
        build_template_command(&template, &base_command)?
    } else {
        CommandSpec::from_launch_command(&base_command)
    };

    Ok(LaunchPlan { spec })
}

fn resolve_game_dir(settings: &AppSettings) -> Result<PathBuf, Arma3Error> {
    if let Some(p) = non_empty_path(&settings.arma3.arma3_game_dir) {
        return Ok(p);
    }
    detect_arma3_install_path().ok_or_else(|| {
        Arma3Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Arma 3 path is not set. Choose the game directory or arma3_x64.exe in Settings.",
        ))
    })
}

fn detect_arma3_install_path() -> Option<PathBuf> {
    let dir = fleet_arma3::discover_steam_arma3()?;
    #[cfg(target_os = "windows")]
    {
        // Match HEMTT behavior: discover install dir via Steam, then run the executable from it.
        let install = Arma3Install { dir };
        return install.executable_path().ok();
    }
    #[cfg(not(target_os = "windows"))]
    {
        Some(dir)
    }
}

#[derive(Debug, Clone)]
struct ResolvedLaunchMode {
    wrapper_method: LaunchMethod,
    custom_template: Option<String>,
}

fn resolve_launch_mode(settings: &AppSettings) -> Result<ResolvedLaunchMode, Arma3Error> {
    let method = settings
        .arma3
        .arma3_launch_method
        .normalize_for_current_platform();
    let custom_template = if method == Arma3LaunchMethod::Custom {
        let Some(template) = non_empty_string(&settings.arma3.arma3_custom_launch_template) else {
            return Err(Arma3Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "custom launch template is empty",
            )));
        };
        Some(template)
    } else {
        None
    };

    let wrapper_method = {
        #[cfg(target_os = "windows")]
        {
            match method {
                Arma3LaunchMethod::Arma3Exe | Arma3LaunchMethod::Custom => LaunchMethod::Arma3Exe,
                _ => {
                    return Err(Arma3Error::UnsupportedLaunchMethod {
                        method: method.as_str().to_string(),
                    });
                }
            }
        }
        #[cfg(target_os = "linux")]
        {
            match method {
                Arma3LaunchMethod::Steam => LaunchMethod::SteamNative,
                Arma3LaunchMethod::Custom => LaunchMethod::SteamNative,
                Arma3LaunchMethod::Arma3Exe => {
                    return Err(Arma3Error::UnsupportedLaunchMethod {
                        method: method.as_str().to_string(),
                    });
                }
            }
        }
        #[cfg(all(not(target_os = "linux"), not(target_os = "windows")))]
        {
            match method {
                Arma3LaunchMethod::Steam | Arma3LaunchMethod::Custom => LaunchMethod::SteamNative,
                _ => {
                    return Err(Arma3Error::UnsupportedLaunchMethod {
                        method: method.as_str().to_string(),
                    });
                }
            }
        }
    };

    Ok(ResolvedLaunchMode {
        wrapper_method,
        custom_template,
    })
}

fn build_args(
    profile: &Profile,
    settings: &AppSettings,
    kind: ActionKind,
    extra_args: Vec<OsString>,
) -> Result<Vec<String>, Arma3Error> {
    let mut args = if !profile.launch_params.trim().is_empty() {
        parse_args(&profile.launch_params)?
    } else {
        parse_args(&settings.arma3.arma3_default_args)?
    };

    let has_connect_override = extra_args.iter().any(|a| {
        let s = a.to_string_lossy();
        s.starts_with("-connect=") || s == "-connect"
    });

    if kind == ActionKind::Join && !has_connect_override {
        if let Some(server) = profile.arma3_server.as_ref() {
            args.extend(server_join_args(
                &server.address,
                server.port,
                &server.password,
            ));
        }
    }

    for arg in extra_args {
        args.push(arg.to_string_lossy().to_string());
    }

    Ok(args)
}

pub fn server_join_args(address: &str, port: u16, password: &str) -> Vec<String> {
    let address = address.trim();
    if address.is_empty() {
        return Vec::new();
    }

    let mut args = vec![format!("-connect={address}"), format!("-port={port}")];
    if !password.trim().is_empty() {
        args.push(format!("-password={password}"));
    }
    args
}

fn discover_mods(profile: &Profile) -> Result<Vec<PathBuf>, Arma3Error> {
    let root = profile.dest_path().map_err(|e| {
        Arma3Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            e.to_string(),
        ))
    })?;
    discover_mods_from_repo(profile, &root)
}

fn discover_mods_from_repo(profile: &Profile, root: &Path) -> Result<Vec<PathBuf>, Arma3Error> {
    let ProfileSourceKind::Http(repo_url) = profile.source_kind();

    let state_root = profile_state_root_dir().map_err(|e| {
        Arma3Error::Io(std::io::Error::other(format!(
            "resolve profile state root: {e}"
        )))
    })?;
    let cache_root = fleet_domain::repo_cache_dir(&state_root, &profile.id);
    let mod_names = match swifty_repo::enabled_mod_names(&cache_root, repo_url) {
        Ok(Some(names)) => names,
        Ok(None) => {
            return Err(Arma3Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "local swifty cache missing; sync the profile first",
            )));
        }
        Err(e) => {
            return Err(Arma3Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                e.to_string(),
            )));
        }
    };

    let mut out: Vec<PathBuf> = mod_names.into_iter().map(|m| root.join(m)).collect();
    out.sort_by(|a: &PathBuf, b: &PathBuf| a.to_string_lossy().cmp(&b.to_string_lossy()));
    Ok(out)
}

fn parse_args(args: &str) -> Result<Vec<String>, Arma3Error> {
    let parts = shell_words::split(args).map_err(|e| {
        Arma3Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            e.to_string(),
        ))
    })?;
    Ok(parts)
}

fn build_template_command(template: &str, base: &LaunchCommand) -> Result<CommandSpec, Arma3Error> {
    let tokens = shell_words::split(template).map_err(|e| {
        Arma3Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            e.to_string(),
        ))
    })?;
    if tokens.is_empty() {
        return Err(Arma3Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "launch template is empty",
        )));
    }

    let uses_args = tokens.iter().any(|t| t == "$ARGS" || t == "${ARGS}");
    let uses_mods = tokens.iter().any(|t| t == "$MODS" || t == "${MODS}");
    if !uses_args || !uses_mods {
        return Err(Arma3Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "custom launch template must include $ARGS and $MODS",
        )));
    }
    let mod_arg = base.args.iter().find(|a| a.starts_with("-mod=")).cloned();

    let mut out: Vec<String> = Vec::new();
    for token in tokens {
        match token.as_str() {
            "$ARGS" | "${ARGS}" => {
                let iter = base.args.iter().filter(|a| {
                    if uses_mods {
                        !a.starts_with("-mod=")
                    } else {
                        true
                    }
                });
                out.extend(iter.cloned());
            }
            "$MODS" | "${MODS}" => {
                if let Some(mods) = mod_arg.clone() {
                    out.push(mods);
                }
            }
            _ => out.push(token),
        }
    }

    if !uses_args {
        out.extend(base.args.iter().cloned());
    } else if !uses_mods {
        if let Some(mods) = mod_arg {
            if !out.iter().any(|a| a.starts_with("-mod=")) {
                out.push(mods);
            }
        }
    }

    let program = out.remove(0);
    Ok(CommandSpec {
        program,
        args: out,
        env: base
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        cwd: None,
    })
}

fn non_empty_path(s: &str) -> Option<PathBuf> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}

fn non_empty_string(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn validate_launch_compatibility(report: &ProfileAssessmentReport) -> Result<(), ApiError> {
    if matches!(
        report.local_health,
        LocalHealthState::Ready | LocalHealthState::LocalDrift
    ) {
        return Ok(());
    }

    Err(ApiError::new(
        "launch_incompatible",
        format!("launch blocked: local health {:?}", report.local_health),
    ))
}

#[derive(Debug, Clone)]
struct LaunchPlan {
    spec: CommandSpec,
}

#[derive(Debug, Clone)]
struct CommandSpec {
    program: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
}

impl CommandSpec {
    fn from_launch_command(plan: &LaunchCommand) -> Self {
        Self {
            program: plan.executable.clone(),
            args: plan.args.clone(),
            cwd: None,
            env: plan
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }

    fn spawn(&self) -> Result<std::process::Child, String> {
        let mut cmd = std::process::Command::new(&self.program);
        cmd.args(&self.args);
        if let Some(cwd) = &self.cwd {
            cmd.current_dir(cwd);
        }
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd.spawn().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{build_args, resolve_launch_mode, validate_launch_compatibility, ActionKind};
    use fleet_arma3::LaunchMethod;
    use fleet_domain::health::{LocalHealthState, ProfileAssessmentReport, RemoteFreshnessState};
    use fleet_domain::types::ProfileServerInfo;
    use fleet_domain::{AppSettings, Profile};
    use std::ffi::OsString;

    fn default_settings() -> AppSettings {
        let mut settings = AppSettings::default();
        settings.arma3.arma3_default_args = String::new();
        settings
    }

    fn default_profile() -> Profile {
        Profile {
            id: "p".into(),
            name: "p".into(),
            source: "https://example.com/repo.json".into(),
            destination: "/tmp".into(),
            ..Default::default()
        }
    }

    #[test]
    fn build_args_uses_profile_launch_params_over_settings() {
        let mut profile = default_profile();
        profile.launch_params = "-foo -bar".into();

        let mut settings = default_settings();
        settings.arma3.arma3_default_args = "-baz".into();

        let args = build_args(&profile, &settings, ActionKind::Launch, Vec::new()).unwrap();
        assert!(args.contains(&"-foo".to_string()));
        assert!(args.contains(&"-bar".to_string()));
        assert!(!args.contains(&"-baz".to_string()));
    }

    #[test]
    fn build_args_join_adds_connect_when_missing() {
        let mut profile = default_profile();
        profile.arma3_server = Some(ProfileServerInfo {
            address: "127.0.0.1".into(),
            port: 2302,
            password: "pw".into(),
        });

        let settings = default_settings();
        let args = build_args(&profile, &settings, ActionKind::Join, Vec::new()).unwrap();
        assert!(args.contains(&"-connect=127.0.0.1".to_string()));
        assert!(args.contains(&"-port=2302".to_string()));
        assert!(args.contains(&"-password=pw".to_string()));
    }

    #[test]
    fn build_args_join_respects_connect_override() {
        let mut profile = default_profile();
        profile.arma3_server = Some(ProfileServerInfo {
            address: "127.0.0.1".into(),
            port: 2302,
            password: "pw".into(),
        });

        let settings = default_settings();
        let extra = vec![OsString::from("-connect=1.2.3.4")];
        let args = build_args(&profile, &settings, ActionKind::Join, extra).unwrap();
        assert!(args.contains(&"-connect=1.2.3.4".to_string()));
        assert!(!args.contains(&"-connect=127.0.0.1".to_string()));
        assert!(!args.contains(&"-port=2302".to_string()));
        assert!(!args.contains(&"-password=pw".to_string()));
    }

    #[test]
    fn resolve_launch_mode_custom_base_matches_platform() {
        let mut settings = AppSettings::default();
        settings.arma3.arma3_launch_method = fleet_domain::Arma3LaunchMethod::Custom;
        settings.arma3.arma3_custom_launch_template = "arma3_x64.exe $ARGS $MODS".to_string();
        let resolved = resolve_launch_mode(&settings).unwrap();
        #[cfg(target_os = "windows")]
        assert_eq!(resolved.wrapper_method, LaunchMethod::Arma3Exe);
        #[cfg(not(target_os = "windows"))]
        assert_eq!(resolved.wrapper_method, LaunchMethod::SteamNative);
    }

    #[test]
    fn validate_launch_compatibility_rejects_missing_or_modified() {
        let err = validate_launch_compatibility(&ProfileAssessmentReport {
            profile_id: "p1".to_string(),
            local_health: LocalHealthState::MissingDestination,
            remote_freshness: RemoteFreshnessState::Unknown,
            checked_at_unix_ms: 0,
            expected_missing_in_inventory_count: 0,
            inventory_unexpected_paths_count: 0,
            unexpected_delete_paths: Vec::new(),
        })
        .expect_err("must reject incompatible");
        assert_eq!(err.code, "launch_incompatible");
    }
}
