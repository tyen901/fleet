use fleet_inventory::{FleetInventory, InventoryError};
use flux::{
    ConfirmedFile, ContentKey, Error, ErrorKind, FileSpec, Inventory, Manifest, ObservationToken,
    ObservedFile, ProfileId, Segment, TargetPath,
};

const PROFILE: ProfileId = ProfileId([1; 32]);

#[tokio::test]
async fn observation_completion_waits_for_sqlite_writer_and_preserves_readers() {
    let temp = tempfile::tempdir().unwrap();
    let inventory = FleetInventory::open(temp.path(), PROFILE).unwrap();
    let spec = file(0);
    let database = temp.path().join(".fleet/observations.sqlite");
    let blocker = rusqlite::Connection::open(&database).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut writer = inventory.begin_observation(&spec.path).unwrap();
    writer.append(&spec.segments).unwrap();
    let mut completion = tokio::spawn(writer.finish(evidence(1)));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut completion)
            .await
            .is_err()
    );
    assert!(inventory.observed(&spec.path).unwrap().is_none());
    blocker.execute_batch("COMMIT").unwrap();
    completion.await.unwrap().unwrap();
    assert_eq!(inventory.observed(&spec.path).unwrap(), Some(evidence(1)));
    drop(inventory);
    let reopened = FleetInventory::open(temp.path(), PROFILE).unwrap();
    assert_eq!(reopened.observed(&spec.path).unwrap(), Some(evidence(1)));
}

fn evidence(value: u8) -> ObservedFile {
    ObservedFile::new(
        ObservationToken::from_bytes(vec![value; 64]).unwrap(),
        4,
        PROFILE,
    )
}

fn file(index: usize) -> FileSpec {
    FileSpec {
        path: TargetPath::new(format!("file-{index}")).unwrap(),
        length: 4,
        segments: vec![Segment {
            offset: 0,
            key: ContentKey::new(PROFILE, vec![index as u8], 4).unwrap(),
        }],
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parallel_observations_remain_atomic_and_terminal_callbacks_can_read() {
    let temp = tempfile::tempdir().unwrap();
    let inventory = FleetInventory::open(temp.path(), PROFILE).unwrap();
    let manifest = Manifest::new(PROFILE, (0..8).map(file).collect(), vec![]).unwrap();
    std::thread::scope(|scope| {
        for spec in manifest.files() {
            let inventory = &inventory;
            let runtime = tokio::runtime::Handle::current();
            scope.spawn(move || {
                let mut writer = inventory.begin_observation(&spec.path).unwrap();
                writer.append(&spec.segments).unwrap();
                assert!(inventory.observed(&spec.path).unwrap().is_none());
                assert!(inventory
                    .lookup(&spec.segments[0].key, 1)
                    .unwrap()
                    .is_empty());
                runtime.block_on(writer.finish(evidence(1))).unwrap();
                assert_eq!(inventory.lookup(&spec.segments[0].key, 1).unwrap().len(), 1);
            });
        }
    });
    inventory
        .commit_terminal(&manifest, &mut |sink| {
            for spec in manifest.files() {
                let observation = inventory.observed(&spec.path)?.unwrap();
                let mut segments = Vec::new();
                inventory.segments(&spec.path, &mut |segment| {
                    segments.push(segment);
                    Ok(())
                })?;
                assert_eq!(segments, spec.segments);
                sink(ConfirmedFile {
                    path: &spec.path,
                    segments: &spec.segments,
                    observation,
                })?;
            }
            Ok(())
        })
        .unwrap();
    let failed = inventory.commit_terminal(&manifest, &mut |sink| {
        let spec = &manifest.files()[0];
        sink(ConfirmedFile {
            path: &spec.path,
            segments: &spec.segments,
            observation: evidence(2),
        })?;
        Err(Error::new(ErrorKind::State, "producer failed"))
    });
    assert!(failed.is_err());
    assert_eq!(
        inventory.observed(&manifest.files()[0].path).unwrap(),
        Some(evidence(1))
    );
}

#[test]
fn outstanding_observation_holds_session_exclusion_and_discards_partial_facts() {
    let temp = tempfile::tempdir().unwrap();
    let inventory = FleetInventory::open(temp.path(), PROFILE).unwrap();
    let spec = file(0);
    let mut writer = inventory.begin_observation(&spec.path).unwrap();
    writer.append(&spec.segments).unwrap();
    drop(inventory);
    assert!(matches!(
        FleetInventory::open(temp.path(), PROFILE),
        Err(InventoryError::Locked)
    ));
    drop(writer);
    let reopened = FleetInventory::open(temp.path(), PROFILE).unwrap();
    assert!(reopened.observed(&spec.path).unwrap().is_none());
    assert!(reopened
        .lookup(&spec.segments[0].key, 1)
        .unwrap()
        .is_empty());
}

#[test]
fn read_only_open_does_not_create_state_and_all_observations_are_target_local() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    assert!(FleetInventory::open_existing(&target, PROFILE)
        .unwrap()
        .is_none());
    assert!(!target.exists());
    let inventory = FleetInventory::open(&target, PROFILE).unwrap();
    assert!(target.join(".fleet/observations.sqlite").is_file());
    assert!(target.join(".fleet/observations.lock").is_file());
    assert!(!target.join("observations.sqlite").exists());
    drop(inventory);
    assert!(FleetInventory::open_existing(&target, PROFILE)
        .unwrap()
        .is_some());
}
