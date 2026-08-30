use fleet_domain::LocalStateHealth;
use fleet_inventory::{
    target_path_from_relative_path, InventoryDesiredFile, InventoryError, InventoryObservedFile,
    InventoryReconcileMode, InventoryReconcileWrite, MaterializationInventory,
};
use futures_util::{stream, StreamExt, TryStreamExt};
use ignore::{gitignore::GitignoreBuilder, WalkBuilder};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub(crate) enum ReconcileProgress {
    Walking {
        files: u64,
        bytes: u64,
    },
    Scanning {
        files_done: u64,
        files_total: u64,
        bytes_done: u64,
        bytes_total: u64,
    },
    Finalizing,
}

#[derive(Clone, Debug)]
pub(crate) struct LocalContentSnapshot {
    pub(crate) profile_id: String,
    pub(crate) health: LocalStateHealth,
    pub(crate) checked_at_unix_ms: u64,
    pub(crate) observed_freshness: BTreeMap<String, flux::FreshnessProof>,
    pub(crate) exact_paths: Vec<String>,
    pub(crate) missing_paths: Vec<String>,
    pub(crate) modified_paths: Vec<String>,
    pub(crate) unexpected_paths: Vec<String>,
}

#[derive(Clone)]
struct ObservedFile {
    path: flux::TargetPath,
    freshness: flux::FreshnessProof,
}

pub(crate) struct LocalReconcileJob {
    pub(crate) inventory: MaterializationInventory,
    pub(crate) profile_id: String,
    pub(crate) dest: PathBuf,
    pub(crate) manifest: flux::ValidatedManifest,
    pub(crate) ignore_rules: String,
    pub(crate) mode: InventoryReconcileMode,
    pub(crate) cancel: CancellationToken,
}

pub(crate) async fn reconcile_inventory(
    job: LocalReconcileJob,
    progress: Option<Arc<dyn Fn(ReconcileProgress) + Send + Sync>>,
) -> Result<LocalContentSnapshot, InventoryError> {
    let LocalReconcileJob {
        inventory,
        profile_id,
        dest,
        manifest,
        ignore_rules,
        mode,
        cancel,
    } = job;
    let desired = desired_files(&manifest);
    if !dest.exists() {
        return Ok(LocalContentSnapshot {
            profile_id,
            health: LocalStateHealth::MissingDestination,
            checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
            observed_freshness: BTreeMap::new(),
            exact_paths: Vec::new(),
            missing_paths: desired
                .iter()
                .map(|file| file.path.as_str().to_string())
                .collect(),
            modified_paths: Vec::new(),
            unexpected_paths: Vec::new(),
        });
    }

    let target = Arc::new(
        flux::TargetSession::open(&flux::TargetSpec { root: dest.clone() })
            .map_err(|error| InventoryError::Message(error.to_string()))?,
    );
    let observed = tokio::task::spawn_blocking({
        let dest = dest.clone();
        let target = Arc::clone(&target);
        let cancel = cancel.clone();
        move || observe_files(&dest, &target, &ignore_rules, &cancel)
    })
    .await
    .map_err(|error| InventoryError::Message(error.to_string()))??;
    if let Some(sink) = progress.as_ref() {
        sink(ReconcileProgress::Walking {
            files: observed.len() as u64,
            bytes: observed.iter().map(|item| item.freshness.len).sum(),
        });
    }
    let observed_facts = observed
        .iter()
        .map(|item| InventoryObservedFile {
            path: item.path.clone(),
            freshness: item.freshness,
        })
        .collect::<Vec<_>>();
    let plan = tokio::task::spawn_blocking({
        let inventory = inventory.clone();
        let observed = observed_facts.clone();
        let desired = desired.clone();
        move || inventory.plan_reconcile(&observed, &desired, mode)
    })
    .await
    .map_err(|error| InventoryError::Message(error.to_string()))??;
    let candidates = plan
        .scan_candidate_positions
        .iter()
        .map(|position| observed[*position].clone())
        .collect::<Vec<_>>();
    let files_total = candidates.len() as u64;
    let bytes_total = candidates
        .iter()
        .map(|item| item.freshness.len)
        .sum::<u64>();
    let files_done = Arc::new(AtomicU64::new(0));
    let bytes_done = Arc::new(AtomicU64::new(0));
    let parallelism = std::thread::available_parallelism()
        .map_err(|error| InventoryError::Message(error.to_string()))?;
    let scanned = stream::iter(candidates.into_iter().map(|item| {
        let target = Arc::clone(&target);
        let cancel = cancel.clone();
        let progress = progress.clone();
        let files_done = Arc::clone(&files_done);
        let bytes_done = Arc::clone(&bytes_done);
        async move {
            if cancel.is_cancelled() {
                return Err(InventoryError::Canceled);
            }
            let fact = scan_target_file(&item, &target).await?;
            let completed_files = files_done.fetch_add(1, Ordering::Relaxed) + 1;
            let completed_bytes =
                bytes_done.fetch_add(item.freshness.len, Ordering::Relaxed) + item.freshness.len;
            if let Some(sink) = progress.as_ref() {
                sink(ReconcileProgress::Scanning {
                    files_done: completed_files,
                    files_total,
                    bytes_done: completed_bytes,
                    bytes_total,
                });
            }
            Ok(fact)
        }
    }))
    .buffer_unordered(parallelism.get())
    .try_collect::<Vec<_>>()
    .await?;
    let assessment = tokio::task::spawn_blocking({
        let inventory = inventory.clone();
        let desired = desired.clone();
        move || {
            inventory.apply_reconcile(InventoryReconcileWrite {
                managed_paths: plan.managed_paths,
                upsert_facts: scanned,
                remove_reusable_facts: plan.remove_reusable_facts,
            })?;
            inventory.assess_expected(&desired)
        }
    })
    .await
    .map_err(|error| InventoryError::Message(error.to_string()))??;
    if let Some(sink) = progress.as_ref() {
        sink(ReconcileProgress::Finalizing);
    }
    let health = if assessment.missing_paths.is_empty()
        && assessment.modified_paths.is_empty()
        && assessment.unexpected_paths.is_empty()
    {
        LocalStateHealth::Ready
    } else {
        LocalStateHealth::LocalDrift
    };
    Ok(LocalContentSnapshot {
        profile_id,
        health,
        checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        observed_freshness: observed
            .iter()
            .map(|item| (item.path.as_str().to_string(), item.freshness))
            .collect(),
        exact_paths: assessment.exact_paths,
        missing_paths: assessment.missing_paths,
        modified_paths: assessment.modified_paths,
        unexpected_paths: assessment.unexpected_paths,
    })
}

