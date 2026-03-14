use crate::api::{OperationStage, ProgressMetric, ProgressScope, ProgressUnit};
use crate::engine::{OperationContext, ResolvedProfile};
use crate::local_state;
use crate::operations::OperationError;
use crate::support::locking::{check_lock_state, InventoryLockState};
use fleet_domain::health::{
    AssessScope, LocalStateHealth, ProfileStateReport, RemoteFreshnessState,
};
use fleet_domain::ProfileSourceKind;
use fleet_inventory::{Inventory, InventoryError};
use flux_manifest::ManifestEntry;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

pub(crate) async fn run_assess(
    mut ctx: OperationContext,
    scope: AssessScope,
) -> anyhow::Result<OperationContext> {
    ensure_not_canceled(&ctx)?;
    ctx.emitter.enter_stage(OperationStage::Validating);
    let resolved = match resolve_profile(&ctx) {
        Ok(resolved) => resolved,
        Err(local_health) => {
            ctx.final_report = Some(invalid_report(&ctx.profile.id, local_health, None));
            ctx.emitter.exit_stage(OperationStage::Validating);
            return Ok(ctx);
        }
    };
    ctx.resolved = Some(resolved.clone());
    ctx.emitter.exit_stage(OperationStage::Validating);

    ctx.emitter
        .enter_stage(OperationStage::LoadingExpectedState);
    let cached_expected = load_cached_manifest(&ctx);
    let remote = if matches!(scope, AssessScope::Remote) {
        Some(load_remote_state(&ctx).await)
    } else {
        None
    };
    ctx.emitter.exit_stage(OperationStage::LoadingExpectedState);

    match check_lock_state(&resolved.paths.profile.inventory.lock).await {
        Ok(InventoryLockState::Locked { .. }) => {
            ctx.final_report = Some(invalid_report(
                &ctx.profile.id,
                LocalStateHealth::Blocked,
                remote.as_ref().map(|r| r.remote_freshness.clone()),
            ));
            return Ok(ctx);
        }
        Ok(InventoryLockState::NotLocked) => {}
        Err(_) => {
            ctx.final_report = Some(invalid_report(
                &ctx.profile.id,
                LocalStateHealth::ProbeFailed,
                remote.as_ref().map(|r| r.remote_freshness.clone()),
            ));
            return Ok(ctx);
        }
    }

    ctx.emitter.enter_stage(OperationStage::ScanningDisk);
    let snapshot = evaluate_local_state_snapshot(&ctx, &resolved).await?;
    ctx.tracked_paths = snapshot.tracked_paths.clone();
    emit_verify_summary(&ctx, &snapshot);
    ctx.emitter.exit_stage(OperationStage::VerifyingInventory);

    let mut assessment = snapshot.assessment;
    if !is_hard_local_invalid_state(&assessment.health) {
        let expected_paths = if let Some(remote) = remote.as_ref() {
            remote.expected_paths.as_ref()
        } else {
            cached_expected.as_ref()
        };
        if let Some(expected_paths) = expected_paths {
            apply_expected_validation(
                &mut assessment,
                &snapshot.tracked_paths,
                &ctx.profile.destination,
                expected_paths,
            );
        }
    }

    ctx.emitter.enter_stage(OperationStage::Finalizing);
    ctx.final_report = Some(ProfileStateReport {
        profile_id: assessment.profile_id,
        local_health: assessment.health,
        remote_freshness: remote.map(|r| r.remote_freshness).or({
            if matches!(scope, AssessScope::Remote) {
                Some(RemoteFreshnessState::Unknown)
            } else {
                None
            }
        }),
        checked_at_unix_ms: assessment.checked_at_unix_ms,
        expected_missing_in_inventory_count: assessment.expected_missing_count,
        inventory_unexpected_paths_count: assessment.unexpected_count,
        unexpected_delete_paths: assessment.unexpected_paths,
    });
    ctx.emitter.exit_stage(OperationStage::Finalizing);
    Ok(ctx)
}

