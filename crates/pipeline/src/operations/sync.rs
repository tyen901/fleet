use crate::api::{OperationStage, ProgressMetric, ProgressScope, ProgressUnit};
use crate::engine::{OperationContext, ResolvedProfile};
use crate::local_state;
use crate::support::locking::{acquire_lock, check_lock_state, InventoryLockState};
use crate::support::repo_cache::{restore_repo_cache_blob, snapshot_repo_cache_blob};
use anyhow::Context;
use fleet_domain::health::{ProfileStateReport, RemoteFreshnessState};
use fleet_domain::{ProfileSourceKind, SyncProgress};
use fleet_inventory::Inventory;
use flux_manifest::ManifestEntry;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) async fn run_sync(mut ctx: OperationContext) -> anyhow::Result<OperationContext> {
    ensure_not_canceled(&ctx)?;
    ctx.emitter.enter_stage(OperationStage::Validating);
    let resolved = resolve_profile(&ctx)?;
    ctx.resolved = Some(resolved.clone());
    ctx.emitter.exit_stage(OperationStage::Validating);

    if matches!(
        check_lock_state(&resolved.paths.profile.inventory.lock).await?,
        InventoryLockState::Locked { .. }
    ) {
        anyhow::bail!("inventory lock is currently held by another running operation");
    }

    let _lock = acquire_lock(resolved.paths.profile.inventory.lock.clone()).await?;
    ctx.repo_cache_snapshot =
        snapshot_repo_cache_blob(&resolved.paths.profile.repo_cache, &ctx.profile).await?;

    let run_result = async {
        ctx.emitter
            .enter_stage(OperationStage::LoadingExpectedState);
        let manifest = load_manifest(&ctx, &resolved).await?;
        ctx.manifest = Some(manifest.clone());
        ctx.emitter.exit_stage(OperationStage::LoadingExpectedState);

        ctx.emitter.enter_stage(OperationStage::PreparingInventory);
        let inventory = open_inventory_for_sync(&resolved.paths.profile.inventory.db)?;
        refresh_inventory_before_sync(&inventory, &ctx, &resolved, &manifest)?;
        ctx.inventory = Some(inventory.clone());
        ctx.emitter.exit_stage(OperationStage::PreparingInventory);

        ctx.emitter.enter_stage(OperationStage::Reconciling);
        let _report = run_reconcile(&ctx, &resolved, manifest.clone()).await?;
        ctx.emitter.exit_stage(OperationStage::Reconciling);

        trim_stale_finalized_rows(
            &inventory,
            &resolved.dest_path,
            &ctx.config.inventory_ignore_rules_text,
        )?;

        ctx.emitter.enter_stage(OperationStage::Pruning);
        let expected_paths = manifest_expected_file_paths(&manifest);
        let delete_paths = inventory
            .finalized_paths()?
            .into_iter()
            .filter(|path| !expected_paths.contains(path))
            .map(PathBuf::from)
            .filter(|path| {
                !crate::support::prune_policy::is_protected_root_entry(&resolved.dest_path, path)
            })
            .collect::<Vec<_>>();
        apply_deletes(&ctx, &resolved, delete_paths.clone()).await?;
        inventory.remove_paths(delete_paths.clone())?;
        ctx.emitter.exit_stage(OperationStage::Pruning);

        ctx.emitter.enter_stage(OperationStage::Auditing);
        let assessment = local_state::assess_snapshot(
            &inventory,
            &ctx.profile.id,
            &resolved.dest_path,
            &ctx.config.inventory_ignore_rules_text,
            Some(Arc::new({
                let emitter = ctx.emitter.clone();
                move |progress| emit_audit_progress(&emitter, progress)
            })),
        )?
        .assessment;
        let unexpected_delete_paths = assessment
            .unexpected_paths
            .iter()
            .map(PathBuf::from)
            .filter(|path| {
                !crate::support::prune_policy::is_protected_root_entry(&resolved.dest_path, path)
            })
            .collect::<Vec<_>>();
        if !unexpected_delete_paths.is_empty() {
            apply_deletes(&ctx, &resolved, unexpected_delete_paths).await?;
            let _ = local_state::assess_snapshot(
                &inventory,
                &ctx.profile.id,
                &resolved.dest_path,
                &ctx.config.inventory_ignore_rules_text,
                Some(Arc::new({
                    let emitter = ctx.emitter.clone();
                    move |progress| emit_audit_progress(&emitter, progress)
                })),
            )?;
        }
        ctx.emitter.exit_stage(OperationStage::Auditing);

        ctx.emitter.enter_stage(OperationStage::Finalizing);
        let mut report = assess_after_sync(&ctx, &resolved).await?;
        report.remote_freshness = Some(RemoteFreshnessState::UpToDate);
        ctx.final_report = Some(report);
        ctx.emitter.exit_stage(OperationStage::Finalizing);
        Ok::<(), anyhow::Error>(())
    }
    .await;

    if let Err(err) = run_result {
        if let Err(restore_err) = restore_repo_cache_blob(ctx.repo_cache_snapshot.clone()).await {
            return Err(restore_err).context(format!(
                "sync failed and repo cache restore also failed after error: {err:#}"
            ));
        }
        return Err(err);
    }

    Ok(ctx)
}

