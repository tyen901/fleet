//! Fast metadata download service for Fleet.
//!
//! Purpose-built for small blobs:
//! - repo.json
//! - mod.srf
//!
//! Design goals:
//! - One shared reqwest::Client (connection pooling)
//! - GET-only (no HEAD preflights)
//! - Bounded concurrency across files (not per-file range parallelism)
//! - Short, sane timeouts (avoid multi-minute “hangs”)
//! - Minimal event emission (Started/Finished/Failed)

use anyhow::{anyhow, Context, Result};
use fleet_domain::{DownloadEvent, DownloadPhase};
use reqwest::header::{HeaderMap, HeaderValue, ETAG, LAST_MODIFIED, USER_AGENT};
use reqwest::StatusCode;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tracing::{debug, error, warn};

pub type DownloadEventSink = Arc<dyn Fn(DownloadEvent) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct DownloadServiceConfig {
    pub user_agent: String,

    /// TCP connect timeout.
    pub connect_timeout: Duration,

    /// Total request timeout (send + body).
    pub timeout: Duration,

    /// Retry count for transient failures.
    pub retries: u16,

    /// Max number of concurrent downloads in download_many_to_folder().
    pub parallel_requests: u16,
}

impl Default for DownloadServiceConfig {
    fn default() -> Self {
        Self {
            user_agent: "fleet/0.x (metadata)".to_string(),
            connect_timeout: Duration::from_secs(5),
            timeout: Duration::from_secs(20),
            retries: 2,
            parallel_requests: 8,
        }
    }
}

#[derive(Clone)]
pub struct DownloadService {
    cfg: DownloadServiceConfig,
    client: reqwest::Client,
}

