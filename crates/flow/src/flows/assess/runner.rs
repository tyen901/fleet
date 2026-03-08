use crate::events::{EventSink, FlowEventKind, LogLevel};
use crate::locking::{check_lock_state, InventoryLockState};
use crate::FlowConfig;
use fleet_domain::health::{
    AssessPhase, LocalStateHealth, ProfileStateReport, RemoteFreshnessState,
};
use fleet_domain::{FleetPaths, Profile, ProfileSourceKind};
use fleet_local_state::{BaselineStatus, LocalStateAssessment};
use flux_manifest::ManifestEntry;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

pub async fn run_assess_flow(
    cfg: FlowConfig,
    profile: Profile,
    include_remote: bool,
    cancel: CancellationToken,
) -> anyhow::Result<ProfileStateReport> {
    run_assess_flow_with_sink(cfg, profile, include_remote, cancel, Arc::new(NoopSink)).await
}

pub async fn run_assess_flow_with_sink(
    cfg: FlowConfig,
    profile: Profile,
    include_remote: bool,
    cancel: CancellationToken,
    sink: Arc<dyn EventSink>,
) -> anyhow::Result<ProfileStateReport> {
    info!(
        flow_kind = "check",
        profile_id = %profile.id,
        op = "run_assess_flow",
        phase = if include_remote { "remote" } else { "local" },
        "assessment flow started"
    );
    check_canceled(&cancel)?;
    emit_assess_phase(
        sink.as_ref(),
        AssessPhase::ValidatingContext,
        "Validating profile context...",
    );

    let mut assessment = evaluate_local_state(&cfg, &profile, sink.clone()).await?;
    emit_assess_phase(
        sink.as_ref(),
        AssessPhase::EvaluatingLocal,
        "Evaluating local state...",
    );
    let remote_freshness = if !include_remote {
        if !is_hard_local_invalid_state(&assessment.health) {
            if let Some(expected_paths) = cached_expected_paths(&cfg, &profile) {
                apply_expected_validation(
                    &mut assessment,
                    &profile.destination,
                    Some(&expected_paths),
                );
            }
        }
        None
    } else if is_hard_local_invalid_state(&assessment.health) {
        Some(RemoteFreshnessState::Unknown)
    } else {
        emit_assess_phase(
            sink.as_ref(),
            AssessPhase::LoadingRemoteManifest,
            "Loading remote manifest...",
        );
        let remote_assessment = evaluate_remote_expected_state(&cfg, &profile, &cancel).await;
        emit_assess_phase(
            sink.as_ref(),
            AssessPhase::ComparingExpectedState,
            "Comparing local and remote expected state...",
        );
        apply_expected_validation(
            &mut assessment,
            &profile.destination,
            remote_assessment.expected_paths.as_ref(),
        );
        Some(remote_assessment.remote_freshness)
    };
    emit_assess_phase(
        sink.as_ref(),
        AssessPhase::Finalizing,
        "Finalizing check report...",
    );

    debug!(
        flow_kind = "check",
        profile_id = %profile.id,
        op = "run_assess_flow",
        expected_missing = assessment.expected_missing_count,
        unexpected = assessment.unexpected_count,
        "assessment strict expected-state signals"
    );
    let report = ProfileStateReport {
        profile_id: assessment.profile_id,
        local_health: assessment.health,
        remote_freshness,
        checked_at_unix_ms: assessment.checked_at_unix_ms,
        expected_missing_in_inventory_count: assessment.expected_missing_count,
        inventory_unexpected_paths_count: assessment.unexpected_count,
        unexpected_delete_paths: assessment.unexpected_paths,
    };
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

fn emit_assess_phase(sink: &dyn EventSink, phase: AssessPhase, text: &str) {
    sink.emit(FlowEventKind::AssessPhaseChanged { phase });
    sink.emit(FlowEventKind::Message {
        level: LogLevel::Info,
        text: text.to_string(),
    });
}

struct RemoteExpectedState {
    remote_freshness: RemoteFreshnessState,
    expected_paths: Option<BTreeSet<String>>,
}

async fn evaluate_local_state(
    cfg: &FlowConfig,
    profile: &Profile,
    sink: Arc<dyn EventSink>,
) -> anyhow::Result<LocalStateAssessment> {
    let dest_path = match profile.dest_path() {
        Ok(path) => path,
        Err(_) => {
            return Ok(LocalStateAssessment {
                profile_id: profile.id.clone(),
                health: LocalStateHealth::InvalidProfile,
                checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
                expected_missing_count: 0,
                unexpected_count: 0,
                unexpected_paths: Vec::new(),
                baseline_status: BaselineStatus::Missing,
                tracked_paths: Vec::new(),
            });
        }
    };
    if profile.validated_source_kind().is_err() {
        return Ok(LocalStateAssessment {
            profile_id: profile.id.clone(),
            health: LocalStateHealth::InvalidProfile,
            checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
            expected_missing_count: 0,
            unexpected_count: 0,
            unexpected_paths: Vec::new(),
            baseline_status: BaselineStatus::Missing,
            tracked_paths: Vec::new(),
        });
    }

    let layout = FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), &profile.id);
    match check_lock_state(&layout.profile.local_state.lock).await {
        Ok(InventoryLockState::NotLocked) => {}
        Ok(InventoryLockState::Locked { .. }) => {
            return Ok(LocalStateAssessment {
                profile_id: profile.id.clone(),
                health: LocalStateHealth::Blocked,
                checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
                expected_missing_count: 0,
                unexpected_count: 0,
                unexpected_paths: Vec::new(),
                baseline_status: BaselineStatus::Missing,
                tracked_paths: Vec::new(),
            });
        }
        Err(_) => {
            return Ok(LocalStateAssessment {
                profile_id: profile.id.clone(),
                health: LocalStateHealth::ProbeFailed,
                checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
                expected_missing_count: 0,
                unexpected_count: 0,
                unexpected_paths: Vec::new(),
                baseline_status: BaselineStatus::Missing,
                tracked_paths: Vec::new(),
            });
        }
    }
    emit_assess_phase(
        sink.as_ref(),
        AssessPhase::ScanningLocal,
        "Scanning local files...",
    );
    let cfg_cloned = cfg.clone();
    let profile_id_for_engine = profile.id.clone();
    let profile_id_for_error = profile.id.clone();
    tokio::task::spawn_blocking(move || {
        cfg_cloned.local_state.assess(
            &profile_id_for_engine,
            &dest_path,
            &layout.profile.local_state.db,
            &layout.profile.local_state.lock,
            &cfg_cloned.local_state_config,
            None,
        )
    })
    .await?
    .map_err(|err| match err {
        fleet_local_state::LocalStateError::CorruptDatabase => anyhow::Error::new(err),
        other => anyhow::Error::new(other),
    })
    .or_else(|err| {
        if err
            .chain()
            .filter_map(|cause| cause.downcast_ref::<fleet_local_state::LocalStateError>())
            .any(fleet_local_state::LocalStateError::is_corrupted_database)
        {
            Ok(LocalStateAssessment {
                profile_id: profile_id_for_error,
                health: LocalStateHealth::InventoryCorrupt,
                checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
                expected_missing_count: 0,
                unexpected_count: 0,
                unexpected_paths: Vec::new(),
                baseline_status: BaselineStatus::Present,
                tracked_paths: Vec::new(),
            })
        } else {
            Err(err)
        }
    })
}

