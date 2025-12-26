use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use fleet_index::{DesiredState, FleetIndex};
use fleet_manifest::{ingest::ingest_mod_manifest, FetchRange, ModManifest, RelPath};
use fleet_sync::model::{
    CheckRequest, CheckTuning, EngineError, FileStateDelete, FileStateUpsert, RepairRequest,
    RepairTuning, StoreError, SyncFreshRequest, SyncFreshTuning, TimestampNs, UnknownPathPolicy,
};
use fleet_sync::ports::{
    EventSink, RemoteCapabilities, RemoteRepo, RemoteStream, RemoteStreamImpl, StateStore,
    SyncEvent,
};
use fleet_sync::SyncEngine;
use fleet_types::swifty::{checksums::mod_checksum_from_files, model as sw};
use relative_path::RelativePathBuf;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct TestSink {
    events: Mutex<Vec<SyncEvent>>,
}

impl EventSink for TestSink {
    fn push(&self, ev: SyncEvent) {
        self.events.lock().unwrap().push(ev);
    }
}

impl TestSink {
    fn events(&self) -> Vec<SyncEvent> {
        self.events.lock().unwrap().clone()
    }
}

struct VecStream {
    chunks: Vec<Bytes>,
    idx: usize,
    delay_ms: u64,
}

#[async_trait::async_trait]
impl RemoteStreamImpl for VecStream {
    async fn next_chunk(&mut self) -> anyhow::Result<Option<Bytes>> {
        if self.idx >= self.chunks.len() {
            return Ok(None);
        }
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        let out = self.chunks[self.idx].clone();
        self.idx += 1;
        Ok(Some(out))
    }
}

#[derive(Clone)]
struct CountingRemote {
    supports_ranges: bool,
    manifests: HashMap<String, ModManifest>,
    files: HashMap<(String, String), Vec<u8>>,
    delays_ms: u64,
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
        if self.delays_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delays_ms)).await;
        }
        self.manifests
            .get(mod_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("manifest not found"))
    }

    async fn fetch_file(&self, mod_id: &str, rel_path: &RelPath) -> anyhow::Result<RemoteStream> {
        self.fetch_file_calls.fetch_add(1, Ordering::Relaxed);
        let data = self
            .files
            .get(&(mod_id.to_string(), rel_path.as_str().to_string()))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("file not found"))?;
        let chunk = Bytes::from(data);
        Ok(RemoteStream::new(Box::new(VecStream {
            chunks: vec![chunk],
            idx: 0,
            delay_ms: self.delays_ms,
        })))
    }

    async fn fetch_file_range(
        &self,
        mod_id: &str,
        rel_path: &RelPath,
        range: FetchRange,
    ) -> anyhow::Result<RemoteStream> {
        self.fetch_range_calls.fetch_add(1, Ordering::Relaxed);
        let data = self
            .files
            .get(&(mod_id.to_string(), rel_path.as_str().to_string()))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("file not found"))?;
        let start = range.offset as usize;
        let end = range.end_exclusive() as usize;
        let slice = data
            .get(start..end)
            .ok_or_else(|| anyhow::anyhow!("range OOB"))?;

        // Drip bytes slowly to allow cancellation to interrupt.
        let chunks = slice.iter().map(|b| Bytes::from(vec![*b])).collect();
        Ok(RemoteStream::new(Box::new(VecStream {
            chunks,
            idx: 0,
            delay_ms: self.delays_ms.max(1),
        })))
    }
}

#[derive(Clone)]
struct Md5Checksummer;

impl fleet_sync::ports::Checksummer for Md5Checksummer {
    fn algorithm_name(&self) -> &str {
        "md5"
    }

    fn hash_file(&self, path: &std::path::Path) -> anyhow::Result<Vec<u8>> {
        let bytes = std::fs::read(path)?;
        Ok(md5::compute(&bytes).0.to_vec())
    }

    fn hash_range(&self, path: &std::path::Path, offset: u64, len: u64) -> anyhow::Result<Vec<u8>> {
        let bytes = std::fs::read(path)?;
        let start = offset as usize;
        let end = (offset + len) as usize;
        Ok(md5::compute(&bytes[start..end]).0.to_vec())
    }
}

