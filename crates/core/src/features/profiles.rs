use crate::core::run_config_blocking;
use crate::state::AppState;
use crate::storage::{profile_state_root_dir, ProfilesConfig};
use crate::Core;
use fleet_domain::{Profile, ProfileId, ProfileSourceKind};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ProfileSaveAndReassessResult {
    pub profile: Profile,
    #[serde(default)]
    pub reassess_warning: Option<String>,
}

impl Core {
    pub async fn list_profiles(&self) -> anyhow::Result<ProfilesConfig> {
        run_config_blocking(self.config_repo(), |c| c.load_profiles()).await
    }

    pub async fn load_profile(&self, profile_id: &ProfileId) -> anyhow::Result<Profile> {
        let pid = profile_id.clone();
        run_config_blocking(self.config_repo(), move |c| {
            let cfg = c.load_profiles()?;
            cfg.profiles
                .into_iter()
                .find(|p| p.id == pid)
                .ok_or_else(|| anyhow::anyhow!("unknown profile id: {pid}"))
        })
        .await
    }

    pub async fn save_profile(&self, profile: Profile) -> anyhow::Result<Profile> {
        let mut profile = profile;
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
                .await?;

                if !exists {
                    generated = Some(s);
                    break;
                }
            }

            if let Some(id) = generated {
                profile.id = id;
            } else {
                anyhow::bail!("failed to generate unique profile id");
            }
        }

        profile.destination = profile.destination.trim().to_string();
        profile.source = profile.source.trim().to_string();
        profile.validated_source_kind()?;

        let profile = run_config_blocking(self.config_repo(), move |config| {
            let mut cfg = config.load_profiles()?;

            let normalize_path = |path: &str| {
                let trimmed = path.trim();
                trimmed.trim_end_matches(['/', '\\']).to_string()
            };

            let destination = normalize_path(&profile.destination);
            if !destination.is_empty() {
                for existing in cfg.profiles.iter() {
                    if existing.id != profile.id
                        && normalize_path(&existing.destination) == destination
                    {
                        anyhow::bail!(
                            "destination_in_use: destination already used by profile {}",
                            existing.id
                        );
                    }
                }
            }

            if let Some(idx) = cfg.profiles.iter().position(|p| p.id == profile.id) {
                cfg.profiles[idx] = profile.clone();
            } else {
                cfg.profiles.push(profile.clone());
            }
            config.save_profiles(&cfg)?;
            Ok::<_, anyhow::Error>(profile)
        })
        .await?;

        Ok(profile)
    }

    pub async fn delete_profile(&self, profile_id: &ProfileId) -> anyhow::Result<()> {
        let pid = profile_id.clone();
        run_config_blocking(self.config_repo(), move |config| {
            let mut cfg = config.load_profiles()?;
            cfg.profiles.retain(|p| p.id != pid);
            config.save_profiles(&cfg)?;
            Ok::<_, anyhow::Error>(())
        })
        .await
    }

    pub async fn reset_profiles(&self) -> anyhow::Result<()> {
        run_config_blocking(self.config_repo(), |c| c.delete_profiles()).await
    }

    pub async fn profile_repo_servers(
        &self,
        profile_id: &ProfileId,
    ) -> Result<Vec<fleet_domain::RepoServer>, crate::ApiError> {
        let profile = self
            .load_profile(profile_id)
            .await
            .map_err(|e| crate::ApiError::new("not_found", e.to_string()))?;
        profile_repo_servers(&profile).await
    }

    pub async fn profile_save(&self, profile: Profile) -> Result<Profile, crate::ApiError> {
        let saved = self.save_profile(profile.clone()).await.map_err(|e| {
            let msg = e.to_string();
            if let Some(rest) = msg.strip_prefix("destination_in_use:") {
                crate::ApiError::new("destination_in_use", rest.trim().to_string())
            } else {
                crate::ApiError::new("error", msg)
            }
        })?;

        self.update_state(|state| {
            let pid = saved.id.clone();
            state.profiles.insert(pid.clone(), saved.clone());
            state.profile_states.entry(pid.clone()).or_insert_with(|| {
                crate::state::ProfileState::new(pid, fleet_domain::time::now_unix_ms())
            });
        });

        Ok(saved)
    }

    pub async fn profile_save_and_reassess(
        &self,
        profile: Profile,
    ) -> Result<ProfileSaveAndReassessResult, crate::ApiError> {
        let previous = if profile.id.trim().is_empty() {
            None
        } else {
            self.load_profile(&profile.id.trim().to_string()).await.ok()
        };

        let saved = self.profile_save(profile).await?;
        let path_context_changed = profile_path_context_changed(previous.as_ref(), &saved);

        let mut reassess_warning = None;
        if path_context_changed {
            let now = fleet_domain::time::now_unix_ms();
            let pid = saved.id.clone();
            self.update_state(|state| {
                clear_profile_check_state(state, &pid, now);
            });

            if let Err(err) = self.start_check_local(saved.id.clone()).await {
                tracing::warn!(
                    profile_id = %saved.id,
                    code = %err.code,
                    message = %err.message,
                    "failed to start local health re-check after profile save"
                );
                reassess_warning =
                    Some("Health re-check could not start. Use Retry Check.".to_string());
            }
        }

        Ok(ProfileSaveAndReassessResult {
            profile: saved,
            reassess_warning,
        })
    }

    pub async fn profile_delete(&self, profile_id: ProfileId) -> Result<(), crate::ApiError> {
        self.delete_profile(&profile_id)
            .await
            .map_err(|e| crate::ApiError::new("error", e.to_string()))?;

        self.update_state(|state| {
            state.profiles.remove(&profile_id);
            state.profile_states.remove(&profile_id);
            if let Some(sync) = &state.sync {
                if sync.profile_id == profile_id {
                    state.sync = None;
                }
            }
        });

        Ok(())
    }
}

