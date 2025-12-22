use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use fleet_index::{DesiredState, FleetIndex};
use sync_engine::events::SyncEvent;
use sync_engine::fetch::{FileEntry, ModManifest};
use sync_engine::remote::{RemoteCapabilities, RemoteRepo, RemoteStream, RemoteStreamImpl};
use sync_engine::types::{RepairRequest, RepairTuning, VerifyRequest, VerifyTuning};

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

impl sync_engine::events::EventSink for TestSink {
    fn push(&self, ev: SyncEvent) {
        self.events.lock().unwrap().push(ev);
    }
}

#[derive(Clone)]
struct TestChecksummer;

impl sync_engine::types::Checksummer for TestChecksummer {
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
    let mut idx = setup_index(&enabled_mods);
    let sink = Arc::new(TestSink::default());

    let req = RepairRequest {
        repo_name: "repo".to_string(),
        checkout_root: PathBuf::from(root),
        enabled_mods,
        remote: Arc::new(remote.clone()),
        checksummer: Arc::new(TestChecksummer),
        tuning: RepairTuning::default(),
    };

    let outcome = sync_engine::flows::repair(req, &mut idx, sink).await.unwrap();
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
    let mut idx = setup_index(&enabled_mods);
    let sink = Arc::new(TestSink::default());

    let verify_req = VerifyRequest {
        repo_name: "repo".to_string(),
        checkout_root: PathBuf::from(root),
        enabled_mods: enabled_mods.clone(),
        remote: Arc::new(remote.clone()),
        checksummer: Arc::new(TestChecksummer),
        tuning: VerifyTuning::default(),
    };
    let report = sync_engine::flows::verify(verify_req, &mut idx, sink.clone())
        .await
        .unwrap();
    assert!(report.ok);

    let req = RepairRequest {
        repo_name: "repo".to_string(),
        checkout_root: PathBuf::from(root),
        enabled_mods,
        remote: Arc::new(remote.clone()),
        checksummer: Arc::new(TestChecksummer),
        tuning: RepairTuning::default(),
    };

    let outcome = sync_engine::flows::repair(req, &mut idx, sink).await.unwrap();
    assert!(outcome.report.skipped);
    assert_eq!(remote.fetch_file_calls.load(Ordering::Relaxed), 0);
    assert_eq!(remote.fetch_range_calls.load(Ordering::Relaxed), 0);
}
