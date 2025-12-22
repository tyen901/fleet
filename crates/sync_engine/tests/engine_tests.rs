use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use sync_engine::apply::{apply_ops, ApplyOptions};
use sync_engine::events::{EventSink, SyncEvent};
use sync_engine::fetch::fetch_all;
use sync_engine::fetch::{FileEntry, FilePart, ModManifest};
use sync_engine::manifest::{validate_and_normalize_manifest, ValidatedModManifest};
use sync_engine::plan::{plan_mod, FileTarget, PlannedOp, RepairStrategy};
use sync_engine::quarantine::quarantine_unexpected;
use sync_engine::remote::{RemoteCapabilities, RemoteRepo, RemoteStream, RemoteStreamImpl};
use sync_engine::types::VerifyIssueKind;
use sync_engine::types::{Checksummer, RepairRequest, RepairTuning, VerifyRequest, VerifyTuning};

use fleet_index::{DesiredState, FleetIndex};

#[derive(Clone)]
struct TestChecksummer;

impl Checksummer for TestChecksummer {
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
struct FakeRemote {
    supports_ranges: bool,
    manifests: HashMap<String, ModManifest>,
    files: HashMap<(String, String), Vec<u8>>,
    fetch_manifest_calls: Arc<AtomicUsize>,
    fetch_file_calls: Arc<AtomicUsize>,
    fetch_range_calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl RemoteRepo for FakeRemote {
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

impl EventSink for TestSink {
    fn push(&self, ev: SyncEvent) {
        self.events.lock().unwrap().push(ev);
    }
}

fn build_parts(data: &[u8], part_size: usize) -> Vec<FilePart> {
    let mut parts = Vec::new();
    let mut offset = 0u64;
    while (offset as usize) < data.len() {
        let end = ((offset as usize) + part_size).min(data.len());
        let chunk = &data[offset as usize..end];
        let checksum = blake3::hash(chunk).as_bytes().to_vec();
        parts.push(FilePart {
            offset,
            len: chunk.len() as u64,
            checksum,
        });
        offset += chunk.len() as u64;
    }
    parts
}

fn build_manifest(mod_id: &str, rel_path: &str, data: &[u8], part_size: usize) -> ModManifest {
    let parts = build_parts(data, part_size);
    let file_checksum = blake3::hash(data).as_bytes().to_vec();
    ModManifest {
        mod_id: mod_id.to_string(),
        files: vec![FileEntry {
            rel_path: rel_path.to_string(),
            size: data.len() as u64,
            file_checksum,
            parts,
        }],
    }
}

fn build_validated_manifest(
    mod_id: &str,
    rel_path: &str,
    data: &[u8],
    part_size: usize,
) -> ValidatedModManifest {
    validate_and_normalize_manifest(build_manifest(mod_id, rel_path, data, part_size)).unwrap()
}

fn mtime_ns(path: &std::path::Path) -> i64 {
    let md = std::fs::metadata(path).unwrap();
    md.modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64
}

#[test]
fn planner_cache_hit_yields_skip() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    let data = b"hello world".to_vec();
    let file_path = mod_root.join("file.bin");
    std::fs::write(&file_path, &data).unwrap();

    let manifest = build_validated_manifest("@mod", "file.bin", &data, 4);
    let checksum = manifest.files[0].file_checksum.clone();
    let cache_state = fleet_index::FileState {
        size: data.len() as u64,
        mtime_ns: mtime_ns(&file_path),
        checksum,
    };

    let mut cache = HashMap::new();
    cache.insert("file.bin".to_string(), cache_state);

    let tuning = RepairTuning::default();
    let checksummer = TestChecksummer;
    let (plan, _hints) = plan_mod(root, &manifest, &cache, true, &tuning, &checksummer).unwrap();

    assert_eq!(plan.ops[0].target.strategy, RepairStrategy::Skip);
}

#[test]
#[cfg(unix)]
fn planner_symlink_triggers_full() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    let real = mod_root.join("real.bin");
    std::fs::write(&real, b"data").unwrap();

    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, mod_root.join("file.bin")).unwrap();

    let manifest = build_validated_manifest("@mod", "file.bin", b"data", 4);
    let checksummer = TestChecksummer;
    let (plan, _hints) = plan_mod(
        root,
        &manifest,
        &HashMap::new(),
        true,
        &RepairTuning::default(),
        &checksummer,
    )
    .unwrap();

    assert_eq!(plan.ops[0].target.strategy, RepairStrategy::Full);
}

