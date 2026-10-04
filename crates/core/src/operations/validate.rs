use crate::operations::progress::progress_channel;
use crate::operations::{local_files, OperationPublisher, OperationStage};
use fleet_domain::health::LocalFileReport;
use fleet_domain::Profile;
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub(crate) async fn validate(
    profile: &Profile,
    state_root: &Path,
    publisher: OperationPublisher,
    cancellation: CancellationToken,
) -> Result<LocalFileReport, crate::ApiError> {
    publisher.stage(OperationStage::Validating);
    publisher.stage(OperationStage::LoadingExpectedState);
    let (observers, progress_receiver) = progress_channel(publisher.clone());
    let validation = local_files::validate(profile, state_root, cancellation, observers);
    let report = progress_receiver
        .observe(publisher.clone(), validation)
        .await?;
    Ok(report)
}
