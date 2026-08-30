use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum LocalFileHealth {
    Unknown,
    MissingDestination,
    InventoryUnavailable,
    Missing,
    Dirty,
    Clean,
    InvalidProfile,
}
