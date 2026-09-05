mod input;
mod profile;
mod runner;
mod source;

use std::sync::Arc;

pub use flux::{Outcome, Phase, Snapshot};
pub use input::{
    load_cached_swifty_materialization_input, load_swifty_materialization_input, swifty_profile_id,
    MaterializationInput,
};
pub use profile::{HashProgressObserver, HashProgressObserverRef};
pub use runner::{check_target, materialize, verify_manifest};

pub type SnapshotObserver = Arc<dyn Fn(flux::Snapshot) + Send + Sync>;

pub fn is_cancellation(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<flux::Error>()
        .is_some_and(|error| error.kind() == flux::ErrorKind::Cancelled)
}