#[test]
fn planner_strategy_selection_respects_ratio_and_part_caps() {
    struct Case {
        name: &'static str,
        part_size: usize,
        corrupt_prefix_bytes: usize,
        tuning: RepairTuning,
        expect: RepairStrategy,
    }

    let cases = vec![
        Case {
            name: "small corruption -> Patch (ratio under threshold)",
            part_size: 10,
            corrupt_prefix_bytes: 10, // 10/100 = 0.10
            tuning: RepairTuning {
                patch_max_bad_ratio: 0.3,
                patch_min_range_bytes: 0,
                patch_max_fetch_ratio: 1.0,
                ..Default::default()
            },
            expect: RepairStrategy::Patch,
        },
        Case {
            name: "large corruption -> Full (ratio over threshold)",
            part_size: 10,
            corrupt_prefix_bytes: 60, // 60/100 = 0.60
            tuning: RepairTuning {
                patch_max_bad_ratio: 0.3,
                patch_min_range_bytes: 0,
                patch_max_fetch_ratio: 1.0,
                ..Default::default()
            },
            expect: RepairStrategy::Full,
        },
        Case {
            name: "too many bad parts -> Full (part cap)",
            part_size: 5,             // 100 bytes => 20 parts
            corrupt_prefix_bytes: 25, // corrupts 5 parts
            tuning: RepairTuning {
                patch_max_bad_ratio: 0.9,
                patch_max_bad_parts: Some(1),
                patch_min_range_bytes: 0,
                patch_max_fetch_ratio: 1.0,
                ..Default::default()
            },
            expect: RepairStrategy::Full,
        },
    ];

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();
    let file_path = mod_root.join("file.bin");

    let data = vec![1u8; 100];
    let checksummer = TestChecksummer;
    let cache = HashMap::new();

    for case in cases {
        let manifest = build_validated_manifest("@mod", "file.bin", &data, case.part_size);

        let mut corrupted = data.clone();
        for b in corrupted.iter_mut().take(case.corrupt_prefix_bytes) {
            *b = b.wrapping_add(1);
        }
        std::fs::write(&file_path, &corrupted).unwrap();

        let (plan, _hints) =
            plan_mod(root, &manifest, &cache, true, &case.tuning, &checksummer).unwrap();

        assert_eq!(
            plan.ops[0].target.strategy, case.expect,
            "case: {}",
            case.name
        );
    }
}
#[tokio::test]
async fn applier_atomic_replace_handles_existing_targets() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    let data = b"content".to_vec();
    let manifest = build_manifest("@mod", "file.bin", &data, 4);
    let entry = &manifest.files[0];

    let mut files = HashMap::new();
    files.insert(("@mod".to_string(), "file.bin".to_string()), data.clone());

    let remote = FakeRemote {
        supports_ranges: true,
        manifests: vec![("@mod".to_string(), manifest.clone())]
            .into_iter()
            .collect(),
        files,
        fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
        fetch_file_calls: Arc::new(AtomicUsize::new(0)),
        fetch_range_calls: Arc::new(AtomicUsize::new(0)),
    };

    let target = FileTarget {
        size: entry.size,
        file_checksum: entry.file_checksum.clone(),
        parts: entry.parts.clone(),
        strategy: RepairStrategy::Full,
        parts_to_fetch: entry.parts.clone(),
    };

    let op = PlannedOp {
        mod_id: "@mod".to_string(),
        rel_path: "file.bin".to_string(),
        abs_path: mod_root.join("file.bin"),
        target,
        estimated_bytes: entry.size,
    };

    // Existing file
    std::fs::write(mod_root.join("file.bin"), b"old").unwrap();
    let req = RepairRequest {
        repo_name: "repo".to_string(),
        checkout_root: root.to_path_buf(),
        enabled_mods: vec!["@mod".to_string()],
        remote: Arc::new(remote.clone()),
        checksummer: Arc::new(TestChecksummer),
        tuning: RepairTuning::default(),
    };
    let sink = Arc::new(TestSink::default());
    let result = apply_ops(
        vec![op.clone()],
        &req,
        sink.clone(),
        ApplyOptions {
            supports_ranges: true,
        },
    )
    .await
    .unwrap();
    assert!(result.failures.is_empty());
    assert_eq!(result.report.files_downloaded, 1);
    assert_eq!(std::fs::read(mod_root.join("file.bin")).unwrap(), data);

    // Existing directory
    std::fs::remove_file(mod_root.join("file.bin")).unwrap();
    std::fs::create_dir_all(mod_root.join("file.bin")).unwrap();
    let _result = apply_ops(
        vec![op.clone()],
        &req,
        sink.clone(),
        ApplyOptions {
            supports_ranges: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(mod_root.join("file.bin")).unwrap(), data);

    // Existing symlink
    std::fs::remove_file(mod_root.join("file.bin")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(mod_root.join("file.bin.back"), mod_root.join("file.bin"))
            .unwrap();
        std::fs::write(mod_root.join("file.bin.back"), b"old").unwrap();
        let _result = apply_ops(
            vec![op.clone()],
            &req,
            sink.clone(),
            ApplyOptions {
                supports_ranges: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(mod_root.join("file.bin")).unwrap(), data);
    }
}

#[tokio::test]
async fn patch_falls_back_to_full_when_baseline_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    let data = b"content".to_vec();
    let manifest = build_manifest("@mod", "file.bin", &data, 4);
    let entry = &manifest.files[0];

    let mut files = HashMap::new();
    files.insert(("@mod".to_string(), "file.bin".to_string()), data.clone());

    let remote = FakeRemote {
        supports_ranges: true,
        manifests: vec![("@mod".to_string(), manifest.clone())]
            .into_iter()
            .collect(),
        files,
        fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
        fetch_file_calls: Arc::new(AtomicUsize::new(0)),
        fetch_range_calls: Arc::new(AtomicUsize::new(0)),
    };

    let target = FileTarget {
        size: entry.size,
        file_checksum: entry.file_checksum.clone(),
        parts: entry.parts.clone(),
        strategy: RepairStrategy::Patch,
        parts_to_fetch: entry.parts.clone(),
    };

    let op = PlannedOp {
        mod_id: "@mod".to_string(),
        rel_path: "file.bin".to_string(),
        abs_path: mod_root.join("file.bin"),
        target,
        estimated_bytes: entry.size,
    };

    let req = RepairRequest {
        repo_name: "repo".to_string(),
        checkout_root: root.to_path_buf(),
        enabled_mods: vec!["@mod".to_string()],
        remote: Arc::new(remote.clone()),
        checksummer: Arc::new(TestChecksummer),
        tuning: RepairTuning::default(),
    };
    let sink = Arc::new(TestSink::default());
    let result = apply_ops(
        vec![op],
        &req,
        sink.clone(),
        ApplyOptions {
            supports_ranges: true,
        },
    )
    .await
    .unwrap();
    assert!(result.failures.is_empty());
    assert_eq!(result.report.files_downloaded, 1);
    assert_eq!(result.report.files_patched, 0);
}

