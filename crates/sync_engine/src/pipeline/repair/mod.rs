use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use crate::manifest::ValidatedModManifest;
use crate::model::{
    AbortReason, FileFailure, FileStateDelete, FileStateUpsert, RepairOutcome, RepairReport,
    RepairRequest, TimestampNs,
};
use crate::pipeline::fetch_all;
use crate::ports::SyncEvent;
use crate::ports::{Checksummer, EventSink, RemoteRepo, StateStore};
use crate::skip_check;
use crate::unexpected::{handle_unexpected_paths, UnexpectedStats};
use crate::util::now_ns;
use tokio_util::sync::CancellationToken;

pub(crate) mod applier;
pub(crate) mod planner;

pub(crate) async fn run(
    req: RepairRequest,
    remote: Arc<dyn RemoteRepo>,
    store: Arc<dyn StateStore>,
    checksummer: Arc<dyn Checksummer>,
    sink: &dyn EventSink,
    cancel: &CancellationToken,
) -> Result<RepairOutcome, crate::model::EngineError> {
    let start = Instant::now();
    sink.push(SyncEvent::RepairStarted {
        repo: req.repo_name.clone(),
    });

    let result: Result<RepairOutcome, crate::model::EngineError> = async {
        tokio::fs::create_dir_all(req.checkout_root.join(".fleet"))
            .await
            .map_err(|e| crate::model::EngineError::Internal(e.into()))?;

        let desired = store
            .desired_state_get()
            .map_err(crate::model::EngineError::Store)?
            .ok_or_else(|| {
                crate::model::EngineError::InvalidInput("desired_state missing".to_string())
            })?;
        super::validate_enabled_mods(&desired.enabled_mods_hash, &req.enabled_mods)
            .map_err(|e| crate::model::EngineError::InvalidInput(e.to_string()))?;

        let fetch = fetch_all(
            remote.clone(),
            &req.enabled_mods,
            req.tuning.scan_concurrency,
            cancel,
        )
        .await?;
        sink.push(SyncEvent::RemoteCapabilities {
            supports_ranges: fetch.capabilities.supports_ranges,
        });

        let baseline = build_baseline(&fetch.manifests);
        let baseline_digest = super::baseline_digest_hex(&baseline);
        store
            .expected_replace_all_if_digest_changed(&desired.state_id, baseline, &baseline_digest)
            .map_err(crate::model::EngineError::Store)?;

        type ExpectedTriplet = (String, u64, Option<Vec<u8>>);

        if req.tuning.auto_fix_case {
            for manifest in &fetch.manifests {
                let checkout_root = req.checkout_root.clone();
                let mod_id = manifest.mod_id.clone();
                let expected: Vec<ExpectedTriplet> = manifest
                    .files
                    .iter()
                    .map(|f| (f.rel_path.clone(), f.size, Some(f.file_checksum.clone())))
                    .collect();
                let checksummer = checksummer.clone();
                let tuning = fleet_fs_case::CaseFixTuning::default();
                let _ = tokio::task::spawn_blocking(move || {
                    let hash_file = move |p: &std::path::Path| {
                        checksummer
                            .hash_file(p)
                            .map_err(|e| std::io::Error::other(e.to_string()))
                    };
                    fleet_fs_case::case_sweep_and_fix(
                        &checkout_root,
                        &mod_id,
                        &expected,
                        &tuning,
                        Some(&hash_file),
                    )
                })
                .await
                .map_err(|e| crate::model::EngineError::Internal(anyhow::anyhow!(e.to_string())))?
                .map_err(|e| crate::model::EngineError::Internal(anyhow::anyhow!(e.to_string())))?;
            }
        }

        let skip = tokio::task::spawn_blocking({
            let checkout_root = req.checkout_root.clone();
            let manifests = fetch.manifests.clone();
            let policy = skip_check::SkipCheckPolicy::default();
            let store = store.clone();
            move || skip_check::evaluate_skip(store.as_ref(), &checkout_root, &manifests, policy)
        })
        .await
        .map_err(|e| crate::model::EngineError::Internal(anyhow::anyhow!(e.to_string())))?
        .map_err(crate::model::EngineError::Internal)?;

        match &skip {
            skip_check::SkipCheckDecision::Skippable(_) => {
                sink.push(SyncEvent::RepairSkipEvaluated {
                    skippable: true,
                    reason: None,
                });
                let report = RepairReport {
                    skipped: true,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    ..Default::default()
                };
                let outcome = RepairOutcome {
                    report,
                    failures: Vec::new(),
                    aborted: None,
                };
                sink.push(SyncEvent::RepairFinished {
                    ok: true,
                    skipped: true,
                });
                return Ok(outcome);
            }
            skip_check::SkipCheckDecision::NotSkippable { reason, .. } => {
                sink.push(SyncEvent::RepairSkipEvaluated {
                    skippable: false,
                    reason: Some(format!("{reason:?}")),
                });
            }
        }

        let mut report = RepairReport::default();
        let mut failures: Vec<FileFailure> = Vec::new();
        let mut aborted: Option<AbortReason> = None;

        'mods: for manifest in &fetch.manifests {
            if cancel.is_cancelled() {
                return Err(crate::model::EngineError::Cancelled);
            }
            sink.push(SyncEvent::ModStarted {
                mod_id: manifest.mod_id.clone(),
            });

            let cache = if req.tuning.use_index {
                super::build_cache_snapshot(store.as_ref(), &desired.state_id, manifest)
                    .map_err(crate::model::EngineError::Store)?
            } else {
                HashMap::new()
            };

            let plan_res = planner::plan_mod_spawn_blocking(
                &req.checkout_root,
                manifest.clone(),
                cache,
                fetch.capabilities.supports_ranges,
                req.tuning.clone(),
                checksummer.clone(),
                cancel,
            )
            .await?;

            let (plan, cache_hints) = match plan_res {
                Ok(v) => v,
                Err(planner::PlannerError::UnsafeOnDisk {
                    mod_id,
                    rel_path,
                    message,
                }) => {
                    sink.push(SyncEvent::Error {
                        message: message.clone(),
                    });
                    store
                        .file_state_delete(&desired.state_id, &mod_id, &rel_path)
                        .map_err(crate::model::EngineError::Store)?;
                    failures.push(FileFailure {
                        mod_id: mod_id.clone(),
                        rel_path: rel_path.clone(),
                        message,
                        aborting: true,
                    });
                    aborted = Some(AbortReason::UnsafeOnDisk {
                        message: failures
                            .last()
                            .map(|f| f.message.clone())
                            .unwrap_or_else(|| "unsafe on disk".to_string()),
                    });
                    cancel.cancel();
                    break 'mods;
                }
                Err(planner::PlannerError::Other(e)) => {
                    return Err(crate::model::EngineError::Internal(e))
                }
            };

            if aborted.is_some() {
                break 'mods;
            }

            let (to_apply, skipped) = split_ops(plan.ops);

            for op in &to_apply {
                let strategy = match op.target.strategy {
                    planner::RepairStrategy::Full => "full",
                    planner::RepairStrategy::Patch => "patch",
                    planner::RepairStrategy::Skip => "skip",
                };
                sink.push(SyncEvent::FileNeedsRepair {
                    mod_id: op.mod_id.clone(),
                    path: op.rel_path.clone(),
                    strategy: strategy.to_string(),
                });
            }

            for op in &skipped {
                sink.push(SyncEvent::FileUpToDate {
                    mod_id: op.mod_id.clone(),
                    path: op.rel_path.clone(),
                });
            }

            let apply_outcome = applier::apply_ops(
                to_apply,
                &req.checkout_root,
                remote.clone(),
                checksummer.clone(),
                &req.tuning,
                sink,
                cancel,
                applier::ApplyOptions {
                    supports_ranges: fetch.capabilities.supports_ranges,
                },
            )
            .await?;

            report += &apply_outcome.report;

            let mut upserts: Vec<FileStateUpsert> = Vec::new();
            let mut deletes: Vec<FileStateDelete> = Vec::new();
            for update in apply_outcome.index_updates {
                match update {
                    applier::IndexUpdate::UpsertFileState {
                        mod_id,
                        rel_path,
                        size,
                        mtime_ns,
                        checksum,
                    } => upserts.push(FileStateUpsert {
                        mod_id,
                        rel_path,
                        size,
                        mtime_ns,
                        checksum,
                    }),
                    applier::IndexUpdate::DeleteFileState { mod_id, rel_path } => {
                        deletes.push(FileStateDelete { mod_id, rel_path })
                    }
                }
            }
            for hint in cache_hints {
                upserts.push(FileStateUpsert {
                    mod_id: hint.mod_id,
                    rel_path: hint.rel_path,
                    size: hint.size,
                    mtime_ns: hint.mtime_ns,
                    checksum: hint.checksum,
                });
            }
            store
                .file_state_apply_batch(&desired.state_id, upserts, deletes)
                .map_err(crate::model::EngineError::Store)?;

            failures.extend(apply_outcome.failures);

            if let Some(reason) = apply_outcome.aborted {
                aborted = Some(reason);
                break;
            }

            sink.push(SyncEvent::ModFinished {
                mod_id: manifest.mod_id.clone(),
            });
        }

        if aborted.is_none() {
            let mut expected_by_mod: HashMap<String, HashSet<String>> = HashMap::new();
            for manifest in &fetch.manifests {
                let mut set = HashSet::new();
                for file in &manifest.files {
                    set.insert(file.rel_path.clone());
                }
                expected_by_mod.insert(manifest.mod_id.clone(), set);
            }

            for (mod_id, expected) in expected_by_mod {
                let stats = match handle_unexpected_paths(
                    &req.checkout_root,
                    &mod_id,
                    &expected,
                    &req.tuning,
                    sink,
                    cancel,
                )
                .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        if cancel.is_cancelled() || e.to_string().contains("cancelled") {
                            return Err(crate::model::EngineError::Cancelled);
                        }
                        return Err(crate::model::EngineError::Internal(e));
                    }
                };
                merge_unexpected(&mut report, stats.clone());
                if matches!(
                    req.tuning.unexpected_paths,
                    crate::model::UnexpectedPathPolicy::Prompt
                ) && (stats.found_files + stats.found_dirs) > 0
                {
                    aborted = Some(AbortReason::UnexpectedPaths {
                        message: "unexpected files/directories found".to_string(),
                        mod_id,
                        files: stats.found_files,
                        dirs: stats.found_dirs,
                        bytes: stats.found_bytes,
                    });
                    break;
                }
            }
        }

        report.elapsed_ms = start.elapsed().as_millis() as u64;

        let outcome = RepairOutcome {
            report,
            failures,
            aborted,
        };

        if outcome.ok() {
            let _ = store.verified_set(&desired.state_id, TimestampNs(now_ns()));
        } else if !outcome.report.skipped {
            let _ = store.verified_clear();
        }

        sink.push(SyncEvent::RepairFinished {
            ok: outcome.ok(),
            skipped: outcome.report.skipped,
        });
        Ok(outcome)
    }
    .await;

    if result.is_err() {
        let _ = store.verified_clear();
        sink.push(SyncEvent::RepairFinished {
            ok: false,
            skipped: false,
        });
    }

    result
}

