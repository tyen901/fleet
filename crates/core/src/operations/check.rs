use crate::operations::progress::FluxProgressObserver;
use crate::operations::{check_repo, local_files, OperationPublisher, OperationStage};
use fleet_domain::health::CheckReport;
use fleet_domain::Profile;
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(crate) async fn check(
    profile: &Profile,
    state_root: &Path,
    publisher: OperationPublisher,
    cancellation: CancellationToken,
) -> Result<CheckReport, crate::ApiError> {
    publisher.stage(OperationStage::Validating);
    publisher.stage(OperationStage::LoadingExpectedState);
    let progress = Arc::new(FluxProgressObserver::new(publisher.clone()));

    let work = async {
        tokio::join!(
            check_repo::check_repo(profile, state_root),
            local_files::check(profile, state_root, cancellation.clone(), Some(progress))
        )
    };
    let (repo, inventory) = tokio::select! {
        _ = cancellation.cancelled() => {
            return Err(crate::ApiError::new("canceled", "canceled"));
        }
        result = work => result,
    };
    publisher.stage(OperationStage::Finalizing);
    Ok(CheckReport {
        profile_id: profile.id.clone(),
        repo,
        local: inventory?,
    })
}
