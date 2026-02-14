use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use fleet_download::{DownloadEventSink, DownloadService};

use crate::{
    build_conditional_headers, fetch_repo_json, now_unix_ms, sha1_hex, HttpCacheHints,
    RepoCacheStore, RepoJsonFetch,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepoTouchReport {
    pub repo_url: String,
    pub status: RepoTouchStatus,
    pub checked_at_unix_ms: u64,

    // Optional details for UI/telemetry/debug
    pub http_status: Option<u16>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum RepoTouchStatus {
    UpToDate,
    UpdateAvailable,
    NoCache,
}

#[derive(Clone, Debug)]
pub struct RepoTouchOptions<'a> {
    pub temp_file_name: &'a str,
}

impl<'a> Default for RepoTouchOptions<'a> {
    fn default() -> Self {
        Self {
            temp_file_name: "repo.json",
        }
    }
}

pub async fn touch_repo_json(
    repo_url: &str,
    store: &dyn RepoCacheStore,
    downloads: &DownloadService,
    sink: Option<DownloadEventSink>,
    opts: RepoTouchOptions<'_>,
) -> Result<RepoTouchReport> {
    let now_ms = now_unix_ms()?;

    let cache_opt = store
        .load_repo_cache(repo_url)
        .await
        .with_context(|| format!("load swifty cache for {repo_url}"))?;

    let Some(mut cache) = cache_opt else {
        return Ok(RepoTouchReport {
            repo_url: repo_url.to_string(),
            status: RepoTouchStatus::NoCache,
            checked_at_unix_ms: now_ms,
            http_status: None,
            etag: None,
            last_modified: None,
        });
    };

    let repo_headers = build_conditional_headers(cache.repo_http.as_ref())?;
    let result = fetch_repo_json(
        downloads,
        "swifty.repo.touch",
        repo_url,
        repo_headers,
        sink,
        opts.temp_file_name,
        "download repo.json touch",
    )
    .await?;

    match result {
        RepoJsonFetch::NotModified {
            etag,
            last_modified,
        } => {
            cache.repo_fetched_at_unix_ms = now_ms;
            cache.repo_http = Some(HttpCacheHints {
                etag,
                last_modified,
            });

            store
                .save_repo_cache(repo_url, &cache)
                .await
                .context("save cache after 304 touch")?;

            Ok(RepoTouchReport {
                repo_url: repo_url.to_string(),
                status: RepoTouchStatus::UpToDate,
                checked_at_unix_ms: now_ms,
                http_status: Some(304),
                etag: cache.repo_http.as_ref().and_then(|h| h.etag.clone()),
                last_modified: cache
                    .repo_http
                    .as_ref()
                    .and_then(|h| h.last_modified.clone()),
            })
        }

        RepoJsonFetch::Downloaded {
            status,
            etag,
            last_modified,
            bytes,
        } => {
            let remote_checksum = sha1_hex(&bytes);
            let cached_checksum = cache.repo_json_checksum.clone();

            if cached_checksum.as_deref() == Some(remote_checksum.as_str()) {
                cache.repo_fetched_at_unix_ms = now_ms;
                cache.repo_json_checksum = Some(remote_checksum);
                let prior = cache.repo_http.take();
                cache.repo_http = Some(HttpCacheHints {
                    etag: etag.or_else(|| prior.as_ref().and_then(|p| p.etag.clone())),
                    last_modified: last_modified
                        .or_else(|| prior.as_ref().and_then(|p| p.last_modified.clone())),
                });

                store
                    .save_repo_cache(repo_url, &cache)
                    .await
                    .context("save cache after 200 touch")?;

                Ok(RepoTouchReport {
                    repo_url: repo_url.to_string(),
                    status: RepoTouchStatus::UpToDate,
                    checked_at_unix_ms: now_ms,
                    http_status: Some(status),
                    etag: cache.repo_http.as_ref().and_then(|h| h.etag.clone()),
                    last_modified: cache
                        .repo_http
                        .as_ref()
                        .and_then(|h| h.last_modified.clone()),
                })
            } else {
                Ok(RepoTouchReport {
                    repo_url: repo_url.to_string(),
                    status: RepoTouchStatus::UpdateAvailable,
                    checked_at_unix_ms: now_ms,
                    http_status: Some(status),
                    etag,
                    last_modified,
                })
            }
        }
    }
}
