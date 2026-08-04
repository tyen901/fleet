use fleet_inventory::{
    InventoryDesiredFile, InventoryObservedFile, InventoryRefreshWrite, MaterializationInventory,
};
use flux::{
    FreshnessProof, LocalFileFact, LocalFileSegmentFact, OpaqueSegmentIdentity, ProfileFingerprint,
    SegmentKey, TargetPath, ValidationSpec,
};
use futures_util::StreamExt;

#[tokio::test]
async fn managed_path_can_exist_without_reusable_fact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let path = TargetPath::new("mods/extra.pbo").expect("target path");

    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![path.clone()],
            ..Default::default()
        })
        .expect("apply refresh");

    assert!(inventory
        .lookup_files(std::slice::from_ref(&path))
        .expect("lookup file")[0]
        .is_none());
    assert_eq!(managed_paths(&inventory).await, vec![path]);
}

#[tokio::test]
async fn apply_refresh_removes_paths_not_in_new_snapshot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let keep = TargetPath::new("mods/keep.pbo").expect("target path");
    let remove = TargetPath::new("mods/remove.pbo").expect("target path");

    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![keep.clone(), remove],
            ..Default::default()
        })
        .expect("seed managed paths");
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![keep.clone()],
            ..Default::default()
        })
        .expect("replace managed paths");

    assert_eq!(managed_paths(&inventory).await, vec![keep]);
}

#[tokio::test]
async fn apply_refresh_cascades_removed_file_and_segment_rows() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let keep = local_fact("mods/keep.pbo", segment_key_with_id(8));
    let remove_key = segment_key_with_id(9);
    let remove = local_fact("mods/remove.pbo", remove_key.clone());
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![keep.path.clone(), remove.path.clone()],
            upsert_facts: vec![keep.clone(), remove.clone()],
            ..Default::default()
        })
        .expect("seed facts");

    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![keep.path.clone()],
            ..Default::default()
        })
        .expect("replace managed paths");

    assert_eq!(managed_paths(&inventory).await, vec![keep.path.clone()]);
    assert!(inventory
        .lookup_files(std::slice::from_ref(&remove.path))
        .expect("lookup removed file")[0]
        .is_none());
    assert!(inventory
        .lookup_segments(std::slice::from_ref(&remove_key), 10)
        .expect("lookup removed segments")[0]
        .hits
        .is_empty());
}

#[tokio::test]
async fn reset_removes_managed_paths_and_reusable_facts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    let inventory = MaterializationInventory::open(&db_path).expect("open");
    let fact = local_fact("mods/known.pbo", segment_key());

    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![fact.path.clone()],
            upsert_facts: vec![fact.clone()],
            ..Default::default()
        })
        .expect("seed facts");
    assert!(inventory
        .lookup_files(std::slice::from_ref(&fact.path))
        .expect("lookup seeded file")[0]
        .is_some());

    let reset = MaterializationInventory::reset(&db_path).expect("reset");

    assert!(managed_paths(&reset).await.is_empty());
    assert!(reset
        .lookup_files(std::slice::from_ref(&fact.path))
        .expect("lookup reset file")[0]
        .is_none());
}

#[tokio::test]
async fn apply_refresh_removing_reusable_fact_preserves_managed_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let fact = local_fact("mods/a.pbo", segment_key());
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![fact.path.clone()],
            upsert_facts: vec![fact.clone()],
            ..Default::default()
        })
        .expect("seed");

    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![fact.path.clone()],
            remove_reusable_facts: vec![fact.path.clone()],
            ..Default::default()
        })
        .expect("remove reusable fact");

    assert_eq!(managed_paths(&inventory).await, vec![fact.path.clone()]);
    assert!(inventory
        .lookup_files(std::slice::from_ref(&fact.path))
        .expect("lookup file")[0]
        .is_none());
}

#[test]
fn schema_rejects_negative_file_len() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    let inventory = MaterializationInventory::open(&db_path).expect("open");
    let fact = local_fact("mods/a.pbo", segment_key_with_id(4));
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![fact.path.clone()],
            upsert_facts: vec![fact.clone()],
            ..Default::default()
        })
        .expect("seed");

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    assert!(conn
        .execute(
            "UPDATE files SET len=-1 WHERE rel_path=?1",
            rusqlite::params![fact.path.as_str()],
        )
        .is_err());
}

#[test]
fn schema_rejects_invalid_modified_nanos() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    let inventory = MaterializationInventory::open(&db_path).expect("open");
    let fact = local_fact("mods/a.pbo", segment_key_with_id(10));
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![fact.path.clone()],
            upsert_facts: vec![fact.clone()],
            ..Default::default()
        })
        .expect("seed");

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    assert!(conn
        .execute(
            "UPDATE files SET modified_nanos=-1 WHERE rel_path=?1",
            rusqlite::params![fact.path.as_str()],
        )
        .is_err());
}

