use fleet_inventory::FleetInventory;
use flux::{
    ConfirmedFile, ContentKey, FileSpec, Inventory, Manifest, ObservationToken, ObservedFile,
    ProfileId, Result, Segment, TargetPath,
};

fn profile() -> ProfileId {
    ProfileId([7; 32])
}

fn token(byte: u8) -> ObservationToken {
    ObservationToken::from_bytes(vec![byte; 64]).expect("valid token")
}

fn path(value: &str) -> TargetPath {
    TargetPath::new(value).expect("valid target path")
}

fn segment(byte: u8) -> Segment {
    Segment {
        offset: 0,
        key: ContentKey::new(profile(), vec![byte], 1).expect("valid key"),
    }
}

#[test]
fn observations_reopen_and_reverse_lookup_from_sql_index() {
    let temp = tempfile::tempdir().expect("tempdir");
    let target = temp.path().join("target");
    std::fs::create_dir(&target).expect("target");
    let db = temp.path().join("observations.sqlite");
    let file = path("mod/file.pbo");
    let store = FleetInventory::open(&db, &target, profile()).expect("open");
    let mut writer = store.begin_observation(&file).expect("begin");
    writer.append(&[segment(3)]).expect("append");
    writer
        .finish(ObservedFile::new(token(1), 1, profile()))
        .expect("finish");
    assert_eq!(
        store.observed(&file).expect("observed").unwrap().version(),
        &token(1)
    );
    drop(store);

    let reopened = FleetInventory::open(&db, &target, profile()).expect("reopen");
    let hits = reopened.lookup(&segment(3).key, 4).expect("lookup");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, file);
    assert_eq!(hits[0].offset, 0);
}

#[test]
fn failed_finish_keeps_previous_committed_fact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let target = temp.path().join("target");
    std::fs::create_dir(&target).expect("target");
    let db = temp.path().join("observations.sqlite");
    let file = path("file");
    let store = FleetInventory::open(&db, &target, profile()).expect("open");
    let mut first = store.begin_observation(&file).expect("begin");
    first.append(&[segment(1)]).expect("append");
    first
        .finish(ObservedFile::new(token(1), 1, profile()))
        .expect("finish");
    let mut failed = store.begin_observation(&file).expect("begin");
    failed.append(&[segment(2)]).expect("append");
    assert!(failed
        .finish(ObservedFile::new(token(2), 2, profile()))
        .is_err());
    assert_eq!(store.lookup(&segment(1).key, 4).expect("lookup").len(), 1);
    assert!(store.lookup(&segment(2).key, 4).expect("lookup").is_empty());
}

#[test]
fn terminal_transaction_rolls_back_and_success_prunes_with_empty_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let target = temp.path().join("target");
    std::fs::create_dir(&target).expect("target");
    let db = temp.path().join("observations.sqlite");
    let obsolete = path("obsolete");
    let new_target = path("new");
    let empty_target = path("empty");
    let old_segment = segment(1);
    let new_segment = segment(2);
    let store = FleetInventory::open(&db, &target, profile()).expect("open");
    let mut writer = store.begin_observation(&obsolete).expect("begin");
    writer
        .append(std::slice::from_ref(&old_segment))
        .expect("append");
    writer
        .finish(ObservedFile::new(token(3), 1, profile()))
        .expect("finish");

    let manifest: &'static Manifest = Box::leak(Box::new(
        Manifest::new(
            profile(),
            vec![
                FileSpec {
                    path: new_target.clone(),
                    length: 1,
                    segments: vec![new_segment.clone()],
                },
                FileSpec {
                    path: empty_target.clone(),
                    length: 0,
                    segments: Vec::new(),
                },
            ],
            Vec::new(),
        )
        .expect("manifest"),
    ));

    let mut failing = |sink: &mut dyn FnMut(ConfirmedFile<'static>) -> Result<()>| {
        let file = manifest
            .files()
            .iter()
            .find(|file| file.path == new_target)
            .expect("new file");
        sink(ConfirmedFile {
            path: &file.path,
            segments: &file.segments,
            observation: ObservedFile::new(token(4), 1, profile()),
        })?;
        Err(flux::Error::new(
            flux::ErrorKind::Validation,
            "terminal producer failed",
        ))
    };
    assert!(store.commit_terminal(manifest, &mut failing).is_err());
    assert_eq!(store.lookup(&old_segment.key, 4).expect("lookup").len(), 1);
    assert!(store
        .lookup(&new_segment.key, 4)
        .expect("lookup")
        .is_empty());

    let mut succeeding = |sink: &mut dyn FnMut(ConfirmedFile<'static>) -> Result<()>| {
        let new_file = manifest
            .files()
            .iter()
            .find(|file| file.path == new_target)
            .expect("new file");
        sink(ConfirmedFile {
            path: &new_file.path,
            segments: &new_file.segments,
            observation: ObservedFile::new(token(5), 1, profile()),
        })?;
        let empty_file = manifest
            .files()
            .iter()
            .find(|file| file.path == empty_target)
            .expect("empty file");
        sink(ConfirmedFile {
            path: &empty_file.path,
            segments: &empty_file.segments,
            observation: ObservedFile::new(token(6), 0, profile()),
        })
    };
    store
        .commit_terminal(manifest, &mut succeeding)
        .expect("terminal commit");
    assert!(store
        .lookup(&old_segment.key, 4)
        .expect("lookup")
        .is_empty());
    assert_eq!(store.lookup(&new_segment.key, 4).expect("lookup").len(), 1);
    assert_eq!(
        store
            .observed(&empty_target)
            .expect("observed")
            .expect("empty fact")
            .length(),
        0
    );
}

#[test]
fn live_store_excludes_second_session_until_released() {
    let temp = tempfile::tempdir().expect("tempdir");
    let target = temp.path().join("target");
    std::fs::create_dir(&target).expect("target");
    let db = temp.path().join("observations.sqlite");
    let store = FleetInventory::open(&db, &target, profile()).expect("open");
    let file = path("live");
    let writer = store.begin_observation(&file).expect("begin");
    assert!(matches!(
        FleetInventory::open(&db, &target, profile()),
        Err(fleet_inventory::InventoryError::Locked)
    ));
    drop(writer);
}

#[test]
fn target_binding_rejects_reuse_for_another_destination() {
    let temp = tempfile::tempdir().expect("tempdir");
    let target = temp.path().join("target");
    let other_target = temp.path().join("other-target");
    std::fs::create_dir(&target).expect("target");
    std::fs::create_dir(&other_target).expect("other target");
    let db = temp.path().join("observations.sqlite");
    let store = FleetInventory::open(&db, &target, profile()).expect("open");
    drop(store);
    assert!(matches!(
        FleetInventory::open(&db, &other_target, profile()),
        Err(fleet_inventory::InventoryError::Incompatible)
    ));
}
