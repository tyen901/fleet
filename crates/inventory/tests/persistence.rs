use fleet_inventory::FleetInventoryProvider;
use flux::{
    CheckInventory, ExpectedFileFact, InventoryReader, LocalFileFact, LocalFileSegmentFact,
    ManagedInventoryBatch, ManagedInventoryChange, ManagedInventoryWriter, OpaqueSegmentIdentity,
    ProfileFingerprint, SegmentKey, TargetFileVersion, TargetPath, ValidationSpec,
    VerifiedFactBatch, VerifiedFactChange, VerifiedFactWriter,
};
use futures_util::StreamExt;

#[tokio::test]
async fn verified_facts_persist_without_claiming_managed_path_ownership() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    let inventory = FleetInventoryProvider::open_or_recreate(&db_path).expect("open inventory");
    let fact = fact("observed/addon.pbo", 1);

    inventory
        .apply_verified_batch(VerifiedFactBatch {
            changes: vec![VerifiedFactChange::Upsert(fact.clone())],
        })
        .await
        .expect("persist verified fact");

    assert_eq!(
        lookup_file(&inventory, &fact.path).await,
        Some(fact.clone())
    );
    assert!(managed_paths(&inventory, 2).await.is_empty());
    let segment_results = inventory
        .lookup_segments(&[fact.segments[0].key.clone()], 4)
        .await
        .expect("lookup segment");
    assert_eq!(segment_results.len(), 1);
    assert_eq!(segment_results[0].hits.len(), 1);
    assert_eq!(segment_results[0].hits[0].path, fact.path);

    drop(inventory);
    let reopened = FleetInventoryProvider::open_existing(&db_path).expect("reopen inventory");
    assert_eq!(lookup_file(&reopened, &fact.path).await, Some(fact));
    assert!(managed_paths(&reopened, 1).await.is_empty());
}

#[tokio::test]
async fn fact_and_managed_path_mutations_have_distinct_ownership_semantics() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    let inventory = FleetInventoryProvider::open_or_recreate(&db_path).expect("open inventory");
    let managed = fact("managed/addon.pbo", 2);

    inventory
        .apply_managed_batch(ManagedInventoryBatch {
            changes: vec![ManagedInventoryChange::Manage(managed.clone())],
        })
        .await
        .expect("manage fact");
    assert_eq!(
        managed_paths(&inventory, 1).await,
        vec![managed.path.clone()]
    );
    assert_eq!(
        lookup_file(&inventory, &managed.path).await,
        Some(managed.clone())
    );

    inventory
        .apply_verified_batch(VerifiedFactBatch {
            changes: vec![VerifiedFactChange::Remove(managed.path.clone())],
        })
        .await
        .expect("remove reusable fact only");
    assert_eq!(lookup_file(&inventory, &managed.path).await, None);
    assert_eq!(
        managed_paths(&inventory, 1).await,
        vec![managed.path.clone()]
    );

    inventory
        .apply_managed_batch(ManagedInventoryBatch {
            changes: vec![ManagedInventoryChange::Delete(managed.path.clone())],
        })
        .await
        .expect("delete managed path");
    assert_eq!(lookup_file(&inventory, &managed.path).await, None);
    assert!(managed_paths(&inventory, 1).await.is_empty());
}

#[tokio::test]
async fn managed_batches_are_sorted_bounded_and_persisted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    let inventory = FleetInventoryProvider::open_or_recreate(&db_path).expect("open inventory");
    let facts = [
        fact("z/last.pbo", 3),
        fact("a/first.pbo", 4),
        fact("m/middle.pbo", 5),
    ];

    inventory
        .apply_managed_batch(ManagedInventoryBatch {
            changes: facts
                .iter()
                .cloned()
                .map(ManagedInventoryChange::Manage)
                .collect(),
        })
        .await
        .expect("persist managed facts");

    assert_eq!(
        managed_paths(&inventory, 2).await,
        vec![
            TargetPath::new("a/first.pbo").expect("path"),
            TargetPath::new("m/middle.pbo").expect("path"),
            TargetPath::new("z/last.pbo").expect("path"),
        ]
    );
}

#[tokio::test]
async fn fast_assessment_compares_expected_facts_and_managed_scope_in_one_query() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = FleetInventoryProvider::open_or_recreate(&temp.path().join("inventory.sqlite"))
        .expect("open inventory");
    let matching = fact("expected/matching.pbo", 1);
    let changed_manifest = fact("expected/changed.pbo", 2);
    let obsolete = fact("obsolete/old.pbo", 3);
    inventory
        .apply_managed_batch(ManagedInventoryBatch {
            changes: vec![
                ManagedInventoryChange::Manage(matching.clone()),
                ManagedInventoryChange::Manage(changed_manifest.clone()),
                ManagedInventoryChange::Manage(obsolete.clone()),
            ],
        })
        .await
        .expect("seed managed inventory");

    let replacement = fact("expected/changed.pbo", 4);
    let expected = [expected_fact(&matching), expected_fact(&replacement)];
    let assessment = inventory
        .assess_expected_state(&expected)
        .await
        .expect("assess expected state");

    assert_eq!(assessment.files.len(), 2);
    assert!(assessment.files[0].content_matches);
    assert_eq!(assessment.files[0].stored_version, Some(matching.version));
    assert!(!assessment.files[1].content_matches);
    assert_eq!(assessment.obsolete_paths, vec![obsolete.path]);
}

async fn lookup_file(
    inventory: &FleetInventoryProvider,
    path: &TargetPath,
) -> Option<LocalFileFact> {
    inventory
        .lookup_files(std::slice::from_ref(path))
        .await
        .expect("lookup file")
        .into_iter()
        .next()
        .expect("one lookup result")
}

async fn managed_paths(inventory: &FleetInventoryProvider, batch_size: usize) -> Vec<TargetPath> {
    let mut stream = inventory.managed_path_batches(batch_size);
    let mut paths = Vec::new();
    while let Some(batch) = stream.next().await {
        paths.extend(batch.expect("managed path batch").paths);
    }
    paths
}

fn fact(path: &str, id: u8) -> LocalFileFact {
    let profile = ProfileFingerprint::new([id; 32]);
    let key = SegmentKey::new(
        profile,
        OpaqueSegmentIdentity::new(vec![id]).expect("identity"),
        4,
    )
    .expect("segment key");
    LocalFileFact {
        path: TargetPath::new(path).expect("path"),
        version: TargetFileVersion::from_storage(4, vec![1, id]).expect("version"),
        segments: vec![LocalFileSegmentFact {
            range: 0..4,
            validation: ValidationSpec {
                profile,
                key: key.clone(),
                len: 4,
            },
            key,
        }],
    }
}

fn expected_fact(fact: &LocalFileFact) -> ExpectedFileFact {
    ExpectedFileFact {
        path: fact.path.clone(),
        len: fact.len(),
        segments: fact.segments.clone(),
    }
}
