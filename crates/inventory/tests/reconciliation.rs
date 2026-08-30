use fleet_inventory::{
    InventoryDesiredFile, InventoryObservedFile, InventoryReconcileMode, InventoryReconcileWrite,
    MaterializationInventory,
};
use flux::{
    FreshnessProof, LocalFileFact, LocalFileSegmentFact, OpaqueSegmentIdentity, ProfileFingerprint,
    SegmentKey, TargetPath, ValidationSpec,
};
use futures_util::StreamExt;

#[test]
fn incremental_reconciliation_reuses_only_equal_flux_freshness() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let unchanged = file_record("mods/unchanged.pbo", 1, 1);
    let changed = file_record("mods/changed.pbo", 2, 1);
    let missing = file_record("mods/missing.pbo", 3, 1);
    inventory
        .apply_reconcile(InventoryReconcileWrite {
            managed_paths: vec![
                unchanged.path.clone(),
                changed.path.clone(),
                missing.path.clone(),
            ],
            upsert_facts: vec![unchanged.clone(), changed.clone(), missing.clone()],
            ..Default::default()
        })
        .expect("seed inventory");

    let observed = vec![
        observed(&unchanged.path, 4, 1),
        observed(&changed.path, 4, 2),
        observed_path("mods/new.pbo", 4, 1),
        observed_path("notes.txt", 4, 1),
    ];
    let desired = vec![
        desired(&unchanged),
        desired(&changed),
        desired_path("mods/new.pbo", 4, 4),
    ];

    let plan = inventory
        .plan_reconcile(&observed, &desired, InventoryReconcileMode::Incremental)
        .expect("plan reconciliation");

    assert_eq!(plan.scan_candidate_positions, vec![1, 2]);
    assert_eq!(plan.remove_reusable_facts, vec![missing.path]);
    assert_eq!(plan.managed_paths.len(), 4);
}

#[test]
fn full_reconciliation_scans_every_reusable_or_expected_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let record = file_record("mods/a.pbo", 1, 1);
    inventory
        .apply_reconcile(InventoryReconcileWrite {
            managed_paths: vec![record.path.clone()],
            upsert_facts: vec![record.clone()],
            ..Default::default()
        })
        .expect("seed inventory");

    let plan = inventory
        .plan_reconcile(
            &[observed(&record.path, 4, 1)],
            &[desired(&record)],
            InventoryReconcileMode::Full,
        )
        .expect("plan full reconciliation");

    assert_eq!(plan.scan_candidate_positions, vec![0]);
}

#[test]
fn assessment_compares_the_complete_ordered_segment_manifest() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let exact = file_record("mods/exact.pbo", 1, 1);
    let modified = file_record("mods/modified.pbo", 2, 1);
    let unexpected = file_record("mods/unexpected.pbo", 3, 1);
    inventory
        .apply_reconcile(InventoryReconcileWrite {
            managed_paths: vec![
                exact.path.clone(),
                modified.path.clone(),
                unexpected.path.clone(),
            ],
            upsert_facts: vec![exact.clone(), modified.clone(), unexpected],
            ..Default::default()
        })
        .expect("seed inventory");

    let mut wrong_segments = desired(&modified);
    wrong_segments.segments[0] = segment(9);
    let report = inventory
        .assess_expected(&[
            desired(&exact),
            wrong_segments,
            desired_path("mods/missing.pbo", 4, 4),
        ])
        .expect("assess inventory");

    assert_eq!(report.exact_paths, vec!["mods/exact.pbo"]);
    assert_eq!(report.modified_paths, vec!["mods/modified.pbo"]);
    assert_eq!(report.missing_paths, vec!["mods/missing.pbo"]);
    assert_eq!(report.unexpected_paths, vec!["mods/unexpected.pbo"]);
}

#[tokio::test]
async fn applying_reconciliation_atomically_replaces_the_managed_snapshot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let removed = file_record("mods/removed.pbo", 1, 1);
    inventory
        .apply_reconcile(InventoryReconcileWrite {
            managed_paths: vec![removed.path.clone()],
            upsert_facts: vec![removed.clone()],
            ..Default::default()
        })
        .expect("seed inventory");
    let replacement = TargetPath::new("mods/replacement.pbo").expect("path");

    inventory
        .apply_reconcile(InventoryReconcileWrite {
            managed_paths: vec![replacement.clone()],
            ..Default::default()
        })
        .expect("replace snapshot");

    assert_eq!(managed_paths(&inventory).await, vec![replacement]);
    assert!(inventory
        .lookup_files(&[removed.path])
        .expect("lookup removed file")[0]
        .is_none());
}

fn open_inventory(temp: &tempfile::TempDir) -> MaterializationInventory {
    MaterializationInventory::open(&temp.path().join("inventory.sqlite")).expect("open inventory")
}

async fn managed_paths(inventory: &MaterializationInventory) -> Vec<TargetPath> {
    let mut stream = inventory.managed_path_batches(2);
    let mut paths = Vec::new();
    while let Some(batch) = stream.next().await {
        paths.extend(batch.expect("managed path batch").paths);
    }
    paths
}

fn file_record(path: &str, id: u8, modified_secs: i64) -> LocalFileFact {
    LocalFileFact {
        path: TargetPath::new(path).expect("target path"),
        len: 4,
        freshness: freshness(4, modified_secs),
        segments: vec![segment(id)],
    }
}

fn observed_path(path: &str, len: u64, modified_secs: i64) -> InventoryObservedFile {
    InventoryObservedFile {
        path: TargetPath::new(path).expect("target path"),
        freshness: freshness(len, modified_secs),
    }
}

fn observed(path: &TargetPath, len: u64, modified_secs: i64) -> InventoryObservedFile {
    InventoryObservedFile {
        path: path.clone(),
        freshness: freshness(len, modified_secs),
    }
}

fn desired(record: &LocalFileFact) -> InventoryDesiredFile {
    InventoryDesiredFile {
        path: record.path.clone(),
        size_bytes: record.len,
        segments: record.segments.clone(),
    }
}

fn desired_path(path: &str, size_bytes: u64, id: u8) -> InventoryDesiredFile {
    InventoryDesiredFile {
        path: TargetPath::new(path).expect("target path"),
        size_bytes,
        segments: vec![segment(id)],
    }
}

fn freshness(len: u64, modified_secs: i64) -> FreshnessProof {
    FreshnessProof {
        len,
        modified_secs,
        modified_nanos: 0,
    }
}

fn segment(id: u8) -> LocalFileSegmentFact {
    let key = SegmentKey::new(
        ProfileFingerprint::new([id; 32]),
        OpaqueSegmentIdentity::new(vec![id; 16]).expect("identity"),
        4,
    )
    .expect("segment key");
    LocalFileSegmentFact {
        range: 0..4,
        validation: ValidationSpec {
            profile: key.profile,
            key: key.clone(),
            len: 4,
        },
        key,
    }
}