fn make_parts(bytes: &[u8], part_size: usize) -> Vec<sw::PartManifest> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off < bytes.len() {
        let end = (off + part_size).min(bytes.len());
        out.push(sw::PartManifest {
            length: (end - off) as u64,
            start: off as u64,
            checksum: fleet_types::Md5Digest::from_bytes(md5::compute(&bytes[off..end]).0),
        });
        off = end;
    }
    out
}

fn build_manifest(
    mod_id: &str,
    rel_path: &str,
    bytes: &[u8],
    part_size: Option<usize>,
) -> ModManifest {
    let parts = part_size
        .map(|sz| make_parts(bytes, sz))
        .unwrap_or_default();
    let files = vec![sw::FileManifest {
        path: RelativePathBuf::from(rel_path),
        length: bytes.len() as u64,
        checksum: fleet_types::Md5Digest::from_bytes(md5::compute(bytes).0),
        parts,
    }];
    let checksum = mod_checksum_from_files(&files);
    let swifty = sw::ModManifest {
        name: mod_id.to_string(),
        checksum,
        files,
    };
    ingest_mod_manifest(swifty).unwrap()
}

fn build_empty_manifest(mod_id: &str) -> ModManifest {
    let files: Vec<sw::FileManifest> = Vec::new();
    let checksum = mod_checksum_from_files(&files);
    let swifty = sw::ModManifest {
        name: mod_id.to_string(),
        checksum,
        files,
    };
    ingest_mod_manifest(swifty).unwrap()
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
    fn desired_state_get(&self) -> Result<Option<fleet_sync::model::DesiredState>, StoreError> {
        let got = self
            .inner
            .lock()
            .unwrap()
            .get_desired_state()
            .map_err(|e| StoreError::Other(e.to_string()))?;
        Ok(got.map(|s| fleet_sync::model::DesiredState {
            state_id: s.state_id,
            enabled_mods_hash: s.enabled_mods_hash,
        }))
    }

    fn expected_replace_all_if_digest_changed(
        &self,
        state_id: &str,
        rows: Vec<fleet_sync::model::ExpectedFile>,
        digest_hex: &str,
    ) -> Result<(), StoreError> {
        let rows: Vec<fleet_index::ExpectedFile> = rows
            .into_iter()
            .map(|r| fleet_index::ExpectedFile {
                mod_id: r.mod_id,
                rel_path: r.rel_path,
                size: r.size,
            })
            .collect();
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

    fn expected_get_all(
        &self,
        state_id: &str,
    ) -> Result<Vec<fleet_sync::model::ExpectedFile>, StoreError> {
        let mut out = Vec::new();
        self.inner
            .lock()
            .unwrap()
            .expected_for_each(state_id, |row| {
                out.push(fleet_sync::model::ExpectedFile {
                    mod_id: row.mod_id,
                    rel_path: row.rel_path,
                    size: row.size,
                });
                Ok(())
            })
            .map_err(|e| StoreError::Other(e.to_string()))?;
        Ok(out)
    }

    fn file_state_get_all_for_mod(
        &self,
        state_id: &str,
        mod_id: &str,
    ) -> Result<std::collections::HashMap<String, fleet_sync::model::FileState>, StoreError> {
        let got = self
            .inner
            .lock()
            .unwrap()
            .file_state_get_all_for_mod(state_id, mod_id)
            .map_err(|e| StoreError::Other(e.to_string()))?;
        Ok(got
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    fleet_sync::model::FileState {
                        size: v.size,
                        mtime_ns: TimestampNs(v.mtime_ns),
                        checksum: v.checksum,
                    },
                )
            })
            .collect())
    }

    fn file_state_apply_batch(
        &self,
        state_id: &str,
        upserts: Vec<FileStateUpsert>,
        deletes: Vec<FileStateDelete>,
    ) -> Result<(), StoreError> {
        let up = upserts
            .into_iter()
            .map(|u| (u.mod_id, u.rel_path, u.size, u.mtime_ns.0, u.checksum));
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

    fn verified_get(&self) -> Result<Option<fleet_sync::model::VerifiedState>, StoreError> {
        let got = self
            .inner
            .lock()
            .unwrap()
            .verified_get()
            .map_err(|e| StoreError::Other(e.to_string()))?;
        Ok(got.map(|v| fleet_sync::model::VerifiedState {
            state_id: v.state_id,
            verified_at: TimestampNs(v.verified_at_ns),
        }))
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

#[tokio::test]
async fn sync_fresh_does_not_quarantine_expected_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let enabled_mods = vec!["@mod".to_string()];
    let idx = setup_index(&enabled_mods);
    let store = Arc::new(IndexStore::new(idx));

    let bytes = b"pbo-bytes".to_vec();
    let manifest = build_manifest("@mod", "addons/a.pbo", &bytes, None);

    // Create an unexpected file under expected directory prefix.
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(mod_root.join("addons")).unwrap();
    std::fs::write(mod_root.join("addons").join("junk.txt"), b"junk").unwrap();

    let remote = CountingRemote {
        supports_ranges: true,
        manifests: vec![("@mod".to_string(), manifest.clone())]
            .into_iter()
            .collect(),
        files: vec![(
            ("@mod".to_string(), "addons/a.pbo".to_string()),
            bytes.clone(),
        )]
        .into_iter()
        .collect(),
        delays_ms: 0,
        fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
        fetch_file_calls: Arc::new(AtomicUsize::new(0)),
        fetch_range_calls: Arc::new(AtomicUsize::new(0)),
    };

    let engine = SyncEngine::new(Arc::new(remote), store, Arc::new(Md5Checksummer));
    let sink = Arc::new(TestSink::default());
    let cancel = CancellationToken::new();

    let req = SyncFreshRequest {
        repo_name: "repo".to_string(),
        checkout_root: PathBuf::from(root),
        staging_root: PathBuf::from(root).join("_staging"),
        enabled_mods: enabled_mods.clone(),
        tuning: SyncFreshTuning {
            unknown_paths: UnknownPathPolicy::Delete,
            ..Default::default()
        },
    };

    let out = engine
        .sync_fresh(req, sink.as_ref(), &cancel)
        .await
        .unwrap();
    assert!(out.ok());

    // Expected file remains in-place (its directory prefix must not be quarantined).
    assert!(root.join("@mod").join("addons").join("a.pbo").exists());
    // Unexpected file was quarantined or removed, but must not break expected dir.
    assert!(!root.join("@mod").join("addons").join("junk.txt").exists());
}

#[tokio::test]
async fn cancellation_during_patch_does_not_commit_partial_results() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let enabled_mods = vec!["@mod".to_string()];
    let idx = setup_index(&enabled_mods);
    let store = Arc::new(IndexStore::new(idx));

    let part_size = 16;
    let remote_bytes: Vec<u8> = (0..128u32).map(|i| (i % 251) as u8).collect();
    let mut local_bytes = remote_bytes.clone();
    local_bytes[3] ^= 0xFF;

    let manifest = build_manifest("@mod", "addons/a.pbo", &remote_bytes, Some(part_size));

    let mod_root = root.join("@mod").join("addons");
    std::fs::create_dir_all(&mod_root).unwrap();
    let target = mod_root.join("a.pbo");
    std::fs::write(&target, &local_bytes).unwrap();

    let remote = CountingRemote {
        supports_ranges: true,
        manifests: vec![("@mod".to_string(), manifest.clone())]
            .into_iter()
            .collect(),
        files: vec![(
            ("@mod".to_string(), "addons/a.pbo".to_string()),
            remote_bytes.clone(),
        )]
        .into_iter()
        .collect(),
        delays_ms: 2,
        fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
        fetch_file_calls: Arc::new(AtomicUsize::new(0)),
        fetch_range_calls: Arc::new(AtomicUsize::new(0)),
    };

    let engine = SyncEngine::new(Arc::new(remote), store, Arc::new(Md5Checksummer));
    let sink = Arc::new(TestSink::default());
    let cancel = CancellationToken::new();

    let tuning = RepairTuning {
        patch_max_bad_ratio: 1.0,
        patch_max_fetch_ratio: 1.0,
        patch_max_bad_parts: None,
        patch_max_range_requests: Some(64),
        ..Default::default()
    };

    let req = RepairRequest {
        repo_name: "repo".to_string(),
        checkout_root: PathBuf::from(root),
        staging_root: PathBuf::from(root).join("_staging"),
        enabled_mods,
        tuning,
    };

    let task = {
        let cancel = cancel.clone();
        let engine = engine;
        let sink = sink.clone();
        tokio::spawn(async move { engine.repair(req, sink.as_ref(), &cancel).await })
    };

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    cancel.cancel();
    let res = task.await.unwrap();
    assert!(matches!(res, Err(EngineError::Cancelled)));

    // Final file must not have been fixed/committed after cancellation.
    let final_bytes = std::fs::read(&target).unwrap();
    assert_eq!(final_bytes, local_bytes);

    // No verified events after cancel.
    let verified = sink
        .events()
        .into_iter()
        .any(|e| matches!(e, SyncEvent::FileVerified { .. }));
    assert!(!verified);
}

