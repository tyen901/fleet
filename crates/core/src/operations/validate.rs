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
    publisher.stage(OperationStage::VerifyingInventory);
    local_files::inspect(
        profile,
        state_root,
        cancellation,
        local_files::Inspection::Verify(&publisher),
    )
    .await
}
