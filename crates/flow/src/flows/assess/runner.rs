use crate::events::{EventSink, FlowEventKind, LogLevel};
use crate::inventory_access::open_inventory_root;
use crate::locking::{check_lock_state, InventoryLockState};
use crate::prune_policy;
use crate::FlowConfig;
use fleet_domain::health::{
    CheckPhase, LocalHealthState, ProfileAssessmentReport, RemoteFreshnessState,
};
use fleet_domain::{FleetPaths, Profile, ProfileSourceKind};
use flux_manifest::ManifestEntry;
use inventory::{DirtyKind, InventoryState};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

pub async fn run_assess_flow(
    cfg: FlowConfig,
    profile: Profile,
    include_remote: bool,
    cancel: CancellationToken,
) -> anyhow::Result<ProfileAssessmentReport> {
    run_assess_flow_with_sink(cfg, profile, include_remote, cancel, Arc::new(NoopSink)).await
}

pub async fn run_assess_flow_with_sink(
    cfg: FlowConfig,
    profile: Profile,
    include_remote: bool,
    cancel: CancellationToken,
    sink: Arc<dyn EventSink>,
) -> anyhow::Result<ProfileAssessmentReport> {
    info!(
        flow_kind = "check",
        profile_id = %profile.id,
        op = "run_assess_flow",
        phase = if include_remote { "remote" } else { "local" },
        "assessment flow started"
    );
    check_canceled(&cancel)?;
    emit_check_phase(
        sink.as_ref(),
        CheckPhase::ValidatingContext,
        "Validating profile context...",
    );

    let mut local_assessment = evaluate_local_health(&cfg, &profile, &cancel, sink.clone()).await;
    emit_check_phase(
        sink.as_ref(),
        CheckPhase::EvaluatingLocal,
        "Evaluating local state...",
    );
    let local_health = local_assessment.local_health.clone();
    let remote_freshness = if !include_remote {
        if !is_hard_local_invalid_state(&local_health) {
            if let Some(expected_paths) = cached_expected_paths(&cfg, &profile) {
                apply_expected_validation(
                    &mut local_assessment,
                    &profile.destination,
                    Some(&expected_paths),
                );
            }
        }
        RemoteFreshnessState::NotRelevant
    } else if is_hard_local_invalid_state(&local_health) {
        RemoteFreshnessState::Unknown
    } else {
        emit_check_phase(
            sink.as_ref(),
            CheckPhase::LoadingRemoteManifest,
            "Loading remote manifest...",
        );
        let remote_assessment = evaluate_remote_expected_state(&cfg, &profile, &cancel).await;
        emit_check_phase(
            sink.as_ref(),
            CheckPhase::ComparingExpectedState,
            "Comparing local and remote expected state...",
        );
        apply_expected_validation(
            &mut local_assessment,
            &profile.destination,
            remote_assessment.expected_paths.as_ref(),
        );
        remote_assessment.remote_freshness
    };
    emit_check_phase(
        sink.as_ref(),
        CheckPhase::Finalizing,
        "Finalizing check report...",
    );

    let report = ProfileAssessmentReport {
        profile_id: profile.id.clone(),
        local_health: local_assessment.local_health,
        remote_freshness,
        checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        expected_missing_in_inventory_count: local_assessment.expected_missing_in_inventory.len()
            as u64,
        inventory_unexpected_paths_count: local_assessment.inventory_unexpected_paths.len() as u64,
        unexpected_delete_paths: local_assessment.unexpected_delete_paths,
    };
    debug!(
        flow_kind = "check",
        profile_id = %profile.id,
        op = "run_assess_flow",
        expected_validation = ?local_assessment.expected_validation_state,
        expected_missing = local_assessment.expected_missing_in_inventory.len(),
        inventory_unexpected = local_assessment.inventory_unexpected_paths.len(),
        "assessment strict expected-state signals"
    );
    info!(
        flow_kind = "check",
        profile_id = %profile.id,
        op = "run_assess_flow",
        outcome = "ok",
        reason = "assessment_complete",
        "assessment flow finished"
    );
    sink.emit(FlowEventKind::Message {
        level: LogLevel::Info,
        text: "Check complete.".to_string(),
    });
    Ok(report)
}