#[tokio::test]
async fn staging_tmp_files_are_cleaned_up_on_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let enabled_mods = vec!["@mod".to_string()];
    let idx = setup_index(&enabled_mods);
    let store = Arc::new(IndexStore::new(idx));

    let bytes = b"content".to_vec();
    let manifest = build_manifest("@mod", "file.bin", &bytes, None);

    // Remote has manifest but no file bytes -> stages then fails during download.
    let remote = CountingRemote {
        supports_ranges: true,
        manifests: vec![("@mod".to_string(), manifest.clone())]
            .into_iter()
            .collect(),
        files: HashMap::new(),
        delays_ms: 0,
        fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
        fetch_file_calls: Arc::new(AtomicUsize::new(0)),
        fetch_range_calls: Arc::new(AtomicUsize::new(0)),
    };

    let engine = SyncEngine::new(Arc::new(remote), store, Arc::new(Md5Checksummer));
    let sink = Arc::new(TestSink::default());
    let cancel = CancellationToken::new();

    let req = RepairRequest {
        repo_name: "repo".to_string(),
        checkout_root: PathBuf::from(root),
        staging_root: PathBuf::from(root).join("_staging"),
        enabled_mods,
        tuning: RepairTuning {
            patch_max_bad_ratio: 0.0,
            patch_max_fetch_ratio: 0.0,
            ..Default::default()
        },
    };

    let out = engine.repair(req, sink.as_ref(), &cancel).await.unwrap();
    assert!(!out.ok());

    // No staged temp file litter under @mod.
    let mod_root = root.join("@mod");
    if mod_root.exists() {
        for entry in walkdir::WalkDir::new(&mod_root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            let name = entry.file_name().to_string_lossy();
            assert!(
                !name.contains(".fleet.tmp."),
                "found tmp litter: {}",
                entry.path().display()
            );
        }
    }
}