#[tokio::test]
async fn quarantine_ignores_symlinks_and_quarantines_unexpected() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    std::fs::write(mod_root.join("expected.txt"), b"ok").unwrap();
    std::fs::write(mod_root.join("extra.txt"), b"bad").unwrap();
    std::fs::create_dir_all(mod_root.join("extra_dir")).unwrap();
    std::fs::write(mod_root.join("extra_dir/file.bin"), b"bad").unwrap();

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(mod_root.join("extra.txt"), mod_root.join("link.txt")).unwrap();
    }

    let mut expected = HashSet::new();
    expected.insert("expected.txt".to_string());

    let tuning = RepairTuning {
        delete_empty_dirs: true,
        ..Default::default()
    };

    let sink = Arc::new(TestSink::default());
    let stats = quarantine_unexpected(root, "@mod", &expected, &tuning, sink.clone())
        .await
        .unwrap();

    assert!(stats.files >= 1);
    assert!(!mod_root.join("extra.txt").exists());
    assert!(!mod_root.join("extra_dir").exists());

    #[cfg(unix)]
    {
        let md = std::fs::symlink_metadata(mod_root.join("link.txt")).unwrap();
        assert!(md.file_type().is_symlink());
    }
}

#[tokio::test]
async fn quarantine_respects_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    let big_path = mod_root.join("big.bin");
    std::fs::write(&big_path, vec![0u8; 32]).unwrap();

    let expected = HashSet::new();
    let tuning = RepairTuning {
        max_quarantine_bytes: Some(16),
        ..Default::default()
    };

    let sink = Arc::new(TestSink::default());
    let stats = quarantine_unexpected(root, "@mod", &expected, &tuning, sink.clone())
        .await
        .unwrap();

    assert_eq!(stats.files, 0);
    assert!(big_path.exists());
}

#[tokio::test]
async fn quarantine_deletes_empty_expected_prefix_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    let expected_dir = mod_root.join("expected");
    std::fs::create_dir_all(&expected_dir).unwrap();

    let mut expected = HashSet::new();
    expected.insert("expected/file.bin".to_string());

    let tuning = RepairTuning {
        delete_empty_dirs: true,
        ..Default::default()
    };

    let sink = Arc::new(TestSink::default());
    let stats = quarantine_unexpected(root, "@mod", &expected, &tuning, sink.clone())
        .await
        .unwrap();

    assert_eq!(stats.empty_dirs_deleted, 1);
    assert!(!expected_dir.exists());
}