struct NoopSink;

impl EventSink for NoopSink {
    fn emit(&self, _event: FlowEventKind) {}
}

fn emit_check_phase(sink: &dyn EventSink, phase: CheckPhase, text: &str) {
    sink.emit(FlowEventKind::CheckPhaseChanged { phase });
    sink.emit(FlowEventKind::Message {
        level: LogLevel::Info,
        text: text.to_string(),
    });
}

struct LocalAssessmentResult {
    local_health: LocalHealthState,
    unexpected_delete_paths: Vec<String>,
    expected_validation_state: ExpectedValidationState,
    expected_missing_in_inventory: Vec<String>,
    inventory_unexpected_paths: Vec<String>,
    inventory_file_paths: BTreeSet<String>,
}

fn local_assessment(local_health: LocalHealthState) -> LocalAssessmentResult {
    LocalAssessmentResult {
        local_health,
        unexpected_delete_paths: Vec::new(),
        expected_validation_state: ExpectedValidationState::NotRequested,
        expected_missing_in_inventory: Vec::new(),
        inventory_unexpected_paths: Vec::new(),
        inventory_file_paths: BTreeSet::new(),
    }
}

fn local_error() -> LocalAssessmentResult {
    local_assessment(LocalHealthState::Error)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpectedValidationState {
    NotRequested,
    Available,
    Unavailable,
}

struct RemoteExpectedState {
    remote_freshness: RemoteFreshnessState,
    expected_paths: Option<BTreeSet<String>>,
}

async fn evaluate_local_health(
    cfg: &FlowConfig,
    profile: &Profile,
    cancel: &CancellationToken,
    sink: Arc<dyn EventSink>,
) -> LocalAssessmentResult {
    if check_canceled(cancel).is_err() {
        warn!(
            flow_kind = "check",
            profile_id = %profile.id,
            op = "evaluate_local_health",
            outcome = "canceled",
            "local health evaluation canceled"
        );
        return local_error();
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
        return local_error();
    }
    let dest_path = match profile.dest_path() {
        Ok(path) => path,
        Err(_) => return local_error(),
    };

    let dest_exists = fs::try_exists(&dest_path).await;
    if check_canceled(cancel).is_err() {
        return local_error();
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
            return local_assessment(LocalHealthState::MissingDestination);
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
            return local_error();
        }
    }

    let layout = FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), &profile.id);
    let state_dir_exists = fs::try_exists(&layout.state_dir).await;
    if check_canceled(cancel).is_err() {
        return local_error();
    }
    let db_exists = fs::try_exists(&layout.inventory_db).await;
    if check_canceled(cancel).is_err() {
        return local_error();
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
            return local_assessment(LocalHealthState::LocalStateMissing);
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
            return local_error();
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
            return local_error();
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
            return local_error();
        }
    }
    if check_canceled(cancel).is_err() {
        return local_error();
    }

    let policy = cfg.scanner_config.policy.clone();
    let profile_id = profile.id.clone();
    let cfg_cloned = cfg.clone();
    emit_check_phase(
        sink.as_ref(),
        CheckPhase::ScanningLocal,
        "Scanning local files...",
    );

    let mut local_state_task =
        tokio::task::spawn_blocking(move || -> anyhow::Result<LocalAssessmentResult> {
            let root =
                open_inventory_root(&cfg_cloned, &layout.inventory_db, &profile_id, &dest_path)?;
            let inventory_snapshot = root.snapshot()?;
            let initial_inventory_file_paths = inventory_snapshot
                .files
                .into_iter()
                .map(|file| fleet_domain::normalize_rel_slashes(&file.file.rel_path))
                .collect::<BTreeSet<_>>();
            if root.metrics()?.last_stamp.is_none() {
                let mut result = local_assessment(LocalHealthState::LocalStateMissing);
                result.inventory_file_paths = initial_inventory_file_paths;
                return Ok(result);
            }

            let inventory_snapshot = root.snapshot()?;
            let inventory_file_paths = inventory_snapshot
                .files
                .into_iter()
                .map(|file| fleet_domain::normalize_rel_slashes(&file.file.rel_path))
                .collect::<BTreeSet<_>>();
            let state = root.state(&policy)?;
            let (local_health, unexpected_delete_paths, inventory_file_paths) = match state {
                InventoryState::Clean { .. } => {
                    (LocalHealthState::Ready, Vec::new(), inventory_file_paths)
                }
                InventoryState::Dirty { .. } => {
                    let mut paths = root
                        .dirty_files(&policy)?
                        .into_iter()
                        .filter(|dirty| dirty.kind == DirtyKind::Added)
                        .map(|dirty| std::path::PathBuf::from(dirty.rel_path))
                        .filter(|rel| !prune_policy::is_protected_root_entry(&dest_path, rel))
                        .map(|rel| rel.to_string_lossy().to_string())
                        .collect::<Vec<_>>();
                    paths.sort();
                    paths.dedup();
                    (LocalHealthState::LocalDrift, paths, inventory_file_paths)
                }
                InventoryState::MissingRoot { .. } => (
                    LocalHealthState::MissingDestination,
                    Vec::new(),
                    BTreeSet::new(),
                ),
            };

            Ok(LocalAssessmentResult {
                local_health,
                unexpected_delete_paths,
                expected_validation_state: ExpectedValidationState::NotRequested,
                expected_missing_in_inventory: Vec::new(),
                inventory_unexpected_paths: Vec::new(),
                inventory_file_paths,
            })
        });

    let local_state = tokio::select! {
        _ = cancel.cancelled() => {
            local_state_task.abort();
            return local_error();
        }
        result = &mut local_state_task => result,
    };

    if check_canceled(cancel).is_err() {
        return local_error();
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
            local_error()
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
            local_error()
        }
    }
}

