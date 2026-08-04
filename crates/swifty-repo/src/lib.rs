use anyhow::{Context, Result};
use async_trait::async_trait;
use atomic_write_file::AtomicWriteFile;
use reqwest::header::{HeaderMap, HeaderValue, IF_MODIFIED_SINCE, IF_NONE_MATCH};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::debug;

use fleet_download::{DownloadEventSink, DownloadResult, DownloadService, DownloadSpec};

/// Internal cached blob stored as JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepoCacheBlob {
    pub schema_version: u32,
    pub repo_url: String,
    pub repo_fetched_at_unix_ms: u64,
    pub repo: swifty_artifacts::RepoSpec,
    pub mods: BTreeMap<String, CachedModSrf>,
    pub repo_http: Option<HttpCacheHints>,
    #[serde(default)]
    pub icon_image_checksum: Option<String>,
    #[serde(default)]
    pub repo_image_checksum: Option<String>,
    #[serde(default)]
    pub repo_json_checksum: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachedModSrf {
    pub checksum: swifty_artifacts::Md5Digest,
    pub fetched_at_unix_ms: u64,
    pub manifest: swifty_artifacts::SrfMod,
    pub http: Option<HttpCacheHints>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpCacheHints {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedRepoServer {
    pub address: String,
    pub port: u16,
    pub password: String,
}

#[async_trait]
pub trait RepoCacheStore: Send + Sync {
    async fn load_repo_cache(&self, repo_url: &str) -> Result<Option<RepoCacheBlob>>;
    async fn save_repo_cache(&self, repo_url: &str, blob: &RepoCacheBlob) -> Result<()>;
    async fn delete_repo_cache(&self, repo_url: &str) -> Result<()>;
    fn cache_root_path(&self) -> Option<&Path> {
        None
    }
}

/// Simple filesystem-backed store: writes JSON files under a cache dir.
///
/// This is the single authority for cache paths/filenames.
pub struct FsRepoCacheStore {
    root: PathBuf,
}

impl FsRepoCacheStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn blob_path(&self, repo_url: &str) -> PathBuf {
        repo_cache_blob_path(&self.root, repo_url)
    }
}

pub fn repo_cache_key(repo_url: &str) -> String {
    fleet_domain::hash::sha1_hex(repo_url.as_bytes())
}

pub fn repo_cache_blob_path(cache_root: &Path, repo_url: &str) -> PathBuf {
    cache_root.join(format!("{}.json", repo_cache_key(repo_url)))
}

pub fn repo_cache_asset_path(cache_root: &Path, repo_url: &str, asset_name: &str) -> PathBuf {
    let safe_name = asset_name.trim_start_matches('/').replace(['/', '\\'], "_");
    cache_root.join(format!("{}.{}", repo_cache_key(repo_url), safe_name))
}

pub fn load_cached_repo_blocking(
    cache_root: &Path,
    repo_url: &str,
) -> Result<Option<RepoCacheBlob>> {
    let path = repo_cache_blob_path(cache_root, repo_url);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(anyhow::Error::new(err))
                .with_context(|| format!("read cache file {}", path.display()));
        }
    };
    let cache = serde_json::from_slice(&bytes).context("parse cache json")?;
    Ok(Some(cache))
}

pub async fn cached_repo_servers(
    cache_root: &Path,
    repo_url: &str,
) -> Result<Option<Vec<CachedRepoServer>>> {
    let cache = FsRepoCacheStore::new(cache_root.to_path_buf())
        .load_repo_cache(repo_url)
        .await?;
    Ok(cache.map(|cache| {
        cache
            .repo
            .servers
            .into_iter()
            .map(|s| CachedRepoServer {
                address: s.address,
                port: s.port,
                password: s.password,
            })
            .collect()
    }))
}

pub fn enabled_mod_names(cache_root: &Path, repo_url: &str) -> Result<Option<Vec<String>>> {
    Ok(load_cached_repo_blocking(cache_root, repo_url)?
        .map(|cache| collect_enabled_mod_names(cache.repo)))
}

