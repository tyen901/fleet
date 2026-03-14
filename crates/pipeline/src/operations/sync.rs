use crate::api::{OperationStage, ProgressMetric, ProgressScope, ProgressUnit};
use crate::engine::{OperationContext, ResolvedProfile};
use crate::local_state;
use crate::operations::OperationError;
use crate::support::locking::FileLockGuard;
use crate::support::locking::{acquire_lock, check_lock_state, InventoryLockState};
use crate::support::repo_cache::{restore_repo_cache_blob, snapshot_repo_cache_blob};
use fleet_domain::health::{ProfileStateReport, RemoteFreshnessState};
use fleet_domain::{ProfileSourceKind, SyncProgress, ThroughputEstimator};
use fleet_inventory::{Inventory, InventoryError};
use flux_manifest::ManifestEntry;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
enum InventoryRefreshPhase {
    Walking,
    MatchingTrusted,
    Rescanning,
    Finalizing,
}

struct InventoryRefreshRateState {
    phase: Option<InventoryRefreshPhase>,
    estimator: ThroughputEstimator,
}

impl InventoryRefreshRateState {
    fn new() -> Self {
        Self {
            phase: None,
            estimator: ThroughputEstimator::new(Instant::now()),
        }
    }

    fn update(
        &mut self,
        phase: InventoryRefreshPhase,
        bytes_done: u64,
        bytes_total: Option<u64>,
    ) -> (Option<u64>, Option<u64>) {
        let now = Instant::now();
        if self.phase != Some(phase) {
            self.phase = Some(phase);
            self.estimator = ThroughputEstimator::new(now);
        }
        self.estimator.record(bytes_done, now);
        let throughput_bytes_per_sec = self
            .estimator
            .bytes_per_sec(now)
            .map(|rate| rate.round() as u64)
            .filter(|rate| *rate > 0);
        let eta_seconds =
            bytes_total.and_then(|total| self.estimator.eta_seconds(bytes_done, total, now));
        (throughput_bytes_per_sec, eta_seconds)
    }
}

struct ResolvedSyncContext {
    resolved: ResolvedProfile,
    _lock_guard: FileLockGuard,
}

struct ExpectedState {
    manifest: fleet_manifest::DesiredManifest,
    expected_paths: BTreeSet<String>,
}

struct PreparedInventory {
    inventory: Inventory,
}

struct PrunePlan {
    delete_paths: Vec<PathBuf>,
}

