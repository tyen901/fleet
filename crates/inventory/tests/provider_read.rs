use fleet_inventory::{InventoryRefreshWrite, MaterializationInventory};
use flux::{
    FreshnessProof, LocalFileFact, LocalFileSegmentFact, ManagedPathBatch, OpaqueSegmentIdentity,
    ProfileFingerprint, SegmentKey, TargetPath, ValidationSpec,
};
use futures_util::StreamExt;

#[test]
fn lookup_files_reads_reusable_file_with_segments() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let fact = local_fact("mods/a.pbo", segment_key_with_id(1));
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![fact.path.clone()],
            upsert_facts: vec![fact.clone()],
            ..Default::default()
        })
        .expect("seed");

    let files = inventory
        .lookup_files(std::slice::from_ref(&fact.path))
        .expect("lookup files");

    assert_eq!(files, vec![Some(fact)]);
}

#[test]
fn lookup_files_preserves_request_order_and_duplicate_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let a = local_fact("mods/a.pbo", segment_key_with_id(21));
    let b = local_fact("mods/b.pbo", segment_key_with_id(22));
    let missing = TargetPath::new("mods/missing.pbo").expect("target path");
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![a.path.clone(), b.path.clone()],
            upsert_facts: vec![a.clone(), b.clone()],
            ..Default::default()
        })
        .expect("seed");

    let files = inventory
        .lookup_files(&[b.path.clone(), missing, a.path.clone(), b.path.clone()])
        .expect("lookup files");

    assert_eq!(files, vec![Some(b.clone()), None, Some(a), Some(b)]);
}

#[test]
fn lookup_segments_returns_hits_for_matching_segment_key() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let key = segment_key_with_id(2);
    let a = local_fact("mods/a.pbo", key.clone());
    let b = local_fact("mods/b.pbo", key.clone());
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![a.path.clone(), b.path.clone()],
            upsert_facts: vec![a, b],
            ..Default::default()
        })
        .expect("seed");

    let result = inventory
        .lookup_segments(std::slice::from_ref(&key), 10)
        .expect("lookup segments");

    assert_eq!(result[0].hits.len(), 2);
}

#[test]
fn lookup_segments_respects_limit_per_key() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let key = segment_key_with_id(3);
    let a = local_fact("mods/a.pbo", key.clone());
    let b = local_fact("mods/b.pbo", key.clone());
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![b.path.clone(), a.path.clone()],
            upsert_facts: vec![b, a],
            ..Default::default()
        })
        .expect("seed");

    let result = inventory
        .lookup_segments(std::slice::from_ref(&key), 1)
        .expect("lookup segments");

    assert_eq!(result[0].hits.len(), 1);
    assert_eq!(result[0].hits[0].path.as_str(), "mods/a.pbo");
}

#[test]
fn lookup_segments_query_plan_uses_segment_lookup_index() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    let inventory = MaterializationInventory::open(&db_path).expect("open");
    let key = segment_key_with_id(30);
    let fact = local_fact("mods/a.pbo", key.clone());
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![fact.path.clone()],
            upsert_facts: vec![fact],
            ..Default::default()
        })
        .expect("seed");

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    conn.execute_batch(
        "CREATE TEMP TABLE requested_segment_keys (
             position INTEGER PRIMARY KEY NOT NULL,
             profile_fingerprint BLOB NOT NULL,
             identity_bytes BLOB NOT NULL,
             range_len INTEGER NOT NULL
         ) WITHOUT ROWID;
         INSERT INTO requested_segment_keys(position, profile_fingerprint, identity_bytes, range_len)
         VALUES (0, zeroblob(32), zeroblob(16), 4);",
    )
    .expect("temp keys");

    let plan = explain_details(
        &conn,
        "EXPLAIN QUERY PLAN
         WITH ranked_hits AS (
             SELECT rk.position,
                   mp.rel_path,
                    s.range_start,
                    s.range_len,
                    f.len,
                    f.modified_secs,
                    f.modified_nanos,
                    row_number() OVER (
                        PARTITION BY rk.position
                        ORDER BY mp.rel_path ASC, s.range_start ASC
                    ) AS hit_rank
             FROM requested_segment_keys rk
             JOIN file_segments s
               ON s.profile_fingerprint = rk.profile_fingerprint
              AND s.identity_bytes = rk.identity_bytes
              AND s.range_len = rk.range_len
             JOIN files f ON f.path_id = s.path_id
             JOIN managed_paths mp ON mp.id = s.path_id
         )
         SELECT position,
                rel_path,
                range_start,
                range_len,
                len,
                modified_secs,
                modified_nanos
         FROM ranked_hits
         WHERE hit_rank <= 1
         ORDER BY position ASC, hit_rank ASC",
    );

    assert!(
        plan.iter().any(|detail| detail
            .contains("SEARCH s USING COVERING INDEX idx_file_segments_lookup")),
        "{plan:#?}"
    );
}

#[test]
fn lookup_files_segment_query_plan_uses_segment_primary_key_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    let inventory = MaterializationInventory::open(&db_path).expect("open");
    let fact = local_fact("mods/a.pbo", segment_key_with_id(31));
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![fact.path.clone()],
            upsert_facts: vec![fact],
            ..Default::default()
        })
        .expect("seed");

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    conn.execute_batch(
        "CREATE TEMP TABLE requested_paths (
             position INTEGER PRIMARY KEY NOT NULL,
             rel_path TEXT NOT NULL
         ) WITHOUT ROWID;
         INSERT INTO requested_paths(position, rel_path) VALUES (0, 'mods/a.pbo');",
    )
    .expect("temp paths");

    let plan = explain_details(
        &conn,
        "EXPLAIN QUERY PLAN
         SELECT rp.position,
                s.range_start,
                s.range_len,
                s.profile_fingerprint,
                s.identity_bytes
         FROM requested_paths rp
         JOIN managed_paths mp ON mp.rel_path = rp.rel_path
         JOIN file_segments s ON s.path_id = mp.id
         ORDER BY rp.position ASC, s.segment_index ASC",
    );

    assert!(
        plan.iter()
            .any(|detail| detail.contains("SEARCH s USING PRIMARY KEY (path_id=?)")),
        "{plan:#?}"
    );
}