async fn evaluate_remote_expected_state(
    cfg: &FlowConfig,
    profile: &Profile,
    cancel: &CancellationToken,
) -> RemoteExpectedState {
    info!(
        flow_kind = "check",
        profile_id = %profile.id,
        op = "evaluate_remote_expected_state",
        phase = "remote",
        "remote expected-state evaluation started"
    );
    let layout = FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), &profile.id);

    let repo_url = match profile.validated_source_kind() {
        Ok(ProfileSourceKind::Http(url)) => url.to_string(),
        Err(_) => {
            warn!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_remote_expected_state",
                outcome = "failed",
                reason = "invalid_repo_source",
                "remote expected-state evaluation skipped due to invalid source"
            );
            return RemoteExpectedState {
                remote_freshness: RemoteFreshnessState::Error,
                expected_paths: None,
            };
        }
    };

    let refreshed_manifest = fleet_manifest::load_desired_manifest_with_freshness(
        &repo_url,
        &layout.repo_cache,
        &cfg.downloads,
        None,
    )
    .await;
    if check_canceled(cancel).is_err() {
        return RemoteExpectedState {
            remote_freshness: RemoteFreshnessState::Error,
            expected_paths: None,
        };
    }

    match refreshed_manifest {
        Ok(loaded) => {
            let expected_paths = manifest_expected_file_paths(&loaded.manifest);
            let remote_freshness = map_manifest_freshness(loaded.freshness);
            info!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_remote_expected_state",
                outcome = "ok",
                remote = ?remote_freshness,
                expected_paths = expected_paths.len(),
                "remote expected-state evaluation complete"
            );
            RemoteExpectedState {
                remote_freshness,
                expected_paths: Some(expected_paths),
            }
        }
        Err(err) => {
            warn!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_remote_expected_state",
                outcome = "failed",
                reason = "manifest_refresh_failed",
                "remote expected manifest refresh failed; attempting cache fallback"
            );
            debug!(
                flow_kind = "check",
                profile_id = %profile.id,
                op = "evaluate_remote_expected_state",
                error = %err,
                "remote expected manifest refresh error details"
            );

            let cached_manifest =
                fleet_manifest::load_cached_desired_manifest(&repo_url, &layout.repo_cache);
            let expected_paths = match cached_manifest {
                Ok(Some(manifest)) => Some(manifest_expected_file_paths(&manifest)),
                Ok(None) => None,
                Err(cache_err) => {
                    warn!(
                        flow_kind = "check",
                        profile_id = %profile.id,
                        op = "evaluate_remote_expected_state",
                        outcome = "failed",
                        reason = "manifest_cache_fallback_failed",
                        "remote expected manifest cache fallback failed"
                    );
                    debug!(
                        flow_kind = "check",
                        profile_id = %profile.id,
                        op = "evaluate_remote_expected_state",
                        error = %cache_err,
                        "remote expected manifest cache fallback error details"
                    );
                    None
                }
            };

            let remote_freshness = if expected_paths.is_some() {
                RemoteFreshnessState::Error
            } else {
                RemoteFreshnessState::Unknown
            };
            RemoteExpectedState {
                remote_freshness,
                expected_paths,
            }
        }
    }
}

