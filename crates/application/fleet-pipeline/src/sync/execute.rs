use std::collections::HashMap;
use std::fs;

use camino::Utf8Path;
use fleet_core::path_utils::FleetPath;
use fleet_core::SyncPlan;
use fleet_infra::net::{DownloadEvent, DownloadRequest, Downloader};
use tokio::sync::mpsc::Sender;

use crate::io_utils::robust_rename;
use crate::sync::{SyncError, SyncOptions, SyncStats};
use fleet_scanner::Scanner;

fn validate_relative_path(path: &str) -> Result<(), SyncError> {
    if path.contains("..") {
        return Err(SyncError::Execution(format!(
            "Security: Path contains parent directory traversal '..': {path}"
        )));
    }
    if path.starts_with('/')
        || path.starts_with('\\')
        || (path.len() > 1 && path.chars().nth(1) == Some(':'))
    {
        return Err(SyncError::Execution(format!(
            "Security: Path appears absolute: {path}"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct SyncArtifact {
    pub mod_name: String,
    pub rel_path: String,
    pub checksum: String,
    pub size: u64,
    pub final_mtime: u64,
}

#[async_trait::async_trait]
pub trait PlanExecutor: Send + Sync {
    async fn execute(
        &self,
        root: &Utf8Path,
        repo_url: &str,
        plan: SyncPlan,
        opts: &SyncOptions,
        progress_tx: Option<Sender<DownloadEvent>>,
    ) -> Result<(Vec<SyncArtifact>, SyncStats), SyncError>;
}

pub struct DefaultPlanExecutor {
    client: reqwest::Client,
}

impl DefaultPlanExecutor {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl PlanExecutor for DefaultPlanExecutor {
    async fn execute(
        &self,
        root: &Utf8Path,
        repo_url: &str,
        plan: SyncPlan,
        opts: &SyncOptions,
        progress_tx: Option<Sender<DownloadEvent>>,
    ) -> Result<(Vec<SyncArtifact>, SyncStats), SyncError> {
        let mut stats = SyncStats::default();
        let root_std = root.as_std_path();

        // Deletes
        for del in &plan.deletes {
            validate_relative_path(&del.path)?;
            let path = root.join(&del.path);
            if !path.as_std_path().starts_with(root_std) {
                return Err(SyncError::Execution(format!(
                    "Security: Delete path escapes root: {path}"
                )));
            }
            if path.exists() {
                if path.is_dir() {
                    let _ = tokio::fs::remove_dir_all(path.as_std_path()).await;
                    stats.mods_deleted += 1;
                } else {
                    let _ = tokio::fs::remove_file(path.as_std_path()).await;
                    stats.files_deleted += 1;
                }
            }
        }

        // Renames
        for ren in &plan.renames {
            validate_relative_path(&ren.old_path)?;
            validate_relative_path(&ren.new_path)?;

            let old = root.join(&ren.old_path);
            let new = root.join(&ren.new_path);
            if !old.as_std_path().starts_with(root_std) || !new.as_std_path().starts_with(root_std)
            {
                return Err(SyncError::Execution(format!(
                    "Security: Rename path escapes root: {} -> {}",
                    ren.old_path, ren.new_path
                )));
            }
            if old.exists() {
                let _ = robust_rename(old.as_std_path(), new.as_std_path()).await;
                stats.renames += 1;
            }
        }

        // Downloads
        let mut requests = Vec::new();
        #[derive(Debug)]
        struct DlCtx {
            mod_name: String,
            rel_path: String,
            checksum: String,
            size: u64,
        }
        let mut ctx_map = HashMap::new();

        for (i, action) in plan.downloads.iter().enumerate() {
            // SECURITY CHECK
            validate_relative_path(&action.mod_name)?;
            validate_relative_path(&action.rel_path)?;

            let id = i as u64;
            let url = build_file_url(repo_url, &action.mod_name, &action.rel_path)
                .map_err(SyncError::Execution)?;
            // Normalize relative path so on-disk layout is consistent
            let normalized_rel = FleetPath::normalize(&action.rel_path);
            // Re-validate after normalization just to be safe
            validate_relative_path(&normalized_rel)?;

            let target = root.join(&action.mod_name).join(&normalized_rel);
            if !target.as_std_path().starts_with(root_std) {
                return Err(SyncError::Execution(format!(
                    "Security: Download target escapes root: {}",
                    target
                )));
            }
            requests.push(DownloadRequest {
                id,
                url,
                target_path: target,
                expected_size: action.size,
                expected_checksum: Some(action.expected_checksum.clone()),
            });
            ctx_map.insert(
                id,
                DlCtx {
                    mod_name: action.mod_name.clone(),
                    rel_path: normalized_rel,
                    checksum: action.expected_checksum.clone(),
                    size: action.size,
                },
            );

            stats.files_planned_download += 1;
            stats.bytes_planned_download += action.size;
        }

        let downloader =
            Downloader::new(self.client.clone(), opts.max_threads, opts.rate_limit_bytes);
        let results = downloader.download_batch(requests, progress_tx).await;

        let mut artifacts = Vec::new();
        let mut failed = 0;
        for res in results {
            if res.success {
                if let Some(ctx) = ctx_map.get(&res.id) {
                    let abs_path = root.join(&ctx.mod_name).join(&ctx.rel_path);
                    let now = std::time::SystemTime::now();
                    let _ = filetime::set_file_mtime(
                        abs_path.as_std_path(),
                        filetime::FileTime::from_system_time(now),
                    );

                    // Read back exactly what the OS recorded.
                    // Do not trust 'now' because some filesystems coarsen or adjust timestamps.
                    match fs::metadata(abs_path.as_std_path()) {
                        Ok(meta) => {
                            let mtime = Scanner::mtime(&meta);
                            let size = meta.len();
                            artifacts.push(SyncArtifact {
                                mod_name: ctx.mod_name.clone(),
                                rel_path: ctx.rel_path.clone(),
                                checksum: ctx.checksum.clone(),
                                size,
                                final_mtime: mtime,
                            });
                        }
                        Err(e) => {
                            tracing::error!("Failed to stat downloaded file {}: {}", abs_path, e);
                            failed += 1;
                        }
                    }
                } else {
                    failed += 1;
                }
            } else {
                failed += 1;
            }
        }

        if failed > 0 {
            return Err(SyncError::Execution(format!("Failed downloads: {failed}")));
        }

        Ok((artifacts, stats))
    }
}

fn build_file_url(repo_url: &str, mod_name: &str, rel_path: &str) -> Result<String, String> {
    let base = crate::sync::remote::normalize_repo_base(repo_url)
        .map_err(|e| format!("invalid repo url {repo_url}: {e}"))?;

    let normalized_rel = FleetPath::normalize(rel_path);
    let mut url = base;
    url.path_segments_mut()
        .map_err(|_| "invalid repo url segments".to_string())?
        .pop_if_empty();

    {
        let mut segs = url
            .path_segments_mut()
            .map_err(|_| "cannot mutate url".to_string())?;
        segs.push(mod_name);
        for part in normalized_rel.split('/') {
            if !part.is_empty() {
                segs.push(part);
            }
        }
    }

    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::build_file_url;

    #[test]
    fn build_file_url_accepts_repo_json_url() {
        let url = build_file_url(
            "https://cdn.deltasync.io/data/pca_2.2.9/repo.json",
            "@tiny",
            "addons/file.txt",
        )
        .unwrap();

        assert_eq!(
            url,
            "https://cdn.deltasync.io/data/pca_2.2.9/@tiny/addons/file.txt"
        );
    }

    #[test]
    fn build_file_url_accepts_base_url_without_trailing_slash() {
        let url =
            build_file_url("https://example.com/data/pca_2.2.9", "@tiny", "file.txt").unwrap();
        assert_eq!(url, "https://example.com/data/pca_2.2.9/@tiny/file.txt");
    }

    #[test]
    fn build_file_url_encodes_segments() {
        let url =
            build_file_url("https://example.com/base/", "@My Mod", "addons/pack.pbo").unwrap();
        assert!(url.contains("addons/pack.pbo"));
        assert!(url.contains("%20") || url.contains("My Mod") || url.contains("My+Mod"));
    }
}
