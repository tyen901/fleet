use fleet_index::ids::{enabled_mods_hash, normalize_repo_id, state_id};
use fleet_index::local_check::{LocalCheckOptions, LocalCheckOutcome};
use fleet_index::skip_repair::{SkipRepairDecision, SkipRepairPolicy, SkipRepairReason};
use fleet_index::store::DesiredStateChange;
use fleet_index::{DesiredState, ExpectedFile, FleetIndex};
use std::fs;
use std::io::Write;

fn make_desired(repo_id: &str, mods: &[&str]) -> DesiredState {
    let mut mods_sorted: Vec<String> = mods.iter().map(|s| s.to_string()).collect();
    mods_sorted.sort();
    let enabled_hash = enabled_mods_hash(&mods_sorted);
    let state = state_id(repo_id, &enabled_hash);
    DesiredState {
        repo_url: "https://example.invalid/repo".to_string(),
        repo_id: normalize_repo_id(repo_id),
        enabled_mods_hash: enabled_hash,
        state_id: state,
        updated_at_unix_s: 123,
    }
}

fn mtime_ns(path: &std::path::Path) -> i64 {
    let md = fs::metadata(path).unwrap();
    md.modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64
}

#[test]
#[cfg(unix)]
fn skip_repair_fails_on_symlink_ancestor() {
    let tmp = tempfile::tempdir().unwrap();
    let checkout_root = tmp.path();
    let mod_root = checkout_root.join("@mod");
    fs::create_dir_all(&mod_root).unwrap();

    // outside/bad.bin exists, but will be reachable only through a symlink ancestor in @mod/addons
    let outside = checkout_root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let bad = outside.join("bad.bin");
    fs::write(&bad, b"hello").unwrap(); // size = 5
    let bad_mtime = mtime_ns(&bad);

    // Create ancestor symlink: @mod/addons -> outside
    std::os::unix::fs::symlink(&outside, mod_root.join("addons")).unwrap();

    let mut idx = FleetIndex::open_in_memory().unwrap();
    let desired = make_desired("abcd", &["@mod"]);
    idx.set_desired_state(desired.clone()).unwrap();
    idx.verified_set(&desired.state_id, 10).unwrap();

    idx.expected_replace_all(
        &desired.state_id,
        vec![ExpectedFile {
            mod_id: "@mod".to_string(),
            rel_path: "addons/bad.bin".to_string(),
            size: 5,
        }],
    )
    .unwrap();

    // Provide cache so the only reason to reject "skip" is the unsafe ancestor.
    idx.file_state_upsert(
        &desired.state_id,
        "@mod",
        "addons/bad.bin",
        5,
        bad_mtime,
        b"checksum",
    )
    .unwrap();

    let decision = idx
        .evaluate_skip_repair(checkout_root, SkipRepairPolicy::default())
        .unwrap();

    match decision {
        SkipRepairDecision::NotSkippable { reason, evidence } => {
            assert!(matches!(reason, SkipRepairReason::LocalCheckFailed));
            assert!(evidence.local_unsafe_path > 0);
        }
        _ => panic!("expected NotSkippable due to symlink ancestor"),
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
    let changed = idx.set_desired_state(desired2.clone()).unwrap();
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
fn local_check_detects_issues_and_caps() {
    let tmp = tempfile::tempdir().unwrap();
    let checkout_root = tmp.path();

    let mod_root = checkout_root.join("@mod");
    fs::create_dir_all(&mod_root).unwrap();

    let mut ok_file = fs::File::create(mod_root.join("ok.txt")).unwrap();
    writeln!(ok_file, "hello").unwrap();

    let mut wrong_file = fs::File::create(mod_root.join("wrong.txt")).unwrap();
    write!(wrong_file, "x").unwrap();

    fs::create_dir_all(mod_root.join("dir.txt")).unwrap();

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(mod_root.join("ok.txt"), mod_root.join("link.txt")).unwrap();
    }

    let mut idx = FleetIndex::open_in_memory().unwrap();
    let desired = make_desired("abcd", &["@mod"]);
    idx.set_desired_state(desired.clone()).unwrap();

    idx.expected_replace_all(
        &desired.state_id,
        vec![
            ExpectedFile {
                mod_id: "@mod".to_string(),
                rel_path: "ok.txt".to_string(),
                size: 6,
            },
            ExpectedFile {
                mod_id: "@mod".to_string(),
                rel_path: "missing.txt".to_string(),
                size: 1,
            },
            ExpectedFile {
                mod_id: "@mod".to_string(),
                rel_path: "wrong.txt".to_string(),
                size: 10,
            },
            ExpectedFile {
                mod_id: "@mod".to_string(),
                rel_path: "dir.txt".to_string(),
                size: 1,
            },
            #[cfg(unix)]
            ExpectedFile {
                mod_id: "@mod".to_string(),
                rel_path: "link.txt".to_string(),
                size: 1,
            },
        ],
    )
    .unwrap();

    let outcome = idx
        .local_check(checkout_root, LocalCheckOptions { max_issues: 2 })
        .unwrap();

    let LocalCheckOutcome::Report(report) = outcome else {
        panic!("expected report")
    };

    assert_eq!(report.ok, 1);
    assert_eq!(report.missing, 1);
    assert_eq!(report.wrong_size, 1);
    assert_eq!(report.not_a_file, 1 + cfg!(unix) as u64);
    assert_eq!(report.issues.len(), 2);
}

#[test]
fn skip_repair_requires_verified_match() {
    let tmp = tempfile::tempdir().unwrap();
    let checkout_root = tmp.path();
    fs::create_dir_all(checkout_root.join("@mod")).unwrap();

    let mut idx = FleetIndex::open_in_memory().unwrap();
    let desired = make_desired("abcd", &["@mod"]);
    idx.set_desired_state(desired.clone()).unwrap();

    idx.expected_replace_all(
        &desired.state_id,
        vec![ExpectedFile {
            mod_id: "@mod".to_string(),
            rel_path: "ok.txt".to_string(),
            size: 0,
        }],
    )
    .unwrap();

    let decision = idx
        .evaluate_skip_repair(checkout_root, SkipRepairPolicy::default())
        .unwrap();
    match decision {
        SkipRepairDecision::NotSkippable { reason, .. } => {
            assert!(matches!(reason, SkipRepairReason::NotVerified))
        }
        _ => panic!("expected NotSkippable"),
    }
}

#[test]
fn skip_repair_fails_on_cache_missing_and_mtime_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let checkout_root = tmp.path();
    let mod_root = checkout_root.join("@mod");
    fs::create_dir_all(&mod_root).unwrap();

    let file_path = mod_root.join("ok.txt");
    fs::write(&file_path, b"hello").unwrap();

    let mut idx = FleetIndex::open_in_memory().unwrap();
    let desired = make_desired("abcd", &["@mod"]);
    idx.set_desired_state(desired.clone()).unwrap();
    idx.verified_set(&desired.state_id, 10).unwrap();

    idx.expected_replace_all(
        &desired.state_id,
        vec![ExpectedFile {
            mod_id: "@mod".to_string(),
            rel_path: "ok.txt".to_string(),
            size: 5,
        }],
    )
    .unwrap();

    let decision = idx
        .evaluate_skip_repair(checkout_root, SkipRepairPolicy::default())
        .unwrap();
    match decision {
        SkipRepairDecision::NotSkippable { reason, .. } => {
            assert!(matches!(reason, SkipRepairReason::CacheMissing))
        }
        _ => panic!("expected NotSkippable"),
    }

    let mtime = mtime_ns(&file_path);
    idx.file_state_upsert(
        &desired.state_id,
        "@mod",
        "ok.txt",
        5,
        mtime - 1,
        b"checksum",
    )
    .unwrap();

    let decision = idx
        .evaluate_skip_repair(checkout_root, SkipRepairPolicy::default())
        .unwrap();
    match decision {
        SkipRepairDecision::NotSkippable { reason, .. } => {
            assert!(matches!(reason, SkipRepairReason::MtimeMismatch))
        }
        _ => panic!("expected NotSkippable"),
    }
}

