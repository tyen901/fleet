use camino::Utf8Path;
use fleet_core::diff::diff as diff_manifests;
use fleet_core::SyncPlan;
use fleet_infra::net::DownloadEvent;
use futures::StreamExt;
use tokio::sync::mpsc::Sender;

use crate::sync::execute::{DefaultPlanExecutor, PlanExecutor};
use crate::sync::local::{DefaultLocalStateProvider, LocalState, LocalStateProvider};
use crate::sync::remote::{HttpRemoteStateProvider, RemoteStateProvider};
use crate::sync::storage::{
    FileRepoSummaryStore, LocalFileSummary, LocalManifestSummary, RepoSummary, RepoSummaryStore,
};
use crate::sync::{SyncError, SyncMode, SyncOptions, SyncRequest, SyncResult, SyncStats};
use fleet_core::path_utils::FleetPath;
use fleet_persistence::{
    CacheDeleteRecord, CacheRenameRecord, CacheUpsertRecord, FleetDataStore, RedbFleetDataStore,
};
use fleet_scanner::Scanner;
use std::collections::HashMap;
use std::sync::Arc;

pub struct DefaultSyncEngine {
    remote: Box<dyn RemoteStateProvider>,
    local: Box<dyn LocalStateProvider>,
    executor: Box<dyn PlanExecutor>,
    fleet_data: Arc<dyn FleetDataStore>,
    repo_summary_store: Arc<dyn RepoSummaryStore>,
}

impl DefaultSyncEngine {
    pub fn new(client: reqwest::Client) -> Self {
        let remote = Box::new(HttpRemoteStateProvider::new(client.clone()));
        let fleet_data: Arc<dyn FleetDataStore> = Arc::new(RedbFleetDataStore);
        let local = Box::new(DefaultLocalStateProvider::new(fleet_data.clone()));
        let executor = Box::new(DefaultPlanExecutor::new(client));
        let repo_summary_store: Arc<dyn RepoSummaryStore> = Arc::new(FileRepoSummaryStore::new());
        Self {
            remote,
            local,
            executor,
            fleet_data,
            repo_summary_store,
        }
    }

    pub fn with_components(
        remote: Box<dyn RemoteStateProvider>,
        local: Box<dyn LocalStateProvider>,
        executor: Box<dyn PlanExecutor>,
        fleet_data: Arc<dyn FleetDataStore>,
        repo_summary_store: Arc<dyn RepoSummaryStore>,
    ) -> Self {
        Self {
            remote,
            local,
            executor,
            fleet_data,
            repo_summary_store,
        }
    }

    /// Step 1: Network only. Fetch repo.json and mod.srf files.
    /// This is the Phase 1: Network Discovery step.
    pub async fn fetch_remote_state(
        &self,
        req: &SyncRequest,
    ) -> Result<crate::sync::FetchResult, SyncError> {
        let cached_repo_summary = if let Some(pid) = req.profile_id.as_deref() {
            self.repo_summary_store
                .load_repo_summary(pid)
                .map_err(|e| SyncError::Remote(format!("load repo summary failed: {e}")))?
        } else {
            None
        };

        let remote_mtime = self
            .remote
            .head_repo_json_mtime(&req.repo_url)
            .await
            .unwrap_or(None);

        let mut repo_external: Option<fleet_core::formats::RepositoryExternal> = None;

        if let (Some(cached), Some(ref mtime)) = (&cached_repo_summary, &remote_mtime) {
            if cached.last_modified.as_ref() == Some(mtime) {
                if let Ok(repo_ext) = serde_json::from_str::<fleet_core::formats::RepositoryExternal>(
                    &cached.repo_json,
                ) {
                    repo_external = Some(repo_ext);
                }
            }
        }

        if repo_external.is_none() {
            let fetched = self.remote.fetch_repo_json(&req.repo_url).await?;

            if let Some(pid) = req.profile_id.as_deref() {
                let summary = RepoSummary {
                    last_modified: remote_mtime.clone(),
                    repo_json: serde_json::to_string(&fetched)
                        .map_err(|e| SyncError::Remote(format!("serialize repo.json: {e}")))?,
                };
                let _ = self.repo_summary_store.save_repo_summary(pid, &summary);
            }

            repo_external = Some(fetched);
        }

        let repository: fleet_core::repo::Repository = repo_external
            .clone()
            .ok_or_else(|| SyncError::Remote("repository unavailable".into()))?
            .into();
        let base = crate::sync::remote::normalize_repo_base(&req.repo_url)?;

        let mut mods = Vec::new();
        // Differential Analysis: reuse local manifest entries when checksum matches
        let mut mods_to_fetch = Vec::new();

        // Try to load the last known manifest we synced to
        let last_known_manifest = self.fleet_data.load_baseline_manifest(&req.local_root).ok();

        let total_mods = repository.required_mods.len();

        for rmod in repository.required_mods {
            let mut found_locally = false;

            if let Some(ref local) = last_known_manifest {
                if let Some(local_mod) = local.mods.iter().find(|m| m.name == rmod.mod_name) {
                    // Only reuse if checksum matches exactly and the caller isn't
                    // running in CacheOnly mode. CacheOnly should always fetch the
                    // remote mod definitions to build a fresh manifest.
                    if local_mod.checksum == rmod.checksum
                        && !matches!(req.mode, SyncMode::CacheOnly)
                    {
                        mods.push(local_mod.clone());
                        found_locally = true;
                    }
                }
            }

            if !found_locally {
                mods_to_fetch.push(rmod);
            }
        }

        let mods_to_fetch_count = mods_to_fetch.len();

        // Fetch only what changed, concurrently
        let remote_ref = &*self.remote;
        let fetch_stream = futures::stream::iter(mods_to_fetch.into_iter())
            .map(move |rmod| {
                let base = base.clone();
                let remote = remote_ref;
                async move { remote.fetch_mod_srf(&base, &rmod.mod_name).await }
            })
            .buffer_unordered(20);

        let results: Vec<Result<fleet_core::Mod, SyncError>> = fetch_stream.collect().await;

        for res in results {
            mods.push(res?);
        }

        let stats = crate::sync::FetchStats {
            mods_total: total_mods,
            mods_fetched: mods_to_fetch_count,
            mods_cached: total_mods.saturating_sub(mods_to_fetch_count),
        };

        Ok(crate::sync::FetchResult {
            manifest: fleet_core::Manifest {
                version: "1.0".to_string(),
                mods,
            },
            stats,
        })
    }

