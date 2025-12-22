use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use fleet_index::{DesiredState, FleetIndex};
use sync_engine::ports::SyncEvent;
use sync_engine::ports::{RemoteCapabilities, RemoteRepo, RemoteStream, RemoteStreamImpl, StateStore};
use sync_engine::ports::{FileEntry, ModManifest};
use sync_engine::model::{
    CheckRequest, CheckTuning, FileStateDelete, FileStateUpsert, RepairRequest, RepairTuning,
    StoreError, TimestampNs,
};
use sync_engine::SyncEngine;

struct VecStream {
    data: Vec<u8>,
    pos: usize,
}

#[async_trait::async_trait]
impl RemoteStreamImpl for VecStream {
    async fn next_chunk(&mut self) -> anyhow::Result<Option<Bytes>> {
        if self.pos >= self.data.len() {
            return Ok(None);
        }
        let end = (self.pos + 1024).min(self.data.len());
        let chunk = Bytes::copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        Ok(Some(chunk))
    }
}

#[derive(Clone)]
struct CountingRemote {
    supports_ranges: bool,
    manifests: HashMap<String, ModManifest>,
    files: HashMap<(String, String), Vec<u8>>,
    fetch_manifest_calls: Arc<AtomicUsize>,
    fetch_file_calls: Arc<AtomicUsize>,
    fetch_range_calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl RemoteRepo for CountingRemote {
    async fn capabilities(&self) -> anyhow::Result<RemoteCapabilities> {
        Ok(RemoteCapabilities {
            supports_ranges: self.supports_ranges,
        })
    }

    async fn fetch_mod_manifest(&self, mod_id: &str) -> anyhow::Result<ModManifest> {
        self.fetch_manifest_calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.manifests.get(mod_id).unwrap().clone())
    }

    async fn fetch_file(&self, mod_id: &str, rel_path: &str) -> anyhow::Result<RemoteStream> {
        self.fetch_file_calls.fetch_add(1, Ordering::Relaxed);
        let data = self
            .files
            .get(&(mod_id.to_string(), rel_path.to_string()))
            .unwrap();
        Ok(RemoteStream::new(Box::new(VecStream {
            data: data.clone(),
            pos: 0,
        })))
    }

    async fn fetch_range(
        &self,
        mod_id: &str,
        rel_path: &str,
        offset: u64,
        len: u64,
    ) -> anyhow::Result<RemoteStream> {
        self.fetch_range_calls.fetch_add(1, Ordering::Relaxed);
        let data = self
            .files
            .get(&(mod_id.to_string(), rel_path.to_string()))
            .unwrap();
        let start = offset as usize;
        let end = (offset + len) as usize;
        Ok(RemoteStream::new(Box::new(VecStream {
            data: data[start..end].to_vec(),
            pos: 0,
        })))
    }
}

#[derive(Default)]
struct TestSink {
    events: Mutex<Vec<SyncEvent>>,
}

impl sync_engine::EventSink for TestSink {
    fn push(&self, ev: SyncEvent) {
        self.events.lock().unwrap().push(ev);
    }
}

#[derive(Clone)]
struct TestChecksummer;

impl sync_engine::Checksummer for TestChecksummer {
    fn algorithm_name(&self) -> &str {
        "blake3"
    }

    fn hash_file(&self, path: &std::path::Path) -> anyhow::Result<Vec<u8>> {
        let data = std::fs::read(path)?;
        Ok(blake3::hash(&data).as_bytes().to_vec())
    }

    fn hash_range(&self, path: &std::path::Path, offset: u64, len: u64) -> anyhow::Result<Vec<u8>> {
        let data = std::fs::read(path)?;
        let start = offset as usize;
        let end = (offset + len) as usize;
        Ok(blake3::hash(&data[start..end]).as_bytes().to_vec())
    }
}

