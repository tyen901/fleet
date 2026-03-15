use crate::api::{OperationOutput, OperationStage};
use crate::engine::{OperationContext, ResolvedProfile};
use crate::local_state;
use crate::operations::OperationError;
use crate::support::locking::FileLockGuard;
use crate::support::locking::{acquire_lock, check_lock_state, InventoryLockState};
use fleet_domain::health::InventoryCheckReport;
use fleet_domain::LocalStateHealth;
use fleet_inventory::Inventory;
use std::path::PathBuf;

struct ResolvedDeleteContext {
    resolved: ResolvedProfile,
    _lock_guard: FileLockGuard,
}

pub(crate) async fn run_delete(mut ctx: OperationContext) -> anyhow::Result<OperationContext> {
    super::assess::ensure_not_canceled(&ctx)?;
    ctx.emitter.enter_stage(OperationStage::Validating);
    let resolved_ctx = resolve_and_lock_delete_context(&ctx).await?;
    ctx.resolved = Some(resolved_ctx.resolved.clone());
    ctx.emitter.exit_stage(OperationStage::Validating);

    ctx.emitter
        .enter_stage(OperationStage::LoadingExpectedState);
    let Some(expected_paths) = super::assess::load_cached_manifest(&ctx) else {
        ctx.emitter.exit_stage(OperationStage::LoadingExpectedState);
        return Err(anyhow::Error::new(OperationError::MissingCachedManifest));
    };
    ctx.emitter.exit_stage(OperationStage::LoadingExpectedState);

    ctx.emitter.enter_stage(OperationStage::ScanningDisk);
    let snapshot =
        super::assess::evaluate_local_state_snapshot(&ctx, &resolved_ctx.resolved).await?;
    ctx.tracked_paths = snapshot.tracked_paths.clone();
    ctx.emitter.exit_stage(OperationStage::VerifyingInventory);

    let inventory = Inventory::open(&resolved_ctx.resolved.paths.profile.inventory.db)
        .map_err(super::sync::map_inventory_error)?;
    ctx.inventory = Some(inventory.clone());

    let cleanup = if is_delete_blocked(&snapshot.assessment.health) {
        super::assess::ManifestCleanupAssessment::default()
    } else {
        super::assess::manifest_cleanup_assessment(&snapshot, Some(&expected_paths))
    };

    let delete_paths = cleanup
        .delete_candidates
        .iter()
        .map(PathBuf::from)
        .filter(|path| {
            !crate::support::prune_policy::is_protected_root_entry(
                &resolved_ctx.resolved.dest_path,
                path,
            )
        })
        .collect::<Vec<_>>();

    if !delete_paths.is_empty() {
        ctx.emitter.enter_stage(OperationStage::Pruning);
        super::sync::apply_deletes(&ctx, &resolved_ctx.resolved, delete_paths.clone()).await?;
        inventory
            .remove_paths(delete_paths.into_iter())
            .map_err(super::sync::map_inventory_error)?;
        ctx.emitter.exit_stage(OperationStage::Pruning);
    }

    ctx.emitter.enter_stage(OperationStage::Finalizing);
    let report = finalize_delete_report(&ctx, &resolved_ctx.resolved, &expected_paths).await?;
    ctx.final_output = Some(OperationOutput::Delete(report));
    ctx.emitter.exit_stage(OperationStage::Finalizing);
    Ok(ctx)
}

