use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, IF_MODIFIED_SINCE, IF_NONE_MATCH};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracing::debug;

use fleet_download::{
    atomic_replace_file, DownloadEventSink, DownloadResult, DownloadService, DownloadSpec,
};

use sha1::{Digest, Sha1};
pub mod touch;

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

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    let mut hasher = Sha1::new();
    hasher.update(repo_url.as_bytes());
    let digest = hasher.finalize();
    format!("{:x}", digest)
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
        let tmp = p.with_extension("json.tmp");
        let s = serde_json::to_vec_pretty(&blob).context("serialize cache blob")?;
        tokio::fs::write(&tmp, s)
            .await
            .with_context(|| format!("write tmp cache {}", tmp.display()))?;
        atomic_replace_file(&tmp, &p).await?;
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
    let now_ms = now_unix_ms()?;
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
    let (mut cache, remote_repo) = match (cache_opt, repo_fetch) {
        (Some(cache), RepoJsonFetch::NotModified { .. }) => {
            if cache.repo_fetched_at_unix_ms == 0 {
                anyhow::bail!("received 304 for repo.json but cached repo is empty");
            }
            debug!(repo_url, "repo.json not modified; using cache");
            let repo = cache.repo.clone();
            (cache, repo)
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
            cache.repo_json_checksum = Some(sha1_hex(&bytes));

            // Update cache hints from GET response (fallback to prior values if absent)
            let prior = cache.repo_http.take();
            cache.repo_http = Some(HttpCacheHints {
                etag: etag.or_else(|| prior.as_ref().and_then(|p| p.etag.clone())),
                last_modified: last_modified
                    .or_else(|| prior.as_ref().and_then(|p| p.last_modified.clone())),
            });
            cache.repo_fetched_at_unix_ms = now_ms;

            (cache, repo)
        }
        (None, RepoJsonFetch::NotModified { .. }) => {
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
                repo_json_checksum: Some(sha1_hex(&bytes)),
            };

            (cache, repo)
        }
    };

    if let Some(cache_root) = store.cache_root_path() {
        sync_repo_assets(
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

            let now = now_unix_ms()?;
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
    cache.repo = remote_repo.clone();
    store
        .save_repo_cache(&cache.repo_url, &cache)
        .await
        .context("save cache")?;

    Ok(RepoSyncResult {
        repo: remote_repo,
        mods: output_mods,
        fetched_mods,
        reused_mods,
    })
}

async fn sync_repo_assets(
    repo_url: &str,
    repo: &swifty_artifacts::RepoSpec,
    cache: &mut RepoCacheBlob,
    cache_root: &Path,
    downloads: &DownloadService,
    sink: Option<DownloadEventSink>,
) {
    let Ok(base_url) = url::Url::parse(repo_url).and_then(|u| u.join("./")) else {
        return;
    };

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
                    *cached_checksum = Some(checksum.to_string());
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

pub(crate) fn sha1_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    format!("{:x}", digest)
}

pub(crate) fn now_unix_ms() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock must be >= unix epoch")?
        .as_millis() as u64)
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
    NotModified {
        etag: Option<String>,
        last_modified: Option<String>,
    },
    Downloaded {
        status: u16,
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
        DownloadResult::NotModified {
            etag,
            last_modified,
        } => Ok(RepoJsonFetch::NotModified {
            etag,
            last_modified,
        }),
        DownloadResult::Downloaded(outcome) => {
            let bytes = tokio::fs::read(&outcome.path)
                .await
                .with_context(|| format!("read {}", outcome.path.display()))?;
            Ok(RepoJsonFetch::Downloaded {
                status: outcome.status,
                etag: outcome.etag,
                last_modified: outcome.last_modified,
                bytes,
            })
        }
    }
}