#[tokio::test]
async fn patch_event_totals_are_consistent() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    // Use the real planner so `parts_to_fetch` reflects *only* the bad parts.
    let data = vec![b'A'; 100];
    let manifest = build_manifest("@mod", "file.bin", &data, 10);
    let validated = validate_and_normalize_manifest(manifest.clone()).unwrap();

    let mut corrupted = data.clone();
    corrupted[0] = b'X'; // corrupt exactly one part (offset 0..10)
    std::fs::write(mod_root.join("file.bin"), &corrupted).unwrap();

    let tuning = RepairTuning {
        patch_max_bad_ratio: 0.5,
        patch_min_range_bytes: 0,
        patch_max_fetch_ratio: 1.0,
        ..Default::default()
    };
    let checksummer = TestChecksummer;
    let cache = HashMap::new();
    let (plan, _hints) = plan_mod(root, &validated, &cache, true, &tuning, &checksummer).unwrap();
    assert_eq!(plan.ops.len(), 1);
    assert_eq!(plan.ops[0].target.strategy, RepairStrategy::Patch);
    assert_eq!(
        plan.ops[0].target.parts_to_fetch.len(),
        1,
        "expected exactly one bad part to fetch"
    );
    let expected_range_calls = plan.ops[0].target.parts_to_fetch.len();

    let mut files = HashMap::new();
    files.insert(("@mod".to_string(), "file.bin".to_string()), data.clone());

    let remote = FakeRemote {
        supports_ranges: true,
        manifests: vec![("@mod".to_string(), manifest.clone())]
            .into_iter()
            .collect(),
        files,
        fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
        fetch_file_calls: Arc::new(AtomicUsize::new(0)),
        fetch_range_calls: Arc::new(AtomicUsize::new(0)),
    };
    let remote_arc = Arc::new(remote.clone());

    let req = RepairRequest {
        repo_name: "repo".to_string(),
        checkout_root: root.to_path_buf(),
        enabled_mods: vec!["@mod".to_string()],
        remote: remote_arc,
        checksummer: Arc::new(TestChecksummer),
        tuning: RepairTuning::default(),
    };
    let sink = Arc::new(TestSink::default());
    let outcome = apply_ops(
        plan.ops,
        &req,
        sink.clone(),
        ApplyOptions {
            supports_ranges: true,
        },
    )
    .await
    .unwrap();
    assert!(outcome.failures.is_empty());

    // Patch path should use ranges (not full-file fetch).
    assert_eq!(
        remote.fetch_file_calls.load(Ordering::Relaxed),
        0,
        "patch should not fetch full file"
    );
    assert_eq!(
        remote.fetch_range_calls.load(Ordering::Relaxed),
        expected_range_calls,
        "unexpected number of fetch_range calls"
    );

    let events = sink.events.lock().unwrap();
    let mut started_total = None;
    let mut progress_total = None;
    for ev in events.iter() {
        match ev {
            SyncEvent::FileStarted { bytes_total, .. } => started_total = Some(*bytes_total),
            SyncEvent::FileProgress { bytes_total, .. } => progress_total = Some(*bytes_total),
            _ => {}
        }
    }
    assert_eq!(started_total, progress_total);
}

