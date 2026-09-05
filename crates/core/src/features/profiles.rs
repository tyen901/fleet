use crate::core::run_config_blocking;
use crate::state::{ensure_profile_runtime_mut, recompute_profile_status, AppState};
use crate::storage::{profile_state_root_dir, ProfilesConfig};
use crate::Core;
use fleet_domain::{validated_repo_url, Profile, ProfileId, RepoServer};
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};

#[derive(Debug)]
enum SaveProfileError {
    DestinationInUse { existing_profile_id: String },
    InvalidSource(anyhow::Error),
    Persistence(anyhow::Error),
}

impl std::fmt::Display for SaveProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DestinationInUse {
                existing_profile_id,
            } => write!(
                f,
                "destination already used by profile {}",
                existing_profile_id
            ),
            Self::InvalidSource(err) | Self::Persistence(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for SaveProfileError {}

impl Core {
    pub async fn list_profiles(&self) -> anyhow::Result<ProfilesConfig> {
        info!(op = "list_profiles", "profiles list requested");
        run_config_blocking(self.config_repo(), |c| c.load_profiles()).await
    }

    pub async fn load_profile(&self, profile_id: &ProfileId) -> anyhow::Result<Profile> {
        info!(op = "load_profile", profile_id = %profile_id, "profile load requested");
        if let Some(profile) = self.read_state(|state| state.profiles.get(profile_id).cloned()) {
            return Ok(profile);
        }

        let pid = profile_id.clone();
        let res = run_config_blocking(self.config_repo(), move |c| {
            let cfg = c.load_profiles()?;
            cfg.profiles
                .into_iter()
                .find(|p| p.id == pid)
                .ok_or_else(|| anyhow::anyhow!("unknown profile id: {pid}"))
        })
        .await;
        if res.is_err() {
            warn!(
                op = "load_profile",
                profile_id = %profile_id,
                outcome = "failed",
                reason = "unknown_profile_id",
                "profile load failed"
            );
        }
        res
    }

    async fn save_profile_with_semantics(
        &self,
        profile: Profile,
    ) -> Result<Profile, SaveProfileError> {
        let mut profile = profile;
        let requested_profile_id = profile.id.trim().to_string();
        info!(
            op = "save_profile",
            profile_id = if requested_profile_id.is_empty() {
                "new"
            } else {
                requested_profile_id.as_str()
            },
            "profile save requested"
        );
        profile.id = profile.id.trim().to_string();
        if profile.id.is_empty() {
            const ATTEMPTS: usize = 8;
            use rand::{distributions::Alphanumeric, Rng};
            let mut generated = None;
            for _ in 0..ATTEMPTS {
                let s: String = rand::thread_rng()
                    .sample_iter(&Alphanumeric)
                    .map(|c| c as char)
                    .filter(|c| c.is_ascii_alphanumeric())
                    .map(|c| c.to_ascii_lowercase())
                    .take(4)
                    .collect();

                let s_clone = s.clone();
                let exists = run_config_blocking(self.config_repo(), move |config| {
                    let cfg = config.load_profiles()?;
                    Ok::<_, anyhow::Error>(cfg.profiles.iter().any(|p| p.id == s_clone))
                })
                .await
                .map_err(SaveProfileError::Persistence)?;

                if !exists {
                    generated = Some(s);
                    break;
                }
            }

            if let Some(id) = generated {
                profile.id = id;
                debug!(
                    op = "save_profile",
                    profile_id = %profile.id,
                    outcome = "generated_id",
                    "generated profile id"
                );
            } else {
                error!(
                    op = "save_profile",
                    profile_id = "new",
                    outcome = "failed",
                    reason = "id_generation_failed",
                    "profile save failed while generating id"
                );
                return Err(SaveProfileError::Persistence(anyhow::anyhow!(
                    "failed to generate unique profile id"
                )));
            }
        }

        profile.destination = profile.destination.trim().to_string();
        profile.source = profile.source.trim().to_string();
        profile.additional_mod_folders =
            normalize_additional_mod_folders(&profile.additional_mod_folders);
        if let Err(err) = validated_repo_url(&profile.source) {
            error!(
                op = "save_profile",
                profile_id = %profile.id,
                outcome = "failed",
                reason = "invalid_repo_source",
                "profile save validation failed"
            );
            debug!(
                op = "save_profile",
                profile_id = %profile.id,
                error = %err,
                "profile source validation details"
            );
            return Err(SaveProfileError::InvalidSource(err));
        }

        let profile_id_for_log = profile.id.clone();
        let res = run_config_blocking(self.config_repo(), move |config| {
            let profile = profile;
            let mut cfg = config
                .load_profiles()
                .map_err(SaveProfileError::Persistence)?;

            let destination = normalize_destination_for_compare(&profile.destination);
            if !destination.is_empty() {
                for existing in cfg.profiles.iter() {
                    if existing.id != profile.id
                        && normalize_destination_for_compare(&existing.destination) == destination
                    {
                        warn!(
                            op = "save_profile",
                            profile_id = %profile.id,
                            outcome = "rejected",
                            reason = "destination_in_use",
                            "profile save rejected due to destination conflict"
                        );
                        return Ok(Err(SaveProfileError::DestinationInUse {
                            existing_profile_id: existing.id.clone(),
                        }));
                    }
                }
            }

            if let Some(idx) = cfg.profiles.iter().position(|p| p.id == profile.id) {
                cfg.profiles[idx] = profile.clone();
            } else {
                cfg.profiles.push(profile.clone());
            }
            config
                .save_profiles(&cfg)
                .map_err(SaveProfileError::Persistence)?;
            Ok::<_, anyhow::Error>(Ok(profile))
        })
        .await
        .map_err(SaveProfileError::Persistence)?;

        match res {
            Ok(saved) => {
                info!(
                    op = "save_profile",
                    profile_id = %saved.id,
                    outcome = "ok",
                    "profile save succeeded"
                );
                Ok(saved)
            }
            Err(err) => {
                error!(
                    op = "save_profile",
                    profile_id = %profile_id_for_log,
                    outcome = "failed",
                    reason = "config_write_failed",
                    "profile save failed"
                );
                debug!(
                    op = "save_profile",
                    profile_id = %profile_id_for_log,
                    error = %err,
                    "profile save error details"
                );
                Err(err)
            }
        }
    }

    pub async fn reset_profiles(&self) -> anyhow::Result<()> {
        run_config_blocking(self.config_repo(), |c| c.delete_profiles()).await
    }

    pub async fn profile_save(&self, profile: Profile) -> Result<Profile, crate::ApiError> {
        let previous = if profile.id.trim().is_empty() {
            None
        } else {
            self.load_profile(&profile.id.trim().to_string()).await.ok()
        };
        let profile_mutation = if previous
            .as_ref()
            .is_some_and(|previous| profile_path_context_changed(Some(previous), &profile))
        {
            Some(
                self.operation_runtime()
                    .reserve_profile_mutation(profile.id.clone())?,
            )
        } else {
            None
        };
        let requested_profile_id = profile.id.clone();
        let saved = self
            .save_profile_with_semantics(profile.clone())
            .await
            .map_err(|e| {
            let api_err = map_profile_save_error(&e);
            error!(
                op = "profile_save",
                profile_id = if requested_profile_id.trim().is_empty() { "new" } else { requested_profile_id.as_str() },
                outcome = "failed",
                code = %api_err.code,
                reason = "save_profile_failed",
                "profile save API failed"
            );
            debug!(
                op = "profile_save",
                profile_id = if requested_profile_id.trim().is_empty() { "new" } else { requested_profile_id.as_str() },
                error = %e,
                "profile save API error details"
            );
            api_err
        })?;

        let path_context_changed = profile_path_context_changed(previous.as_ref(), &saved);

        self.update_state(|state| {
            let pid = saved.id.clone();
            state.profiles.insert(pid.clone(), saved.clone());
            let now = fleet_domain::time::now_unix_ms();
            if path_context_changed {
                clear_profile_check_state(state, &pid, now);
            } else {
                let _ = ensure_profile_runtime_mut(state, &pid, now);
            }
            set_profile_repo_servers_runtime(state, &pid, Vec::new());
            recompute_profile_status(state, &pid);
        });
        self.spawn_profile_repo_cache_refresh(saved.id.clone());
        drop(profile_mutation);
        if path_context_changed {
            let core = self.clone();
            let profile_id = saved.id.clone();
            tokio::spawn(async move {
                let check = core
                    .start_operation(profile_id, fleet_domain::health::OperationKind::Check)
                    .await;
                if let Ok(session_id) = check {
                    let _ = core.await_finished(session_id).await;
                }
            });
        }
        info!(
            op = "profile_save",
            profile_id = %saved.id,
            outcome = "ok",
            "profile saved and state updated"
        );

        Ok(saved)
    }

    pub async fn profile_delete(&self, profile_id: ProfileId) -> Result<(), crate::ApiError> {
        info!(
            op = "profile_delete",
            profile_id = %profile_id,
            "profile delete API requested"
        );
        let _mutation = self
            .operation_runtime()
            .reserve_profile_mutation(profile_id.clone())?;
        let pid = profile_id.clone();
        run_config_blocking(self.config_repo(), move |config| {
            let mut cfg = config.load_profiles()?;
            cfg.profiles.retain(|profile| profile.id != pid);
            config.save_profiles(&cfg)
        })
        .await
        .map_err(|error| crate::ApiError::new("profile_delete", error.to_string()))?;

        self.update_state(|state| {
            state.profiles.remove(&profile_id);
            state.profile_runtime_by_id.remove(&profile_id);
        });
        info!(
            op = "profile_delete",
            profile_id = %profile_id,
            outcome = "ok",
            "profile deleted and state updated"
        );

        Ok(())
    }

    pub(crate) fn spawn_profile_repo_cache_refresh(&self, profile_id: ProfileId) {
        let core = self.clone();
        tokio::spawn(async move {
            if let Err(err) = core.refresh_profile_repo_cache(profile_id.clone()).await {
                warn!(
                    profile_id = %profile_id,
                    code = %err.code,
                    message = %err.message,
                    "failed to refresh profile repo cache state"
                );
            }
        });
    }

    pub(crate) async fn refresh_profile_repo_cache(
        &self,
        profile_id: ProfileId,
    ) -> Result<(), crate::ApiError> {
        let profile = if let Some(profile) =
            self.read_state(|state| state.profiles.get(&profile_id).cloned())
        {
            profile
        } else {
            return Ok(());
        };

        let servers = load_cached_repo_servers(&profile)
            .await?
            .unwrap_or_default();

        let servers_for_state = servers.clone();
        self.update_state(|state| {
            set_profile_repo_servers_runtime(state, &profile_id, servers_for_state);
        });

        Ok(())
    }
}
pub(crate) fn load_cached_repo_servers_blocking(
    profile: &Profile,
) -> Result<Option<Vec<RepoServer>>, crate::ApiError> {
    let (cache_root, repo_url) = swifty_cache_target(profile)?;

    let Some(cache) = swifty_repo::load_cached_repo_blocking(&cache_root, &repo_url)
        .map_err(|e| crate::ApiError::new("swifty_cache", e.to_string()))?
    else {
        return Ok(None);
    };

    let servers = cache
        .repo
        .servers
        .into_iter()
        .map(|server| RepoServer {
            address: server.address,
            port: server.port,
            password: server.password,
        })
        .collect::<Vec<_>>();

    Ok(Some(servers))
}

pub(crate) async fn load_cached_repo_servers(
    profile: &Profile,
) -> Result<Option<Vec<RepoServer>>, crate::ApiError> {
    let profile = profile.clone();
    tokio::task::spawn_blocking(move || load_cached_repo_servers_blocking(&profile))
        .await
        .map_err(|e| crate::ApiError::new("swifty_cache", e.to_string()))?
}

pub(crate) fn set_profile_repo_servers_runtime(
    state: &mut AppState,
    profile_id: &str,
    servers: Vec<RepoServer>,
) {
    let now_ms = fleet_domain::time::now_unix_ms();
    let runtime = ensure_profile_runtime_mut(state, profile_id, now_ms);
    runtime.repo_servers = servers;
}

fn swifty_cache_target(profile: &Profile) -> Result<(PathBuf, String), crate::ApiError> {
    profile
        .dest_path()
        .map_err(|e| crate::ApiError::new("invalid_profile", e.to_string()))?;
    let repo_url = validated_repo_url(&profile.source)
        .map_err(|e| crate::ApiError::new("invalid_profile", e.to_string()))?;
    let state_root =
        profile_state_root_dir().map_err(|e| crate::ApiError::new("state_root", e.to_string()))?;
    Ok((
        fleet_domain::repo_cache_dir(&state_root, &profile.id),
        repo_url.to_string(),
    ))
}

pub fn validate_profile_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == ' ')
}

