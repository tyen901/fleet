use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::events::SyncEvent;
use crate::fetch::fetch_all;
use crate::manifest::{ValidatedFileEntry, ValidatedModManifest};
use crate::model::{
    CheckIssue, CheckIssueKind, CheckReport, CheckRequest, FileStateDelete, FileStateUpsert,
    TimestampNs,
};
use crate::ports::{Checksummer, EventSink, RemoteRepo, StateStore};
use crate::safe_fs::ensure_no_symlink_ancestors;
use crate::safe_path::safe_join_mod_file;
use crate::time_util::{file_mtime_ns, now_ns};
use futures::StreamExt;

pub(crate) async fn run(
    req: CheckRequest,
    remote: Arc<dyn RemoteRepo>,
    store: Arc<dyn StateStore>,
    checksummer: Arc<dyn Checksummer>,
    sink: &dyn EventSink,
) -> Result<CheckReport, crate::model::EngineError> {
    let start = Instant::now();
    sink.push(SyncEvent::VerifyStarted {
        repo: req.repo_name.clone(),
    });

    let result: anyhow::Result<CheckReport> = async {
        tokio::fs::create_dir_all(req.checkout_root.join(".fleet")).await?;

        let desired = store
            .desired_state_get()?
            .ok_or_else(|| anyhow::anyhow!("desired_state missing"))?;
        super::validate_enabled_mods(&desired.enabled_mods_hash, &req.enabled_mods)?;

        let fetch = fetch_all(remote.clone(), &req.enabled_mods, req.tuning.scan_concurrency).await?;
        sink.push(SyncEvent::RemoteCapabilities {
            supports_ranges: fetch.capabilities.supports_ranges,
        });

        let baseline = build_baseline(&fetch.manifests);
        let baseline_digest = super::baseline_digest_hex(&baseline);
        store.expected_replace_all_if_digest_changed(&desired.state_id, baseline, &baseline_digest)?;

        let mut report = CheckReport::default();

        for manifest in &fetch.manifests {
            sink.push(SyncEvent::ModStarted {
                mod_id: manifest.mod_id.clone(),
            });

            let cache = if req.tuning.use_index {
                super::build_cache_snapshot(store.as_ref(), &desired.state_id, manifest)?
            } else {
                HashMap::new()
            };

            verify_mod(
                &req,
                store.as_ref(),
                &desired.state_id,
                manifest,
                &cache,
                &mut report,
                sink,
                checksummer.clone(),
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
            store.verified_set(&desired.state_id, TimestampNs(now_ns()))?;
        } else {
            store.verified_clear()?;
        }

        report.elapsed_ms = start.elapsed().as_millis() as u64;
        sink.push(SyncEvent::VerifyFinished { ok: report.ok });
        Ok(report)
    }
    .await;

    if result.is_err() {
        let _ = store.verified_clear();
        sink.push(SyncEvent::VerifyFinished { ok: false });
    }

    result.map_err(crate::model::EngineError::Internal)
}

async fn verify_mod(
    req: &CheckRequest,
    store: &dyn StateStore,
    state_id: &str,
    manifest: &ValidatedModManifest,
    cache: &HashMap<String, crate::model::FileState>,
    report: &mut CheckReport,
    sink: &dyn EventSink,
    checksummer: Arc<dyn Checksummer>,
) -> anyhow::Result<()> {
    if req.tuning.auto_fix_case {
        let checkout_root = req.checkout_root.clone();
        let mod_id = manifest.mod_id.clone();
        let expected: Vec<(String, u64, Option<Vec<u8>>)> = manifest
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
            fleet_fs_case::case_sweep_and_fix(&checkout_root, &mod_id, &expected, &tuning, Some(&hash_file))
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
            let checksummer = checksummer.clone();
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

    let mut upserts: Vec<FileStateUpsert> = Vec::new();
    let mut deletes: Vec<FileStateDelete> = Vec::new();

    while let Some(res) = outcomes.next().await {
        let outcome = res??;
        apply_verify_outcome(req, report, outcome, sink, &mut upserts, &mut deletes)?;
    }

    store.file_state_apply_batch(state_id, upserts, deletes)?;
    Ok(())
}

struct VerifyOutcome {
    mod_id: String,
    rel_path: String,
    ok: bool,
    size: u64,
    mtime_ns: i64,
    checksum: Vec<u8>,
    issue: Option<CheckIssueKind>,
    unsafe_message: Option<String>,
}

fn verify_one_file(
    checkout_root: &std::path::Path,
    mod_id: &str,
    rel_path: &str,
    file: &ValidatedFileEntry,
    cached: Option<crate::model::FileState>,
    checksummer: &dyn Checksummer,
) -> anyhow::Result<VerifyOutcome> {
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
                issue: Some(CheckIssueKind::UnsafePath),
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
                issue: Some(CheckIssueKind::UnsafeOnDisk),
                unsafe_message: Some(err.to_string()),
            });
        }
    }

    let md = match std::fs::symlink_metadata(&abs_path) {
        Ok(md) => md,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(VerifyOutcome {
                mod_id: mod_id.to_string(),
                rel_path: rel_path.to_string(),
                ok: false,
                size: file.size,
                mtime_ns: 0,
                checksum: file.file_checksum.clone(),
                issue: Some(CheckIssueKind::Missing),
                unsafe_message: None,
            })
        }
        Err(e) => return Err(e.into()),
    };

    let ft = md.file_type();
    if ft.is_symlink() || !ft.is_file() {
        return Ok(VerifyOutcome {
            mod_id: mod_id.to_string(),
            rel_path: rel_path.to_string(),
            ok: false,
            size: file.size,
            mtime_ns: 0,
            checksum: file.file_checksum.clone(),
            issue: Some(CheckIssueKind::NotAFile),
            unsafe_message: None,
        });
    }

    let got_size = md.len();
    if got_size != file.size {
        return Ok(VerifyOutcome {
            mod_id: mod_id.to_string(),
            rel_path: rel_path.to_string(),
            ok: false,
            size: file.size,
            mtime_ns: 0,
            checksum: file.file_checksum.clone(),
            issue: Some(CheckIssueKind::WrongSize {
                expected: file.size,
                got: got_size,
            }),
            unsafe_message: None,
        });
    }

    let Some(mtime_ns) = file_mtime_ns(&md).map(|t| t.0) else {
        return Ok(VerifyOutcome {
            mod_id: mod_id.to_string(),
            rel_path: rel_path.to_string(),
            ok: false,
            size: file.size,
            mtime_ns: 0,
            checksum: file.file_checksum.clone(),
            issue: Some(CheckIssueKind::PartMismatch { offset: 0, len: 0 }),
            unsafe_message: None,
        });
    };

    if let Some(cached) = cached {
        if cached.size == got_size && cached.mtime_ns == mtime_ns && cached.checksum == file.file_checksum {
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

    // Full hash if no parts are present.
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
                issue: Some(CheckIssueKind::PartMismatch { offset: 0, len: file.size }),
                unsafe_message: None,
            });
        }
    } else {
        // Part-based check: hash all parts and detect first mismatch.
        let parts: Vec<(u64, u64)> = file.parts.iter().map(|p| (p.offset, p.len)).collect();
        let got_hashes = checksummer.hash_ranges(&abs_path, &parts)?;
        for (idx, got) in got_hashes.into_iter().enumerate() {
            if got != file.parts[idx].checksum {
                let p = &file.parts[idx];
                return Ok(VerifyOutcome {
                    mod_id: mod_id.to_string(),
                    rel_path: rel_path.to_string(),
                    ok: false,
                    size: file.size,
                    mtime_ns,
                    checksum: file.file_checksum.clone(),
                    issue: Some(CheckIssueKind::PartMismatch {
                        offset: p.offset,
                        len: p.len,
                    }),
                    unsafe_message: None,
                });
            }
        }
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
    req: &CheckRequest,
    report: &mut CheckReport,
    outcome: VerifyOutcome,
    sink: &dyn EventSink,
    upserts: &mut Vec<FileStateUpsert>,
    deletes: &mut Vec<FileStateDelete>,
) -> anyhow::Result<()> {
    if outcome.ok {
        report.verified_ok += 1;
        upserts.push(FileStateUpsert {
            mod_id: outcome.mod_id.clone(),
            rel_path: outcome.rel_path.clone(),
            size: outcome.size,
            mtime_ns: TimestampNs(outcome.mtime_ns),
            checksum: outcome.checksum.clone(),
        });
        sink.push(SyncEvent::FileVerified {
            mod_id: outcome.mod_id,
            path: outcome.rel_path,
        });
        return Ok(());
    }

    if let Some(kind) = &outcome.issue {
        match kind {
            CheckIssueKind::Missing => report.missing += 1,
            CheckIssueKind::WrongSize { .. } => report.wrong_size += 1,
            CheckIssueKind::NotAFile => report.not_a_file += 1,
            CheckIssueKind::UnsafePath => report.unsafe_path += 1,
            CheckIssueKind::UnsafeOnDisk => report.unsafe_path += 1,
            CheckIssueKind::PartMismatch { .. } => report.checksum_mismatch += 1,
        }

        if report.issues.len() < req.tuning.max_issues {
            report.issues.push(CheckIssue {
                mod_id: outcome.mod_id.clone(),
                rel_path: outcome.rel_path.clone(),
                kind: kind.clone(),
            });
        }
    }

    if matches!(outcome.issue, Some(CheckIssueKind::UnsafeOnDisk)) {
        sink.push(SyncEvent::Error {
            message: outcome
                .unsafe_message
                .unwrap_or_else(|| "unsafe path (symlink ancestor)".to_string()),
        });
    }

    deletes.push(FileStateDelete {
        mod_id: outcome.mod_id,
        rel_path: outcome.rel_path,
    });
    Ok(())
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
