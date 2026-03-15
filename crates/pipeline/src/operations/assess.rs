use crate::api::{OperationOutput, OperationStage, ProgressMetric, ProgressScope, ProgressUnit};
use crate::engine::{OperationContext, ResolvedProfile};
use crate::local_state;
use crate::operations::OperationError;
use crate::support::locking::{check_lock_state, InventoryLockState};
use fleet_domain::health::{InventoryCheckReport, RepoCheckFreshness, RepoCheckReport};
use fleet_domain::LocalStateHealth;
use fleet_domain::ProfileSourceKind;
use fleet_inventory::{Inventory, InventoryError};
use flux_manifest::ManifestEntry;
use std::collections::BTreeSet;
use std::sync::Arc;
use tokio::sync::mpsc;

pub(crate) async fn run_check_repo(mut ctx: OperationContext) -> anyhow::Result<OperationContext> {
    ctx.emitter.enter_stage(OperationStage::Validating);
    let repo_url = match ctx.profile.validated_source_kind() {
        Ok(ProfileSourceKind::Http(repo_url)) => repo_url.to_string(),
        Err(_) => {
            ctx.final_output = Some(OperationOutput::CheckRepo(RepoCheckReport {
                profile_id: ctx.profile.id.clone(),
                local_revision: None,
                remote_revision: None,
                freshness: RepoCheckFreshness::Error,
                checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
            }));
            ctx.emitter.exit_stage(OperationStage::Validating);
            return Ok(ctx);
        }
    };
    ctx.emitter.exit_stage(OperationStage::Validating);

    ctx.emitter
        .enter_stage(OperationStage::LoadingExpectedState);
    let repo_cache_dir =
        fleet_domain::repo_cache_dir(&ctx.config.profile_state_root_dir, &ctx.profile.id);
    let report = match fleet_manifest::probe_desired_manifest_freshness(
        &repo_url,
        &repo_cache_dir,
        &ctx.config.downloads,
        None,
    )
    .await
    {
        Ok(probe) => RepoCheckReport {
            profile_id: ctx.profile.id.clone(),
            local_revision: probe.local_revision,
            remote_revision: probe.remote_revision,
            freshness: match probe.freshness {
                fleet_manifest::DesiredManifestFreshness::Unknown => RepoCheckFreshness::Unknown,
                fleet_manifest::DesiredManifestFreshness::UpToDate => RepoCheckFreshness::UpToDate,
                fleet_manifest::DesiredManifestFreshness::UpdateAvailable => {
                    RepoCheckFreshness::UpdateAvailable
                }
            },
            checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        },
        Err(_) => RepoCheckReport {
            profile_id: ctx.profile.id.clone(),
            local_revision: swifty_repo::load_cached_repo_blocking(&repo_cache_dir, &repo_url)
                .ok()
                .flatten()
                .and_then(|cache| swifty_repo::repo_blob_revision(&cache)),
            remote_revision: None,
            freshness: RepoCheckFreshness::Error,
            checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        },
    };
    ctx.emitter.exit_stage(OperationStage::LoadingExpectedState);

    ctx.emitter.enter_stage(OperationStage::Finalizing);
    ctx.final_output = Some(OperationOutput::CheckRepo(report));
    ctx.emitter.exit_stage(OperationStage::Finalizing);
    Ok(ctx)
}

pub(crate) async fn run_check_inventory(
    mut ctx: OperationContext,
) -> anyhow::Result<OperationContext> {
    ensure_not_canceled(&ctx)?;
    ctx.emitter.enter_stage(OperationStage::Validating);
    let resolved = match resolve_profile(&ctx) {
        Ok(resolved) => resolved,
        Err(local_health) => {
            ctx.final_output = Some(OperationOutput::CheckInventory(invalid_report(
                &ctx.profile.id,
                local_health,
            )));
            ctx.emitter.exit_stage(OperationStage::Validating);
            return Ok(ctx);
        }
    };
    ctx.resolved = Some(resolved.clone());
    ctx.emitter.exit_stage(OperationStage::Validating);

    ctx.emitter
        .enter_stage(OperationStage::LoadingExpectedState);
    let cached_expected = load_cached_manifest(&ctx);
    ctx.emitter.exit_stage(OperationStage::LoadingExpectedState);

    match check_lock_state(&resolved.paths.profile.inventory.lock).await {
        Ok(InventoryLockState::Locked { .. }) => {
            ctx.final_output = Some(OperationOutput::CheckInventory(invalid_report(
                &ctx.profile.id,
                LocalStateHealth::Blocked,
            )));
            return Ok(ctx);
        }
        Ok(InventoryLockState::NotLocked) => {}
        Err(_) => {
            ctx.final_output = Some(OperationOutput::CheckInventory(invalid_report(
                &ctx.profile.id,
                LocalStateHealth::ProbeFailed,
            )));
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
        if let Some(expected_paths) = cached_expected.as_ref() {
            apply_expected_validation(&mut assessment, &snapshot.tracked_paths, expected_paths);
        }
    }

    ctx.emitter.enter_stage(OperationStage::Finalizing);
    ctx.final_output = Some(OperationOutput::CheckInventory(InventoryCheckReport {
        profile_id: assessment.profile_id,
        local_health: assessment.health,
        checked_at_unix_ms: assessment.checked_at_unix_ms,
        expected_missing_in_inventory_count: assessment.expected_missing_count,
        inventory_unexpected_paths_count: assessment.unexpected_count,
        unexpected_delete_paths: assessment.unexpected_paths,
    }));
    ctx.emitter.exit_stage(OperationStage::Finalizing);
    Ok(ctx)
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
            Some("Enumerating files".to_string()),
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
            bytes_done,
            bytes_total,
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
            Some(ProgressMetric {
                label: Some("Bytes".to_string()),
                done: Some(bytes_done),
                total: bytes_total,
                unit: ProgressUnit::Bytes,
            }),
            None,
            None,
        ),
    }
}

