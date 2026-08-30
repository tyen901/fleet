use crate::operations::{local_files, OperationPublisher};
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
    local_files::validate(profile, state_root, publisher, cancellation).await
}