#[tokio::test]
async fn patch_falls_back_to_full_when_remote_lacks_range_support() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    // Plan for Patch, but apply against a remote that does not support ranges.
    let data = vec![b'Z'; 100];
    let manifest = build_manifest("@mod", "file.bin", &data, 10);
    let validated = validate_and_normalize_manifest(manifest.clone()).unwrap();

    let mut corrupted = data.clone();
    corrupted[0] = b'Y';
    std::fs::write(mod_root.join("file.bin"), &corrupted).unwrap();

    let tuning = RepairTuning {
        patch_max_bad_ratio: 0.9,
        patch_min_range_bytes: 0,
        patch_max_fetch_ratio: 1.0,
        ..Default::default()
    };
    let checksummer = TestChecksummer;
    let cache = HashMap::new();
    let (plan, _hints) = plan_mod(root, &validated, &cache, true, &tuning, &checksummer).unwrap();
    assert_eq!(plan.ops.len(), 1);
    assert_eq!(plan.ops[0].target.strategy, RepairStrategy::Patch);

    let mut files = HashMap::new();
    files.insert(("@mod".to_string(), "file.bin".to_string()), data.clone());

    let remote = FakeRemote {
        supports_ranges: false,
        manifests: vec![("@mod".to_string(), manifest.clone())]
            .into_iter()
            .collect(),
        files,
        fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
        fetch_file_calls: Arc::new(AtomicUsize::new(0)),
        fetch_range_calls: Arc::new(AtomicUsize::new(0)),
    };
    let remote_arc = Arc::new(remote.clone());

    let req = RepairRequest {
        repo_name: "repo".to_string(),
        checkout_root: root.to_path_buf(),
        enabled_mods: vec!["@mod".to_string()],
        remote: remote_arc,
        checksummer: Arc::new(TestChecksummer),
        tuning: RepairTuning::default(),
    };
    let sink = Arc::new(TestSink::default());
    let outcome = apply_ops(
        plan.ops,
        &req,
        sink.clone(),
        ApplyOptions {
            supports_ranges: false,
        },
    )
    .await
    .unwrap();
    assert!(outcome.failures.is_empty());

    // Expect full-file fetch rather than ranged fetches.
    assert_eq!(remote.fetch_range_calls.load(Ordering::Relaxed), 0);
    assert_eq!(remote.fetch_file_calls.load(Ordering::Relaxed), 1);
    assert_eq!(std::fs::read(mod_root.join("file.bin")).unwrap(), data);
}

#[tokio::test]
async fn apply_continues_on_non_safety_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    let ok_data = b"12345678".to_vec();
    let bad_expected = b"abcdefgh".to_vec();

    let ok_parts = build_parts(&ok_data, ok_data.len());
    let bad_parts = build_parts(&bad_expected, bad_expected.len());

    let mut files = HashMap::new();
    files.insert(("@mod".to_string(), "ok.bin".to_string()), ok_data.clone());
    files.insert(
        ("@mod".to_string(), "bad.bin".to_string()),
        b"abcd".to_vec(),
    );

    let remote = FakeRemote {
        supports_ranges: true,
        manifests: HashMap::new(),
        files,
        fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
        fetch_file_calls: Arc::new(AtomicUsize::new(0)),
        fetch_range_calls: Arc::new(AtomicUsize::new(0)),
    };

    let ok_target = FileTarget {
        size: ok_data.len() as u64,
        file_checksum: blake3::hash(&ok_data).as_bytes().to_vec(),
        parts: ok_parts.clone(),
        strategy: RepairStrategy::Full,
        parts_to_fetch: ok_parts.clone(),
    };
    let bad_target = FileTarget {
        size: bad_expected.len() as u64,
        file_checksum: blake3::hash(&bad_expected).as_bytes().to_vec(),
        parts: bad_parts.clone(),
        strategy: RepairStrategy::Full,
        parts_to_fetch: bad_parts.clone(),
    };

    let ops = vec![
        PlannedOp {
            mod_id: "@mod".to_string(),
            rel_path: "ok.bin".to_string(),
            abs_path: mod_root.join("ok.bin"),
            target: ok_target,
            estimated_bytes: ok_data.len() as u64,
        },
        PlannedOp {
            mod_id: "@mod".to_string(),
            rel_path: "bad.bin".to_string(),
            abs_path: mod_root.join("bad.bin"),
            target: bad_target,
            estimated_bytes: bad_expected.len() as u64,
        },
    ];

    let req = RepairRequest {
        repo_name: "repo".to_string(),
        checkout_root: root.to_path_buf(),
        enabled_mods: vec!["@mod".to_string()],
        remote: Arc::new(remote),
        checksummer: Arc::new(TestChecksummer),
        tuning: RepairTuning::default(),
    };
    let sink = Arc::new(TestSink::default());

    let outcome = apply_ops(
        ops,
        &req,
        sink.clone(),
        ApplyOptions {
            supports_ranges: true,
        },
    )
    .await
    .unwrap();

    assert_eq!(outcome.failures.len(), 1);
    assert!(outcome.aborted.is_none());
    assert_eq!(std::fs::read(mod_root.join("ok.bin")).unwrap(), ok_data);
}