#[derive(Clone)]
struct RemoteExpectedState {
    remote_freshness: RemoteFreshnessState,
    expected_paths: Option<BTreeSet<String>>,
}

fn resolve_profile(ctx: &OperationContext) -> Result<ResolvedProfile, LocalStateHealth> {
    let dest_path = ctx
        .profile
        .dest_path()
        .map_err(|_| LocalStateHealth::InvalidProfile)?;
    ctx.profile
        .validated_source_kind()
        .map_err(|_| LocalStateHealth::InvalidProfile)?;
    Ok(ResolvedProfile {
        dest_path,
        paths: fleet_domain::FleetPaths::for_profile(
            ctx.config.profile_state_root_dir.clone(),
            &ctx.profile.id,
        ),
    })
}

fn load_cached_manifest(ctx: &OperationContext) -> Option<BTreeSet<String>> {
    let resolved = ctx.resolved.as_ref()?;
    let ProfileSourceKind::Http(repo_url) = ctx.profile.validated_source_kind().ok()?;
    match fleet_manifest::load_cached_desired_manifest(repo_url, &resolved.paths.profile.repo_cache)
    {
        Ok(Some(manifest)) => Some(manifest_expected_file_paths(&manifest)),
        _ => None,
    }
}

async fn load_remote_state(ctx: &OperationContext) -> RemoteExpectedState {
    let resolved = ctx.resolved.as_ref().expect("resolved");
    let ProfileSourceKind::Http(repo_url) = match ctx.profile.validated_source_kind() {
        Ok(kind) => kind,
        Err(_) => {
            return RemoteExpectedState {
                remote_freshness: RemoteFreshnessState::Error,
                expected_paths: None,
            }
        }
    };

    match fleet_manifest::load_desired_manifest_with_freshness(
        repo_url,
        &resolved.paths.profile.repo_cache,
        &ctx.config.downloads,
        None,
    )
    .await
    {
        Ok(loaded) => RemoteExpectedState {
            remote_freshness: match loaded.freshness {
                fleet_manifest::DesiredManifestFreshness::Unknown => RemoteFreshnessState::Unknown,
                fleet_manifest::DesiredManifestFreshness::UpToDate => {
                    RemoteFreshnessState::UpToDate
                }
                fleet_manifest::DesiredManifestFreshness::UpdateAvailable => {
                    RemoteFreshnessState::UpdateAvailable
                }
            },
            expected_paths: Some(manifest_expected_file_paths(&loaded.manifest)),
        },
        Err(_) => match fleet_manifest::load_cached_desired_manifest(
            repo_url,
            &resolved.paths.profile.repo_cache,
        ) {
            Ok(Some(manifest)) => RemoteExpectedState {
                remote_freshness: RemoteFreshnessState::Error,
                expected_paths: Some(manifest_expected_file_paths(&manifest)),
            },
            _ => RemoteExpectedState {
                remote_freshness: RemoteFreshnessState::Unknown,
                expected_paths: None,
            },
        },
    }
}

