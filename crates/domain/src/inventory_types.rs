use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum ManifestHealth {
    Unknown,
    MissingDestination,
    InventoryUnavailable,
    Missing,
    Different,
    Exact,
    InvalidProfile,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum UnexpectedHealth {
    NotChecked,
    Clean,
    Present,
}
