//! Fast metadata download service for Fleet.
//!
//! Purpose-built for small blobs such as `repo.json` and `mod.srf`.

use anyhow::{anyhow, Context, Result};
use atomic_write_file::AtomicWriteFile;
use reqwest::header::{HeaderMap, HeaderValue, ETAG, LAST_MODIFIED, USER_AGENT};
use reqwest::StatusCode;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, error};

#[derive(Debug, Clone)]
pub struct DownloadServiceConfig {
    pub user_agent: String,
    pub connect_timeout: Duration,
    pub timeout: Duration,
    pub retries: u16,
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
    client: ClientWithMiddleware,
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
        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(cfg.retries as u32);
        let client = ClientBuilder::new(client)
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();
        Self { cfg, client }
    }

    pub fn new_default() -> Self {
        Self::new(DownloadServiceConfig::default())
    }

    pub fn parallel_requests(&self) -> usize {
        usize::from(self.cfg.parallel_requests.max(1))
    }

    pub async fn download_one(
        &self,
        id: impl Into<String>,
        url: &str,
        extra_headers: Option<HeaderMap>,
    ) -> Result<DownloadResult> {
        let id = id.into();
        let request = self.client.get(url);
        let request = match extra_headers {
            Some(headers) => request.headers(headers),
            None => request,
        };
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                let error =
                    anyhow::Error::new(error).context(format!("send metadata request {url}"));
                error!(id, url, error = %error, "metadata download failed");
                return Err(error);
            }
        };
        let status = response.status();
        let etag = header_str(response.headers(), ETAG);
        let last_modified = header_str(response.headers(), LAST_MODIFIED);

        if status == StatusCode::NOT_MODIFIED {
            debug!(id, url, "metadata not modified");
            return Ok(DownloadResult::NotModified);
        }
        if status != StatusCode::OK {
            let error = anyhow!("unexpected status {status} for GET {url}");
            error!(id, url, error = %error, "metadata download failed");
            return Err(error);
        }

        let bytes = match response.bytes().await {
            Ok(bytes) => bytes.to_vec(),
            Err(error) => {
                let error =
                    anyhow::Error::new(error).context(format!("read metadata response body {url}"));
                error!(id, url, error = %error, "metadata download failed");
                return Err(error);
            }
        };
        debug!(id, url, bytes = bytes.len(), "metadata downloaded");
        Ok(DownloadResult::Downloaded(DownloadOutcome {
            id,
            bytes,
            etag,
            last_modified,
        }))
    }

    pub fn download_many(
        &self,
        specs: Vec<DownloadSpec>,
    ) -> impl futures_util::Stream<Item = Result<DownloadOutcome>> + '_ {
        use futures_util::stream::{self, StreamExt};

        let max_in_flight = self.parallel_requests();
        let service = self.clone();
        stream::iter(specs.into_iter().map(move |spec| {
            let service = service.clone();
            async move {
                let url = spec.url.clone();
                match service
                    .download_one(spec.id, &url, None)
                    .await
                    .with_context(|| format!("download metadata {url}"))?
                {
                    DownloadResult::Downloaded(outcome) => Ok(outcome),
                    DownloadResult::NotModified => Err(anyhow!("unexpected 304 for {url}")),
                }
            }
        }))
        .buffer_unordered(max_in_flight)
    }

    pub async fn download_one_to_file(
        &self,
        id: impl Into<String>,
        url: &str,
        download_folder: &Path,
        file_name: &Path,
    ) -> Result<()> {
        match self.download_one(id, url, None).await? {
            DownloadResult::NotModified => Ok(()),
            DownloadResult::Downloaded(outcome) => {
                let out_path = download_folder.join(file_name);
                if let Some(parent) = out_path.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .with_context(|| format!("create parent dir {}", parent.display()))?;
                }
                write_bytes_atomically(out_path, outcome.bytes).await
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadSpec {
    pub id: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub enum DownloadResult {
    NotModified,
    Downloaded(DownloadOutcome),
}

#[derive(Debug, Clone)]
pub struct DownloadOutcome {
    pub id: String,
    pub bytes: Vec<u8>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

fn header_str(headers: &HeaderMap, key: reqwest::header::HeaderName) -> Option<String> {
    headers
        .get(key)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use axum::extract::State;
    use axum::routing::get;
    use axum::Router;
    use futures_util::TryStreamExt;
    use tokio::sync::{Barrier, Notify};
    use tokio::time::{timeout, Duration};

    use super::*;

    struct ConcurrencyState {
        started: AtomicUsize,
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
        ready: Notify,
        gate: Barrier,
    }

    #[tokio::test]
    async fn download_many_runs_requests_concurrently_without_exceeding_limit() {
        let state = Arc::new(ConcurrencyState {
            started: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
            ready: Notify::new(),
            gate: Barrier::new(3),
        });
        let app = Router::new()
            .route("/{*path}", get(blocking_handler))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind server");
        let address = listener.local_addr().expect("server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve requests");
        });
        let service = DownloadService::new(DownloadServiceConfig {
            parallel_requests: 2,
            ..DownloadServiceConfig::default()
        });
        let specs = (0..3)
            .map(|id| DownloadSpec {
                id: format!("request-{id}"),
                url: format!("http://{address}/{id}"),
            })
            .collect();
        let task =
            tokio::spawn(async move { service.download_many(specs).try_collect::<Vec<_>>().await });

        timeout(Duration::from_secs(5), async {
            while state.started.load(Ordering::SeqCst) < 2 {
                state.ready.notified().await;
            }
        })
        .await
        .expect("two concurrent requests reached the server");
        assert_eq!(state.started.load(Ordering::SeqCst), 2);
        assert_eq!(state.in_flight.load(Ordering::SeqCst), 2);

        state.gate.wait().await;
        task.await
            .expect("download task")
            .expect("download results");

        assert_eq!(state.started.load(Ordering::SeqCst), 3);
        assert_eq!(state.max_in_flight.load(Ordering::SeqCst), 2);
        server.abort();
    }

    async fn blocking_handler(State(state): State<Arc<ConcurrencyState>>) -> &'static str {
        let started = state.started.fetch_add(1, Ordering::SeqCst) + 1;
        let in_flight = state.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        state.max_in_flight.fetch_max(in_flight, Ordering::SeqCst);
        if started <= 2 {
            state.ready.notify_one();
            state.gate.wait().await;
        }
        state.in_flight.fetch_sub(1, Ordering::SeqCst);
        "metadata"
    }
}
