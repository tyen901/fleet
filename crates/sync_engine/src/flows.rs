use crate::apply::{apply_ops, quarantine_unexpected, ApplyOptions, IndexUpdate, QuarantineStats};
use crate::events::{EventSink, SyncEvent};
use crate::fetch::{fetch_all, FileEntry, ModManifest};
use crate::plan::{plan_mod, CacheHint, PlanError, PlannedOp, RepairStrategy};
use crate::safe_fs::ensure_no_symlink_ancestors;
use crate::safe_path::{safe_join_mod_file, validate_mod_id, validate_rel_path};
use crate::types::{
    RepairReport, RepairRequest, VerifyIssue, VerifyIssueKind, VerifyReport, VerifyRequest,
};
use crate::verify_parts::first_part_mismatch;
use anyhow::Result;
use fleet_index::{ExpectedFile, FleetIndex, SkipRepairPolicy};
use futures::StreamExt;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

pub async fn verify(
    req: VerifyRequest,
    idx: &mut FleetIndex,
    sink: Arc<dyn EventSink>,
) -> Result<VerifyReport> {
    let start = Instant::now();
    sink.push(SyncEvent::VerifyStarted {
        repo: req.repo_name.clone(),
    });

    let result = async {
        tokio::fs::create_dir_all(req.checkout_root.join(".fleet")).await?;

        let desired = idx
            .get_desired_state()?
            .ok_or_else(|| anyhow::anyhow!("desired_state missing"))?;
        validate_enabled_mods(&desired.enabled_mods_hash, &req.enabled_mods)?;

        let fetch = fetch_all(
            req.remote.clone(),
            &req.enabled_mods,
            req.tuning.scan_concurrency,
        )
        .await?;
        sink.push(SyncEvent::RemoteCapabilities {
            supports_ranges: fetch.capabilities.supports_ranges,
        });

        let baseline = build_baseline(&fetch.manifests);
        idx.expected_replace_all(&desired.state_id, baseline)?;

        let mut report = VerifyReport::default();

        for manifest in &fetch.manifests {
            sink.push(SyncEvent::ModStarted {
                mod_id: manifest.mod_id.clone(),
            });

            let cache = if req.tuning.use_index {
                build_cache_snapshot(idx, &desired.state_id, manifest)?
            } else {
                HashMap::new()
            };

            verify_mod(
                &req,
                idx,
                &desired.state_id,
                manifest,
                &cache,
                &mut report,
                sink.clone(),
            )
            .await?;

            sink.push(SyncEvent::ModFinished {
                mod_id: manifest.mod_id.clone(),
            });
        }

        report.ok = report.missing == 0
            && report.wrong_size == 0
            && report.not_a_file == 0
            && report.checksum_mismatch == 0
            && report.unsafe_path == 0;

        if report.ok {
            idx.verified_set(&desired.state_id, now_ns())?;
        } else {
            idx.verified_clear()?;
        }

        report.elapsed_ms = start.elapsed().as_millis() as u64;
        sink.push(SyncEvent::VerifyFinished { ok: report.ok });
        Ok(report)
    }
    .await;

    if result.is_err() {
        let _ = idx.verified_clear();
        sink.push(SyncEvent::VerifyFinished { ok: false });
    }

    result
}