fn build_baseline(manifests: &[ValidatedModManifest]) -> Vec<crate::model::ExpectedFile> {
    let mut rows = Vec::new();
    for manifest in manifests {
        for file in &manifest.files {
            rows.push(crate::model::ExpectedFile {
                mod_id: manifest.mod_id.clone(),
                rel_path: file.rel_path.clone(),
                size: file.size,
            });
        }
    }
    rows
}

fn split_ops(ops: Vec<planner::PlannedOp>) -> (Vec<planner::PlannedOp>, Vec<planner::PlannedOp>) {
    let mut to_apply = Vec::new();
    let mut skipped = Vec::new();
    for op in ops {
        if matches!(op.target.strategy, planner::RepairStrategy::Skip) {
            skipped.push(op);
        } else {
            to_apply.push(op);
        }
    }
    (to_apply, skipped)
}

fn merge_unexpected(dst: &mut RepairReport, stats: UnexpectedStats) {
    dst.unexpected_found_files = dst.unexpected_found_files.saturating_add(stats.found_files);
    dst.unexpected_found_dirs = dst.unexpected_found_dirs.saturating_add(stats.found_dirs);
    dst.unexpected_found_bytes = dst.unexpected_found_bytes.saturating_add(stats.found_bytes);
    dst.unexpected_deleted_files = dst
        .unexpected_deleted_files
        .saturating_add(stats.deleted_files);
    dst.unexpected_deleted_dirs = dst
        .unexpected_deleted_dirs
        .saturating_add(stats.deleted_dirs);
    dst.unexpected_deleted_bytes = dst
        .unexpected_deleted_bytes
        .saturating_add(stats.deleted_bytes);
    dst.empty_dirs_deleted = dst
        .empty_dirs_deleted
        .saturating_add(stats.empty_dirs_deleted);
}

