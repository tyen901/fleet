use fleet_inventory::{InventoryRefreshWrite, MaterializationInventory};
use flux::{
    FinalizedFileFact, FreshnessProof, LocalFileFact, LocalFileSegmentFact, OpaqueSegmentIdentity,
    ProfileFingerprint, SegmentKey, TargetPath, TerminalInventoryBatch, ValidationSpec,
};
use futures_util::StreamExt;

#[tokio::test]
async fn terminal_finalized_inserts_managed_path_file_and_segments() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let key = segment_key();
    let fact = local_fact("mods/a.pbo", key.clone());

    let mut batch = TerminalInventoryBatch::default();
    batch.push_finalized(FinalizedFileFact {
        path: fact.path.clone(),
        len: fact.len,
        freshness: fact.freshness,
        segments: fact.segments.clone(),
    });
    inventory.apply_terminal_batch(batch).expect("finalize");

    assert_eq!(managed_paths(&inventory).await, vec![fact.path.clone()]);
    assert_eq!(
        inventory
            .lookup_files(std::slice::from_ref(&fact.path))
            .expect("lookup file"),
        vec![Some(fact.clone())]
    );
    assert_eq!(
        inventory
            .lookup_segments(std::slice::from_ref(&key), 10)
            .expect("lookup segments")[0]
            .hits
            .len(),
        1
    );
}

#[tokio::test]
async fn terminal_deleted_removes_managed_path_file_and_segments() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let key = segment_key();
    let fact = local_fact("mods/a.pbo", key.clone());
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![fact.path.clone()],
            upsert_facts: vec![fact.clone()],
            ..Default::default()
        })
        .expect("seed");

    let mut batch = TerminalInventoryBatch::default();
    batch.push_deleted(fact.path.clone());
    inventory.apply_terminal_batch(batch).expect("delete");

    assert!(managed_paths(&inventory).await.is_empty());
    assert!(inventory
        .lookup_files(std::slice::from_ref(&fact.path))
        .expect("lookup file")[0]
        .is_none());
    assert!(inventory
        .lookup_segments(std::slice::from_ref(&key), 10)
        .expect("lookup after")[0]
        .hits
        .is_empty());
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

fn segment_key() -> SegmentKey {
    SegmentKey::new(
        ProfileFingerprint::new([7; 32]),
        OpaqueSegmentIdentity::new(vec![7; 16]).expect("identity"),
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
