use fleet_inventory::{InventoryRefreshWrite, MaterializationInventory};
use flux::{
    FreshnessProof, LocalFileFact, LocalFileSegmentFact, OpaqueSegmentIdentity, ProfileFingerprint,
    SegmentKey, TargetPath, TerminalInventoryBatch, ValidationSpec,
};
use rusqlite::Connection;

#[test]
fn opening_old_v1_inventory_hard_resets_to_required_v4_schema() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    {
        let conn = Connection::open(&db_path).expect("open old db");
        conn.execute_batch(
            "PRAGMA user_version = 1;
             CREATE TABLE files (
                rel_path TEXT PRIMARY KEY NOT NULL,
                len INTEGER NOT NULL,
                modified_secs INTEGER NOT NULL,
                modified_nanos INTEGER NOT NULL
             );
             CREATE TABLE file_segments (
                rel_path TEXT NOT NULL REFERENCES files(rel_path) ON DELETE CASCADE,
                segment_index INTEGER NOT NULL,
                range_start INTEGER NOT NULL,
                range_len INTEGER NOT NULL,
                profile_fingerprint BLOB NOT NULL,
                identity_bytes BLOB NOT NULL,
                PRIMARY KEY (rel_path, segment_index)
             );
             INSERT INTO files(rel_path, len, modified_secs, modified_nanos)
             VALUES ('mods/old.pbo', 4, 1, 0);
             INSERT INTO file_segments(
                rel_path, segment_index, range_start, range_len, profile_fingerprint, identity_bytes
             ) VALUES ('mods/old.pbo', 0, 0, 4, zeroblob(32), zeroblob(16));",
        )
        .expect("create old schema");
    }

    let inventory = MaterializationInventory::open(&db_path).expect("open inventory");
    let conn = Connection::open(&db_path).expect("open recreated db");
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("user version");
    assert_eq!(version, 4);
    assert!(table_exists(&conn, "managed_paths"));
    assert!(index_exists(&conn, "idx_file_segments_lookup"));
    assert!(!index_exists(&conn, "idx_managed_paths_rel_path"));
    assert!(has_cascade_fk(
        &conn,
        "files",
        "managed_paths",
        "path_id",
        "id"
    ));
    assert!(has_cascade_fk(
        &conn,
        "file_segments",
        "files",
        "path_id",
        "path_id"
    ));
    let old_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM files f
             JOIN managed_paths mp ON mp.id = f.path_id
             WHERE mp.rel_path='mods/old.pbo'",
            [],
            |row| row.get(0),
        )
        .expect("old row count");
    assert_eq!(old_rows, 0);

    let key = segment_key();
    let fact = local_fact("mods/a.pbo", key.clone());
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![fact.path.clone()],
            upsert_facts: vec![fact.clone()],
            ..Default::default()
        })
        .expect("seed recreated db");
    let mut batch = TerminalInventoryBatch::default();
    batch.push_deleted(fact.path.clone());
    inventory.apply_terminal_batch(batch).expect("delete");
    assert!(inventory
        .lookup_segments(std::slice::from_ref(&key), 10)
        .expect("lookup segments")[0]
        .hits
        .is_empty());
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
        |_| Ok(()),
    )
    .is_ok()
}

fn index_exists(conn: &Connection, index: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1",
        [index],
        |_| Ok(()),
    )
    .is_ok()
}

fn has_cascade_fk(
    conn: &Connection,
    table: &str,
    referenced: &str,
    from_column: &str,
    to_column: &str,
) -> bool {
    let mut stmt = conn
        .prepare(&format!("PRAGMA foreign_key_list({table})"))
        .expect("foreign key list");
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(6)?,
            ))
        })
        .expect("foreign key rows");
    for row in rows {
        let (table, from, to, on_delete) = row.expect("foreign key row");
        if table == referenced
            && from == from_column
            && to == to_column
            && on_delete.eq_ignore_ascii_case("CASCADE")
        {
            return true;
        }
    }
    false
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