#[tokio::test]
#[cfg(unix)]
async fn apply_aborts_on_unsafe_on_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, mod_root.join("addons")).unwrap();

    let data = b"data".to_vec();
    let parts = build_parts(&data, data.len());

    let op = PlannedOp {
        mod_id: "@mod".to_string(),
        rel_path: "addons/bad.bin".to_string(),
        abs_path: mod_root.join("addons").join("bad.bin"),
        target: FileTarget {
            size: data.len() as u64,
            file_checksum: blake3::hash(&data).as_bytes().to_vec(),
            parts: parts.clone(),
            strategy: RepairStrategy::Full,
            parts_to_fetch: parts.clone(),
        },
        estimated_bytes: data.len() as u64,
    };

    let req = RepairRequest {
        repo_name: "repo".to_string(),
        checkout_root: root.to_path_buf(),
        enabled_mods: vec!["@mod".to_string()],
        remote: Arc::new(FakeRemote {
            supports_ranges: true,
            manifests: HashMap::new(),
            files: HashMap::new(),
            fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
            fetch_file_calls: Arc::new(AtomicUsize::new(0)),
            fetch_range_calls: Arc::new(AtomicUsize::new(0)),
        }),
        checksummer: Arc::new(TestChecksummer),
        tuning: RepairTuning::default(),
    };
    let sink = Arc::new(TestSink::default());

    let outcome = apply_ops(
        vec![op],
        &req,
        sink.clone(),
        ApplyOptions {
            supports_ranges: true,
        },
    )
    .await
    .unwrap();

    assert!(outcome.aborted.is_some());
    assert_eq!(outcome.failures.len(), 1);
    assert!(outcome.failures[0].aborting);
}

#[test]
fn fetch_rejects_manifest_mod_id_mismatch() {
    let mut files = HashMap::new();
    files.insert(
        ("@mod".to_string(), "file.bin".to_string()),
        b"data".to_vec(),
    );
    let manifest = ModManifest {
        mod_id: "other".to_string(),
        files: vec![FileEntry {
            rel_path: "file.bin".to_string(),
            size: 4,
            file_checksum: blake3::hash(b"data").as_bytes().to_vec(),
            parts: build_parts(b"data", 4),
        }],
    };

    let remote = FakeRemote {
        supports_ranges: true,
        manifests: vec![("@mod".to_string(), manifest)].into_iter().collect(),
        files,
        fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
        fetch_file_calls: Arc::new(AtomicUsize::new(0)),
        fetch_range_calls: Arc::new(AtomicUsize::new(0)),
    };

    let rt = tokio::runtime::Runtime::new().unwrap();
    let mods = vec!["@mod".to_string()];
    let res = rt.block_on(async { fetch_all(Arc::new(remote), &mods, 2).await });
    assert!(res.is_err());
}

#[tokio::test]
async fn verify_then_repair_skips_without_remote_fetch() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    let data = b"content".to_vec();
    let manifest = build_manifest("@mod", "file.bin", &data, 4);
    std::fs::write(mod_root.join("file.bin"), &data).unwrap();

    let mut manifests = HashMap::new();
    manifests.insert("@mod".to_string(), manifest.clone());

    let mut files = HashMap::new();
    files.insert(("@mod".to_string(), "file.bin".to_string()), data.clone());

    let remote = FakeRemote {
        supports_ranges: true,
        manifests,
        files,
        fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
        fetch_file_calls: Arc::new(AtomicUsize::new(0)),
        fetch_range_calls: Arc::new(AtomicUsize::new(0)),
    };

    let mut idx = FleetIndex::open_in_memory().unwrap();
    let enabled_mods = vec!["@mod".to_string()];
    let mut enabled_sorted = enabled_mods.clone();
    enabled_sorted.sort();
    let repo_id = fleet_index::normalize_repo_id("abcd");
    let enabled_hash = fleet_index::enabled_mods_hash(&enabled_sorted);
    let state_id = fleet_index::state_id(&repo_id, &enabled_hash);
    idx.set_desired_state(DesiredState {
        repo_url: "http://example".to_string(),
        repo_id,
        enabled_mods_hash: enabled_hash,
        state_id: state_id.clone(),
        updated_at_unix_s: 1,
    })
    .unwrap();

    let verify_req = VerifyRequest {
        repo_name: "repo".to_string(),
        checkout_root: root.to_path_buf(),
        enabled_mods: enabled_mods.clone(),
        remote: Arc::new(remote.clone()),
        checksummer: Arc::new(TestChecksummer),
        tuning: VerifyTuning::default(),
    };
    let sink = Arc::new(TestSink::default());
    let report = sync_engine::flows::verify(verify_req, &mut idx, sink.clone())
        .await
        .unwrap();
    assert!(report.ok);
    assert!(idx.verified_get().unwrap().is_some());
    assert!(idx
        .file_state_get(&state_id, "@mod", "file.bin")
        .unwrap()
        .is_some());

    let remote_calls_before = remote.fetch_manifest_calls.load(Ordering::Relaxed);

    let repair_req = RepairRequest {
        repo_name: "repo".to_string(),
        checkout_root: root.to_path_buf(),
        enabled_mods,
        remote: Arc::new(remote.clone()),
        checksummer: Arc::new(TestChecksummer),
        tuning: RepairTuning::default(),
    };
    let outcome = sync_engine::flows::repair(repair_req, &mut idx, sink.clone())
        .await
        .unwrap();
    assert!(outcome.report.skipped);
    assert_eq!(
        remote.fetch_manifest_calls.load(Ordering::Relaxed),
        remote_calls_before
    );
}