#[async_trait]
impl RepoCacheStore for FsRepoCacheStore {
    async fn load_repo_cache(&self, repo_url: &str) -> Result<Option<RepoCacheBlob>> {
        let p = self.blob_path(repo_url);
        let bytes = match tokio::fs::read(&p).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(anyhow::Error::new(e))
                    .with_context(|| format!("read cache file {}", p.display()))
            }
        };
        let b: RepoCacheBlob = tokio::task::spawn_blocking(move || {
            serde_json::from_slice(&bytes).context("parse cache json")
        })
        .await
        .context("parse cache json task join")??;
        Ok(Some(b))
    }

    async fn save_repo_cache(&self, repo_url: &str, blob: &RepoCacheBlob) -> Result<()> {
        tokio::fs::create_dir_all(&self.root)
            .await
            .with_context(|| format!("create cache dir {}", self.root.display()))?;
        let p = self.blob_path(repo_url);
        let s = serde_json::to_vec(&blob).context("serialize cache blob")?;
        write_bytes_atomically(p, s).await?;
        Ok(())
    }

    async fn delete_repo_cache(&self, repo_url: &str) -> Result<()> {
        let p = self.blob_path(repo_url);
        match tokio::fs::remove_file(&p).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(anyhow::Error::new(e))
                .with_context(|| format!("remove cache file {}", p.display())),
        }
    }

    fn cache_root_path(&self) -> Option<&Path> {
        Some(&self.root)
    }
}

async fn write_bytes_atomically(path: PathBuf, bytes: Vec<u8>) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut file = AtomicWriteFile::options()
            .open(&path)
            .with_context(|| format!("open atomic writer {}", path.display()))?;
        file.write_all(&bytes)
            .with_context(|| format!("write atomic file {}", path.display()))?;
        file.commit()
            .with_context(|| format!("commit atomic file {}", path.display()))?;
        Ok(())
    })
    .await
    .context("atomic file write task join")?
}

pub trait ModSrfResolver: Send + Sync {
    fn mod_srf_url(&self, repo_json_url: &str, mod_name: &str) -> Result<String>;
}

pub struct DefaultModSrfResolver;

impl ModSrfResolver for DefaultModSrfResolver {
    fn mod_srf_url(&self, repo_json_url: &str, mod_name: &str) -> Result<String> {
        // Resolve relative to the repo.json URL.
        // If repo_json_url ends with `/repo.json`, this produces `{mod_name}/mod.srf`.
        let base = url::Url::parse(repo_json_url).context("parse repo url")?;
        let rel = format!("{mod_name}/mod.srf");
        let srf = base.join(&rel).context("join mod.srf url")?;
        debug!(
            repo_url = repo_json_url,
            mod_name,
            mod_rel = rel.as_str(),
            mod_url = srf.as_str(),
            "resolved mod.srf URL"
        );
        Ok(srf.to_string())
    }
}