async fn evaluate_local_state_snapshot(
    ctx: &OperationContext,
    resolved: &ResolvedProfile,
) -> anyhow::Result<local_state::LocalInventorySnapshot> {
    let cfg = ctx.config.clone();
    let db_path = resolved.paths.profile.inventory.db.clone();
    let dest_path = resolved.dest_path.clone();
    let (scan_tx, mut scan_rx) = mpsc::channel(256);
    let scan_handle = tokio::task::spawn_blocking(move || -> Result<_, InventoryError> {
        let inventory = Inventory::open(&db_path)?;
        local_state::scan_disk_state(
            &inventory,
            &dest_path,
            &cfg.inventory_ignore_rules_text,
            Some(Arc::new(move |progress| {
                let _ = scan_tx.blocking_send(progress);
            })),
        )
    });

    let mut scan_handle = std::pin::pin!(scan_handle);
    let scan_result = loop {
        tokio::select! {
            Some(progress) = scan_rx.recv() => emit_scan_progress(ctx, progress),
            result = &mut scan_handle => {
                match result? {
                    Ok(disk_state) => break Ok(disk_state),
                    Err(err) if err.is_corrupted_database() => {
                        return Ok(local_state::LocalInventorySnapshot {
                            assessment: local_state::LocalStateAssessment {
                                profile_id: ctx.profile.id.clone(),
                                health: LocalStateHealth::InventoryCorrupt,
                                checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
                                expected_missing_count: 0,
                                unexpected_count: 0,
                                unexpected_paths: Vec::new(),
                            },
                            tracked_paths: Vec::new(),
                            missing_tracked_paths: Vec::new(),
                            modified_tracked_paths: Vec::new(),
                        });
                    }
                    Err(err) => break Err(anyhow::Error::new(err)),
                }
            }
        }
    }?;

    ctx.emitter.exit_stage(OperationStage::ScanningDisk);
    ctx.emitter.enter_stage(OperationStage::VerifyingInventory);

    let profile_id = ctx.profile.id.clone();
    let db_path = resolved.paths.profile.inventory.db.clone();
    let (verify_tx, mut verify_rx) = mpsc::channel(256);
    let verify_handle = tokio::task::spawn_blocking(move || -> Result<_, InventoryError> {
        let inventory = Inventory::open(&db_path)?;
        let audit = local_state::verify_trusted_files(
            &inventory,
            scan_result,
            Some(Arc::new(move |progress| {
                let _ = verify_tx.blocking_send(progress);
            })),
        )?;
        let expected_missing_count = audit.missing_finalized.len() as u64;
        let unexpected_count = audit.unexpected_paths.len() as u64;
        let modified = !audit.modified_finalized.is_empty();
        let health = if expected_missing_count > 0 || unexpected_count > 0 || modified {
            LocalStateHealth::LocalDrift
        } else {
            LocalStateHealth::Ready
        };
        Ok(local_state::LocalInventorySnapshot {
            assessment: local_state::LocalStateAssessment {
                profile_id,
                health,
                checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
                expected_missing_count,
                unexpected_count,
                unexpected_paths: audit.unexpected_paths,
            },
            tracked_paths: audit.valid_finalized,
            missing_tracked_paths: audit.missing_finalized,
            modified_tracked_paths: audit.modified_finalized,
        })
    });

    let mut verify_handle = std::pin::pin!(verify_handle);
    loop {
        tokio::select! {
            Some(progress) = verify_rx.recv() => emit_verify_progress(ctx, progress),
            result = &mut verify_handle => {
                match result? {
                    Ok(snapshot) => return Ok(snapshot),
                    Err(err) if err.is_corrupted_database() => {
                        return Ok(local_state::LocalInventorySnapshot {
                            assessment: local_state::LocalStateAssessment {
                                profile_id: ctx.profile.id.clone(),
                                health: LocalStateHealth::InventoryCorrupt,
                                checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
                                expected_missing_count: 0,
                                unexpected_count: 0,
                                unexpected_paths: Vec::new(),
                            },
                            tracked_paths: Vec::new(),
                            missing_tracked_paths: Vec::new(),
                            modified_tracked_paths: Vec::new(),
                        });
                    }
                    Err(err) => return Err(anyhow::Error::new(err)),
                }
            }
        }
    }
}

fn emit_scan_progress(ctx: &OperationContext, progress: local_state::WalkProgress) {
    match progress {
        local_state::WalkProgress::Enumerating {
            _entries_seen: _,
            files_matched,
        } => ctx.emitter.progress_metric(
            OperationStage::ScanningDisk,
            ProgressScope::InventoryEnumerate,
            Some("Enumerating managed files".to_string()),
            ProgressMetric {
                label: Some("Files".to_string()),
                done: Some(files_matched),
                total: None,
                unit: ProgressUnit::Files,
            },
            None,
            None,
            None,
        ),
        local_state::WalkProgress::Metadata {
            files_done,
            files_total,
            bytes_done: _,
            bytes_total: _,
        } => ctx.emitter.progress_metric(
            OperationStage::ScanningDisk,
            ProgressScope::InventoryMetadata,
            Some("Reading file metadata".to_string()),
            ProgressMetric {
                label: Some("Files".to_string()),
                done: Some(files_done),
                total: Some(files_total),
                unit: ProgressUnit::Files,
            },
            None,
            None,
            None,
        ),
    }
}