pub fn is_destination_unique(state: &AppState, destination: &str, ignore_id: Option<&str>) -> bool {
    let destination = normalize_destination_for_compare(destination);
    if destination.is_empty() {
        return false;
    }

    !state.profiles.values().any(|profile| {
        if let Some(ignore) = ignore_id {
            if profile.id == ignore {
                return false;
            }
        }
        normalize_destination_for_compare(&profile.destination) == destination
    })
}

fn profile_path_context_changed(previous: Option<&Profile>, next: &Profile) -> bool {
    let Some(previous) = previous else {
        return true;
    };

    normalize_destination_for_compare(&previous.destination)
        != normalize_destination_for_compare(&next.destination)
        || normalize_source_for_compare(&previous.source)
            != normalize_source_for_compare(&next.source)
}

fn normalize_destination_for_compare(value: &str) -> String {
    let value = value.trim();
    #[cfg(windows)]
    let value = value.replace('\\', "/");
    #[cfg(not(windows))]
    let value = value.to_string();

    let root = Path::new(&value).has_root() && Path::new(&value).parent().is_none();
    let value = value.trim_end_matches('/');
    let value = if root {
        if value.is_empty() {
            "/".to_string()
        } else {
            format!("{value}/")
        }
    } else {
        value.to_string()
    };

    #[cfg(windows)]
    {
        value.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        value
    }
}