fn emit_verify_progress(ctx: &OperationContext, progress: local_state::VerifyProgress) {
    ctx.emitter.progress_metric(
        OperationStage::VerifyingInventory,
        ProgressScope::InventoryVerify,
        Some("Verifying managed files".to_string()),
        ProgressMetric {
            label: Some("Tracked files".to_string()),
            done: Some(progress.files_done),
            total: Some(progress.files_total),
            unit: ProgressUnit::Files,
        },
        Some(ProgressMetric {
            label: Some("Missing + modified".to_string()),
            done: Some(progress.missing_count + progress.modified_count),
            total: Some(progress.files_total),
            unit: ProgressUnit::Files,
        }),
        None,
        None,
    );
}

fn emit_verify_summary(ctx: &OperationContext, snapshot: &local_state::LocalInventorySnapshot) {
    let files_total = snapshot.tracked_paths.len() as u64;
    let problem_count =
        (snapshot.missing_tracked_paths.len() + snapshot.modified_tracked_paths.len()) as u64;
    ctx.emitter.progress_metric(
        OperationStage::VerifyingInventory,
        ProgressScope::InventoryVerify,
        Some("Verifying managed files".to_string()),
        ProgressMetric {
            label: Some("Tracked files".to_string()),
            done: Some(files_total),
            total: Some(files_total),
            unit: ProgressUnit::Files,
        },
        Some(ProgressMetric {
            label: Some("Missing + modified".to_string()),
            done: Some(problem_count),
            total: Some(files_total),
            unit: ProgressUnit::Files,
        }),
        None,
        None,
    );
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
    expected_paths: &BTreeSet<String>,
) {
    let tracked = tracked_paths
        .iter()
        .map(|path| fleet_domain::normalize_rel_slashes(path))
        .collect::<BTreeSet<_>>();
    let expected_missing = expected_paths
        .iter()
        .filter(|path| !tracked.contains(*path))
        .cloned()
        .collect::<Vec<_>>();

    let mut merged_unexpected = assessment
        .unexpected_paths
        .iter()
        .filter_map(|path| {
            let normalized = fleet_domain::normalize_rel_slashes(path);
            (!expected_paths.contains(&normalized)).then_some(path.clone())
        })
        .collect::<Vec<_>>();

    merged_unexpected.extend(
        tracked
            .iter()
            .filter(|path| !expected_paths.contains(*path))
            .cloned(),
    );
    merged_unexpected.sort();
    merged_unexpected.dedup();

    assessment.unexpected_paths = merged_unexpected;
    assessment.unexpected_count = assessment.unexpected_paths.len() as u64;
    assessment.expected_missing_count = expected_missing.len() as u64;
    if assessment.expected_missing_count > 0 || assessment.unexpected_count > 0 {
        assessment.health = LocalStateHealth::LocalDrift;
    }
}

fn invalid_report(profile_id: &str, local_health: LocalStateHealth) -> InventoryCheckReport {
    InventoryCheckReport {
        profile_id: profile_id.to_string(),
        local_health,
        checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        expected_missing_in_inventory_count: 0,
        inventory_unexpected_paths_count: 0,
        unexpected_delete_paths: Vec::new(),
    }
}

fn is_hard_local_invalid_state(local_health: &LocalStateHealth) -> bool {
    matches!(
        local_health,
        LocalStateHealth::Blocked
            | LocalStateHealth::InvalidProfile
            | LocalStateHealth::ProbeFailed
            | LocalStateHealth::InventoryCorrupt
    )
}

fn ensure_not_canceled(ctx: &OperationContext) -> anyhow::Result<()> {
    if ctx.cancel.is_cancelled() {
        return Err(anyhow::Error::new(OperationError::Canceled));
    }
    Ok(())
}