#[tokio::test]
#[cfg(unix)]
async fn unsafe_on_disk_verify_reports_and_repair_aborts_even_if_cached() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    let safe_data = b"safe".to_vec();
    let unsafe_data = b"unsafe".to_vec();
    std::fs::write(mod_root.join("safe.bin"), &safe_data).unwrap();

    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let unsafe_target = outside.join("bad.bin");
    std::fs::write(&unsafe_target, &unsafe_data).unwrap();

    // Unsafe traversal: @mod/addons -> outside
    std::os::unix::fs::symlink(&outside, mod_root.join("addons")).unwrap();
    let unsafe_rel = "addons/bad.bin".to_string();

    let manifest = ModManifest {
        mod_id: "@mod".to_string(),
        files: vec![
            FileEntry {
                rel_path: "safe.bin".to_string(),
                size: safe_data.len() as u64,
                file_checksum: blake3::hash(&safe_data).as_bytes().to_vec(),
                parts: build_parts(&safe_data, 4),
            },
            FileEntry {
                rel_path: unsafe_rel.clone(),
                size: unsafe_data.len() as u64,
                file_checksum: blake3::hash(&unsafe_data).as_bytes().to_vec(),
                parts: build_parts(&unsafe_data, 4),
            },
        ],
    };
    let unsafe_entry = manifest
        .files
        .iter()
        .find(|f| f.rel_path == unsafe_rel)
        .unwrap()
        .clone();

    let mut manifests = HashMap::new();
    manifests.insert("@mod".to_string(), manifest.clone());

    let mut files = HashMap::new();
    files.insert(
        ("@mod".to_string(), "safe.bin".to_string()),
        safe_data.clone(),
    );
    files.insert(
        ("@mod".to_string(), unsafe_rel.clone()),
        unsafe_data.clone(),
    );

    let remote = FakeRemote {
        supports_ranges: true,
        manifests,
        files,
        fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
        fetch_file_calls: Arc::new(AtomicUsize::new(0)),
        fetch_range_calls: Arc::new(AtomicUsize::new(0)),
    };

    let enabled_mods = vec!["@mod".to_string()];
    let mut enabled_sorted = enabled_mods.clone();
    enabled_sorted.sort();
    let repo_id = fleet_index::normalize_repo_id("abcd");
    let enabled_hash = fleet_index::enabled_mods_hash(&enabled_sorted);
    let state_id = fleet_index::state_id(&repo_id, &enabled_hash);

    let mut idx = FleetIndex::open_in_memory().unwrap();
    idx.set_desired_state(DesiredState {
        repo_url: "http://example".to_string(),
        repo_id,
        enabled_mods_hash: enabled_hash,
        state_id: state_id.clone(),
        updated_at_unix_s: 1,
    })
    .unwrap();

    // Verify should report UnsafeOnDisk but still keep the safe file state.
    let verify_req = VerifyRequest {
        repo_name: "repo".to_string(),
        checkout_root: root.to_path_buf(),
        enabled_mods: enabled_mods.clone(),
        remote: Arc::new(remote.clone()),
        checksummer: Arc::new(TestChecksummer),
        tuning: VerifyTuning::default(),
    };
    let sink = Arc::new(TestSink::default());
    let report = sync_engine::flows::verify(verify_req, &mut idx, sink.clone())
        .await
        .unwrap();
    assert!(!report.ok);
    assert!(report
        .issues
        .iter()
        .any(|i| matches!(i.kind, VerifyIssueKind::UnsafeOnDisk)));
    assert!(idx
        .file_state_get(&state_id, "@mod", "safe.bin")
        .unwrap()
        .is_some());

    // Simulate "cached" state for the unsafe file, then ensure repair still aborts.
    idx.verified_set(&state_id, 10).unwrap();
    idx.expected_replace_all(
        &state_id,
        vec![
            fleet_index::ExpectedFile {
                mod_id: "@mod".to_string(),
                rel_path: "safe.bin".to_string(),
                size: safe_data.len() as u64,
            },
            fleet_index::ExpectedFile {
                mod_id: "@mod".to_string(),
                rel_path: unsafe_rel.clone(),
                size: unsafe_data.len() as u64,
            },
        ],
    )
    .unwrap();
    let unsafe_mtime = mtime_ns(&mod_root.join(&unsafe_rel));
    idx.file_state_upsert(
        &state_id,
        "@mod",
        &unsafe_rel,
        unsafe_data.len() as u64,
        unsafe_mtime,
        &unsafe_entry.file_checksum,
    )
    .unwrap();

    let repair_req = RepairRequest {
        repo_name: "repo".to_string(),
        checkout_root: root.to_path_buf(),
        enabled_mods,
        remote: Arc::new(remote),
        checksummer: Arc::new(TestChecksummer),
        tuning: RepairTuning::default(),
    };
    let outcome = sync_engine::flows::repair(repair_req, &mut idx, sink.clone())
        .await
        .unwrap();
    assert!(outcome.aborted.is_some());
    assert!(unsafe_target.exists());
    assert!(idx.verified_get().unwrap().is_none());
    assert!(idx
        .file_state_get(&state_id, "@mod", &unsafe_rel)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn verify_then_repair_repairs_corruption_and_verify_becomes_ok() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mod_root = root.join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    let data = vec![7u8; 100];
    let manifest = build_manifest("@mod", "file.bin", &data, 10);

    let mut corrupted = data.clone();
    corrupted[0] = 9;
    std::fs::write(mod_root.join("file.bin"), &corrupted).unwrap();

    let mut manifests = HashMap::new();
    manifests.insert("@mod".to_string(), manifest.clone());

    let mut files = HashMap::new();
    files.insert(("@mod".to_string(), "file.bin".to_string()), data.clone());

    let remote = FakeRemote {
        supports_ranges: true,
        manifests,
        files,
        fetch_manifest_calls: Arc::new(AtomicUsize::new(0)),
        fetch_file_calls: Arc::new(AtomicUsize::new(0)),
        fetch_range_calls: Arc::new(AtomicUsize::new(0)),
    };

    let enabled_mods = vec!["@mod".to_string()];
    let mut enabled_sorted = enabled_mods.clone();
    enabled_sorted.sort();
    let repo_id = fleet_index::normalize_repo_id("abcd");
    let enabled_hash = fleet_index::enabled_mods_hash(&enabled_sorted);
    let state_id = fleet_index::state_id(&repo_id, &enabled_hash);

    let mut idx = FleetIndex::open_in_memory().unwrap();
    idx.set_desired_state(DesiredState {
        repo_url: "http://example".to_string(),
        repo_id,
        enabled_mods_hash: enabled_hash,
        state_id: state_id.clone(),
        updated_at_unix_s: 1,
    })
    .unwrap();

    let sink = Arc::new(TestSink::default());

    // Pre-repair verify: should NOT be ok.
    let verify_req1 = VerifyRequest {
        repo_name: "repo".to_string(),
        checkout_root: root.to_path_buf(),
        enabled_mods: enabled_mods.clone(),
        remote: Arc::new(remote.clone()),
        checksummer: Arc::new(TestChecksummer),
        tuning: VerifyTuning::default(),
    };
    let report1 = sync_engine::flows::verify(verify_req1, &mut idx, sink.clone())
        .await
        .unwrap();
    assert!(!report1.ok);

    // Repair: should do real work (not skip).
    let repair_req = RepairRequest {
        repo_name: "repo".to_string(),
        checkout_root: root.to_path_buf(),
        enabled_mods: enabled_mods.clone(),
        remote: Arc::new(remote.clone()),
        checksummer: Arc::new(TestChecksummer),
        tuning: RepairTuning::default(),
    };
    let repair_outcome = sync_engine::flows::repair(repair_req, &mut idx, sink.clone())
        .await
        .unwrap();
    assert!(!repair_outcome.report.skipped);
    assert_eq!(std::fs::read(mod_root.join("file.bin")).unwrap(), data);

    // Post-repair verify: should become ok.
    let verify_req2 = VerifyRequest {
        repo_name: "repo".to_string(),
        checkout_root: root.to_path_buf(),
        enabled_mods,
        remote: Arc::new(remote),
        checksummer: Arc::new(TestChecksummer),
        tuning: VerifyTuning::default(),
    };
    let report2 = sync_engine::flows::verify(verify_req2, &mut idx, sink.clone())
        .await
        .unwrap();
    assert!(report2.ok);
    assert!(idx
        .file_state_get(&state_id, "@mod", "file.bin")
        .unwrap()
        .is_some());
}
