use crate::inventory_access::open_inventory_root;
use crate::locking::{check_lock_state, InventoryLockState};
use crate::FlowConfig;
use fleet_domain::health::{LocalHealthState, ProfileAssessmentReport, RemoteFreshnessState};
use fleet_domain::{FleetPaths, Profile, ProfileSourceKind};
use inventory::InventoryState;
use tokio::fs;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

pub async fn run_assess_flow(
    cfg: FlowConfig,
    profile: Profile,
    include_remote: bool,
    cancel: CancellationToken,
) -> anyhow::Result<ProfileAssessmentReport> {
    info!(
        flow_kind = "check",
        profile_id = %profile.id,
        op = "run_assess_flow",
        phase = if include_remote { "remote" } else { "local" },
        "assessment flow started"
    );
    check_canceled(&cancel)?;

    let local_health = evaluate_local_health(&cfg, &profile, &cancel).await;
    let remote_freshness = if !include_remote {
        RemoteFreshnessState::NotRelevant
    } else if local_health != LocalHealthState::Ready {
        RemoteFreshnessState::Unknown
    } else {
        evaluate_remote_freshness(&cfg, &profile, &cancel).await
    };

    let report = ProfileAssessmentReport {
        profile_id: profile.id.clone(),
        local_health,
        remote_freshness,
        checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
    };
    info!(
        flow_kind = "check",
        profile_id = %profile.id,
        op = "run_assess_flow",
        outcome = "ok",
        reason = "assessment_complete",
        "assessment flow finished"
    );
    Ok(report)
}

async fn evaluate_local_health(
    cfg: &FlowConfig,
    profile: &Profile,
    cancel: &CancellationToken,
) -> LocalHealthState {
    if check_canceled(cancel).is_err() {
        warn!(
            flow_kind = "check",
            profile_id = %profile.id,
            op = "evaluate_local_health",
            outcome = "canceled",
            "local health evaluation canceled"
        );
        return LocalHealthState::Error;
    }

    if profile.dest_path().is_err() || profile.validated_source_kind().is_err() {
        warn!(
            flow_kind = "check",
            profile_id = %profile.id,
            op = "evaluate_local_health",
            outcome = "failed",
            reason = "invalid_profile_context",
            "local health evaluation failed profile validation"
        );
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
        Ok(false) => {
            info!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_local_health",
                outcome = "missing_destination",
                "local health detected missing destination"
            );
            return LocalHealthState::MissingDestination;
        }
        Err(_) => {
            error!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_local_health",
                outcome = "failed",
                reason = "destination_probe_failed",
                "local health failed while probing destination"
            );
            return LocalHealthState::Error;
        }
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
        (Ok(_), Ok(_)) => {
            info!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_local_health",
                outcome = "missing_local_state",
                "local health detected missing local state"
            );
            return LocalHealthState::LocalStateMissing;
        }
        _ => {
            error!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_local_health",
                outcome = "failed",
                reason = "state_probe_failed",
                "local health failed while probing state files"
            );
            return LocalHealthState::Error;
        }
    }

    match check_lock_state(&layout.inventory_lock).await {
        Ok(InventoryLockState::NotLocked) => {}
        Ok(InventoryLockState::Locked { .. }) => {
            warn!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_local_health",
                outcome = "locked",
                reason = "inventory_lock_held",
                "local health found active inventory lock"
            );
            return LocalHealthState::Error;
        }
        Err(_) => {
            error!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_local_health",
                outcome = "failed",
                reason = "inventory_lock_probe_failed",
                "local health failed while checking lock state"
            );
            return LocalHealthState::Error;
        }
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
        Ok(Ok(v)) => {
            info!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_local_health",
                outcome = "ok",
                reason = "local_state_computed",
                "local health evaluation complete"
            );
            v
        }
        Ok(Err(err)) => {
            error!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_local_health",
                outcome = "failed",
                reason = "inventory_state_failed",
                "local health inventory state failed"
            );
            debug!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_local_health",
                error = %err,
                "local health inventory state error details"
            );
            LocalHealthState::Error
        }
        Err(err) => {
            error!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_local_health",
                outcome = "failed",
                reason = "blocking_task_failed",
                "local health background task failed"
            );
            debug!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_local_health",
                error = %err,
                "local health blocking task error details"
            );
            LocalHealthState::Error
        }
    }
}

async fn evaluate_remote_freshness(
    cfg: &FlowConfig,
    profile: &Profile,
    cancel: &CancellationToken,
) -> RemoteFreshnessState {
    info!(
        flow_kind = "check",
        profile_id = %profile.id,
        op = "evaluate_remote_freshness",
        phase = "remote",
        "remote freshness evaluation started"
    );
    let layout = FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), &profile.id);

    let repo_url = match profile.validated_source_kind() {
        Ok(ProfileSourceKind::Http(url)) => url.to_string(),
        Err(_) => {
            warn!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_remote_freshness",
                outcome = "failed",
                reason = "invalid_repo_source",
                "remote freshness skipped due to invalid source"
            );
            return RemoteFreshnessState::Error;
        }
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
            swifty_repo::touch::RepoTouchStatus::UpToDate => {
                info!(
                    flow_kind = "check",
                    profile_id = %profile.id,
                    op = "evaluate_remote_freshness",
                    outcome = "up_to_date",
                    "remote freshness up to date"
                );
                RemoteFreshnessState::UpToDate
            }
            swifty_repo::touch::RepoTouchStatus::UpdateAvailable => {
                info!(
                    flow_kind = "check",
                    profile_id = %profile.id,
                    op = "evaluate_remote_freshness",
                    outcome = "update_available",
                    "remote freshness update available"
                );
                RemoteFreshnessState::UpdateAvailable
            }
            swifty_repo::touch::RepoTouchStatus::NoCache => {
                warn!(
                    flow_kind = "check",
                    profile_id = %profile.id,
                    op = "evaluate_remote_freshness",
                    outcome = "unknown",
                    reason = "no_cache",
                    "remote freshness unavailable because no cache exists"
                );
                RemoteFreshnessState::Unknown
            }
        },
        Err(err) => {
            error!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_remote_freshness",
                outcome = "failed",
                reason = "touch_repo_failed",
                "remote freshness evaluation failed"
            );
            debug!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_remote_freshness",
                error = %err,
                "remote freshness error details"
            );
            RemoteFreshnessState::Error
        }
    }
}

fn check_canceled(cancel: &CancellationToken) -> anyhow::Result<()> {
    if cancel.is_cancelled() {
        anyhow::bail!("canceled");
    }
    Ok(())
}
