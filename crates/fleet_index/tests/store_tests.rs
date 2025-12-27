use fleet_index::ids::{enabled_mods_hash, normalize_repo_id, state_id};
use fleet_index::store::DesiredStateChange;
use fleet_index::{DesiredState, ExpectedFile, FleetIndex};
use std::fs;
use std::io::Write;

fn make_desired(repo_id: &str, mods: &[&str]) -> DesiredState {
    let mut mods_sorted: Vec<String> = mods.iter().map(|s| s.to_string()).collect();
    mods_sorted.sort();
    let enabled_hash = enabled_mods_hash(&mods_sorted);
    let repo_revision = "rev1".to_string();
    let state = state_id(repo_id, &enabled_hash, &repo_revision);
    DesiredState {
        repo_url: "https://example.invalid/repo".to_string(),
        repo_id: normalize_repo_id(repo_id),
        repo_revision,
        enabled_mods_hash: enabled_hash,
        state_id: state,
        updated_at_unix_s: 123,
    }
}

#[test]
fn desired_state_change_clears_verified_only_on_change() {
    let mut idx = FleetIndex::open_in_memory().unwrap();
    let desired = make_desired("abcd", &["@mod"]);

    idx.set_desired_state(desired.clone()).unwrap();
    idx.verified_set(&desired.state_id, 10).unwrap();

    let unchanged = idx.set_desired_state(desired.clone()).unwrap();
    match unchanged {
        DesiredStateChange::Unchanged { .. } => {}
        _ => panic!("expected unchanged"),
    }
    assert!(idx.verified_get().unwrap().is_some());

    let mut desired2 = desired.clone();
    desired2.state_id = "different".to_string();
    let changed = idx.set_desired_state(desired2).unwrap();
    match changed {
        DesiredStateChange::Changed { .. } => {}
        _ => panic!("expected changed"),
    }
    assert!(idx.verified_get().unwrap().is_none());
}

#[test]
fn expected_replace_all_atomic_on_invalid() {
    let mut idx = FleetIndex::open_in_memory().unwrap();
    let desired = make_desired("abcd", &["@mod"]);
    idx.set_desired_state(desired.clone()).unwrap();

    idx.expected_replace_all(
        &desired.state_id,
        vec![ExpectedFile {
            mod_id: "@mod".to_string(),
            rel_path: "addons/file.pbo".to_string(),
            size: 10,
        }],
    )
    .unwrap();

    let res = idx.expected_replace_all(
        &desired.state_id,
        vec![ExpectedFile {
            mod_id: "@mod".to_string(),
            rel_path: "../bad".to_string(),
            size: 10,
        }],
    );
    assert!(res.is_err());

    let mut count = 0;
    idx.expected_for_each(&desired.state_id, |_| {
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn open_or_recover_renames_sqlite_wal_shm() {
    let tmp = tempfile::tempdir().unwrap();
    let checkout_root = tmp.path();
    let sqlite_path = checkout_root.join("index.sqlite");
    {
        let conn = rusqlite::Connection::open(&sqlite_path).unwrap();
        conn.execute_batch("CREATE TABLE t(x INTEGER);").unwrap();
    }
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(&sqlite_path)
            .unwrap();
        f.write_all(b"not a sqlite db").unwrap();
        f.set_len(1).unwrap();
    }
    fs::write(checkout_root.join("index.sqlite-wal"), b"wal").unwrap();
    fs::write(checkout_root.join("index.sqlite-shm"), b"shm").unwrap();

    let _ = FleetIndex::open_or_recover_at_path(&sqlite_path).unwrap();

    let mut renamed = 0;
    for entry in fs::read_dir(checkout_root).unwrap() {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        if name.starts_with("index.sqlite.broken.")
            || name.starts_with("index.sqlite-wal.broken.")
            || name.starts_with("index.sqlite-shm.broken.")
        {
            renamed += 1;
        }
    }

    assert!(renamed >= 3);
}

#[test]
fn expected_replace_all_v2_round_trips_and_is_replace_all() {
    let mut idx = FleetIndex::open_in_memory().unwrap();

    idx.expected_replace_all_v2(
        "s1",
        vec![fleet_index::ExpectedFileRow {
            mod_id: "@m".to_string(),
            rel_path: "addons/a.pbo".to_string(),
            size: 10,
            file_md5: [1u8; 16],
        }],
        vec![
            fleet_index::ExpectedPartRow {
                mod_id: "@m".to_string(),
                rel_path: "addons/a.pbo".to_string(),
                idx: 0,
                offset: 0,
                len: 10,
                part_md5: [2u8; 16],
            },
            fleet_index::ExpectedPartRow {
                mod_id: "@m".to_string(),
                rel_path: "addons/a.pbo".to_string(),
                idx: 1,
                offset: 10,
                len: 0,
                part_md5: [3u8; 16],
            },
        ],
    )
    .unwrap();

    assert!(idx.baseline_exists("s1").unwrap());

    let files = idx.expected_load_v2("s1").unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].mod_id, "@m");
    assert_eq!(files[0].rel_path, "addons/a.pbo");
    assert_eq!(files[0].size, 10);
    assert_eq!(files[0].file_md5, [1u8; 16]);

    let parts = idx.expected_parts_load_v1("s1").unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].idx, 0);
    assert_eq!(parts[0].offset, 0);
    assert_eq!(parts[0].len, 10);
    assert_eq!(parts[1].idx, 1);
    assert_eq!(parts[1].offset, 10);
    assert_eq!(parts[1].len, 0);

    idx.expected_replace_all_v2("s1", std::iter::empty(), std::iter::empty())
        .unwrap();
    assert!(idx.baseline_exists("s1").unwrap());
    assert!(idx.expected_load_v2("s1").unwrap().is_empty());
    assert!(idx.expected_parts_load_v1("s1").unwrap().is_empty());
}
