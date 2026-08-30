use crate::operations::{check_repo, local_files, OperationPublisher, OperationStage};
use fleet_domain::health::{CheckReport, OperationKind};
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
    let repo_publisher = OperationPublisher::silent(profile.id.clone(), OperationKind::Check);
    let inventory_publisher = OperationPublisher::silent(profile.id.clone(), OperationKind::Check);
    publisher.stage(OperationStage::LoadingExpectedState);

    let work = async {
        tokio::join!(
            check_repo::check_repo(profile, state_root, repo_publisher),
            local_files::check(
                profile,
                state_root,
                inventory_publisher,
                cancellation.clone(),
            )
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
        repo: repo?,
        local: inventory?,
    })
}