#[test]
fn lookup_files_rejects_corrupt_file_values() {
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
            "UPDATE files
             SET modified_nanos=1000000000
             WHERE path_id = (SELECT id FROM managed_paths WHERE rel_path=?1)",
            rusqlite::params![fact.path.as_str()],
        )
        .is_err());
}

#[test]
fn provider_read_rejects_negative_file_len() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    let inventory = MaterializationInventory::open(&db_path).expect("open");
    let key = segment_key_with_id(11);
    let fact = local_fact("mods/a.pbo", key.clone());
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
            "UPDATE files
             SET len=-1
             WHERE path_id = (SELECT id FROM managed_paths WHERE rel_path=?1)",
            rusqlite::params![fact.path.as_str()],
        )
        .is_err());
}

#[test]
fn provider_read_rejects_negative_segment_range_start() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    let inventory = MaterializationInventory::open(&db_path).expect("open");
    let key = segment_key_with_id(5);
    let fact = local_fact("mods/a.pbo", key.clone());
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![fact.path.clone()],
            upsert_facts: vec![fact],
            ..Default::default()
        })
        .expect("seed");

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    assert!(conn
        .execute(
            "UPDATE file_segments
             SET range_start=-1
             WHERE path_id = (SELECT id FROM managed_paths WHERE rel_path='mods/a.pbo')",
            [],
        )
        .is_err());
}

#[test]
fn provider_read_rejects_negative_segment_range_len() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    let inventory = MaterializationInventory::open(&db_path).expect("open");
    let key = segment_key_with_id(6);
    let fact = local_fact("mods/a.pbo", key.clone());
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![fact.path.clone()],
            upsert_facts: vec![fact],
            ..Default::default()
        })
        .expect("seed");

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    assert!(conn
        .execute(
            "UPDATE file_segments
             SET range_len=-1
             WHERE path_id = (SELECT id FROM managed_paths WHERE rel_path='mods/a.pbo')",
            [],
        )
        .is_err());
}

#[test]
fn provider_read_rejects_segment_range_overflow() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("inventory.sqlite");
    let inventory = MaterializationInventory::open(&db_path).expect("open");
    let key = segment_key_with_id(7);
    let fact = local_fact("mods/a.pbo", key.clone());
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![fact.path.clone()],
            upsert_facts: vec![fact],
            ..Default::default()
        })
        .expect("seed");

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    conn.execute(
        "UPDATE file_segments
         SET range_start=?1, range_len=4
         WHERE path_id = (SELECT id FROM managed_paths WHERE rel_path='mods/a.pbo')",
        rusqlite::params![i64::MAX],
    )
    .expect("corrupt range overflow");

    assert!(inventory
        .lookup_segments(std::slice::from_ref(&key), 10)
        .is_err());
}

#[tokio::test]
async fn managed_path_batches_reads_managed_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let path = TargetPath::new("extra.bin").expect("target path");
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![path.clone()],
            ..Default::default()
        })
        .expect("apply refresh");

    let mut stream = inventory.managed_path_batches(10);
    let ManagedPathBatch { paths } = stream.next().await.expect("batch").expect("result");
    assert_eq!(paths, vec![path]);
}

#[tokio::test]
async fn managed_path_batches_zero_batch_size_returns_no_batches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let path = TargetPath::new("extra.bin").expect("target path");
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: vec![path],
            ..Default::default()
        })
        .expect("apply refresh");

    let mut stream = inventory.managed_path_batches(0);
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn managed_path_batches_paginates_in_sorted_order() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inventory = open_inventory(&temp);
    let paths = [
        "mods/d.pbo",
        "mods/a.pbo",
        "mods/e.pbo",
        "mods/b.pbo",
        "mods/c.pbo",
    ]
    .into_iter()
    .map(|path| TargetPath::new(path).expect("target path"))
    .collect::<Vec<_>>();
    inventory
        .apply_refresh(InventoryRefreshWrite {
            managed_paths: paths,
            ..Default::default()
        })
        .expect("apply refresh");

    let mut stream = inventory.managed_path_batches(2);
    let first = stream
        .next()
        .await
        .expect("first")
        .expect("first batch")
        .paths;
    let second = stream
        .next()
        .await
        .expect("second")
        .expect("second batch")
        .paths;
    let third = stream
        .next()
        .await
        .expect("third")
        .expect("third batch")
        .paths;
    assert!(stream.next().await.is_none());

    assert_eq!(target_strings(&first), vec!["mods/a.pbo", "mods/b.pbo"]);
    assert_eq!(target_strings(&second), vec!["mods/c.pbo", "mods/d.pbo"]);
    assert_eq!(target_strings(&third), vec!["mods/e.pbo"]);
}

fn open_inventory(temp: &tempfile::TempDir) -> MaterializationInventory {
    MaterializationInventory::open(&temp.path().join("inventory.sqlite")).expect("open")
}

fn target_strings(paths: &[TargetPath]) -> Vec<&str> {
    paths.iter().map(TargetPath::as_str).collect()
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

fn explain_details(conn: &rusqlite::Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).expect("prepare explain");
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(3))
        .expect("query plan rows");
    rows.map(|row| row.expect("query plan detail")).collect()
}