#[test]
fn skip_repair_succeeds_only_when_strict() {
    let tmp = tempfile::tempdir().unwrap();
    let checkout_root = tmp.path();
    let mod_root = checkout_root.join("@mod");
    fs::create_dir_all(&mod_root).unwrap();

    let file_path = mod_root.join("ok.txt");
    fs::write(&file_path, b"hello").unwrap();
    let mtime = mtime_ns(&file_path);

    let mut idx = FleetIndex::open_in_memory().unwrap();
    let desired = make_desired("abcd", &["@mod"]);
    idx.set_desired_state(desired.clone()).unwrap();
    idx.verified_set(&desired.state_id, 10).unwrap();

    idx.expected_replace_all(
        &desired.state_id,
        vec![ExpectedFile {
            mod_id: "@mod".to_string(),
            rel_path: "ok.txt".to_string(),
            size: 5,
        }],
    )
    .unwrap();

    idx.file_state_upsert(&desired.state_id, "@mod", "ok.txt", 5, mtime, b"checksum")
        .unwrap();

    let decision = idx
        .evaluate_skip_repair(checkout_root, SkipRepairPolicy::default())
        .unwrap();

    match decision {
        SkipRepairDecision::Skippable(ev) => {
            assert_eq!(ev.cache_missing, 0);
            assert_eq!(ev.mtime_mismatch, 0);
        }
        _ => panic!("expected skippable"),
    }
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

    let _ = FleetIndex::open_or_recover(checkout_root).unwrap();

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