pub(crate) async fn run_sync(mut ctx: OperationContext) -> anyhow::Result<OperationContext> {
    ensure_not_canceled(&ctx)?;
    ctx.emitter.enter_stage(OperationStage::Validating);
    let resolved_ctx = resolve_and_lock_sync_context(&ctx).await?;
    ctx.resolved = Some(resolved_ctx.resolved.clone());
    ctx.emitter.exit_stage(OperationStage::Validating);

    let run_result = async {
        let expected_state =
            load_expected_state_with_repo_cache_snapshot(&mut ctx, &resolved_ctx).await?;
        let prepared_inventory =
            open_and_prepare_inventory(&mut ctx, &resolved_ctx, &expected_state)?;
        run_reconcile_phase(&ctx, &resolved_ctx, &expected_state).await?;
        trim_stale_trusted_rows(&resolved_ctx, &prepared_inventory, &ctx)?;
        let _prune_plan =
            plan_and_apply_prune(&ctx, &resolved_ctx, &expected_state, &prepared_inventory).await?;
        audit_and_clean_unexpected(&ctx, &resolved_ctx, &prepared_inventory).await?;
        finalize_sync_report(&mut ctx, &resolved_ctx).await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;

    if let Err(err) = run_result {
        return Err(restore_repo_cache_after_failure(ctx.repo_cache_snapshot.clone(), err).await);
    }

    Ok(ctx)
}

async fn resolve_and_lock_sync_context(
    ctx: &OperationContext,
) -> anyhow::Result<ResolvedSyncContext> {
    let resolved = resolve_profile(ctx)?;
    match check_lock_state(&resolved.paths.profile.inventory.lock).await {
        Ok(InventoryLockState::Locked { .. }) => {
            return Err(anyhow::Error::new(OperationError::InventoryLocked));
        }
        Ok(InventoryLockState::NotLocked) => {}
        Err(err) => return Err(map_inventory_error(err)),
    }
    let lock_guard = acquire_lock(resolved.paths.profile.inventory.lock.clone())
        .await
        .map_err(map_inventory_error)?;
    Ok(ResolvedSyncContext {
        resolved,
        _lock_guard: lock_guard,
    })
}

fn resolve_profile(ctx: &OperationContext) -> anyhow::Result<ResolvedProfile> {
    let dest_path = ctx.profile.dest_path()?;
    ctx.profile
        .validated_source_kind()
        .map_err(|_| anyhow::Error::new(OperationError::InvalidProfile))?;
    Ok(ResolvedProfile {
        dest_path,
        paths: fleet_domain::FleetPaths::for_profile(
            ctx.config.profile_state_root_dir.clone(),
            &ctx.profile.id,
        ),
    })
}

async fn load_expected_state_with_repo_cache_snapshot(
    ctx: &mut OperationContext,
    resolved_ctx: &ResolvedSyncContext,
) -> anyhow::Result<ExpectedState> {
    ctx.emitter
        .enter_stage(OperationStage::LoadingExpectedState);
    ctx.repo_cache_snapshot = snapshot_repo_cache_blob(
        &resolved_ctx.resolved.paths.profile.repo_cache,
        &ctx.profile,
    )
    .await?;
    let manifest = load_manifest(ctx, &resolved_ctx.resolved).await?;
    let expected_paths = manifest_expected_file_paths(&manifest);
    ctx.manifest = Some(manifest.clone());
    ctx.emitter.exit_stage(OperationStage::LoadingExpectedState);
    Ok(ExpectedState {
        manifest,
        expected_paths,
    })
}

async fn load_manifest(
    ctx: &OperationContext,
    resolved: &ResolvedProfile,
) -> anyhow::Result<fleet_manifest::DesiredManifest> {
    let ProfileSourceKind::Http(repo_url) = ctx
        .profile
        .validated_source_kind()
        .map_err(|_| anyhow::Error::new(OperationError::InvalidProfile))?;
    fleet_manifest::load_desired_manifest(
        repo_url,
        &resolved.paths.profile.repo_cache,
        &ctx.config.downloads,
        None,
    )
    .await
}

fn open_and_prepare_inventory(
    ctx: &mut OperationContext,
    resolved_ctx: &ResolvedSyncContext,
    expected_state: &ExpectedState,
) -> anyhow::Result<PreparedInventory> {
    ctx.emitter.enter_stage(OperationStage::PreparingInventory);
    let inventory = open_inventory_for_sync(&resolved_ctx.resolved.paths.profile.inventory.db)?;
    refresh_inventory_before_sync(
        &inventory,
        ctx,
        &resolved_ctx.resolved,
        &expected_state.manifest,
    )?;
    ctx.inventory = Some(inventory.clone());
    ctx.emitter.exit_stage(OperationStage::PreparingInventory);
    Ok(PreparedInventory { inventory })
}

fn open_inventory_for_sync(db_path: &Path) -> anyhow::Result<Inventory> {
    match Inventory::open(db_path) {
        Ok(inventory) => Ok(inventory),
        Err(err) if err.is_corrupted_database() => {
            if db_path.exists() {
                std::fs::remove_file(db_path)?;
            }
            Inventory::open(db_path).map_err(map_inventory_error)
        }
        Err(err) => Err(map_inventory_error(err)),
    }
}

async fn run_reconcile_phase(
    ctx: &OperationContext,
    resolved_ctx: &ResolvedSyncContext,
    expected_state: &ExpectedState,
) -> anyhow::Result<()> {
    ctx.emitter.enter_stage(OperationStage::Reconciling);
    let _report =
        run_reconcile(ctx, &resolved_ctx.resolved, expected_state.manifest.clone()).await?;
    ctx.emitter.exit_stage(OperationStage::Reconciling);
    Ok(())
}

fn trim_stale_trusted_rows(
    resolved_ctx: &ResolvedSyncContext,
    prepared_inventory: &PreparedInventory,
    ctx: &OperationContext,
) -> anyhow::Result<()> {
    let _ = local_state::trim_stale_trusted_files(
        &prepared_inventory.inventory,
        &resolved_ctx.resolved.dest_path,
        &ctx.config.inventory_ignore_rules_text,
        None,
    )
    .map_err(map_inventory_error)?;
    Ok(())
}

async fn plan_and_apply_prune(
    ctx: &OperationContext,
    resolved_ctx: &ResolvedSyncContext,
    expected_state: &ExpectedState,
    prepared_inventory: &PreparedInventory,
) -> anyhow::Result<PrunePlan> {
    ctx.emitter.enter_stage(OperationStage::Pruning);
    let prune_plan = PrunePlan {
        delete_paths: prepared_inventory
            .inventory
            .finalized_paths()
            .map_err(map_inventory_error)?
            .into_iter()
            .filter(|path| !expected_state.expected_paths.contains(path))
            .map(PathBuf::from)
            .filter(|path| {
                !crate::support::prune_policy::is_protected_root_entry(
                    &resolved_ctx.resolved.dest_path,
                    path,
                )
            })
            .collect(),
    };
    apply_deletes(ctx, &resolved_ctx.resolved, prune_plan.delete_paths.clone()).await?;
    prepared_inventory
        .inventory
        .remove_paths(prune_plan.delete_paths.clone())
        .map_err(map_inventory_error)?;
    ctx.emitter.exit_stage(OperationStage::Pruning);
    Ok(prune_plan)
}

async fn audit_and_clean_unexpected(
    ctx: &OperationContext,
    resolved_ctx: &ResolvedSyncContext,
    prepared_inventory: &PreparedInventory,
) -> anyhow::Result<()> {
    ctx.emitter.enter_stage(OperationStage::Auditing);
    let assessment = audit_snapshot(ctx, resolved_ctx, prepared_inventory)?.assessment;
    let unexpected_delete_paths = assessment
        .unexpected_paths
        .iter()
        .map(PathBuf::from)
        .filter(|path| {
            !crate::support::prune_policy::is_protected_root_entry(
                &resolved_ctx.resolved.dest_path,
                path,
            )
        })
        .collect::<Vec<_>>();
    if !unexpected_delete_paths.is_empty() {
        apply_deletes(ctx, &resolved_ctx.resolved, unexpected_delete_paths).await?;
        let _ = audit_snapshot(ctx, resolved_ctx, prepared_inventory)?;
    }
    ctx.emitter.exit_stage(OperationStage::Auditing);
    Ok(())
}

fn audit_snapshot(
    ctx: &OperationContext,
    resolved_ctx: &ResolvedSyncContext,
    prepared_inventory: &PreparedInventory,
) -> anyhow::Result<local_state::LocalInventorySnapshot> {
    local_state::assess_snapshot(
        &prepared_inventory.inventory,
        &ctx.profile.id,
        &resolved_ctx.resolved.dest_path,
        &ctx.config.inventory_ignore_rules_text,
        Some(Arc::new({
            let emitter = ctx.emitter.clone();
            move |progress| emit_audit_progress(&emitter, progress)
        })),
    )
    .map_err(map_inventory_error)
}

async fn finalize_sync_report(
    ctx: &mut OperationContext,
    resolved_ctx: &ResolvedSyncContext,
) -> anyhow::Result<()> {
    ctx.emitter.enter_stage(OperationStage::Finalizing);
    let mut report = assess_after_sync(ctx, &resolved_ctx.resolved).await?;
    report.remote_freshness = Some(RemoteFreshnessState::UpToDate);
    ctx.final_report = Some(report);
    ctx.emitter.exit_stage(OperationStage::Finalizing);
    Ok(())
}

fn refresh_inventory_before_sync(
    inventory: &Inventory,
    ctx: &OperationContext,
    resolved: &ResolvedProfile,
    manifest: &fleet_manifest::DesiredManifest,
) -> anyhow::Result<()> {
    let emitter = ctx.emitter.clone();
    let rate_state = Arc::new(Mutex::new(InventoryRefreshRateState::new()));
    let _ = local_state::refresh_trusted_inventory_from_disk(
        inventory,
        &resolved.dest_path,
        manifest,
        &ctx.config.inventory_ignore_rules_text,
        Some(Arc::new(move |progress| {
            let (phase, status_text, files_done, files_total, bytes_done, bytes_total) =
                match progress {
                    local_state::InventoryRefreshProgress::Walking {
                        files_done,
                        files_total,
                        bytes_done,
                        bytes_total,
                    } => (
                        InventoryRefreshPhase::Walking,
                        "Reading file metadata".to_string(),
                        files_done,
                        files_total,
                        bytes_done,
                        bytes_total,
                    ),
                    local_state::InventoryRefreshProgress::MatchingTrusted {
                        files_done,
                        files_total,
                        bytes_done,
                        bytes_total,
                    } => (
                        InventoryRefreshPhase::MatchingTrusted,
                        "Matching trusted inventory against disk".to_string(),
                        files_done,
                        Some(files_total),
                        bytes_done,
                        Some(bytes_total),
                    ),
                    local_state::InventoryRefreshProgress::Rescanning {
                        files_done,
                        files_total,
                        bytes_done,
                        bytes_total,
                    } => (
                        InventoryRefreshPhase::Rescanning,
                        "Rescanning changed trusted files".to_string(),
                        files_done,
                        Some(files_total),
                        bytes_done,
                        Some(bytes_total),
                    ),
                    local_state::InventoryRefreshProgress::Finalizing {
                        files_done,
                        files_total,
                        bytes_done,
                        bytes_total,
                    } => (
                        InventoryRefreshPhase::Finalizing,
                        "Finalizing trusted inventory refresh".to_string(),
                        files_done,
                        Some(files_total),
                        bytes_done,
                        Some(bytes_total),
                    ),
                };

            let (throughput_bytes_per_sec, eta_seconds) = match rate_state.lock() {
                Ok(mut state) => state.update(phase, bytes_done, bytes_total),
                Err(_) => (None, None),
            };

            emitter.progress_metric(
                OperationStage::PreparingInventory,
                ProgressScope::InventoryRefresh,
                Some(status_text),
                ProgressMetric {
                    label: Some("Files".to_string()),
                    done: Some(files_done),
                    total: files_total,
                    unit: ProgressUnit::Files,
                },
                if bytes_done > 0 || bytes_total.is_some() {
                    Some(ProgressMetric {
                        label: Some("Bytes".to_string()),
                        done: Some(bytes_done),
                        total: bytes_total,
                        unit: ProgressUnit::Bytes,
                    })
                } else {
                    None
                },
                throughput_bytes_per_sec,
                eta_seconds,
            );
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
        .map_err(|err| {
            if ctx.cancel.is_cancelled() {
                anyhow::Error::new(OperationError::Canceled)
            } else {
                err
            }
        })
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
            bytes_done: _,
            bytes_total: _,
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
    let inventory =
        Inventory::open(&resolved.paths.profile.inventory.db).map_err(map_inventory_error)?;
    let snapshot = local_state::assess_snapshot(
        &inventory,
        &ctx.profile.id,
        &resolved.dest_path,
        &ctx.config.inventory_ignore_rules_text,
        None,
    )
    .map_err(map_inventory_error)?;
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
        return Err(anyhow::Error::new(OperationError::Canceled));
    }
    Ok(())
}

async fn restore_repo_cache_after_failure(
    snapshot: Option<crate::support::repo_cache::RepoCacheSnapshot>,
    err: anyhow::Error,
) -> anyhow::Error {
    if let Err(restore_err) = restore_repo_cache_blob(snapshot).await {
        return restore_err.context(format!(
            "sync failed and repo cache restore also failed after error: {err:#}"
        ));
    }
    err
}

fn map_inventory_error(err: InventoryError) -> anyhow::Error {
    match err {
        InventoryError::CorruptDatabase => anyhow::Error::new(OperationError::InventoryCorrupt),
        InventoryError::Locked => anyhow::Error::new(OperationError::InventoryLocked),
        other => anyhow::Error::new(other),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        audit_and_clean_unexpected, open_inventory_for_sync, plan_and_apply_prune,
        resolve_and_lock_sync_context, ExpectedState, PreparedInventory,
    };
    use crate::config::PipelineConfig;
    use crate::engine::{EventEmitter, OperationContext, SessionControl};
    use crate::local_state;
    use fleet_domain::health::OperationKind;
    use fleet_domain::{inventory_db_path, Profile};
    use fleet_inventory::Inventory;
    use flux_inventory_contract::CommittedFileRecord;
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use tokio::sync::broadcast;
    use tokio_util::sync::CancellationToken;

    struct SyncTestFixture {
        _tempdir: tempfile::TempDir,
        dest: PathBuf,
        profile: Profile,
        config: PipelineConfig,
        inventory: Inventory,
    }

    impl SyncTestFixture {
        fn new() -> Self {
            let tempdir = tempfile::tempdir().expect("tempdir");
            let state_root = tempdir.path().join("state");
            let dest = tempdir.path().join("dest");
            std::fs::create_dir_all(&dest).expect("create dest");
            std::fs::create_dir_all(&state_root).expect("create state root");

            let profile = Profile {
                id: "p1".to_string(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: dest.to_string_lossy().to_string(),
                ..Default::default()
            };
            let db_path = inventory_db_path(&state_root, &profile.id);
            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent).expect("create inventory dir");
            }
            let inventory = Inventory::open(&db_path).expect("open inventory");

            let mut config = PipelineConfig::new_default();
            config.profile_state_root_dir = state_root.clone();

            Self {
                _tempdir: tempdir,
                dest,
                profile,
                config,
                inventory,
            }
        }

        fn context(&self) -> OperationContext {
            let (tx, _) = broadcast::channel(32);
            let session_id = 7;
            let operation = OperationKind::Sync;
            OperationContext::new(
                session_id,
                self.profile.clone(),
                operation,
                self.config.clone(),
                SessionControl {
                    cancel: CancellationToken::new(),
                    emitter: EventEmitter::new(tx, session_id, self.profile.id.clone(), operation),
                },
            )
        }

        fn write_file(&self, rel_path: &str, contents: &[u8]) {
            let fs_path = self.dest.join(rel_path);
            if let Some(parent) = fs_path.parent() {
                std::fs::create_dir_all(parent).expect("create parent");
            }
            std::fs::write(fs_path, contents).expect("write file");
        }

        fn seed_inventory(&self, rel_paths: &[&str]) {
            let records = rel_paths
                .iter()
                .map(|rel_path| self.committed_record(rel_path))
                .collect::<Vec<_>>();
            self.inventory
                .upsert_trusted_files_batch(&records)
                .expect("seed inventory");
            self.inventory
                .initialize_trusted_baseline()
                .expect("initialize baseline");
        }

        fn committed_record(&self, rel_path: &str) -> CommittedFileRecord {
            let fs_path = self.dest.join(rel_path);
            let metadata = std::fs::metadata(&fs_path).expect("metadata");
            CommittedFileRecord {
                rel_path: PathBuf::from(rel_path),
                size_bytes: metadata.len(),
                mtime_ns: metadata_mtime_ns(&metadata),
                segments: Vec::new(),
            }
        }
    }

    fn metadata_mtime_ns(metadata: &std::fs::Metadata) -> u64 {
        metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn prune_updates_disk_and_inventory_together() {
        let fixture = SyncTestFixture::new();
        fixture.write_file("mods/keep.pbo", b"keep");
        fixture.write_file("mods/stale.pbo", b"stale");
        fixture.seed_inventory(&["mods/keep.pbo", "mods/stale.pbo"]);

        let ctx = fixture.context();
        let resolved_ctx = resolve_and_lock_sync_context(&ctx)
            .await
            .expect("resolved sync context");
        let prepared_inventory = PreparedInventory {
            inventory: fixture.inventory.clone(),
        };
        let expected_state = ExpectedState {
            manifest: flux_manifest::DesiredManifest {
                version: flux_manifest::ManifestVersion::V1,
                entries: Vec::new(),
                prune_paths: Vec::new(),
            },
            expected_paths: BTreeSet::from(["mods/keep.pbo".to_string()]),
        };

        let prune_plan =
            plan_and_apply_prune(&ctx, &resolved_ctx, &expected_state, &prepared_inventory)
                .await
                .expect("apply prune");

        assert_eq!(
            prune_plan.delete_paths,
            vec![PathBuf::from("mods/stale.pbo")]
        );
        assert!(!fixture.dest.join("mods/stale.pbo").exists());
        assert_eq!(
            prepared_inventory
                .inventory
                .finalized_paths()
                .expect("finalized paths"),
            vec!["mods/keep.pbo"]
        );
    }

    #[tokio::test]
    async fn audit_cleanup_removes_unexpected_files_without_stale_inventory_rows() {
        let fixture = SyncTestFixture::new();
        fixture.write_file("mods/tracked.pbo", b"tracked");
        fixture.write_file("mods/rogue.pbo", b"rogue");
        fixture.seed_inventory(&["mods/tracked.pbo"]);

        let ctx = fixture.context();
        let resolved_ctx = resolve_and_lock_sync_context(&ctx)
            .await
            .expect("resolved sync context");
        let prepared_inventory = PreparedInventory {
            inventory: fixture.inventory.clone(),
        };

        audit_and_clean_unexpected(&ctx, &resolved_ctx, &prepared_inventory)
            .await
            .expect("audit cleanup");

        assert!(!fixture.dest.join("mods/rogue.pbo").exists());
        let snapshot = local_state::assess_snapshot(
            &prepared_inventory.inventory,
            &fixture.profile.id,
            &fixture.dest,
            "",
            None,
        )
        .expect("assess snapshot");
        assert_eq!(snapshot.assessment.unexpected_count, 0);
        assert_eq!(
            prepared_inventory
                .inventory
                .finalized_paths()
                .expect("finalized paths"),
            vec!["mods/tracked.pbo"]
        );
    }

    #[test]
    fn open_inventory_for_sync_recovers_corrupt_database() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let db_path = tempdir.path().join("inventory.sqlite");
        std::fs::write(&db_path, b"not-a-sqlite-db").expect("write corrupt db");

        let inventory = open_inventory_for_sync(&db_path).expect("recover inventory");

        assert!(inventory
            .finalized_paths()
            .expect("finalized paths")
            .is_empty());
        assert!(db_path.exists());
    }
}
