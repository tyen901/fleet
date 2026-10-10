use fleet_inventory::FleetInventory;
use flux::{
    ConfirmedFile, ContentKey, Error, ErrorKind, FileSpec, Inventory, Manifest, ObservationToken,
    ObservedFile, ProfileId, Segment, TargetPath,
};
use rusqlite::Connection;

const PROFILE: ProfileId = ProfileId([7; 32]);

fn file(path: &str, parts: &[(u8, u64)]) -> FileSpec {
    let mut offset = 0;
    let segments = parts
        .iter()
        .map(|(identity, length)| {
            let segment = Segment {
                offset,
                key: ContentKey::new(PROFILE, vec![*identity], *length).unwrap(),
            };
            offset += length;
            segment
        })
        .collect();
    FileSpec {
        path: TargetPath::new(path).unwrap(),
        length: offset,
        segments,
    }
}

fn manifest(files: Vec<FileSpec>) -> Manifest {
    Manifest::new(PROFILE, files, vec![]).unwrap()
}

fn evidence(version: u8, length: u64) -> ObservedFile {
    ObservedFile::new(
        ObservationToken::from_bytes(vec![version; 64]).unwrap(),
        length,
        PROFILE,
    )
}

fn observe(inventory: &FleetInventory, spec: &FileSpec, version: u8) {
    let mut writer = inventory.begin_observation(&spec.path).unwrap();
    writer.append(&spec.segments).unwrap();
    writer.finish(evidence(version, spec.length)).unwrap();
}

fn terminal(inventory: &FleetInventory, manifest: &Manifest) {
    inventory
        .commit_terminal(manifest, &mut |sink| {
            for file in manifest.files() {
                let observation = inventory
                    .observed(&file.path)?
                    .ok_or_else(|| Error::new(ErrorKind::State, "missing test observation"))?;
                sink(ConfirmedFile {
                    path: &file.path,
                    segments: &file.segments,
                    observation,
                })?;
            }
            Ok(())
        })
        .unwrap();
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}

#[test]
fn known_recipe_is_shared_by_observed_files_and_lookup_uses_occurrences() {
    let temp = tempfile::tempdir().unwrap();
    let inventory = FleetInventory::open(
        &temp.path().join("observations.sqlite"),
        temp.path(),
        PROFILE,
    )
    .unwrap();
    let first = file("a", &[(1, 4), (2, 3)]);
    let second = file("b", &[(1, 4), (2, 3)]);
    let goal = manifest(vec![first.clone(), second.clone()]);

    inventory.register_manifest(&goal).unwrap();
    observe(&inventory, &first, 1);
    observe(&inventory, &second, 2);

    let conn = Connection::open(temp.path().join("observations.sqlite")).unwrap();
    assert_eq!(count(&conn, "content"), 2);
    assert_eq!(count(&conn, "recipes"), 1);
    assert_eq!(count(&conn, "recipe_segments"), 2);
    assert_eq!(count(&conn, "observed_files"), 2);
    let hits = inventory.lookup(&first.segments[0].key, 10).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].path.as_str(), "a");
    assert_eq!(hits[1].path.as_str(), "b");
}

#[test]
fn partial_corruption_creates_one_recipe_and_reuses_matching_content() {
    let temp = tempfile::tempdir().unwrap();
    let inventory = FleetInventory::open(
        &temp.path().join("observations.sqlite"),
        temp.path(),
        PROFILE,
    )
    .unwrap();
    let clean = file("clean", &[(1, 4), (2, 3)]);
    let damaged = file("damaged", &[(1, 4), (3, 3)]);
    let goal = manifest(vec![clean.clone()]);
    inventory.register_manifest(&goal).unwrap();
    observe(&inventory, &clean, 1);
    observe(&inventory, &damaged, 2);

    let conn = Connection::open(temp.path().join("observations.sqlite")).unwrap();
    assert_eq!(count(&conn, "recipes"), 2);
    assert_eq!(count(&conn, "content"), 3);
    assert_eq!(count(&conn, "recipe_segments"), 4);
    let shared_recipe_count: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT rs.recipe_id)
             FROM recipe_segments rs
             JOIN content c ON c.id = rs.content_id
             WHERE c.identity = ?1 AND c.length = ?2",
            rusqlite::params![vec![1u8], 4i64],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(shared_recipe_count, 2);
    assert_eq!(
        inventory.lookup(&clean.segments[0].key, 10).unwrap().len(),
        2
    );
}

#[test]
fn registering_a_new_goal_keeps_installed_old_recipe() {
    let temp = tempfile::tempdir().unwrap();
    let inventory = FleetInventory::open(
        &temp.path().join("observations.sqlite"),
        temp.path(),
        PROFILE,
    )
    .unwrap();
    let old = file("old", &[(1, 4), (2, 3)]);
    let new = file("new", &[(1, 4), (3, 3)]);
    let old_goal = manifest(vec![old.clone()]);
    inventory.register_manifest(&old_goal).unwrap();
    observe(&inventory, &old, 1);
    terminal(&inventory, &old_goal);

    let new_goal = manifest(vec![old.clone(), new.clone()]);
    inventory.register_manifest(&new_goal).unwrap();
    observe(&inventory, &new, 2);
    let conn = Connection::open(temp.path().join("observations.sqlite")).unwrap();
    assert_eq!(count(&conn, "recipes"), 2);
    terminal(&inventory, &new_goal);
    assert_eq!(count(&conn, "recipes"), 2);
    assert!(inventory.observed(&old.path).unwrap().is_some());
    assert!(inventory.observed(&new.path).unwrap().is_some());
}

