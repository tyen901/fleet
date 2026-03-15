use crate::core::run_config_blocking;
use crate::state::{ensure_profile_runtime_mut, recompute_profile_status, AppState};
use crate::storage::{profile_state_root_dir, ProfilesConfig};
use crate::Core;
use fleet_domain::LocalStateMetrics;
use fleet_domain::{Profile, ProfileId, ProfileSourceKind, RepoServer};
use fleet_inventory::Inventory;
use std::path::PathBuf;
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

    pub async fn profile_set_selected(
        &self,
        profile_id: Option<ProfileId>,
    ) -> Result<(), crate::ApiError> {
        if let Some(ref requested_id) = profile_id {
            let exists = self.read_state(|state| state.profiles.contains_key(requested_id));
            if !exists {
                return Err(crate::ApiError::new("not_found", requested_id.clone()));
            }
        }

        self.update_state(|state| {
            state.selected_profile_id = profile_id.clone();
        });
        Ok(())
    }

    pub async fn save_profile(&self, profile: Profile) -> anyhow::Result<Profile> {
        self.save_profile_with_semantics(profile)
            .await
            .map_err(anyhow::Error::new)
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
        if let Err(err) = profile.validated_source_kind() {
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
            let mut profile = profile;
            let mut cfg = config
                .load_profiles()
                .map_err(SaveProfileError::Persistence)?;

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
                if normalize_source_for_compare(&cfg.profiles[idx].source)
                    != normalize_source_for_compare(&profile.source)
                {
                    // Source changed; force fresh cache revision baseline for the new repo URL.
                    profile.swifty_repo_revision = String::new();
                }
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

    pub async fn delete_profile(&self, profile_id: &ProfileId) -> anyhow::Result<()> {
        info!(
            op = "delete_profile",
            profile_id = %profile_id,
            "profile delete requested"
        );
        let pid = profile_id.clone();
        let res = run_config_blocking(self.config_repo(), move |config| {
            let mut cfg = config.load_profiles()?;
            cfg.profiles.retain(|p| p.id != pid);
            config.save_profiles(&cfg)?;
            Ok::<_, anyhow::Error>(())
        })
        .await;
        if res.is_ok() {
            info!(
                op = "delete_profile",
                profile_id = %profile_id,
                outcome = "ok",
                "profile delete succeeded"
            );
        } else {
            error!(
                op = "delete_profile",
                profile_id = %profile_id,
                outcome = "failed",
                reason = "config_write_failed",
                "profile delete failed"
            );
        }
        res
    }

    pub async fn reset_profiles(&self) -> anyhow::Result<()> {
        run_config_blocking(self.config_repo(), |c| c.delete_profiles()).await
    }

    pub async fn profile_inventory_metrics(
        &self,
        profile_id: &ProfileId,
    ) -> Result<LocalStateMetrics, crate::ApiError> {
        let profile = self
            .load_profile(profile_id)
            .await
            .map_err(|e| crate::ApiError::new("not_found", e.to_string()))?;

        tokio::task::spawn_blocking(move || load_profile_inventory_metrics(&profile))
            .await
            .map_err(|e| crate::ApiError::new("inventory_metrics", e.to_string()))?
    }

    pub async fn profile_save(&self, profile: Profile) -> Result<Profile, crate::ApiError> {
        let previous = if profile.id.trim().is_empty() {
            None
        } else {
            self.load_profile(&profile.id.trim().to_string()).await.ok()
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
            set_profile_repo_servers_runtime(state, &pid, Vec::new(), false);
            recompute_profile_status(state, &pid);
        });
        self.spawn_profile_repo_cache_refresh(saved.id.clone(), false);
        if path_context_changed {
            let core = self.clone();
            let profile_id = saved.id.clone();
            tokio::spawn(async move {
                let repo_session = core
                    .start_operation(
                        profile_id.clone(),
                        fleet_domain::health::OperationKind::CheckRepo,
                    )
                    .await;
                if let Ok(session_id) = repo_session {
                    let _ = core.await_finished(session_id).await;
                }
                let _ = core
                    .start_operation(
                        profile_id,
                        fleet_domain::health::OperationKind::CheckInventory,
                    )
                    .await;
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
        self.delete_profile(&profile_id)
            .await
            .map_err(|e| crate::ApiError::new("error", e.to_string()))?;

        self.update_state(|state| {
            state.profiles.remove(&profile_id);
            state.profile_runtime_by_id.remove(&profile_id);
            if state
                .selected_profile_id
                .as_ref()
                .is_some_and(|selected_id| selected_id == &profile_id)
            {
                state.selected_profile_id = None;
            }
        });
        info!(
            op = "profile_delete",
            profile_id = %profile_id,
            outcome = "ok",
            "profile deleted and state updated"
        );

        Ok(())
    }

    pub(crate) fn spawn_profile_repo_cache_refresh(
        &self,
        profile_id: ProfileId,
        allow_server_reconcile: bool,
    ) {
        let core = self.clone();
        tokio::spawn(async move {
            if let Err(err) = core
                .refresh_profile_repo_cache(profile_id.clone(), allow_server_reconcile)
                .await
            {
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
        allow_server_reconcile: bool,
    ) -> Result<(), crate::ApiError> {
        let profile = if let Some(profile) =
            self.read_state(|state| state.profiles.get(&profile_id).cloned())
        {
            profile
        } else {
            return Ok(());
        };

        let snapshot = load_cached_repo_server_snapshot(&profile).await?;
        let (servers, revision) = if let Some(snapshot) = snapshot {
            (snapshot.servers, snapshot.revision)
        } else {
            (Vec::new(), None)
        };

        let servers_for_state = servers.clone();
        self.update_state(|state| {
            set_profile_repo_servers_runtime(state, &profile_id, servers_for_state, true);
        });

        self.reconcile_profile_server_from_cache_revision(
            &profile_id,
            revision,
            &servers,
            allow_server_reconcile,
        )
        .await?;
        Ok(())
    }

    async fn reconcile_profile_server_from_cache_revision(
        &self,
        profile_id: &str,
        cache_revision: Option<String>,
        servers: &[RepoServer],
        allow_server_reconcile: bool,
    ) -> Result<(), crate::ApiError> {
        // Keep startup/background cache reads side-effect free for profile storage.
        // Persisted server/revision changes are only allowed on explicit reconcile paths
        // (triggered after operations that can update repo cache on disk).
        if !allow_server_reconcile {
            return Ok(());
        }

        let Some(cache_revision) = cache_revision.filter(|value| !value.trim().is_empty()) else {
            return Ok(());
        };
        let profile_id_owned = profile_id.to_string();
        let cache_revision_owned = cache_revision.clone();
        let servers_owned = servers.to_vec();

        let updated_profile = run_config_blocking(self.config_repo(), move |config| {
            let mut cfg = config.load_profiles()?;
            let Some(idx) = cfg
                .profiles
                .iter()
                .position(|profile| profile.id == profile_id_owned)
            else {
                return Ok::<_, anyhow::Error>(None);
            };

            let mut next = cfg.profiles[idx].clone();
            let previous_revision = next.swifty_repo_revision.trim().to_string();
            if previous_revision == cache_revision_owned {
                return Ok(None);
            }

            next.arma3_server = canonical_profile_server(next.arma3_server.clone(), &servers_owned);
            next.swifty_repo_revision = cache_revision_owned.clone();

            cfg.profiles[idx] = next.clone();
            config.save_profiles(&cfg)?;
            Ok(Some(next))
        })
        .await
        .map_err(|e| crate::ApiError::new("profile_reconcile", e.to_string()))?;

        if let Some(updated) = updated_profile {
            self.update_state(|state| {
                state.profiles.insert(updated.id.clone(), updated);
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CachedRepoServerSnapshot {
    pub servers: Vec<RepoServer>,
    pub revision: Option<String>,
}

fn canonical_profile_server(
    saved: Option<fleet_domain::types::ProfileServerInfo>,
    servers: &[RepoServer],
) -> Option<fleet_domain::types::ProfileServerInfo> {
    if let Some(saved_server) = saved {
        let exists_in_cache = servers.iter().any(|server| {
            server.address.trim() == saved_server.address.trim() && server.port == saved_server.port
        });
        if exists_in_cache {
            return Some(saved_server);
        }
    }

    servers
        .first()
        .map(|server| fleet_domain::types::ProfileServerInfo {
            address: server.address.clone(),
            port: server.port,
            password: server.password.clone(),
        })
}

fn cached_repo_revision(cache: &swifty_repo::RepoCacheBlob) -> Option<String> {
    if let Some(revision) = cache
        .repo_json_checksum
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(revision.to_string());
    }

    let checksum = cache.repo.checksum.trim();
    if !checksum.is_empty() {
        return Some(checksum.to_string());
    }

    // Some legacy cache blobs may not have repo_json_checksum/repo.checksum.
    // Fall back to a deterministic server fingerprint so reconciliation still works.
    let mut server_fingerprint = cache
        .repo
        .servers
        .iter()
        .map(|server| {
            format!(
                "{}:{}:{}",
                server.address.trim().to_ascii_lowercase(),
                server.port,
                server.password.trim()
            )
        })
        .collect::<Vec<_>>();
    server_fingerprint.sort();
    Some(format!("servers:{}", server_fingerprint.join("|")))
}

pub(crate) fn load_cached_repo_server_snapshot_blocking(
    profile: &Profile,
) -> Result<Option<CachedRepoServerSnapshot>, crate::ApiError> {
    let Some((cache_root, repo_url)) = swifty_cache_target(profile)? else {
        return Ok(None);
    };

    let Some(cache) = swifty_repo::load_cached_repo_blocking(&cache_root, &repo_url)
        .map_err(|e| crate::ApiError::new("swifty_cache", e.to_string()))?
    else {
        return Ok(None);
    };

    let revision = cached_repo_revision(&cache);
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

    Ok(Some(CachedRepoServerSnapshot { servers, revision }))
}

pub(crate) async fn load_cached_repo_server_snapshot(
    profile: &Profile,
) -> Result<Option<CachedRepoServerSnapshot>, crate::ApiError> {
    let profile = profile.clone();
    tokio::task::spawn_blocking(move || load_cached_repo_server_snapshot_blocking(&profile))
        .await
        .map_err(|e| crate::ApiError::new("swifty_cache", e.to_string()))?
}

pub(crate) fn set_profile_repo_servers_runtime(
    state: &mut AppState,
    profile_id: &str,
    servers: Vec<RepoServer>,
    loaded: bool,
) {
    let now_ms = fleet_domain::time::now_unix_ms();
    let runtime = ensure_profile_runtime_mut(state, profile_id, now_ms);
    runtime.repo_servers = servers;
    runtime.repo_servers_loaded = loaded;
}

fn load_profile_inventory_metrics(profile: &Profile) -> Result<LocalStateMetrics, crate::ApiError> {
    let destination = profile
        .dest_path()
        .map_err(|e| crate::ApiError::new("invalid_profile", e.to_string()))?;
    let state_root =
        profile_state_root_dir().map_err(|e| crate::ApiError::new("state_root", e.to_string()))?;
    let inventory_db = fleet_domain::inventory_db_path(&state_root, &profile.id);
    if let Some(parent) = inventory_db.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| crate::ApiError::new("inventory_store", e.to_string()))?;
    }
    Inventory::open(&inventory_db)
        .and_then(|inventory| inventory.load_metrics(&destination))
        .map_err(|err| {
            if err.is_corrupted_database() {
                crate::ApiError::new(
                    fleet_domain::INVENTORY_REBUILD_REQUIRED_CODE,
                    fleet_domain::REBUILD_REQUIRED_MESSAGE,
                )
            } else {
                crate::ApiError::new("inventory_metrics", err.to_string())
            }
        })
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
    let runtime = ensure_profile_runtime_mut(state, profile_id, now_ms);
    runtime.repo_check = None;
    runtime.inventory_check = None;
    runtime.last_error = None;
    runtime.active = None;
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
        canonical_profile_server, clear_profile_check_state, normalize_destination_for_compare,
        profile_path_context_changed,
    };
    use crate::state::AppState;
    use crate::test_support::{EnvVarGuard, ENV_VAR_LOCK};
    use crate::Core;
    use fleet_domain::health::{
        InventoryCheckReport, LocalStateHealth, RepoCheckFreshness, RepoCheckReport,
    };
    use fleet_domain::types::ProfileServerInfo;
    use fleet_domain::{ApiError, Profile};

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
    fn clear_profile_check_state_removes_stale_checks_and_error() {
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
                inventory_check: Some(InventoryCheckReport {
                    profile_id: profile_id.clone(),
                    local_health: LocalStateHealth::ProbeFailed,
                    checked_at_unix_ms: 10,
                    expected_missing_in_inventory_count: 0,
                    inventory_unexpected_paths_count: 0,
                    unexpected_delete_paths: Vec::new(),
                }),
                active: Some(crate::state::ActiveOperationState::new(
                    7,
                    fleet_domain::health::OperationKind::CheckInventory,
                    10,
                )),
                last_operation: None,
                last_error: Some(ApiError::new("check_failed", "symlink path rejected")),
                repo_servers: Vec::new(),
                repo_servers_loaded: false,
                status: crate::state::ProfileStatusState::unknown(10),
            },
        );

        clear_profile_check_state(&mut state, &profile_id, 42);

        let updated = state
            .profile_runtime_by_id
            .get(&profile_id)
            .expect("profile runtime");
        assert!(updated.repo_check.is_none());
        assert!(updated.inventory_check.is_none());
        assert!(updated.last_error.is_none());
        assert!(updated.active.is_none());
        assert_eq!(updated.status.last_check_ms, 0);
    }

    #[test]
    fn profile_set_selected_does_not_persist_in_profiles_config() {
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
            core.profile_set_selected(Some("p1".to_string()))
                .await
                .expect("set selected");

            let cfg = core.list_profiles().await.expect("list profiles");
            assert_eq!(cfg.profiles.len(), 1);
            assert_eq!(cfg.profiles[0].id, "p1");
        });
    }

    #[test]
    fn profile_set_selected_rejects_unknown_profile_id() {
        let _guard = ENV_VAR_LOCK.lock().expect("env lock");

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let _env = EnvVarGuard::set_path("FLEET_CONFIG_DIR", temp_dir.path());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        runtime.block_on(async {
            let core = Core::spawn_threaded_default().expect("core");
            let err = core
                .profile_set_selected(Some("missing".to_string()))
                .await
                .expect_err("set selected should fail");
            assert_eq!(err.code, "not_found");
            assert_eq!(err.message, "missing");
        });
    }

    #[test]
    fn deleting_selected_profile_removes_profile_from_config() {
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
            core.profile_set_selected(Some("p1".to_string()))
                .await
                .expect("set selected");

            core.profile_delete("p1".to_string())
                .await
                .expect("delete profile");

            let cfg = core.list_profiles().await.expect("list profiles");
            assert!(cfg.profiles.is_empty());
        });
    }

    #[test]
    fn canonical_profile_server_keeps_saved_server_when_still_present() {
        let saved = Some(ProfileServerInfo {
            address: "127.0.0.1".to_string(),
            port: 2302,
            password: "pw".to_string(),
        });
        let servers = vec![
            fleet_domain::RepoServer {
                address: "127.0.0.1".to_string(),
                port: 2302,
                password: "pw".to_string(),
            },
            fleet_domain::RepoServer {
                address: "127.0.0.2".to_string(),
                port: 2302,
                password: String::new(),
            },
        ];
        let resolved = canonical_profile_server(saved.clone(), &servers);
        assert_eq!(resolved, saved);
    }

    #[test]
    fn canonical_profile_server_falls_back_to_first_cached_server_when_saved_missing() {
        let saved = Some(ProfileServerInfo {
            address: "old.example.com".to_string(),
            port: 2302,
            password: String::new(),
        });
        let servers = vec![
            fleet_domain::RepoServer {
                address: "new.example.com".to_string(),
                port: 2402,
                password: "next".to_string(),
            },
            fleet_domain::RepoServer {
                address: "other.example.com".to_string(),
                port: 2502,
                password: String::new(),
            },
        ];
        let resolved = canonical_profile_server(saved, &servers).expect("resolved server");
        assert_eq!(resolved.address, "new.example.com");
        assert_eq!(resolved.port, 2402);
        assert_eq!(resolved.password, "next");
    }

    #[test]
    fn canonical_profile_server_returns_none_when_no_cached_servers_exist() {
        let saved = Some(ProfileServerInfo {
            address: "old.example.com".to_string(),
            port: 2302,
            password: String::new(),
        });
        let resolved = canonical_profile_server(saved, &[]);
        assert!(resolved.is_none());
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
            core.profile_save(sample_profile("p1", "/tmp/shared"))
                .await
                .expect("save initial profile");

            let err = core
                .profile_save(sample_profile("p2", "/tmp/shared"))
                .await
                .expect_err("destination conflict should fail");

            assert_eq!(err.code, "destination_in_use");
            assert_eq!(err.message, "destination already used by profile p1");
        });
    }
}
