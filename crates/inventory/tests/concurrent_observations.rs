use fleet_inventory::{FleetInventory, InventoryError};
use flux::{
    ConfirmedFile, ContentKey, Error, ErrorKind, FileSpec, Inventory, Manifest, ObservationToken,
    ObservedFile, ProfileId, Segment, TargetPath,
};

const PROFILE: ProfileId = ProfileId([1; 32]);

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

#[test]
fn parallel_observations_remain_atomic_and_terminal_callbacks_can_read() {
    let temp = tempfile::tempdir().unwrap();
    let inventory =
        FleetInventory::open(&temp.path().join("facts.sqlite"), temp.path(), PROFILE).unwrap();
    let manifest = Manifest::new(PROFILE, (0..8).map(file).collect(), vec![]).unwrap();
    std::thread::scope(|scope| {
        for spec in manifest.files() {
            let inventory = &inventory;
            scope.spawn(move || {
                let mut writer = inventory.begin_observation(&spec.path).unwrap();
                writer.append(&spec.segments).unwrap();
                assert!(inventory.observed(&spec.path).unwrap().is_none());
                assert!(inventory
                    .lookup(&spec.segments[0].key, 1)
                    .unwrap()
                    .is_empty());
                writer.finish(evidence(1)).unwrap();
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
    let db = temp.path().join("facts.sqlite");
    let inventory = FleetInventory::open(&db, temp.path(), PROFILE).unwrap();
    let spec = file(0);
    let mut writer = inventory.begin_observation(&spec.path).unwrap();
    writer.append(&spec.segments).unwrap();
    drop(inventory);
    assert!(matches!(
        FleetInventory::open(&db, temp.path(), PROFILE),
        Err(InventoryError::Locked)
    ));
    drop(writer);
    let reopened = FleetInventory::open(&db, temp.path(), PROFILE).unwrap();
    assert!(reopened.observed(&spec.path).unwrap().is_none());
    assert!(reopened
        .lookup(&spec.segments[0].key, 1)
        .unwrap()
        .is_empty());
}
