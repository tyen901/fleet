use crate::operations::progress::FluxProgressObserver;
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
    let (progress, progress_receiver) =
        FluxProgressObserver::channel(fleet_domain::OperationKind::Validate);
    let validation = local_files::validate(profile, state_root, cancellation, Some(progress));
    let report = progress_receiver
        .observe(publisher.clone(), validation)
        .await?;
    publisher.stage(OperationStage::Finalizing);
    Ok(report)
}
