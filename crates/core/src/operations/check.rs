use crate::operations::progress::progress_channel;
use crate::operations::{check_repo, local_files, OperationPublisher, OperationStage};
use fleet_domain::health::CheckReport;
use fleet_domain::Profile;
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub(crate) async fn check(
    profile: &Profile,
    state_root: &Path,
    publisher: OperationPublisher,
    cancellation: CancellationToken,
) -> Result<CheckReport, crate::ApiError> {
    publisher.stage(OperationStage::Validating);
    publisher.stage(OperationStage::LoadingExpectedState);
    let (observers, receiver) = progress_channel(publisher.clone());
    let work = async {
        tokio::join!(
            async {
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => None,
                    report = check_repo::check_repo(profile, state_root) => Some(report),
                }
            },
            local_files::check(profile, state_root, cancellation.clone(), observers)
        )
    };
    let (repo, inventory) = receiver.observe(publisher.clone(), work).await;
    if cancellation.is_cancelled() {
        return Err(crate::ApiError::new("canceled", "canceled"));
    }
    publisher.stage(OperationStage::Finalizing);
    Ok(CheckReport {
        profile_id: profile.id.clone(),
        repo: repo.ok_or_else(|| crate::ApiError::new("canceled", "canceled"))?,
        local: inventory?,
    })
}