    /// Validate that the repository URL is reachable and returns a parsable repo.json.
    pub async fn validate_repo_url(&self, repo_url: &str) -> Result<(), SyncError> {
        let _ = self.remote.fetch_repo_json(repo_url).await?;
        Ok(())
    }

    /// Step 2: Disk only. Hash/stat local files with optional progress callbacks.
    pub async fn scan_local_state(
        &self,
        req: &SyncRequest,
        on_progress: Option<Box<dyn Fn(fleet_scanner::ScanStats) + Send + Sync>>,
    ) -> Result<LocalState, SyncError> {
        self.local
            .local_state(&req.local_root, req.mode, on_progress)
            .await
    }

    /// Step 3: CPU only. Diff remote + local into a SyncPlan.
    pub fn compute_plan(
        &self,
        remote: &fleet_core::Manifest,
        local: &LocalState,
        req: &SyncRequest,
    ) -> Result<SyncPlan, SyncError> {
        Ok(diff_manifests(remote, &local.manifest))
    }

    /// Builds a plan without any network I/O by comparing current local state against the last
    /// persisted local summary stored in `fleet.redb` (captured at the end of a successful sync).
    pub fn compute_local_integrity_plan(
        &self,
        req: &SyncRequest,
        local: &LocalState,
    ) -> Result<SyncPlan, SyncError> {
        let expected = self
            .fleet_data
            .load_baseline_summary(&req.local_root)
            .map_err(|e| match e.kind() {
                fleet_persistence::StorageErrorKind::Missing => {
                    SyncError::Local("Local baseline missing (run `repair` to initialize)".into())
                }
                fleet_persistence::StorageErrorKind::Busy => SyncError::Local(
                    "Local database is busy (another Fleet instance may be running)".into(),
                ),
                fleet_persistence::StorageErrorKind::NewerSchema => SyncError::Local(
                    "Local database is from a newer Fleet; update Fleet and try again".into(),
                ),
                fleet_persistence::StorageErrorKind::Corrupt => {
                    SyncError::Local("Local database is corrupt; run `repair` to recreate".into())
                }
                _ => SyncError::Local(format!("fleet.redb baseline load failed: {e}")),
            })?;

        let current = local
            .summary
            .clone()
            .ok_or_else(|| SyncError::Local("Local scan did not produce a summary".into()))?;

        Ok(build_fast_plan(&expected, &current))
    }

    /// Pure planning step - fetch remote, scan local, diff.
    pub async fn plan(&self, req: &SyncRequest) -> Result<SyncPlan, SyncError> {
        let fetch_res = self.fetch_remote_state(req).await?;
        let local = self.scan_local_state(req, None).await?;
        self.compute_plan(&fetch_res.manifest, &local, req)
    }

    /// Plan + execute.
    pub async fn plan_and_execute(
        &self,
        req: &SyncRequest,
        progress_tx: Option<Sender<DownloadEvent>>,
    ) -> Result<SyncResult, SyncError> {
        let fetch_res = self.fetch_remote_state(req).await?;
        let manifest = fetch_res.manifest;
        let local = self.scan_local_state(req, None).await?;
        let plan = self.compute_plan(&manifest, &local, req)?;
        self.execute_with_plan_internal(req, plan, Some(manifest), progress_tx)
            .await
    }

