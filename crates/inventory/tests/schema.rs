use fleet_inventory::FleetInventoryProvider;
use rusqlite::Connection;

#[test]
fn obsolete_schema_is_destroyed_and_recreated_without_migration() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    let conn = Connection::open(&db_path).expect("open old inventory");
    conn.execute_batch(
        "PRAGMA user_version = 4;
         CREATE TABLE obsolete_state(value TEXT NOT NULL);
         INSERT INTO obsolete_state(value) VALUES ('must disappear');",
    )
    .expect("write obsolete schema");
    drop(conn);

    FleetInventoryProvider::open_or_recreate(&db_path).expect("replace obsolete inventory");

    let conn = Connection::open(&db_path).expect("open replacement");
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("user version");
    assert_eq!(version, 6);
    assert!(!table_exists(&conn, "obsolete_state"));
    assert!(table_exists(&conn, "managed_paths"));
}

#[test]
fn unversioned_schema_is_destroyed_and_recreated() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    let conn = Connection::open(&db_path).expect("open unversioned inventory");
    conn.execute_batch(
        "CREATE TABLE obsolete_state(value TEXT NOT NULL);
         INSERT INTO obsolete_state(value) VALUES ('must disappear');",
    )
    .expect("write unversioned schema");
    drop(conn);

    FleetInventoryProvider::open_or_recreate(&db_path).expect("replace unversioned inventory");

    let conn = Connection::open(&db_path).expect("open replacement");
    assert!(!table_exists(&conn, "obsolete_state"));
    assert!(table_exists(&conn, "managed_paths"));
}

#[test]
fn corrupt_database_is_destroyed_and_recreated() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    std::fs::write(&db_path, b"not a sqlite database").expect("write corrupt inventory");

    FleetInventoryProvider::open_or_recreate(&db_path).expect("replace corrupt inventory");

    let conn = Connection::open(&db_path).expect("open replacement");
    assert!(table_exists(&conn, "managed_paths"));
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
        |_| Ok(()),
    )
    .is_ok()
}