#[cfg(test)]
mod tests {
    use super::{applier, planner};
    use crate::model::{Durability, RepairRequest, RepairTuning};
    use crate::ports::{
        Checksummer, EventSink, ModManifest, RemoteCapabilities, RemoteRepo, RemoteStream,
        RemoteStreamImpl,
    };
    use bytes::Bytes;
    use std::collections::HashMap;
    use std::fs;
    use std::io::{Read, Seek, SeekFrom};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    fn fnv1a64(data: &[u8]) -> [u8; 8] {
        let mut hash: u64 = 0xcbf29ce484222325;
        for &b in data {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash.to_le_bytes()
    }

    #[derive(Clone)]
    struct TestChecksummer;

    impl Checksummer for TestChecksummer {
        fn algorithm_name(&self) -> &str {
            "fnv1a64"
        }

        fn hash_file(&self, path: &Path) -> anyhow::Result<Vec<u8>> {
            let mut f = fs::File::open(path)?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            Ok(fnv1a64(&buf).to_vec())
        }

        fn hash_range(&self, path: &Path, offset: u64, len: u64) -> anyhow::Result<Vec<u8>> {
            let mut f = fs::File::open(path)?;
            f.seek(SeekFrom::Start(offset))?;
            let mut buf = vec![0u8; len as usize];
            f.read_exact(&mut buf)?;
            Ok(fnv1a64(&buf).to_vec())
        }
    }

    #[derive(Default)]
    struct TestSink {
        events: Mutex<Vec<crate::ports::SyncEvent>>,
    }

    impl TestSink {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }
    }

