use camino::Utf8PathBuf;
use futures::stream::{self, StreamExt};
use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use reqwest::Client;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::Sender;
use tracing::warn;

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub id: u64,
    pub url: String,
    pub target_path: Utf8PathBuf,
    pub expected_size: u64,
    pub expected_checksum: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DownloadResult {
    pub id: u64,
    pub success: bool,
    pub bytes_downloaded: u64,
}

#[derive(Debug)]
pub enum DownloadEvent {
    Started { id: u64, total_bytes: u64 },
    Progress { id: u64, bytes_delta: u64 },
    Completed { id: u64, success: bool },
}

pub struct Downloader {
    client: Client,
    concurrency: usize,
    rate_limit_bytes: Option<u64>,
}

impl Downloader {
    pub fn new(client: Client, concurrency: usize, rate_limit_bytes: Option<u64>) -> Self {
        Self {
            client,
            concurrency,
            rate_limit_bytes,
        }
    }

    /// Generic batch download. Does NOT handle deletes, renames, or domain logic.
    pub async fn download_batch(
        &self,
        items: Vec<DownloadRequest>,
        progress_tx: Option<Sender<DownloadEvent>>,
    ) -> Vec<DownloadResult> {
        let limiter = self.rate_limit_bytes.and_then(|bps| {
            NonZeroU32::new(bps as u32)
                .map(|nz| Arc::new(RateLimiter::direct(Quota::per_second(nz))))
        });
        // FIX: Use buffer_unordered to drive concurrency without deadlock
        stream::iter(items)
            .map(|item| {
                let client = self.client.clone();
                let tx = progress_tx.clone();
                let lim = limiter.clone();

                async move { Self::download_single(client, item, tx, lim).await }
            })
            .buffer_unordered(self.concurrency)
            .collect()
            .await
    }

    async fn download_single(
        client: Client,
        req: DownloadRequest,
        tx: Option<Sender<DownloadEvent>>,
        lim: Option<Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>>>,
    ) -> DownloadResult {
        if let Some(ref t) = tx {
            let _ = t
                .send(DownloadEvent::Started {
                    id: req.id,
                    total_bytes: req.expected_size,
                })
                .await;
        }

        let tmp_path = req.target_path.with_extension("part");

        if let Some(parent) = req.target_path.parent() {
            let _ = tokio::fs::create_dir_all(parent.as_std_path()).await;
        }

        let mut success = false;
        let mut total_written = 0;

        for _attempt in 0..3 {
            if let Ok(resp) = client.get(&req.url).send().await {
                if resp.status().is_success() {
                    if let Ok(mut file) = File::create(tmp_path.as_std_path()).await {
                        let mut stream = resp.bytes_stream();
                        let mut write_err = false;

                        let mut accumulated = 0u64;
                        let mut last_emit = Instant::now();

                        while let Some(chunk_res) = stream.next().await {
                            match chunk_res {
                                Ok(chunk) => {
                                    if let Some(l) = &lim {
                                        if let Some(nz) = NonZeroU32::new(chunk.len() as u32) {
                                            l.until_n_ready(nz).await.ok();
                                        }
                                    }
                                    if file.write_all(&chunk).await.is_ok() {
                                        let len = chunk.len() as u64;
                                        total_written += len;
                                        accumulated += len;

                                        if accumulated > 1_000_000
                                            || last_emit.elapsed().as_millis() > 100
                                        {
                                            if let Some(ref t) = tx {
                                                let _ = t
                                                    .send(DownloadEvent::Progress {
                                                        id: req.id,
                                                        bytes_delta: accumulated,
                                                    })
                                                    .await;
                                            }
                                            accumulated = 0;
                                            last_emit = Instant::now();
                                        }
                                    } else {
                                        write_err = true;
                                        break;
                                    }
                                }
                                Err(_) => {
                                    write_err = true;
                                    break;
                                }
                            }
                        }

                        if accumulated > 0 {
                            if let Some(ref t) = tx {
                                let _ = t
                                    .send(DownloadEvent::Progress {
                                        id: req.id,
                                        bytes_delta: accumulated,
                                    })
                                    .await;
                            }
                        }

                        if !write_err {
                            let _ = file.flush().await;

                            // Verification: if an expected checksum is provided, compute it
                            // using `fleet-hashing` before committing the file to the final path.
                            let mut verified = true;
                            if let Some(expected) = &req.expected_checksum {
                                let tmp_path_clone = tmp_path.clone();
                                let target_filename = req
                                    .target_path
                                    .file_name()
                                    .map(|s| s.to_string())
                                    .unwrap_or_default();

                                let check_res = tokio::task::spawn_blocking(move || {
                                    let logical = camino::Utf8Path::new(&target_filename);
                                    crate::hashing::compute_file_checksum(&tmp_path_clone, logical)
                                        .ok()
                                })
                                .await;

                                match check_res {
                                    Ok(Some(actual)) => {
                                        if !actual.eq_ignore_ascii_case(expected) {
                                            warn!(
                                                "Checksum mismatch for {}: expected {}, got {}",
                                                req.url, expected, actual
                                            );
                                            verified = false;
                                        }
                                    }
                                    _ => {
                                        warn!("Failed to compute checksum for {}", req.url);
                                        verified = false;
                                    }
                                }
                            }

                            if verified
                                && tokio::fs::rename(
                                    tmp_path.as_std_path(),
                                    req.target_path.as_std_path(),
                                )
                                .await
                                .is_ok()
                            {
                                success = true;
                                break;
                            }
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        if !success {
            let _ = tokio::fs::remove_file(&tmp_path).await;
        }

        if let Some(ref t) = tx {
            let _ = t
                .send(DownloadEvent::Completed {
                    id: req.id,
                    success,
                })
                .await;
        }

        DownloadResult {
            id: req.id,
            success,
            bytes_downloaded: total_written,
        }
    }
}
