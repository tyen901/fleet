use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use flux_api::PruneMode;
use flux_api::{HaveState, SyncConfig};
use flux_types::Verifier;
use tokio_util::sync::CancellationToken;

use crate::convert;
use crate::flux_sqlite::SqliteFluxInventory;
use crate::progress;
use crate::{FluxProgressSink, FluxSyncOptions, FluxSyncReport};

#[allow(clippy::too_many_arguments)]
pub(crate) async fn sync(
    flux_cache_dir: std::path::PathBuf,
    dest: &Path,
    inventory_db_path: &Path,
    inventory_name: &str,
    desired: flux_manifest::DesiredManifest,
    opts: FluxSyncOptions,
    cancel: CancellationToken,
    progress_sink: Option<FluxProgressSink>,
) -> Result<FluxSyncReport> {
    let desired_state = convert::desired_manifest_to_desired_state(&desired)
        .context("convert manifest -> DesiredState")?;

    let local_store: Arc<dyn flux_types::SegmentStore> = Arc::new(
        flux_segment_cache::SegmentCache::new(flux_cache_dir.clone()),
    );

    let start = Instant::now();
    let cancel_clone = cancel.clone();
    let dest = dest.to_path_buf();
    let inventory_db_path = inventory_db_path.to_path_buf();
    let inventory_name = inventory_name.to_string();

    let (progress_tx, progress_rx) = std::sync::mpsc::channel::<flux_api::ProgressEvent>();
    let (bridge, totals, last) =
        progress::spawn_bridge(cancel.clone(), progress_sink.clone(), progress_rx);

    let report_res = tokio::task::spawn_blocking(move || -> Result<flux_api::SyncReport> {
        if cancel_clone.is_cancelled() {
            anyhow::bail!("canceled");
        }

        let inventory = Arc::new(
            SqliteFluxInventory::open_sqlite(&inventory_db_path, &inventory_name, &dest)
                .context("open fleet_inventory for flux")?,
        );

        let retriever_cfg = retriever::RetrievalConfig::default();
        let retriever = retriever::Retriever::new(retriever_cfg).context("build retriever")?;
        let remote: Arc<dyn flux_provider::Provider> = crate::retrieval::provider_arc(retriever);
        let have = HaveState {
            target_root: dest.clone(),
            local_store,
            nearby_indices: vec![],
            remote,
            inventory,
            verifier: None::<Arc<dyn Verifier>>,
        };

        let cfg = SyncConfig {
            progress_sender: Some(progress_tx),
            prune_mode: if opts.enable_prune {
                PruneMode::ScanAndCompute
            } else {
                PruneMode::ExplicitOnly
            },
            apply_prune: false,
            ..SyncConfig::default()
        };

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("create flux runtime")?;
        let report = rt
            .block_on(
                async move { flux_api::sync_async(cfg, desired_state, have, cancel_clone).await },
            )
            .context("flux_api::sync_async")?;
        Ok(report)
    })
    .await;

    let _ = bridge.join();
    let report = report_res.context("join flux worker")??;
    let duration_ms = start.elapsed().as_millis() as u64;

    if let Some(sink) = &progress_sink {
        let totals = totals.lock().map(|g| g.clone()).unwrap_or_default();
        let last = last.lock().map(|g| g.clone()).unwrap_or_default();
        progress::emit_final_progress(sink, &totals, &last, &report);
    }

    Ok(FluxSyncReport {
        duration_ms,
        bytes_reused: report.bytes_reused,
        bytes_downloaded: report.bytes_downloaded,
        files_finalized: report.files_finalized,
        prune_paths: report.prune_paths,
    })
}

pub(crate) fn prune_only(
    flux_cache_dir: std::path::PathBuf,
    dest: &Path,
    inventory_db_path: &Path,
    inventory_name: &str,
    prune_paths: Vec<std::path::PathBuf>,
) -> Result<flux_api::PruneReport> {
    let inventory = Arc::new(
        SqliteFluxInventory::open_sqlite(inventory_db_path, inventory_name, dest)
            .context("open fleet_inventory for flux")?,
    );

    let local_store: Arc<dyn flux_types::SegmentStore> =
        Arc::new(flux_segment_cache::SegmentCache::new(flux_cache_dir));

    let retriever_cfg = retriever::RetrievalConfig::default();
    let retriever = retriever::Retriever::new(retriever_cfg).context("build retriever")?;
    let remote: Arc<dyn flux_provider::Provider> = crate::retrieval::provider_arc(retriever);

    let have = HaveState {
        target_root: dest.to_path_buf(),
        local_store,
        nearby_indices: vec![],
        remote,
        inventory,
        verifier: None::<Arc<dyn Verifier>>,
    };

    flux_api::prune_only(None, have, prune_paths).context("flux_api::prune_only")
}