    impl EventSink for TestSink {
        fn push(&self, ev: crate::ports::SyncEvent) {
            self.events.lock().unwrap().push(ev);
        }
    }

    struct MockRemoteRepo {
        caps: RemoteCapabilities,
        manifests: Mutex<HashMap<String, ModManifest>>,
        files: Mutex<HashMap<(String, String), Bytes>>,
        chunk_size: usize,
        range_calls: Mutex<Vec<(String, String, u64, u64)>>,
    }

    impl MockRemoteRepo {
        fn new(chunk_size: usize) -> Self {
            Self {
                caps: RemoteCapabilities {
                    supports_ranges: true,
                },
                manifests: Mutex::new(HashMap::new()),
                files: Mutex::new(HashMap::new()),
                chunk_size,
                range_calls: Mutex::new(Vec::new()),
            }
        }

        fn range_calls(&self) -> Vec<(String, String, u64, u64)> {
            self.range_calls.lock().unwrap().clone()
        }

        fn with_file(self, mod_id: &str, rel_path: &str, bytes: Bytes) -> Self {
            self.files
                .lock()
                .unwrap()
                .insert((mod_id.to_string(), rel_path.to_string()), bytes);
            self
        }
    }

    struct BytesStream {
        bytes: Bytes,
        pos: usize,
        chunk_size: usize,
    }

    #[async_trait::async_trait]
    impl RemoteStreamImpl for BytesStream {
        async fn next_chunk(&mut self) -> anyhow::Result<Option<Bytes>> {
            if self.pos >= self.bytes.len() {
                return Ok(None);
            }
            let end = (self.pos + self.chunk_size).min(self.bytes.len());
            let out = self.bytes.slice(self.pos..end);
            self.pos = end;
            Ok(Some(out))
        }
    }

    #[async_trait::async_trait]
    impl RemoteRepo for MockRemoteRepo {
        async fn capabilities(&self) -> anyhow::Result<RemoteCapabilities> {
            Ok(self.caps.clone())
        }

        async fn fetch_mod_manifest(&self, mod_id: &str) -> anyhow::Result<ModManifest> {
            Ok(self.manifests.lock().unwrap().get(mod_id).cloned().unwrap())
        }

        async fn fetch_file(&self, mod_id: &str, rel_path: &str) -> anyhow::Result<RemoteStream> {
            let b = self
                .files
                .lock()
                .unwrap()
                .get(&(mod_id.to_string(), rel_path.to_string()))
                .cloned()
                .unwrap();
            Ok(RemoteStream::new(Box::new(BytesStream {
                bytes: b,
                pos: 0,
                chunk_size: self.chunk_size,
            })))
        }