fn resolve_profile(ctx: &OperationContext) -> anyhow::Result<ResolvedProfile> {
    let dest_path = ctx.profile.dest_path()?;
    ctx.profile.validated_source_kind()?;
    Ok(ResolvedProfile {
        dest_path,
        paths: fleet_domain::FleetPaths::for_profile(
            ctx.config.profile_state_root_dir.clone(),
            &ctx.profile.id,
        ),
    })
}

async fn load_manifest(
    ctx: &OperationContext,
    resolved: &ResolvedProfile,
) -> anyhow::Result<fleet_manifest::DesiredManifest> {
    let ProfileSourceKind::Http(repo_url) = ctx.profile.validated_source_kind()?;
    fleet_manifest::load_desired_manifest(
        repo_url,
        &resolved.paths.profile.repo_cache,
        &ctx.config.downloads,
        None,
    )
    .await
}

fn open_inventory_for_sync(db_path: &Path) -> anyhow::Result<Inventory> {
    match Inventory::open(db_path) {
        Ok(inventory) => Ok(inventory),
        Err(err) if err.is_corrupted_database() => {
            if db_path.exists() {
                std::fs::remove_file(db_path)?;
            }
            Ok(Inventory::open(db_path)?)
        }
        Err(err) => Err(anyhow::Error::new(err)),
    }
}

