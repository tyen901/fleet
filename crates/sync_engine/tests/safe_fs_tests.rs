use sync_engine::safe_fs::ensure_no_symlink_ancestors;

#[test]
fn safe_fs_happy_path() {
    let tmp = tempfile::tempdir().unwrap();
    let mod_root = tmp.path().join("@mod");
    std::fs::create_dir_all(mod_root.join("addons")).unwrap();
    let parent = mod_root.join("addons");
    ensure_no_symlink_ancestors(&mod_root, &parent).unwrap();
}

#[test]
#[cfg(unix)]
fn safe_fs_symlink_ancestor_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let mod_root = tmp.path().join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    let target = tmp.path().join("outside");
    std::fs::create_dir_all(&target).unwrap();

    let link = mod_root.join("addons");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let parent = link.join("nested");
    let err = ensure_no_symlink_ancestors(&mod_root, &parent).unwrap_err();
    assert!(err.to_string().contains("unsafe path (symlink ancestor)"));
}

#[test]
#[cfg(unix)]
fn safe_fs_final_path_symlink_ok_for_ancestry() {
    let tmp = tempfile::tempdir().unwrap();
    let mod_root = tmp.path().join("@mod");
    std::fs::create_dir_all(&mod_root).unwrap();

    let target = mod_root.join("real");
    std::fs::write(&target, b"data").unwrap();

    let link = mod_root.join("file.bin");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let parent = link.parent().unwrap();
    ensure_no_symlink_ancestors(&mod_root, parent).unwrap();
}
