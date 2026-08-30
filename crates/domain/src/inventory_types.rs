use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum LocalStateHealth {
    Unknown,
    MissingDestination,
    LocalStateMissing,
    LocalDrift,
    Ready,
    Blocked,
    InvalidProfile,
    ProbeFailed,
}