    pub async fn execute_with_plan(
        &self,
        req: &SyncRequest,
        plan: SyncPlan,
        progress_tx: Option<Sender<DownloadEvent>>,
    ) -> Result<SyncResult, SyncError> {
        self.execute_with_plan_internal(req, plan, None, progress_tx)
            .await
    }

    async fn execute_with_plan_internal(
        &self,
        req: &SyncRequest,
        plan: SyncPlan,
        remote_manifest: Option<fleet_core::Manifest>,
        progress_tx: Option<Sender<DownloadEvent>>,
    ) -> Result<SyncResult, SyncError> {
        if plan.deletes.is_empty() && plan.renames.is_empty() && plan.downloads.is_empty() {
            return Ok(SyncResult {
                plan,
                executed: false,
                stats: SyncStats::default(),
            });
        }

        let (artifacts, stats) = self
            .executor
            .execute(
                &req.local_root,
                &req.repo_url,
                plan.clone(),
                &req.options,
                progress_tx,
            )
            .await?;

        let manifest_to_save = if let Some(m) = remote_manifest {
            m
        } else {
            self.remote
                .fetch_remote(&req.repo_url)
                .await
                .map_err(|e| SyncError::Remote(format!("{e}")))?
                .manifest
        };

        let summary = compute_summary_from_manifest(&req.local_root, &manifest_to_save);

        let cache_updates: Vec<CacheUpsertRecord> = artifacts
            .iter()
            .map(|a| CacheUpsertRecord {
                mod_name: a.mod_name.clone(),
                rel_path: a.rel_path.clone(),
                mtime: a.final_mtime,
                size: a.size,
                checksum: a.checksum.clone(),
            })
            .collect();

        let cache_deletes = plan
            .deletes
            .iter()
            .filter_map(|d| split_mod_rel(&d.path))
            .map(|(mod_name, rel_path)| CacheDeleteRecord { mod_name, rel_path })
            .collect::<Vec<_>>();

        let cache_renames = plan
            .renames
            .iter()
            .filter_map(|r| {
                let (old_mod, old_rel) = split_mod_rel(&r.old_path)?;
                let (new_mod, new_rel) = split_mod_rel(&r.new_path)?;
                if old_mod != new_mod {
                    return None;
                }
                Some(CacheRenameRecord {
                    mod_name: old_mod,
                    old_rel_path: old_rel?,
                    new_rel_path: new_rel?,
                })
            })
            .collect::<Vec<_>>();

        self.fleet_data
            .commit_sync_snapshot(
                &req.local_root,
                &manifest_to_save,
                &summary,
                &cache_updates,
                &cache_deletes,
                &cache_renames,
            )
            .map_err(|e| SyncError::Local(format!("fleet.redb commit failed: {e}")))?;

        Ok(SyncResult {
            plan,
            executed: true,
            stats,
        })
    }

    /// Persist the given manifest as the local baseline and write a matching summary file.
    /// This is used by "repair" to bootstrap `fleet.redb` without executing a sync.
    pub fn persist_remote_snapshot(
        &self,
        root: &Utf8Path,
        manifest: &fleet_core::Manifest,
    ) -> Result<(), SyncError> {
        let summary = compute_summary_from_manifest(root, manifest);
        self.fleet_data
            .commit_repair_snapshot(root, manifest, &summary)
            .map_err(|e| SyncError::Local(format!("fleet.redb repair commit failed: {e}")))?;

        Ok(())
    }
}

fn split_mod_rel(path: &str) -> Option<(String, Option<String>)> {
    let cleaned = path.trim_end_matches('/');
    if let Some((mod_name, rel)) = cleaned.split_once('/') {
        if rel.is_empty() {
            return Some((mod_name.to_string(), None));
        }
        return Some((mod_name.to_string(), Some(FleetPath::normalize(rel))));
    }
    Some((cleaned.to_string(), None))
}

fn compute_summary_from_manifest(
    root: &Utf8Path,
    manifest: &fleet_core::Manifest,
) -> Vec<LocalManifestSummary> {
    let mut summaries = Vec::new();
    for m in &manifest.mods {
        let mod_root = root.join(&m.name);
        let mut files = Vec::new();
        for f in &m.files {
            // Normalize remote path so it matches Scanner's normalized output
            let normalized = FleetPath::normalize(&f.path);
            let fs_path = mod_root.join(&normalized);
            if let Ok(meta) = std::fs::metadata(&fs_path) {
                let mtime = Scanner::mtime(&meta);
                files.push(LocalFileSummary {
                    rel_path: normalized.clone(),
                    mtime,
                    size: meta.len(),
                    checksum: f.checksum.clone(),
                });
            } else {
                files.push(LocalFileSummary {
                    rel_path: normalized.clone(),
                    mtime: 0,
                    size: f.length,
                    checksum: f.checksum.clone(),
                });
            }
        }
        summaries.push(LocalManifestSummary {
            mod_name: m.name.clone(),
            files,
        });
    }
    summaries
}