fn normalize_source_for_compare(value: &str) -> String {
    value.trim().to_string()
}

// Additional mod folders are opaque directory values appended to the launch mod
// list as-is; Fleet does not interpret or resolve them.
fn normalize_additional_mod_folders(paths: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for path in paths {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = trimmed.to_string();
        if !out.contains(&normalized) {
            out.push(normalized);
        }
    }
    out
}

fn clear_profile_check_state(state: &mut AppState, profile_id: &str, now_ms: u64) {
    let runtime = ensure_profile_runtime_mut(state, profile_id, now_ms);
    runtime.repo_check = None;
    runtime.check = None;
    runtime.validation = None;
    runtime.materialization = None;
    runtime.last_operation = None;
    recompute_profile_status(state, profile_id);
}

fn map_profile_save_error(err: &SaveProfileError) -> crate::ApiError {
    match err {
        SaveProfileError::DestinationInUse {
            existing_profile_id,
        } => crate::ApiError::new(
            "destination_in_use",
            format!(
                "destination already used by profile {}",
                existing_profile_id
            ),
        ),
        SaveProfileError::InvalidSource(source_err) => {
            crate::ApiError::new("error", source_err.to_string())
        }
        SaveProfileError::Persistence(persistence_err) => {
            crate::ApiError::new("error", persistence_err.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        clear_profile_check_state, normalize_additional_mod_folders,
        normalize_destination_for_compare, profile_path_context_changed,
    };
    use crate::state::AppState;
    use crate::test_support::{EnvVarGuard, ENV_VAR_LOCK};
    use crate::Core;
    use fleet_domain::health::{
        LocalFileHealth, LocalFileReport, RepoCheckFreshness, RepoCheckReport,
    };
    use fleet_domain::Profile;

    fn sample_profile(id: &str, destination: &str) -> Profile {
        Profile {
            id: id.to_string(),
            name: "Profile".to_string(),
            source: "https://example.com/repo.json".to_string(),
            destination: destination.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn destination_normalization_ignores_whitespace_and_trailing_separators() {
        assert_eq!(
            normalize_destination_for_compare("  /tmp/fleet/mods/// "),
            normalize_destination_for_compare("/tmp/fleet/mods")
        );
    }

    #[test]
    fn destination_normalization_preserves_roots() {
        assert_eq!(
            normalize_destination_for_compare("////"),
            normalize_destination_for_compare("/")
        );
    }

    #[cfg(windows)]
    #[test]
    fn destination_normalization_uses_windows_path_rules() {
        assert_eq!(
            normalize_destination_for_compare(" C:\\Fleet\\Mods\\ "),
            normalize_destination_for_compare("c:/fleet/mods///")
        );
        assert_eq!(
            normalize_destination_for_compare("C:\\\\\\\\"),
            normalize_destination_for_compare("c:/")
        );
        assert_ne!(
            normalize_destination_for_compare("C:"),
            normalize_destination_for_compare("C:/")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn destination_normalization_keeps_non_windows_paths_case_sensitive() {
        assert_ne!(
            normalize_destination_for_compare("/tmp/Fleet/Mods"),
            normalize_destination_for_compare("/tmp/fleet/mods")
        );
        assert_eq!(normalize_destination_for_compare("mods\\"), "mods\\");
    }

    #[test]
    fn additional_mod_folders_are_trimmed_and_deduped() {
        let paths = vec![
            " @ace ".to_string(),
            String::new(),
            "@ace".to_string(),
            "@acre2".to_string(),
        ];

        let normalized = normalize_additional_mod_folders(&paths);

        assert_eq!(normalized, vec!["@ace".to_string(), "@acre2".to_string()]);
    }

    #[test]
    fn path_context_change_detects_destination_and_source_updates() {
        let previous = Profile {
            id: "p1".to_string(),
            name: "Profile".to_string(),
            source: "https://example.com/repo.json".to_string(),
            destination: "/tmp/mods".to_string(),
            ..Default::default()
        };

        let unchanged = Profile {
            destination: if cfg!(windows) {
                "/TMP/mods///".to_string()
            } else {
                "/tmp/mods///".to_string()
            },
            source: "  https://example.com/repo.json ".to_string(),
            ..previous.clone()
        };
        assert!(!profile_path_context_changed(Some(&previous), &unchanged));

        let changed_destination = Profile {
            destination: "/tmp/mods-new".to_string(),
            ..previous.clone()
        };
        assert!(profile_path_context_changed(
            Some(&previous),
            &changed_destination
        ));

        let changed_source = Profile {
            source: "https://example.com/repo-v2.json".to_string(),
            ..previous
        };
        assert!(profile_path_context_changed(
            Some(&unchanged),
            &changed_source
        ));
    }

    #[test]
    fn user_story_profile_path_change_discards_all_stale_local_evidence() {
        let mut state = AppState::default();
        let profile_id = "p1".to_string();

        state.profiles.insert(
            profile_id.clone(),
            Profile {
                id: profile_id.clone(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/mods".to_string(),
                ..Default::default()
            },
        );
        state.profile_runtime_by_id.insert(
            profile_id.clone(),
            crate::state::ProfileRuntimeState {
                profile_id: profile_id.clone(),
                repo_check: Some(RepoCheckReport {
                    profile_id: profile_id.clone(),
                    local_revision: Some("local".to_string()),
                    remote_revision: None,
                    freshness: RepoCheckFreshness::Unknown,
                    checked_at_unix_ms: 10,
                }),
                check: Some(LocalFileReport {
                    profile_id: profile_id.clone(),
                    verification: fleet_domain::VerificationKind::Fast,
                    health: LocalFileHealth::RequiresSync,
                    checked_at_unix_ms: 10,
                }),
                validation: None,
                materialization: None,
                active: Some(crate::state::ActiveOperationState::new(
                    7,
                    fleet_domain::health::OperationKind::Check,
                    10,
                )),
                last_operation: None,
                repo_servers: Vec::new(),
                status: crate::state::ProfileStatusState::unknown(10),
            },
        );

        clear_profile_check_state(&mut state, &profile_id, 42);

        let updated = state
            .profile_runtime_by_id
            .get(&profile_id)
            .expect("profile runtime");
        assert!(updated.repo_check.is_none());
        assert!(updated.check.is_none());
        assert!(updated.validation.is_none());
        assert!(updated.materialization.is_none());
        assert!(updated.active.is_some());
        assert_eq!(updated.status.last_check_ms, 0);
    }

    #[test]
    fn user_story_profile_path_change_is_rejected_while_an_operation_owns_the_profile() {
        let _guard = ENV_VAR_LOCK.lock().expect("env lock");
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let _env = EnvVarGuard::set_path("FLEET_CONFIG_DIR", temp_dir.path());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        runtime.block_on(async {
            let core = Core::new_for_test().expect("core");
            let profile = sample_profile("p1", "/tmp/p1");
            core.update_state(|state| {
                state.profiles.insert(profile.id.clone(), profile.clone());
            });
            let _operation = core
                .operation_runtime()
                .reserve_profile_mutation(profile.id.clone())
                .expect("reserve active profile");
            let changed = Profile {
                destination: "/tmp/p1-new".to_string(),
                ..profile
            };

            let error = core
                .profile_save(changed)
                .await
                .expect_err("active profile edit must fail");
            assert_eq!(error.code, "profile_busy");
        });
    }

    #[test]
    fn load_profile_prefers_loaded_state_before_config_reload() {
        let _guard = ENV_VAR_LOCK.lock().expect("env lock");

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let _env = EnvVarGuard::set_path("FLEET_CONFIG_DIR", temp_dir.path());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        runtime.block_on(async {
            let core = Core::new_for_test().expect("core");
            let profile = sample_profile("p1", "/tmp/p1");
            core.update_state(|state| {
                state.profiles.insert(profile.id.clone(), profile.clone());
            });

            let loaded = core
                .load_profile(&"p1".to_string())
                .await
                .expect("load profile");
            assert_eq!(loaded, profile);
        });
    }

    #[test]
    fn deleting_profile_removes_profile_from_config() {
        let _guard = ENV_VAR_LOCK.lock().expect("env lock");

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let _env = EnvVarGuard::set_path("FLEET_CONFIG_DIR", temp_dir.path());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        runtime.block_on(async {
            let core = Core::spawn_threaded_default().expect("core");
            let profile = sample_profile("p1", "/tmp/p1");
            core.profile_save(profile).await.expect("save profile");
            core.profile_delete("p1".to_string())
                .await
                .expect("delete profile");

            let cfg = core.list_profiles().await.expect("list profiles");
            assert!(cfg.profiles.is_empty());
        });
    }

    #[test]
    fn profile_save_destination_conflict_returns_destination_in_use() {
        let _guard = ENV_VAR_LOCK.lock().expect("env lock");

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let _env = EnvVarGuard::set_path("FLEET_CONFIG_DIR", temp_dir.path());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        runtime.block_on(async {
            let core = Core::spawn_threaded_default().expect("core");
            core.profile_save(sample_profile(
                "p1",
                if cfg!(windows) {
                    "C:\\Fleet\\Shared\\"
                } else {
                    "/tmp/shared/"
                },
            ))
            .await
            .expect("save initial profile");

            let err = core
                .profile_save(sample_profile(
                    "p2",
                    if cfg!(windows) {
                        " c:/fleet/shared/// "
                    } else {
                        " /tmp/shared/// "
                    },
                ))
                .await
                .expect_err("destination conflict should fail");

            assert_eq!(err.code, "destination_in_use");
            assert_eq!(err.message, "destination already used by profile p1");
        });
    }
}
