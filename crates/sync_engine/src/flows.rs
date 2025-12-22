use crate::apply::{apply_ops, ApplyOptions, IndexUpdate};
use crate::events::{EventSink, SyncEvent};
use crate::fetch::fetch_all;
use crate::manifest::{ValidatedFileEntry, ValidatedModManifest};
use crate::plan::{plan_mod, PlanError, PlannedOp, RepairStrategy};
use crate::safe_fs::ensure_no_symlink_ancestors;
use crate::safe_path::{safe_join_mod_file, validate_mod_id};
use crate::skip_check;
use crate::time_util::now_ns;
use crate::types::{
    AbortReason, FileFailure, RepairOutcome, RepairReport, RepairRequest, VerifyIssue,
    VerifyIssueKind, VerifyReport, VerifyRequest,
};
use crate::unexpected::{handle_unexpected_paths, UnexpectedStats};
use crate::verify_parts::first_part_mismatch;
use anyhow::Result;
use fleet_index::{ExpectedFile, FleetIndex};
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
        let baseline_digest = baseline_digest_hex(&baseline);
        idx.expected_replace_all_if_digest_changed(&desired.state_id, baseline, &baseline_digest)?;

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
) -> Result<RepairOutcome> {
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
        let baseline_digest = baseline_digest_hex(&baseline);
        idx.expected_replace_all_if_digest_changed(&desired.state_id, baseline, &baseline_digest)?;

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
                let checksummer = req.checksummer.clone();
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
                .await??;
            }
        }

        let skip = skip_check::evaluate_skip(
            idx,
            &req.checkout_root,
            &fetch.manifests,
            skip_check::SkipCheckPolicy::default(),
        )?;
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
                Err(err) => match err {
                    PlanError::UnsafeOnDisk {
                        mod_id,
                        rel_path,
                        source,
                    } => {
                        sink.push(SyncEvent::Error {
                            message: source.to_string(),
                        });
                        idx.file_state_delete(&desired.state_id, &mod_id, &rel_path)?;

                        failures.push(FileFailure {
                            mod_id: mod_id.clone(),
                            rel_path: rel_path.clone(),
                            message: source.to_string(),
                            aborting: true,
                        });

                        aborted = Some(AbortReason::UnsafeOnDisk {
                            message: source.to_string(),
                        });

                        break 'mods;
                    }
                    other => return Err(other.into()),
                },
            };

            if aborted.is_some() {
                break;
            }

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
            report += &apply_outcome.report;

            let mut upserts: Vec<(String, String, u64, i64, Vec<u8>)> = Vec::new();
            let mut deletes: Vec<(String, String)> = Vec::new();
            for update in apply_outcome.index_updates {
                match update {
                    IndexUpdate::UpsertFileState {
                        mod_id,
                        rel_path,
                        size,
                        mtime_ns,
                        checksum,
                    } => upserts.push((mod_id, rel_path, size, mtime_ns, checksum)),
                    IndexUpdate::DeleteFileState { mod_id, rel_path } => {
                        deletes.push((mod_id, rel_path))
                    }
                }
            }
            for hint in cache_hints {
                upserts.push((
                    hint.mod_id,
                    hint.rel_path,
                    hint.size,
                    hint.mtime_ns,
                    hint.checksum,
                ));
            }
            idx.file_state_apply_batch(&desired.state_id, upserts, deletes)?;

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
                let stats = handle_unexpected_paths(
                    &req.checkout_root,
                    &mod_id,
                    &expected,
                    &req.tuning,
                    sink.clone(),
                )
                .await?;
                merge_unexpected(&mut report, stats.clone());
                if matches!(
                    req.tuning.unexpected_paths,
                    crate::types::UnexpectedPathPolicy::Prompt
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
            idx.verified_set(&desired.state_id, now_ns())?;
        } else if !outcome.report.skipped {
            let _ = idx.verified_clear();
        }

        sink.push(SyncEvent::RepairFinished {
            ok: outcome.ok(),
            skipped: outcome.report.skipped,
        });
        Ok(outcome)
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
    manifest: &ValidatedModManifest,
    cache: &HashMap<String, fleet_index::FileState>,
    report: &mut VerifyReport,
    sink: Arc<dyn EventSink>,
) -> Result<()> {
    if req.tuning.auto_fix_case {
        let checkout_root = req.checkout_root.clone();
        let mod_id = manifest.mod_id.clone();
        let expected: Vec<(String, u64, Option<Vec<u8>>)> = manifest
            .files
            .iter()
            .map(|f| (f.rel_path.clone(), f.size, Some(f.file_checksum.clone())))
            .collect();
        let checksummer = req.checksummer.clone();
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
        .await??;
    }

    report.expected_files = report
        .expected_files
        .saturating_add(manifest.files.len() as u64);

    let scan_concurrency = req.tuning.scan_concurrency.max(1);
    let mut outcomes = futures::stream::iter(manifest.files.clone())
        .map(|file| {
            let checkout_root = req.checkout_root.clone();
            let mod_id = manifest.mod_id.clone();
            let rel_path = file.rel_path.clone();
            let cached = cache.get(&rel_path).cloned();
            let checksummer = req.checksummer.clone();
            tokio::task::spawn_blocking(move || {
                verify_one_file(
                    &checkout_root,
                    &mod_id,
                    &rel_path,
                    &file,
                    cached,
                    checksummer.as_ref(),
                )
            })
        })
        .buffer_unordered(scan_concurrency);

    let mut upserts: Vec<(String, String, u64, i64, Vec<u8>)> = Vec::new();
    let mut deletes: Vec<(String, String)> = Vec::new();

    while let Some(res) = outcomes.next().await {
        let outcome = res??;
        apply_verify_outcome(
            req,
            report,
            outcome,
            sink.clone(),
            &mut upserts,
            &mut deletes,
        )?;
    }

    idx.file_state_apply_batch(state_id, upserts, deletes)?;
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
    file: &ValidatedFileEntry,
    cached: Option<fleet_index::FileState>,
    checksummer: &dyn crate::types::Checksummer,
) -> Result<VerifyOutcome> {
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

    if let Some(parent) = abs_path.parent() {
        let mod_root = checkout_root.join(mod_id);
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

    if file.parts.is_empty() {
        let got = checksummer.hash_file(&abs_path)?;
        if got != file.file_checksum {
            return Ok(VerifyOutcome {
                mod_id: mod_id.to_string(),
                rel_path: rel_path.to_string(),
                ok: false,
                size: file.size,
                mtime_ns,
                checksum: file.file_checksum.clone(),
                issue: Some(VerifyIssueKind::PartMismatch {
                    offset: 0,
                    len: file.size,
                }),
                unsafe_message: None,
            });
        }
    } else if let Some((offset, len)) = first_part_mismatch(&abs_path, &file.parts, checksummer)? {
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
    report: &mut VerifyReport,
    outcome: VerifyOutcome,
    sink: Arc<dyn EventSink>,
    upserts: &mut Vec<(String, String, u64, i64, Vec<u8>)>,
    deletes: &mut Vec<(String, String)>,
) -> Result<()> {
    if outcome.ok {
        report.verified_ok += 1;
        upserts.push((
            outcome.mod_id.clone(),
            outcome.rel_path.clone(),
            outcome.size,
            outcome.mtime_ns,
            outcome.checksum.clone(),
        ));
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

    deletes.push((outcome.mod_id, outcome.rel_path));
    Ok(())
}

fn build_baseline(manifests: &[ValidatedModManifest]) -> Vec<ExpectedFile> {
    let mut rows = Vec::new();
    for manifest in manifests {
        for file in &manifest.files {
            rows.push(ExpectedFile {
                mod_id: manifest.mod_id.clone(),
                rel_path: file.rel_path.clone(),
                size: file.size,
            });
        }
    }
    rows
}

fn build_cache_snapshot(
    idx: &FleetIndex,
    state_id: &str,
    manifest: &ValidatedModManifest,
) -> Result<HashMap<String, fleet_index::FileState>> {
    let all = idx.file_state_get_all_for_mod(state_id, &manifest.mod_id)?;
    let mut map = HashMap::new();
    for file in &manifest.files {
        if let Some(state) = all.get(&file.rel_path) {
            map.insert(file.rel_path.clone(), state.clone());
        }
    }
    Ok(map)
}

fn baseline_digest_hex(rows: &[ExpectedFile]) -> String {
    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| (&a.mod_id, &a.rel_path, a.size).cmp(&(&b.mod_id, &b.rel_path, b.size)));
    let mut hasher = blake3::Hasher::new();
    for r in rows {
        hasher.update(r.mod_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(r.rel_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(&r.size.to_le_bytes());
        hasher.update(b"\0");
    }
    hasher.finalize().to_hex().to_string()
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
