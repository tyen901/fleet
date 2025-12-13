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
    FileManifestStore, FileRepoSummaryStore, LocalFileSummary, LocalManifestSummary, ManifestStore,
    RepoSummary, RepoSummaryStore,
};
use crate::sync::{SyncError, SyncMode, SyncOptions, SyncRequest, SyncResult, SyncStats};
use fleet_core::path_utils::FleetPath;
use fleet_scanner::Scanner;
use std::collections::HashMap;
use std::sync::Arc;

pub struct DefaultSyncEngine {
    remote: Box<dyn RemoteStateProvider>,
    local: Box<dyn LocalStateProvider>,
    executor: Box<dyn PlanExecutor>,
    manifest_store: Arc<dyn ManifestStore>,
    repo_summary_store: Arc<dyn RepoSummaryStore>,
}

impl DefaultSyncEngine {
    pub fn new(client: reqwest::Client) -> Self {
        let remote = Box::new(HttpRemoteStateProvider::new(client.clone()));
        let manifest_store: Arc<dyn ManifestStore> = Arc::new(FileManifestStore::new());
        let local = Box::new(DefaultLocalStateProvider::new(None, manifest_store.clone()));
        let executor = Box::new(DefaultPlanExecutor::new(client));
        let repo_summary_store: Arc<dyn RepoSummaryStore> = Arc::new(FileRepoSummaryStore::new());
        Self {
            remote,
            local,
            executor,
            manifest_store,
            repo_summary_store,
        }
    }

    pub fn with_components(
        remote: Box<dyn RemoteStateProvider>,
        local: Box<dyn LocalStateProvider>,
        executor: Box<dyn PlanExecutor>,
        manifest_store: Arc<dyn ManifestStore>,
        repo_summary_store: Arc<dyn RepoSummaryStore>,
    ) -> Self {
        Self {
            remote,
            local,
            executor,
            manifest_store,
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
        let last_known_manifest = self.manifest_store.load(&req.local_root).ok();

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
    /// saved local summary (captured at the end of a successful sync).
    pub fn compute_local_integrity_plan(
        &self,
        req: &SyncRequest,
        local: &LocalState,
    ) -> Result<SyncPlan, SyncError> {
        let empty = || SyncPlan {
            renames: Vec::new(),
            checks: Vec::new(),
            downloads: Vec::new(),
            deletes: Vec::new(),
        };

        let expected = match self.manifest_store.load_summary(&req.local_root) {
            Ok(s) => s,
            Err(_) => return Ok(empty()),
        };
        let current = match local.summary.clone() {
            Some(s) => s,
            None => return Ok(empty()),
        };

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

        if let Err(e) = self.manifest_store.save(&req.local_root, &manifest_to_save) {
            return Err(SyncError::Local(format!("manifest save failed: {e}")));
        }
        let summary = compute_summary_from_manifest(&req.local_root, &manifest_to_save);
        if let Err(e) = self.manifest_store.save_summary(&req.local_root, &summary) {
            return Err(SyncError::Local(format!("summary save failed: {e}")));
        }

        // Suppress unused warning for artifacts in case we don't use them further yet.
        let _ = artifacts;

        Ok(SyncResult {
            plan,
            executed: true,
            stats,
        })
    }

    /// Persist the given manifest as the local baseline and write a matching summary file.
    /// This is used by "repair" to bootstrap `.fleet-local-manifest.json` + `.fleet-local-summary.json`
    /// without executing a sync.
    pub fn persist_remote_snapshot(
        &self,
        root: &Utf8Path,
        manifest: &fleet_core::Manifest,
    ) -> Result<(), SyncError> {
        self.manifest_store
            .save(root, manifest)
            .map_err(|e| SyncError::Local(format!("manifest save failed: {e}")))?;

        let summary = compute_summary_from_manifest(root, manifest);
        self.manifest_store
            .save_summary(root, &summary)
            .map_err(|e| SyncError::Local(format!("summary save failed: {e}")))?;

        Ok(())
    }
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
    use super::*;
    use crate::sync::execute::SyncArtifact;
    use crate::sync::local::{LocalState, LocalTrustLevel};
    use crate::sync::storage::{
        LocalFileSummary, LocalManifestSummary, ManifestStore, RepoSummaryStore,
    };
    use camino::Utf8PathBuf;
    use fleet_core::formats::repo::RepoModExternal;
    use fleet_core::repo::RepoMod;
    use fleet_core::{File, FileType, Manifest, Mod};
    use std::sync::Mutex;
    use tokio::sync::mpsc::Sender;

    struct MemoryManifestStore {
        inner: Mutex<Option<Manifest>>,
        summary: Mutex<Option<Vec<LocalManifestSummary>>>,
    }

    impl MemoryManifestStore {
        fn new(manifest: Option<Manifest>) -> Arc<Self> {
            Arc::new(Self {
                inner: Mutex::new(manifest),
                summary: Mutex::new(None),
            })
        }
    }

    impl ManifestStore for MemoryManifestStore {
        fn load(&self, _root: &Utf8Path) -> Result<Manifest, String> {
            self.inner
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| "missing".to_string())
        }

        fn save(&self, _root: &Utf8Path, manifest: &Manifest) -> Result<(), String> {
            *self.inner.lock().unwrap() = Some(manifest.clone());
            Ok(())
        }

        fn load_summary(&self, _root: &Utf8Path) -> Result<Vec<LocalManifestSummary>, String> {
            self.summary
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| "missing".into())
        }

        fn save_summary(
            &self,
            _root: &Utf8Path,
            summary: &[LocalManifestSummary],
        ) -> Result<(), String> {
            *self.summary.lock().unwrap() = Some(summary.to_vec());
            Ok(())
        }
    }