pub async fn repair(
    req: RepairRequest,
    idx: &mut FleetIndex,
    sink: Arc<dyn EventSink>,
) -> Result<RepairReport> {
    let start = Instant::now();
    sink.push(SyncEvent::RepairStarted {
        repo: req.repo_name.clone(),
    });

    let result = async {
        tokio::fs::create_dir_all(req.checkout_root.join(".fleet")).await?;

        let desired = idx
            .get_desired_state()?
            .ok_or_else(|| anyhow::anyhow!("desired_state missing"))?;
        validate_enabled_mods(&desired.enabled_mods_hash, &req.enabled_mods)?;

        let skip = idx.evaluate_skip_repair(&req.checkout_root, SkipRepairPolicy::default())?;
        match &skip {
            fleet_index::SkipRepairDecision::Skippable(_) => {
                sink.push(SyncEvent::RepairSkipEvaluated {
                    skippable: true,
                    reason: None,
                });
                let report = RepairReport {
                    skipped: true,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    ..Default::default()
                };
                sink.push(SyncEvent::RepairFinished {
                    ok: true,
                    skipped: true,
                });
                return Ok(report);
            }
            fleet_index::SkipRepairDecision::NotSkippable { reason, .. } => {
                sink.push(SyncEvent::RepairSkipEvaluated {
                    skippable: false,
                    reason: Some(format!("{reason:?}")),
                });
            }
        }

        let fetch = fetch_all(
            req.remote.clone(),
            &req.enabled_mods,
            req.tuning.scan_concurrency,
        )
        .await?;
        sink.push(SyncEvent::RemoteCapabilities {
            supports_ranges: fetch.capabilities.supports_ranges,
        });

        let baseline = build_baseline(&fetch.manifests);
        idx.expected_replace_all(&desired.state_id, baseline)?;

        let mut report = RepairReport::default();

        for manifest in &fetch.manifests {
            sink.push(SyncEvent::ModStarted {
                mod_id: manifest.mod_id.clone(),
            });

            let cache = if req.tuning.use_index {
                build_cache_snapshot(idx, &desired.state_id, manifest)?
            } else {
                HashMap::new()
            };

            let plan_res = tokio::task::spawn_blocking({
                let checkout_root = req.checkout_root.clone();
                let manifest = manifest.clone();
                let cache = cache.clone();
                let tuning = req.tuning.clone();
                let checksummer = req.checksummer.clone();
                let supports_ranges = fetch.capabilities.supports_ranges;
                move || {
                    plan_mod(
                        &checkout_root,
                        &manifest,
                        &cache,
                        supports_ranges,
                        &tuning,
                        checksummer.as_ref(),
                    )
                }
            })
            .await?;

            let (plan, cache_hints) = match plan_res {
                Ok(v) => v,
                Err(err) => {
                    if let PlanError::UnsafeOnDisk {
                        mod_id,
                        rel_path,
                        source,
                    } = &err
                    {
                        sink.push(SyncEvent::Error {
                            message: source.to_string(),
                        });
                        idx.file_state_delete(&desired.state_id, mod_id, rel_path)?;
                    }
                    return Err(err.into());
                }
            };

            let (to_apply, skipped) = split_ops(plan.ops);

            for op in &to_apply {
                let strategy = match op.target.strategy {
                    RepairStrategy::Full => "full",
                    RepairStrategy::Patch => "patch",
                    RepairStrategy::Skip => "skip",
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

            let apply_outcome = apply_ops(
                to_apply,
                &req,
                sink.clone(),
                ApplyOptions {
                    supports_ranges: fetch.capabilities.supports_ranges,
                },
            )
            .await?;
            merge_repair_report(&mut report, &apply_outcome.report);

            apply_index_updates(idx, &desired.state_id, apply_outcome.index_updates)?;
            apply_cache_hints(idx, &desired.state_id, cache_hints)?;

            if let Some(failure) = apply_outcome.failures.into_iter().next() {
                sink.push(SyncEvent::Error {
                    message: failure.error.to_string(),
                });
                return Err(failure.error);
            }

            sink.push(SyncEvent::ModFinished {
                mod_id: manifest.mod_id.clone(),
            });
        }

        if req.tuning.quarantine {
            let mut expected_by_mod: HashMap<String, HashSet<String>> = HashMap::new();
            for manifest in &fetch.manifests {
                let mut set = HashSet::new();
                for file in &manifest.files {
                    set.insert(file.rel_path.replace('\\', "/"));
                }
                expected_by_mod.insert(manifest.mod_id.clone(), set);
            }

            for (mod_id, expected) in expected_by_mod {
                let stats = quarantine_unexpected(
                    &req.checkout_root,
                    &mod_id,
                    &expected,
                    &req.tuning,
                    sink.clone(),
                )
                .await?;
                merge_quarantine(&mut report, stats);
            }
        }

        report.elapsed_ms = start.elapsed().as_millis() as u64;
        idx.verified_set(&desired.state_id, now_ns())?;
        sink.push(SyncEvent::RepairFinished {
            ok: true,
            skipped: false,
        });
        Ok(report)
    }
    .await;

    if result.is_err() {
        let _ = idx.verified_clear();
        sink.push(SyncEvent::RepairFinished {
            ok: false,
            skipped: false,
        });
    }

    result
}

async fn verify_mod(
    req: &VerifyRequest,
    idx: &mut FleetIndex,
    state_id: &str,
    manifest: &ModManifest,
    cache: &HashMap<String, fleet_index::FileState>,
    report: &mut VerifyReport,
    sink: Arc<dyn EventSink>,
) -> Result<()> {
    let sem = Arc::new(tokio::sync::Semaphore::new(
        req.tuning.scan_concurrency.max(1),
    ));
    let mut tasks = futures::stream::FuturesUnordered::new();

    for file in &manifest.files {
        report.expected_files += 1;
        let mod_id = manifest.mod_id.clone();
        let rel_path = file.rel_path.replace('\\', "/");
        let permit = sem.clone().acquire_owned().await?;
        let checksummer = req.checksummer.clone();
        let file = file.clone();
        let cached = cache.get(&rel_path).cloned();
        let checkout_root = req.checkout_root.clone();

        tasks.push(tokio::task::spawn_blocking(move || {
            let _permit = permit;
            verify_one_file(
                &checkout_root,
                &mod_id,
                &rel_path,
                &file,
                cached,
                checksummer.as_ref(),
            )
        }));
    }

    while let Some(res) = tasks.next().await {
        let outcome = res??;
        apply_verify_outcome(req, idx, state_id, report, outcome, sink.clone())?;
    }

    Ok(())
}

struct VerifyOutcome {
    mod_id: String,
    rel_path: String,
    ok: bool,
    size: u64,
    mtime_ns: i64,
    checksum: Vec<u8>,
    issue: Option<VerifyIssueKind>,
    unsafe_message: Option<String>,
}

fn verify_one_file(
    checkout_root: &std::path::Path,
    mod_id: &str,
    rel_path: &str,
    file: &FileEntry,
    cached: Option<fleet_index::FileState>,
    checksummer: &dyn crate::types::Checksummer,
) -> Result<VerifyOutcome> {
    if validate_mod_id(mod_id).is_err() || validate_rel_path(rel_path).is_err() {
        return Ok(VerifyOutcome {
            mod_id: mod_id.to_string(),
            rel_path: rel_path.to_string(),
            ok: false,
            size: file.size,
            mtime_ns: 0,
            checksum: file.file_checksum.clone(),
            issue: Some(VerifyIssueKind::UnsafePath),
            unsafe_message: None,
        });
    }

    let abs_path = match safe_join_mod_file(checkout_root, mod_id, rel_path) {
        Ok(p) => p,
        Err(_) => {
            return Ok(VerifyOutcome {
                mod_id: mod_id.to_string(),
                rel_path: rel_path.to_string(),
                ok: false,
                size: file.size,
                mtime_ns: 0,
                checksum: file.file_checksum.clone(),
                issue: Some(VerifyIssueKind::UnsafePath),
                unsafe_message: None,
            })
        }
    };

    let mod_root = checkout_root.join(mod_id);
    if let Some(parent) = abs_path.parent() {
        if let Err(err) = ensure_no_symlink_ancestors(&mod_root, parent) {
            return Ok(VerifyOutcome {
                mod_id: mod_id.to_string(),
                rel_path: rel_path.to_string(),
                ok: false,
                size: file.size,
                mtime_ns: 0,
                checksum: file.file_checksum.clone(),
                issue: Some(VerifyIssueKind::UnsafeOnDisk),
                unsafe_message: Some(err.to_string()),
            });
        }
    }

    let metadata = match std::fs::symlink_metadata(&abs_path) {
        Ok(md) => {
            let ft = md.file_type();
            if ft.is_symlink() || !ft.is_file() {
                return Ok(VerifyOutcome {
                    mod_id: mod_id.to_string(),
                    rel_path: rel_path.to_string(),
                    ok: false,
                    size: file.size,
                    mtime_ns: 0,
                    checksum: file.file_checksum.clone(),
                    issue: Some(VerifyIssueKind::NotAFile),
                    unsafe_message: None,
                });
            }
            md
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(VerifyOutcome {
                mod_id: mod_id.to_string(),
                rel_path: rel_path.to_string(),
                ok: false,
                size: file.size,
                mtime_ns: 0,
                checksum: file.file_checksum.clone(),
                issue: Some(VerifyIssueKind::Missing),
                unsafe_message: None,
            });
        }
        Err(e) => return Err(e.into()),
    };

    if metadata.len() != file.size {
        return Ok(VerifyOutcome {
            mod_id: mod_id.to_string(),
            rel_path: rel_path.to_string(),
            ok: false,
            size: file.size,
            mtime_ns: 0,
            checksum: file.file_checksum.clone(),
            issue: Some(VerifyIssueKind::WrongSize {
                expected: file.size,
                got: metadata.len(),
            }),
            unsafe_message: None,
        });
    }

    let mtime_ns = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .and_then(|n| i64::try_from(n).ok())
        .unwrap_or(0);

    if let Some(cached) = cached {
        if cached.size == file.size
            && cached.mtime_ns == mtime_ns
            && cached.checksum == file.file_checksum
        {
            return Ok(VerifyOutcome {
                mod_id: mod_id.to_string(),
                rel_path: rel_path.to_string(),
                ok: true,
                size: file.size,
                mtime_ns,
                checksum: file.file_checksum.clone(),
                issue: None,
                unsafe_message: None,
            });
        }
    }

    if let Some((offset, len)) = first_part_mismatch(&abs_path, &file.parts, checksummer)? {
        return Ok(VerifyOutcome {
            mod_id: mod_id.to_string(),
            rel_path: rel_path.to_string(),
            ok: false,
            size: file.size,
            mtime_ns,
            checksum: file.file_checksum.clone(),
            issue: Some(VerifyIssueKind::PartMismatch { offset, len }),
            unsafe_message: None,
        });
    }

    Ok(VerifyOutcome {
        mod_id: mod_id.to_string(),
        rel_path: rel_path.to_string(),
        ok: true,
        size: file.size,
        mtime_ns,
        checksum: file.file_checksum.clone(),
        issue: None,
        unsafe_message: None,
    })
}

fn apply_verify_outcome(
    req: &VerifyRequest,
    idx: &mut FleetIndex,
    state_id: &str,
    report: &mut VerifyReport,
    outcome: VerifyOutcome,
    sink: Arc<dyn EventSink>,
) -> Result<()> {
    if outcome.ok {
        report.verified_ok += 1;
        idx.file_state_upsert(
            state_id,
            &outcome.mod_id,
            &outcome.rel_path,
            outcome.size,
            outcome.mtime_ns,
            &outcome.checksum,
        )?;
        sink.push(SyncEvent::FileVerified {
            mod_id: outcome.mod_id,
            path: outcome.rel_path,
        });
        return Ok(());
    }

    if let Some(kind) = &outcome.issue {
        match kind {
            VerifyIssueKind::Missing => report.missing += 1,
            VerifyIssueKind::WrongSize { .. } => report.wrong_size += 1,
            VerifyIssueKind::NotAFile => report.not_a_file += 1,
            VerifyIssueKind::UnsafePath => report.unsafe_path += 1,
            VerifyIssueKind::UnsafeOnDisk => report.unsafe_path += 1,
            VerifyIssueKind::PartMismatch { .. } => report.checksum_mismatch += 1,
        }

        if report.issues.len() < req.tuning.max_issues {
            report.issues.push(VerifyIssue {
                mod_id: outcome.mod_id.clone(),
                rel_path: outcome.rel_path.clone(),
                kind: kind.clone(),
            });
        }
    }

    if matches!(outcome.issue, Some(VerifyIssueKind::UnsafeOnDisk)) {
        sink.push(SyncEvent::Error {
            message: outcome
                .unsafe_message
                .unwrap_or_else(|| "unsafe path (symlink ancestor)".to_string()),
        });
    }

    idx.file_state_delete(state_id, &outcome.mod_id, &outcome.rel_path)?;
    Ok(())
}

fn build_baseline(manifests: &[ModManifest]) -> Vec<ExpectedFile> {
    let mut rows = Vec::new();
    for manifest in manifests {
        for file in &manifest.files {
            rows.push(ExpectedFile {
                mod_id: manifest.mod_id.clone(),
                rel_path: file.rel_path.replace('\\', "/"),
                size: file.size,
            });
        }
    }
    rows
}

fn build_cache_snapshot(
    idx: &FleetIndex,
    state_id: &str,
    manifest: &ModManifest,
) -> Result<HashMap<String, fleet_index::FileState>> {
    let mut map = HashMap::new();
    for file in &manifest.files {
        let rel = file.rel_path.replace('\\', "/");
        if let Some(state) = idx.file_state_get(state_id, &manifest.mod_id, &rel)? {
            map.insert(rel, state);
        }
    }
    Ok(map)
}

fn validate_enabled_mods(expected_hash: &str, enabled_mods: &[String]) -> Result<()> {
    for mod_id in enabled_mods {
        validate_mod_id(mod_id)?;
    }
    let mut mods_sorted = enabled_mods.to_vec();
    mods_sorted.sort();
    let got = fleet_index::enabled_mods_hash(&mods_sorted);
    if got != expected_hash {
        anyhow::bail!("enabled mods hash mismatch");
    }
    Ok(())
}

fn apply_index_updates(
    idx: &mut FleetIndex,
    state_id: &str,
    updates: Vec<IndexUpdate>,
) -> Result<()> {
    for update in updates {
        match update {
            IndexUpdate::UpsertFileState {
                mod_id,
                rel_path,
                size,
                mtime_ns,
                checksum,
            } => {
                idx.file_state_upsert(state_id, &mod_id, &rel_path, size, mtime_ns, &checksum)?;
            }
            IndexUpdate::DeleteFileState { mod_id, rel_path } => {
                idx.file_state_delete(state_id, &mod_id, &rel_path)?;
            }
        }
    }
    Ok(())
}

fn apply_cache_hints(idx: &mut FleetIndex, state_id: &str, hints: Vec<CacheHint>) -> Result<()> {
    for hint in hints {
        idx.file_state_upsert(
            state_id,
            &hint.mod_id,
            &hint.rel_path,
            hint.size,
            hint.mtime_ns,
            &hint.checksum,
        )?;
    }
    Ok(())
}

fn split_ops(ops: Vec<PlannedOp>) -> (Vec<PlannedOp>, Vec<PlannedOp>) {
    let mut to_apply = Vec::new();
    let mut skipped = Vec::new();
    for op in ops {
        if matches!(op.target.strategy, RepairStrategy::Skip) {
            skipped.push(op);
        } else {
            to_apply.push(op);
        }
    }
    (to_apply, skipped)
}

fn merge_repair_report(dst: &mut RepairReport, src: &RepairReport) {
    dst.files_downloaded = dst.files_downloaded.saturating_add(src.files_downloaded);
    dst.files_patched = dst.files_patched.saturating_add(src.files_patched);
    dst.bytes_downloaded = dst.bytes_downloaded.saturating_add(src.bytes_downloaded);
    dst.bytes_patched = dst.bytes_patched.saturating_add(src.bytes_patched);
    dst.quarantine_files = dst.quarantine_files.saturating_add(src.quarantine_files);
    dst.quarantine_dirs = dst.quarantine_dirs.saturating_add(src.quarantine_dirs);
    dst.quarantine_bytes = dst.quarantine_bytes.saturating_add(src.quarantine_bytes);
    dst.empty_dirs_deleted = dst
        .empty_dirs_deleted
        .saturating_add(src.empty_dirs_deleted);
}

fn merge_quarantine(dst: &mut RepairReport, stats: QuarantineStats) {
    dst.quarantine_files = dst.quarantine_files.saturating_add(stats.files);
    dst.quarantine_dirs = dst.quarantine_dirs.saturating_add(stats.dirs);
    dst.quarantine_bytes = dst.quarantine_bytes.saturating_add(stats.bytes);
    dst.empty_dirs_deleted = dst
        .empty_dirs_deleted
        .saturating_add(stats.empty_dirs_deleted);
}

fn now_ns() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i64,
        Err(_) => 0,
    }
}
