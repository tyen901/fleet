use std::collections::HashMap;

use crate::model::{
    DesiredState, ExpectedFile, FileState, FileStateDelete, FileStateUpsert, StoreError,
    TimestampNs, VerifiedState,
};

pub use crate::events::{EventSink, SyncEvent};
pub use crate::model::Checksummer;
pub use crate::fetch::{FileEntry, FilePart, ModManifest};
pub use crate::remote::{RemoteCapabilities, RemoteRepo, RemoteStream, RemoteStreamImpl};

pub trait StateStore: Send + Sync {
    fn desired_state_get(&self) -> Result<Option<DesiredState>, StoreError>;

    fn expected_replace_all_if_digest_changed(
        &self,
        state_id: &str,
        rows: Vec<ExpectedFile>,
        digest_hex: &str,
    ) -> Result<(), StoreError>;

    fn baseline_exists(&self, state_id: &str) -> Result<bool, StoreError>;

    fn expected_get_all(&self, state_id: &str) -> Result<Vec<ExpectedFile>, StoreError>;

    fn file_state_get_all_for_mod(
        &self,
        state_id: &str,
        mod_id: &str,
    ) -> Result<HashMap<String, FileState>, StoreError>;

    fn file_state_apply_batch(
        &self,
        state_id: &str,
        upserts: Vec<FileStateUpsert>,
        deletes: Vec<FileStateDelete>,
    ) -> Result<(), StoreError>;

    fn file_state_delete(
        &self,
        state_id: &str,
        mod_id: &str,
        rel_path: &str,
    ) -> Result<(), StoreError>;

    fn verified_get(&self) -> Result<Option<VerifiedState>, StoreError>;
    fn verified_set(&self, state_id: &str, verified_at: TimestampNs) -> Result<(), StoreError>;
    fn verified_clear(&self) -> Result<(), StoreError>;
}
