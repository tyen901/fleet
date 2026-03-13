use fleet_domain::LocalStateHealth;
use flux_manifest::{DesiredManifest, ManifestEntry};
use flux_types::Signature;
use std::collections::BTreeMap;
use std::path::Path;

mod audit;
mod parallel;
mod refresh;
mod scan;
mod walk;

pub(crate) use audit::{
    assess_snapshot, scan_disk_state, trim_stale_trusted_files, verify_trusted_files,
    AuditProgress, VerifyProgress,
};
pub(crate) use refresh::{refresh_trusted_inventory_from_disk, InventoryRefreshProgress};
pub(crate) use walk::WalkProgress;

#[derive(Clone, Debug)]
pub(crate) struct LocalStateAssessment {
    pub profile_id: String,
    pub health: LocalStateHealth,
    pub checked_at_unix_ms: u64,
    pub expected_missing_count: u64,
    pub unexpected_count: u64,
    pub unexpected_paths: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct StaleTrustedPaths {
    pub missing: Vec<String>,
    pub modified: Vec<String>,
}

impl StaleTrustedPaths {
    pub(crate) fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.modified.is_empty()
    }

    pub(crate) fn all_paths(&self) -> Vec<String> {
        let mut out = self.missing.clone();
        out.extend(self.modified.iter().cloned());
        out.sort();
        out.dedup();
        out
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LocalInventorySnapshot {
    pub assessment: LocalStateAssessment,
    pub tracked_paths: Vec<String>,
    pub missing_tracked_paths: Vec<String>,
    pub modified_tracked_paths: Vec<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub(crate) struct TrustRefreshResult {
    pub reused_paths: Vec<String>,
    pub rescanned_paths: Vec<String>,
    pub stale_paths: StaleTrustedPaths,
}

#[derive(Clone, Debug)]
struct DesiredFile {
    size_bytes: u64,
    segments: Vec<(Signature, u64)>,
}

fn manifest_files(manifest: &DesiredManifest) -> BTreeMap<String, DesiredFile> {
    let mut out = BTreeMap::new();
    for entry in &manifest.entries {
        let ManifestEntry::File(file) = entry else {
            continue;
        };
        out.insert(
            normalize_rel(&file.rel_path),
            DesiredFile {
                size_bytes: file.size_bytes,
                segments: file
                    .segments
                    .iter()
                    .map(|segment| (segment.signature.clone(), segment.len))
                    .collect(),
            },
        );
    }
    out
}

fn metadata_mtime_ns(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default()
}

fn now_unix_ms() -> u64 {
    fleet_domain::time::now_unix_ms()
}

fn normalize_rel(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::audit::assess_snapshot;
    use super::parallel::{chunk_progress_reporter, execute_chunked};
    use super::refresh::{refresh_trusted_inventory_from_disk, InventoryRefreshProgress};
    use super::scan::scan_local_file;
    use super::walk::WalkItem;
    use super::{normalize_rel, StaleTrustedPaths};
    use fleet_domain::{LocalStateHealth, LocalStateProgress, LocalStateStage};
    use fleet_inventory::{Inventory, InventoryError};
    use flux_inventory_contract::CommittedFileRecord;
    use flux_manifest::{DesiredManifest, ManifestEntry, ManifestFile, ManifestVersion};
    use flux_types::{SegmentSpec, SourceRef};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn execute_chunked_uses_serial_fallback_when_worker_count_is_one() {
        let items = vec![0_u32, 1, 2, 3];
        let caller_thread = thread::current().id();
        let chunk_threads = execute_chunked(&items, 1, 2, |chunk| {
            Ok::<_, InventoryError>(vec![thread::current().id(); chunk.len()])
        })
        .expect("chunk execution");

        let flattened = chunk_threads.into_iter().flatten().collect::<Vec<_>>();
        assert!(!flattened.is_empty());
        assert!(flattened.into_iter().all(|id| id == caller_thread));
    }

    #[test]
    fn execute_chunked_preserves_input_chunk_order() {
        let items = (0_u32..12).collect::<Vec<_>>();
        let chunk_results = execute_chunked(&items, 4, 3, |chunk| {
            let first = chunk.first().copied().unwrap_or_default();
            let delay_ms = (12 - first) as u64;
            thread::sleep(Duration::from_millis(delay_ms));
            Ok::<_, InventoryError>(chunk.to_vec())
        })
        .expect("chunk execution");

        let flattened = chunk_results.into_iter().flatten().collect::<Vec<_>>();
        assert_eq!(flattened, items);
    }

    #[test]
    fn chunk_progress_updates_are_monotonic_and_flush_total() {
        let events = Arc::new(Mutex::new(Vec::<LocalStateProgress>::new()));
        let sink_events = Arc::clone(&events);
        let sink = Arc::new(move |progress: LocalStateProgress| {
            sink_events.lock().expect("lock").push(progress);
        });
        let mut reporter = chunk_progress_reporter(Some(sink), LocalStateStage::Verifying, 600_u64);

        reporter.advance_by(256);
        reporter.advance_by(256);
        reporter.advance_by(88);
        reporter.final_flush();

        let events = events.lock().expect("lock");
        let scanned = events
            .iter()
            .map(|event| event.files_scanned)
            .collect::<Vec<_>>();
        assert_eq!(scanned, vec![256, 512, 600, 600]);
        assert!(scanned.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(events
            .iter()
            .all(|event| event.files_total == 600 && event.stage == LocalStateStage::Verifying));
    }

    #[test]
    fn assess_clean_baseline_matches_serial_semantics() {
        let fixture = TestFixture::new();
        fixture.write_file("mods/a.pbo", b"alpha");
        fixture.write_file("mods/b.pbo", b"bravo");
        fixture.seed_inventory(&["mods/a.pbo", "mods/b.pbo"]);

        let actual = assess_snapshot(&fixture.inventory, "profile", fixture.dest(), "", None)
            .expect("assess snapshot");

        assert_eq!(actual.assessment.health, LocalStateHealth::Ready);
        assert_eq!(actual.assessment.expected_missing_count, 0);
        assert_eq!(actual.assessment.unexpected_count, 0);
        assert_eq!(actual.assessment.unexpected_paths, Vec::<String>::new());
        assert_eq!(actual.tracked_paths, vec!["mods/a.pbo", "mods/b.pbo"]);
    }

    #[test]
    fn assess_with_unexpected_files_reports_local_drift() {
        let fixture = TestFixture::new();
        fixture.write_file("mods/a.pbo", b"alpha");
        fixture.write_file("mods/unexpected.pbo", b"rogue");
        fixture.seed_inventory(&["mods/a.pbo"]);

        let actual = assess_snapshot(&fixture.inventory, "profile", fixture.dest(), "", None)
            .expect("assess snapshot");

        assert_eq!(actual.assessment.health, LocalStateHealth::LocalDrift);
        assert_eq!(
            actual.assessment.unexpected_paths,
            vec!["mods/unexpected.pbo"]
        );
        assert_eq!(actual.assessment.unexpected_count, 1);
        assert_eq!(actual.assessment.expected_missing_count, 0);
        assert_eq!(actual.tracked_paths, vec!["mods/a.pbo"]);
    }

    #[test]
    fn assess_with_missing_finalized_files_reports_local_drift() {
        let fixture = TestFixture::new();
        fixture.write_file("mods/a.pbo", b"alpha");
        fixture.write_file("mods/missing.pbo", b"to be removed");
        fixture.seed_inventory(&["mods/a.pbo", "mods/missing.pbo"]);
        std::fs::remove_file(fixture.dest().join("mods/missing.pbo")).expect("remove missing");

        let actual = assess_snapshot(&fixture.inventory, "profile", fixture.dest(), "", None)
            .expect("assess snapshot");

        assert_eq!(actual.assessment.health, LocalStateHealth::LocalDrift);
        assert_eq!(actual.assessment.expected_missing_count, 1);
        assert_eq!(actual.assessment.unexpected_count, 0);
        assert_eq!(actual.tracked_paths, vec!["mods/a.pbo"]);
    }

    #[test]
    fn assess_with_modified_finalized_files_reports_local_drift() {
        let fixture = TestFixture::new();
        fixture.write_file("mods/a.pbo", b"alpha");
        fixture.seed_inventory(&["mods/a.pbo"]);
        fixture.write_file("mods/a.pbo", b"alpha-modified");

        let actual = assess_snapshot(&fixture.inventory, "profile", fixture.dest(), "", None)
            .expect("assess snapshot");

        assert_eq!(actual.assessment.health, LocalStateHealth::LocalDrift);
        assert_eq!(actual.assessment.expected_missing_count, 0);
        assert_eq!(actual.assessment.unexpected_count, 0);
        assert_eq!(actual.tracked_paths, Vec::<String>::new());
    }

    #[test]
    fn refresh_with_mixed_reused_rescanned_and_stale_files_updates_inventory() {
        let (actual_fixture, manifest) = setup_mixed_refresh_fixture();
        let actual = refresh_trusted_inventory_from_disk(
            &actual_fixture.inventory,
            actual_fixture.dest(),
            &manifest,
            "",
            None,
        )
        .expect("refresh trusted inventory");

        assert_eq!(actual.reused_paths, vec!["mods/reused.pbo"]);
        assert_eq!(
            actual.rescanned_paths,
            vec!["mods/manifest-only.pbo", "mods/rescan-match.pbo"]
        );
        assert_eq!(actual.stale_paths.missing, vec!["mods/stale-missing.pbo"]);
        assert_eq!(actual.stale_paths.modified, vec!["mods/rescan-stale.pbo"]);

        let tracked = actual_fixture.finalized_paths();
        assert_eq!(
            tracked,
            vec![
                "mods/manifest-only.pbo",
                "mods/rescan-match.pbo",
                "mods/reused.pbo",
            ]
        );
    }

    #[test]
    fn refresh_promotes_manifest_matching_untracked_files_into_trusted_state() {
        let fixture = TestFixture::new();
        fixture.write_file("mods/promoted.pbo", b"promoted");
        let manifest = fixture.manifest_for(&["mods/promoted.pbo"]);

        let result = refresh_trusted_inventory_from_disk(
            &fixture.inventory,
            fixture.dest(),
            &manifest,
            "",
            None,
        )
        .expect("refresh trusted inventory");

        assert_eq!(result.reused_paths, Vec::<String>::new());
        assert_eq!(result.rescanned_paths, vec!["mods/promoted.pbo"]);
        assert_eq!(result.stale_paths, StaleTrustedPaths::default());
        assert_eq!(fixture.finalized_paths(), vec!["mods/promoted.pbo"]);
    }

    #[test]
    fn refresh_progress_streams_walking_updates_for_multi_chunk_scan() {
        let fixture = TestFixture::new();
        let mut rel_paths = Vec::new();
        for idx in 0..300 {
            let rel_path = format!("mods/file-{idx:03}.pbo");
            fixture.write_file(&rel_path, b"x");
            rel_paths.push(rel_path);
        }
        let manifest_refs = rel_paths.iter().map(String::as_str).collect::<Vec<_>>();
        let manifest = fixture.manifest_for(&manifest_refs);
        let events = Arc::new(Mutex::new(Vec::<InventoryRefreshProgress>::new()));
        let sink_events = Arc::clone(&events);

        refresh_trusted_inventory_from_disk(
            &fixture.inventory,
            fixture.dest(),
            &manifest,
            "",
            Some(Arc::new(move |progress| {
                sink_events.lock().expect("lock").push(progress);
            })),
        )
        .expect("refresh trusted inventory");

        let walking = events
            .lock()
            .expect("lock")
            .iter()
            .filter_map(|progress| match progress {
                InventoryRefreshProgress::Walking {
                    files_done,
                    files_total: Some(files_total),
                    ..
                } => Some((*files_done, *files_total)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            walking.len() >= 2,
            "expected multiple walking metadata updates for multi-chunk scans"
        );
        assert!(walking.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        assert_eq!(walking.last(), Some(&(300, 300)));
    }

    #[test]
    fn refresh_progress_counts_rescanned_candidates_even_when_not_reinserted() {
        let fixture = TestFixture::new();
        fixture.write_file("mods/changed.pbo", b"before");
        fixture.seed_inventory(&["mods/changed.pbo"]);
        let manifest = fixture.manifest_for(&["mods/changed.pbo"]);
        fixture.write_file("mods/changed.pbo", b"after-after");
        let events = Arc::new(Mutex::new(Vec::<InventoryRefreshProgress>::new()));
        let sink_events = Arc::clone(&events);

        let result = refresh_trusted_inventory_from_disk(
            &fixture.inventory,
            fixture.dest(),
            &manifest,
            "",
            Some(Arc::new(move |progress| {
                sink_events.lock().expect("lock").push(progress);
            })),
        )
        .expect("refresh trusted inventory");

        assert_eq!(result.rescanned_paths, vec!["mods/changed.pbo"]);
        assert_eq!(result.stale_paths.modified, vec!["mods/changed.pbo"]);

        let rescanning = events
            .lock()
            .expect("lock")
            .iter()
            .filter_map(|progress| match progress {
                InventoryRefreshProgress::Rescanning {
                    files_done,
                    files_total,
                    ..
                } => Some((*files_done, *files_total)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(rescanning.last(), Some(&(1, 1)));
    }

    fn setup_mixed_refresh_fixture() -> (TestFixture, DesiredManifest) {
        let fixture = TestFixture::new();
        fixture.write_file("mods/reused.pbo", b"reuse");
        fixture.write_file("mods/rescan-match.pbo", b"rescan match");
        fixture.write_file("mods/rescan-stale.pbo", b"new content");
        fixture.write_file("mods/manifest-only.pbo", b"promote");
        fixture.write_file("mods/stale-missing.pbo", b"gone soon");
        fixture.seed_inventory(&[
            "mods/reused.pbo",
            "mods/rescan-match.pbo",
            "mods/rescan-stale.pbo",
            "mods/stale-missing.pbo",
        ]);
        fixture.write_file("mods/rescan-match.pbo", b"rescan match");
        fixture.write_file("mods/rescan-stale.pbo", b"changed");
        std::fs::remove_file(fixture.dest().join("mods/stale-missing.pbo")).expect("remove stale");
        let manifest = fixture.manifest_for(&[
            "mods/reused.pbo",
            "mods/rescan-match.pbo",
            "mods/manifest-only.pbo",
        ]);
        (fixture, manifest)
    }

    struct TestFixture {
        _tempdir: tempfile::TempDir,
        dest: PathBuf,
        inventory: Inventory,
    }

    impl TestFixture {
        fn new() -> Self {
            let tempdir = tempfile::tempdir().expect("tempdir");
            let dest = tempdir.path().join("dest");
            std::fs::create_dir_all(&dest).expect("create dest");
            let inventory =
                Inventory::open(&tempdir.path().join("inventory.sqlite")).expect("open");
            Self {
                _tempdir: tempdir,
                dest,
                inventory,
            }
        }

        fn dest(&self) -> &Path {
            &self.dest
        }

        fn write_file(&self, rel_path: &str, contents: &[u8]) {
            let fs_path = self.dest.join(rel_path);
            if let Some(parent) = fs_path.parent() {
                std::fs::create_dir_all(parent).expect("create parent");
            }
            std::fs::write(&fs_path, contents).expect("write file");
        }

        fn seed_inventory(&self, rel_paths: &[&str]) {
            let records = rel_paths
                .iter()
                .map(|rel_path| self.committed_record(rel_path))
                .collect::<Vec<_>>();
            self.inventory
                .upsert_trusted_files_batch(&records)
                .expect("seed inventory");
            self.inventory
                .initialize_trusted_baseline()
                .expect("initialize baseline");
        }

        fn finalized_paths(&self) -> Vec<String> {
            self.inventory.finalized_paths().expect("finalized paths")
        }

        fn committed_record(&self, rel_path: &str) -> CommittedFileRecord {
            let fs_path = self.dest.join(rel_path);
            let walk_item = walk_item_for_path(&self.dest, &fs_path);
            let scanned = scan_local_file(&walk_item).expect("scan local file");
            CommittedFileRecord {
                rel_path: PathBuf::from(rel_path),
                size_bytes: scanned.size_bytes,
                mtime_ns: scanned.mtime_ns,
                segments: scanned.segments,
            }
        }

        fn manifest_for(&self, rel_paths: &[&str]) -> DesiredManifest {
            DesiredManifest {
                version: ManifestVersion::V1,
                entries: rel_paths
                    .iter()
                    .map(|rel_path| {
                        let fs_path = self.dest.join(rel_path);
                        let walk_item = walk_item_for_path(&self.dest, &fs_path);
                        let scanned = scan_local_file(&walk_item).expect("scan local file");
                        ManifestEntry::File(ManifestFile {
                            rel_path: PathBuf::from(rel_path),
                            size_bytes: scanned.size_bytes,
                            segments: scanned
                                .segments
                                .iter()
                                .map(|(signature, len)| SegmentSpec {
                                    source: SourceRef::Http {
                                        url: Arc::<str>::from("https://example.invalid/file"),
                                    },
                                    src_offset: 0,
                                    len: *len,
                                    signature: signature.clone(),
                                })
                                .collect(),
                            mode: None,
                            mtime_ns: None,
                        })
                    })
                    .collect(),
                prune_paths: Vec::new(),
            }
        }
    }

    fn walk_item_for_path(root: &Path, fs_path: &Path) -> WalkItem {
        let metadata = std::fs::symlink_metadata(fs_path).expect("metadata");
        WalkItem {
            fs_path: fs_path.to_path_buf(),
            rel_path: normalize_rel(fs_path.strip_prefix(root).expect("strip prefix")),
            size_bytes: metadata.len(),
            mtime_ns: super::metadata_mtime_ns(&metadata),
        }
    }
}
