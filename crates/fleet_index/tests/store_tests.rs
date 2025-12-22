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
    let fleet_dir = checkout_root.join(".fleet");
    fs::create_dir_all(&fleet_dir).unwrap();

    let sqlite_path = fleet_dir.join("index.sqlite");
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
    fs::write(fleet_dir.join("index.sqlite-wal"), b"wal").unwrap();
    fs::write(fleet_dir.join("index.sqlite-shm"), b"shm").unwrap();

    let _ = FleetIndex::open_or_recover_at_path(&sqlite_path).unwrap();

    let mut renamed = 0;
    for entry in fs::read_dir(&fleet_dir).unwrap() {
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