#[derive(Debug)]
pub struct RepoSyncResult {
    pub repo: swifty_artifacts::RepoSpec,
    pub mods: BTreeMap<String, swifty_artifacts::SrfMod>,
    pub fetched_mods: Vec<String>,
    pub reused_mods: Vec<String>,
    pub freshness: RepoFreshness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepoFreshness {
    Unknown,
    UpToDate,
    UpdateAvailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoProbeResult {
    pub local_revision: Option<String>,
    pub remote_revision: Option<String>,
    pub freshness: RepoFreshness,
}

pub fn repo_blob_revision(cache: &RepoCacheBlob) -> Option<String> {
    if let Some(revision) = cache
        .repo_json_checksum
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(revision.to_string());
    }

    let checksum = cache.repo.checksum.trim();
    if !checksum.is_empty() {
        return Some(checksum.to_string());
    }

    None
}

fn collect_enabled_mod_names(repo: swifty_artifacts::RepoSpec) -> Vec<String> {
    let mut names: Vec<String> = repo
        .required_mods
        .into_iter()
        .chain(repo.optional_mods)
        .filter(|m| m.enabled)
        .map(|m| m.mod_name)
        .collect();
    names.sort();
    names
}

pub async fn sync_repo_metadata(
    repo_url: &str,
    store: &dyn RepoCacheStore,
    resolver: &dyn ModSrfResolver,
    downloads: &DownloadService,
    sink: Option<DownloadEventSink>,
) -> Result<RepoSyncResult> {
    debug!(repo_url, "swifty sync start");

    // Step A: load cache (optional)
    let cache_opt = store.load_repo_cache(repo_url).await?;
    let repo_headers =
        build_conditional_headers(cache_opt.as_ref().and_then(|c| c.repo_http.as_ref()))?;
    let now_ms = fleet_domain::time::now_unix_ms();
    let repo_id = repo_manifest_id(repo_url);
    let repo_fetch = fetch_repo_json(
        downloads,
        repo_id.as_str(),
        repo_url,
        repo_headers,
        sink.clone(),
        repo_id.as_str(),
        "download repo manifest",
    )
    .await?;

    // Step B: fetch repo.json via conditional GET
    let (mut cache, remote_repo, freshness, mut cache_dirty) = match (cache_opt, repo_fetch) {
        (Some(cache), RepoJsonFetch::NotModified) => {
            if cache.repo_fetched_at_unix_ms == 0 {
                anyhow::bail!("received 304 for repo.json but cached repo is empty");
            }
            debug!(repo_url, "repo.json not modified; using cache");
            let repo = cache.repo.clone();
            (cache, repo, RepoFreshness::UpToDate, false)
        }
        (
            Some(mut cache),
            RepoJsonFetch::Downloaded {
                bytes,
                etag,
                last_modified,
                ..
            },
        ) => {
            let repo = swifty_artifacts::read_repo_json(&bytes).context("parse repo.json")?;
            let remote_checksum = fleet_domain::hash::sha1_hex(&bytes);
            let prior_checksum = cache.repo_json_checksum.clone();
            let freshness = match prior_checksum.as_deref() {
                Some(before) if before == remote_checksum.as_str() => RepoFreshness::UpToDate,
                Some(_) => RepoFreshness::UpdateAvailable,
                None => RepoFreshness::Unknown,
            };
            let mut cache_dirty = false;

            if prior_checksum.as_deref() != Some(remote_checksum.as_str()) {
                cache.repo_json_checksum = Some(remote_checksum);
                cache.repo_fetched_at_unix_ms = now_ms;
                cache_dirty = true;
            }

            // Update cache hints from GET response (fallback to prior values if absent)
            let prior = cache.repo_http.clone();
            let merged_repo_http = HttpCacheHints {
                etag: etag.or_else(|| prior.as_ref().and_then(|p| p.etag.clone())),
                last_modified: last_modified
                    .or_else(|| prior.as_ref().and_then(|p| p.last_modified.clone())),
            };
            if cache.repo_http.as_ref() != Some(&merged_repo_http) {
                cache.repo_http = Some(merged_repo_http);
                cache_dirty = true;
            }

            (cache, repo, freshness, cache_dirty)
        }
        (None, RepoJsonFetch::NotModified) => {
            anyhow::bail!("received 304 for repo.json but no cache exists");
        }
        (
            None,
            RepoJsonFetch::Downloaded {
                bytes,
                etag,
                last_modified,
                ..
            },
        ) => {
            let repo = swifty_artifacts::read_repo_json(&bytes).context("parse repo.json")?;

            let cache = RepoCacheBlob {
                schema_version: 1,
                repo_url: repo_url.to_string(),
                repo_fetched_at_unix_ms: now_ms,
                repo: repo.clone(),
                mods: BTreeMap::new(),
                repo_http: Some(HttpCacheHints {
                    etag,
                    last_modified,
                }),
                icon_image_checksum: None,
                repo_image_checksum: None,
                repo_json_checksum: Some(fleet_domain::hash::sha1_hex(&bytes)),
            };

            (cache, repo, RepoFreshness::Unknown, true)
        }
    };

    if let Some(cache_root) = store.cache_root_path() {
        cache_dirty |= sync_repo_assets(
            repo_url,
            &remote_repo,
            &mut cache,
            cache_root,
            downloads,
            sink.clone(),
        )
        .await;
    }

    // Step C: build remote mod checksum view
    let mut remote_mods: BTreeMap<String, swifty_artifacts::Md5Digest> = BTreeMap::new();
    for m in remote_repo
        .required_mods
        .iter()
        .chain(remote_repo.optional_mods.iter())
    {
        let key = m.mod_name.to_ascii_lowercase();
        remote_mods.insert(key, m.checksum);
    }
    debug!(
        repo_url,
        required = remote_repo.required_mods.len(),
        optional = remote_repo.optional_mods.len(),
        total = remote_mods.len(),
        "swifty repo mod counts"
    );

    // Step D: plan which mods to fetch
    let mut mods_to_fetch: Vec<String> = Vec::new();
    let mut reused_mods: Vec<String> = Vec::new();
    for (k, checksum) in &remote_mods {
        match cache.mods.get(k) {
            None => mods_to_fetch.push(k.clone()),
            Some(existing) => {
                if existing.checksum != *checksum {
                    mods_to_fetch.push(k.clone())
                } else {
                    reused_mods.push(k.clone())
                }
            }
        }
    }

    // Optionally prune cached mods not in remote anymore
    let stale_keys: Vec<String> = cache
        .mods
        .keys()
        .filter(|k| !remote_mods.contains_key(*k))
        .cloned()
        .collect();
    if !stale_keys.is_empty() {
        cache_dirty = true;
    }
    for k in stale_keys {
        cache.mods.remove(&k);
    }

    let mut fetched_mods: Vec<String> = Vec::new();
    let mut output_mods: BTreeMap<String, swifty_artifacts::SrfMod> = BTreeMap::new();

    // Fill output_mods with reused ones
    for k in &reused_mods {
        if let Some(existing) = cache.mods.get(k) {
            output_mods.insert(k.clone(), existing.manifest.clone());
        }
    }
    debug!(
        repo_url,
        reuse_count = reused_mods.len(),
        fetch_count = mods_to_fetch.len(),
        "swifty repo mod fetch plan"
    );

    // Step E: download needed mods concurrently (bounded by DownloadServiceConfig.parallel_requests)
    if !mods_to_fetch.is_empty() {
        cache_dirty = true;
        let tmp = tempfile::tempdir().context("create tempdir for mod.srf batch")?;

        let mut specs = Vec::with_capacity(mods_to_fetch.len());
        for k in &mods_to_fetch {
            let url = resolver
                .mod_srf_url(repo_url, k)
                .with_context(|| format!("resolve URL for mod {k}"))?;

            specs.push(DownloadSpec {
                id: format!("mod:{k}"),
                url,
                file_name: std::path::PathBuf::from(format!("{k}.srf")),
            });
        }

        let outcomes = downloads
            .download_many_to_folder(tmp.path(), specs, sink.clone())
            .await
            .context("download mod.srf batch")?;

        for out in outcomes {
            if out.status != 200 {
                anyhow::bail!("unexpected status {} fetching {}", out.status, out.url);
            }

            let bytes = tokio::fs::read(&out.path)
                .await
                .with_context(|| format!("read {}", out.path.display()))?;

            let parsed = swifty_artifacts::read_mod_srf(&bytes)
                .with_context(|| format!("parse mod.srf {}", out.path.display()))?;
            debug!(
                mod_name = parsed.name.as_str(),
                mod_checksum = ?parsed.checksum,
                bytes = bytes.len(),
                "parsed mod.srf"
            );

            let k = parsed.name.to_ascii_lowercase();
            let expected_checksum = remote_mods.get(&k).context("missing remote checksum")?;
            if &parsed.checksum != expected_checksum {
                anyhow::bail!("checksum mismatch for mod {k}");
            }

            let now = fleet_domain::time::now_unix_ms();
            cache.mods.insert(
                k.clone(),
                CachedModSrf {
                    checksum: *expected_checksum,
                    fetched_at_unix_ms: now,
                    manifest: parsed.clone(),
                    http: None,
                },
            );

            output_mods.insert(k.clone(), parsed);
            fetched_mods.push(k);
        }
    }

    // Step F: update repo in cache and persist
    if cache_dirty {
        cache.repo = remote_repo.clone();
        store
            .save_repo_cache(&cache.repo_url, &cache)
            .await
            .context("save cache")?;
    }

    Ok(RepoSyncResult {
        repo: remote_repo,
        mods: output_mods,
        fetched_mods,
        reused_mods,
        freshness,
    })
}

pub async fn probe_repo_freshness(
    repo_url: &str,
    store: &dyn RepoCacheStore,
    downloads: &DownloadService,
    sink: Option<DownloadEventSink>,
) -> Result<RepoProbeResult> {
    let cache_opt = store.load_repo_cache(repo_url).await?;
    let local_revision = cache_opt.as_ref().and_then(repo_blob_revision);
    let repo_headers =
        build_conditional_headers(cache_opt.as_ref().and_then(|c| c.repo_http.as_ref()))?;
    let repo_id = repo_manifest_id(repo_url);
    let repo_fetch = fetch_repo_json(
        downloads,
        repo_id.as_str(),
        repo_url,
        repo_headers,
        sink,
        repo_id.as_str(),
        "probe repo manifest",
    )
    .await?;

    match (cache_opt, repo_fetch) {
        (Some(cache), RepoJsonFetch::NotModified) => Ok(RepoProbeResult {
            local_revision,
            remote_revision: cache.repo_json_checksum.or_else(|| {
                let checksum = cache.repo.checksum.trim();
                (!checksum.is_empty()).then(|| checksum.to_string())
            }),
            freshness: RepoFreshness::UpToDate,
        }),
        (cache_opt, RepoJsonFetch::Downloaded { bytes, .. }) => {
            let remote_revision = Some(fleet_domain::hash::sha1_hex(&bytes));
            let freshness = match cache_opt.as_ref().and_then(repo_blob_revision).as_deref() {
                Some(local) if Some(local.to_string()) == remote_revision => {
                    RepoFreshness::UpToDate
                }
                Some(_) => RepoFreshness::UpdateAvailable,
                None => RepoFreshness::Unknown,
            };
            Ok(RepoProbeResult {
                local_revision,
                remote_revision,
                freshness,
            })
        }
        (None, RepoJsonFetch::NotModified) => {
            anyhow::bail!("received 304 for repo.json but no cache exists")
        }
    }
}

async fn sync_repo_assets(
    repo_url: &str,
    repo: &swifty_artifacts::RepoSpec,
    cache: &mut RepoCacheBlob,
    cache_root: &Path,
    downloads: &DownloadService,
    sink: Option<DownloadEventSink>,
) -> bool {
    let Ok(base_url) = url::Url::parse(repo_url).and_then(|u| u.join("./")) else {
        return false;
    };
    let mut cache_dirty = false;

    let assets = [
        (
            "icon.png",
            repo.icon_image_path.as_deref(),
            repo.icon_image_checksum.as_deref(),
            &mut cache.icon_image_checksum,
        ),
        (
            "repo.png",
            repo.repo_image_path.as_deref(),
            repo.repo_image_checksum.as_deref(),
            &mut cache.repo_image_checksum,
        ),
    ];

    for (role_name, path_opt, checksum_opt, cached_checksum) in assets {
        let Some(path) = path_opt.map(str::trim).filter(|p| !p.is_empty()) else {
            continue;
        };
        let Ok(url) = base_url.join(path) else {
            continue;
        };
        let dest_path = repo_cache_asset_path(cache_root, repo_url, role_name);
        if !should_download_asset(checksum_opt, cached_checksum.as_deref(), &dest_path) {
            continue;
        }
        let Some(file_name) = dest_path.file_name().map(PathBuf::from) else {
            continue;
        };
        match downloads
            .download_one_to_file(
                format!("repo-asset:{role_name}"),
                url.as_str(),
                cache_root,
                &file_name,
                None,
                sink.clone(),
            )
            .await
        {
            Ok(_) => {
                if let Some(checksum) = checksum_opt {
                    if cached_checksum.as_deref() != Some(checksum) {
                        *cached_checksum = Some(checksum.to_string());
                        cache_dirty = true;
                    }
                }
            }
            Err(err) => {
                debug!(
                    asset = role_name,
                    url = url.as_str(),
                    error = %err,
                    "failed to download swifty repo asset"
                );
            }
        }
    }
    cache_dirty
}

fn should_download_asset(
    repo_checksum: Option<&str>,
    cached_checksum: Option<&str>,
    dest_path: &Path,
) -> bool {
    if !dest_path.exists() {
        return true;
    }
    match (repo_checksum, cached_checksum) {
        (Some(repo), Some(cached)) => repo != cached,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

fn repo_manifest_id(repo_url: &str) -> String {
    let Ok(u) = url::Url::parse(repo_url) else {
        return "swifty.repo".to_string();
    };

    let Some(seg) = u
        .path_segments()
        .and_then(|mut s| s.next_back())
        .filter(|s| !s.is_empty())
    else {
        return "swifty.repo".to_string();
    };

    seg.to_string()
}

pub(crate) fn build_conditional_headers(
    hints: Option<&HttpCacheHints>,
) -> Result<Option<HeaderMap>> {
    let (if_none_match, if_modified_since) = hints
        .map(|h| (h.etag.as_deref(), h.last_modified.as_deref()))
        .unwrap_or((None, None));

    let mut cond = HeaderMap::new();
    if let Some(v) = if_none_match {
        cond.insert(
            IF_NONE_MATCH,
            HeaderValue::from_str(v).context("bad If-None-Match header value")?,
        );
    }
    if let Some(v) = if_modified_since {
        cond.insert(
            IF_MODIFIED_SINCE,
            HeaderValue::from_str(v).context("bad If-Modified-Since header value")?,
        );
    }
    Ok((!cond.is_empty()).then_some(cond))
}

pub(crate) enum RepoJsonFetch {
    NotModified,
    Downloaded {
        etag: Option<String>,
        last_modified: Option<String>,
        bytes: Vec<u8>,
    },
}

pub(crate) async fn fetch_repo_json(
    downloads: &DownloadService,
    download_id: &str,
    repo_url: &str,
    headers: Option<HeaderMap>,
    sink: Option<DownloadEventSink>,
    temp_file_name: &str,
    op_name: &str,
) -> Result<RepoJsonFetch> {
    let tmp = tempfile::tempdir().with_context(|| format!("create tempdir for {op_name}"))?;
    let result = downloads
        .download_one_to_file(
            download_id,
            repo_url,
            tmp.path(),
            Path::new(temp_file_name),
            headers,
            sink,
        )
        .await
        .with_context(|| format!("{op_name} {repo_url}"))?;

    match result {
        DownloadResult::NotModified { .. } => Ok(RepoJsonFetch::NotModified),
        DownloadResult::Downloaded(outcome) => {
            let bytes = tokio::fs::read(&outcome.path)
                .await
                .with_context(|| format!("read {}", outcome.path.display()))?;
            Ok(RepoJsonFetch::Downloaded {
                etag: outcome.etag,
                last_modified: outcome.last_modified,
                bytes,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use axum::extract::State;
    use axum::http::header::{CONTENT_TYPE, ETAG, IF_NONE_MATCH, LAST_MODIFIED};
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
    use axum::response::Response;
    use axum::routing::get;
    use axum::Router;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    const LAST_MODIFIED_VALUE: &str = "Fri, 23 Jan 2026 22:12:11 GMT";

    #[derive(Clone, Default)]
    struct MockStore {
        inner: Arc<Mutex<MockStoreState>>,
    }

    #[derive(Default)]
    struct MockStoreState {
        cache: Option<RepoCacheBlob>,
        save_calls: usize,
    }

    impl MockStore {
        fn with_cache(cache: RepoCacheBlob) -> Self {
            Self {
                inner: Arc::new(Mutex::new(MockStoreState {
                    cache: Some(cache),
                    save_calls: 0,
                })),
            }
        }

        fn empty() -> Self {
            Self {
                inner: Arc::new(Mutex::new(MockStoreState::default())),
            }
        }

        fn save_calls(&self) -> usize {
            self.inner.lock().expect("lock store").save_calls
        }
    }

    #[async_trait]
    impl RepoCacheStore for MockStore {
        async fn load_repo_cache(&self, _repo_url: &str) -> Result<Option<RepoCacheBlob>> {
            Ok(self.inner.lock().expect("lock store").cache.clone())
        }

        async fn save_repo_cache(&self, _repo_url: &str, blob: &RepoCacheBlob) -> Result<()> {
            let mut state = self.inner.lock().expect("lock store");
            state.cache = Some(blob.clone());
            state.save_calls += 1;
            Ok(())
        }

        async fn delete_repo_cache(&self, _repo_url: &str) -> Result<()> {
            self.inner.lock().expect("lock store").cache = None;
            Ok(())
        }
    }

    #[derive(Clone)]
    struct RepoServerState {
        body: Arc<String>,
        etag: String,
        conditional_304: bool,
    }

    fn md5(hex: &str) -> swifty_artifacts::Md5Digest {
        swifty_artifacts::Md5Digest::parse_hex(hex).expect("valid md5")
    }

    fn empty_repo_spec(repo_checksum: &str) -> swifty_artifacts::RepoSpec {
        swifty_artifacts::RepoSpec {
            repo_name: "test-pack".to_string(),
            checksum: repo_checksum.to_string(),
            required_mods: vec![],
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

    fn repo_with_required_mod(
        repo_checksum: &str,
        mod_name: &str,
        mod_checksum: &str,
    ) -> swifty_artifacts::RepoSpec {
        swifty_artifacts::RepoSpec {
            repo_name: "test-pack".to_string(),
            checksum: repo_checksum.to_string(),
            required_mods: vec![swifty_artifacts::RepoMod {
                mod_name: mod_name.to_string(),
                checksum: md5(mod_checksum),
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

    fn empty_mod_manifest(mod_name: &str, mod_checksum: &str) -> swifty_artifacts::SrfMod {
        swifty_artifacts::SrfMod {
            name: mod_name.to_string(),
            checksum: md5(mod_checksum),
            files: vec![],
        }
    }

    fn make_cache_blob(
        repo_url: &str,
        repo: swifty_artifacts::RepoSpec,
        repo_json_checksum: Option<String>,
        etag: Option<&str>,
        mods: BTreeMap<String, CachedModSrf>,
    ) -> RepoCacheBlob {
        RepoCacheBlob {
            schema_version: 1,
            repo_url: repo_url.to_string(),
            repo_fetched_at_unix_ms: 1,
            repo,
            mods,
            repo_http: etag.map(|value| HttpCacheHints {
                etag: Some(value.to_string()),
                last_modified: Some(LAST_MODIFIED_VALUE.to_string()),
            }),
            icon_image_checksum: None,
            repo_image_checksum: None,
            repo_json_checksum,
        }
    }

    async fn repo_json_handler(
        State(state): State<RepoServerState>,
        headers: HeaderMap,
    ) -> Response {
        let conditional_match = headers
            .get(IF_NONE_MATCH)
            .and_then(|h| h.to_str().ok())
            .is_some_and(|value| value == state.etag.as_str());

        if state.conditional_304 && conditional_match {
            return response_with_headers(StatusCode::NOT_MODIFIED, String::new(), &state.etag);
        }

        response_with_headers(StatusCode::OK, state.body.as_str().to_string(), &state.etag)
    }

    fn response_with_headers(status: StatusCode, body: String, etag: &str) -> Response {
        let mut response = Response::new(axum::body::Body::from(body));
        *response.status_mut() = status;
        let headers = response.headers_mut();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(ETAG, HeaderValue::from_str(etag).expect("valid etag"));
        headers.insert(LAST_MODIFIED, HeaderValue::from_static(LAST_MODIFIED_VALUE));
        response
    }

    async fn spawn_repo_server(
        repo: &swifty_artifacts::RepoSpec,
        etag: &str,
        conditional_304: bool,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let body = serde_json::to_string(repo).expect("serialize repo");
        let state = RepoServerState {
            body: Arc::new(body),
            etag: etag.to_string(),
            conditional_304,
        };
        let app = Router::new()
            .route("/repo.json", get(repo_json_handler))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        (format!("http://{addr}/repo.json"), task)
    }

    #[tokio::test]
    async fn sync_repo_metadata_returns_up_to_date_and_skips_save_on_304() {
        let repo = empty_repo_spec("0000000000000000000000000000000000000000");
        let (repo_url, server) = spawn_repo_server(&repo, "\"etag-v1\"", true).await;
        let checksum = fleet_domain::hash::sha1_hex(
            serde_json::to_string(&repo).expect("serialize").as_bytes(),
        );
        let store = MockStore::with_cache(make_cache_blob(
            &repo_url,
            repo,
            Some(checksum),
            Some("\"etag-v1\""),
            BTreeMap::new(),
        ));

        let result = sync_repo_metadata(
            &repo_url,
            &store,
            &DefaultModSrfResolver,
            &DownloadService::new_default(),
            None,
        )
        .await
        .expect("sync metadata");

        assert_eq!(result.freshness, RepoFreshness::UpToDate);
        assert!(result.fetched_mods.is_empty());
        assert!(result.reused_mods.is_empty());
        assert_eq!(store.save_calls(), 0);
        server.abort();
    }

    #[tokio::test]
    async fn sync_repo_metadata_returns_update_available_when_repo_checksum_changes() {
        let old_repo = empty_repo_spec("0000000000000000000000000000000000000000");
        let new_repo = empty_repo_spec("1111111111111111111111111111111111111111");
        let (repo_url, server) = spawn_repo_server(&new_repo, "\"etag-v2\"", false).await;
        let old_checksum = fleet_domain::hash::sha1_hex(
            serde_json::to_string(&old_repo)
                .expect("serialize old repo")
                .as_bytes(),
        );
        let store = MockStore::with_cache(make_cache_blob(
            &repo_url,
            old_repo,
            Some(old_checksum),
            None,
            BTreeMap::new(),
        ));

        let result = sync_repo_metadata(
            &repo_url,
            &store,
            &DefaultModSrfResolver,
            &DownloadService::new_default(),
            None,
        )
        .await
        .expect("sync metadata");

        assert_eq!(result.freshness, RepoFreshness::UpdateAvailable);
        assert_eq!(store.save_calls(), 1);
        server.abort();
    }

    #[tokio::test]
    async fn sync_repo_metadata_returns_unknown_without_prior_repo_checksum() {
        let repo = empty_repo_spec("0000000000000000000000000000000000000000");
        let (repo_url, server) = spawn_repo_server(&repo, "\"etag-v1\"", false).await;
        let store = MockStore::with_cache(make_cache_blob(
            &repo_url,
            repo,
            None,
            None,
            BTreeMap::new(),
        ));

        let result = sync_repo_metadata(
            &repo_url,
            &store,
            &DefaultModSrfResolver,
            &DownloadService::new_default(),
            None,
        )
        .await
        .expect("sync metadata");

        assert_eq!(result.freshness, RepoFreshness::Unknown);
        assert_eq!(store.save_calls(), 1);
        server.abort();
    }

    #[tokio::test]
    async fn sync_repo_metadata_reuses_cached_mods_when_checksums_match() {
        let mod_name = "ace";
        let mod_checksum = "00000000000000000000000000000000";
        let repo = repo_with_required_mod(
            "0000000000000000000000000000000000000000",
            mod_name,
            mod_checksum,
        );
        let (repo_url, server) = spawn_repo_server(&repo, "\"etag-v1\"", true).await;
        let checksum = fleet_domain::hash::sha1_hex(
            serde_json::to_string(&repo).expect("serialize").as_bytes(),
        );
        let cache_mod = CachedModSrf {
            checksum: md5(mod_checksum),
            fetched_at_unix_ms: 1,
            manifest: empty_mod_manifest(mod_name, mod_checksum),
            http: None,
        };
        let store = MockStore::with_cache(make_cache_blob(
            &repo_url,
            repo,
            Some(checksum),
            Some("\"etag-v1\""),
            BTreeMap::from([(mod_name.to_string(), cache_mod)]),
        ));

        let result = sync_repo_metadata(
            &repo_url,
            &store,
            &DefaultModSrfResolver,
            &DownloadService::new_default(),
            None,
        )
        .await
        .expect("sync metadata");

        assert_eq!(result.freshness, RepoFreshness::UpToDate);
        assert!(result.fetched_mods.is_empty());
        assert_eq!(result.reused_mods, vec![mod_name.to_string()]);
        assert_eq!(store.save_calls(), 0);
        server.abort();
    }

    #[tokio::test]
    async fn sync_repo_metadata_returns_unknown_for_initial_cache_creation() {
        let repo = empty_repo_spec("0000000000000000000000000000000000000000");
        let (repo_url, server) = spawn_repo_server(&repo, "\"etag-v1\"", false).await;
        let store = MockStore::empty();

        let result = sync_repo_metadata(
            &repo_url,
            &store,
            &DefaultModSrfResolver,
            &DownloadService::new_default(),
            None,
        )
        .await
        .expect("sync metadata");

        assert_eq!(result.freshness, RepoFreshness::Unknown);
        assert_eq!(store.save_calls(), 1);
        server.abort();
    }

    #[tokio::test]
    async fn probe_repo_freshness_is_read_only_for_not_modified_repo() {
        let repo = empty_repo_spec("0000000000000000000000000000000000000000");
        let (repo_url, server) = spawn_repo_server(&repo, "\"etag-v1\"", true).await;
        let checksum = fleet_domain::hash::sha1_hex(
            serde_json::to_string(&repo).expect("serialize").as_bytes(),
        );
        let store = MockStore::with_cache(make_cache_blob(
            &repo_url,
            repo,
            Some(checksum.clone()),
            Some("\"etag-v1\""),
            BTreeMap::new(),
        ));

        let result = probe_repo_freshness(&repo_url, &store, &DownloadService::new_default(), None)
            .await
            .expect("probe freshness");

        assert_eq!(result.local_revision.as_deref(), Some(checksum.as_str()));
        assert_eq!(result.remote_revision.as_deref(), Some(checksum.as_str()));
        assert_eq!(result.freshness, RepoFreshness::UpToDate);
        assert_eq!(store.save_calls(), 0);
        server.abort();
    }

    #[tokio::test]
    async fn probe_repo_freshness_detects_update_without_writing_cache() {
        let old_repo = empty_repo_spec("0000000000000000000000000000000000000000");
        let new_repo = empty_repo_spec("1111111111111111111111111111111111111111");
        let (repo_url, server) = spawn_repo_server(&new_repo, "\"etag-v2\"", false).await;
        let old_checksum = fleet_domain::hash::sha1_hex(
            serde_json::to_string(&old_repo)
                .expect("serialize old repo")
                .as_bytes(),
        );
        let new_checksum = fleet_domain::hash::sha1_hex(
            serde_json::to_string(&new_repo)
                .expect("serialize new repo")
                .as_bytes(),
        );
        let store = MockStore::with_cache(make_cache_blob(
            &repo_url,
            old_repo,
            Some(old_checksum.clone()),
            None,
            BTreeMap::new(),
        ));

        let result = probe_repo_freshness(&repo_url, &store, &DownloadService::new_default(), None)
            .await
            .expect("probe freshness");

        assert_eq!(
            result.local_revision.as_deref(),
            Some(old_checksum.as_str())
        );
        assert_eq!(
            result.remote_revision.as_deref(),
            Some(new_checksum.as_str())
        );
        assert_eq!(result.freshness, RepoFreshness::UpdateAvailable);
        assert_eq!(store.save_calls(), 0);
        server.abort();
    }
}
