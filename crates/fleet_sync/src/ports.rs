use std::collections::HashMap;

use crate::model::{
    DesiredState, ExpectedFile, FileState, FileStateDelete, FileStateUpsert, StoreError,
    TimestampNs, VerifiedState,
};
use async_trait::async_trait;
use fleet_index::{ExpectedFileRow, ExpectedPartRow, ObservedPartRow, ObservedRow};
use fleet_manifest_domain::{FetchRange, ModManifest, RelPath};

pub trait Checksummer: Send + Sync {
    fn algorithm_name(&self) -> &str;
    fn hash_file(&self, path: &std::path::Path) -> anyhow::Result<Vec<u8>>;
    fn hash_range(&self, path: &std::path::Path, offset: u64, len: u64) -> anyhow::Result<Vec<u8>>;

    fn hash_ranges(
        &self,
        path: &std::path::Path,
        ranges: &[(u64, u64)],
    ) -> anyhow::Result<Vec<Vec<u8>>> {
        let mut out = Vec::with_capacity(ranges.len());
        for (off, len) in ranges {
            out.push(self.hash_range(path, *off, *len)?);
        }
        Ok(out)
    }
}

pub struct RemoteStream {
    inner: Box<dyn RemoteStreamImpl>,
}

impl RemoteStream {
    pub fn new(inner: Box<dyn RemoteStreamImpl>) -> Self {
        Self { inner }
    }

    pub async fn next_chunk(&mut self) -> anyhow::Result<Option<bytes::Bytes>> {
        self.inner.next_chunk().await
    }
}

#[async_trait]
pub trait RemoteStreamImpl: Send {
    async fn next_chunk(&mut self) -> anyhow::Result<Option<bytes::Bytes>>;
}

pub trait EventSink: Send + Sync {
    fn push(&self, ev: SyncEvent);
}

pub struct NoopSink;

impl EventSink for NoopSink {
    fn push(&self, _: SyncEvent) {}
}

#[derive(Clone, Debug)]
pub enum SyncEvent {
    CheckStarted {
        repo: String,
    },
    CheckFinished {
        ok: bool,
    },

    RepairStarted {
        repo: String,
    },
    RepairSkipEvaluated {
        skippable: bool,
        reason: Option<String>,
    },
    RepairFinished {
        ok: bool,
        skipped: bool,
    },

    SyncFreshStarted {
        repo: String,
    },
    SyncFreshFinished {
        ok: bool,
    },

    RemoteCapabilities {
        supports_ranges: bool,
    },

    ModStarted {
        mod_id: String,
    },
    ModFinished {
        mod_id: String,
    },

    FileUpToDate {
        mod_id: String,
        path: String,
    },
    FileNeedsRepair {
        mod_id: String,
        path: String,
        strategy: String,
    },
    FileStarted {
        mod_id: String,
        path: String,
        bytes_total: u64,
    },
    FileProgress {
        mod_id: String,
        path: String,
        bytes_done: u64,
        bytes_total: u64,
    },
    FileVerified {
        mod_id: String,
        path: String,
    },

    UnexpectedPathsFound {
        mod_id: String,
        files: u64,
        dirs: u64,
        bytes: u64,
        sample: Vec<String>,
    },
    UnexpectedPathDeleted {
        mod_id: String,
        path: String,
        bytes: u64,
        is_dir: bool,
    },
    UnexpectedPathsActionRequired {
        mod_id: String,
        message: String,
    },
    UnexpectedPathsCapReached {
        mod_id: String,
        message: String,
    },
    EmptyDirDeleted {
        path: String,
    },

    Warning {
        message: String,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, Default)]
pub struct RemoteCapabilities {
    pub supports_ranges: bool,
}

#[async_trait]
pub trait RemoteRepo: Send + Sync {
    async fn capabilities(&self) -> anyhow::Result<RemoteCapabilities> {
        Ok(RemoteCapabilities::default())
    }

    async fn fetch_mod_manifest(&self, mod_id: &str) -> anyhow::Result<ModManifest>;
    async fn fetch_file(&self, mod_id: &str, rel_path: &RelPath) -> anyhow::Result<RemoteStream>;
    async fn fetch_file_range(
        &self,
        mod_id: &str,
        rel_path: &RelPath,
        range: FetchRange,
    ) -> anyhow::Result<RemoteStream>;
}

pub trait StateStore: Send + Sync {
    fn desired_state_get(&self) -> Result<Option<DesiredState>, StoreError>;

    fn expected_tmp_replace_all(
        &self,
        files: Vec<ExpectedFileRow>,
        parts: Vec<ExpectedPartRow>,
    ) -> Result<(), StoreError>;
    fn expected_tmp_load_files(&self) -> Result<Vec<ExpectedFileRow>, StoreError>;
    fn expected_tmp_load_parts(&self) -> Result<Vec<ExpectedPartRow>, StoreError>;

    fn expected_replace_all_v2(
        &self,
        state_id: &str,
        files: Vec<ExpectedFileRow>,
        parts: Vec<ExpectedPartRow>,
    ) -> Result<(), StoreError>;

    fn expected_load_v2(&self, state_id: &str) -> Result<Vec<ExpectedFileRow>, StoreError>;
    fn expected_parts_load_v1(&self, state_id: &str) -> Result<Vec<ExpectedPartRow>, StoreError>;

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

    fn observed_upsert_batch(
        &self,
        state_id: &str,
        rows: Vec<ObservedRow>,
    ) -> Result<(), StoreError>;
    fn observed_parts_upsert_batch(
        &self,
        state_id: &str,
        rows: Vec<ObservedPartRow>,
    ) -> Result<(), StoreError>;

    fn observed_get_all_for_mod_v2(
        &self,
        state_id: &str,
        mod_id: &str,
    ) -> Result<HashMap<String, ObservedRow>, StoreError>;

    fn observed_parts_get_all_for_file_v1(
        &self,
        state_id: &str,
        mod_id: &str,
        rel_path: &str,
    ) -> Result<Vec<ObservedPartRow>, StoreError>;

    fn verified_get(&self) -> Result<Option<VerifiedState>, StoreError>;
    fn verified_set(&self, state_id: &str, verified_at: TimestampNs) -> Result<(), StoreError>;
    fn verified_clear(&self) -> Result<(), StoreError>;
}