    struct FakeRemote {
        manifest: Manifest,
    }

    #[async_trait::async_trait]
    impl RemoteStateProvider for FakeRemote {
        async fn head_repo_json_mtime(&self, _repo_url: &str) -> Result<Option<String>, SyncError> {
            Ok(None)
        }

        async fn fetch_repo_json(
            &self,
            _repo_url: &str,
        ) -> Result<fleet_core::formats::RepositoryExternal, SyncError> {
            let mods: Vec<RepoModExternal> = self
                .manifest
                .mods
                .iter()
                .map(|m| RepoModExternal {
                    mod_name: m.name.clone(),
                    checksum: m.checksum.clone(),
                    enabled: true,
                })
                .collect();
            Ok(fleet_core::formats::RepositoryExternal {
                repo_name: "test".into(),
                checksum: "c".into(),
                required_mods: mods.clone(),
                optional_mods: Vec::new(),
            })
        }

        async fn fetch_mod_srf(
            &self,
            _base: &reqwest::Url,
            mod_name: &str,
        ) -> Result<Mod, SyncError> {
            self.manifest
                .mods
                .iter()
                .find(|m| m.name == mod_name)
                .cloned()
                .ok_or_else(|| SyncError::Remote("mod not found".into()))
        }

        async fn fetch_remote(
            &self,
            _repo_url: &str,
        ) -> Result<crate::sync::remote::RemoteState, SyncError> {
            Ok(crate::sync::remote::RemoteState {
                manifest: self.manifest.clone(),
            })
        }
    }

    struct NoopRepoSummaryStore;

    impl RepoSummaryStore for NoopRepoSummaryStore {
        fn load_repo_summary(&self, _profile_id: &str) -> Result<Option<RepoSummary>, String> {
            Ok(None)
        }

        fn save_repo_summary(
            &self,
            _profile_id: &str,
            _summary: &RepoSummary,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    struct FakeLocal {
        manifest: Manifest,
    }

    #[async_trait::async_trait]
    impl LocalStateProvider for FakeLocal {
        async fn local_state(
            &self,
            _root: &Utf8Path,
            _mode: SyncMode,
            _on_progress: Option<Box<dyn Fn(fleet_scanner::ScanStats) + Send + Sync>>,
        ) -> Result<LocalState, SyncError> {
            Ok(LocalState {
                manifest: self.manifest.clone(),
                summary: None,
                trust: LocalTrustLevel::CacheOnly,
            })
        }
    }

    struct NoopExecutor;

    #[async_trait::async_trait]
    impl PlanExecutor for NoopExecutor {
        async fn execute(
            &self,
            _root: &Utf8Path,
            _repo_url: &str,
            plan: SyncPlan,
            _opts: &SyncOptions,
            _progress_tx: Option<Sender<DownloadEvent>>,
        ) -> Result<(Vec<SyncArtifact>, SyncStats), SyncError> {
            Ok((
                Vec::new(),
                SyncStats {
                    files_planned_download: plan.downloads.len() as u64,
                    ..SyncStats::default()
                },
            ))
        }
    }

    fn simple_manifest(with_file: bool) -> Manifest {
        let mut files = Vec::new();
        if with_file {
            files.push(File {
                path: "addons/a.pbo".into(),
                length: 10,
                checksum: "abc".into(),
                file_type: FileType::File,
                parts: Vec::new(),
            });
        }
        Manifest {
            version: "1.0".into(),
            mods: vec![Mod {
                name: "@m".into(),
                checksum: "m1".into(),
                files,
            }],
        }
    }

    #[tokio::test]
    async fn plan_empty_when_manifests_match() {
        let manifest = simple_manifest(true);
        let store = MemoryManifestStore::new(Some(manifest.clone()));
        let engine = DefaultSyncEngine::with_components(
            Box::new(FakeRemote {
                manifest: manifest.clone(),
            }),
            Box::new(FakeLocal {
                manifest: manifest.clone(),
            }),
            Box::new(NoopExecutor),
            store,
            Arc::new(NoopRepoSummaryStore),
        );

        let req = SyncRequest {
            repo_url: "https://x/".into(),
            local_root: Utf8PathBuf::from("/tmp"),
            mode: SyncMode::CacheOnly,
            options: SyncOptions::default(),
            profile_id: None,
        };

        let plan = engine.plan(&req).await.unwrap();
        assert_eq!(plan.downloads.len(), 0);
        assert_eq!(plan.deletes.len(), 0);
    }

    #[tokio::test]
    async fn plan_includes_download_when_missing_locally() {
        let remote_manifest = simple_manifest(true);
        let local_manifest = simple_manifest(false);
        let store = MemoryManifestStore::new(Some(local_manifest.clone()));
        let engine = DefaultSyncEngine::with_components(
            Box::new(FakeRemote {
                manifest: remote_manifest,
            }),
            Box::new(FakeLocal {
                manifest: local_manifest,
            }),
            Box::new(NoopExecutor),
            store,
            Arc::new(NoopRepoSummaryStore),
        );

        let req = SyncRequest {
            repo_url: "https://x/".into(),
            local_root: Utf8PathBuf::from("/tmp"),
            mode: SyncMode::CacheOnly,
            options: SyncOptions::default(),
            profile_id: None,
        };

        let plan = engine.plan(&req).await.unwrap();
        assert_eq!(plan.downloads.len(), 1);
    }

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
        assert!(plan
            .downloads
            .iter()
            .any(|d| d.mod_name == "@m" && d.rel_path == "b.txt"));
        assert!(plan.deletes.iter().any(|d| d.path == "@m/c.txt"));
    }
}
