use std::fs::File;
use std::os::unix::fs::symlink;

use inventory::{compute_stamp, ScanPolicy};

#[test]
fn compute_stamp_ignores_symlinks() {
    let td = tempfile::tempdir().unwrap();
    let root = td.path();

    let real = root.join("a.txt");
    File::create(&real).unwrap();

    let link = root.join("b.txt");
    symlink(&real, &link).unwrap();

    let policy = ScanPolicy::default();
    let stamp = compute_stamp(root, &policy).unwrap();

    assert_eq!(stamp.file_count, 1);
}
