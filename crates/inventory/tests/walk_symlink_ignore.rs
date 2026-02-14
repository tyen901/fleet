use std::fs::File;
use std::os::unix::fs::symlink;

use inventory::scanner::walk::WalkStream;
use inventory::ScanPolicy;

#[test]
fn symlinks_are_ignored_in_walk() {
    let td = tempfile::tempdir().unwrap();
    let root = td.path();

    let real = root.join("real.txt");
    File::create(&real).unwrap();

    let link = root.join("link.txt");
    symlink(&real, &link).unwrap();

    let policy = ScanPolicy::default();

    let mut ws = WalkStream::new(root, &policy).unwrap();
    let mut seen = Vec::new();
    while let Some(Ok(item)) = ws.next() {
        seen.push(item.rel_path);
    }

    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0], "real.txt");
}