fn is_hard_local_invalid_state(state: &LocalHealthState) -> bool {
    matches!(
        state,
        LocalHealthState::MissingDestination
            | LocalHealthState::LocalStateMissing
            | LocalHealthState::Error
    )
}

fn manifest_expected_file_paths(manifest: &fleet_manifest::DesiredManifest) -> BTreeSet<String> {
    manifest
        .entries
        .iter()
        .filter_map(|entry| match entry {
            ManifestEntry::File(file) => {
                let rel = file.rel_path.to_string_lossy();
                Some(fleet_domain::normalize_rel_slashes(rel.as_ref()))
            }
            _ => None,
        })
        .collect()
}

fn map_manifest_freshness(
    freshness: fleet_manifest::DesiredManifestFreshness,
) -> RemoteFreshnessState {
    match freshness {
        fleet_manifest::DesiredManifestFreshness::Unknown => RemoteFreshnessState::Unknown,
        fleet_manifest::DesiredManifestFreshness::UpToDate => RemoteFreshnessState::UpToDate,
        fleet_manifest::DesiredManifestFreshness::UpdateAvailable => {
            RemoteFreshnessState::UpdateAvailable
        }
    }
}

fn cached_expected_paths(cfg: &FlowConfig, profile: &Profile) -> Option<BTreeSet<String>> {
    let layout = FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), &profile.id);
    let repo_url = match profile.validated_source_kind() {
        Ok(ProfileSourceKind::Http(url)) => url.to_string(),
        Err(_) => return None,
    };
    match fleet_manifest::load_cached_desired_manifest(&repo_url, &layout.repo_cache) {
        Ok(Some(manifest)) => Some(manifest_expected_file_paths(&manifest)),
        _ => None,
    }
}

fn apply_expected_validation(
    assessment: &mut LocalAssessmentResult,
    destination: &str,
    expected_paths: Option<&BTreeSet<String>>,
) {
    let Some(expected_paths) = expected_paths else {
        assessment.expected_validation_state = ExpectedValidationState::Unavailable;
        assessment.local_health = LocalHealthState::LocalDrift;
        return;
    };

    assessment.expected_validation_state = ExpectedValidationState::Available;
    assessment.expected_missing_in_inventory = expected_paths
        .difference(&assessment.inventory_file_paths)
        .cloned()
        .collect();
    assessment.inventory_unexpected_paths = assessment
        .inventory_file_paths
        .difference(expected_paths)
        .filter(|rel| {
            !prune_policy::is_protected_root_entry(Path::new(destination), Path::new(rel.as_str()))
        })
        .cloned()
        .collect();

    let mut merged_unexpected = assessment
        .unexpected_delete_paths
        .iter()
        .map(|rel| fleet_domain::normalize_rel_slashes(rel))
        .filter(|rel| !expected_paths.contains(rel))
        .collect::<Vec<_>>();
    merged_unexpected.extend(assessment.inventory_unexpected_paths.iter().cloned());
    merged_unexpected.sort();
    merged_unexpected.dedup();
    assessment.unexpected_delete_paths = merged_unexpected;

    if !assessment.expected_missing_in_inventory.is_empty()
        || !assessment.inventory_unexpected_paths.is_empty()
    {
        assessment.local_health = LocalHealthState::LocalDrift;
    }
}

fn check_canceled(cancel: &CancellationToken) -> anyhow::Result<()> {
    if cancel.is_cancelled() {
        anyhow::bail!("canceled");
    }
    Ok(())
}