fn setup_index(enabled_mods: &[String]) -> FleetIndex {
    let mut enabled_sorted = enabled_mods.to_vec();
    enabled_sorted.sort();
    let repo_id = fleet_index::normalize_repo_id("abcd");
    let repo_revision = "rev1".to_string();
    let enabled_hash = fleet_index::enabled_mods_hash(&enabled_sorted);
    let state_id = fleet_index::state_id(&repo_id, &enabled_hash, &repo_revision);

    let mut idx = FleetIndex::open_in_memory().unwrap();
    idx.set_desired_state(DesiredState {
        repo_url: "http://example".to_string(),
        repo_id,
        repo_revision,
        enabled_mods_hash: enabled_hash,
        state_id,
        updated_at_unix_s: 1,
    })
    .unwrap();
    idx
}

struct IndexStore {
    inner: Mutex<FleetIndex>,
}

impl IndexStore {
    fn new(idx: FleetIndex) -> Self {
        Self {
            inner: Mutex::new(idx),
        }
    }
}

impl StateStore for IndexStore {
    fn desired_state_get(&self) -> Result<Option<sync_engine::model::DesiredState>, StoreError> {
        self.inner
            .lock()
            .unwrap()
            .get_desired_state()
            .map_err(|e| StoreError::Other(e.to_string()))
    }

    fn expected_replace_all_if_digest_changed(
        &self,
        state_id: &str,
        rows: Vec<sync_engine::model::ExpectedFile>,
        digest_hex: &str,
    ) -> Result<(), StoreError> {
        self.inner
            .lock()
            .unwrap()
            .expected_replace_all_if_digest_changed(state_id, rows, digest_hex)
            .map(|_| ())
            .map_err(|e| StoreError::Other(e.to_string()))
    }

    fn baseline_exists(&self, state_id: &str) -> Result<bool, StoreError> {
        self.inner
            .lock()
            .unwrap()
            .baseline_exists(state_id)
            .map_err(|e| StoreError::Other(e.to_string()))
    }

    fn expected_get_all(&self, state_id: &str) -> Result<Vec<sync_engine::model::ExpectedFile>, StoreError> {
        let mut out = Vec::new();
        self.inner
            .lock()
            .unwrap()
            .expected_for_each(state_id, |row| {
                out.push(row);
                Ok(())
            })
            .map_err(|e| StoreError::Other(e.to_string()))?;
        Ok(out)
    }

    fn file_state_get_all_for_mod(
        &self,
        state_id: &str,
        mod_id: &str,
    ) -> Result<std::collections::HashMap<String, sync_engine::model::FileState>, StoreError> {
        self.inner
            .lock()
            .unwrap()
            .file_state_get_all_for_mod(state_id, mod_id)
            .map_err(|e| StoreError::Other(e.to_string()))
    }

    fn file_state_apply_batch(
        &self,
        state_id: &str,
        upserts: Vec<FileStateUpsert>,
        deletes: Vec<FileStateDelete>,
    ) -> Result<(), StoreError> {
        let up = upserts.into_iter().map(|u| {
            (
                u.mod_id,
                u.rel_path,
                u.size,
                u.mtime_ns.0,
                u.checksum,
            )
        });
        let del = deletes.into_iter().map(|d| (d.mod_id, d.rel_path));
        self.inner
            .lock()
            .unwrap()
            .file_state_apply_batch(state_id, up, del)
            .map_err(|e| StoreError::Other(e.to_string()))
    }

    fn file_state_delete(
        &self,
        state_id: &str,
        mod_id: &str,
        rel_path: &str,
    ) -> Result<(), StoreError> {
        self.inner
            .lock()
            .unwrap()
            .file_state_delete(state_id, mod_id, rel_path)
            .map_err(|e| StoreError::Other(e.to_string()))
    }

    fn verified_get(&self) -> Result<Option<sync_engine::model::VerifiedState>, StoreError> {
        self.inner
            .lock()
            .unwrap()
            .verified_get()
            .map_err(|e| StoreError::Other(e.to_string()))
    }

    fn verified_set(&self, state_id: &str, verified_at: TimestampNs) -> Result<(), StoreError> {
        self.inner
            .lock()
            .unwrap()
            .verified_set(state_id, verified_at.0)
            .map_err(|e| StoreError::Other(e.to_string()))
    }