#[derive(Debug, Default)]
struct SummaryDiff {
    changed_files: Vec<String>,
    missing_files: Vec<String>,
    extra_files: Vec<String>,
}

fn diff_summary(repo: &LocalManifestSummary, local: &LocalManifestSummary) -> SummaryDiff {
    let repo_map: HashMap<_, _> = repo
        .files
        .iter()
        .map(|f| (f.rel_path.clone(), (f.mtime, f.size)))
        .collect();
    let local_map: HashMap<_, _> = local
        .files
        .iter()
        .map(|f| (f.rel_path.clone(), (f.mtime, f.size)))
        .collect();

    let mut changed_files = Vec::new();
    let mut missing_files = Vec::new();
    let mut extra_files = Vec::new();

    for (rel, (mtime, size)) in &repo_map {
        match local_map.get(rel) {
            Some((l_mtime, l_size)) => {
                if l_mtime != mtime || l_size != size {
                    changed_files.push(rel.clone());
                }
            }
            None => missing_files.push(rel.clone()),
        }
    }

    for rel in local_map.keys() {
        if !repo_map.contains_key(rel) {
            extra_files.push(rel.clone());
        }
    }

    SummaryDiff {
        changed_files,
        missing_files,
        extra_files,
    }
}

fn build_fast_plan(
    expected: &[LocalManifestSummary],
    current: &[LocalManifestSummary],
) -> SyncPlan {
    let mut downloads = Vec::new();
    let mut deletes = Vec::new();

    let current_map: HashMap<_, _> = current.iter().map(|m| (m.mod_name.clone(), m)).collect();

    for repo_mod in expected {
        if let Some(local_mod) = current_map.get(&repo_mod.mod_name) {
            let diff = diff_summary(repo_mod, local_mod);
            for rel in diff.changed_files.iter().chain(diff.missing_files.iter()) {
                if let Some(file) = repo_mod.files.iter().find(|f| &f.rel_path == rel) {
                    downloads.push(fleet_core::DownloadAction {
                        mod_name: repo_mod.mod_name.clone(),
                        rel_path: file.rel_path.clone(),
                        size: file.size,
                        expected_checksum: file.checksum.clone(),
                    });
                }
            }
            for rel in diff.extra_files {
                deletes.push(fleet_core::DeleteAction {
                    path: format!("{}/{}", repo_mod.mod_name, rel),
                });
            }
        } else {
            for file in &repo_mod.files {
                downloads.push(fleet_core::DownloadAction {
                    mod_name: repo_mod.mod_name.clone(),
                    rel_path: file.rel_path.clone(),
                    size: file.size,
                    expected_checksum: file.checksum.clone(),
                });
            }
        }
    }

    for local_mod in current {
        if !expected.iter().any(|m| m.mod_name == local_mod.mod_name) {
            deletes.push(fleet_core::DeleteAction {
                path: local_mod.mod_name.clone(),
            });
        }
    }

    SyncPlan {
        renames: Vec::new(),
        checks: Vec::new(),
        downloads,
        deletes,
    }
}

#[cfg(test)]
mod tests {
    use super::build_fast_plan;
    use crate::sync::storage::{LocalFileSummary, LocalManifestSummary};

    #[test]
    fn fast_plan_detects_changes() {
        let expected = vec![LocalManifestSummary {
            mod_name: "@m".into(),
            files: vec![
                LocalFileSummary {
                    rel_path: "a.txt".into(),
                    mtime: 1,
                    size: 10,
                    checksum: "abc".into(),
                },
                LocalFileSummary {
                    rel_path: "b.txt".into(),
                    mtime: 1,
                    size: 5,
                    checksum: "def".into(),
                },
            ],
        }];
        let current = vec![LocalManifestSummary {
            mod_name: "@m".into(),
            files: vec![
                LocalFileSummary {
                    rel_path: "a.txt".into(),
                    mtime: 2,
                    size: 10,
                    checksum: "".into(),
                },
                LocalFileSummary {
                    rel_path: "c.txt".into(),
                    mtime: 1,
                    size: 1,
                    checksum: "".into(),
                },
            ],
        }];

        let plan = build_fast_plan(&expected, &current);
        assert_eq!(plan.downloads.len(), 2);
        assert_eq!(plan.deletes.len(), 1);
        assert!(plan
            .downloads
            .iter()
            .any(|d| d.mod_name == "@m" && d.rel_path == "a.txt"));
        assert!(plan.deletes.iter().any(|d| d.path == "@m/c.txt"));
    }
}