        async fn fetch_range(
            &self,
            mod_id: &str,
            rel_path: &str,
            offset: u64,
            len: u64,
        ) -> anyhow::Result<RemoteStream> {
            self.range_calls.lock().unwrap().push((
                mod_id.to_string(),
                rel_path.to_string(),
                offset,
                len,
            ));
            let b = self
                .files
                .lock()
                .unwrap()
                .get(&(mod_id.to_string(), rel_path.to_string()))
                .cloned()
                .unwrap();
            let off = offset as usize;
            let end = (offset + len) as usize;
            let slice = b.slice(off..end.min(b.len()));
            Ok(RemoteStream::new(Box::new(BytesStream {
                bytes: slice,
                pos: 0,
                chunk_size: self.chunk_size,
            })))
        }
    }

    fn make_parts(bytes: &[u8], part_size: usize) -> Vec<crate::ports::FilePart> {
        let mut out = Vec::new();
        let mut offset: u64 = 0;
        while (offset as usize) < bytes.len() {
            let start = offset as usize;
            let end = (start + part_size).min(bytes.len());
            out.push(crate::ports::FilePart {
                offset,
                len: (end - start) as u64,
                checksum: fnv1a64(&bytes[start..end]).to_vec(),
            });
            offset += (end - start) as u64;
        }
        out
    }

    fn make_manifest(
        mod_id: &str,
        rel_path: &str,
        bytes: &[u8],
        part_size: usize,
    ) -> crate::manifest::ValidatedModManifest {
        crate::manifest::ValidatedModManifest {
            mod_id: mod_id.to_string(),
            files: vec![crate::manifest::ValidatedFileEntry {
                rel_path: rel_path.to_string(),
                size: bytes.len() as u64,
                file_checksum: fnv1a64(bytes).to_vec(),
                parts: make_parts(bytes, part_size),
            }],
        }
    }

    fn write_local_file(root: &Path, mod_id: &str, rel_path: &str, bytes: &[u8]) -> PathBuf {
        let mod_root = root.join(mod_id);
        fs::create_dir_all(&mod_root).unwrap();
        let abs = mod_root.join(rel_path);
        fs::write(&abs, bytes).unwrap();
        abs
    }

    #[tokio::test]
    async fn patch_coalesces_across_small_gap_into_single_range_request() {
        let file_size = 8 * 1024;
        let part_size = 512;

        let remote_bytes: Vec<u8> = (0..file_size).map(|i| (i % 251) as u8).collect();
        let manifest = make_manifest("m", "a.bin", &remote_bytes, part_size);

        let mut local_bytes = remote_bytes.clone();
        local_bytes[10] ^= 0xFF;
        local_bytes[1030] ^= 0xFF;

        let tmp = tempfile::tempdir().unwrap();
        write_local_file(tmp.path(), "m", "a.bin", &local_bytes);

        let tuning = RepairTuning {
            patch_merge_gap_bytes: 512,
            patch_min_range_bytes: 0,
            patch_max_bad_ratio: 1.0,
            patch_max_fetch_ratio: 1.0,
            patch_max_range_requests: Some(64),
            durability: Durability::BestEffort,
            ..Default::default()
        };

        let checksummer: Arc<dyn Checksummer> = Arc::new(TestChecksummer);
        let (plan, _hints) = planner::plan_mod(
            tmp.path(),
            &manifest,
            &HashMap::new(),
            true,
            &tuning,
            checksummer.as_ref(),
        )
        .unwrap();

        let op = plan.ops.into_iter().next().unwrap();
        assert!(matches!(op.target.strategy, planner::RepairStrategy::Patch));
        assert_eq!(op.target.parts_to_fetch.len(), 1);

        let remote = Arc::new(MockRemoteRepo::new(1024).with_file(
            "m",
            "a.bin",
            Bytes::from(remote_bytes.clone()),
        ));

        let req = RepairRequest {
            repo_name: "r".to_string(),
            checkout_root: tmp.path().to_path_buf(),
            enabled_mods: vec!["m".to_string()],
            tuning,
        };

        let sink: Arc<dyn EventSink> = Arc::new(TestSink::new());
        applier::apply_ops(
            vec![op],
            &req.checkout_root,
            remote.clone(),
            checksummer.clone(),
            &req.tuning,
            sink.as_ref(),
            &tokio_util::sync::CancellationToken::new(),
            applier::ApplyOptions {
                supports_ranges: true,
            },
        )
        .await
        .unwrap();

        let calls = remote.range_calls();
        assert_eq!(calls.len(), 1);
        let (_m, _p, off, len) = &calls[0];
        assert_eq!(*off, 0);
        assert_eq!(*len, 1536);

        let final_bytes = fs::read(tmp.path().join("m").join("a.bin")).unwrap();
        assert_eq!(final_bytes, remote_bytes);
    }

