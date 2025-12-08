use crate::domain::{AppSettings, Profile, Route};
use crate::pipeline::{PipelineRunEvent, PipelineRunId};

#[derive(Debug, Clone)]
pub enum DomainEvent {
    // Boot state
    BootLoadingStarted,
    InitialStateLoaded {
        profiles: Vec<Profile>,
        settings: AppSettings,
    },
    BootFailed {
        message: String,
    },

    // Navigation
    RouteChanged(Route),

    // Editor lifecycle
    DraftOpened(Profile),
    DraftCommitted(Profile),
    DraftCancelled,

    // Pipeline
    PipelineEvent {
        run_id: PipelineRunId,
        ev: PipelineRunEvent,
    },

    // User-visible errors
    UserError(String),
}