#[tokio::test]
async fn apply_refresh_rolls_back_when_transaction_fails_after_snapshot_mutation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    let inventory = MaterializationInventory::open(&db_path).expect("open");
    let initial = local_fact("mods/ok.pbo", segment_key_with_id(5));
    let attempted = local_fact("mods/new.pbo", segment_key_with_id(6));
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![initial.path.clone()],
            upsert_facts: vec![initial.clone()],
            ..Default::default()
        })
        .expect("seed");
    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    conn.execute_batch(
        "CREATE TRIGGER fail_refresh_file_upsert
         BEFORE INSERT ON files
         BEGIN
           SELECT RAISE(FAIL, 'forced refresh failure');
         END;",
    )
    .expect("create trigger");

    assert!(inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![attempted.path.clone()],
            remove_reusable_facts: vec![initial.path.clone()],
            upsert_facts: vec![attempted.clone()],
        })
        .is_err());

    assert_eq!(managed_paths(&inventory).await, vec![initial.path.clone()]);
    assert_eq!(
        inventory
            .lookup_files(std::slice::from_ref(&initial.path))
            .expect("lookup initial"),
        vec![Some(initial)]
    );
    assert!(inventory
        .lookup_files(std::slice::from_ref(&attempted.path))
        .expect("lookup attempted")[0]
        .is_none());
}

#[test]
fn refresh_planning_classifies_kept_scan_candidates_and_stale_facts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let kept = local_fact("mods/kept.pbo", segment_key_with_id(11));
    let modified = local_fact("mods/modified.pbo", segment_key_with_id(12));
    let missing = local_fact("mods/missing.pbo", segment_key_with_id(13));
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![
                kept.path.clone(),
                modified.path.clone(),
                missing.path.clone(),
            ],
            upsert_facts: vec![kept.clone(), modified.clone(), missing.clone()],
            ..Default::default()
        })
        .expect("seed");

    let observed = vec![
        observed(&kept, kept.freshness.modified_secs),
        observed(&modified, 99),
        InventoryObservedFile {
            path: TargetPath::new("mods/extra.pbo").expect("target path"),
            len: 8,
            freshness: FreshnessProof {
                len: 8,
                modified_secs: 1,
                modified_nanos: 0,
            },
        },
    ];
    let desired = vec![InventoryDesiredFile {
        path: modified.path.clone(),
        size_bytes: modified.len,
    }];

    let plan = inventory.plan_refresh(&observed, &desired).expect("plan");

    assert_eq!(
        target_strings(&plan.managed_paths),
        vec!["mods/kept.pbo", "mods/modified.pbo", "mods/extra.pbo"]
    );
    assert_eq!(
        target_strings(&plan.kept_reusable_facts),
        vec!["mods/kept.pbo"]
    );
    assert_eq!(plan.scan_candidate_positions, vec![1]);
    assert_eq!(
        target_strings(&plan.remove_reusable_facts),
        vec!["mods/missing.pbo", "mods/modified.pbo"]
    );
    assert_eq!(plan.missing_stale_paths, vec!["mods/missing.pbo"]);
    assert_eq!(plan.modified_stale_paths, vec!["mods/modified.pbo"]);
}

#[test]
fn audit_classifies_valid_missing_and_modified_with_observed_input() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let valid = local_fact("mods/valid.pbo", segment_key_with_id(16));
    let modified = local_fact("mods/modified.pbo", segment_key_with_id(17));
    let missing = local_fact("mods/missing.pbo", segment_key_with_id(18));
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![
                valid.path.clone(),
                modified.path.clone(),
                missing.path.clone(),
            ],
            upsert_facts: vec![valid.clone(), modified.clone(), missing],
            ..Default::default()
        })
        .expect("seed");

    let report = inventory
        .audit_observed_files(&[observed(&valid, 1), observed(&modified, 2)])
        .expect("audit");

    assert_eq!(
        report.observed_paths,
        vec!["mods/valid.pbo", "mods/modified.pbo"]
    );
    assert_eq!(report.valid_reusable_paths, vec!["mods/valid.pbo"]);
    assert_eq!(report.missing_reusable_paths, vec!["mods/missing.pbo"]);
    assert_eq!(report.modified_reusable_paths, vec!["mods/modified.pbo"]);
}

fn open_inventory(temp: &tempfile::TempDir) -> MaterializationInventory {
    MaterializationInventory::open(&temp.path().join("inventory.sqlite")).expect("open")
}

async fn managed_paths(inventory: &MaterializationInventory) -> Vec<TargetPath> {
    let mut stream = inventory.managed_path_batches(2);
    let mut paths = Vec::new();
    while let Some(batch) = stream.next().await {
        paths.extend(batch.expect("managed path batch").paths);
    }
    paths
}

fn local_fact(path: &str, key: SegmentKey) -> LocalFileFact {
    LocalFileFact {
        path: TargetPath::new(path).expect("target path"),
        len: 4,
        freshness: FreshnessProof {
            len: 4,
            modified_secs: 1,
            modified_nanos: 0,
        },
        segments: vec![LocalFileSegmentFact {
            range: 0..4,
            key: key.clone(),
            validation: validation(key),
        }],
    }
}

fn observed(fact: &LocalFileFact, modified_secs: i64) -> InventoryObservedFile {
    InventoryObservedFile {
        path: fact.path.clone(),
        len: fact.len,
        freshness: FreshnessProof {
            len: fact.len,
            modified_secs,
            modified_nanos: fact.freshness.modified_nanos,
        },
    }
}

fn segment_key() -> SegmentKey {
    segment_key_with_id(7)
}

fn segment_key_with_id(id: u8) -> SegmentKey {
    SegmentKey::new(
        ProfileFingerprint::new([id; 32]),
        OpaqueSegmentIdentity::new(vec![id; 16]).expect("identity"),
        4,
    )
    .expect("segment key")
}

fn validation(key: SegmentKey) -> ValidationSpec {
    ValidationSpec {
        profile: key.profile,
        key,
        len: 4,
    }
}

fn target_strings(paths: &[TargetPath]) -> Vec<&str> {
    paths.iter().map(TargetPath::as_str).collect()
}