fn emit_verify_progress(ctx: &OperationContext, progress: local_state::VerifyProgress) {
    let local_state::VerifyProgress {
        files_done,
        files_total,
        missing_count,
        modified_count,
    } = progress;
    ctx.emitter.progress_metric(
        OperationStage::VerifyingInventory,
        ProgressScope::InventoryVerify,
        Some("Comparing tracked inventory against disk state".to_string()),
        ProgressMetric {
            label: Some("Tracked files".to_string()),
            done: Some(files_done),
            total: Some(files_total),
            unit: ProgressUnit::Files,
        },
        Some(ProgressMetric {
            label: Some("Missing + modified".to_string()),
            done: Some(missing_count.saturating_add(modified_count)),
            total: Some(files_total),
            unit: ProgressUnit::Files,
        }),
        None,
        None,
    )
}

fn emit_verify_summary(ctx: &OperationContext, snapshot: &local_state::LocalInventorySnapshot) {
    let files_total = snapshot.tracked_paths.len() as u64
        + snapshot.missing_tracked_paths.len() as u64
        + snapshot.modified_tracked_paths.len() as u64;
    ctx.emitter.progress_metric(
        OperationStage::VerifyingInventory,
        ProgressScope::InventoryVerify,
        Some("Comparing tracked inventory against disk state".to_string()),
        ProgressMetric {
            label: Some("Tracked files".to_string()),
            done: Some(files_total),
            total: Some(files_total),
            unit: ProgressUnit::Files,
        },
        Some(ProgressMetric {
            label: Some("Missing + modified".to_string()),
            done: Some(
                snapshot.missing_tracked_paths.len() as u64
                    + snapshot.modified_tracked_paths.len() as u64,
            ),
            total: Some(files_total),
            unit: ProgressUnit::Files,
        }),
        None,
        None,
    );
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
            ManifestEntry::File(file) => Some(fleet_domain::normalize_rel_slashes(
                file.rel_path.to_string_lossy().as_ref(),
            )),
            _ => None,
        })
        .collect()
}

fn apply_expected_validation(
    assessment: &mut local_state::LocalStateAssessment,
    tracked_paths: &[String],
    destination: &str,
    expected_paths: &BTreeSet<String>,
) {
    let tracked_set = tracked_paths.iter().cloned().collect::<BTreeSet<_>>();
    let expected_missing = expected_paths
        .difference(&tracked_set)
        .cloned()
        .collect::<Vec<_>>();
    let inventory_unexpected = tracked_paths
        .iter()
        .filter(|rel| !expected_paths.contains(rel.as_str()))
        .filter(|rel| {
            !crate::support::prune_policy::is_protected_root_entry(
                Path::new(destination),
                Path::new(rel.as_str()),
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut merged_unexpected = assessment
        .unexpected_paths
        .iter()
        .map(|path| fleet_domain::normalize_rel_slashes(path))
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

fn invalid_report(
    profile_id: &str,
    local_health: LocalStateHealth,
    remote_freshness: Option<RemoteFreshnessState>,
) -> ProfileStateReport {
    ProfileStateReport {
        profile_id: profile_id.to_string(),
        local_health,
        remote_freshness,
        checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        expected_missing_in_inventory_count: 0,
        inventory_unexpected_paths_count: 0,
        unexpected_delete_paths: Vec::new(),
    }
}

fn ensure_not_canceled(ctx: &OperationContext) -> anyhow::Result<()> {
    if ctx.cancel.is_cancelled() {
        return Err(anyhow::Error::new(OperationError::Canceled));
    }
    Ok(())
}
