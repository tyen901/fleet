//! Disposable renderer telemetry. Enabled only by FLEET_SIMULATE_SYNC=1.
use crate::operations::{
    OperationProgressEvent, OperationPublisher, OperationStage, ProgressTrack, ProgressTrackKind,
    TaskUsage,
};
use fleet_domain::health::{
    CheckReport, LocalFileHealth, LocalFileReport, RepoCheckFreshness, RepoCheckReport, SyncReport,
    VerificationKind,
};
use fleet_domain::Profile;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
const TOTAL_BYTES: u64 = 400 * 1024 * 1024;
pub(crate) fn is_enabled() -> bool {
    std::env::var("FLEET_SIMULATE_SYNC").is_ok_and(|value| value == "1")
}
pub(crate) async fn scan(
    publisher: &OperationPublisher,
    cancel: &CancellationToken,
) -> Result<(), crate::ApiError> {
    for step in 0..=10 {
        if cancel.is_cancelled() {
            return Err(crate::ApiError::new("canceled", "canceled"));
        }
        publisher.progress(OperationProgressEvent {
            stage: OperationStage::VerifyingInventory,
            tracks: vec![ProgressTrack {
                kind: ProgressTrackKind::LocalCheck,
                done: TOTAL_BYTES * step / 10,
                total: Some(TOTAL_BYTES),
            }],
            usage: TaskUsage {
                network_bytes_per_sec: 0,
                disk_bytes_per_sec: TOTAL_BYTES,
                eta_seconds: Some(1),
            },
        });
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
    Ok(())
}
pub(crate) async fn sync(
    profile: &Profile,
    publisher: OperationPublisher,
    cancel: CancellationToken,
) -> Result<SyncReport, crate::ApiError> {
    scan(&publisher, &cancel).await?;
    let hold = std::env::var("FLEET_SIMULATE_SYNC_HOLD_PERCENT")
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    for step in 0..=20 {
        if cancel.is_cancelled() {
            tokio::time::sleep(Duration::from_millis(500)).await;
            return Err(crate::ApiError::new("canceled", "canceled"));
        }
        publisher.progress(OperationProgressEvent {
            stage: OperationStage::Sync,
            tracks: vec![
                ProgressTrack {
                    kind: ProgressTrackKind::Download,
                    done: TOTAL_BYTES / 2 * step / 20,
                    total: Some(TOTAL_BYTES / 2),
                },
                ProgressTrack {
                    kind: ProgressTrackKind::Patch,
                    done: TOTAL_BYTES * step / 20,
                    total: Some(TOTAL_BYTES),
                },
            ],
            usage: TaskUsage {
                network_bytes_per_sec: 20 * 1024 * 1024,
                disk_bytes_per_sec: 50 * 1024 * 1024,
                eta_seconds: Some((20 - step) / 2),
            },
        });
        if hold == Some(step * 100 / 20) {
            tokio::select! {_=cancel.cancelled()=>{tokio::time::sleep(Duration::from_millis(500)).await;return Err(crate::ApiError::new("canceled","canceled"));},_=tokio::time::sleep(Duration::from_secs(3))=>{}}
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    publisher.stage(OperationStage::Finalizing);
    let report = check(profile, true);
    Ok(SyncReport {
        profile_id: profile.id.clone(),
        repo: report.repo,
        local: LocalFileReport {
            verification: VerificationKind::Materialized,
            ..report.local
        },
    })
}
/// Deterministic profile health for the disposable UI fixture.
pub(crate) fn check(profile: &Profile, materialized: bool) -> CheckReport {
    let now = fleet_domain::time::now_unix_ms();
    CheckReport {
        profile_id: profile.id.clone(),
        repo: RepoCheckReport {
            profile_id: profile.id.clone(),
            local_revision: None,
            remote_revision: None,
            freshness: RepoCheckFreshness::UpToDate,
            checked_at_unix_ms: now,
        },
        local: LocalFileReport {
            profile_id: profile.id.clone(),
            verification: VerificationKind::Fast,
            health: if materialized {
                LocalFileHealth::Clean
            } else {
                LocalFileHealth::Missing
            },
            checked_at_unix_ms: now,
        },
    }
}
