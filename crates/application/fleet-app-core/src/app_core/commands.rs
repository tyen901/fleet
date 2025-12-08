use crate::domain::{ProfileId, Route};

#[derive(Debug, Clone)]
pub enum AppCommand {
    // Boot
    LoadInitialState,

    // Navigation
    Navigate(Route),

    // Editor lifecycle
    StartNewProfile,
    EditProfile(ProfileId),
    SaveProfileDraft,
    CancelProfileDraft,
    DeleteProfile(ProfileId),

    // Pipeline
    StartCheck(ProfileId),
    ExecuteSync(ProfileId),
    CancelPipeline,

    // Launch
    Launch(ProfileId),
}