fn observe_files(
    dest: &Path,
    target: &flux::TargetSession,
    ignore_rules_text: &str,
    cancel: &CancellationToken,
) -> Result<Vec<ObservedFile>, InventoryError> {
    let ignore_matcher = inline_ignore_matcher(dest, ignore_rules_text)?;
    let mut builder = WalkBuilder::new(dest);
    builder
        .follow_links(false)
        .hidden(false)
        .parents(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .ignore(true)
        .sort_by_file_path(|left, right| left.cmp(right));
    if let Some(ignore_matcher) = ignore_matcher {
        builder.filter_entry(move |entry| {
            !ignore_matcher
                .matched(
                    entry.path(),
                    entry.file_type().is_some_and(|kind| kind.is_dir()),
                )
                .is_ignore()
        });
    }
    let mut observed = Vec::new();
    for entry in builder.build() {
        if cancel.is_cancelled() {
            return Err(InventoryError::Canceled);
        }
        let entry = entry.map_err(|error| InventoryError::Message(error.to_string()))?;
        if entry.depth() == 0 {
            continue;
        }
        let Some(kind) = entry.file_type() else {
            return Err(InventoryError::Message(
                "walk entry has no file type".to_string(),
            ));
        };
        if !kind.is_file() || kind.is_symlink() {
            continue;
        }
        let fs_path = entry.into_path();
        let rel_path = fs_path
            .strip_prefix(dest)
            .map_err(|error| InventoryError::Message(error.to_string()))?;
        let path = target_path_from_relative_path(rel_path)?;
        let freshness = target
            .freshness_for_path(&path)
            .map_err(|error| InventoryError::Message(error.to_string()))?;
        observed.push(ObservedFile { path, freshness });
    }
    observed.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(observed)
}

async fn scan_target_file(
    item: &ObservedFile,
    target: &flux::TargetSession,
) -> Result<flux::LocalFileFact, InventoryError> {
    let before = item.freshness;
    let profile = fleet_flux::SwiftyFluxProfile;
    let mut scanner = flux::ContentProfile::begin_file_inventory(&profile, &item.path, before.len)
        .map_err(|error| InventoryError::Message(error.to_string()))?;
    let mut segments = Vec::new();
    if before.len > 0 {
        let bytes = target
            .stream_target_range(&item.path, before.len, before, 0..before.len)
            .await
            .map_err(|error| InventoryError::Message(error.to_string()))?;
        futures_util::pin_mut!(bytes);
        while let Some(chunk) = bytes.next().await {
            segments.extend(
                scanner
                    .push(chunk.map_err(|error| InventoryError::Message(error.to_string()))?)
                    .map_err(|error| InventoryError::Message(error.to_string()))?,
            );
        }
    }
    let finished = scanner
        .finish()
        .map_err(|error| InventoryError::Message(error.to_string()))?;
    segments.extend(finished.trailing_segments);
    let fact = flux::LocalFileFact {
        path: item.path.clone(),
        len: before.len,
        freshness: before,
        segments: segments
            .into_iter()
            .map(|segment| flux::LocalFileSegmentFact {
                range: segment.range,
                key: segment.key,
                validation: segment.validation,
            })
            .collect(),
    };
    fact.validate_basic()
        .map_err(|error| InventoryError::Message(error.to_string()))?;
    Ok(fact)
}

pub(crate) fn desired_files(manifest: &flux::ValidatedManifest) -> Vec<InventoryDesiredFile> {
    manifest
        .files
        .iter()
        .map(|file| InventoryDesiredFile {
            path: file.path.clone(),
            size_bytes: file.len,
            segments: file
                .segments
                .iter()
                .map(|segment| flux::LocalFileSegmentFact {
                    range: segment.range.clone(),
                    key: segment.key.clone(),
                    validation: segment.validation.clone(),
                })
                .collect(),
        })
        .collect()
}

fn inline_ignore_matcher(
    dest: &Path,
    ignore_rules_text: &str,
) -> Result<Option<ignore::gitignore::Gitignore>, InventoryError> {
    let trimmed = ignore_rules_text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let mut builder = GitignoreBuilder::new(dest);
    for line in trimmed.lines() {
        builder
            .add_line(None, line)
            .map_err(|error| InventoryError::Message(error.to_string()))?;
    }
    builder
        .build()
        .map(Some)
        .map_err(|error| InventoryError::Message(error.to_string()))
}