async fn evaluate_remote_expected_state(
    cfg: &FlowConfig,
    profile: &Profile,
    cancel: &CancellationToken,
) -> RemoteExpectedState {
    let layout = FleetPaths::for_profile(cfg.profile_state_root_dir.clone(), &profile.id);

    let repo_url = match profile.validated_source_kind() {
        Ok(ProfileSourceKind::Http(url)) => url.to_string(),
        Err(_) => {
            return RemoteExpectedState {
                remote_freshness: RemoteFreshnessState::Error,
                expected_paths: None,
            };
        }
    };

    let refreshed_manifest = fleet_manifest::load_desired_manifest_with_freshness(
        &repo_url,
        &layout.profile.repo_cache,
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
        Ok(loaded) => RemoteExpectedState {
            remote_freshness: map_manifest_freshness(loaded.freshness),
            expected_paths: Some(manifest_expected_file_paths(&loaded.manifest)),
        },
        Err(_) => {
            let expected_paths = match fleet_manifest::load_cached_desired_manifest(
                &repo_url,
                &layout.profile.repo_cache,
            ) {
                Ok(Some(manifest)) => Some(manifest_expected_file_paths(&manifest)),
                _ => None,
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

fn is_hard_local_invalid_state(state: &LocalStateHealth) -> bool {
    matches!(
        state,
        LocalStateHealth::MissingDestination
            | LocalStateHealth::LocalStateMissing
            | LocalStateHealth::Blocked
            | LocalStateHealth::InvalidProfile
            | LocalStateHealth::ProbeFailed
            | LocalStateHealth::InventoryCorrupt
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
    match fleet_manifest::load_cached_desired_manifest(&repo_url, &layout.profile.repo_cache) {
        Ok(Some(manifest)) => Some(manifest_expected_file_paths(&manifest)),
        _ => None,
    }
}

fn apply_expected_validation(
    assessment: &mut LocalStateAssessment,
    destination: &str,
    expected_paths: Option<&BTreeSet<String>>,
) {
    let tracked = assessment
        .tracked_paths
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let Some(expected_paths) = expected_paths else {
        assessment.health = LocalStateHealth::LocalDrift;
        return;
    };

    let expected_missing = expected_paths
        .difference(&tracked)
        .cloned()
        .collect::<Vec<_>>();
    let inventory_unexpected = tracked
        .difference(expected_paths)
        .filter(|rel| {
            !crate::prune_policy::is_protected_root_entry(
                Path::new(destination),
                Path::new(rel.as_str()),
            )
        })
        .cloned()
        .collect::<Vec<_>>();

    let mut merged_unexpected = assessment
        .unexpected_paths
        .iter()
        .map(|rel| fleet_domain::normalize_rel_slashes(rel))
        .filter(|rel| !expected_paths.contains(rel))
        .collect::<Vec<_>>();
    merged_unexpected.extend(inventory_unexpected);
    merged_unexpected.sort();
    merged_unexpected.dedup();
    assessment.unexpected_paths = merged_unexpected;
    assessment.unexpected_count = assessment.unexpected_paths.len() as u64;
    assessment.expected_missing_count = expected_missing.len() as u64;

    if assessment.expected_missing_count > 0 || assessment.unexpected_count > 0 {
        assessment.health = LocalStateHealth::LocalDrift;
    }
}

fn check_canceled(cancel: &CancellationToken) -> anyhow::Result<()> {
    if cancel.is_cancelled() {
        anyhow::bail!("canceled");
    }
    Ok(())
}
