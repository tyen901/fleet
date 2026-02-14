use crate::core::run_config_blocking;
use crate::state::AppState;
use crate::storage::{profile_state_root_dir, ProfilesConfig};
use crate::Core;
use fleet_domain::{Profile, ProfileId, ProfileSourceKind};
use std::path::PathBuf;

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
