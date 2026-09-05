//! Scripted sync progress for the automated UI render flow.
//!
//! Enabled by `FLEET_SIMULATE_SYNC=1`. Emits a deterministic progress sequence
//! and returns an up-to-date report without touching the network, the repo
//! cache, the inventory, or the profile destination.

use crate::operations::{
    OperationProgressEvent, OperationPublisher, OperationStage, ProgressMetric, ProgressUnit,
};
use fleet_domain::health::{
    LocalFileHealth, LocalFileReport, RepoCheckFreshness, RepoCheckReport, SyncReport,
    VerificationKind,
};
use fleet_domain::Profile;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const ENV_FLAG: &str = "FLEET_SIMULATE_SYNC";
const ENV_HOLD_PERCENT: &str = "FLEET_SIMULATE_SYNC_HOLD_PERCENT";
const TOTAL_FILES: u64 = 20;
const TOTAL_BYTES: u64 = 400 * 1024 * 1024;
const STEP_DELAY: Duration = Duration::from_millis(120);
const CANCEL_DELAY: Duration = Duration::from_millis(500);

pub(crate) fn is_enabled() -> bool {
    std::env::var(ENV_FLAG).is_ok_and(|value| value == "1")
}

/// Percentage at which the sequence parks until cancelled.
fn hold_percent() -> Option<u64> {
    std::env::var(ENV_HOLD_PERCENT)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
}

pub(crate) async fn sync(
    profile: &Profile,
    publisher: OperationPublisher,
    cancel: CancellationToken,
) -> Result<SyncReport, crate::ApiError> {
    publisher.stage(OperationStage::Validating);
    publisher.stage(OperationStage::LoadingExpectedState);
    publisher.stage(OperationStage::Sync);

    for step in 0..=TOTAL_FILES {
        if cancel.is_cancelled() {
            return Err(crate::ApiError::new("canceled", "operation canceled"));
        }
        let done_bytes = TOTAL_BYTES * step / TOTAL_FILES;
        publisher.progress(OperationProgressEvent {
            stage: OperationStage::Sync,
            status_text: Some("Syncing files".to_string()),
            primary: ProgressMetric {
                label: Some("Installed".to_string()),
                done: Some(done_bytes),
                total: Some(TOTAL_BYTES),
                unit: ProgressUnit::Bytes,
            },
            secondary: Some(ProgressMetric {
                label: Some("Downloaded".to_string()),
                done: Some(done_bytes),
                total: Some(TOTAL_BYTES),
                unit: ProgressUnit::Bytes,
            }),
            throughput_bytes_per_sec: Some(12 * 1024 * 1024),
            eta_seconds: Some(TOTAL_FILES - step),
        });
        if hold_percent() == Some(step * 100 / TOTAL_FILES) {
            cancel.cancelled().await;
            tokio::time::sleep(CANCEL_DELAY).await;
            return Err(crate::ApiError::new("canceled", "operation canceled"));
        }
        tokio::time::sleep(STEP_DELAY).await;
    }

    publisher.stage(OperationStage::Finalizing);

    let checked_at = fleet_domain::time::now_unix_ms();
    Ok(SyncReport {
        profile_id: profile.id.clone(),
        repo: RepoCheckReport {
            profile_id: profile.id.clone(),
            local_revision: Some("simulated".to_string()),
            remote_revision: Some("simulated".to_string()),
            freshness: RepoCheckFreshness::UpToDate,
            checked_at_unix_ms: checked_at,
        },
        local: LocalFileReport {
            profile_id: profile.id.clone(),
            verification: VerificationKind::Materialized,
            health: LocalFileHealth::Clean,
            checked_at_unix_ms: checked_at,
        },
    })
}