#[test]
fn terminal_failure_rolls_back_all_fact_changes() {
    let temp = tempfile::tempdir().unwrap();
    let inventory = FleetInventory::open(
        &temp.path().join("observations.sqlite"),
        temp.path(),
        PROFILE,
    )
    .unwrap();
    let spec = file("file", &[(1, 4)]);
    let goal = manifest(vec![spec.clone()]);
    inventory.register_manifest(&goal).unwrap();
    observe(&inventory, &spec, 1);
    terminal(&inventory, &goal);
    let conn = Connection::open(temp.path().join("observations.sqlite")).unwrap();
    let original_id: i64 = conn
        .query_row(
            "SELECT id FROM observed_files WHERE path = ?1",
            [&spec.path.as_str()],
            |row| row.get(0),
        )
        .unwrap();

    let failed = inventory.commit_terminal(&goal, &mut |sink| {
        sink(ConfirmedFile {
            path: &spec.path,
            segments: &spec.segments,
            observation: evidence(2, spec.length),
        })?;
        Err(Error::new(ErrorKind::State, "producer failed"))
    });
    assert!(failed.is_err());
    assert_eq!(
        inventory.observed(&spec.path).unwrap(),
        Some(evidence(1, 4))
    );

    inventory
        .commit_terminal(&goal, &mut |sink| {
            sink(ConfirmedFile {
                path: &spec.path,
                segments: &spec.segments,
                observation: evidence(2, spec.length),
            })
        })
        .unwrap();
    assert_eq!(
        inventory.observed(&spec.path).unwrap(),
        Some(evidence(2, 4))
    );
    let updated_id: i64 = conn
        .query_row(
            "SELECT id FROM observed_files WHERE path = ?1",
            [&spec.path.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(updated_id, original_id);

    let omitted = inventory.commit_terminal(&goal, &mut |_sink| Ok(()));
    assert!(omitted.is_err());
    assert_eq!(
        inventory.observed(&spec.path).unwrap(),
        Some(evidence(2, 4))
    );
}

#[test]
fn incomplete_observation_is_invisible_and_previous_complete_fact_survives() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("observations.sqlite");
    let inventory = FleetInventory::open(&db, temp.path(), PROFILE).unwrap();
    let spec = file("file", &[(1, 4)]);
    observe(&inventory, &spec, 1);
    let mut writer = inventory.begin_observation(&spec.path).unwrap();
    writer.append(&spec.segments).unwrap();
    assert_eq!(
        inventory.observed(&spec.path).unwrap(),
        Some(evidence(1, 4))
    );
    drop(writer);
    assert_eq!(
        inventory.observed(&spec.path).unwrap(),
        Some(evidence(1, 4))
    );
    drop(inventory);
    let reopened = FleetInventory::open(&db, temp.path(), PROFILE).unwrap();
    assert_eq!(reopened.observed(&spec.path).unwrap(), Some(evidence(1, 4)));
}

#[test]
fn observation_keeps_session_lock_until_finished_or_discarded() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("observations.sqlite");
    let inventory = FleetInventory::open(&db, temp.path(), PROFILE).unwrap();
    let spec = file("file", &[(1, 4)]);
    let mut writer = inventory.begin_observation(&spec.path).unwrap();
    writer.append(&spec.segments).unwrap();
    drop(inventory);
    assert!(FleetInventory::open(&db, temp.path(), PROFILE).is_err());
    drop(writer);
    FleetInventory::open(&db, temp.path(), PROFILE).unwrap();
}

#[test]
fn read_only_payload_measurement_shows_one_recipe_for_many_files() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("observations.sqlite");
    let inventory = FleetInventory::open(&db, temp.path(), PROFILE).unwrap();
    let files = (0..64)
        .map(|index| file(&format!("file-{index}"), &[(1, 4), (2, 3)]))
        .collect::<Vec<_>>();
    let goal = manifest(files.clone());
    inventory.register_manifest(&goal).unwrap();
    for (index, spec) in files.iter().enumerate() {
        observe(&inventory, spec, index as u8);
    }

    let conn = Connection::open(db).unwrap();
    let (files, recipes, recipe_segments, content): (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT
                 (SELECT COUNT(*) FROM observed_files),
                 (SELECT COUNT(*) FROM recipes),
                 (SELECT COUNT(*) FROM recipe_segments),
                 (SELECT COUNT(*) FROM content)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(files, 64);
    assert_eq!(recipes, 1);
    assert_eq!(recipe_segments, 2);
    assert_eq!(content, 2);
}