#[tokio::test]
async fn fetch_all_respects_cancellation_and_stops_scheduling() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let enabled_mods: Vec<String> = (0..50).map(|i| format!("@m{i}")).collect();
    let idx = setup_index(&enabled_mods);
    let store = Arc::new(IndexStore::new(idx));

    let mut manifests = HashMap::new();
    for m in &enabled_mods {
        manifests.insert(m.clone(), build_empty_manifest(m));
    }

    let remote = CountingRemote {
        supports_ranges: true,
        manifests,
        files: HashMap::new(),
        delays_ms: 20,
        fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
        fetch_file_calls: Arc::new(AtomicUsize::new(0)),
        fetch_range_calls: Arc::new(AtomicUsize::new(0)),
    };

    let engine = SyncEngine::new(Arc::new(remote.clone()), store, Arc::new(Md5Checksummer));
    let sink = Arc::new(TestSink::default());
    let cancel = CancellationToken::new();

    let req = CheckRequest {
        repo_name: "repo".to_string(),
        checkout_root: PathBuf::from(root),
        enabled_mods: enabled_mods.clone(),
        tuning: CheckTuning {
            scan_concurrency: 1,
            ..Default::default()
        },
    };

    let task = {
        let cancel = cancel.clone();
        let sink = sink.clone();
        tokio::spawn(async move { engine.check(req, sink.as_ref(), &cancel).await })
    };

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    cancel.cancel();
    let res = task.await.unwrap();
    assert!(matches!(res, Err(EngineError::Cancelled)));

    // With scan_concurrency=1, cancellation should prevent scheduling most fetches.
    assert!(remote.fetch_manifest_calls.load(Ordering::Relaxed) < enabled_mods.len());
}