async fn profile_repo_servers(
    profile: &Profile,
) -> Result<Vec<fleet_domain::RepoServer>, crate::ApiError> {
    let Some((cache_root, repo_url)) = swifty_cache_target(profile)? else {
        return Ok(Vec::new());
    };

    let servers = swifty_repo::cached_repo_servers(&cache_root, &repo_url)
        .await
        .map_err(|e| crate::ApiError::new("swifty_cache", e.to_string()))?;

    Ok(servers
        .unwrap_or_default()
        .into_iter()
        .map(|s| fleet_domain::RepoServer {
            address: s.address,
            port: s.port,
            password: s.password,
        })
        .collect())
}

fn swifty_cache_target(profile: &Profile) -> Result<Option<(PathBuf, String)>, crate::ApiError> {
    profile
        .dest_path()
        .map_err(|e| crate::ApiError::new("invalid_profile", e.to_string()))?;
    let ProfileSourceKind::Http(repo_url) = profile.source_kind();
    let state_root =
        profile_state_root_dir().map_err(|e| crate::ApiError::new("state_root", e.to_string()))?;
    Ok(Some((
        fleet_domain::repo_cache_dir(&state_root, &profile.id),
        repo_url.to_string(),
    )))
}

pub fn apply_profile_save_to_state(
    state: &AppState,
    active_id: Option<String>,
    saved: Profile,
) -> (AppState, Option<String>) {
    let mut next_state = state.clone();
    next_state.profiles.insert(saved.id.clone(), saved.clone());

    let next_active = match active_id {
        Some(current) if next_state.profiles.contains_key(&current) => Some(current),
        _ => Some(saved.id),
    };

    (next_state, next_active)
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

pub fn validate_repo_url(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://")) && lower.ends_with("repo.json")
}

pub fn is_destination_unique(state: &AppState, destination: &str, ignore_id: Option<&str>) -> bool {
    let trimmed = destination.trim();
    if trimmed.is_empty() {
        return false;
    }
    let dest = trimmed.to_ascii_lowercase();

    !state.profiles.values().any(|profile| {
        if let Some(ignore) = ignore_id {
            if profile.id == ignore {
                return false;
            }
        }
        profile.destination.trim().to_ascii_lowercase() == dest
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
    value
        .trim()
        .trim_end_matches(['/', '\\'])
        .to_ascii_lowercase()
}

fn normalize_source_for_compare(value: &str) -> String {
    value.trim().to_string()
}

fn clear_profile_check_state(state: &mut AppState, profile_id: &str, now_ms: u64) {
    let v = state
        .profile_states
        .entry(profile_id.to_string())
        .or_insert_with(|| crate::state::ProfileState::new(profile_id.to_string(), now_ms));
    v.assessment = None;
    v.error = None;
    v.active_operation = None;
    v.last_checked_ms = now_ms;
}

#[cfg(test)]
mod tests {
    use super::{
        clear_profile_check_state, normalize_destination_for_compare, profile_path_context_changed,
    };
    use crate::state::AppState;
    use fleet_domain::health::{
        LocalHealthState, OperationKind, ProfileAssessmentReport, RemoteFreshnessState,
    };
    use fleet_domain::{ApiError, Profile};

    #[test]
    fn destination_normalization_ignores_case_whitespace_and_trailing_slashes() {
        assert_eq!(
            normalize_destination_for_compare("  /Tmp/Fleet/Mods/// "),
            normalize_destination_for_compare("/tmp/fleet/mods")
        );
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
            destination: "/TMP/mods///".to_string(),
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
    fn clear_profile_check_state_removes_stale_assessment_and_error() {
        let mut state = AppState::default();
        let profile_id = "p1".to_string();

        state.profile_states.insert(
            profile_id.clone(),
            crate::state::ProfileState {
                profile_id: profile_id.clone(),
                assessment: Some(ProfileAssessmentReport {
                    profile_id: profile_id.clone(),
                    local_health: LocalHealthState::Error,
                    remote_freshness: RemoteFreshnessState::Unknown,
                    checked_at_unix_ms: 10,
                }),
                last_checked_ms: 10,
                active_operation: Some(OperationKind::Checking),
                error: Some(ApiError::new("check_failed", "symlink path rejected")),
            },
        );

        clear_profile_check_state(&mut state, &profile_id, 42);

        let updated = state
            .profile_states
            .get(&profile_id)
            .expect("profile state");
        assert!(updated.assessment.is_none());
        assert!(updated.error.is_none());
        assert!(updated.active_operation.is_none());
        assert_eq!(updated.last_checked_ms, 42);
    }
}