impl DownloadService {
    pub fn new(cfg: DownloadServiceConfig) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&cfg.user_agent).expect("invalid user-agent"),
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(cfg.connect_timeout)
            .timeout(cfg.timeout)
            .pool_max_idle_per_host(16)
            .build()
            .expect("build reqwest client");

        Self { cfg, client }
    }

    pub fn new_default() -> Self {
        Self::new(DownloadServiceConfig::default())
    }

    pub async fn download_one_to_file(
        &self,
        id: impl Into<String>,
        url: &str,
        download_folder: &Path,
        file_name: &Path,
        extra_headers: Option<HeaderMap>,
        sink: Option<DownloadEventSink>,
    ) -> Result<DownloadResult> {
        tokio::fs::create_dir_all(download_folder)
            .await
            .with_context(|| format!("create download dir {}", download_folder.display()))?;

        let id = id.into();
        let started_at = Instant::now();
        emit_started(&sink, &id, url);

        let out_path = download_folder.join(file_name);
        if let Some(parent) = out_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create parent dir {}", parent.display()))?;
        }
        let tmp_path = out_path.with_extension("tmp");

        let mut attempt: u16 = 0;
        let mut last_err: Option<anyhow::Error> = None;
        let progress_emit_interval = Duration::from_millis(100);

        while attempt <= self.cfg.retries {
            if attempt > 0 {
                let backoff = backoff_delay(attempt);
                warn!(
                    id = id.as_str(),
                    url = url,
                    attempt = attempt,
                    backoff_ms = backoff.as_millis(),
                    "retrying download"
                );
                tokio::time::sleep(backoff).await;
            }

            let req = self.client.get(url);
            let req = if let Some(h) = extra_headers.clone() {
                req.headers(h)
            } else {
                req
            };

            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    if is_retryable_reqwest_error(&e) && attempt < self.cfg.retries {
                        last_err = Some(anyhow::Error::new(e).context("reqwest send"));
                        attempt = attempt.saturating_add(1);
                        continue;
                    }

                    let err = anyhow::Error::new(e).context("reqwest send");
                    emit_failed(&sink, &id, url, &err);
                    return Err(err);
                }
            };

            let status = resp.status();
            let etag = header_str(resp.headers(), ETAG);
            let last_modified = header_str(resp.headers(), LAST_MODIFIED);
            let content_len = resp.content_length();

            if status == StatusCode::NOT_MODIFIED {
                emit_finished(&sink, &id, url, started_at, 0, content_len);
                return Ok(DownloadResult::NotModified {
                    etag,
                    last_modified,
                });
            }

            if status != StatusCode::OK {
                if is_retryable_status(status) && attempt < self.cfg.retries {
                    last_err = Some(anyhow!("HTTP {status}").context(format!("GET {url}")));
                    attempt = attempt.saturating_add(1);
                    continue;
                }

                let err = anyhow!("unexpected status {status} for GET {url}");
                emit_failed(&sink, &id, url, &err);
                return Err(err);
            }

            let mut last_progress_at = Instant::now();
            let mut emitted_progress = false;
            let bytes_written = match write_response_to_file(resp, &tmp_path, |written| {
                if sink.is_none() {
                    return;
                }
                let now = Instant::now();
                let should_emit = !emitted_progress
                    || now.duration_since(last_progress_at) >= progress_emit_interval
                    || content_len.is_some_and(|total| written >= total);
                if should_emit {
                    emitted_progress = true;
                    last_progress_at = now;
                    emit_progress(&sink, &id, url, written, content_len);
                }
            })
            .await
            {
                Ok(n) => n,
                Err(e) => {
                    let _ = tokio::fs::remove_file(&tmp_path).await;

                    if attempt < self.cfg.retries {
                        last_err = Some(e);
                        attempt = attempt.saturating_add(1);
                        continue;
                    }

                    emit_failed(&sink, &id, url, &e);
                    return Err(e);
                }
            };

            crate::atomic_replace_file(&tmp_path, &out_path).await?;

            emit_finished(&sink, &id, url, started_at, bytes_written, content_len);

            return Ok(DownloadResult::Downloaded(DownloadOutcome {
                url: url.to_string(),
                status: status.as_u16(),
                path: out_path,
                bytes_written,
                etag,
                last_modified,
            }));
        }

        let err = last_err.unwrap_or_else(|| anyhow!("download failed: {url}"));
        emit_failed(&sink, &id, url, &err);
        Err(err)
    }

    pub async fn download_many_to_folder(
        &self,
        download_folder: &Path,
        specs: Vec<DownloadSpec>,
        sink: Option<DownloadEventSink>,
    ) -> Result<Vec<DownloadOutcome>> {
        tokio::fs::create_dir_all(download_folder)
            .await
            .with_context(|| format!("create download dir {}", download_folder.display()))?;

        let max_in_flight = std::cmp::max(1u16, self.cfg.parallel_requests) as usize;
        let files_total = specs.len() as u64;
        let files_completed = Arc::new(AtomicU64::new(0));

        use futures_util::stream::{self, StreamExt};

        let this = self.clone();
        let folder = download_folder.to_path_buf();

        let mut outcomes: Vec<DownloadOutcome> = Vec::with_capacity(specs.len());

        let mut stream = stream::iter(specs.into_iter().map(|spec| {
            let this = this.clone();
            let folder = folder.clone();
            let sink = sink.clone().map(|base_sink| {
                let completed_counter = Arc::clone(&files_completed);
                Arc::new(move |mut ev: DownloadEvent| {
                    ev.files_total = Some(files_total);
                    let completed_now = match ev.phase {
                        DownloadPhase::Finished | DownloadPhase::Failed => completed_counter
                            .fetch_add(1, Ordering::Relaxed)
                            .saturating_add(1),
                        _ => completed_counter.load(Ordering::Relaxed),
                    };
                    ev.files_completed = Some(completed_now.min(files_total));
                    base_sink(ev);
                }) as DownloadEventSink
            });
            async move {
                this.download_one_to_file(spec.id, &spec.url, &folder, &spec.file_name, None, sink)
                    .await
                    .with_context(|| format!("download failed: {}", spec.url))
            }
        }))
        .buffer_unordered(max_in_flight);

        while let Some(res) = stream.next().await {
            match res {
                Ok(result) => match result {
                    DownloadResult::Downloaded(out) => outcomes.push(out),
                    DownloadResult::NotModified { .. } => {
                        return Err(anyhow!("unexpected 304 for {}", "download_many_to_folder"))
                    }
                },
                Err(e) => return Err(e),
            }
        }

        Ok(outcomes)
    }
}