    fn verified_clear(&self) -> Result<(), StoreError> {
        self.inner
            .lock()
            .unwrap()
            .verified_clear()
            .map_err(|e| StoreError::Other(e.to_string()))
    }
}

fn build_manifest(mod_id: &str, rel_path: &str, bytes: &[u8]) -> ModManifest {
    ModManifest {
        mod_id: mod_id.to_string(),
        files: vec![FileEntry {
            rel_path: rel_path.to_string(),
            size: bytes.len() as u64,
            file_checksum: blake3::hash(bytes).as_bytes().to_vec(),
            parts: Vec::new(),
        }],
    }
}

#[tokio::test]
async fn safety_abort_does_not_fetch_remote_file_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    // Make an unsafe on-disk layout: expected path's ancestor is a symlink.
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(std::env::temp_dir(), mod_root.join("addons")).unwrap();
    }

    let bytes = b"pbo".to_vec();
    let manifest = build_manifest("@mod", "addons/a.pbo", &bytes);

    let remote = CountingRemote {
        supports_ranges: true,
        manifests: vec![("@mod".to_string(), manifest.clone())].into_iter().collect(),
        files: vec![(("@mod".to_string(), "addons/a.pbo".to_string()), bytes)]
            .into_iter()
            .collect(),
        fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
        fetch_file_calls: Arc::new(AtomicUsize::new(0)),
        fetch_range_calls: Arc::new(AtomicUsize::new(0)),
    };

    let enabled_mods = vec!["@mod".to_string()];
    let idx = setup_index(&enabled_mods);
    let store = Arc::new(IndexStore::new(idx));
    let engine = SyncEngine::new(
        Arc::new(remote.clone()),
        store,
        Arc::new(TestChecksummer),
    );
    let sink = Arc::new(TestSink::default());

    let req = RepairRequest {
        repo_name: "repo".to_string(),
        checkout_root: PathBuf::from(root),
        enabled_mods,
        tuning: RepairTuning::default(),
    };

    let outcome = engine.repair(req, sink.as_ref()).await.unwrap();
    assert!(outcome.aborted.is_some());
    assert_eq!(remote.fetch_file_calls.load(Ordering::Relaxed), 0);
    assert_eq!(remote.fetch_range_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn skip_logic_does_not_fetch_remote_file_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    let bytes = b"content".to_vec();
    std::fs::write(mod_root.join("file.bin"), &bytes).unwrap();

    let manifest = build_manifest("@mod", "file.bin", &bytes);

    let remote = CountingRemote {
        supports_ranges: true,
        manifests: vec![("@mod".to_string(), manifest.clone())].into_iter().collect(),
        files: vec![(("@mod".to_string(), "file.bin".to_string()), bytes)]
            .into_iter()
            .collect(),
        fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
        fetch_file_calls: Arc::new(AtomicUsize::new(0)),
        fetch_range_calls: Arc::new(AtomicUsize::new(0)),
    };

    let enabled_mods = vec!["@mod".to_string()];
    let idx = setup_index(&enabled_mods);
    let store = Arc::new(IndexStore::new(idx));
    let engine = SyncEngine::new(
        Arc::new(remote.clone()),
        store,
        Arc::new(TestChecksummer),
    );
    let sink = Arc::new(TestSink::default());

    let check_req = CheckRequest {
        repo_name: "repo".to_string(),
        checkout_root: PathBuf::from(root),
        enabled_mods: enabled_mods.clone(),
        tuning: CheckTuning::default(),
    };
    let report = engine.check(check_req, sink.as_ref()).await.unwrap();
    assert!(report.ok);

    let req = RepairRequest {
        repo_name: "repo".to_string(),
        checkout_root: PathBuf::from(root),
        enabled_mods,
        tuning: RepairTuning::default(),
    };

    let outcome = engine.repair(req, sink.as_ref()).await.unwrap();
    assert!(outcome.report.skipped);
    assert_eq!(remote.fetch_file_calls.load(Ordering::Relaxed), 0);
    assert_eq!(remote.fetch_range_calls.load(Ordering::Relaxed), 0);
}