fn refresh_inventory_before_sync(
    inventory: &Inventory,
    ctx: &OperationContext,
    resolved: &ResolvedProfile,
    manifest: &fleet_manifest::DesiredManifest,
) -> anyhow::Result<()> {
    let emitter = ctx.emitter.clone();
    let _ = local_state::refresh_trusted_inventory_from_disk(
        inventory,
        &resolved.dest_path,
        manifest,
        &ctx.config.inventory_ignore_rules_text,
        Some(Arc::new(move |progress| match progress {
            local_state::InventoryRefreshProgress::Walking {
                files_done,
                files_total,
            } => emitter.progress_metric(
                OperationStage::PreparingInventory,
                ProgressScope::InventoryRefresh,
                Some("Walking local managed files".to_string()),
                ProgressMetric {
                    label: Some("Files".to_string()),
                    done: Some(files_done),
                    total: files_total,
                    unit: ProgressUnit::Files,
                },
                None,
                None,
                None,
            ),
            local_state::InventoryRefreshProgress::MatchingTrusted {
                files_done,
                files_total,
            } => emitter.progress_metric(
                OperationStage::PreparingInventory,
                ProgressScope::InventoryRefresh,
                Some("Matching trusted inventory against disk".to_string()),
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
            local_state::InventoryRefreshProgress::Rescanning {
                files_done,
                files_total,
            } => emitter.progress_metric(
                OperationStage::PreparingInventory,
                ProgressScope::InventoryRefresh,
                Some("Rescanning changed trusted files".to_string()),
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
            local_state::InventoryRefreshProgress::Finalizing {
                files_done,
                files_total,
            } => emitter.progress_metric(
                OperationStage::PreparingInventory,
                ProgressScope::InventoryRefresh,
                Some("Finalizing trusted inventory refresh".to_string()),
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
        })),
    )?;
    Ok(())
}

async fn run_reconcile(
    ctx: &OperationContext,
    resolved: &ResolvedProfile,
    manifest: fleet_manifest::DesiredManifest,
) -> anyhow::Result<fleet_reconcile::FluxSyncReport> {
    let engine = fleet_reconcile::FluxEngine::new(resolved.paths.profile.reconcile.cache.clone());
    let emitter = ctx.emitter.clone();
    let progress_sink = Arc::new(move |progress: SyncProgress| {
        emitter.progress_metric(
            OperationStage::Reconciling,
            ProgressScope::ReconcileBytes,
            Some("Downloading and finalizing required files".to_string()),
            ProgressMetric {
                label: Some("Bytes".to_string()),
                done: progress.bytes_done.or(progress.bytes_downloaded),
                total: progress.bytes_total,
                unit: ProgressUnit::Bytes,
            },
            Some(ProgressMetric {
                label: Some("Files".to_string()),
                done: progress.files_finalized,
                total: progress.files_total,
                unit: ProgressUnit::Files,
            }),
            progress.bytes_per_sec,
            progress.eta_seconds,
        );
    });
    engine
        .sync(
            &resolved.dest_path,
            &resolved.paths.profile.inventory.db,
            manifest,
            fleet_reconcile::FluxSyncOptions {
                enable_prune: false,
            },
            ctx.cancel.clone(),
            Some(progress_sink),
        )
        .await
}

fn trim_stale_finalized_rows(
    inventory: &Inventory,
    dest: &Path,
    ignore_rules_text: &str,
) -> anyhow::Result<()> {
    let _ = local_state::trim_stale_trusted_files(inventory, dest, ignore_rules_text, None)?;
    Ok(())
}

async fn apply_deletes(
    ctx: &OperationContext,
    resolved: &ResolvedProfile,
    delete_paths: Vec<PathBuf>,
) -> anyhow::Result<()> {
    if delete_paths.is_empty() {
        return Ok(());
    }
    let dest_path = resolved.dest_path.clone();
    let reconcile_cache = resolved.paths.profile.reconcile.cache.clone();
    let db_path = resolved.paths.profile.inventory.db.clone();
    let total = delete_paths.len() as u64;
    for (index, chunk) in delete_paths.chunks(128).enumerate() {
        let chunk_vec = chunk.to_vec();
        let dest_path = dest_path.clone();
        let reconcile_cache = reconcile_cache.clone();
        let db_path = db_path.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let engine = fleet_reconcile::FluxEngine::new(reconcile_cache);
            engine.prune_only(&dest_path, &db_path, chunk_vec.clone())?;
            let _ = fleet_domain::filesystem::remove_empty_parent_dirs(&dest_path, &chunk_vec)?;
            Ok(())
        })
        .await??;
        let done = ((index + 1) * 128).min(total as usize) as u64;
        ctx.emitter.progress_metric(
            OperationStage::Pruning,
            ProgressScope::Prune,
            Some("Removing managed files that are no longer expected".to_string()),
            ProgressMetric {
                label: Some("Paths".to_string()),
                done: Some(done),
                total: Some(total),
                unit: ProgressUnit::Paths,
            },
            None,
            None,
            None,
        );
    }
    Ok(())
}

fn emit_audit_progress(
    emitter: &crate::engine::EventEmitter,
    progress: local_state::AuditProgress,
) {
    match progress {
        local_state::AuditProgress::Scan(local_state::WalkProgress::Enumerating {
            _entries_seen: _,
            files_matched,
        }) => emitter.progress_metric(
            OperationStage::Auditing,
            ProgressScope::AuditEnumerate,
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
        local_state::AuditProgress::Scan(local_state::WalkProgress::Metadata {
            files_done,
            files_total,
        }) => emitter.progress_metric(
            OperationStage::Auditing,
            ProgressScope::AuditEnumerate,
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
        local_state::AuditProgress::Verify {
            files_done,
            files_total,
            missing_count,
            modified_count,
        } => emitter.progress_metric(
            OperationStage::Auditing,
            ProgressScope::AuditVerify,
            Some("Comparing tracked inventory against disk state".to_string()),
            ProgressMetric {
                label: Some("Tracked files".to_string()),
                done: Some(files_done),
                total: Some(files_total),
                unit: ProgressUnit::Files,
            },
            Some(ProgressMetric {
                label: Some("Missing + modified".to_string()),
                done: Some(missing_count + modified_count),
                total: Some(files_total),
                unit: ProgressUnit::Files,
            }),
            None,
            None,
        ),
    }
}

async fn assess_after_sync(
    ctx: &OperationContext,
    resolved: &ResolvedProfile,
) -> anyhow::Result<ProfileStateReport> {
    let inventory = Inventory::open(&resolved.paths.profile.inventory.db)?;
    let snapshot = local_state::assess_snapshot(
        &inventory,
        &ctx.profile.id,
        &resolved.dest_path,
        &ctx.config.inventory_ignore_rules_text,
        None,
    )?;
    Ok(ProfileStateReport {
        profile_id: ctx.profile.id.clone(),
        local_health: snapshot.assessment.health,
        remote_freshness: Some(RemoteFreshnessState::UpToDate),
        checked_at_unix_ms: snapshot.assessment.checked_at_unix_ms,
        expected_missing_in_inventory_count: snapshot.assessment.expected_missing_count,
        inventory_unexpected_paths_count: snapshot.assessment.unexpected_count,
        unexpected_delete_paths: snapshot.assessment.unexpected_paths,
    })
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

fn ensure_not_canceled(ctx: &OperationContext) -> anyhow::Result<()> {
    if ctx.cancel.is_cancelled() {
        anyhow::bail!("canceled");
    }
    Ok(())
}