#[derive(Debug, Clone)]
pub struct DownloadSpec {
    pub id: String,
    pub url: String,
    /// Relative to download_folder (recommended).
    pub file_name: PathBuf,
}

#[derive(Debug, Clone)]
pub enum DownloadResult {
    NotModified {
        etag: Option<String>,
        last_modified: Option<String>,
    },
    Downloaded(DownloadOutcome),
}

#[derive(Debug, Clone)]
pub struct DownloadOutcome {
    pub url: String,
    pub status: u16,
    pub path: PathBuf,
    pub bytes_written: u64,

    /// Cache hints from GET response (when present).
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

fn emit<F: FnOnce() -> DownloadEvent>(sink: &Option<DownloadEventSink>, make: F) {
    if let Some(s) = sink {
        s(make());
    }
}

fn emit_failed(sink: &Option<DownloadEventSink>, id: &str, url: &str, err: &anyhow::Error) {
    if sink.is_some() {
        emit(sink, || DownloadEvent {
            id: id.to_string(),
            url: url.to_string(),
            phase: DownloadPhase::Failed,
            bytes_downloaded: 0,
            bytes_total: None,
            files_total: None,
            files_completed: None,
            message: Some(format!("{err:#}")),
        });
    } else {
        // Ensure errors are visible in logs even when no sink is present.
        error!(id = %id, url = %url, error = %err, "download failed");
    }
}

fn emit_started(sink: &Option<DownloadEventSink>, id: &str, url: &str) {
    emit(sink, || DownloadEvent {
        id: id.to_string(),
        url: url.to_string(),
        phase: DownloadPhase::Started,
        bytes_downloaded: 0,
        bytes_total: None,
        files_total: None,
        files_completed: None,
        message: None,
    });
}

fn emit_progress(
    sink: &Option<DownloadEventSink>,
    id: &str,
    url: &str,
    bytes_downloaded: u64,
    bytes_total: Option<u64>,
) {
    emit(sink, || DownloadEvent {
        id: id.to_string(),
        url: url.to_string(),
        phase: DownloadPhase::Progress,
        bytes_downloaded,
        bytes_total,
        files_total: None,
        files_completed: None,
        message: None,
    });
}

fn emit_finished(
    sink: &Option<DownloadEventSink>,
    id: &str,
    url: &str,
    _started_at: Instant,
    bytes_written: u64,
    bytes_total: Option<u64>,
) {
    if sink.is_some() {
        emit(sink, || DownloadEvent {
            id: id.to_string(),
            url: url.to_string(),
            phase: DownloadPhase::Finished,
            bytes_downloaded: bytes_written,
            bytes_total,
            files_total: None,
            files_completed: None,
            message: None,
        });
    } else {
        debug!(id = %id, url = %url, bytes = bytes_written, "download finished");
    }
}

fn header_str(headers: &HeaderMap, key: reqwest::header::HeaderName) -> Option<String> {
    headers
        .get(key)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

fn is_retryable_status(s: StatusCode) -> bool {
    s.is_server_error()
        || s == StatusCode::REQUEST_TIMEOUT
        || s == StatusCode::TOO_MANY_REQUESTS
        || s == StatusCode::BAD_GATEWAY
        || s == StatusCode::SERVICE_UNAVAILABLE
        || s == StatusCode::GATEWAY_TIMEOUT
}

fn is_retryable_reqwest_error(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect() || e.is_request()
}

fn backoff_delay(attempt: u16) -> Duration {
    let base = 200u64;
    let pow = 1u64 << std::cmp::min(attempt as u32, 6);
    Duration::from_millis(base.saturating_mul(pow))
}

async fn write_response_to_file<F>(
    resp: reqwest::Response,
    tmp_path: &Path,
    mut on_progress: F,
) -> Result<u64>
where
    F: FnMut(u64),
{
    use futures_util::StreamExt;

    let mut f = tokio::fs::File::create(tmp_path)
        .await
        .with_context(|| format!("create {}", tmp_path.display()))?;

    let mut written: u64 = 0;
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read response chunk")?;
        f.write_all(&chunk).await.context("write chunk")?;
        written = written.saturating_add(chunk.len() as u64);
        on_progress(written);
    }

    f.flush().await.context("flush")?;
    Ok(written)
}