async fn finalize_delete_report(
    ctx: &OperationContext,
    resolved: &ResolvedProfile,
    expected_paths: &std::collections::BTreeSet<String>,
) -> anyhow::Result<InventoryCheckReport> {
    let profile_id = ctx.profile.id.clone();
    let db_path = resolved.paths.profile.inventory.db.clone();
    let dest_path = resolved.dest_path.clone();
    let ignore_rules = ctx.config.inventory_ignore_rules_text.clone();
    let expected_paths = expected_paths.clone();

    tokio::task::spawn_blocking(move || -> anyhow::Result<InventoryCheckReport> {
        let inventory = Inventory::open(&db_path).map_err(super::sync::map_inventory_error)?;
        let snapshot =
            local_state::assess_snapshot(&inventory, &profile_id, &dest_path, &ignore_rules, None)
                .map_err(super::sync::map_inventory_error)?;
        let cleanup = if is_delete_blocked(&snapshot.assessment.health) {
            super::assess::ManifestCleanupAssessment::default()
        } else {
            super::assess::manifest_cleanup_assessment(&snapshot, Some(&expected_paths))
        };
        Ok(super::assess::build_inventory_check_report(
            &snapshot, cleanup,
        ))
    })
    .await?
}

async fn resolve_and_lock_delete_context(
    ctx: &OperationContext,
) -> anyhow::Result<ResolvedDeleteContext> {
    let resolved = resolve_profile(ctx)?;
    match check_lock_state(&resolved.paths.profile.inventory.lock).await {
        Ok(InventoryLockState::Locked { .. }) => {
            return Err(anyhow::Error::new(OperationError::InventoryLocked));
        }
        Ok(InventoryLockState::NotLocked) => {}
        Err(err) => return Err(super::sync::map_inventory_error(err)),
    }
    let lock_guard = acquire_lock(resolved.paths.profile.inventory.lock.clone())
        .await
        .map_err(super::sync::map_inventory_error)?;
    Ok(ResolvedDeleteContext {
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

fn is_delete_blocked(local_health: &LocalStateHealth) -> bool {
    matches!(
        local_health,
        LocalStateHealth::Blocked
            | LocalStateHealth::InvalidProfile
            | LocalStateHealth::ProbeFailed
            | LocalStateHealth::InventoryCorrupt
            | LocalStateHealth::LocalStateMissing
            | LocalStateHealth::MissingDestination
    )
}

#[cfg(test)]
mod tests {
    use super::run_delete;
    use crate::api::OperationOutput;
    use crate::config::PipelineConfig;
    use crate::engine::{EventEmitter, OperationContext, SessionControl};
    use fleet_domain::health::OperationKind;
    use fleet_domain::{inventory_db_path, Profile};
    use fleet_inventory::Inventory;
    use flux_inventory_contract::CommittedFileRecord;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use tokio::sync::broadcast;
    use tokio_util::sync::CancellationToken;

    struct DeleteTestFixture {
        _tempdir: tempfile::TempDir,
        dest: PathBuf,
        profile: Profile,
        config: PipelineConfig,
        inventory: Inventory,
    }

    impl DeleteTestFixture {
        fn new() -> Self {
            let tempdir = tempfile::tempdir().expect("tempdir");
            let state_root = tempdir.path().join("state");
            let dest = tempdir.path().join("dest");
            let repo_cache = fleet_domain::repo_cache_dir(&state_root, "p1");
            std::fs::create_dir_all(&dest).expect("create dest");
            std::fs::create_dir_all(&repo_cache).expect("create repo cache");

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
            config.profile_state_root_dir = state_root;

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
            let operation = OperationKind::Delete;
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
                mtime_ns: metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_nanos() as u64)
                    .unwrap_or_default(),
                segments: Vec::new(),
            }
        }

        fn write_cached_manifest(&self, rel_paths: &[&str]) {
            let repo_cache =
                fleet_domain::repo_cache_dir(&self.config.profile_state_root_dir, &self.profile.id);
            let source_url = self.profile.source.as_str();
            let cache_blob = swifty_repo::RepoCacheBlob {
                schema_version: 1,
                repo_url: source_url.to_string(),
                repo_fetched_at_unix_ms: 1,
                repo: base_repo_spec("@mod"),
                mods: BTreeMap::from([(
                    "@mod".to_string(),
                    swifty_repo::CachedModSrf {
                        checksum: zero_md5(),
                        fetched_at_unix_ms: 1,
                        manifest: srf_mod(
                            "@mod",
                            &rel_paths
                                .iter()
                                .map(|path| path.trim_start_matches("@mod/"))
                                .collect::<Vec<_>>(),
                        ),
                        http: None,
                    },
                )]),
                repo_http: None,
                icon_image_checksum: None,
                repo_image_checksum: None,
                repo_json_checksum: None,
            };
            write_cached_repo_blob(&repo_cache, source_url, &cache_blob);
        }
    }

    #[tokio::test]
    async fn delete_removes_rogue_and_stale_tracked_paths() {
        let fixture = DeleteTestFixture::new();
        fixture.write_file("@mod/keep.pbo", b"keep");
        fixture.write_file("@mod/stale.pbo", b"stale");
        fixture.write_file("@mod/rogue.pbo", b"rogue");
        fixture.seed_inventory(&["@mod/keep.pbo", "@mod/stale.pbo"]);
        fixture.write_cached_manifest(&["@mod/keep.pbo"]);

        let ctx = run_delete(fixture.context()).await.expect("delete run");
        let report = match ctx.final_output.expect("output") {
            OperationOutput::Delete(report) => report,
            other => panic!("unexpected output: {other:?}"),
        };

        assert!(!fixture.dest.join("@mod/stale.pbo").exists());
        assert!(!fixture.dest.join("@mod/rogue.pbo").exists());
        assert_eq!(report.inventory_unexpected_paths_count, 0);
        assert_eq!(report.unexpected_delete_paths, Vec::<String>::new());
        assert_eq!(
            fixture
                .inventory
                .finalized_paths()
                .expect("finalized paths"),
            vec!["@mod/keep.pbo"]
        );
    }

    #[tokio::test]
    async fn delete_requires_cached_manifest() {
        let fixture = DeleteTestFixture::new();
        fixture.write_file("@mod/rogue.pbo", b"rogue");
        fixture.seed_inventory(&[]);

        let err = match run_delete(fixture.context()).await {
            Ok(_) => panic!("expected missing cache error"),
            Err(err) => err,
        };
        let op_err = crate::operations::find_operation_error(&err).expect("operation error");
        assert_eq!(op_err.api_error().code, "missing_cached_manifest");
    }

    fn zero_md5() -> swifty_artifacts::Md5Digest {
        swifty_artifacts::Md5Digest::parse_hex("00000000000000000000000000000000")
            .expect("valid md5")
    }

    fn base_repo_spec(mod_name: &str) -> swifty_artifacts::RepoSpec {
        swifty_artifacts::RepoSpec {
            repo_name: "test".to_string(),
            checksum: "deadbeef".to_string(),
            required_mods: vec![swifty_artifacts::RepoMod {
                mod_name: mod_name.to_string(),
                checksum: zero_md5(),
                enabled: true,
            }],
            optional_mods: vec![],
            icon_image_path: None,
            icon_image_checksum: None,
            repo_image_path: None,
            repo_image_checksum: None,
            required_dlcs: vec![],
            client_parameters: String::new(),
            repo_basic_authentication: None,
            version: String::new(),
            servers: vec![],
        }
    }

    fn srf_mod(mod_name: &str, file_paths: &[&str]) -> swifty_artifacts::SrfMod {
        swifty_artifacts::SrfMod {
            name: mod_name.to_string(),
            checksum: zero_md5(),
            files: file_paths
                .iter()
                .map(|path| swifty_artifacts::SrfFile {
                    path: (*path).to_string(),
                    length: 0,
                    checksum: zero_md5(),
                    r#type: None,
                    parts: vec![],
                })
                .collect(),
        }
    }

    fn write_cached_repo_blob(
        cache_root: &std::path::Path,
        repo_url: &str,
        blob: &swifty_repo::RepoCacheBlob,
    ) {
        std::fs::create_dir_all(cache_root).expect("create cache dir");
        let path = swifty_repo::repo_cache_blob_path(cache_root, repo_url);
        let bytes = serde_json::to_vec(blob).expect("serialize cache blob");
        std::fs::write(path, bytes).expect("write cache blob");
    }
}
