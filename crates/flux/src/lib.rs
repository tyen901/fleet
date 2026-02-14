mod convert;
mod flux_sqlite;
mod progress;
mod retrieval;
mod runner;

use std::path::PathBuf;
use std::sync::Arc;

pub struct FluxEngine {
    flux_cache_dir: PathBuf,
}

#[derive(Clone, Copy)]
pub struct FluxSyncOptions {
    pub enable_prune: bool,
}

pub struct FluxSyncReport {
    pub duration_ms: u64,
    pub bytes_reused: u64,
    pub bytes_downloaded: u64,
    pub files_finalized: u64,
    pub prune_paths: Vec<std::path::PathBuf>,
}

pub type FluxProgressSink = Arc<dyn Fn(fleet_domain::sync::SyncProgress) + Send + Sync>;

impl FluxEngine {
    pub fn new(flux_cache_dir: PathBuf) -> Self {
        Self { flux_cache_dir }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn sync(
        &self,
        dest: &std::path::Path,
        inventory_db_path: &std::path::Path,
        inventory_name: &str,
        desired: flux_manifest::DesiredManifest,
        opts: FluxSyncOptions,
        cancel: tokio_util::sync::CancellationToken,
        progress_sink: Option<FluxProgressSink>,
    ) -> anyhow::Result<FluxSyncReport> {
        runner::sync(
            self.flux_cache_dir.clone(),
            dest,
            inventory_db_path,
            inventory_name,
            desired,
            opts,
            cancel,
            progress_sink,
        )
        .await
    }

    pub fn prune_only(
        &self,
        dest: &std::path::Path,
        inventory_db_path: &std::path::Path,
        inventory_name: &str,
        prune_paths: Vec<std::path::PathBuf>,
    ) -> anyhow::Result<()> {
        runner::prune_only(
            self.flux_cache_dir.clone(),
            dest,
            inventory_db_path,
            inventory_name,
            prune_paths,
        )
        .map(|_| ())
    }
}
