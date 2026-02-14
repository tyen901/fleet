use crate::inventory_access::open_inventory_root;
use crate::locking::{check_lock_state, InventoryLockState};
use crate::FlowConfig;
use fleet_domain::health::{LocalHealthState, ProfileAssessmentReport, RemoteFreshnessState};
use fleet_domain::{FleetPaths, Profile, ProfileSourceKind};
use inventory::InventoryState;
use tokio::fs;
use tokio_util::sync::CancellationToken;

pub async fn run_assess_flow(
    cfg: FlowConfig,
    profile: Profile,
    include_remote: bool,
    cancel: CancellationToken,
) -> anyhow::Result<ProfileAssessmentReport> {
    check_canceled(&cancel)?;

    let local_health = evaluate_local_health(&cfg, &profile, &cancel).await;
    let remote_freshness = if !include_remote {
        RemoteFreshnessState::NotRelevant
    } else if local_health != LocalHealthState::Ready {
        RemoteFreshnessState::Unknown
    } else {
        evaluate_remote_freshness(&cfg, &profile, &cancel).await
    };

    Ok(ProfileAssessmentReport {
        profile_id: profile.id.clone(),
        local_health,
        remote_freshness,
        checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
    })
}

async fn evaluate_local_health(
    cfg: &FlowConfig,
    profile: &Profile,
    cancel: &CancellationToken,
) -> LocalHealthState {
    if check_canceled(cancel).is_err() {
        return LocalHealthState::Error;
    }

    if profile.dest_path().is_err() || profile.validated_source_kind().is_err() {
        return LocalHealthState::Error;
    }
    let dest_path = match profile.dest_path() {
        Ok(path) => path,
        Err(_) => return LocalHealthState::Error,
    };

    let dest_exists = fs::try_exists(&dest_path).await;
    if check_canceled(cancel).is_err() {
        return LocalHealthState::Error;
    }
    match dest_exists {
        Ok(true) => {}
        Ok(false) => return LocalHealthState::MissingDestination,
        Err(_) => return LocalHealthState::Error,
    }

    let layout = FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), &profile.id);
    let state_dir_exists = fs::try_exists(&layout.state_dir).await;
    if check_canceled(cancel).is_err() {
        return LocalHealthState::Error;
    }
    let db_exists = fs::try_exists(&layout.inventory_db).await;
    if check_canceled(cancel).is_err() {
        return LocalHealthState::Error;
    }
    match (state_dir_exists, db_exists) {
        (Ok(true), Ok(true)) => {}
        (Ok(_), Ok(_)) => return LocalHealthState::LocalStateMissing,
        _ => return LocalHealthState::Error,
    }

    match check_lock_state(&layout.inventory_lock).await {
        Ok(InventoryLockState::NotLocked) => {}
        Ok(InventoryLockState::Locked { .. }) => return LocalHealthState::Error,
        Err(_) => return LocalHealthState::Error,
    }
    if check_canceled(cancel).is_err() {
        return LocalHealthState::Error;
    }

    let policy = cfg.scanner_config.policy.clone();
    let profile_id = profile.id.clone();
    let cfg_cloned = cfg.clone();
    let local_state = tokio::task::spawn_blocking(move || -> anyhow::Result<LocalHealthState> {
        let root = open_inventory_root(&cfg_cloned, &layout.inventory_db, &profile_id, &dest_path)?;
        if root.metrics()?.last_stamp.is_none() {
            return Ok(LocalHealthState::LocalStateMissing);
        }

        let state = root.state(&policy)?;
        let local_health = match state {
            InventoryState::Clean { .. } => LocalHealthState::Ready,
            InventoryState::Dirty { .. } => LocalHealthState::LocalDrift,
            InventoryState::MissingRoot { .. } => LocalHealthState::MissingDestination,
        };
        Ok(local_health)
    })
    .await;

    if check_canceled(cancel).is_err() {
        return LocalHealthState::Error;
    }

    match local_state {
        Ok(Ok(v)) => v,
        Ok(Err(_)) => LocalHealthState::Error,
        Err(_) => LocalHealthState::Error,
    }
}

async fn evaluate_remote_freshness(
    cfg: &FlowConfig,
    profile: &Profile,
    cancel: &CancellationToken,
) -> RemoteFreshnessState {
    let layout = FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), &profile.id);

    let repo_url = match profile.validated_source_kind() {
        Ok(ProfileSourceKind::Http(url)) => url.to_string(),
        Err(_) => return RemoteFreshnessState::Error,
    };

    let store = swifty_repo::FsRepoCacheStore::new(layout.repo_cache);
    let result = swifty_repo::touch::touch_repo_json(
        &repo_url,
        &store,
        &cfg.downloads,
        None,
        swifty_repo::touch::RepoTouchOptions::default(),
    )
    .await;
    if check_canceled(cancel).is_err() {
        return RemoteFreshnessState::Error;
    }

    match result {
        Ok(report) => match report.status {
            swifty_repo::touch::RepoTouchStatus::UpToDate => RemoteFreshnessState::UpToDate,
            swifty_repo::touch::RepoTouchStatus::UpdateAvailable => {
                RemoteFreshnessState::UpdateAvailable
            }
            swifty_repo::touch::RepoTouchStatus::NoCache => RemoteFreshnessState::Unknown,
        },
        Err(_) => RemoteFreshnessState::Error,
    }
}

fn check_canceled(cancel: &CancellationToken) -> anyhow::Result<()> {
    if cancel.is_cancelled() {
        anyhow::bail!("canceled");
    }
    Ok(())
}
