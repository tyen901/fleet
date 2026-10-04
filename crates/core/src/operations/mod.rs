pub mod events;

#[cfg(feature = "flux")]
pub(crate) mod check;
#[cfg(feature = "flux")]
pub(crate) mod check_repo;
#[cfg(feature = "flux")]
pub(crate) mod local_files;
#[cfg(feature = "flux")]
pub(crate) mod progress;
pub(crate) mod runtime;
pub(crate) mod simulated;
#[cfg(feature = "flux")]
pub(crate) mod sync;
#[cfg(feature = "flux")]
pub(crate) mod validate;

pub use events::{
    OperationOutput, OperationProgressEvent, OperationSessionEvent, OperationSessionEventKind,
    OperationStage, ProgressTrack, ProgressTrackKind, TaskUsage,
};
pub(crate) use runtime::{OperationPublisher, OperationRuntime};

#[cfg(not(feature = "flux"))]
pub(crate) fn backend_unavailable() -> crate::ApiError {
    crate::ApiError::new(
        "backend_unavailable",
        "File maintenance is unavailable in this build. Enable the flux feature to refresh, verify, or sync managed files.",
    )
}