    #[tokio::test]
    async fn patch_enforces_min_range_size_by_expanding_request() {
        let file_size = 8 * 1024;
        let part_size = 512;

        let remote_bytes: Vec<u8> = (0..file_size).map(|i| (i % 239) as u8).collect();
        let manifest = make_manifest("m", "b.bin", &remote_bytes, part_size);

        let mut local_bytes = remote_bytes.clone();
        local_bytes[4096 + 3] ^= 0xAA;

        let tmp = tempfile::tempdir().unwrap();
        write_local_file(tmp.path(), "m", "b.bin", &local_bytes);

        let tuning = RepairTuning {
            patch_merge_gap_bytes: 0,
            patch_min_range_bytes: 2048,
            patch_max_bad_ratio: 1.0,
            patch_max_fetch_ratio: 1.0,
            patch_max_range_requests: Some(64),
            ..Default::default()
        };

        let checksummer: Arc<dyn Checksummer> = Arc::new(TestChecksummer);
        let (plan, _hints) = planner::plan_mod(
            tmp.path(),
            &manifest,
            &HashMap::new(),
            true,
            &tuning,
            checksummer.as_ref(),
        )
        .unwrap();
        let op = plan.ops.into_iter().next().unwrap();
        assert!(matches!(op.target.strategy, planner::RepairStrategy::Patch));
        assert_eq!(op.target.parts_to_fetch.len(), 1);

        assert_eq!(op.target.parts_to_fetch[0].offset, 3072);
        assert_eq!(op.target.parts_to_fetch[0].len, 2048);

        let remote = Arc::new(MockRemoteRepo::new(1024).with_file(
            "m",
            "b.bin",
            Bytes::from(remote_bytes.clone()),
        ));
        let req = RepairRequest {
            repo_name: "r".to_string(),
            checkout_root: tmp.path().to_path_buf(),
            enabled_mods: vec!["m".to_string()],
            tuning,
        };

        let sink: Arc<dyn EventSink> = Arc::new(TestSink::new());
        applier::apply_ops(
            vec![op],
            &req.checkout_root,
            remote.clone(),
            checksummer.clone(),
            &req.tuning,
            sink.as_ref(),
            &tokio_util::sync::CancellationToken::new(),
            applier::ApplyOptions {
                supports_ranges: true,
            },
        )
        .await
        .unwrap();

        let calls = remote.range_calls();
        assert_eq!(calls.len(), 1);
        let (_m, _p, off, len) = &calls[0];
        assert_eq!(*off, 3072);
        assert_eq!(*len, 2048);

        let final_bytes = fs::read(tmp.path().join("m").join("b.bin")).unwrap();
        assert_eq!(final_bytes, remote_bytes);
    }

    #[test]
    fn planner_falls_back_to_full_if_min_range_forces_near_full_download() {
        let file_size = 4096;
        let part_size = 512;

        let remote_bytes: Vec<u8> = (0..file_size).map(|i| (i % 199) as u8).collect();
        let manifest = make_manifest("m", "c.bin", &remote_bytes, part_size);

        let mut local_bytes = remote_bytes.clone();
        local_bytes[7] ^= 0x11;

        let tmp = tempfile::tempdir().unwrap();
        write_local_file(tmp.path(), "m", "c.bin", &local_bytes);

        let tuning = RepairTuning {
            patch_merge_gap_bytes: 0,
            patch_min_range_bytes: 4096,
            patch_max_bad_ratio: 1.0,
            patch_max_fetch_ratio: 0.75,
            patch_max_range_requests: Some(64),
            ..Default::default()
        };

        let checksummer: Arc<dyn Checksummer> = Arc::new(TestChecksummer);
        let (plan, _hints) = planner::plan_mod(
            tmp.path(),
            &manifest,
            &HashMap::new(),
            true,
            &tuning,
            checksummer.as_ref(),
        )
        .unwrap();
        let op = plan.ops.into_iter().next().unwrap();
        assert!(matches!(op.target.strategy, planner::RepairStrategy::Full));
    }
}
